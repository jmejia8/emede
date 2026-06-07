const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;
const { openUrl } = window.__TAURI__.opener;

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

const DEFAULT_MARGIN_PERCENT = 10;

const PRESETS = {
  light: {
    font_family: DEFAULT_FONT,
    font_size: "12pt",
    color_fg: "#2c2c2c",
    color_bg: "#faf8f5",
    margin: "10%",
  },
  sepia: {
    font_family: DEFAULT_FONT,
    font_size: "12pt",
    color_fg: "#433422",
    color_bg: "#f4ecd8",
    margin: "10%",
  },
  dark: {
    font_family: DEFAULT_FONT,
    font_size: "12pt",
    color_fg: "#d4d0c8",
    color_bg: "#1a1a1a",
    margin: "10%",
  },
  gruvbox: {
    font_family: DEFAULT_FONT,
    font_size: "12pt",
    color_fg: "#ebdbb2",
    color_bg: "#282828",
    margin: "10%",
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

function applySettings(settings) {
  currentSettings = settings;
  const sizePt = toPt(settings.font_size, 12);
  const marginPercent = toMarginPercent(settings.margin);

  document.documentElement.style.setProperty("--font-serif", settings.font_family);
  document.documentElement.style.setProperty("--font-size", `${sizePt}pt`);
  document.documentElement.style.setProperty("--reader-margin", `${marginPercent}%`);
  document.documentElement.style.setProperty("--color-fg", settings.color_fg);
  document.documentElement.style.setProperty("--color-bg", settings.color_bg);

  settingFont.value = settings.font_family;
  if (!settingFont.value) settingFont.value = DEFAULT_FONT;
  settingSize.value = sizePt;
  settingSizeLabel.textContent = `${sizePt}pt`;
  settingMargin.value = marginPercent;
  settingMarginLabel.textContent = `${marginPercent}%`;
  settingFg.value = settings.color_fg;
  settingBg.value = settings.color_bg;
}

function settingsFromForm() {
  return {
    font_family: settingFont.value || DEFAULT_FONT,
    font_size: `${Number(settingSize.value)}pt`,
    color_fg: settingFg.value,
    color_bg: settingBg.value,
    margin: `${Number(settingMargin.value)}%`,
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

async function applyDocument(result, { initial = false, reload = false, openToken } = {}) {
  if (openToken !== undefined && openToken !== activeOpenToken) return;

  const scrollTop = reload ? document.documentElement.scrollTop : 0;

  if (window.MathJax?.typesetClear) {
    window.MathJax.typesetClear([contentEl]);
  }

  contentEl.innerHTML = result.html;
  emptyStateEl.classList.add("hidden");
  missingStateEl.classList.add("hidden");
  errorStateEl.classList.add("hidden");

  const win = getCurrentWindow();
  await win.setTitle(`${result.title} — emede`);

  if (!reload) {
    requestAnimationFrame(() => {
      if (openToken === undefined || openToken === activeOpenToken) {
        loadingStateEl.classList.add("hidden");
        contentEl.classList.add("visible");
      }
    });
  }

  if (reload) {
    document.documentElement.scrollTop = scrollTop;
  }

  scheduleTypesetMath();
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
  const openToken = ++activeOpenToken;
  showLoadingState();

  try {
    const result = await invoke("render_markdown", { path });
    await applyDocument(result, { initial: true, openToken });
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
    settingMarginLabel.textContent = `${Number(settingMargin.value)}%`;
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
  wireExternalLinks();
  wireSettings();

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
