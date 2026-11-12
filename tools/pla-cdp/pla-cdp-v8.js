/**
 * @file PLA-CDP v8: fuse patch + inspector launcher + 9333 CDP proxy.
 *
 * 26.2.3+ layout (hardened): app.asar + plasticity.jsc.pack. Content inside
 * app.asar cannot be touched (Plasticity self-verifies). But the Electron
 * fuse wire in Plasticity.exe is NOT verified -> flipping one byte
 * (fuse index 3, NodeCliInspect) revives `--inspect`, a Node-level inspector
 * that the app cannot strip (it only removes the Chromium-level
 * remote-debugging-port switch). See
 * docs/know-how/plasticity-2623-hardening.md section 9.
 *
 * This tool:
 *   fuse      flip the NodeCliInspect fuse byte (idempotent, backup first)
 *   status    scan installs, report layout + fuse state + proxy port
 *   launch    start Plasticity with --inspect=127.0.0.1:9334 and keep the
 *             9333 CDP proxy running (compat shim for PLAhelper)
 *   proxy     run only the 9333 proxy (app assumed already running w/ inspect)
 *
 * Compat model (zero changes on the Rust side):
 *   GET 127.0.0.1:9333/json/version   -> inspector /json/version (node)
 *   GET 127.0.0.1:9333/json/list      -> one fake "page" target whose
 *        webSocketDebuggerUrl points back at the proxy; PLAhelper connects
 *        there and every {"id","method","params"} is forwarded into the
 *        real renderer via webContents.debugger (attached over the
 *        inspector session), responses and events piped back.
 *
 * ASCII-only output on purpose (GBK console).
 */

'use strict';

const fs = require('fs');
const path = require('path');
const http = require('http');
const { spawn } = require('child_process');

const INSPECTOR_PORT = 9334;
const PROXY_PORT = 9333;
const FUSE_SENTINEL = Buffer.from('dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX');
const FUSE_INDEX_INSPECT = 3; // RunAsNode, CookieEncryption, NodeOptions, NodeCliInspect, ...

/** Reuse v7 root discovery. */
const { installDirs } = require('./pla-cdp-v7.js');

/**
 * Reads the fuse wire of an exe.
 * @param {string} exePath
 * @return {{offset:number, wire:string}|null} null when sentinel not found.
 */
function readFuse(exePath) {
  const fh = fs.openSync(exePath, 'r');
  const stat = fs.fstatSync(fh);
  const WINDOW = 4096;
  let fileOff = 0;
  try {
    while (fileOff < stat.size) {
      const len = Math.min(WINDOW + FUSE_SENTINEL.length, stat.size - fileOff);
      const buf = Buffer.alloc(len);
      fs.readSync(fh, buf, 0, len, fileOff);
      const at = buf.indexOf(FUSE_SENTINEL);
      if (at >= 0) {
        const abs = fileOff + at;
        const head = Buffer.alloc(FUSE_SENTINEL.length + 2);
        fs.readSync(fh, head, 0, head.length, abs);
        const ver = head[FUSE_SENTINEL.length];
        const wireLen = head[FUSE_SENTINEL.length + 1];
        const wireBuf = Buffer.alloc(wireLen);
        fs.readSync(fh, wireBuf, 0, wireLen, abs + FUSE_SENTINEL.length + 2);
        return { offset: abs + FUSE_SENTINEL.length + 2, wire: wireBuf.toString('latin1') };
      }
      fileOff += WINDOW;
    }
  } finally {
    fs.closeSync(fh);
  }
  return null;
}

/**
 * Flips one fuse bit. Idempotent. Backup `.pla-fuse-backup` on first change.
 * @param {string} exePath
 * @param {number} index fuse index to set to '1'.
 * @return {string} human readable result line.
 */
