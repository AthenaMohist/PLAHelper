/**
 * PLAhelper 后台 Plasticity 的主进程钩子(用户 2026-11-18: 后台起一个独立的 Plasticity 做独立任务, 不能闪现、不能抢鼠标 / 焦点)。
 * 由 pla-bg.js 在 `--inspect-brk` 等调试器的那一刻(主脚本还没跑)用 Runtime.evaluate 注入, 之后才 runIfWaitingForDebugger 放行。
 * (Electron 的 --inspect-brk 不会真的停在第一行, 只是等调试器 —— 实测 2026-11-18; 这时没有 require, 用 process.getBuiltinModule。)
 * 钩的是 Module._load: 第一次有人 require('electron') 时换成打过补丁的版本; userData 不是 globalThis.__PLA_BG_EXPECT__ 就立刻退出
 * (在 Plasticity 申请单实例锁之前, 免得转给前台那个)。
 *
 * - 所有 BrowserWindow: show:false 创建、不进任务栏、不可聚焦、不节流(backgroundThrottling:false → 不可见也照常渲染 / 跑 rAF);
 *   show / showInactive / focus / maximize / restore / moveTop / setFullScreen / flashFrame / setAlwaysOnTop 一律吞掉(只记日志)。
 * - 原生对话框不弹: 另存为 → 用 __PLA_BG__.savePath(一次性)否则取消; 打开 → 用 __PLA_BG__.openPaths 否则取消; 消息框 → 回 cancelId / 0; 错误框 → 只记。
 * - shell.openExternal / openPath 不执行。
 * 状态 / 日志在 globalThis.__PLA_BG__。本文件是一个表达式(返回 'hooked')。
 */
(() => {
  const Module = process.getBuiltinModule('module');
  const G = globalThis.__PLA_BG__ = globalThis.__PLA_BG__ || { log: [], windows: [], savePath: null, openPaths: null, blocked: 0 };
  const log = (what) => { G.log.push({ t: Date.now(), what: String(what).slice(0, 300) }); if (G.log.length > 300) G.log.shift(); };
  let patched = null;
  const origLoad = Module._load;
  Module._load = function (request) {
    const m = origLoad.apply(this, arguments);
    if (request !== 'electron') return m;
    if (!patched) patched = patch(m);
    return patched;
  };
  G.hooked = Date.now();
  log('hooked');
  return 'hooked';

  function patch(electron) {
  const want = String(globalThis.__PLA_BG_EXPECT__ || '').toLowerCase();
  const ud = String(electron.app.getPath('userData')).toLowerCase();
  if (!want || ud !== want) { log('userData mismatch ' + ud); process.exit(3); }
  const Orig = electron.BrowserWindow;
  const NOOP = ['show', 'showInactive', 'focus', 'maximize', 'restore', 'moveTop', 'setFullScreen', 'setKiosk', 'flashFrame', 'setAlwaysOnTop', 'center'];
  class BgWindow extends Orig {
    constructor(opts) {
      const o = Object.assign({}, opts || {});
      o.show = false;
      o.skipTaskbar = true;
      o.focusable = false;
      o.webPreferences = Object.assign({}, o.webPreferences || {}, { backgroundThrottling: false });
      super(o);
      try { this.setSkipTaskbar(true); } catch (e) {}
      G.windows.push({ id: this.id, at: Date.now() });
      log('window created ' + this.id);
      for (const m of NOOP) {
        this[m] = (...a) => { G.blocked++; log('blocked ' + m); return undefined; };
      }
      // 内容在不可见窗口里也要照常渲染
      try { this.webContents.setBackgroundThrottling(false); } catch (e) {}
      this.webContents.on('did-finish-load', () => { try { this.webContents.setBackgroundThrottling(false); } catch (e) {} log('loaded ' + this.id); });
    }
  }
  // 静态查找: 继承来的版本只认原类的实例(实测 getAllWindows 回空), 代理 / Plasticity 都要它 → 用 BaseWindow 的全集;
  // 窗口不可聚焦, getFocusedWindow 回第一个窗口(菜单 / 命令按"当前窗口"派发)
  const allWins = () => (electron.BaseWindow ? electron.BaseWindow.getAllWindows() : Orig.getAllWindows()).filter((w) => w && typeof w.webContents === 'object');
  BgWindow.getAllWindows = allWins;
  BgWindow.fromId = (id) => allWins().find((w) => w.id === id) || null;
  BgWindow.fromWebContents = (wc) => allWins().find((w) => w.webContents === wc) || Orig.fromWebContents(wc);
  BgWindow.getFocusedWindow = () => Orig.getFocusedWindow() || allWins()[0] || null;
  // Plasticity require('electron') 拿到的 BrowserWindow = BgWindow
  const ePatched = new Proxy(electron, {
    get(t, k) {
      if (k === 'BrowserWindow') return BgWindow;
      return Reflect.get(t, k);
    },
  });
  // 对话框
  const { dialog, shell } = electron;
  const pickSave = () => { const p = G.savePath; G.savePath = null; return p; };
  dialog.showSaveDialog = async (...a) => { const p = pickSave(); log('save dialog -> ' + (p || 'cancel')); return p ? { canceled: false, filePath: p } : { canceled: true, filePath: '' }; };
  dialog.showSaveDialogSync = (...a) => { const p = pickSave(); log('save dialog sync -> ' + (p || 'cancel')); return p || undefined; };
  dialog.showOpenDialog = async () => { const p = G.openPaths; G.openPaths = null; log('open dialog -> ' + (p ? p.join(';') : 'cancel')); return p ? { canceled: false, filePaths: p } : { canceled: true, filePaths: [] }; };
  dialog.showOpenDialogSync = () => { const p = G.openPaths; G.openPaths = null; log('open dialog sync -> ' + (p ? p.join(';') : 'cancel')); return p || undefined; };
  dialog.showMessageBox = async (win, opts) => { const o = (win && win.buttons !== undefined) || (win && win.message) ? win : (opts || {}); const r = typeof o.cancelId === 'number' ? o.cancelId : 0; log('message box: ' + (o.message || '') + ' -> ' + r); return { response: r, checkboxChecked: false }; };
  dialog.showMessageBoxSync = (win, opts) => { const o = (win && win.message) ? win : (opts || {}); const r = typeof o.cancelId === 'number' ? o.cancelId : 0; log('message box sync: ' + (o.message || '') + ' -> ' + r); return r; };
  dialog.showErrorBox = (title, content) => { log('error box: ' + title + ' / ' + content); };
  shell.openExternal = async (u) => { log('openExternal blocked ' + u); };
  shell.openPath = async (p) => { log('openPath blocked ' + p); return ''; };
  log('electron patched (userData ok)');
  return ePatched;
  }
})()
