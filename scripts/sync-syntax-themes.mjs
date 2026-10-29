// Refresh the curated syntax themes in src/syntax-themes/ and rewrite index.json.
//
// Run by hand after bumping highlight.js or @catppuccin/highlightjs, not at
// postinstall: the files are committed so the Rust side can `include_str!` them
// for `--share` and `--export` without an `npm install`, and the list is fixed
// by design. The two emede ports (Nord Light, Kanagawa Wave) are hand-written
// and only indexed here.
//
// For each theme the index records the background and text color of its
// `.hljs` rule, which the app moves onto `pre` (see `src/syntax.css`), and the
// script warns about any token color below 3:1 on that background.
import { cpSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const out = resolve(root, "src/syntax-themes");
const modules = resolve(root, "node_modules");

const HLJS = "@highlightjs/cdn-assets";
const CATPPUCCIN = "@catppuccin/highlightjs";

/// The curated list, in display order within each of Light and Dark.
const THEMES = [
  // Light
  { id: "default", label: "highlight.js default", from: HLJS },
  { id: "github", label: "GitHub", from: HLJS },
  { id: "atom-one-light", label: "Atom One Light", from: HLJS },
  { id: "stackoverflow-light", label: "Stack Overflow Light", from: HLJS },
  { id: "tokyo-night-light", label: "Tokyo Night Light", from: HLJS },
  { id: "vs", label: "Visual Studio", from: HLJS },
  { id: "xcode", label: "Xcode", from: HLJS },
  { id: "a11y-light", label: "a11y Light", from: HLJS },
  { id: "intellij-light", label: "IntelliJ Light", from: HLJS },
  { id: "base16/solarized-light", label: "Solarized Light", from: HLJS },
  { id: "base16/gruvbox-light-soft", label: "Gruvbox Light Soft", from: HLJS },
  { id: "rose-pine-dawn", label: "Rosé Pine Dawn", from: HLJS },
  { id: "catppuccin-latte", label: "Catppuccin Latte", from: CATPPUCCIN },
  { id: "nord-light", label: "Nord Light", from: "emede" },
  // Dark
  { id: "github-dark", label: "GitHub Dark", from: HLJS },
  { id: "github-dark-dimmed", label: "GitHub Dark Dimmed", from: HLJS },
  { id: "atom-one-dark", label: "Atom One Dark", from: HLJS },
  { id: "tokyo-night-dark", label: "Tokyo Night Dark", from: HLJS },
  { id: "vs2015", label: "Visual Studio 2015", from: HLJS },
  { id: "monokai", label: "Monokai", from: HLJS },
  { id: "night-owl", label: "Night Owl", from: HLJS },
  { id: "nord", label: "Nord", from: HLJS },
  { id: "a11y-dark", label: "a11y Dark", from: HLJS },
  { id: "base16/dracula", label: "Dracula", from: HLJS },
  { id: "base16/gruvbox-dark-soft", label: "Gruvbox Dark Soft", from: HLJS },
  { id: "base16/solarized-dark", label: "Solarized Dark", from: HLJS },
  { id: "base16/material-palenight", label: "Material Palenight", from: HLJS },
  { id: "rose-pine-moon", label: "Rosé Pine Moon", from: HLJS },
  { id: "catppuccin-mocha", label: "Catppuccin Mocha", from: CATPPUCCIN },
  { id: "kanagawa-wave", label: "Kanagawa Wave", from: "emede" },
];

const NAMED_COLORS = {
  white: "#ffffff",
  black: "#000000",
  navy: "#000080",
  gray: "#808080",
  grey: "#808080",
};

/// A CSS color as `#rrggbb`, or null for anything not opaque and simple.
function parseColor(value) {
  const v = value.trim().toLowerCase();
  if (NAMED_COLORS[v]) return NAMED_COLORS[v];
  let m = /^#([0-9a-f]{3})$/.exec(v);
  if (m) return `#${[...m[1]].map((c) => c + c).join("")}`;
  m = /^#([0-9a-f]{6})$/.exec(v);
  if (m) return v;
  m = /^rgb\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)$/.exec(v);
  if (m) return `#${m.slice(1, 4).map((c) => Number(c).toString(16).padStart(2, "0")).join("")}`;
  return null;
}

