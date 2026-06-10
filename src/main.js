import { FindInPage } from "./find.js";
import {
  createKeybindingController,
  normalizeKeybindingMode,
  renderKeybindingHelp,
} from "./keybindings.js";
import { getScrollRoot } from "./scroll.js";
import {
  applyViewState,
  flushViewState,
  flushViewStateAsync,
  getScrollEventTarget,
  loadViewState,
  saveViewState,
} from "./viewstate.js";

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;
const { openUrl } = window.__TAURI__.opener;

const DEFAULT_FONT = '"Literata", "Source Serif 4", "Noto Serif", serif';
const DEFAULT_FONT_CODE = '"IBM Plex Mono", "JetBrains Mono", "Fira Code", monospace';

const FONT_GROUPS = [
  {
    label: "Serif",
    fonts: [
      { label: "Literata (default)", value: DEFAULT_FONT },
      { label: "C059", value: '"C059", serif' },
      { label: "DejaVu Serif", value: '"DejaVu Serif", serif' },
      { label: "Liberation Serif", value: '"Liberation Serif", serif' },
      { label: "Nimbus Roman", value: '"Nimbus Roman", serif' },
      { label: "Noto Serif", value: '"Noto Serif", serif' },
    ],
  },
  {
    label: "Sans-serif",
    fonts: [
      { label: "Cantarell", value: '"Cantarell", sans-serif' },
      { label: "DejaVu Sans", value: '"DejaVu Sans", sans-serif' },
      { label: "Liberation Sans", value: '"Liberation Sans", sans-serif' },
      { label: "Noto Sans", value: '"Noto Sans", sans-serif' },
    ],
  },
  {
    label: "Monospace",
    fonts: [
      { label: "IBM Plex Mono (default)", value: DEFAULT_FONT_CODE },
      { label: "JetBrains Mono", value: '"JetBrains Mono", monospace' },
      { label: "Fira Code", value: '"Fira Code", monospace' },
      { label: "DejaVu Sans Mono", value: '"DejaVu Sans Mono", monospace' },
      { label: "Noto Sans Mono", value: '"Noto Sans Mono", monospace' },
    ],
  },
];

const PROSE_FONT_GROUPS = FONT_GROUPS.filter((group) => group.label !== "Monospace");
const CODE_FONT_GROUPS = FONT_GROUPS.filter((group) => group.label === "Monospace");

const DEFAULT_MARGIN_PERCENT = 10;

const BUNDLED_COLOR_TEMPLATES = [
  {
    id: "light",
    label: "Light",
    path: "themes/light.css",
  },
  {
    id: "sepia",
    label: "Sepia",
    path: "themes/sepia.css",
  },
  {
    id: "dark",
    label: "Dark",
    path: "themes/dark.css",
  },
  {
    id: "gruvbox",
    label: "Gruvbox",
    path: "themes/gruvbox.css",
  },
  {
    id: "things-light",
    label: "Things",
    path: "themes/things-light.css",
  },
  {
    id: "nord-light",
    label: "Nord",
    path: "themes/nord-light.css",
  },
  {
    id: "dracula-dark",
    label: "Dracula",
    path: "themes/dracula-dark.css",
  },
  {
    id: "catppuccin-mocha",
    label: "Moca",
    path: "themes/catppuccin-mocha.css",
  },
];

const COLOR_TEMPLATE_PROPERTIES = {
  "--color-fg": { setting: "color_fg", css: "--color-fg", required: true },
  "--emede-color-fg": { setting: "color_fg", css: "--color-fg", required: true },
  "--color-bg": { setting: "color_bg", css: "--color-bg", required: true },
  "--emede-color-bg": { setting: "color_bg", css: "--color-bg", required: true },
  "--color-title": { setting: "color_title", css: "--color-title" },
  "--color-heading": { setting: "color_title", css: "--color-title" },
  "--color-headings": { setting: "color_title", css: "--color-title" },
  "--color-bold": { setting: "color_bold", css: "--color-bold" },
  "--color-strong": { setting: "color_bold", css: "--color-bold" },
  "--color-italic": { setting: "color_italic", css: "--color-italic" },
  "--color-em": { setting: "color_italic", css: "--color-italic" },
  "--color-quote": { setting: "color_quote", css: "--color-quote" },
  "--color-blockquote": { setting: "color_quote", css: "--color-quote" },
  "--color-link": { setting: "color_link", css: "--color-link" },
  "--color-code": { setting: "color_code", css: "--color-code" },
  "--color-code-bg": { setting: "color_code_bg", css: "--color-code-bg" },
  "--color-border": { setting: "color_border", css: "--color-border" },
  "--color-muted": { setting: "color_muted", css: "--color-muted" },
};

const OPTIONAL_COLOR_TEMPLATE_SETTINGS = Object.values(COLOR_TEMPLATE_PROPERTIES)
  .filter((property) => !property.required)
  .map((property) => property.setting)
  .filter((setting, index, settings) => settings.indexOf(setting) === index);

