// Pele vs Mermaid bake-off: parse success, render time, SVG size, label
// completeness, and side-by-side screenshots. Run from the report folder:
//   node scripts/bench.mjs            (RUNS=30 INNER=10 by default)
import { createRequire } from "node:module";
import { readFileSync, readdirSync, writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const NM = join(ROOT, "harness", "node_modules");
const RUNS = Number(process.env.RUNS ?? 30);
const INNER = Number(process.env.INNER ?? 10);
const WARMUP = 3;
const { chromium } = createRequire(join(NM, "x.js"))("playwright");
const WIDTH = 760; // content column, close to emede's default reading width
const FONT = "Noto Sans";

const corpus = readdirSync(join(ROOT, "corpus"))
  .filter((f) => f.endsWith(".mmd"))
  .sort()
  .map((f) => ({ id: f.replace(/\.mmd$/, ""), src: readFileSync(join(ROOT, "corpus", f), "utf8") }));

// emede's light theme, and the subset of main.js's Mermaid themeVariables it drives.
const LIGHT = { fg: "#2c2c2c", bg: "#fff", codeBg: "#e5e4e1", border: "#d8d5cf", link: "#2c2c2c" };
const THEMES = {
  light: LIGHT,
  sepia: { fg: "#433422", bg: "#f4ecd8", codeBg: "#e2dac6", border: "#d8ccb2", link: "#433422" },
  dark: { fg: "#d4d0c8", bg: "#1a1a1a", codeBg: "#252525", border: "#3a3a3a", link: "#d4d0c8" },
  gruvbox: { fg: "#ebdbb2", bg: "#282828", codeBg: "#3c3836", border: "#504945", link: "#83a598" },
};
// Port of emede's autoPalette (src/main.js): golden-angle hues, low
// saturation, blended 28% toward the page background.
const hexRgb = (h) => {
  h = h.replace("#", "");
  if (h.length === 3) h = [...h].map((c) => c + c).join("");
  return { r: parseInt(h.slice(0, 2), 16), g: parseInt(h.slice(2, 4), 16), b: parseInt(h.slice(4, 6), 16) };
};
const hsl = (h, s, l) => {
  s /= 100; l /= 100;
  const k = (n) => (n + h / 30) % 12, a = s * Math.min(l, 1 - l);
  const f = (n) => Math.round(255 * (l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)))));
  return { r: f(0), g: f(8), b: f(4) };
};
const autoPalette = (count, bg) => {
  const b = hexRgb(bg);
  const dark = (0.2126 * b.r + 0.7152 * b.g + 0.0722 * b.b) / 255 < 0.5;
  return Array.from({ length: count }, (_, i) => {
    const c = hsl(Math.round((i * 137.5) % 360), 38, dark ? 58 : 52);
    const m = (x, y) => Math.round(x * 0.72 + y * 0.28);
    return `rgb(${m(c.r, b.r)}, ${m(c.g, b.g)}, ${m(c.b, b.b)})`;
  });
};
const peleVars = (t) =>
  `--pele-bg:${t.bg};--pele-fg:${t.fg};--pele-surface:${t.codeBg};--pele-border:${t.border};` +
  `--pele-surface-alt:color-mix(in srgb, ${t.fg} 6%, ${t.bg});--pele-muted:color-mix(in srgb, ${t.fg} 65%, ${t.bg});` +
  `--pele-line:${t.fg};--pele-accent:${t.link};--pele-font:'${FONT}';` +
  autoPalette(8, t.bg).map((c, i) => `--pele-series-${i + 1}:${c}`).join(";");