function flipFuse(exePath, index) {
  const fuse = readFuse(exePath);
  if (!fuse) {
    return 'ERROR: fuse sentinel not found in ' + exePath;
  }
  if (fuse.wire[index] === '1') {
    return `already patched (wire=${fuse.wire})`;
  }
  const backup = exePath + '.pla-fuse-backup';
  if (!fs.existsSync(backup)) {
    fs.copyFileSync(exePath, backup);
  }
  const b = Buffer.from('1');
  try {
    const fh = fs.openSync(exePath, 'r+');
    try {
      fs.writeSync(fh, b, 0, 1, fuse.offset + index);
    } finally {
      fs.closeSync(fh);
    }
  } catch (err) {
    if (err && err.code === 'EBUSY') {
      // On Windows, a running .exe cannot be opened for write ('r+'),
      // but CAN be renamed while running so a patched copy takes its path.
      const tmpPatched = exePath + '.pla-tmp-patched';
      const oldRunning = exePath + '.pla-running-old';
      fs.copyFileSync(backup, tmpPatched);
      const fh = fs.openSync(tmpPatched, 'r+');
      try {
        fs.writeSync(fh, b, 0, 1, fuse.offset + index);
      } finally {
        fs.closeSync(fh);
      }
      try { if (fs.existsSync(oldRunning)) fs.unlinkSync(oldRunning); } catch (e) {}
      fs.renameSync(exePath, oldRunning);
      fs.renameSync(tmpPatched, exePath);
    } else {
      throw err;
    }
  }
  const check = readFuse(exePath);
  return check.wire[index] === '1'
    ? `patched (wire ${fuse.wire} -> ${check.wire}), backup: ${path.basename(backup)}`
    : 'ERROR: verification failed';
}

/**
 * Restores the fuse backup.
 * @param {string} exePath
 * @return {string}
 */
function restoreFuse(exePath) {
  const backup = exePath + '.pla-fuse-backup';
  if (!fs.existsSync(backup)) {
    return 'no backup found';
  }
  fs.copyFileSync(backup, exePath);
  return 'restored from ' + path.basename(backup);
}

/**
 * Checks whether a file is writable by the current user.
 * @param {string} filePath
 * @return {boolean}
 */
function isWritable(filePath) {
  try {
    fs.accessSync(filePath, fs.constants.R_OK | fs.constants.W_OK);
    return true;
  } catch (e) {
    return false;
  }
}

/**
 * Finds the newest hardened install, preferring user-writable installs first.
 * @return {{dir:string, exe:string}|null}
 */
function newestHardened() {
  const candidates = [];
  for (const dir of installDirs()) {
    const exe = path.join(dir, 'Plasticity.exe');
    const asar = path.join(dir, 'resources', 'app.asar');
    if (fs.existsSync(exe) && fs.existsSync(asar)) {
      const writable = isWritable(exe);
      const base = path.basename(dir); // e.g. app-26.2.3
      candidates.push({ dir, exe, writable, base });
    }
  }
  candidates.sort((a, b) => {
    if (a.writable !== b.writable) return a.writable ? -1 : 1;
    return b.base.localeCompare(a.base, undefined, { numeric: true });
  });
  return candidates.length ? { dir: candidates[0].dir, exe: candidates[0].exe } : null;
}

/**
 * Parses a V8 heap snapshot JSON file in-process and returns the heap object ID
 * of the Editor instance (the unique node with outgoing property edges
 * 'selection', 'shading', and 'viewports').
 * @param {string} heapPath
 * @return {number|null}
 */
function findEditorInHeapFile(heapPath) {
  const d = JSON.parse(fs.readFileSync(heapPath, 'utf8'));
  const nodes = d.nodes;
  const strings = d.strings;
  const edges = d.edges;
  const k = 6; // [type, name, id, self_size, edge_count, detachedness]
  const w = 3; // [type, name_or_index, to_node]
  const n = Math.floor(nodes.length / k);
  let eoff = 0;
  for (let i = 0; i < n; i++) {
    const off = i * k;
    const ecnt = nodes[off + 4];
    let mask = 0;
    for (let j = 0; j < ecnt; j++) {
      const eo = eoff + j * w;
      const nameidx = edges[eo + 1];
      if (typeof nameidx === 'number' && nameidx >= 0 && nameidx < strings.length) {
        const nm = strings[nameidx];
        if (nm === 'selection') mask |= 1;
        else if (nm === 'shading') mask |= 2;
        else if (nm === 'viewports') mask |= 4;
      }
    }
    eoff += ecnt * w;
    if (mask === 7) {
      return nodes[off + 2];
    }
  }
  return null;
}

/* ------------------------------------------------------------------ */
/* CDP proxy                                                            */
/* ------------------------------------------------------------------ */

