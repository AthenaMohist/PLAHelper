/**
 * @file PLA-CDP installer v7: detect, patch, restore, done safely.
 *
 * Why a second tool next to `patch-cdp.bat` / `pla-cdp-patch.js`:
 * those only understand the *legacy* layout (plaintext main entry). Starting
 * with 26.2.3 Plasticity ships an encrypted main bundle (see
 * `docs/know-how/plasticity-2623-hardening.md`) that this project cannot patch.
 * Running the old tool against such an install silently does nothing, which is
 * exactly the kind of failure this project has vowed to never hide; this tool
 * inspects first and refuses loudly instead.
 *
 * Layouts understood:
 *   legacy    - `<root>/resources/app/.webpack/main/index.js` is plaintext
 *               (<= 26.2.1). Patchable by prepending the v6 fragment.
 *   hardened  - `<root>/resources/app.asar` + `<root>/resources/plasticity.jsc.pack`
 *               (>= 26.2.3). NOT patchable: main bundle is encrypted AND the app
 *               verifies every file inside app.asar, quitting immediately on any
 *               content change.
 *   unknown   - anything else, reported but never touched.
 *
 * The tool never modifies a `hardened` install. It is ASCII-only on purpose:
 * Windows consoles default to GBK and would mangle UTF-8 Chinese output.
 *
 * Usage:
 *   node pla-cdp-v7.js [status|patch|restore] [--root <dir>]
 */

'use strict';

const fs = require('fs');
const path = require('path');
const os = require('os');

/** Marker written by the v6 patch fragment; used for idempotency checks. */
const PATCH_MARKER = 'pla-cdp-patch';

/** Relative path of the legacy main entry inside an install root. */
const LEGACY_MAIN = ['resources', 'app', '.webpack', 'main', 'index.js'];

/** v6 patch fragment lives next to this file. */
const PATCH_FRAGMENT = 'pla-cdp-patch.js';

/**
 * Candidate installation roots, most likely first.
 * @return {string[]} Absolute directory paths to scan.
 */
function candidateRoots() {
  const roots = [];
  const env = process.env.PLA_PLASTICITY_ROOT;
  if (env) {
    roots.push(env);
  }
  const home = os.homedir();
  if (home) {
    roots.push(path.join(home, 'Programs'));
    roots.push(path.join(home, 'Programs', 'Plasticity'));
  }
  roots.push(path.join(process.env.LOCALAPPDATA || '', 'Programs', 'Plasticity'));
  roots.push(path.join(process.env.PROGRAMFILES || 'C:\\Program Files', 'Plasticity'));
  roots.push(path.join(process.env['PROGRAMFILES(X86)'] || '', 'Plasticity'));
  for (const drive of ['C', 'D', 'E', 'F', 'G']) {
    roots.push(`${drive}:\\Program Files\\Plasticity`);
    roots.push(`${drive}:\\Plasticity`);
  }
  return roots.filter((value, index, all) => value && all.indexOf(value) === index);
}

/**
 * Enumerates installation directories below the candidate roots.
 * @return {string[]} Absolute paths of install roots (e.g. `...\\Plasticity\\app-26.1.3`).
 */
function installDirs() {
  const found = [];
  for (const root of candidateRoots()) {
    if (!fs.existsSync(root)) {
      continue;
    }
    found.push(root);
    let entries = [];
    try {
      entries = fs.readdirSync(root, { withFileTypes: true });
    } catch (error) {
      continue;
    }
    for (const entry of entries) {
      if (entry.isDirectory() && /^app-/.test(entry.name)) {
        found.push(path.join(root, entry.name));
      }
    }
  }
  return found.filter((value, index, all) => all.indexOf(value) === index);
}

/**
 * Classifies one installation directory.
 * @param {string} dir Absolute install path.
 * @return {{dir: string, layout: string, mainPath: (string|null), patched: boolean, backupPath: (string|null), version: string}} Report row.
 */
