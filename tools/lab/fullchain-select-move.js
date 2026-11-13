// 完整链路(demo1, 用户已备份): 选择 → 截图 → 真实按键 G / 5 / Enter 移动面 → 截图 → Ctrl+Z 撤销 → 截图。
// 用法: node x14_fullchain.js <hwnd> <synthetic|realclick> <输出目录>
'use strict';
const fs = require('fs');
const path = require('path');
const [hwnd, modeArg, outDir] = process.argv.slice(2);
const mode = modeArg || 'synthetic';
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 240000);
let ws; let id = 0; const pend = new Map();
const raw = (method, params) => new Promise((res) => { const i = ++id; pend.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
const send = async (method, params) => {
  const r = await raw('Runtime.evaluate', { expression: `__PLA_PROXY_SENDS[${JSON.stringify(hwnd)}](${JSON.stringify(method)}, ${JSON.stringify(params || {})})`, returnByValue: true, awaitPromise: true });
  const v = r.result && r.result.result;
  if (v && v.subtype === 'error') throw new Error('MAIN: ' + v.description);
  return v && v.value;
};
const page = async (expr) => {
  const v = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (v && v.exceptionDetails) throw new Error('PAGE: ' + JSON.stringify(v.exceptionDetails).slice(0, 800));
  return v && v.result && v.result.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function shot(name) {
  const r = await send('Page.captureScreenshot', { format: 'png' });
  const data = r && (r.data || (r.result && r.result.data));
  if (!data) { console.log('  截图失败', JSON.stringify(r).slice(0, 200)); return; }
  const f = path.join(outDir, `${mode}-${name}.png`);
  fs.writeFileSync(f, Buffer.from(data, 'base64'));
  console.log('  截图', f);
}
async function key(k, code, vk, text, modifiers) {
  const base = { key: k, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers: modifiers || 0 };
  await send('Input.dispatchKeyEvent', Object.assign({ type: text ? 'keyDown' : 'rawKeyDown', text: text || undefined, unmodifiedText: text || undefined }, base));
  await sleep(60);
  await send('Input.dispatchKeyEvent', Object.assign({ type: 'keyUp' }, base));
  await sleep(400);
}
const state = async () => JSON.parse(await page(`JSON.stringify((function(){ var ed = window.__PLA_EDITOR__; var s = ed.selection.selected; var ac = null; try { ac = ed.executor.activeCommand; } catch (e) {}
  return { faces: Array.from(s.faceIds).map(String), solids: Array.from(s.solidIds).map(String), active: ac && ac.constructor.name,
           undo: ed.history._undoStack.length, geo: ed.geo._version !== undefined ? ed.geo._version : ed.geo.version, cmdAttr: document.body.getAttribute('command') }; })())`));
const fmt = (s) => `面=${s.faces.length} 实体=${s.solids.length} 活动=${s.active} 撤销=${s.undo} 几何版本=${s.geo} body[command]=${s.cmdAttr}`;

// 选择: synthetic = 自建 SelectionCommand 子类一次完成; realclick = 真实鼠标点击一个面(按面包围盒中心投影)
const SELECT_SYNTH = `(function(){
  var ed = window.__PLA_EDITOR__;
  var Base = Object.getPrototypeOf(ed.commands.DeselectAllCommand.prototype).constructor;
  function faceOf(vid) { var t = ed.geo.lookupTopologyItemById(vid); var v = t && t.view; return v && typeof v.values === 'function' ? v.values().next().value : v; }
  function hitFor(face) { var box = face.getBoundingBox(); var V = box.min.constructor; var c = new V(); box.getCenter(c); return { distance: 1, point: c, intersectable: face, object: face, distance2d: 0 }; }
  class PlaApplySelection extends Base {
    constructor(editor, faces) { super(editor); this.faces = faces; }
    async execute() {
      var cs = this.editor.changeSelection; var sel = this.editor.selection.selected;
      this.faces.forEach(function (face, i) {
        cs.onClick([hitFor(face)], i === 0 ? 0 : 1, 0);
        if (Array.from(sel.faceIds).map(String).indexOf(String(face.versionId)) < 0) cs.onClick([hitFor(face)], 1, 0);
      });
    }
  }
  // 3 个实体, 每个取 2 个"最大"的面(移动效果明显)
  var vids = [];
  ed.geo.items.filter(function (it) { return ed.scene.visible.has(it.view) && it.view.faces; }).slice(0, 3).forEach(function (it) {
    var fg = it.view.faces; var b = fg.bboxes; var idx = [];
    for (var i = 0; i < fg.faceIds.length; i++) { var o = i * 6; idx.push([i, Math.max(b[o+3]-b[o], b[o+4]-b[o+1], b[o+5]-b[o+2])]); }
    idx.sort(function (a, c) { return c[1] - a[1]; });
    idx.slice(0, 6).forEach(function (p) { if (vids.length >= 6) return; vids.push(fg.parentId + 'f' + fg.faceIds[p[0]]); });
  });
  try { ed.executor.cancelActiveCommand(); } catch (e) {}
  var S = ed.selection.signals; var cmd = new PlaApplySelection(ed, vids.map(faceOf));
  var done = new Promise(function (res) { var f = function (c) { if (c === cmd) { S.commandEnded.remove(f); res('ended'); } }; S.commandEnded.add(f); setTimeout(function () { S.commandEnded.remove(f); res('timeout'); }, 5000); });
  ed.enqueue(cmd);
  return done.then(function (h) { return JSON.stringify({ how: h, vids: vids }); });
})()`;

const FIND_FACE = `JSON.stringify((function(){
  var ed = window.__PLA_EDITOR__; var vp; ed.viewports.forEach(function (v) { if (!vp) vp = v; });
  var cam = vp.camera; var V = cam.position.constructor; var r = document.querySelector('plasticity-viewport canvas').getBoundingClientRect();
  var view = ed.geo.items.filter(function (it) { return ed.scene.visible.has(it.view) && it.view.faces; })[0].view; var fg = view.faces; var b = fg.bboxes; var best = null;
  for (var i = 0; i < fg.faceIds.length; i++) { var o = i * 6; var size = Math.max(b[o+3]-b[o], b[o+4]-b[o+1], b[o+5]-b[o+2]);
    var c = new V((b[o]+b[o+3])/2, (b[o+1]+b[o+4])/2, (b[o+2]+b[o+5])/2).applyMatrix4(view.matrixWorld).project(cam);
    if (Math.abs(c.x) < 0.8 && Math.abs(c.y) < 0.8 && (!best || size > best.size)) best = { size: size, x: r.x + (c.x + 1) / 2 * r.width, y: r.y + (1 - c.y) / 2 * r.height }; }
  return best; })())`;

(async () => {
  const list = await (await fetch('http://127.0.0.1:9334/json/list')).json();
  ws = new WebSocket(list.find((t) => t.type === 'node').webSocketDebuggerUrl);
  ws.addEventListener('message', (e) => { const m = JSON.parse(e.data); if (m.id && pend.has(m.id)) { pend.get(m.id)(m); pend.delete(m.id); } });
  await new Promise((r) => ws.addEventListener('open', r));
  try {
    await page(`(function(){ var ed = window.__PLA_EDITOR__; try { ed.executor.cancelActiveCommand(); } catch (e) {} window.__PLA__.call('sel.clear'); return 1; })()`);
    await sleep(800);
    console.log('0 清空:', fmt(await state()), '| 可见实体', await page('window.__PLA_EDITOR__.geo.items.filter(function (it) { return window.__PLA_EDITOR__.scene.visible.has(it.view); }).length'));
    await shot('0-cleared');
    if (mode === 'synthetic') {
      console.log('1 选择(自建命令):', await page(SELECT_SYNTH));
    } else {
      const p = JSON.parse(await page(FIND_FACE));
      await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: p.x, y: p.y, button: 'none' }); await sleep(300);
      for (let k = 0; k < 2; k++) {
        await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: p.x, y: p.y, button: 'left', clickCount: 1 });
        await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: p.x, y: p.y, button: 'left', clickCount: 1 });
        await sleep(800);
      }
      console.log('1 选择(真实点击 x2 下钻) @', Math.round(p.x), Math.round(p.y));
    }
    await sleep(800);
    const s1 = await state();
    console.log('1 选择后:', fmt(s1), s1.faces.join(','));
    await shot('1-selected');
    // 真实按键: 先取消选择后自动挂上的后续命令, 再 G → 5 → Enter
    await page(`(function(){ try { window.__PLA_EDITOR__.executor.cancelActiveCommand(); } catch (e) {} return 1; })()`);
    await sleep(600);
    { const rc = JSON.parse(await page(`JSON.stringify((function(){ var r = document.querySelector('plasticity-viewport canvas').getBoundingClientRect(); return [r.x + r.width * 0.5, r.y + r.height * 0.85]; })())`)); await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: rc[0], y: rc[1], button: 'none' }); await sleep(400); }
    await key('g', 'KeyG', 71, 'g');
    await sleep(600);
    console.log('2 按 G 后:', fmt(await state()));
    await key('5', 'Digit5', 53, '5');
    console.log('3 输入 5 后:', fmt(await state()));
    await key('Enter', 'Enter', 13, '\r');
    await sleep(1200);
    const s4 = await state();
    console.log('4 回车后:', fmt(s4));
    await shot('2-moved');
    await key('z', 'KeyZ', 90, undefined, 2);
    await sleep(1500);
    const s5 = await state();
    console.log('5 Ctrl+Z 后:', fmt(s5));
    await shot('3-undone');
    console.log(`结论: 几何被移动=${s4.geo !== s1.geo} 撤销栈+${s4.undo - s1.undo} | 撤销后几何版本 ${s5.geo}(移动前 ${s1.geo})`);
  } catch (e) { console.log('ERR', e.message); }
  ws.close(); process.exit(0);
})();