/**
 * One proxied CDP session between PLAhelper (client ws) and the renderer
 * (electron debugger, driven over the main inspector).
 * @param {http.IncomingMessage} req
 * @param {import('net').Socket} sock
 * @param {Buffer} head
 */
async function proxySession(req, sock, head) {
  const key = req.headers['sec-websocket-key'];
  if (!key) { sock.destroy(); return; }
  const accept = crypto.accept(key);
  sock.write(
    'HTTP/1.1 101 Switching Protocols\r\n' +
    'Upgrade: websocket\r\n' +
    'Connection: Upgrade\r\n' +
    `Sec-WebSocket-Accept: ${accept}\r\n\r\n`
  );
  if (head && head.length) sock.emit('data', head);

  // Attach to renderer over the inspector, and bridge.
  const io = await InspectorIO.connect();
  const bridge = await RendererBridge.attach(io);
  sock.on('data', (buf) => bridge.feedClient(buf));
  bridge.onServerMessage((obj) => sock.write(crypto.frameText(JSON.stringify(obj))));
  bridge.onEvent((obj) => sock.write(crypto.frameText(JSON.stringify(obj))));
  const bye = () => { try { bridge.close(); } catch (e) {} try { sock.destroy(); } catch (e) {} };
  sock.on('close', bye);
  sock.on('error', bye);
  process.on('exit', bye);
}

/** Tiny ws framing helpers (server side, no extensions). */
const crypto = {
  accept(key) {
    return require('crypto').createHash('sha1')
      .update(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11')
      .digest('base64');
  },
  /** Frames a server->client TEXT message. */
  frameText(str) {
    const payload = Buffer.from(str);
    const len = payload.length;
    let header;
    if (len < 126) {
      header = Buffer.from([0x81, len]);
    } else if (len < 65536) {
      header = Buffer.alloc(4);
      header[0] = 0x81; header[1] = 126;
      header.writeUInt16BE(len, 2);
    } else {
      header = Buffer.alloc(10);
      header[0] = 0x81; header[1] = 127;
      header.writeBigUInt64BE(BigInt(len), 2);
    }
    return Buffer.concat([header, payload]);
  },
  /** Parses client->server frames from a buffer, calls back each TEXT payload. */
  parseFrames(buf, onText) {
    let off = 0;
    while (off + 2 <= buf.length) {
      const b1 = buf[off];
      const b2 = buf[off + 1];
      const opcode = b1 & 0x0f;
      const masked = (b2 & 0x80) !== 0;
      const len = b2 & 0x7f;
      let ptr = off + 2;
      let plen = len;
      if (len === 126) { if (ptr+2 > buf.length) break; plen = buf.readUInt16BE(ptr); ptr += 2; }
      else if (len === 127) { if (ptr+8 > buf.length) break; plen = Number(buf.readBigUInt64BE(ptr)); ptr += 8; }
      const maskLen = masked ? 4 : 0;
      if (ptr + maskLen + plen > buf.length) break; // partial frame, wait
      const mask = masked ? buf.subarray(ptr, ptr + 4) : null;
      ptr += maskLen;
      const payload = Buffer.from(buf.subarray(ptr, ptr + plen));
      if (mask) for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i & 3];
      if (opcode === 0x1) onText(payload.toString('utf8'), ptr + plen);
      if (opcode === 0x9) { /* ping: client->server ping rare; ignore */ }
      if (opcode === 0x8) return -1; // close
      off = ptr + plen;
    }
    return off;
  },
};

/** Inspector websocket client (Node 22 global WebSocket). */
class InspectorIO {
  /** @param {string} wsUrl */
  constructor(wsUrl) {
    this.ws = new WebSocket(wsUrl);
    this.seq = 0;
    this.pending = new Map();
    this.listeners = [];
    this.ws.addEventListener('message', (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id && this.pending.has(m.id)) {
        this.pending.get(m.id)(m);
        this.pending.delete(m.id);
      }
    });
  }
  /** @return {Promise<InspectorIO>} */
  static async connect() {
    const list = await (await fetch(`http://127.0.0.1:${INSPECTOR_PORT}/json/list`)).json();
    const target = list.find((t) => t.type === 'node');
    if (!target) throw new Error('no node target on inspector');
    const io = new InspectorIO(target.webSocketDebuggerUrl);
    await new Promise((res, rej) => {
      io.ws.addEventListener('open', res);
      io.ws.addEventListener('error', rej);
    });
    return io;
  }
  /** @param {string} method @param {object} params @return {Promise<object>} */
  call(method, params) {
    return new Promise((res) => {
      const id = ++this.seq;
      this.pending.set(id, res);
      this.ws.send(JSON.stringify({ id, method, params: params || {} }));
    });
  }
}

