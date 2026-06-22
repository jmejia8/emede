import { getScrollRoot, getScrollViewportHeight } from "./scroll.js";

const KEYBINDING_MODES = new Set(["default", "vim", "emacs", "common"]);

export const KEYBINDING_HELP = {
  default: [
    ["Ctrl+F", "Find in page"],
    ["Ctrl+P", "Print / Save as PDF"],
    ["Ctrl+,", "Open settings"],
    ["Ctrl++ / Ctrl+- / Ctrl+0", "Increase / decrease / reset font size"],
    ["Ctrl+Shift+T", "Toggle table of contents"],
    ["Escape", "Close panels"],
  ],
  vim: [
    ["Ctrl+F", "Find in page"],
    ["Ctrl+P", "Print / Save as PDF"],
    ["j / k", "Scroll down / up one line"],
    ["d / u", "Half page down / up"],
    ["f / b / Space", "Page down / page up / page down"],
    ["Ctrl+d / Ctrl+u", "Half page down / up"],
    ["Ctrl+f / Ctrl+b", "Page down / up"],
    ["gg", "Go to top"],
    ["G", "Go to bottom"],
    ["Ctrl+,", "Open settings"],
    ["Ctrl++ / Ctrl+- / Ctrl+0", "Increase / decrease / reset font size"],
    ["Ctrl+Shift+T", "Toggle table of contents"],
    ["Escape", "Close panels"],
  ],
  emacs: [
    ["Ctrl+F", "Find in page"],
    ["Ctrl+P", "Print / Save as PDF"],
    ["Ctrl+n / Ctrl+p", "Scroll down / up one line"],
    ["Ctrl+v", "Page down"],
    ["Alt+v", "Page up"],
    ["Ctrl+Home / Ctrl+End", "Go to top / bottom"],
    ["Ctrl+,", "Open settings"],
    ["Ctrl++ / Ctrl+- / Ctrl+0", "Increase / decrease / reset font size"],
    ["Ctrl+Shift+T", "Toggle table of contents"],
    ["Escape", "Close panels"],
  ],
  common: [
    ["Ctrl+F", "Find in page"],
    ["Ctrl+P", "Print / Save as PDF"],
    ["j / k", "Scroll down / up one line"],
    ["Space", "Page down"],
    ["Shift+Space", "Page up"],
    ["Ctrl+Home / Ctrl+End", "Go to top / bottom"],
    ["Ctrl+,", "Open settings"],
    ["Ctrl++ / Ctrl+- / Ctrl+0", "Increase / decrease / reset font size"],
    ["Ctrl+Shift+T", "Toggle table of contents"],
    ["Escape", "Close panels"],
  ],
};

export function normalizeKeybindingMode(mode) {
  return KEYBINDING_MODES.has(mode) ? mode : "default";
}

function isTypingTarget(target) {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === "TEXTAREA") return true;
  if (tag === "SELECT") return true;
  if (tag === "INPUT") {
    const type = target.type;
    return !["button", "submit", "reset", "checkbox", "radio", "range", "color"].includes(type);
  }
  return false;
}

function lineHeightPx() {
  const root = document.documentElement;
  const body = document.body;
  const fontSize = parseFloat(getComputedStyle(root).fontSize) || 16;
  const lineHeight = parseFloat(getComputedStyle(body).lineHeight);
  if (Number.isFinite(lineHeight)) return lineHeight;
  return fontSize * 1.75;
}

function scrollByLines(lines) {
  getScrollRoot().scrollBy({
    top: lines * lineHeightPx(),
    behavior: "auto",
  });
}

function scrollByViewport(fraction) {
  getScrollRoot().scrollBy({
    top: getScrollViewportHeight() * fraction,
    behavior: "auto",
  });
}

function scrollToTop() {
  getScrollRoot().scrollTop = 0;
}

function scrollToBottom() {
  const root = getScrollRoot();
  root.scrollTop = root.scrollHeight;
}

function handleAppShortcuts(event, actions) {
  if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "p") {
    event.preventDefault();
    actions.print?.();
    return true;
  }

  if (event.ctrlKey && (event.key === "=" || event.key === "+")) {
    event.preventDefault();
    actions.adjustFontSize(1);
    return true;
  }

  if (event.ctrlKey && event.key === "-") {
    event.preventDefault();
    actions.adjustFontSize(-1);
    return true;
  }

  if (event.ctrlKey && event.key === "0") {
    event.preventDefault();
    actions.resetFontSize();
    return true;
  }

  if (event.ctrlKey && event.key === ",") {
    event.preventDefault();
    actions.toggleSettings(true);
    return true;
  }

  if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === "t") {
    event.preventDefault();
    actions.toggleToc();
    return true;
  }

  if (event.key === "Escape") {
    let handled = false;
    if (actions.searchPanel && !actions.searchPanel.classList.contains("hidden")) {
      if (actions.toggleSearch) actions.toggleSearch(false);
      handled = true;
    }
    if (actions.aboutOverlay && !actions.aboutOverlay.classList.contains("hidden")) {
      actions.toggleAbout(false);
      handled = true;
    }
    if (!actions.settingsPanel.classList.contains("hidden")) {
      actions.toggleSettings(false);
      handled = true;
    }
    if (!actions.tocPanel.classList.contains("hidden")) {
      actions.toggleToc(false);
      handled = true;
    }
    if (handled) {
      event.preventDefault();
      return true;
    }
  }

  return false;
}

