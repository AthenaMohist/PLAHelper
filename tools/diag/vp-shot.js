/**
 * 视口截图(CDP Page.captureScreenshot, clip=视口 rect) — 用于视口状态往返的像素级验收。
 * 用法: node shot.js <out.png>
 */
'use strict';
const fs = require('fs');
const http = require('http');

function getJson(pathname) {
  return new Promise((resolve, reject) => {
    http
      .get({ host: '127.0.0.1', port: 9333, path: pathname }, (res) => {
        let body = '';
        res.on('data', (c) => (body += c));
        res.on('end', () => resolve(JSON.parse(body)));
      })
      .on('error', reject);
  });
}

async function main() {
  const out = process.argv[2] || 'shot.png';
  const targets = await getJson('/json/list');
  const page = targets.find((t) => t.type === 'page');
  if (!page) throw new Error('no page target');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  let id = 0;
  const pending = new Map();
  const send = (method, params) =>
    new Promise((resolve, reject) => {
      const mid = ++id;
      pending.set(mid, { resolve, reject });
      ws.send(JSON.stringify({ id: mid, method, params }));
    });
  ws.addEventListener('message', (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      pending.get(msg.id).resolve(msg);
      pending.delete(msg.id);
    }
  });
  await new Promise((resolve, reject) => {
    ws.addEventListener('open', resolve);
    ws.addEventListener('error', reject);
  });

  const rectEval = await send('Runtime.evaluate', {
    expression:
      "(function(){var ed=window.__PLA_EDITOR__;if(!ed)return '';var vps=[];ed.viewports.forEach(function(v){vps.push(v)});" +
      "var r=vps[0].container.getBoundingClientRect();return JSON.stringify({x:r.x,y:r.y,width:r.width,height:r.height,scale:1})})()",
    returnByValue: true,
  });
  const rect = JSON.parse(rectEval.result.result.value || '{}');
  if (!rect.width) throw new Error('no viewport rect: ' + JSON.stringify(rectEval.result));

  await send('Runtime.evaluate', {
    expression:
      "(function(){var ed=window.__PLA_EDITOR__;var vps=[];ed.viewports.forEach(function(v){vps.push(v)});vps[0].setNeedsRender();return 1})()",
    returnByValue: true,
  });
  await new Promise((r) => setTimeout(r, 600));

  const shot = await send('Page.captureScreenshot', { format: 'png', clip: rect });
  if (shot.result && shot.result.data) {
    fs.writeFileSync(out, Buffer.from(shot.result.data, 'base64'));
    console.log('wrote', out, fs.statSync(out).size, 'bytes; clip', JSON.stringify(rect));
  } else {
    console.log('capture failed:', JSON.stringify(shot));
  }
  ws.close();
}

main().catch((e) => {
  console.error('ERR', e);
  process.exit(1);
});
