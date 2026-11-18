#!/usr/bin/env node
/**
 * @file PLAhelper 后台 Plasticity(用户 2026-11-18): 起一个**独立**的 Plasticity 进程做独立任务, 前台的人照常用浏览器 / 另一个 Plasticity,
 * 互不打扰 —— 后台这个不闪现、不进任务栏、不抢焦点 / 鼠标(PLAhelper 的操作都是页面里的合成事件, 从不动真鼠标)。
 *
 * 独立 = 单独的 userData(`--user-data-dir`, Electron 的单实例锁按 userData 算, 所以不会转给前台那个); 首次从用户的 userData
 * 拷 Local Storage / Preferences / Local State(登录与设置; 实测许可照样有效)。`--inspect-brk` 让主进程等调试器(Electron 不会真停在第一行),
 * 这时注入 pla-bg-hook.js(窗口一律隐藏 / 不节流, 对话框不弹)再放行 —— 窗口从创建起就不可见; userData 不对钩子自己退出(在单实例锁之前)。
 * 再起一个 9343 的 CDP 代理(pla-cdp-v8.js proxy, 主进程 inspector 9344), PLAhelper / pla-mcp 连 9343 就是后台这个。
 *
 * 用法: node pla-bg.js start [--exe <Plasticity.exe>] [--doc <file.plasticity>] [--resync]
 *       node pla-bg.js status | stop | eval <主进程表达式文件> | page <页面表达式文件>(在后台窗口的页面里执行, 可返回 Promise)
 * 输出一行 JSON。
 */
'use strict';
const fs = require('fs');
const path = require('path');
const os = require('os');
const { spawn } = require('child_process');

const INSPECT_PORT = Number(process.env.PLA_BG_INSPECT_PORT || 9344);
const PROXY_PORT = Number(process.env.PLA_BG_PROXY_PORT || 9343);
const HOME = path.join(process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local'), 'PLAhelper', 'bg-plasticity');
const USER_DATA = path.join(HOME, 'userData');
const STATE = path.join(HOME, 'state.json');
const SRC_USER_DATA = path.join(process.env.APPDATA || path.join(os.homedir(), 'AppData', 'Roaming'), 'Plasticity');
const SEED = ['Local Storage', 'Preferences', 'Local State'];
// 没给 --doc 时换成新文档(拷来的设置会让它打开用户上次的文档; 两个实例开同一个文件不安全)。
// 视口元素 = Rust 垫片配置 VIEWPORT_SELECTOR; 命令名 = Plasticity 命令注册表里的 file:new(同垫片 cmd.dispatch 的派发方式)。
const VIEWPORT_SELECTOR = 'plasticity-viewport';
const NEW_DOC_CMD = 'file:new';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const out = (o) => { process.stdout.write(JSON.stringify(o) + '\n'); };

async function inspectorTarget(port) {
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(1500) })).json();
    return list.find((t) => t.type === 'node') || null;
  } catch (e) { return null; }
}

/** 最小 CDP 客户端(Node 自带 WebSocket)。 */
async function cdp(url) {
  const ws = new WebSocket(url);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = () => j(new Error('ws connect failed: ' + url)); });
  let id = 0;
  const pending = new Map();
  const listeners = [];
  ws.onmessage = (m) => {
    const d = JSON.parse(m.data);
    if (d.id && pending.has(d.id)) { const p = pending.get(d.id); pending.delete(d.id); d.error ? p.j(new Error(d.error.message)) : p.r(d.result); }
    else if (d.method) listeners.forEach((f) => f(d));
  };
  return {
    send: (method, params) => new Promise((r, j) => { const i = ++id; pending.set(i, { r, j }); ws.send(JSON.stringify({ id: i, method, params: params || {} })); }),
    on: (f) => listeners.push(f),
    close: () => ws.close(),
  };
}

async function mainEval(expr) {
  const t = await inspectorTarget(INSPECT_PORT);
  if (!t) throw new Error('background Plasticity is not running (no inspector on ' + INSPECT_PORT + ')');
  const c = await cdp(t.webSocketDebuggerUrl);
  try {
    const r = await c.send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error((r.exceptionDetails.exception && r.exceptionDetails.exception.description) || r.exceptionDetails.text);
    return r.result.value;
  } finally { c.close(); }
}

/** 在后台窗口的页面里执行(主进程 webContents.executeJavaScript; 不经代理)。 */
async function pageEval(code) {
  const js = '(() => { const e = process.mainModule.require(\'electron\'); const w = (e.BaseWindow ? e.BaseWindow.getAllWindows() : e.BrowserWindow.getAllWindows()).find((x) => x.webContents); '
    + 'if (!w) throw new Error(\'no window\'); return w.webContents.executeJavaScript(' + JSON.stringify(code) + ', true); })()';
  return mainEval(js);
}

