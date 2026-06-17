// Generates the GameSync source icon (1024x1024 PNG): a blue gradient rounded
// tile with a white circular "sync" double-arrow. Run `npm run tauri icon
// src-tauri/icons/icon.png` afterwards to derive every platform format.
const zlib = require("zlib");
const fs = require("fs");
const path = require("path");

const S = 1024;

function crc32(buf) {
  let c = ~0;
  for (const b of buf) {
    c ^= b;
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return (~c) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const t = Buffer.from(type);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([t, data])));
  return Buffer.concat([len, t, data, crc]);
}

// point-in-triangle via edge signs
function inTriangle(px, py, a, b, c) {
  const d = (p, q, r) =>
    (p[0] - r[0]) * (q[1] - r[1]) - (q[0] - r[0]) * (p[1] - r[1]);
  const d1 = d([px, py], a, b);
  const d2 = d([px, py], b, c);
  const d3 = d([px, py], c, a);
  const neg = d1 < 0 || d2 < 0 || d3 < 0;
  const pos = d1 > 0 || d2 > 0 || d3 > 0;
  return !(neg && pos);
}

function arrowhead(angleDeg, dir) {
  const rMid = (0.215 + 0.3) / 2 * S;
  const a = (angleDeg * Math.PI) / 180;
  const cx = 0.5 * S + rMid * Math.cos(a);
  const cy = 0.5 * S + rMid * Math.sin(a);
  const tang = [-Math.sin(a) * dir, Math.cos(a) * dir]; // tangent
  const rad = [Math.cos(a), Math.sin(a)];
  const L = 0.12 * S;
  const W = (0.3 - 0.215) / 2 * S + 0.035 * S;
  const tip = [cx + tang[0] * L, cy + tang[1] * L];
  const b1 = [cx + rad[0] * W, cy + rad[1] * W];
  const b2 = [cx - rad[0] * W, cy - rad[1] * W];
  return [tip, b1, b2];
}

function buildPng() {
  const px = Buffer.alloc(S * S * 4);
  const r = 0.18 * S;
  const cx = 0.5 * S,
    cy = 0.5 * S;
  const rOut = 0.3 * S,
    rIn = 0.215 * S;
  const gaps = [
    { c: 40, d: 24 },
    { c: 220, d: 24 },
  ];
  const heads = [arrowhead(40, 1), arrowhead(220, 1)];

  for (let y = 0; y < S; y++) {
    for (let x = 0; x < S; x++) {
      const i = (y * S + x) * 4;

      // rounded-square background mask
      const dx = Math.max(r - x, x - (S - 1 - r), 0);
      const dy = Math.max(r - y, y - (S - 1 - r), 0);
      const inTile = dx * dx + dy * dy <= r * r;
      if (!inTile) {
        px[i + 3] = 0;
        continue;
      }
      // vertical gradient #4f8cff -> #2f6ae0
      const t = y / S;
      let R = Math.round(0x4f * (1 - t) + 0x2f * t);
      let G = Math.round(0x8c * (1 - t) + 0x6a * t);
      let B = Math.round(0xff * (1 - t) + 0xe0 * t);
      let A = 255;

      // sync glyph (white)
      const ddx = x - cx,
        ddy = y - cy;
      const dist = Math.sqrt(ddx * ddx + ddy * ddy);
      let ang = (Math.atan2(ddy, ddx) * 180) / Math.PI;
      if (ang < 0) ang += 360;
      const inBand = dist >= rIn && dist <= rOut;
      const inGap = gaps.some((g) => {
        let diff = Math.abs(ang - g.c);
        if (diff > 180) diff = 360 - diff;
        return diff <= g.d;
      });
      const inHead = heads.some((h) => inTriangle(x, y, h[0], h[1], h[2]));
      if ((inBand && !inGap) || inHead) {
        R = G = B = 255;
      }

      px[i] = R;
      px[i + 1] = G;
      px[i + 2] = B;
      px[i + 3] = A;
    }
  }

  const raw = Buffer.alloc(S * (1 + S * 4));
  for (let y = 0; y < S; y++) {
    raw[y * (1 + S * 4)] = 0;
    px.copy(raw, y * (1 + S * 4) + 1, y * S * 4, (y + 1) * S * 4);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(S, 0);
  ihdr.writeUInt32BE(S, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const dir = path.join(__dirname, "icons");
fs.mkdirSync(dir, { recursive: true });
fs.writeFileSync(path.join(dir, "icon.png"), buildPng());
console.log("wrote icons/icon.png (1024x1024 source)");