/// Top-level rules as [selectors, declarations], skipping comments and at-rules.
function rules(css) {
  const text = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const found = [];
  let depth = 0;
  let start = 0;
  let inAtRule = false;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === "{") {
      if (depth === 0) {
        const selector = text.slice(start, i).trim();
        inAtRule = selector.startsWith("@");
        found.push({ selector, start: i + 1, skip: inAtRule });
      }
      depth++;
    } else if (text[i] === "}") {
      depth--;
      if (depth === 0) {
        const rule = found[found.length - 1];
        rule.body = text.slice(rule.start, i);
        start = i + 1;
      }
    }
  }
  return found
    .filter((rule) => !rule.skip && rule.body !== undefined)
    .map((rule) => ({
      selectors: rule.selector.split(",").map((s) => s.trim()),
      decls: Object.fromEntries(
        rule.body
          .split(";")
          .map((d) => d.split(":"))
          .filter((parts) => parts.length >= 2)
          .map(([k, ...v]) => [k.trim().toLowerCase(), v.join(":").trim()]),
      ),
    }));
}

/// The `bg` and `fg` of a theme's `.hljs` (or `code.hljs`) rules.
function baseColors(css) {
  let bg = null;
  let fg = null;
  for (const { selectors, decls } of rules(css)) {
    if (!selectors.some((s) => s === ".hljs" || s === "code.hljs")) continue;
    const background = decls.background ?? decls["background-color"];
    if (background && parseColor(background)) bg = parseColor(background);
    if (decls.color && parseColor(decls.color)) fg = parseColor(decls.color);
  }
  return { bg, fg };
}

function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

function sourceFile(theme) {
  if (theme.from === HLJS) return resolve(modules, HLJS, "styles", `${theme.id}.css`);
  if (theme.from === CATPPUCCIN) return resolve(modules, CATPPUCCIN, "css", `${theme.id}.css`);
  return null;
}

function version(pkg) {
  return JSON.parse(readFileSync(resolve(modules, pkg, "package.json"), "utf8")).version;
}

const versions = { [HLJS]: version(HLJS), [CATPPUCCIN]: version(CATPPUCCIN) };
const index = [];
let failed = false;

for (const theme of THEMES) {
  const target = resolve(out, `${theme.id}.css`);
  const source = sourceFile(theme);
  if (source) {
    mkdirSync(dirname(target), { recursive: true });
    cpSync(source, target);
  }
  const css = readFileSync(target, "utf8");
  const { bg, fg } = baseColors(css);
  if (!bg || !fg) {
    console.error(`${theme.id}: no .hljs background and color`);
    failed = true;
    continue;
  }

  const low = [];
  for (const { selectors, decls } of rules(css)) {
    const color = decls.color && parseColor(decls.color);
    if (!color || selectors.some((s) => s.includes("::selection"))) continue;
    const ratio = contrast(color, bg);
    if (ratio < 3) low.push(`${selectors[0]} ${color} ${ratio.toFixed(2)}:1`);
  }
  if (contrast(fg, bg) < 3) low.unshift(`text ${fg} ${contrast(fg, bg).toFixed(2)}:1`);
  for (const line of new Set(low)) console.warn(`warning: ${theme.id}: ${line}`);

  index.push({
    id: theme.id,
    label: theme.label,
    dark: luminance(bg) < 0.18,
    bg,
    fg,
    source: theme.from === "emede" ? "emede port" : `${theme.from} ${versions[theme.from]}`,
  });
}

if (failed) process.exit(1);
writeFileSync(resolve(out, "index.json"), `${JSON.stringify(index, null, 2)}\n`);
console.log(`syntax themes: indexed ${index.length}`);