function findExe() {
  const roots = [path.join(os.homedir(), 'Programs'), path.join(process.env.LOCALAPPDATA || '', 'Plasticity')];
  const found = [];
  for (const r of roots) {
    let ds = [];
    try { ds = fs.readdirSync(r).filter((d) => /^app-[\d.]+$/.test(d)); } catch (e) {}
    for (const d of ds) { const p = path.join(r, d, 'Plasticity.exe'); if (fs.existsSync(p)) found.push(p); }
  }
  found.sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);
  return found[0] || null;
}

function seedUserData(resync) {
  fs.mkdirSync(USER_DATA, { recursive: true });
  const copied = [];
  for (const n of SEED) {
    const src = path.join(SRC_USER_DATA, n), dst = path.join(USER_DATA, n);
    if (!fs.existsSync(src) || (fs.existsSync(dst) && !resync)) continue;
    try { fs.cpSync(src, dst, { recursive: true, force: true, filter: (s) => !/[\\/]LOCK$/.test(s) }); copied.push(n); } catch (e) { copied.push(n + ' (failed: ' + e.message + ')'); }
  }
  return copied;
}

const STATUS_JS = `(() => {
  const { BrowserWindow } = require('electron');
  const G = globalThis.__PLA_BG__ || {};
  return JSON.stringify({ hooked: !!G.hooked, blocked: G.blocked || 0, log: (G.log || []).slice(-12),
    windows: BrowserWindow.getAllWindows().map((w) => ({ id: w.id, title: w.getTitle(), visible: w.isVisible(), focused: w.isFocused(), focusable: w.isFocusable(),
      throttling: w.webContents.getBackgroundThrottling(), loading: w.webContents.isLoading(), url: w.webContents.getURL().slice(0, 80) })) });
})()`.replace(/require\('electron'\)/g, "process.mainModule.require('electron')");