function handleVimKey(event, state) {
  const key = event.key;

  if (key === "g") {
    if (state.pendingKey === "g" && Date.now() - state.pendingAt < 450) {
      event.preventDefault();
      scrollToTop();
      state.pendingKey = null;
      return true;
    }
    state.pendingKey = "g";
    state.pendingAt = Date.now();
    return true;
  }

  state.pendingKey = null;

  if (key === "G") {
    event.preventDefault();
    scrollToBottom();
    return true;
  }

  if (key === "j") {
    event.preventDefault();
    scrollByLines(1);
    return true;
  }

  if (key === "k") {
    event.preventDefault();
    scrollByLines(-1);
    return true;
  }

  if (key === "d" && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    scrollByViewport(0.5);
    return true;
  }

  if (key === "u" && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    scrollByViewport(-0.5);
    return true;
  }

  if ((key === "f" || key === " ") && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    scrollByViewport(0.92);
    return true;
  }

  if (key === "b" && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    scrollByViewport(-0.92);
    return true;
  }

  if (event.ctrlKey && key === "d") {
    event.preventDefault();
    scrollByViewport(0.5);
    return true;
  }

  if (event.ctrlKey && key === "u") {
    event.preventDefault();
    scrollByViewport(-0.5);
    return true;
  }

  if (event.ctrlKey && key === "f") {
    event.preventDefault();
    scrollByViewport(0.92);
    return true;
  }

  if (event.ctrlKey && key === "b") {
    event.preventDefault();
    scrollByViewport(-0.92);
    return true;
  }

  return false;
}

function handleEmacsKey(event) {
  if (event.ctrlKey && !event.altKey && !event.metaKey && event.key === "n") {
    event.preventDefault();
    scrollByLines(1);
    return true;
  }

  if (event.ctrlKey && !event.altKey && !event.metaKey && event.key === "p") {
    event.preventDefault();
    scrollByLines(-1);
    return true;
  }

  if (event.ctrlKey && !event.altKey && !event.metaKey && event.key === "v") {
    event.preventDefault();
    scrollByViewport(0.92);
    return true;
  }

  if (event.altKey && !event.ctrlKey && !event.metaKey && event.key === "v") {
    event.preventDefault();
    scrollByViewport(-0.92);
    return true;
  }

  if (event.ctrlKey && event.key === "Home") {
    event.preventDefault();
    scrollToTop();
    return true;
  }

  if (event.ctrlKey && event.key === "End") {
    event.preventDefault();
    scrollToBottom();
    return true;
  }

  return false;
}

function handleCommonKey(event) {
  const key = event.key;

  if (key === "j" && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    scrollByLines(1);
    return true;
  }

  if (key === "k" && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    scrollByLines(-1);
    return true;
  }

  if (key === " " && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    scrollByViewport(0.92);
    return true;
  }

  if (key === " " && event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    scrollByViewport(-0.92);
    return true;
  }

  if (event.ctrlKey && event.key === "Home") {
    event.preventDefault();
    scrollToTop();
    return true;
  }

  if (event.ctrlKey && event.key === "End") {
    event.preventDefault();
    scrollToBottom();
    return true;
  }

  return false;
}

export function renderKeybindingHelp(container, mode) {
  const normalized = normalizeKeybindingMode(mode);
  const rows = KEYBINDING_HELP[normalized] ?? KEYBINDING_HELP.default;
  container.replaceChildren();

  const list = document.createElement("dl");
  list.className = "keybindings-list";

  for (const [keys, description] of rows) {
    const dt = document.createElement("dt");
    dt.textContent = keys;

    const dd = document.createElement("dd");
    dd.textContent = description;

    list.appendChild(dt);
    list.appendChild(dd);
  }

  container.appendChild(list);
}

export function createKeybindingController(actions) {
  const state = {
    pendingKey: null,
    pendingAt: 0,
    getMode: actions.getKeybindingMode,
  };

  document.addEventListener("keydown", (event) => {
    if (event.defaultPrevented || event.isComposing) return;

    if ((event.ctrlKey || event.metaKey) && event.key === "f") {
      event.preventDefault();
      if (actions.toggleSearch) actions.toggleSearch();
      return;
    }

    if (isTypingTarget(event.target)) return;

    if (handleAppShortcuts(event, actions)) return;

    const mode = normalizeKeybindingMode(state.getMode());
    if (mode === "default") return;

    if (mode === "vim" && handleVimKey(event, state)) return;
    if (mode === "emacs" && handleEmacsKey(event)) return;
    if (mode === "common" && handleCommonKey(event)) return;
  });
}