const FONT_PRESETS = {
  default: {
    font_family: '"C059", serif',
    font_title: '"Cantarell", sans-serif',
    font_code: DEFAULT_FONT_CODE,
  },
  literata: {
    font_family: DEFAULT_FONT,
    font_title: "",
    font_code: '"JetBrains Mono", monospace',
  },
  source: {
    font_family: '"Source Serif 4", "Noto Serif", serif',
    font_title: '"Source Serif 4", "Noto Serif", serif',
    font_code: '"JetBrains Mono", "Fira Code", monospace',
  },
  noto: {
    font_family: '"Noto Serif", serif',
    font_title: '"Noto Serif", serif',
    font_code: '"Noto Sans Mono", monospace',
  },
  dejavu: {
    font_family: '"DejaVu Serif", serif',
    font_title: '"DejaVu Serif", serif',
    font_code: '"DejaVu Sans Mono", monospace',
  },
  technical: {
    font_family: '"Noto Sans", sans-serif',
    font_title: '"Cantarell", sans-serif',
    font_code: '"Fira Code", "JetBrains Mono", monospace',
  },
};

const contentEl = document.getElementById("content");
const emptyStateEl = document.getElementById("empty-state");
const errorStateEl = document.getElementById("error-state");
const errorMessageEl = document.getElementById("error-message");
const loadingStateEl = document.getElementById("loading-state");
const missingStateEl = document.getElementById("missing-state");
const missingMessageEl = document.getElementById("missing-message");
const tocPanel = document.getElementById("toc-panel");
const tocToggle = document.getElementById("toc-toggle");
const tocClose = document.getElementById("toc-close");
const tocList = document.getElementById("toc-list");
const settingsPanel = document.getElementById("settings-panel");
const settingsToggle = document.getElementById("settings-toggle");
const settingsClose = document.getElementById("settings-close");
const aboutOverlay = document.getElementById("about-overlay");
const aboutLink = document.getElementById("about-link");
const aboutClose = document.getElementById("about-close");
const settingFont = document.getElementById("setting-font");
const settingFontTitle = document.getElementById("setting-font-title");
const settingFontCode = document.getElementById("setting-font-code");
const settingSize = document.getElementById("setting-size");
const settingSizeLabel = document.getElementById("setting-size-label");
const settingMargin = document.getElementById("setting-margin");
const settingMarginLabel = document.getElementById("setting-margin-label");
const settingFg = document.getElementById("setting-fg");
const settingBg = document.getElementById("setting-bg");
const colorTemplateList = document.getElementById("color-template-list");
const importColorTemplateBtn = document.getElementById("import-color-template");
const colorTemplateStatus = document.getElementById("color-template-status");
const settingWindowFrame = document.getElementById("setting-window-frame");
const settingKeybindings = document.getElementById("setting-keybindings");
const settingGpu = document.getElementById("setting-gpu");
const settingJustify = document.getElementById("setting-justify");
const keybindingsHelp = document.getElementById("keybindings-help");
const titlebarTitle = document.getElementById("titlebar-title");
const winMinimize = document.getElementById("win-minimize");
const winMaximize = document.getElementById("win-maximize");
const winClose = document.getElementById("win-close");
const searchBar = document.getElementById("search-bar");
const searchInput = document.getElementById("search-input");
const searchCounter = document.getElementById("search-counter");
const searchPrev = document.getElementById("search-prev");
const searchNext = document.getElementById("search-next");
const searchClose = document.getElementById("search-close");
const openFileBtn = document.getElementById("open-file-btn");

let currentSettings = null;
let currentDocPath = null;
let initialFontSize = 12;
let initialGpuAccel = null;
let saveTimer = null;
let activeOpenToken = 0;
let isRestoringViewState = false;
let colorTemplates = [];
let findInPage;

function populateFontSelect(select, { includeInherit = false, groups = FONT_GROUPS } = {}) {
  select.replaceChildren();
  if (includeInherit) {
    const inheritOption = document.createElement("option");
    inheritOption.value = "";
    inheritOption.textContent = "Same as body";
    select.appendChild(inheritOption);
  }
  for (const group of groups) {
    const optgroup = document.createElement("optgroup");
    optgroup.label = group.label;
    for (const font of group.fonts) {
      const option = document.createElement("option");
      option.value = font.value;
      option.textContent = font.label;
      option.style.fontFamily = font.value;
      optgroup.appendChild(option);
    }
    select.appendChild(optgroup);
  }
}

function populateFontOptions() {
  populateFontSelect(settingFont, { groups: PROSE_FONT_GROUPS });
  populateFontSelect(settingFontTitle, { includeInherit: true, groups: PROSE_FONT_GROUPS });
  populateFontSelect(settingFontCode, { groups: CODE_FONT_GROUPS });
}