const mermaidConfig = (t) => ({
  startOnLoad: false,
  layout: "dagre",
  look: "classic",
  theme: "base",
  themeVariables: {
    ...Object.fromEntries(autoPalette(12, t.bg).flatMap((c, i) => [["pie" + (i + 1), c], ["cScale" + i, c]])),
    pieStrokeColor: t.border, pieOuterStrokeColor: t.border, pieSectionTextColor: t.fg,
    pieTitleTextColor: t.fg, pieOpacity: 1,
    fontFamily: FONT, background: t.bg, primaryColor: t.codeBg, primaryTextColor: t.fg,
    primaryBorderColor: t.fg, secondaryColor: t.codeBg, tertiaryColor: t.bg, lineColor: t.fg,
    mainBkg: t.codeBg, nodeBorder: t.fg, clusterBkg: t.codeBg, clusterBorder: t.fg,
    titleColor: t.fg, edgeLabelBackground: t.bg, nodeTextColor: t.fg, labelColor: t.fg,
    actorBkg: t.codeBg, actorBorder: t.fg, actorTextColor: t.fg, actorLineColor: t.fg,
    signalColor: t.fg, signalTextColor: t.fg, labelBoxBkgColor: t.codeBg,
    labelBoxBorderColor: t.fg, labelTextColor: t.fg, loopTextColor: t.fg,
    noteBkgColor: t.codeBg, noteBorderColor: t.fg, noteTextColor: t.fg,
    activationBkgColor: t.codeBg, activationBorderColor: t.fg, linkColor: t.link,
  },
});

const PAGE = `<!doctype html><html><head><meta charset="utf-8"><style>
  body { margin: 16px; background: #fff; color: #2c2c2c; font-family: '${FONT}'; }
  .cell { width: ${WIDTH}px; background: ${LIGHT.bg}; }
  .row { display: flex; gap: 24px; align-items: flex-start; padding: 12px; background: #fff; }
  .row h3 { font: 600 14px '${FONT}'; margin: 0 0 8px; color: #555; }
  .col { border: 1px solid #ddd; padding: 12px; }
  #host { width: ${WIDTH}px; position: absolute; left: -10000px; top: 0; }
  .err { color: #b00; font: 13px monospace; white-space: pre-wrap; width: ${WIDTH}px; }
</style></head><body><div id="host"></div><div id="stage"></div></body></html>`;

async function setup(page) {
  await page.setContent(PAGE);
  await page.addScriptTag({ path: join(NM, "mermaid/dist/mermaid.min.js") });
  await page.addScriptTag({ path: join(NM, "pele/dist/pele.global.js") });
  await page.evaluate(
    ({ cfg, vars, font, width }) => {
      window.mermaid.initialize(cfg);
      window.__opts = { fontFamily: font, maxWidth: width };
      window.__vars = vars;
      let n = 0;
      // Text of a rendered SVG as whitespace tokens, ignoring <style>.
      window.__tokens = (el) => {
        const w = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
        const out = [];
        for (let t; (t = w.nextNode()); ) {
          if (t.parentElement.closest("style")) continue;
          out.push(...t.data.split(/\s+/).filter(Boolean));
        }
        return out;
      };
      window.__mermaid = async (src) => (await window.mermaid.render("m" + n++, src)).svg;
      window.__pele = (src) => {
        window.pele.forgetTextWidths(); // cold text-width cache: worst case for Pele
        return window.pele.render(src, window.__opts).svg;
      };
      // One timed sample: INNER renders, each inserted and laid out.
      window.__time = async (lib, src, inner) => {
        const host = document.getElementById("host");
        host.setAttribute("style", lib === "pele" ? window.__vars : "");
        const t0 = performance.now();
        for (let i = 0; i < inner; i++) {
          host.innerHTML = lib === "pele" ? window.__pele(src) : await window.__mermaid(src);
          host.getBoundingClientRect();
        }
        return (performance.now() - t0) / inner;
      };
    },
    { cfg: mermaidConfig(LIGHT), vars: peleVars(LIGHT), font: FONT, width: WIDTH },
  );
}