/** Renderer bridge via webContents.debugger over the inspector. */
class RendererBridge {
  /**
   * @param {InspectorIO} io
   * @return {Promise<RendererBridge>}
   */
  static async attach(io) {
    const attachJs = `(() => {
      const electron = process.mainModule.require('electron');
      const wins = electron.BrowserWindow.getAllWindows();
      if (!wins.length) return 'NO_WINDOW';
      const wc = wins[0].webContents;
      const dbg = wc.debugger;
      try { dbg.detach(); } catch (e) {}
      dbg.attach('1.3');
      globalThis.__PLA_PROXY_SEND = (m, p) => dbg.sendCommand(m, p);
      globalThis.__PLA_PROXY_EMIT = null;
      return 'OK';
    })()`;
    const r = await io.call('Runtime.evaluate', { expression: attachJs, returnByValue: true, awaitPromise: true });
    const v = r.result && r.result.result && r.result.result.value;
    if (v !== 'OK') throw new Error('attach failed: ' + JSON.stringify(r).slice(0, 200));

    const br = new RendererBridge(io);
    br.locateAndExposeEditor().catch((e) => {
      process.stderr.write('editor locate failed: ' + e.message + '\n');
    });
    return br;
  }
  constructor(io) {
    this.io = io;
    this.serverMsgCb = null;
    this.eventCb = null;
    this.buffer = Buffer.alloc(0);
  }

