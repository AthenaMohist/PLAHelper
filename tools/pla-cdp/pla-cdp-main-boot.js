/**
 * @file PLA-CDP main-process bootstrap for Plasticity >= 26.2 (bytenode main).
 *
 * Why this file exists
 * --------------------
 * Up to 26.1.3 Plasticity's main process ran from a *plaintext* loader
 * `resources/app/.webpack/main/index.js`, so PLAhelper simply prepended its CDP
 * patch there (v6). 26.2.3 changed everything:
 *
 *   - the app is packed as `resources/app.asar` (no `resources/app` folder),
 *   - the main bundle is NOT inside the archive anymore but sits next to it as
 *     `resources/plasticity.jsc.pack`, wrapped in Plasticity's own `PLSEC001`
 *     container (V8 bytecode + their protection layer),
 *   - `resources/app.asar/.webpack/main/index.js` degraded to a 14 byte stub
 *     (`'use strict';`) - i.e. the launcher no longer relies on it,
 *   - the Electron fuses shipped with the app have
 *     `RunAsNode`, `EnableNodeOptionsEnvVar` and `EnableNodeCliInspectArguments`
 *     all flipped OFF, so neither `ELECTRON_RUN_AS_NODE`, `NODE_OPTIONS` nor
 *     `--inspect` can be used to inject anything,
 *   - the app itself still calls `app.commandLine.removeSwitch()` for both
 *     `remote-debugging-port` and `remote-debugging-pipe`.
 *
 * The only remaining injection point is therefore the archive stub itself: the
 * patcher replaces it with THIS file, which
 *
 *   1. installs the CDP port (port 9333, before any application code runs),
 *   2. installs the `.jsc` / `.pack` bytecode loader used by the app itself,
 *   3. hands execution over to the untouched `plasticity.jsc.pack`.
 *
 * Nothing inside the application bundle is modified; if the original bundle
 * cannot be found the app starts headless-but-alive and says so in the log
 * instead of silently dying.
 *
 * Note this file is ASCII-only on purpose - it is copied verbatim into the
 * application resources on machines with non-UTF8 locales.
 */

'use strict';

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const vm = require('node:vm');
const v8 = require('node:v8');
const Module = require('node:module');

/** Default CDP port; override with env `PLA_CDP_PORT`. */
const DEFAULT_PORT = '9333';

/** Where the diagnostics go. Override with env `PLA_CDP_LOG`. */
const LOG_PATH =
  process.env.PLA_CDP_LOG || path.join(os.homedir(), '.pla-cdp-patch.log');

/**
 * Appends one line to the diagnostics log. Never throws: logging must not be
 * able to brick the host application.
 * @param {string} message Human readable message.
 * @return {void}
 */
function log(message) {
  try {
    fs.appendFileSync(
      LOG_PATH,
      `${new Date().toISOString().slice(11, 23)} ${message}\n`
    );
  } catch (error) {
    /* swallow: the log location may be unwritable on locked-down machines */
  }
}

/**
 * Installs the Chrome DevTools port and neutralises the application's own
 * switch stripping. Must run before any application code.
 * @return {boolean} True when the switch (or an external one) is armed.
 */
function installRemoteDebugging() {
  let electron = null;
  try {
    electron = require('electron');
  } catch (error) {
    log(`CDP-FATAL cannot require('electron'): ${error && error.message}`);
    return false;
  }
  const app = electron && electron.app;
  if (!app || !app.commandLine) {
    log('CDP-FATAL no app.commandLine (not running in the main process?)');
    return false;
  }

  const commandLine = app.commandLine;
  const originalRemove = commandLine.removeSwitch.bind(commandLine);
  const hasSwitch = commandLine.hasSwitch.bind(commandLine);
  const external =
    hasSwitch('remote-debugging-port') || hasSwitch('remote-debugging-pipe');

  // The app strips both debugging switches at startup; ignore those two calls
  // only, everything else keeps the original behaviour.
  commandLine.removeSwitch = function plaRemoveSwitch(name, value) {
    if (name === 'remote-debugging-port' || name === 'remote-debugging-pipe') {
      log(`CDP blocked removeSwitch(${name})`);
      return undefined;
    }
    return arguments.length > 1 ? originalRemove(name, value) : originalRemove(name);
  };

  if (!external) {
    const port = process.env.PLA_CDP_PORT || DEFAULT_PORT;
    commandLine.appendSwitch('remote-debugging-port', port);
    log(`CDP armed port=${port}`);
  } else {
    log('CDP external debugging switch already present; left untouched');
  }

  if (process.env.PLA_CDP_INSTANCE) {
    app.requestSingleInstanceLock = () => true;
    app.setPath(
      'userData',
      process.env.PLA_CDP_PROFILE || path.join(os.homedir(), '.plasticity-cdp-profile')
    );
    log('CDP multi-instance profile armed');
  }
  return true;
}

