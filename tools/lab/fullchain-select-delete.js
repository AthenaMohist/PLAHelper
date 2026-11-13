// 完整链路(demo1, 已备份): 自建命令选面 → Push Face 用 D 输入 2mm → 回车 → 几何是否真的变了 → Ctrl+Z。
'use strict';
const fs = require('fs');
const path = require('path');
const [hwnd, outDir] = process.argv.slice(2);
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 200000);
let ws; let id = 0; const pend = new Map();
const raw = (method, params) => new Promise((res) => { const i = ++id; pend.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
const send = async (method, params) => {
  const r = await raw('Runtime.evaluate', { expression: `__PLA_PROXY_SENDS[${JSON.stringify(hwnd)}](${JSON.stringify(method)}, ${JSON.stringify(params || {})})`, returnByValue: true, awaitPromise: true });
  const v = r.result && r.result.result; if (v && v.subtype === 'error') throw new Error('MAIN: ' + v.description); return v && v.value;
};
const page = async (expr) => {
  const v = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (v && v.exceptionDetails) throw new Error('PAGE: ' + JSON.stringify(v.exceptionDetails).slice(0, 800));
  return v && v.result && v.result.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function shot(name) { const r = await send('Page.captureScreenshot', { format: 'png' }); const d = r && (r.data || (r.result && r.result.data)); if (d) { const f = path.join(outDir, `delete-${name}.png`); fs.writeFileSync(f, Buffer.from(d, 'base64')); console.log('  截图', f); } }
async function key(k, code, vk, text, modifiers) {
  const base = { key: k, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers: modifiers || 0 };
  await send('Input.dispatchKeyEvent', Object.assign({ type: text ? 'keyDown' : 'rawKeyDown', text: text || undefined, unmodifiedText: text || undefined }, base));
  await sleep(60); await send('Input.dispatchKeyEvent', Object.assign({ type: 'keyUp' }, base)); await sleep(500);
}
const state = async () => JSON.parse(await page(`JSON.stringify((function(){ var ed = window.__PLA_EDITOR__; var s = ed.selection.selected; var ac = null; try { ac = ed.executor.activeCommand; } catch (e) {}
  var dist = null; try { var el = Array.from(document.querySelectorAll('input')).map(function (i) { return i.value; }); dist = el.slice(0, 4); } catch (e) {}
  return { faces: s.faceIds.size !== undefined ? s.faceIds.size : Array.from(s.faceIds).length, active: ac && ac.constructor.name, undo: ed.history._undoStack.length, geo: ed.geo._version, inputs: dist }; })())`));
const fmt = (s) => `面=${s.faces} 活动=${s.active} 撤销=${s.undo} 几何版本=${s.geo} 输入框=${JSON.stringify(s.inputs)}`;

const SELECT = fs.readFileSync(path.join(__dirname, 'x14_fullchain.js'), 'utf8').match(/const SELECT_SYNTH = `([\s\S]*?)`;/)[1];

(async () => {
  const list = await (await fetch('http://127.0.0.1:9334/json/list')).json();
  ws = new WebSocket(list.find((t) => t.type === 'node').webSocketDebuggerUrl);
  ws.addEventListener('message', (e) => { const m = JSON.parse(e.data); if (m.id && pend.has(m.id)) { pend.get(m.id)(m); pend.delete(m.id); } });
  await new Promise((r) => ws.addEventListener('open', r));
  try {
    console.log('选择(自建命令):', await page(SELECT));
    await sleep(1000);
    const s1 = await state(); console.log('1 选择后:', fmt(s1));
    // 键盘目标 = 指针所在画布
    const rc = JSON.parse(await page(`JSON.stringify((function(){ var r = document.querySelector('plasticity-viewport canvas').getBoundingClientRect(); return [r.x + r.width * 0.5, r.y + r.height * 0.88]; })())`));
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: rc[0], y: rc[1], button: 'none' }); await sleep(400);
    // 先取消选择后自动挂上的 OffsetFace(否则 body[command] 存在), 再按真实 Shift+Delete(=command:delete)
    await page(`(function(){ try { window.__PLA_EDITOR__.executor.cancelActiveCommand(); } catch (e) {} return 1; })()`);
    await sleep(800);
    console.log('2 取消后续命令后:', fmt(await state()), 'body[command]=', await page(`document.body.getAttribute('command')`));
    await key('Delete', 'Delete', 46, undefined, 8);
    await sleep(2000);
    let s4 = await state(); console.log('3 Shift+Delete 后:', fmt(s4));
    console.log('   实体/片体:', await page(`JSON.stringify((function(){ var s = window.__PLA_EDITOR__.selection.selected; return { solids: Array.from(s.solidIds).length, sheets: Array.from(s.sheetIds).length }; })())`));    await shot('after');
    await key('z', 'KeyZ', 90, undefined, 2); await sleep(1500);
    const s5 = await state(); console.log('5 Ctrl+Z 后:', fmt(s5));
    await shot('undone');
    console.log(`结论: 几何变化=${s4.geo !== s1.geo}(${s1.geo}→${s4.geo}) 撤销栈 ${s1.undo}→${s4.undo} | 撤销后几何版本 ${s5.geo}`);
  } catch (e) { console.log('ERR', e.message); }
  ws.close(); process.exit(0);
})();
