// 实验工具: 在 Plasticity 主进程(Electron main, 9334 inspector)里执行一个 JS 文件(表达式, 可返回 Promise), 打印 JSON 结果。
// 用法: node lab-main.js <file.js>      (页面里执行用 lab.js)
'use strict';
const fs = require('fs');
const [file] = process.argv.slice(2);
const code = fs.readFileSync(file, 'utf8');
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 120000);
(async () => {
  const list = await (await fetch('http://127.0.0.1:9334/json/list')).json();
  const ws = new WebSocket(list.find((t) => t.type === 'node').webSocketDebuggerUrl);
  let id = 0;
  const pend = new Map();
  ws.addEventListener('message', (e) => { const m = JSON.parse(e.data); if (m.id && pend.has(m.id)) { pend.get(m.id)(m); pend.delete(m.id); } });
  await new Promise((r) => ws.addEventListener('open', r));
  const raw = (method, params) => new Promise((res) => { const i = ++id; pend.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
  const r = await raw('Runtime.evaluate', { expression: code, returnByValue: true, awaitPromise: true, includeCommandLineAPI: true });
  const res = r.result || {};
  if (res.exceptionDetails) { console.log('EXC', JSON.stringify(res.exceptionDetails).slice(0, 1500)); process.exit(1); }
  const v = res.result && res.result.value;
  console.log(typeof v === 'string' ? v : JSON.stringify(v, null, 1));
  ws.close();
  process.exit(0);
})();
