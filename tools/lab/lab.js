// 实验工具: 在指定 Plasticity 窗口的页面里执行一个 JS 文件(表达式, 可返回 Promise), 打印 JSON 结果。
// 用法: node lab.js <hwnd> <file.js>
// 经主进程 inspector 调代理登记的 __PLA_PROXY_SENDS[hwnd](node 内置 WebSocket 连不上 9333 代理, pitfalls #40)。
'use strict';
const fs = require('fs');
const [hwnd, file] = process.argv.slice(2);
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
  const params = { expression: code, returnByValue: true, awaitPromise: true };
  const r = await raw('Runtime.evaluate', {
    expression: `__PLA_PROXY_SENDS[${JSON.stringify(hwnd)}]('Runtime.evaluate', ${JSON.stringify(params)})`,
    returnByValue: true, awaitPromise: true,
  });
  const outer = r.result && r.result.result;
  if (!outer || outer.subtype === 'error') { console.log('MAIN ERR', JSON.stringify(r).slice(0, 600)); process.exit(1); }
  const inner = outer.value;
  if (inner && inner.exceptionDetails) { console.log('PAGE EXC', JSON.stringify(inner.exceptionDetails).slice(0, 1500)); process.exit(1); }
  const v = inner && inner.result && inner.result.value;
  console.log(typeof v === 'string' ? v : JSON.stringify(v, null, 1));
  ws.close();
  process.exit(0);
})();