function bodyFontFromSettings(settings) {
  return settings.font_family || DEFAULT_FONT;
}

function syncFontSelect(select, value, fallback) {
  select.value = value ?? "";
  if (!select.value && fallback) select.value = fallback;
}

// Parse a CSS length to an integer point value, tolerating legacy `rem` values.
function toPt(value, fallback) {
  const n = parseFloat(value);
  if (!Number.isFinite(n)) return fallback;
  if (String(value).includes("rem")) return Math.round(n * 12);
  return Math.round(n);
}

// Parse margin to an integer percentage, migrating legacy `pt`/`rem` values.
function toMarginPercent(value, fallback = DEFAULT_MARGIN_PERCENT) {
  const n = parseFloat(value);
  if (!Number.isFinite(n)) return fallback;
  const unit = String(value);
  if (unit.includes("%")) return clampMarginPercent(n);
  if (unit.includes("pt")) return clampMarginPercent(n / 7.2);
  if (unit.includes("rem")) return clampMarginPercent((n * 12) / 7.2);
  return clampMarginPercent(n);
}

function clampMarginPercent(value) {
  return Math.min(25, Math.max(0, Math.round(value)));
}

function normalizeWindowFrame(frame) {
  return frame === "system" ? "system" : "emede";
}

async function applyWindowFrame(frame) {
  const mode = normalizeWindowFrame(frame);
  document.body.classList.remove("frame-emede", "frame-system");
  document.body.classList.add(mode === "system" ? "frame-system" : "frame-emede");

  try {
    await getCurrentWindow().setDecorations(mode === "system");
  } catch (err) {
    console.warn("Failed to set window decorations", err);
  }
}

async function setWindowTitle(text) {
  titlebarTitle.textContent = text;
  try {
    await getCurrentWindow().setTitle(text);
  } catch (err) {
    console.warn("Failed to set window title", err);
  }
}

function formatWindowTitle(label) {
  const trimmed = label?.trim();
  if (!trimmed) return "emede";
  return `${trimmed} — emede`;
}

async function syncMaximizeButton() {
  try {
    const maximized = await getCurrentWindow().isMaximized();
    winMaximize.setAttribute("aria-label", maximized ? "Restore" : "Maximize");
    winMaximize.textContent = maximized ? "\u2750" : "\u25A1";
  } catch (err) {
    console.warn("Failed to read maximize state", err);
  }
}

function isDarkColor(hex) {
  const normalized = hex.replace("#", "");
  if (normalized.length !== 6) return false;
  const r = Number.parseInt(normalized.slice(0, 2), 16);
  const g = Number.parseInt(normalized.slice(2, 4), 16);
  const b = Number.parseInt(normalized.slice(4, 6), 16);
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return luminance < 0.5;
}

function normalizeCssColor(value) {
  const color = value.trim();
  if (!color || !CSS.supports("color", color)) return null;

  const context =
    normalizeCssColor.context ||
    (normalizeCssColor.context = document.createElement("canvas").getContext("2d"));
  context.fillStyle = "#000000";
  context.fillStyle = color;

  const normalized = context.fillStyle.toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(normalized)) return normalized;

  const rgb = normalized.match(/^rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*(1|1\.0|1\.00))?\)$/);
  if (!rgb) return null;

  return `#${rgb
    .slice(1, 4)
    .map((channel) => Number(channel).toString(16).padStart(2, "0"))
    .join("")}`;
}

function extractColorTemplate(cssText, label) {
  const colors = {};
  const declarationPattern = /(--(?:emede-)?color-[a-z-]+)\s*:\s*([^;}{]+)\s*(?:;|$)/gi;

  for (const match of cssText.matchAll(declarationPattern)) {
    const property = COLOR_TEMPLATE_PROPERTIES[match[1].toLowerCase()];
    const color = normalizeCssColor(match[2]);
    if (property && color) {
      colors[property.setting] = color;
    }
  }

  if (!colors.color_fg || !colors.color_bg) {
    throw new Error("CSS template must define valid --color-fg and --color-bg colors.");
  }

  return {
    id: `imported-${Date.now()}`,
    label,
    colors,
    imported: true,
  };
}

async function loadBundledColorTemplates() {
  const templates = [];

  for (const bundledTemplate of BUNDLED_COLOR_TEMPLATES) {
    try {
      const response = await fetch(bundledTemplate.path);
      if (!response.ok) {
        throw new Error(`Failed to load ${bundledTemplate.path}`);
      }

      const cssText = await response.text();
      templates.push({
        ...extractColorTemplate(cssText, bundledTemplate.label),
        id: bundledTemplate.id,
        imported: true,
      });
    } catch (err) {
      console.warn("Failed to load bundled color template", bundledTemplate.path, err);
    }
  }

  colorTemplates = templates;
}

