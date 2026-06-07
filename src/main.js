const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

const DEFAULT_FONT = '"Literata", "Source Serif 4", "Noto Serif", serif';

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
      { label: "JetBrains Mono", value: '"JetBrains Mono", monospace' },
      { label: "Fira Code", value: '"Fira Code", monospace' },
      { label: "DejaVu Sans Mono", value: '"DejaVu Sans Mono", monospace' },
    ],
  },
];

const PRESETS = {
  light: {
    font_family: DEFAULT_FONT,
    font_size: "12pt",
    color_fg: "#2c2c2c",
    color_bg: "#faf8f5",
    margin: "72pt",
  },
  sepia: {
    font_family: DEFAULT_FONT,
    font_size: "12pt",
    color_fg: "#433422",
    color_bg: "#f4ecd8",
    margin: "72pt",
  },
  dark: {
    font_family: DEFAULT_FONT,
    font_size: "12pt",
    color_fg: "#d4d0c8",
    color_bg: "#1a1a1a",
    margin: "72pt",
  },
};

const contentEl = document.getElementById("content");
const emptyStateEl = document.getElementById("empty-state");
const errorStateEl = document.getElementById("error-state");
const loadingStateEl = document.getElementById("loading-state");
const missingStateEl = document.getElementById("missing-state");
const missingMessageEl = document.getElementById("missing-message");
const settingsPanel = document.getElementById("settings-panel");
const settingsToggle = document.getElementById("settings-toggle");
const settingsClose = document.getElementById("settings-close");
const settingFont = document.getElementById("setting-font");
const settingSize = document.getElementById("setting-size");
const settingSizeLabel = document.getElementById("setting-size-label");
const settingMargin = document.getElementById("setting-margin");
const settingMarginLabel = document.getElementById("setting-margin-label");
const settingFg = document.getElementById("setting-fg");
const settingBg = document.getElementById("setting-bg");

let currentSettings = null;
let saveTimer = null;
let activeOpenToken = 0;

function populateFontOptions() {
  for (const group of FONT_GROUPS) {
    const optgroup = document.createElement("optgroup");
    optgroup.label = group.label;
    for (const font of group.fonts) {
      const option = document.createElement("option");
      option.value = font.value;
      option.textContent = font.label;
      option.style.fontFamily = font.value;
      optgroup.appendChild(option);
    }
    settingFont.appendChild(optgroup);
  }
}

// Parse a CSS length to an integer point value, tolerating legacy `rem` values.
function toPt(value, fallback) {
  const n = parseFloat(value);
  if (!Number.isFinite(n)) return fallback;
  if (String(value).includes("rem")) return Math.round(n * 12);
  return Math.round(n);
}

function applySettings(settings) {
  currentSettings = settings;
  const sizePt = toPt(settings.font_size, 12);
  const marginPt = toPt(settings.margin, 72);

  document.documentElement.style.setProperty("--font-serif", settings.font_family);
  document.documentElement.style.setProperty("--font-size", `${sizePt}pt`);
  document.documentElement.style.setProperty("--reader-margin", `${marginPt}pt`);
  document.documentElement.style.setProperty("--color-fg", settings.color_fg);
  document.documentElement.style.setProperty("--color-bg", settings.color_bg);

  settingFont.value = settings.font_family;
  if (!settingFont.value) settingFont.value = DEFAULT_FONT;
  settingSize.value = sizePt;
  settingSizeLabel.textContent = `${sizePt}pt`;
  settingMargin.value = marginPt;
  settingMarginLabel.textContent = `${marginPt}pt`;
  settingFg.value = settings.color_fg;
  settingBg.value = settings.color_bg;
}