async function start(args) {
  if (await inspectorTarget(INSPECT_PORT)) return { already: true, ...JSON.parse(await mainEval(STATUS_JS)) };
  const exe = args.exe || findExe();
  if (!exe) throw new Error('Plasticity.exe not found (pass --exe)');
  const copied = seedUserData(!!args.resync);
  const argv = [`--user-data-dir=${USER_DATA}`, `--inspect-brk=127.0.0.1:${INSPECT_PORT}`];
  if (args.doc) argv.push(args.doc);
  const child = spawn(exe, argv, { detached: true, stdio: 'ignore', windowsHide: true });
  child.unref();
  const pid = child.pid;
  // 等 inspector
  let t = null;
  for (let i = 0; i < 100 && !t; i++) { await sleep(150); t = await inspectorTarget(INSPECT_PORT); }
  if (!t) { try { process.kill(pid); } catch (e) {} throw new Error('no inspector on ' + INSPECT_PORT + ' (is Plasticity.exe fuse-patched for --inspect?)'); }
  // 进程在等调试器: 这时连 process 都还没有(Node 还没起来)→ 设"每个脚本执行前停", 放行后跳过启动脚本,
  // 停在第一个 Plasticity 自己的脚本(app.asar)前: Node 已就绪、Plasticity 的代码还没跑 → 注入钩子 → 撤断点 → 继续
  const c = await cdp(t.webSocketDebuggerUrl);
  const hook = fs.readFileSync(path.join(__dirname, 'pla-bg-hook.js'), 'utf8').replace(/^\/\*\*[\s\S]*?\*\/\s*/, '');
  const expr = 'globalThis.__PLA_BG_EXPECT__ = ' + JSON.stringify(path.resolve(USER_DATA)) + ';\n' + hook;
  let hooked = null, hookErr = null, skipped = 0;
  const done = new Promise((resolve) => {
    c.on(async (m) => {
      if (m.method !== 'Debugger.paused') return;
      const url = (m.params.data && m.params.data.url) || (m.params.callFrames[0] && m.params.callFrames[0].url) || '';
      if (hooked || hookErr || !/app\.asar/i.test(url)) { skipped++; c.send('Debugger.resume').catch(() => {}); return; }
      try {
        const r = await c.send('Runtime.evaluate', { expression: expr, returnByValue: true });
        if (r.exceptionDetails) hookErr = (r.exceptionDetails.exception && r.exceptionDetails.exception.description) || r.exceptionDetails.text;
        else hooked = r.result.value;
      } catch (e) { hookErr = e.message; }
      if (hooked) { try { await c.send('Debugger.removeBreakpoint', { breakpointId: bp }).catch(() => {}); } catch (e) {} }
      resolve();
      if (hooked) c.send('Debugger.resume').catch(() => {});
    });
  });
  await c.send('Runtime.enable');
  await c.send('Debugger.enable');
  const bpr = await c.send('Debugger.setInstrumentationBreakpoint', { instrumentation: 'beforeScriptExecution' });
  const bp = bpr.breakpointId;
  await c.send('Runtime.runIfWaitingForDebugger');
  await Promise.race([done, sleep(20000)]);
  if (!hooked) {
    c.close(); try { process.kill(pid); } catch (e) {}
    throw new Error('hook failed: ' + (hookErr || 'no app.asar script seen (skipped ' + skipped + ')'));
  }
  await c.send('Debugger.disable').catch(() => {});
  c.close();  const ud = path.resolve(USER_DATA);  fs.writeFileSync(STATE, JSON.stringify({ pid, exe, userData: ud, inspect: INSPECT_PORT, proxy: PROXY_PORT, started: Date.now() }));
  // 等窗口加载完
  let st = null;
  for (let i = 0; i < 120; i++) {
    await sleep(500);
    try { st = JSON.parse(await mainEval(STATUS_JS)); } catch (e) { continue; }
    if (st.windows.length && st.windows.every((w) => !w.loading)) break;
  }
  let title = null;
  if (!args.doc) {
    const js = '(async () => { const el = document.querySelector(' + JSON.stringify(VIEWPORT_SELECTOR) + '); if (!el) return "no viewport"; '
      + 'el.dispatchEvent(new CustomEvent(' + JSON.stringify(NEW_DOC_CMD) + ', { bubbles: true })); '
      + 'for (let i = 0; i < 50 && !/^Untitled/.test(document.title); i++) await new Promise((r) => setTimeout(r, 100)); return document.title; })()';
    try { title = await pageEval(js); } catch (e) { title = 'new doc failed: ' + e.message; }
  } else {
    try { title = await pageEval('document.title'); } catch (e) {}
  }
  // CDP 代理(9343 → 后台实例的 inspector 9344)
  const proxy = spawn(process.execPath, [path.join(__dirname, 'pla-cdp-v8.js'), 'proxy'], {
    detached: true, stdio: 'ignore', windowsHide: true,
    env: Object.assign({}, process.env, { PLA_INSPECT_PORT: String(INSPECT_PORT), PLA_PROXY_PORT: String(PROXY_PORT) }),
  });
  proxy.unref();
  const s = JSON.parse(fs.readFileSync(STATE, 'utf8'));
  s.proxyPid = proxy.pid;
  fs.writeFileSync(STATE, JSON.stringify(s));
  return { started: true, pid, hooked, document: title, seeded: copied, userData: ud, proxy: PROXY_PORT, status: st };
}

async function stop() {
  let s = {};
  try { s = JSON.parse(fs.readFileSync(STATE, 'utf8')); } catch (e) {}
  let quit = false;
  try { await mainEval("(() => { setTimeout(() => process.mainModule.require('electron').app.exit(0), 100); return 'bye'; })()"); quit = true; } catch (e) {}
  await sleep(1500);
  if (s.proxyPid) { try { process.kill(s.proxyPid); } catch (e) {} }
  if (!quit && s.pid) { try { process.kill(s.pid); } catch (e) {} }
  try { fs.unlinkSync(STATE); } catch (e) {}
  return { stopped: true, graceful: quit };
}

async function main() {
  const [cmd, ...rest] = process.argv.slice(2);
  const args = {};
  for (let i = 0; i < rest.length; i++) {
    if (rest[i] === '--exe') args.exe = rest[++i];
    else if (rest[i] === '--doc') args.doc = rest[++i];
    else if (rest[i] === '--resync') args.resync = true;
    else args._ = rest[i];
  }
  try {
    if (cmd === 'start') out(await start(args));
    else if (cmd === 'status') out((await inspectorTarget(INSPECT_PORT)) ? { running: true, ...JSON.parse(await mainEval(STATUS_JS)) } : { running: false });
    else if (cmd === 'stop') out(await stop());
    else if (cmd === 'eval') out({ value: await mainEval(fs.readFileSync(args._, 'utf8')) });
    else if (cmd === 'page') out({ value: await pageEval(fs.readFileSync(args._, 'utf8')) });
    else { out({ error: 'usage: pla-bg.js start [--exe X] [--doc F] [--resync] | status | stop | eval <file>' }); process.exitCode = 2; }
  } catch (e) { out({ error: e.message }); process.exitCode = 1; }
  process.exit();
}
main();