const browser = await chromium.launch({
  executablePath: process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1223/chrome-linux64/chrome`,
});
mkdirSync(join(ROOT, "raw"), { recursive: true });
mkdirSync(join(ROOT, "figures"), { recursive: true });

// ---- 1. correctness: parse/render, SVG size, label tokens -----------------
const ctx1 = await browser.newContext({ viewport: { width: 1700, height: 1200 } });
const page = await ctx1.newPage();
page.on("pageerror", (e) => console.error("pageerror:", e.message));
await setup(page);
const info = await page.evaluate(async (corpus) => {
  const host = document.getElementById("host");
  const res = [];
  for (const { id, src } of corpus) {
    const row = { id, type: window.pele.detectType(src) };
    for (const lib of ["mermaid", "pele"]) {
      try {
        const svg = lib === "pele" ? window.__pele(src) : await window.__mermaid(src);
        host.innerHTML = svg;
        const s = host.querySelector("svg");
        const r = s.getBoundingClientRect();
        row[lib] = { ok: true, bytes: svg.length, w: Math.round(r.width), h: Math.round(r.height),
          tokens: window.__tokens(host) };
      } catch (e) {
        row[lib] = { ok: false, error: String(e.message ?? e).slice(0, 300) };
      }
    }
    res.push(row);
  }
  return res;
}, corpus);
writeFileSync(join(ROOT, "raw", "correctness.json"), JSON.stringify(info, null, 1));

// ---- 2. timing --------------------------------------------------------------
const rows = ["diagram,library,run,time_ms"];
for (let run = -WARMUP; run < RUNS; run++) {
  for (const { id, src } of corpus) {
    const libs = run % 2 === 0 ? ["mermaid", "pele"] : ["pele", "mermaid"]; // alternate order
    for (const lib of libs) {
      const ok = info.find((r) => r.id === id)[lib].ok;
      if (!ok) continue;
      const t = await page.evaluate(([l, s, k]) => window.__time(l, s, k), [lib, src, INNER]);
      if (run >= 0) rows.push(`${id},${lib === "pele" ? "Pele" : "Mermaid"},${run},${t.toFixed(4)}`);
    }
  }
  process.stderr.write(`run ${run}\r`);
}
writeFileSync(join(ROOT, "raw", "timing.csv"), rows.join("\n") + "\n");
await ctx1.close();

// ---- 3. screenshots: side by side, emede light theme -----------------------
const ctx2 = await browser.newContext({ viewport: { width: 1700, height: 1200 }, deviceScaleFactor: 1.5 });
const shot = await ctx2.newPage();
await setup(shot);
const versions = await shot.evaluate(() => ({ mermaid: window.mermaid.version?.() ?? "?" }));
for (const { id, src } of corpus) {
  await shot.evaluate(async ({ id, src }) => {
    const stage = document.getElementById("stage");
    const cell = async (lib, title) => {
      let body;
      try {
        body = lib === "pele" ? window.__pele(src) : await window.__mermaid(src);
      } catch (e) {
        body = `<div class="err">${title} error:\n${String(e.message ?? e).replace(/</g, "&lt;")}</div>`;
      }
      return `<div class="col"><h3>${title}</h3><div class="cell" style="${lib === "pele" ? window.__vars : ""}">${body}</div></div>`;
    };
    stage.innerHTML = `<div class="row" id="r">${await cell("mermaid", "Mermaid 12.3.0 (emede config)")}${await cell("pele", "Pele 1.13.0")}</div>`;
  }, { id, src });
  await shot.locator("#r").screenshot({ path: join(ROOT, "figures", `${id}.png`) });
}

// ---- 4. one Pele SVG under four emede themes (no re-render) ----------------
await shot.evaluate(({ src, themes }) => {
  const svg = window.__pele(src); // rendered once
  document.getElementById("stage").innerHTML = `<div class="row" id="r" style="flex-wrap:wrap;width:1600px">` +
    Object.entries(themes).map(([name, t]) =>
      `<div class="col" style="background:${t.bg}"><h3 style="color:${t.fg}">${name}</h3><div class="cell" style="width:740px;background:${t.bg};${t.vars}">${svg}</div></div>`).join("") +
    `</div>`;
}, { src: corpus[0].src, themes: Object.fromEntries(Object.entries(THEMES).map(([k, t]) => [k, { ...t, vars: peleVars(t) }])) });
await shot.locator("#r").screenshot({ path: join(ROOT, "figures", "pele-themes.png") });

writeFileSync(join(ROOT, "raw", "env.json"), JSON.stringify({ browser: browser.version(), versions, RUNS, INNER, WARMUP, WIDTH, FONT }, null, 1));
await browser.close();
console.error("\ndone");
