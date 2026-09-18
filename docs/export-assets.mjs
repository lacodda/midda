// One-off: rasterize midda brand SVGs into PNGs + a multi-size .ico.
// Run from the docs package so it resolves the local sharp install:
//   node export-assets.mjs
import sharp from "sharp";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Resolved from this file rather than written down: an absolute path here is
// one machine's, and the line does not put those in a repository.
const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ASSETS = path.join(REPO, "assets");
const TAURI_ICONS = path.join(REPO, "src-tauri", "icons");
const DOCS_PUBLIC = path.join(REPO, "docs", "public");
const DOCS_ASSETS = path.join(REPO, "docs", "src", "assets");

// The three levels of the mark. Which one a raster takes is decided by
// `levelFor` below, never by habit — the comment that used to sit here said
// the S tile "is what reads at icon sizes" and the loop below took it for
// every size, which is how a 256px icon ends up a flat violet blob.
const S = path.join(ASSETS, "logo-s.svg");
const M = path.join(ASSETS, "logo-m.svg");
const L = path.join(ASSETS, "logo.svg");
const BANNER = path.join(ASSETS, "banner.svg");

// Which level of the mark survives at which size — the line's rule, not a
// preference: S ≤ 27px, M 28–63px, L ≥ 64px. Below 28px the outline and the
// three tonal steps collapse into noise, so the filled tile is all that
// reads; at 64px and up there is room for the mark the product is known by.
function levelFor(size) {
  if (size <= 27) return S;
  if (size <= 63) return M;
  return L;
}

// Largest first. Windows picks by *closest size* and ignores order, but
// tauri-codegen takes entries()[0] literally as the window icon — a 16px
// first entry is a title bar stretched from sixteen pixels.
const ICO_SIZES = [256, 128, 64, 48, 32, 24, 16];

async function png(src, size, out) {
  await sharp(src, { density: 384 }).resize(size, size).png().toFile(out);
}

// Minimal ICO container: header + directory entries + embedded PNG payloads.
function buildIco(pngBuffers, sizes) {
  const count = pngBuffers.length;
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(count, 4);

  const entries = Buffer.alloc(16 * count);
  let offset = 6 + 16 * count;
  pngBuffers.forEach((buf, i) => {
    const size = sizes[i];
    const e = 16 * i;
    entries.writeUInt8(size >= 256 ? 0 : size, e + 0); // width (0 means 256)
    entries.writeUInt8(size >= 256 ? 0 : size, e + 1); // height
    entries.writeUInt8(0, e + 2); // palette
    entries.writeUInt8(0, e + 3); // reserved
    entries.writeUInt16LE(1, e + 4); // color planes
    entries.writeUInt16LE(32, e + 6); // bits per pixel
    entries.writeUInt32LE(buf.length, e + 8);
    entries.writeUInt32LE(offset, e + 12);
    offset += buf.length;
  });

  return Buffer.concat([header, entries, ...pngBuffers]);
}

const icoParts = [];
for (const size of ICO_SIZES) {
  icoParts.push(await sharp(levelFor(size), { density: 384 }).resize(size, size).png().toBuffer());
}
fs.writeFileSync(path.join(ASSETS, "icon.ico"), buildIco(icoParts, ICO_SIZES));
console.log("wrote icon.ico");

// Favicon + docs logo.
// The one documented exception to `levelFor`: a favicon is drawn into 16px
// of browser tab whatever size the file is, and the outline does not survive
// that. The canon names it explicitly, so it is spelled out here rather than
// left looking like an oversight.
await png(S, 32, path.join(ASSETS, "favicon-32.png"));
await png(levelFor(180), 180, path.join(ASSETS, "apple-touch-icon.png"));
await png(levelFor(512), 512, path.join(ASSETS, "logo-512.png"));
console.log("wrote pngs");

// The desktop app's icons. Tauri reads these from src-tauri/icons at build
// time: the .ico is the Windows executable and title bar, the .png is what
// every other platform takes.
fs.mkdirSync(TAURI_ICONS, { recursive: true });
fs.copyFileSync(path.join(ASSETS, "icon.ico"), path.join(TAURI_ICONS, "icon.ico"));
for (const size of [32, 128, 256]) {
  await png(levelFor(size), size, path.join(TAURI_ICONS, size === 256 ? "icon.png" : `${size}x${size}.png`));
}
console.log("wrote the desktop icons");

// The docs site's copies. Written here rather than by hand: a second copy of a
// file drifts, and the one that drifts is always the one nobody opens. midda's
// touch icon was still the S cut in docs/ after assets/ had been fixed.
fs.mkdirSync(DOCS_PUBLIC, { recursive: true });
fs.mkdirSync(DOCS_ASSETS, { recursive: true });
fs.copyFileSync(S, path.join(DOCS_PUBLIC, "favicon.svg"));
fs.copyFileSync(path.join(ASSETS, "apple-touch-icon.png"), path.join(DOCS_PUBLIC, "apple-touch-icon.png"));
fs.copyFileSync(L, path.join(DOCS_ASSETS, "logo.svg"));
console.log("wrote the docs assets");

// Reference renders, one per size in the .ico, each cut from the level the rule
// names for that size. The gate in src-tauri/tests/brand_assets.rs compares the
// .ico's own entries against these byte for byte.
//
// A gate that only weighed the entries could not fail: a PNG cut from the plain
// filled tile still grows with its size, so "the 32px image is heavier than the
// 24px one" is true whichever master both came from. Weight is only a signal
// against a reference, so the reference is written down.
const REFERENCE = path.join(ASSETS, "icon-levels");
fs.rmSync(REFERENCE, { recursive: true, force: true });
fs.mkdirSync(REFERENCE, { recursive: true });
for (const size of ICO_SIZES) {
  await png(levelFor(size), size, path.join(REFERENCE, `${size}.png`));
}

// Which master each size was cut from, written down beside the renders. The
// gate reads this and checks it against its own reading of the rule, so a
// generator that quietly took one master for everything is caught by name
// rather than by weight — weight cannot do it: cut wrongly from S the seven
// entries still grow 393 → 4594, and the step across a band boundary (132% at
// 48→64px) is inside the range a correct set produces (139%).
fs.writeFileSync(
  path.join(REFERENCE, "manifest.json"),
  `${JSON.stringify(
    Object.fromEntries(ICO_SIZES.map((size) => [size, path.basename(levelFor(size))])),
    null,
    2,
  )}
`,
);
console.log("wrote the reference renders");

// GitHub social preview: 1280x640. Two adjustments to the banner: its plate
// spans the full 720px while the artwork only fills the left ~570px (trim the
// tail, or it lands off-centre), and the rounded plate over an identical
// background leaves a visible seam (drop it and keep the inner rows only).
const bannerWidth = 1600;
const bannerHeight = Math.round((bannerWidth * 170) / 720);
const inset = Math.round((bannerWidth * 6) / 720); // clears the plate's rounded edge
const banner = await sharp(BANNER, { density: 384 })
  .resize({ width: bannerWidth })
  .extract({ left: inset, top: inset, width: 1290 - inset, height: bannerHeight - 2 * inset })
  .png()
  .toBuffer();

await sharp({
  create: { width: 1280, height: 640, channels: 4, background: "#1B2126" },
})
  .composite([{ input: await sharp(banner).resize({ width: 880 }).png().toBuffer(), gravity: "centre" }])
  .png()
  .toFile(path.join(ASSETS, "social-preview.png"));
console.log("wrote social-preview.png");
