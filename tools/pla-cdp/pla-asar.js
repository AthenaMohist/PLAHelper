/**
 * @file Minimal dependency-free reader/writer for Electron .asar archives.
 *
 * Why this exists: PLAhelper ships no npm dependencies, but the CDP patcher has
 * to open/repack Plasticity's `resources/app.asar` on end-user machines that do
 * not necessarily have @electron/asar installed.
 *
 * On-disk layout (asar v2, little endian):
 *
 *   uint32        outer pickle payload size (always 4)
 *   uint32        total size of the inner pickle frame (= L + 8)
 *   uint32        inner pickle payload size (= L + 4)
 *   uint32        header JSON byte length (L)
 *   bytes[L]      UTF-8 JSON header: { files: { name: Directory | File } }
 *   bytes[..]     concatenated file payloads; File.offset is relative to here
 *
 * A File node is `{ size, offset, integrity?, unpacked? }`. Files marked
 * `unpacked` live in `<archive>.unpacked/<same relative path>` and are never
 * stored inside the archive blob.
 *
 * Usage:
 *   node pla-asar.js <asar> info
 *   node pla-asar.js <asar> list [prefix]
 *   node pla-asar.js <asar> extract <innerPath> <outFile>
 *   node pla-asar.js <asar> replace <innerPath> <inFile> [--backup]
 *   node pla-asar.js <asar> remove  <innerPath> [--backup]
 *
 * Programmatic:
 *   const { Asar } = require('./pla-asar.js');
 *   const asar = Asar.open('app.asar');
 *   asar.readFile('.webpack/main/index.js');          // Buffer
 *   asar.addFile('.webpack/main/index.js', newBuf);   // staged in memory
 *   asar.save();                                      // atomic rewrite
 */

'use strict';

const fs = require('fs');
const crypto = require('crypto');
const path = require('path');

/** Bytes consumed by the two pickle frames plus the two length prefixes. */
const HEADER_PREFIX_BYTES = 16;

/** Default block size used by @electron/asar for per-file integrity. */
const DEFAULT_BLOCK_SIZE = 4 * 1024 * 1024;

/**
 * @typedef {object} IntegrityRecord
 * @property {string} algorithm
 * @property {string} hash
 * @property {number=} blockSize
 * @property {string[]=} blocks
 */

/**
 * @typedef {object} FileNode
 * @property {number} size
 * @property {string|number} offset
 * @property {boolean=} unpacked
 * @property {IntegrityRecord=} integrity
 */

/**
 * Computes the SHA-256 integrity record for one payload.
 * @param {Buffer} buffer Payload bytes.
 * @param {boolean} withBlocks Whether per-block digests are expected.
 * @return {IntegrityRecord} Integrity record.
 */
function computeIntegrity(buffer, withBlocks) {
  const hash = crypto.createHash('SHA256').update(buffer).digest('hex');
  const record = /** @type {IntegrityRecord} */ ({
    algorithm: 'SHA256',
    hash,
    blockSize: DEFAULT_BLOCK_SIZE,
    blocks: [hash],
  });
  if (!withBlocks) {
    delete record.blockSize;
    delete record.blocks;
    return record;
  }
  const blocks = [];
  for (let offset = 0; offset < buffer.length; offset += DEFAULT_BLOCK_SIZE) {
    blocks.push(
      crypto
        .createHash('SHA256')
        .update(buffer.subarray(offset, Math.min(offset + DEFAULT_BLOCK_SIZE, buffer.length)))
        .digest('hex')
    );
  }
  record.blocks = blocks;
  return record;
}

/**
 * Read/write access to a single .asar archive.
 */
class Asar {
  /**
   * @param {string} archivePath Absolute path of the `.asar` file.
   */
  constructor(archivePath) {
    /** @type {string} */
    this.path = archivePath;
    /** @type {number} */
    this.fileSize = 0;
    /** @type {number} */
    this.dataStart = 0;
    /** @type {object} */
    this.header = { files: {} };
    /** @type {Map<string, FileNode>} */
    this.files = new Map();
    /** @type {Map<string, Buffer>} */
    this.staged = new Map();
    /** @type {Set<string>} */
    this.removed = new Set();
    /** @type {Set<string>} File paths whose stored integrity record is kept verbatim. */
    this.frozenIntegrity = new Set();
  }