function inspectInstall(dir) {
  const legacyMain = path.join(dir, ...LEGACY_MAIN);
  const asar = path.join(dir, 'resources', 'app.asar');
  const pack = path.join(dir, 'resources', 'plasticity.jsc.pack');
  const row = {
    dir,
    layout: 'unknown',
    mainPath: null,
    patched: false,
    backupPath: null,
    version: '',
  };

  const packageJson = path.join(dir, 'resources', 'app', 'package.json');
  if (fs.existsSync(packageJson)) {
    try {
      row.version = JSON.parse(fs.readFileSync(packageJson, 'utf8')).version || '';
    } catch (error) {
      row.version = '';
    }
  }

  if (fs.existsSync(legacyMain)) {
    row.layout = 'legacy';
    row.mainPath = legacyMain;
    row.backupPath = `${legacyMain}.pla-backup`;
    try {
      row.patched = fs.readFileSync(legacyMain, 'utf8').includes(PATCH_MARKER);
    } catch (error) {
      row.patched = false;
    }
    return row;
  }

  if (fs.existsSync(asar) && fs.existsSync(pack)) {
    row.layout = 'hardened';
    try {
      const { Asar } = require('./pla-asar.js');
      row.version = JSON.parse(Asar.open(asar).readFile('package.json').toString('utf8')).version || '';
    } catch (error) {
      /* keep the version empty: reading the archive is a best effort nicety */
    }
  }
  return row;
}

/**
 * Best effort probe of the CDP HTTP endpoint.
 * @return {string} Human readable status line.
 */
function probeCdpPort() {
  try {
    const out = require('child_process').execFileSync(
      'netstat',
      ['-ano'],
      { encoding: 'utf8', windowsHide: true }
    );
    const line = out.split(/\r?\n/).find((value) => value.includes(':9333') && value.includes('LISTENING'));
    return line ? `CDP port 9333 is LISTENING (${line.trim().split(/\s+/).pop()})` : 'CDP port 9333 is closed';
  } catch (error) {
    return 'CDP port 9333 status unknown';
  }
}

/**
 * @return {string[]} Paths of Plasticity processes currently running, if any.
 */
function runningInstances() {
  const names = ['Plasticity.exe'];
  const seen = [];
  for (const name of names) {
    try {
      const output = require('child_process')
        .execFileSync('tasklist', ['/FI', `IMAGENAME eq ${name}`], { encoding: 'utf8' });
      if (output && output.toLowerCase().includes(name.toLowerCase().replace('.exe', ''))) {
        seen.push(name);
      }
    } catch (error) {
      /* tasklist is unavailable outside Windows; harmless here */
    }
  }
  return seen;
}

/**
 * Prints the scan result for every install found.
 * @param {Array<ReturnType<inspectInstall>>} rows Inspection rows.
 * @return {number} Number of patchable installs still needing the patch.
 */
function printReport(rows) {
  let todo = 0;
  process.stdout.write('\nPLA-CDP installer v7 - Plasticity installation scan\n');
  process.stdout.write('='.repeat(64) + '\n');
  for (const row of rows) {
    process.stdout.write(`\n${row.dir}\n`);
    process.stdout.write(`  version : ${row.version || '(unknown)'}\n`);
    process.stdout.write(`  layout  : ${row.layout}\n`);
    if (row.layout === 'legacy') {
      process.stdout.write(`  entry   : ${row.mainPath}\n`);
      process.stdout.write(`  patched : ${row.patched ? 'YES' : 'NO'}\n`);
      if (!row.patched) {
        todo += 1;
      }
    } else if (row.layout === 'hardened') {
      process.stdout.write('  status  : NOT PATCHABLE\n');
      process.stdout.write('  reason  : main bundle is encrypted (plasticity.jsc.pack) and the app\n');
      process.stdout.write('            verifies every file inside app.asar; any content change makes\n');
      process.stdout.write('            it quit right after "App ready". See\n');
      process.stdout.write('            docs/know-how/plasticity-2623-hardening.md\n');
      process.stdout.write('  action  : keep using a legacy install, or ask upstream for a debug flag.\n');
    } else {
      process.stdout.write('  status  : unrecognised layout, left untouched\n');
    }
  }
  process.stdout.write('\n' + '='.repeat(64) + '\n');
  process.stdout.write(`${probeCdpPort()}\n`);
  return todo;
}