function colorTemplateExtrasFromSettings(settings) {
  const extras = {};
  for (const settingKey of OPTIONAL_COLOR_TEMPLATE_SETTINGS) {
    if (settings?.[settingKey]) {
      extras[settingKey] = settings[settingKey];
    }
  }
  return extras;
}

function colorTemplateLabelFromPath(path) {
  const fileName = String(path).split(/[\\/]/).pop() || "Imported CSS";
  return fileName.replace(/\.css$/i, "");
}

function renderColorTemplates() {
  colorTemplateList.replaceChildren();

  for (const template of colorTemplates) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = template.label;
    button.style.color = template.colors.color_fg;
    button.style.background = template.colors.color_bg;
    button.style.borderColor = template.colors.color_fg;
    button.addEventListener("click", () => {
      void applyColorTemplate(template);
    });
    colorTemplateList.appendChild(button);
  }
}

async function applyColorTemplate(template) {
  const baseSettings = currentSettings || settingsFromForm();
  const settings = {
    ...baseSettings,
    ...template.colors,
  };

  if (!template.imported) {
    for (const settingKey of OPTIONAL_COLOR_TEMPLATE_SETTINGS) {
      delete settings[settingKey];
    }
  }

  applySettings(settings);
  await invoke("set_settings", { settings });
}

function applySettings(settings) {
  currentSettings = settings;
  const sizePt = toPt(settings.font_size, 12);
  const marginPercent = toMarginPercent(settings.margin);
  const bodyFont = bodyFontFromSettings(settings);
  const titleFont = settings.font_title || bodyFont;
  const codeFont = settings.font_code || DEFAULT_FONT_CODE;

  document.documentElement.style.setProperty("--font-serif", bodyFont);
  document.documentElement.style.setProperty("--font-title", titleFont);
  document.documentElement.style.setProperty("--font-code", codeFont);
  document.documentElement.style.setProperty("--font-size", `${sizePt}pt`);
  document.documentElement.style.setProperty("--reader-margin", `${marginPercent}%`);
  document.documentElement.style.setProperty("--color-fg", settings.color_fg);
  document.documentElement.style.setProperty("--color-bg", settings.color_bg);
  for (const property of Object.values(COLOR_TEMPLATE_PROPERTIES)) {
    if (property.required) continue;
    if (settings[property.setting]) {
      document.documentElement.style.setProperty(property.css, settings[property.setting]);
    } else {
      document.documentElement.style.removeProperty(property.css);
    }
  }
  document.documentElement.style.colorScheme = isDarkColor(settings.color_bg)
    ? "dark"
    : "light";

  syncFontSelect(settingFont, settings.font_family, DEFAULT_FONT);
  syncFontSelect(settingFontTitle, settings.font_title, "");
  syncFontSelect(settingFontCode, settings.font_code, DEFAULT_FONT_CODE);
  settingSize.value = sizePt;
  settingSizeLabel.textContent = `${sizePt}pt`;
  settingMargin.value = marginPercent;
  settingMarginLabel.textContent = `${marginPercent}%`;
  settingFg.value = settings.color_fg;
  settingBg.value = settings.color_bg;
  settingWindowFrame.value = normalizeWindowFrame(settings.window_frame);
  settingKeybindings.value = normalizeKeybindingMode(settings.keybindings);
  renderKeybindingHelp(keybindingsHelp, settings.keybindings);
  settingGpu.checked = settings.gpu_acceleration;
  settingJustify.checked = settings.justify_text;
  contentEl.classList.toggle("prose-justify", settings.justify_text);
  void applyWindowFrame(settings.window_frame);
}

function settingsFromForm() {
  return {
    ...colorTemplateExtrasFromSettings(currentSettings),
    font_family: settingFont.value || DEFAULT_FONT,
    font_title: settingFontTitle.value,
    font_code: settingFontCode.value || DEFAULT_FONT_CODE,
    font_size: `${Number(settingSize.value)}pt`,
    color_fg: settingFg.value,
    color_bg: settingBg.value,
    margin: `${Number(settingMargin.value)}%`,
    window_frame: settingWindowFrame.value,
    keybindings: settingKeybindings.value,
    gpu_acceleration: settingGpu.checked,
    justify_text: settingJustify.checked,
  };
}

function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    const settings = settingsFromForm();
    applySettings(settings);
    await invoke("set_settings", { settings });
  }, 250);
}

function clearToc() {
  tocList.replaceChildren();
  tocToggle.classList.add("hidden");
  toggleToc(false);
}

function showEmptyState() {
  currentDocPath = null;
  contentEl.innerHTML = "";
  contentEl.classList.remove("visible");
  loadingStateEl.classList.add("hidden");
  emptyStateEl.classList.remove("hidden");
  missingStateEl.classList.add("hidden");
  errorStateEl.classList.add("hidden");
  clearToc();
}