  /**
   * Opens an archive and parses its header.
   * @param {string} archivePath Absolute path of the `.asar` file.
   * @return {Asar} Parsed archive handle.
   */
  static open(archivePath) {
    const asar = new Asar(archivePath);
    asar.reload();
    return asar;
  }

  /**
   * Reads the pickle header and flattens the file tree.
   * @return {void}
   */
  reload() {
    const prefix = Buffer.alloc(HEADER_PREFIX_BYTES);
    const fd = fs.openSync(this.path, 'r');
    try {
      fs.readSync(fd, prefix, 0, HEADER_PREFIX_BYTES, 0);
      this.fileSize = fs.fstatSync(fd).size;
    } finally {
      fs.closeSync(fd);
    }

    const jsonLength = prefix.readUInt32LE(12);
    if (prefix.readUInt32LE(0) !== 4) {
      throw new Error(`not an asar archive (bad outer pickle): ${this.path}`);
    }
    if (
      prefix.readUInt32LE(4) !== jsonLength + 8 ||
      prefix.readUInt32LE(8) !== jsonLength + 4
    ) {
      throw new Error(`unexpected asar pickle frame in ${this.path}`);
    }

    const json = Buffer.alloc(jsonLength);
    const reader = fs.openSync(this.path, 'r');
    try {
      fs.readSync(reader, json, 0, jsonLength, HEADER_PREFIX_BYTES);
    } finally {
      fs.closeSync(reader);
    }

    this.dataStart = HEADER_PREFIX_BYTES + jsonLength;
    this.header = JSON.parse(json.toString('utf8'));

    const flat = new Map();
    /**
     * @param {object} node Directory node.
     * @param {string} prefixParts Path accumulated so far.
     * @return {void}
     */
    const walk = (node, prefixParts) => {
      for (const [name, entry] of Object.entries(node.files || {})) {
        const rel = prefixParts ? `${prefixParts}/${name}` : name;
        if (entry && entry.files) {
          walk(entry, rel);
        } else {
          flat.set(rel, entry);
        }
      }
    };
    walk(this.header, '');
    this.files = flat;
  }

  /**
   * @return {string[]} Every file path inside the archive, sorted.
   */
  listPaths() {
    return Array.from(this.files.keys()).sort();
  }

  /**
   * Reads one payload.
   * @param {string} rel Slash separated path inside the archive.
   * @return {Buffer} Payload bytes.
   */
  readFile(rel) {
    if (this.staged.has(rel)) {
      return /** @type {Buffer} */ (this.staged.get(rel));
    }
    const leaf = this.files.get(rel);
    if (!leaf) {
      throw new Error(`no such file in archive: ${rel}`);
    }
    if (leaf.unpacked) {
      return fs.readFileSync(path.join(`${this.path}.unpacked`, ...rel.split('/')));
    }
    const out = Buffer.alloc(Number(leaf.size));
    const fd = fs.openSync(this.path, 'r');
    try {
      const read = fs.readSync(fd, out, 0, out.length, this.dataStart + Number(leaf.offset));
      if (read !== out.length) {
        throw new Error(`short read for ${rel}: ${read}/${out.length}`);
      }
    } finally {
      fs.closeSync(fd);
    }
    return out;
  }

  /**
   * Stages an insert or update. Nothing touches the file until save().
   * @param {string} rel Slash separated path inside the archive.
   * @param {Buffer|string} content New payload.
   * @return {void}
   */
  addFile(rel, content) {
    this.removed.delete(rel);
    this.staged.set(rel, Buffer.isBuffer(content) ? content : Buffer.from(content, 'utf8'));
  }

  /**
   * Stages a deletion.
   * @param {string} rel Slash separated path inside the archive.
   * @return {void}
   */
  removeFile(rel) {
    this.removed.add(rel);
    this.staged.delete(rel);
  }