/**
 * Applies the v6 patch to legacy installs.
 * @param {Array<ReturnType<inspectInstall>>} rows Inspection rows.
 * @return {number} Number of installs patched by this run.
 */
function applyPatch(rows) {
  const fragmentPath = path.join(__dirname, PATCH_FRAGMENT);
  if (!fs.existsSync(fragmentPath)) {
    process.stdout.write(`ERROR: missing ${PATCH_FRAGMENT} next to this tool\n`);
    return -1;
  }
  const fragment = fs.readFileSync(fragmentPath, 'utf8');
  let done = 0;
  for (const row of rows) {
    if (row.layout !== 'legacy') {
      continue;
    }
    if (row.patched) {
      process.stdout.write(`skip  (already patched): ${row.mainPath}\n`);
      continue;
    }
    if (!row.backupPath) {
      continue;
    }
    if (!fs.existsSync(row.backupPath)) {
      fs.copyFileSync(row.mainPath, row.backupPath);
      process.stdout.write(`backup: ${row.backupPath}\n`);
    }
    const original = fs.readFileSync(row.backupPath, 'utf8');
    const merged = `${fragment}\n${original}`;
    const tmp = `${row.mainPath}.pla-tmp`;
    fs.writeFileSync(tmp, merged, 'utf8');
    fs.renameSync(tmp, row.mainPath);
    const verified = fs.readFileSync(row.mainPath, 'utf8').includes(PATCH_MARKER);
    if (!verified) {
      fs.copyFileSync(row.backupPath, row.mainPath);
      process.stdout.write(`FAILED verification, restored: ${row.mainPath}\n`);
      continue;
    }
    process.stdout.write(`patched: ${row.mainPath}\n`);
    done += 1;
  }
  return done;
}

/**
 * Restores every legacy install from its backup.
 * @param {Array<ReturnType<inspectInstall>>} rows Inspection rows.
 * @return {number} Number of installs restored.
 */
function restore(rows) {
  let done = 0;
  for (const row of rows) {
    if (row.layout !== 'legacy' || !row.backupPath || !fs.existsSync(row.backupPath)) {
      continue;
    }
    fs.copyFileSync(row.backupPath, row.mainPath);
    process.stdout.write(`restored: ${row.mainPath}\n`);
    done += 1;
  }
  return done;
}

/**
 * Entry point.
 * @return {number} Process exit code (0 ok, 2 usage error, 3 nothing to do).
 */
function main() {
  const argv = process.argv.slice(2);
  const command = (argv.find((value) => !value.startsWith('--')) || 'status').toLowerCase();

  const rootsOverride = argv.includes('--root')
    ? argv[argv.indexOf('--root') + 1]
    : null;
  const dirs = rootsOverride && fs.existsSync(rootsOverride) ? [rootsOverride] : installDirs();
  if (dirs.length === 0) {
    process.stdout.write('No Plasticity installation found.\n');
    return 3;
  }

  const rows = dirs.map(inspectInstall);
  const todo = printReport(rows);

  const busy = runningInstances();
  if (busy.length > 0) {
    process.stdout.write(
      `\nNOTE: Plasticity is running (${busy.join(', ')}). A patch only takes\n` +
        '      effect after a full restart; close the app before patching.\n'
    );
  }

  if (command === 'patch') {
    const done = applyPatch(rows);
    if (done < 0) {
      return 2;
    }
    if (done === 0) {
      process.stdout.write('\nNothing to patch.\n');
    } else {
      process.stdout.write(`\nPatched ${done} install(s). Restart Plasticity, then check\n`);
      process.stdout.write('  http://127.0.0.1:9333/json/version\n');
    }
    return 0;
  }

  if (command === 'restore') {
    const done = restore(rows);
    process.stdout.write(`\nRestored ${done} install(s).\n`);
    return 0;
  }

  if (todo > 0) {
    process.stdout.write(`\n${todo} install(s) ready for patching. Run:\n`);
    process.stdout.write(`  node ${path.basename(__filename)} patch\n`);
  }
  return 0;
}

if (require.main === module) {
  process.exitCode = main();
}

module.exports = { candidateRoots, installDirs, inspectInstall };