function showLoadingState() {
  contentEl.innerHTML = "";
  contentEl.classList.remove("visible");
  loadingStateEl.classList.remove("hidden");
  emptyStateEl.classList.add("hidden");
  missingStateEl.classList.add("hidden");
  errorStateEl.classList.add("hidden");
  clearToc();
}

function showMissingFile(message) {
  currentDocPath = null;
  contentEl.innerHTML = "";
  contentEl.classList.remove("visible");
  loadingStateEl.classList.add("hidden");
  emptyStateEl.classList.add("hidden");
  missingMessageEl.textContent = message.replace(/^File not found:\s*/, "");
  missingStateEl.classList.remove("hidden");
  errorStateEl.classList.add("hidden");
  clearToc();
}

function showError(message) {
  currentDocPath = null;
  contentEl.innerHTML = "";
  contentEl.classList.remove("visible");
  loadingStateEl.classList.add("hidden");
  emptyStateEl.classList.add("hidden");
  missingStateEl.classList.add("hidden");
  errorMessageEl.textContent = message;
  errorStateEl.classList.remove("hidden");
  clearToc();
}

async function waitForMathJax() {
  if (window.MathJax?.typesetPromise) return;

  await new Promise((resolve) => {
    const deadline = Date.now() + 5000;
    const tick = () => {
      if (window.MathJax?.typesetPromise) {
        resolve();
        return;
      }
      if (Date.now() > deadline) {
        console.warn("MathJax did not load in time");
        resolve();
        return;
      }
      requestAnimationFrame(tick);
    };
    tick();
  });
}

async function typesetMath() {
  await waitForMathJax();
  if (!window.MathJax?.typesetPromise) return;

  try {
    await Promise.race([
      window.MathJax.typesetPromise([contentEl]),
      wait(12000),
    ]);
  } catch (err) {
    console.warn("MathJax typesetting failed", err);
  }
}

function scheduleTypesetMath() {
  void typesetMath();
}

function isRemoteUrl(src) {
  return /^(?:https?:|data:|mailto:|tel:)/i.test(src);
}

function rewriteLocalImageSrcs(root) {
  for (const img of root.querySelectorAll("img[src]")) {
    const src = img.getAttribute("src");
    if (!src || isRemoteUrl(src)) continue;
    img.src = convertFileSrc(src);
  }
}

function wrapTables(root) {
  for (const table of root.querySelectorAll("table")) {
    if (table.closest(".table-wrapper")) continue;
    const wrapper = document.createElement("div");
    wrapper.className = "table-wrapper";
    table.parentNode.insertBefore(wrapper, table);
    wrapper.appendChild(table);
  }
}

async function restoreSavedViewState(viewState, openToken) {
  if (!viewState || (openToken !== undefined && openToken !== activeOpenToken)) return;

  isRestoringViewState = true;
  try {
    const scrollRoot = getScrollRoot();
    applyViewState(scrollRoot, contentEl, viewState);
    await nextFrame();
    if (openToken !== undefined && openToken !== activeOpenToken) return;
    applyViewState(scrollRoot, contentEl, viewState);

    const scrollBeforeMath = scrollRoot.scrollTop;
    await typesetMath();
    if (openToken !== undefined && openToken !== activeOpenToken) return;

    if (scrollRoot.scrollTop === scrollBeforeMath) {
      applyViewState(scrollRoot, contentEl, viewState);
    }
  } finally {
    isRestoringViewState = false;
  }
}

async function applyDocument(result, { initial = false, reload = false, openToken } = {}) {
  if (openToken !== undefined && openToken !== activeOpenToken) return;

  const scrollRoot = getScrollRoot();
  const scrollTop = reload ? scrollRoot.scrollTop : 0;
  const savedViewState = reload ? null : await loadViewState(result.path);

  if (window.MathJax?.typesetClear) {
    window.MathJax.typesetClear([contentEl]);
  }

  contentEl.innerHTML = result.html;
  rewriteLocalImageSrcs(contentEl);
  wrapTables(contentEl);
  emptyStateEl.classList.add("hidden");
  missingStateEl.classList.add("hidden");
  errorStateEl.classList.add("hidden");
  currentDocPath = result.path;

  await setWindowTitle(formatWindowTitle(result.title));

  if (!reload) {
    requestAnimationFrame(() => {
      if (openToken === undefined || openToken === activeOpenToken) {
        loadingStateEl.classList.add("hidden");
        contentEl.classList.add("visible");
      }
    });
  }

  buildToc();

  if (reload) {
    scrollRoot.scrollTop = scrollTop;
    scheduleTypesetMath();
  } else if (savedViewState) {
    void restoreSavedViewState(savedViewState, openToken);
  } else {
    scheduleTypesetMath();
  }
}

