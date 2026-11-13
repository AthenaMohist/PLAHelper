// 实验工具: 截取指定 Plasticity 窗口页面(CDP Page.captureScreenshot, 只截 Plasticity 渲染内容, 不是桌面截图)。
// 用法: node shot.js <hwnd> <out.png> [vp]   — 带 vp 参数时只截 3D 视口区域。
// 走法同 lab.js: 经主进程 inspector 调代理登记的 __PLA_PROXY_SENDS[hwnd](pitfalls #40)。
// 注意: 页面 canvas 的 toDataURL 读回的是旧缓冲(没开 preserveDrawingBuffer), 不能用来验收渲染 —— 用本工具。
'use strict';
const fs = require('fs');
const [hwnd, out, mode] = process.argv.slice(2);
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 60000);
(async () => {
  const list = await (await fetch('http://127.0.0.1:9334/json/list')).json();
  const ws = new WebSocket(list.find((t) => t.type === 'node').webSocketDebuggerUrl);
  let id = 0;
  const pend = new Map();
  ws.addEventListener('message', (e) => { const m = JSON.parse(e.data); if (m.id && pend.has(m.id)) { pend.get(m.id)(m); pend.delete(m.id); } });
  await new Promise((r) => ws.addEventListener('open', r));
  const raw = (method, params) => new Promise((res) => { const i = ++id; pend.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
  const viaProxy = async (method, params) => {
    const r = await raw('Runtime.evaluate', {
      expression: `__PLA_PROXY_SENDS[${JSON.stringify(hwnd)}](${JSON.stringify(method)}, ${JSON.stringify(params)})`,
      returnByValue: true, awaitPromise: true,
    });
    const v = r.result && r.result.result && r.result.result.value;
    if (!v) throw new Error('proxy: ' + JSON.stringify(r).slice(0, 400));
    return v;
  };
  const params = { format: 'png' };
  if (mode === 'vp') {
    const ev = await viaProxy('Runtime.evaluate', {
      expression: '(() => { const vp = [...window.__PLA_EDITOR__.viewports][0]; const r = vp.container.getBoundingClientRect(); return JSON.stringify([r.x, r.y, r.width, r.height]); })()',
      returnByValue: true,
    });
    const [x, y, w, h] = JSON.parse(ev.result.value);
    params.clip = { x, y, width: w, height: h, scale: 1 };
  }
  const shot = await viaProxy('Page.captureScreenshot', params);
  if (!shot.data) { console.log('NO DATA', JSON.stringify(shot).slice(0, 300)); process.exit(1); }
  fs.writeFileSync(out, Buffer.from(shot.data, 'base64'));
  console.log('ok', out, fs.statSync(out).size);
  ws.close();
  process.exit(0);
})();