  /**
   * Writes a file node into a (possibly brand new) tree, creating parents.
   * @param {object} root Header document.
   * @param {string} rel Slash separated path inside the archive.
   * @param {FileNode} leaf Metadata to store.
   * @return {void}
   */
  static setLeaf(root, rel, leaf) {
    const parts = rel.split('/');
    let cursor = root;
    cursor.files = cursor.files || {};
    for (let i = 0; i < parts.length - 1; i += 1) {
      const next = cursor.files[parts[i]] || {};
      next.files = next.files || {};
      cursor.files[parts[i]] = next;
      cursor = next;
    }
    cursor.files[parts[parts.length - 1]] = leaf;
  }

  /**
   * Serialises a header document into its pickle framing.
   * @param {object} doc Header document.
   * @return {Buffer} Ready-to-write header bytes.
   */
  static packHeader(doc) {
    const json = Buffer.from(JSON.stringify(doc), 'utf8');
    const prefix = Buffer.alloc(HEADER_PREFIX_BYTES);
    prefix.writeUInt32LE(4, 0);
    prefix.writeUInt32LE(json.length + 8, 4);
    prefix.writeUInt32LE(json.length + 4, 8);
    prefix.writeUInt32LE(json.length, 12);
    return Buffer.concat([prefix, json]);
  }

  /**
   * Writes the archive back to disk atomically (temp file + rename).
   *
   * Untouched payloads are streamed from the source archive. Per-file integrity
   * records are recomputed for everything, so archives packed with
   * `--integrity` stay self-consistent.
   *
   * @param {string=} outPath Target path; defaults to an in-place rewrite.
   * @return {{bytes: number, files: number}} Result summary.
   */
  save(outPath = this.path) {
    const target = outPath;
    const tmpPath = `${target}.pla-tmp`;
    if (fs.existsSync(tmpPath)) {
      fs.unlinkSync(tmpPath);
    }

    const survivors = this.listPaths().filter((rel) => !this.removed.has(rel));

    /** @type {Array<{rel: string, leaf: (FileNode|null), buffer: (Buffer|null), size: number, offset: number}>} */
    const plan = [];
    let cursor = 0;
    survivors.forEach((rel) => {
      const leaf = /** @type {FileNode} */ (this.files.get(rel)) || null;
      const override = this.staged.get(rel) || null;
      const isUnpacked = Boolean(leaf && leaf.unpacked) && !override;
      const size = override ? override.length : Number(leaf.size);
      if (!isUnpacked) {
        plan.push({ rel, leaf, buffer: override, size, offset: cursor });
        cursor += size;
      } else {
        plan.push({ rel, leaf, buffer: null, size, offset: -1 });
      }
    });

    const headerDoc = { files: {} };
    const out = fs.openSync(tmpPath, 'w');
    let bytesWritten = 0;
    try {
      // Stage 1: materialise every payload we have to write.
      const payloads = new Map();
      for (const item of plan) {
        if (item.offset < 0) {
          continue; // unpacked: stays outside the archive blob
        }
        if (item.buffer) {
          payloads.set(item.rel, item.buffer);
          continue;
        }
        const payload = Buffer.alloc(item.size);
        const source = fs.openSync(this.path, 'r');
        try {
          const read = fs.readSync(
            source,
            payload,
            0,
            item.size,
            this.dataStart + Number(/** @type {FileNode} */ (item.leaf).offset)
          );
          if (read !== item.size) {
            throw new Error(`short read while repacking ${item.rel}`);
          }
        } finally {
          fs.closeSync(source);
        }
        payloads.set(item.rel, payload);
      }

      // Stage 2: build the new header (offsets + integrity).
      for (const item of plan) {
        const payload = payloads.get(item.rel);
        const leaf = /** @type {FileNode} */ ({ size: item.size, offset: String(item.offset) });
        if (item.leaf && item.leaf.unpacked) {
          leaf.unpacked = true;
          leaf.offset = item.leaf.offset;
        }
        const oldIntegrity = item.leaf ? item.leaf.integrity : null;
        if (payload && !this.frozenIntegrity.has(item.rel)) {
          leaf.integrity = computeIntegrity(payload, Array.isArray(oldIntegrity.blocks));
        } else if (oldIntegrity) {
          // Untouched unpacked payloads live outside the blob; keep their
          // integrity record verbatim instead of reading them back.
          leaf.integrity = oldIntegrity;
        }
        Asar.setLeaf(headerDoc, item.rel, leaf);
      }

      const header = Asar.packHeader(headerDoc);
      fs.writeSync(out, header);
      bytesWritten += header.length;

      for (const item of plan) {
        const payload = payloads.get(item.rel);
        if (!payload) {
          continue;
        }
        fs.writeSync(out, payload);
        bytesWritten += payload.length;
      }
    } catch (error) {
      fs.closeSync(out);
      fs.unlinkSync(tmpPath);
      throw error;
    }
    fs.closeSync(out);

    fs.renameSync(tmpPath, target);
    this.staged.clear();
    this.removed.clear();
    this.reload();
    return { bytes: bytesWritten, files: plan.length };
  }
}

