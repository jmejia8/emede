/**
 * The syntax theme: which highlight.js theme colors the code blocks.
 *
 * `settings.syntax_theme` is a curated id from `syntax-themes/index.json`
 * (`base16/dracula`), `user:<name>` for a theme imported into the config dir,
 * or null for Auto (`syntax-auto.css`, token colors derived from the color
 * theme). Each bundled color preset names a paired theme, which choosing the
 * preset selects.
 *
 * A curated or Auto theme is the `href` of `<link id="syntax-theme">`. An
 * imported one goes into a `<style>` instead, since the CSP allows inline
 * styles but not `blob:` stylesheets. Only once the theme has loaded are its
 * `bg` and `fg` set as `--syntax-bg` / `--syntax-fg` and `data-syntax-theme`
 * set on `:root`, which `syntax.css` turns into the `pre`'s colors, so a block
 * never shows one theme's background under another's tokens. Any failure
 * falls back to Auto.
 */

const { invoke } = window.__TAURI__.core;

const AUTO_HREF = "syntax-auto.css";
const USER_PREFIX = "user:";
/// Below this contrast against the page, a block's edge is invisible.
const FLUSH_CONTRAST = 1.1;

const root = document.documentElement;
const link = document.getElementById("syntax-theme");
let userStyle = null;
let indexPromise = null;
let applyToken = 0;
let settled = Promise.resolve();

/** The curated list, `[{ id, label, dark, bg, fg, source }]`; empty on failure. */
export function loadSyntaxThemeIndex() {
  indexPromise ??= fetch("syntax-themes/index.json")
    .then((response) => {
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      return response.json();
    })
    .catch((err) => {
      console.warn("Syntax theme list unavailable", err);
      indexPromise = null;
      return [];
    });
  return indexPromise;
}

/** Imported themes, `[{ id, label, bg, fg }]`. */
export async function listUserSyntaxThemes() {
  try {
    return await invoke("list_user_syntax_themes");
  } catch (err) {
    console.warn("Failed to list imported syntax themes", err);
    return [];
  }
}

/** Copy a theme file into the config dir; resolves to its list entry. */
export function importSyntaxTheme(path) {
  return invoke("import_syntax_theme", { path });
}

/** Resolves once the most recent `applySyntaxTheme` has painted. */
export function syntaxThemeSettled() {
  return settled;
}

/**
 * Show theme `id` (null for Auto) on a page whose background is `pageBg`.
 * Never rejects. A later call supersedes an earlier one still loading.
 */
export function applySyntaxTheme(id, pageBg) {
  const token = ++applyToken;
  settled = resolveTheme(id)
    .then(async (theme) => {
      if (token !== applyToken) return;
      if (theme && (await loadTheme(theme)) && token === applyToken) {
        showTheme(id, theme, pageBg);
        return;
      }
      if (token !== applyToken) return;
      if (id) console.warn(`Syntax theme ${id} unavailable; using Auto`);
      await loadTheme({ href: AUTO_HREF });
      if (token === applyToken) showTheme(null);
    })
    .catch((err) => console.warn("Failed to apply the syntax theme", err));
  return settled;
}

async function resolveTheme(id) {
  if (!id) return null;
  if (id.startsWith(USER_PREFIX)) {
    try {
      return await invoke("read_user_syntax_theme", { id });
    } catch (err) {
      console.warn(`Failed to read syntax theme ${id}`, err);
      return null;
    }
  }
  const entry = (await loadSyntaxThemeIndex()).find((theme) => theme.id === id);
  return entry ? { ...entry, href: `syntax-themes/${id}.css` } : null;
}

/** Swap the stylesheet; resolves to whether it loaded. */
function loadTheme(theme) {
  if (theme.css !== undefined) {
    userStyle ??= Object.assign(document.createElement("style"), { id: "syntax-theme-user" });
    if (!userStyle.isConnected) link.after(userStyle);
    userStyle.textContent = theme.css;
    link.removeAttribute("href");
    return Promise.resolve(true);
  }
  userStyle?.remove();
  if (link.getAttribute("href") === theme.href && link.sheet) return Promise.resolve(true);
  return new Promise((resolve) => {
    const done = (ok) => {
      link.removeEventListener("load", onLoad);
      link.removeEventListener("error", onError);
      resolve(ok);
    };
    const onLoad = () => done(true);
    const onError = () => done(false);
    link.addEventListener("load", onLoad);
    link.addEventListener("error", onError);
    link.setAttribute("href", theme.href);
  });
}

function showTheme(id, theme, pageBg) {
  if (!id) {
    delete root.dataset.syntaxTheme;
    delete root.dataset.syntaxFlush;
    root.style.removeProperty("--syntax-bg");
    root.style.removeProperty("--syntax-fg");
    return;
  }
  root.style.setProperty("--syntax-bg", theme.bg);
  root.style.setProperty("--syntax-fg", theme.fg);
  root.dataset.syntaxTheme = id;
  if (contrast(theme.bg, pageBg) < FLUSH_CONTRAST) {
    root.dataset.syntaxFlush = "";
  } else {
    delete root.dataset.syntaxFlush;
  }
}

/**
 * Fill the theme `<select>`: Auto, the curated Light and Dark groups, and any
 * imported themes. Each option is painted in its theme's colors where the
 * platform's popup honors that.
 */
export function populateSyntaxThemeSelect(select, curated, imported) {
  select.replaceChildren(new Option("Auto (derived from colors)", ""));
  const group = (label, themes) => {
    if (!themes.length) return;
    const optgroup = document.createElement("optgroup");
    optgroup.label = label;
    for (const theme of themes) {
      const option = new Option(theme.label, theme.id);
      option.style.backgroundColor = theme.bg;
      option.style.color = theme.fg;
      option.dataset.bg = theme.bg;
      option.dataset.fg = theme.fg;
      optgroup.appendChild(option);
    }
    select.appendChild(optgroup);
  };
  group("Light", curated.filter((theme) => !theme.dark));
  group("Dark", curated.filter((theme) => theme.dark));
  group("Imported", imported);
}

/**
 * Select `id` in the `<select>` and paint `swatch` in its colors.
 *
 * A `<select>` cannot hold a value with no matching option, so an id that is
 * not listed (a removed import, or a list not loaded yet) gets one synthesized.
 * Otherwise the control would show Auto, and the next save of any other
 * setting would write Auto over the user's choice.
 */
export function syncSyntaxThemeSelect(select, swatch, id) {
  const value = id ?? "";
  if (![...select.options].some((option) => option.value === value)) {
    const label = value ? `${value} (unavailable)` : "Auto (derived from colors)";
    select.appendChild(new Option(label, value));
  }
  select.value = value;
  const option = select.selectedOptions[0];
  swatch.classList.toggle("syntax-theme-swatch--auto", !option?.dataset.bg);
  swatch.style.backgroundColor = option?.dataset.bg ?? "";
  swatch.style.color = option?.dataset.fg ?? "";
}

function luminance(hex) {
  const match = /^#([0-9a-f]{6})$/i.exec(hex ?? "");
  if (!match) return null;
  const [r, g, b] = [0, 2, 4].map((i) => {
    const c = Number.parseInt(match[1].slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio; Infinity when either color is not `#rrggbb`. */
function contrast(a, b) {
  const la = luminance(a);
  const lb = luminance(b);
  if (la === null || lb === null) return Infinity;
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}