function slugify(text) {
  return text
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s-]/gu, "")
    .replace(/\s+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

function headingId(heading) {
  if (heading.id) return heading.id;
  const anchor = heading.querySelector("a.anchor[id]");
  if (anchor?.id) return anchor.id;
  return "";
}

function ensureHeadingIds() {
  const used = new Set();
  for (const heading of contentEl.querySelectorAll("h1, h2, h3, h4")) {
    let id = headingId(heading);
    if (!id) {
      id = slugify(heading.textContent);
    }
    if (!id) continue;

    let unique = id;
    let suffix = 2;
    while (used.has(unique)) {
      unique = `${id}-${suffix}`;
      suffix += 1;
    }
    used.add(unique);
    heading.id = unique;
  }
}

function buildTocTree(headings) {
  const root = { level: 0, children: [] };
  const stack = [root];

  for (const heading of headings) {
    const level = Number(heading.tagName[1]);
    const node = {
      level,
      id: heading.id,
      text: heading.textContent.trim(),
      children: [],
    };

    while (stack.length > 1 && stack[stack.length - 1].level >= level) {
      stack.pop();
    }

    stack[stack.length - 1].children.push(node);
    stack.push(node);
  }

  return root.children;
}

function renderTocNode(node) {
  const li = document.createElement("li");
  li.className = `toc-item toc-level-${node.level}`;

  const hasChildren = node.children.length > 0;
  const row = document.createElement("div");
  row.className = "toc-row";

  if (hasChildren) {
    const expand = document.createElement("button");
    expand.type = "button";
    expand.className = "toc-expand";
    expand.setAttribute("aria-expanded", "true");
    expand.setAttribute("aria-label", `Collapse “${node.text}”`);
    expand.innerHTML = '<span class="toc-chevron" aria-hidden="true"></span>';
    row.appendChild(expand);
  } else {
    const spacer = document.createElement("span");
    spacer.className = "toc-spacer";
    spacer.setAttribute("aria-hidden", "true");
    row.appendChild(spacer);
  }

  const link = document.createElement("a");
  link.href = `#${node.id}`;
  link.textContent = node.text;
  link.className = "toc-link";
  row.appendChild(link);

  li.appendChild(row);

  if (hasChildren) {
    const childList = document.createElement("ul");
    childList.className = "toc-children";
    for (const child of node.children) {
      childList.appendChild(renderTocNode(child));
    }
    li.appendChild(childList);
  }

  return li;
}

function buildToc() {
  tocList.replaceChildren();
  ensureHeadingIds();

  const headings = contentEl.querySelectorAll("h1, h2, h3, h4");
  if (headings.length === 0) {
    tocToggle.classList.add("hidden");
    toggleToc(false);
    return;
  }

  const tree = document.createElement("ul");
  tree.className = "toc-tree";
  for (const node of buildTocTree(headings)) {
    tree.appendChild(renderTocNode(node));
  }

  tocList.appendChild(tree);
  tocToggle.classList.remove("hidden");
}

function toggleTocSection(button) {
  const item = button.closest(".toc-item");
  const children = item?.querySelector(":scope > .toc-children");
  if (!item || !children) return;

  const expanded = button.getAttribute("aria-expanded") === "true";
  const next = !expanded;
  const label = item.querySelector(".toc-link")?.textContent?.trim() ?? "section";

  button.setAttribute("aria-expanded", String(next));
  button.setAttribute("aria-label", `${next ? "Collapse" : "Expand"} “${label}”`);
  item.classList.toggle("toc-collapsed", !next);
  children.hidden = !next;
}

function wait(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function nextFrame() {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

let windowRevealed = false;

// Reveal the window only once the theme is applied and the first state is painted,
// so the user never sees the default light background flash before content.
async function revealWindow() {
  if (windowRevealed) return;
  windowRevealed = true;

  // Wait for a painted frame, but never block forever if frames are throttled while hidden.
  await Promise.race([nextFrame().then(nextFrame), wait(150)]);

  try {
    await getCurrentWindow().show();
  } catch (err) {
    console.warn("Failed to show window", err);
  }
}

async function openFile(path) {
  toggleSearch(false);
  flushViewState(currentDocPath, contentEl);
  const openToken = ++activeOpenToken;
  showLoadingState();

  try {
    const result = await invoke("render_markdown", { path });
    await applyDocument(result, { initial: true, openToken });
  } catch (err) {
    if (openToken !== activeOpenToken) return;

    currentDocPath = null;
    showMissingFile(String(err));

    await setWindowTitle("emede");
  }
}

async function handlePickAndOpenFile() {
  try {
    const selected = await invoke("plugin:dialog|open", {
      options: {
        multiple: false,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      },
    });
    if (selected) {
      await openFile(selected);
    }
  } catch (err) {
    console.warn("Failed to open file dialog", err);
  }
}

async function handleImportColorTemplate() {
  colorTemplateStatus.textContent =
    "Reading CSS template colors.";

  try {
    const selected = await invoke("plugin:dialog|open", {
      options: {
        multiple: false,
        filters: [{ name: "CSS", extensions: ["css"] }],
      },
    });
    if (!selected) return;

    const cssText = await invoke("read_color_template", { path: selected });
    const template = extractColorTemplate(cssText, colorTemplateLabelFromPath(selected));
    colorTemplates = [
      ...colorTemplates.filter((candidate) => candidate.id !== template.id),
      template,
    ];
    renderColorTemplates();
    await applyColorTemplate(template);
    colorTemplateStatus.textContent = `Imported ${template.label}.`;
  } catch (err) {
    console.warn("Failed to import color template", err);
    colorTemplateStatus.textContent = String(err);
  }
}

function toggleSettings(open) {
  const show = open ?? settingsPanel.classList.contains("hidden");
  settingsPanel.classList.toggle("hidden", !show);
  settingsPanel.setAttribute("aria-hidden", String(!show));
}

function toggleToc(open) {
  const show = open ?? tocPanel.classList.contains("hidden");
  tocPanel.classList.toggle("hidden", !show);
  tocPanel.setAttribute("aria-hidden", String(!show));
}

function toggleAbout(open) {
  const show = open ?? aboutOverlay.classList.contains("hidden");
  aboutOverlay.classList.toggle("hidden", !show);
  aboutOverlay.setAttribute("aria-hidden", String(!show));
  document.body.classList.toggle("has-modal", show);
}

function toggleSearch(open) {
  const show = open ?? searchBar.classList.contains("hidden");
  searchBar.classList.toggle("hidden", !show);
  searchBar.setAttribute("aria-hidden", String(!show));
  if (show) {
    searchInput.focus();
    searchInput.select();
  } else {
    searchInput.blur();
    if (findInPage) findInPage.stop();
    searchInput.value = "";
    searchCounter.textContent = "";
  }
}

function toggleTocPanel() {
  toggleToc(tocPanel.classList.contains("hidden"));
}

function wireToc() {
  tocToggle.addEventListener("click", () => toggleToc(true));
  tocClose.addEventListener("click", () => toggleToc(false));

  tocList.addEventListener("click", (event) => {
    const expandBtn = event.target.closest(".toc-expand");
    if (expandBtn && tocList.contains(expandBtn)) {
      event.preventDefault();
      toggleTocSection(expandBtn);
      return;
    }

    const link = event.target.closest("a.toc-link");
    if (!link || !tocList.contains(link)) return;

    event.preventDefault();
    const id = link.getAttribute("href").slice(1);
    const heading = contentEl.querySelector(`#${CSS.escape(id)}`);
    if (heading) {
      const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      heading.scrollIntoView({ behavior: reduceMotion ? "auto" : "smooth", block: "start" });
    }
    toggleToc(false);
  });
}

function wireExternalLinks() {
  contentEl.addEventListener("click", (event) => {
    const anchor = event.target.closest("a[href]");
    if (!anchor || !contentEl.contains(anchor)) return;

    const href = anchor.getAttribute("href");
    if (!href || href.startsWith("#")) return;

    let url;
    try {
      url = new URL(href, window.location.href);
    } catch {
      return;
    }

    if (!["http:", "https:", "mailto:", "tel:"].includes(url.protocol)) return;

    event.preventDefault();
    void openUrl(url.href).catch((err) => {
      console.warn("Failed to open link in system browser", err);
    });
  });
}

function wireTitlebar() {
  const win = getCurrentWindow();

  winMinimize.addEventListener("click", () => {
    void win.minimize();
  });

  winMaximize.addEventListener("click", () => {
    void win.toggleMaximize().then(syncMaximizeButton);
  });

  winClose.addEventListener("click", () => {
    void win.close();
  });

  void win.onResized(() => {
    void syncMaximizeButton();
  });
  void syncMaximizeButton();
}

function wireSettings() {
  settingsToggle.addEventListener("click", () => toggleSettings(true));
  settingsClose.addEventListener("click", () => toggleSettings(false));
  renderColorTemplates();
  importColorTemplateBtn.addEventListener("click", () => {
    void handleImportColorTemplate();
  });

  [
    settingFont,
    settingFontTitle,
    settingFontCode,
    settingSize,
    settingMargin,
    settingFg,
    settingBg,
    settingWindowFrame,
    settingKeybindings,
  ].forEach((el) => {
    el.addEventListener("input", scheduleSave);
  });

  settingKeybindings.addEventListener("change", () => {
    renderKeybindingHelp(keybindingsHelp, settingKeybindings.value);
  });

  settingJustify.addEventListener("input", scheduleSave);

  settingGpu.addEventListener("change", async () => {
    scheduleSave();
    if (initialGpuAccel === null) return;
    if (settingGpu.checked === initialGpuAccel) return;

    await window.__TAURI__.dialog.message(
      "GPU acceleration change requires a restart to take effect. The application will now restart.",
      { title: "emede", kind: "info" },
    );

    invoke("restart_app");
  });

  settingSize.addEventListener("input", () => {
    settingSizeLabel.textContent = `${Number(settingSize.value)}pt`;
  });

  settingMargin.addEventListener("input", () => {
    settingMarginLabel.textContent = `${Number(settingMargin.value)}%`;
  });

  document.querySelectorAll("[data-font-preset]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const preset = FONT_PRESETS[btn.dataset.fontPreset];
      if (!preset) return;
      const settings = {
        ...(currentSettings || settingsFromForm()),
        ...preset,
      };
      applySettings(settings);
      await invoke("set_settings", { settings });
    });
  });

  aboutLink.addEventListener("click", () => toggleAbout(true));
  aboutClose.addEventListener("click", () => toggleAbout(false));
  aboutOverlay.addEventListener("click", (e) => {
    if (e.target === aboutOverlay) toggleAbout(false);
  });
}