/**
 * CLI entry point.
 * @param {string[]} argv Raw argument slice.
 * @return {number} Exit code.
 */
function main(argv) {
  const [archivePath, command, ...rest] = argv;
  if (!archivePath || !command) {
    process.stderr.write('usage: node pla-asar.js <asar> info|list|extract|replace|remove [...]\n');
    return 2;
  }
  const asar = Asar.open(archivePath);

  switch (command) {
    case 'info': {
      let unpacked = 0;
      let integrity = 0;
      let packedBytes = 0;
      for (const rel of asar.listPaths()) {
        const leaf = /** @type {FileNode} */ (asar.files.get(rel));
        if (leaf.unpacked) {
          unpacked += 1;
        } else {
          packedBytes += Number(leaf.size);
        }
        if (leaf.integrity) {
          integrity += 1;
        }
      }
      process.stdout.write(
        [
          `archive       : ${archivePath}`,
          `file size     : ${asar.fileSize}`,
          `header bytes  : ${asar.dataStart - HEADER_PREFIX_BYTES}`,
          `data start    : ${asar.dataStart}`,
          `entries       : ${asar.files.size} (unpacked ${unpacked}, integrity ${integrity})`,
          `packed bytes  : ${packedBytes}`,
          `payload end   : ${asar.dataStart + packedBytes}`,
          '',
        ].join('\n')
      );
      return 0;
    }
    case 'list': {
      const prefix = rest[0] || '';
      for (const rel of asar.listPaths()) {
        if (!prefix || rel.startsWith(prefix)) {
          const leaf = /** @type {FileNode} */ (asar.files.get(rel));
          process.stdout.write(
            `${String(leaf.size).padStart(12)}  ${rel}${leaf.unpacked ? '  [unpacked]' : ''}\n`
          );
        }
      }
      return 0;
    }
    case 'extract': {
      const [inner, outFile] = rest;
      const buffer = asar.readFile(inner);
      fs.mkdirSync(path.dirname(outFile), { recursive: true });
      fs.writeFileSync(outFile, buffer);
      process.stdout.write(`wrote ${outFile} (${buffer.length} bytes)\n`);
      return 0;
    }
    case 'replace': {
      const [inner, inFile] = rest;
      const backup = `${archivePath}.pla-backup`;
      if (rest.includes('--backup') && !fs.existsSync(backup)) {
        fs.copyFileSync(archivePath, backup);
        process.stdout.write(`backup -> ${backup}\n`);
      }
      asar.addFile(inner, fs.readFileSync(inFile));
      const result = asar.save();
      process.stdout.write(`patched ${inner}; ${result.files} entries, ${result.bytes} bytes\n`);
      return 0;
    }
    case 'remove': {
      const [inner] = rest;
      const backup = `${archivePath}.pla-backup`;
      if (rest.includes('--backup') && !fs.existsSync(backup)) {
        fs.copyFileSync(archivePath, backup);
        process.stdout.write(`backup -> ${backup}\n`);
      }
      asar.removeFile(inner);
      const result = asar.save();
      process.stdout.write(`removed ${inner}; ${result.files} entries, ${result.bytes} bytes\n`);
      return 0;
    }
    default:
      process.stderr.write(`unknown command: ${command}\n`);
      return 2;
  }
}

module.exports = { Asar, computeIntegrity, HEADER_PREFIX_BYTES };

if (require.main === module) {
  process.exitCode = main(process.argv.slice(2));
}