function settingsFromForm() {
  return {
    font_family: settingFont.value || DEFAULT_FONT,
    font_size: `${Number(settingSize.value)}pt`,
    color_fg: settingFg.value,
    color_bg: settingBg.value,
    margin: `${Number(settingMargin.value)}pt`,
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

function showEmptyState() {
  contentEl.innerHTML = "";
  contentEl.classList.remove("visible");
  loadingStateEl.classList.add("hidden");
  emptyStateEl.classList.remove("hidden");
  missingStateEl.classList.add("hidden");
  errorStateEl.classList.add("hidden");
}

function showLoadingState() {
  contentEl.innerHTML = "";
  contentEl.classList.remove("visible");
  loadingStateEl.classList.remove("hidden");
  emptyStateEl.classList.add("hidden");
  missingStateEl.classList.add("hidden");
  errorStateEl.classList.add("hidden");
}

function showMissingFile(message) {
  contentEl.innerHTML = "";
  contentEl.classList.remove("visible");
  loadingStateEl.classList.add("hidden");
  emptyStateEl.classList.add("hidden");
  missingMessageEl.textContent = message.replace(/^File not found:\s*/, "");
  missingStateEl.classList.remove("hidden");
  errorStateEl.classList.add("hidden");
}

function showError(message) {
  contentEl.innerHTML = "";
  contentEl.classList.remove("visible");
  loadingStateEl.classList.add("hidden");
  emptyStateEl.classList.add("hidden");
  missingStateEl.classList.add("hidden");
  errorStateEl.textContent = message;
  errorStateEl.classList.remove("hidden");
}

async function typesetMath() {
  if (!window.MathJax) return;

  if (window.MathJax.startup?.promise) {
    await window.MathJax.startup.promise;
  }

  if (typeof window.MathJax.typesetClear === "function") {
    window.MathJax.typesetClear([contentEl]);
  }

  if (typeof window.MathJax.typesetPromise === "function") {
    await window.MathJax.typesetPromise([contentEl]);
  }
}

function wait(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function prepareDocument() {
  const mathReady = typesetMath().catch((err) => {
    console.warn("MathJax typesetting failed", err);
  });

  // MathJax can occasionally stall during startup; never let it trap the reader behind the loader.
  await Promise.race([mathReady, wait(1200)]);
}

async function openFile(path) {
  const openToken = ++activeOpenToken;
  showLoadingState();

  try {
    const result = await invoke("render_markdown", { path });
    if (openToken !== activeOpenToken) return;

    contentEl.innerHTML = result.html;
    emptyStateEl.classList.add("hidden");
    missingStateEl.classList.add("hidden");
    errorStateEl.classList.add("hidden");

    const win = getCurrentWindow();
    await win.setTitle(`${result.title} — emede`);

    await prepareDocument();
    requestAnimationFrame(() => {
      if (openToken === activeOpenToken) {
        loadingStateEl.classList.add("hidden");
        contentEl.classList.add("visible");
      }
    });
  } catch (err) {
    if (openToken !== activeOpenToken) return;

    showMissingFile(String(err));

    const win = getCurrentWindow();
    await win.setTitle("emede");
  }
}

function toggleSettings(open) {
  const show = open ?? settingsPanel.classList.contains("hidden");
  settingsPanel.classList.toggle("hidden", !show);
  settingsPanel.setAttribute("aria-hidden", String(!show));
}

function wireSettings() {
  settingsToggle.addEventListener("click", () => toggleSettings(true));
  settingsClose.addEventListener("click", () => toggleSettings(false));

  [settingFont, settingSize, settingMargin, settingFg, settingBg].forEach((el) => {
    el.addEventListener("input", scheduleSave);
  });

  settingSize.addEventListener("input", () => {
    settingSizeLabel.textContent = `${Number(settingSize.value)}pt`;
  });

  settingMargin.addEventListener("input", () => {
    settingMarginLabel.textContent = `${Number(settingMargin.value)}pt`;
  });

  document.querySelectorAll("[data-preset]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const preset = PRESETS[btn.dataset.preset];
      if (!preset) return;
      applySettings(preset);
      await invoke("set_settings", { settings: preset });
    });
  });

  document.addEventListener("keydown", (e) => {
    if (e.ctrlKey && e.key === ",") {
      e.preventDefault();
      toggleSettings(true);
    }
    if (e.key === "Escape" && !settingsPanel.classList.contains("hidden")) {
      toggleSettings(false);
    }
  });
}

async function boot() {
  populateFontOptions();
  wireSettings();

  const startupFilePromise = invoke("get_startup_file");

  await listen("file-to-open", (event) => {
    if (event.payload) {
      openFile(event.payload);
    }
  });

  const startupFile = await startupFilePromise;
  if (startupFile) {
    showLoadingState();
  }

  const settings = await invoke("get_settings");
  applySettings(settings);

  if (startupFile) {
    await openFile(startupFile);
  } else {
    showEmptyState();
  }
}

window.addEventListener("DOMContentLoaded", boot);
