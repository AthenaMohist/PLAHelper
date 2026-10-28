/**
 * 极简 PNG 解码 + 图像差异度量(无第三方依赖): 给视口状态恢复做像素级验收。
 * 用法: node pngdiff.js <a.png> <b.png>
 * 输出: 平均绝对差(MAD, 0..255) 与 差异>8 的像素占比。
 */
'use strict';
const fs = require('fs');
const zlib = require('zlib');

/**
 * 解码 8bit 非隔行 PNG(颜色类型 2/6)。
 * @param {string} file 文件路径
 * @return {{width: number, height: number, data: Buffer}} RGB 像素(每像素 3 字节)
 */
function decodePng(file) {
  const buf = fs.readFileSync(file);
  let pos = 8;
  let width = 0;
  let height = 0;
  let colorType = 0;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString('ascii', pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      colorType = data[9];
      if (data[8] !== 8 || data[12] !== 0) {
        throw new Error(`unsupported PNG (depth ${data[8]}, interlace ${data[12]})`);
      }
    } else if (type === 'IDAT') {
      idat.push(data);
    } else if (type === 'IEND') {
      break;
    }
    pos += len + 12;
  }
  const channels = colorType === 6 ? 4 : colorType === 2 ? 3 : 0;
  if (!channels) throw new Error(`unsupported color type ${colorType}`);
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const out = Buffer.alloc(width * height * 3);
  const prev = Buffer.alloc(stride);
  let off = 0;
  for (let y = 0; y < height; y += 1) {
    const filter = raw[off];
    off += 1;
    const line = Buffer.from(raw.subarray(off, off + stride));
    off += stride;
    for (let x = 0; x < stride; x += 1) {
      const a = x >= channels ? line[x - channels] : 0;
      const b = prev[x];
      const c = x >= channels ? prev[x - channels] : 0;
      switch (filter) {
        case 0:
          break;
        case 1:
          line[x] = (line[x] + a) & 0xff;
          break;
        case 2:
          line[x] = (line[x] + b) & 0xff;
          break;
        case 3:
          line[x] = (line[x] + ((a + b) >> 1)) & 0xff;
          break;
        case 4: {
          const p = a + b - c;
          const pa = Math.abs(p - a);
          const pb = Math.abs(p - b);
          const pc = Math.abs(p - c);
          const pr = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
          line[x] = (line[x] + pr) & 0xff;
          break;
        }
        default:
          throw new Error(`bad filter ${filter}`);
      }
    }
    line.copy(prev);
    for (let x = 0; x < width; x += 1) {
      out[(y * width + x) * 3] = line[x * channels];
      out[(y * width + x) * 3 + 1] = line[x * channels + 1];
      out[(y * width + x) * 3 + 2] = line[x * channels + 2];
    }
  }
  return { width, height, data: out };
}

/**
 * 平均绝对差 + 显著差异像素比例。
 * @param {{width:number,height:number,data:Buffer}} a
 * @param {{width:number,height:number,data:Buffer}} b
 * @return {{mad: number, ratio: number}}
 */
function diff(a, b) {
  const n = Math.min(a.width * a.height, b.width * b.height) * 3;
  let sum = 0;
  let big = 0;
  let px = 0;
  for (let i = 0; i < n; i += 3) {
    const d =
      (Math.abs(a.data[i] - b.data[i]) +
        Math.abs(a.data[i + 1] - b.data[i + 1]) +
        Math.abs(a.data[i + 2] - b.data[i + 2])) /
      3;
    sum += d;
    if (d > 8) big += 1;
    px += 1;
  }
  return { mad: +(sum / px).toFixed(3), ratio: +(big / px).toFixed(4) };
}

const a = decodePng(process.argv[2]);
const b = decodePng(process.argv[3]);
console.log(`${process.argv[2]} vs ${process.argv[3]} ->`, JSON.stringify(diff(a, b)));