  /**
   * Heap-locates the Editor instance and exposes it as
   * window.__PLA_EDITOR__ inside the renderer (26.2.3: the editor object
   * lives in webpack closure scope; the heap snapshot is the only way in).
   * Must run on the SAME inspector session that took the snapshot.
   * @return {Promise<string>} 'OK' on success.
   */
  async locateAndExposeEditor() {
    const os = require('os');
    const heapPath = path.join(os.tmpdir(), 'pla-heap.json');
    const SEND = (method, params) => this.io.call('Runtime.evaluate', {
      expression: `__PLA_PROXY_SEND(${JSON.stringify(method)}, ${JSON.stringify(params || {})})`,
      returnByValue: true, awaitPromise: true,
    }).then((r) => {
      const v = r.result && r.result.result && r.result.result.value;
      if (v && v.__pla_err) throw new Error(v.__pla_err);
      return v;
    });
    // 0) fast path: already exposed?
    try {
      const pre = await SEND('Runtime.evaluate', {
        expression: '!!window.__PLA_EDITOR__', returnByValue: true,
      });
      if (pre === true) return 'OK: already';
    } catch (e) { /* fall through to full locate */ }
    // 1) take heap snapshot, collect chunks in the renderer-side main world
    const SNAP_JS = '(async () => {'
      + ' const electron = process.mainModule.require("electron");'
      + ' const w = electron.BrowserWindow.getAllWindows()[0];'
      + ' const dbg = w.webContents.debugger;'
      + ' const chunks = [];'
      + ' const onMsg = (ev, method, params) => { if (method === "HeapProfiler.addHeapSnapshotChunk") chunks.push(params.chunk); };'
      + ' dbg.on("message", onMsg);'
      + ' await __PLA_PROXY_SEND("HeapProfiler.enable", {});'
      + ' await __PLA_PROXY_SEND("HeapProfiler.takeHeapSnapshot", { reportProgress: false });'
      + ' await new Promise(r => setTimeout(r, 8000));'
      + ' dbg.removeListener("message", onMsg);'
      + ' const fs = process.mainModule.require("fs");'
      + ' try { fs.writeFileSync(' + JSON.stringify(heapPath) + ', chunks.join("")); }'
      + ' catch (e) { return "WRITE_ERR: " + e.message; }'
      + ' return "len=" + chunks.join("").length;'
      + '})()';
    const snap = await this.io.call('Runtime.evaluate', { expression: SNAP_JS, returnByValue: true, awaitPromise: true });
    const snapLen = snap.result && snap.result.result && snap.result.result.value;
    if (!snapLen || !String(snapLen).startsWith('len=')) {
      throw new Error('snapshot failed: ' + JSON.stringify(snap).slice(0, 200));
    }
    // 2) parse in-process (pure JS, no external Python dependency), get editor heap object id
    const foundId = findEditorInHeapFile(heapPath);
    if (foundId === null || foundId === undefined) {
      throw new Error('editor not found in heap (' + snapLen + ')');
    }
    const edId = String(foundId);
    // 3) revive + expose (same session)
    const REVIVE_JS = '(async () => {'
      + ' const r = await __PLA_PROXY_SEND("HeapProfiler.getObjectByHeapObjectId", { objectId: "' + edId + '" });'
      + ' if (!r || !r.result) return "ERR: revive " + JSON.stringify(r).slice(0, 120);'
      + ' const t = await __PLA_PROXY_SEND("Runtime.callFunctionOn", {'
      + '   objectId: r.result.objectId,'
      + '   functionDeclaration: "function(){ window.__PLA_EDITOR__ = this; return (this.selection && this.shading && this.viewports) ? \'OK\' : \'MISSING_FEATS\'; }",'
      + '   returnByValue: true });'
      + ' return t.result && t.result.value;'
      + '})()';
    const rev = await this.io.call('Runtime.evaluate', { expression: REVIVE_JS, returnByValue: true, awaitPromise: true });
    const revVal = rev.result && rev.result.result && rev.result.result.value;
    if (revVal !== 'OK') throw new Error('expose failed: ' + JSON.stringify(rev).slice(0, 300));
    return 'OK';
  }
  /** @param {Buffer} buf raw client bytes. */
  feedClient(buf) {
    this.buffer = Buffer.concat([this.buffer, buf]);
    let consumed = 0;
    crypto.parseFrames(this.buffer, (text, frameEnd) => {
      consumed = frameEnd;
      let msg;
      try { msg = JSON.parse(text); } catch (e) { return; }
      const forward = `__PLA_PROXY_SEND(${JSON.stringify(msg.method)}, ${JSON.stringify(msg.params || {})})`;
      this.io.call('Runtime.evaluate', {
        expression: forward,
        returnByValue: true, awaitPromise: true,
      }).then((r) => {
        const raw = r.result && r.result.result && r.result.result.value;
        const wrapped = { id: msg.id, result: (raw && typeof raw === 'object' && !raw.__pla_err) ? raw : (raw !== undefined ? { value: raw } : {}) };
        this.serverMsgCb && this.serverMsgCb(wrapped);
      }).catch((e) => {
        this.serverMsgCb && this.serverMsgCb({ id: msg.id, result: { __pla_err: String(e && e.message) } });
      });
    });
    if (consumed > 0) this.buffer = this.buffer.subarray(consumed);
  }
  onServerMessage(cb) { this.serverMsgCb = cb; }
  onEvent(cb) { this.eventCb = cb; }
  close() {
    try { this.io.ws.close(); } catch (e) {}
  }
}

/**
 * Runs the 9333 proxy: /json/version, /json/list, and ws upgrade.
 * @return {Promise<void>} resolves never (server loop).
 */
function runProxy() {
  return new Promise((resolve, reject) => {
    const server = http.createServer((req, res) => {
      if (req.url === '/json/version') {
        fetch(`http://127.0.0.1:${INSPECTOR_PORT}/json/version`)
          .then((r) => r.json())
          .then((v) => {
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ ...v, Browser: 'pla-cdp-proxy/1.0 (node ' + process.versions.node + ')' }));
          })
          .catch(() => { res.writeHead(502); res.end('inspector unreachable'); });
        return;
      }
      if (req.url === '/json/list') {
        // Content-Length explicit: the Rust client does not speak chunked.
        const body = JSON.stringify([{
          description: 'PLA CDP proxy (renderer via inspector)',
          devtoolsFrontendUrl: '',
          id: 'pla-proxy-renderer',
          title: 'Plasticity',
          type: 'page',
          url: 'file:///plasticity-renderer',
          webSocketDebuggerUrl: `ws://127.0.0.1:${PROXY_PORT}/`,
        }]);
        const payload = Buffer.from(body);
        res.writeHead(200, {
          'Content-Type': 'application/json',
          'Content-Length': payload.length,
        });
        res.end(payload);
        return;
      }
      res.writeHead(404); res.end();
    });
    server.on('upgrade', (req, sock, head) => {
      proxySession(req, sock, head).catch((e) => {
        process.stderr.write(`proxy session error: ${e.message}\n`);
        try { sock.destroy(); } catch (e2) {}
      });
   });
    server.listen(PROXY_PORT, '127.0.0.1', () => {
      process.stdout.write(`proxy listening on 127.0.0.1:${PROXY_PORT}\n`);
      resolve();
    });
  });
}