function adjustFontSize(delta) {
  const currentPt = toPt(currentSettings?.font_size, 12);
  const newPt = Math.min(32, Math.max(8, currentPt + delta));
  settingSize.value = newPt;
  settingSizeLabel.textContent = `${newPt}pt`;
  applySettings(settingsFromForm());
  scheduleSave();
}

function resetFontSize() {
  const pt = initialFontSize;
  settingSize.value = pt;
  settingSizeLabel.textContent = `${pt}pt`;
  applySettings(settingsFromForm());
  scheduleSave();
}

function wireViewState() {
  let closingAfterFlush = false;
  void getCurrentWindow().onCloseRequested(async (event) => {
    if (closingAfterFlush) return;
    event.preventDefault();
    closingAfterFlush = true;
    try {
      if (currentDocPath) {
        await flushViewStateAsync(currentDocPath, contentEl);
      }
    } catch (err) {
      console.warn("Failed to flush view state on close", err);
    }
    try {
      await getCurrentWindow().close();
    } catch (err) {
      console.warn("Failed to close window", err);
      closingAfterFlush = false;
    }
  });
}

function updateSearchCounter() {
  if (!findInPage) return;
  const count = findInPage.matchCount;
  const current = findInPage.currentMatchNumber;
  searchCounter.textContent = count > 0 ? `${current}/${count}` : count === 0 ? "0/0" : "";
}