/**
 * Registers bytecode loaders mirroring what the application itself uses, so the
 * original bundle can be handed over without re-implementing their container
 * format. `.jsc` is the renderer/legacy extension, `.pack` is the main bundle.
 * @return {void}
 */
function installBytecodeLoader() {
  v8.setFlagsFromString('--no-lazy');
  v8.setFlagsFromString('--no-flush-bytecode');

  /**
   * CommonJS handler for V8 cached-data modules.
   * @param {object} module Node module object.
   * @param {string} filename Absolute path of the bytecode file.
   * @return {void}
   */
  const loader = (module, filename) => {
    const cachedData = fs.readFileSync(filename);
    log(
      `BUNDLE loading ${filename} bytes=${cachedData.length} ` +
        `head=${cachedData.subarray(0, 8).toString('hex')}`
    );
    const script = new vm.Script('', { cachedData, filename });
    if (script.cachedDataRejected) {
      throw new Error(`cachedDataRejected for ${filename}`);
    }
    const wrapper = script.runInThisContext();
    wrapper.call(
      module.exports,
      module.exports,
      Module.createRequire(filename),
      module,
      filename,
      path.dirname(filename),
      process,
      global
    );
  };

  Module._extensions['.jsc'] = loader;
  Module._extensions['.pack'] = loader;
}

/**
 * Candidate locations of the untouched application bundle, most likely first.
 * @return {string[]} Absolute paths to try.
 */
function bundleCandidates() {
  const candidates = [];
  if (process.env.PLA_ORIGINAL_PACK) {
    candidates.push(process.env.PLA_ORIGINAL_PACK);
  }
  // Copies planted next to this bootstrap inside app.asar: file reads that go
  // through Electron's archive layer are the ones the app's loader uses, so
  // they are the best bet for getting the container unwrapped.
  candidates.push(path.join(__dirname, 'plasticity.pla-original.jsc'));
  candidates.push(path.join(__dirname, 'plasticity.jsc.pack.pla-original'));
  if (process.resourcesPath) {
    candidates.push(path.join(process.resourcesPath, 'plasticity.jsc.pack.pla-original'));
    candidates.push(path.join(process.resourcesPath, 'plasticity.jsc.pack'));
  }
  return candidates;
}

/**
 * Loads the first bundle that exists and can be executed.
 * @return {boolean} True when the bundle took over.
 */
function handOverToBundle() {
  for (const candidate of bundleCandidates()) {
    if (!fs.existsSync(candidate)) {
      continue;
    }
    try {
      require(candidate);
      log(`BUNDLE handed over via ${candidate}`);
      return true;
    } catch (error) {
      log(`BUNDLE-ERR ${candidate}: ${(error && error.stack) || error}`);
    }
  }
  return false;
}

/**
 * Entry point.
 * @return {void}
 */
function main() {
  log(
    `=== PLA-CDP boot enter pid=${process.pid} electron=${process.versions.electron} ` +
      `node=${process.versions.node} resources=${process.resourcesPath}`
  );
  process.on('exit', (code) => log(`EXIT code=${code}`));
  process.on('uncaughtException', (error) =>
    log(`UNCAUGHT: ${(error && error.stack) || error}`)
  );
  process.on('unhandledRejection', (reason) =>
    log(`UNHANDLED: ${String(reason)}`)
  );

  installRemoteDebugging();
  installBytecodeLoader();

  if (!handOverToBundle()) {
    log('BUNDLE-FATAL no executable application bundle found; app stays alive');
  }
}

try {
  main();
} catch (error) {
  log(`BOOT-FATAL ${(error && error.stack) || error}`);
}

module.exports = { installRemoteDebugging, installBytecodeLoader, bundleCandidates };
