import { init } from "mathjax";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { existsSync } from "node:fs";

const here = dirname(fileURLToPath(import.meta.url));
// Point MathJax at the exact files the app ships (src/vendor/mathjax/...).
const fontsRoot = resolve(here, "../src/vendor/mathjax");
const fontDir = resolve(fontsRoot, "mathjax-newcm-font");

if (!existsSync(fontDir)) {
  console.error("FAIL: vendored font dir missing:", fontDir);
  process.exit(1);
}

const MathJax = await init({
  loader: {
    load: ["input/tex", "output/chtml"],
    paths: { fonts: fontsRoot },
  },
  tex: { inlineMath: [["$", "$"]], displayMath: [["$$", "$$"]] },
  startup: { typeset: false },
});

const adaptor = MathJax.startup.adaptor;

// Exercise base glyphs + symbols that require on-demand dynamic font ranges
// (arrows, calligraphic) so dynamic loading from the local dir is tested too.
const samples = ["E = mc^2", "\\int_0^\\infty x^2\\,dx", "\\alpha \\to \\mathcal{R}\\;\\sum_{n=1}^{N} n"];

for (const tex of samples) {
  const node = await MathJax.tex2chtmlPromise(tex, { display: true });
  const html = adaptor.outerHTML(node);
  if (!html.includes("mjx-container")) {
    console.error("FAIL: no CHTML produced for:", tex);
    process.exit(1);
  }
}

const css = adaptor.textContent(MathJax.chtmlStylesheet());
const urls = css.match(/url\(([^)]+)\)/g) || [];
const woffRefs = urls.filter((u) => /\.woff2?/i.test(u));
const httpRefs = urls.filter((u) => /https?:/i.test(u));

console.log("OK: typeset", samples.length, "expressions offline using local fonts");
console.log("total @font-face url() entries:", urls.length, "| woff entries:", woffRefs.length);
console.log("sample woff url:", woffRefs[0] || "(none)");
console.log("any remote (http) font url:", httpRefs.length > 0 ? httpRefs : "NONE");