function submitSearch(forward = true) {
  const query = searchInput.value.trim();
  if (findInPage.query !== query) {
    findInPage.find(searchInput.value);
  } else if (findInPage.matchCount > 0) {
    if (forward) findInPage.next();
    else findInPage.prev();
  } else {
    findInPage.find(searchInput.value);
  }
  updateSearchCounter();
}

function wireSearch() {
  findInPage = new FindInPage(contentEl);

  searchInput.addEventListener("input", () => {
    const query = searchInput.value.trim();
    if (findInPage.query && findInPage.query !== query) {
      findInPage.stop();
      updateSearchCounter();
    }
  });

  searchInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submitSearch(true);
      event.stopPropagation();
      return;
    }
    if (event.key === "Enter" && event.shiftKey) {
      event.preventDefault();
      submitSearch(false);
      event.stopPropagation();
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      toggleSearch(false);
      event.stopPropagation();
      return;
    }
  });

  searchPrev.addEventListener("click", () => submitSearch(false));

  searchNext.addEventListener("click", () => submitSearch(true));

  searchClose.addEventListener("click", () => toggleSearch(false));
}

function wireKeybindings() {
  createKeybindingController({
    getKeybindingMode: () => currentSettings?.keybindings ?? settingKeybindings.value,
    toggleSettings,
    toggleToc: toggleTocPanel,
    toggleAbout,
    toggleSearch,
    adjustFontSize,
    resetFontSize,
    settingsPanel,
    tocPanel,
    aboutOverlay,
    searchPanel: searchBar,
  });
}

async function boot() {
  populateFontOptions();
  await loadBundledColorTemplates();
  wireExternalLinks();
  wireToc();
  wireTitlebar();
  wireSettings();
  wireViewState();
  wireSearch();
  wireKeybindings();

  openFileBtn.addEventListener("click", () => {
    void handlePickAndOpenFile();
  });

  const startupFilePromise = invoke("get_startup_file");
  const settingsPromise = invoke("get_settings");

  void listen("file-to-open", (event) => {
    if (event.payload) {
      openFile(event.payload);
    }
  });

  let startupFile = null;
  try {
    startupFile = await startupFilePromise;
    const settings = await settingsPromise;
    applySettings(settings);
    initialFontSize = toPt(settings.font_size, 12);
    initialGpuAccel = settings.gpu_acceleration;
  } catch (err) {
    console.warn("Startup initialization failed", err);
  }

  // Paint the first meaningful frame (loader or empty state) before the window appears.
  if (startupFile) {
    showLoadingState();
  } else {
    showEmptyState();
  }

  await revealWindow();

  if (startupFile) {
    await openFile(startupFile);
  }
}

window.addEventListener("DOMContentLoaded", boot);