/* ------------------------------------------------------------------ */
/* CLI                                                                  */
/* ------------------------------------------------------------------ */

/**
 * Entry point.
 * @return {number} exit code.
 */
function main() {
  const argv = process.argv.slice(2);
  const cmd = (argv.find((v) => !v.startsWith('--')) || 'status').toLowerCase();

  const hardened = newestHardened();
  if (cmd === 'fuse') {
    if (!hardened) { console.log('no hardened install found'); return 3; }
    const exe = hardened.exe;
    const fuse = readFuse(exe);
    console.log(`exe    : ${exe}`);
    console.log(`fuse   : ${fuse ? 'wire=' + fuse.wire + ' @' + fuse.offset : 'sentinel not found'}`);
    console.log(`result : ${flipFuse(exe, FUSE_INDEX_INSPECT)}`);
    return 0;
  }
  if (cmd === 'unfuse') {
    if (!hardened) { console.log('no hardened install found'); return 3; }
    console.log(restoreFuse(hardened.exe));
    return 0;
  }
  if (cmd === 'launch') {
    if (!hardened) { console.log('no hardened install found'); return 3; }
    const fuse = readFuse(hardened.exe);
    if (!fuse || fuse.wire[FUSE_INDEX_INSPECT] !== '1') {
      console.log('fuse not patched; run `node pla-cdp-v8.js fuse` first');
      return 2;
    }
    console.log(`launching ${hardened.exe} --inspect=127.0.0.1:${INSPECTOR_PORT}`);
    const child = spawn(hardened.exe, [`--inspect=127.0.0.1:${INSPECTOR_PORT}`], {
      detached: false, stdio: 'ignore', cwd: path.dirname(hardened.exe),
    });
    child.unref();
    waitForInspector(60).then(() => runProxy()).catch((e) => {
      console.error('proxy failed: ' + e.message);
      process.exit(1);
    });
    return 0; // runProxy keeps the loop alive
  }
  if (cmd === 'proxy') {
    runProxy().catch((e) => { console.error('proxy failed: ' + e.message); process.exit(1); });
    return 0;
  }

  // status (default)
  const dirs = installDirs();
  console.log('PLA-CDP v8 scan');
  for (const dir of dirs) {
    const exe = path.join(dir, 'Plasticity.exe');
    const asar = path.join(dir, 'resources', 'app.asar');
    const legacy = fs.existsSync(path.join(dir, ...['resources', 'app', '.webpack', 'main', 'index.js']));
    const layout = legacy ? 'legacy' : (fs.existsSync(asar) ? 'hardened' : 'unknown');
    const fuse = fs.existsSync(exe) ? readFuse(exe) : null;
    console.log(`${dir}`);
    console.log(`  layout=${layout} fuse=${fuse ? fuse.wire : '-'} inspect=${fuse && fuse.wire[FUSE_INDEX_INSPECT] === '1' ? 'ENABLED' : 'off'}`);
  }
  console.log(`proxy port ${PROXY_PORT} (inspector ${INSPECTOR_PORT})`);
  return 0;
}

/**
 * Polls the inspector until reachable.
 * @param {number} seconds
 * @return {Promise<void>}
 */
function waitForInspector(seconds) {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    (function tick() {
      fetch(`http://127.0.0.1:${INSPECTOR_PORT}/json/version`, { signal: AbortSignal.timeout(1500) })
        .then(() => resolve())
        .catch(() => {
          if (Date.now() - start > seconds * 1000) reject(new Error('inspector not up in ' + seconds + 's'));
          else setTimeout(tick, 1000);
        });
    })();
  });
}

if (require.main === module) {
  process.exitCode = main();
}

module.exports = { readFuse, flipFuse, proxySession, RendererBridge };
