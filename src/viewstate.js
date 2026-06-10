import { getScrollRoot } from "./scroll.js";

const { invoke } = window.__TAURI__.core;

const SAVE_DEBOUNCE_MS = 400;

let saveTimer = null;
let pendingSave = null;

function elementOffsetInScrollRoot(element, scrollRoot) {
  const elementRect = element.getBoundingClientRect();
  const rootRect = scrollRoot.getBoundingClientRect();
  return elementRect.top - rootRect.top + scrollRoot.scrollTop;
}

function maxScrollTop(scrollRoot) {
  return Math.max(0, scrollRoot.scrollHeight - scrollRoot.clientHeight);
}

function scrollFraction(scrollRoot) {
  const max = maxScrollTop(scrollRoot);
  if (max <= 0) return 0;
  return scrollRoot.scrollTop / max;
}

function clampScrollTop(scrollRoot, value) {
  return Math.min(maxScrollTop(scrollRoot), Math.max(0, value));
}

export function captureViewState(scrollRoot, contentEl) {
  const viewportTop = scrollRoot.scrollTop;
  let anchorId = null;
  let anchorOffset = 0;

  const headings = contentEl.querySelectorAll("h1, h2, h3, h4");
  for (const heading of headings) {
    if (!heading.id) continue;
    const headingTop = elementOffsetInScrollRoot(heading, scrollRoot);
    if (headingTop <= viewportTop + 1) {
      anchorId = heading.id;
      anchorOffset = viewportTop - headingTop;
    } else {
      break;
    }
  }

  return {
    anchor_id: anchorId,
    anchor_offset: anchorOffset,
    scroll_top: viewportTop,
    scroll_fraction: scrollFraction(scrollRoot),
  };
}

export function applyViewState(scrollRoot, contentEl, state) {
  if (!state) return false;

  let target = null;

  if (state.anchor_id) {
    const heading = contentEl.querySelector(`#${CSS.escape(state.anchor_id)}`);
    if (heading) {
      const headingTop = elementOffsetInScrollRoot(heading, scrollRoot);
      target = headingTop + (state.anchor_offset ?? 0);
    }
  }

  if (target === null && Number.isFinite(state.scroll_fraction)) {
    target = state.scroll_fraction * maxScrollTop(scrollRoot);
  }

  if (target === null && Number.isFinite(state.scroll_top)) {
    target = state.scroll_top;
  }

  if (target === null) return false;

  scrollRoot.scrollTop = clampScrollTop(scrollRoot, target);
  return true;
}

export async function loadViewState(path) {
  if (!path) return null;
  try {
    return await invoke("get_view_state", { path });
  } catch (err) {
    console.warn("Failed to load view state", err);
    return null;
  }
}

async function persistViewState(path, state) {
  if (!path || !state) return;
  try {
    await invoke("set_view_state", { path, state });
  } catch (err) {
    console.warn("Failed to save view state", err);
  }
}

export function saveViewState(path, scrollRoot, contentEl, { immediate = false } = {}) {
  if (!path) return;

  const state = captureViewState(scrollRoot, contentEl);
  pendingSave = { path, state };

  clearTimeout(saveTimer);
  if (immediate) {
    pendingSave = null;
    void persistViewState(path, state);
    return;
  }

  saveTimer = setTimeout(() => {
    const next = pendingSave;
    pendingSave = null;
    if (!next) return;
    void persistViewState(next.path, next.state);
  }, SAVE_DEBOUNCE_MS);
}

export function flushViewState(path, contentEl) {
  if (!path) return;
  saveViewState(path, getScrollRoot(), contentEl, { immediate: true });
}

export async function flushViewStateAsync(path, contentEl) {
  if (!path) return;
  clearTimeout(saveTimer);
  pendingSave = null;
  await persistViewState(path, captureViewState(getScrollRoot(), contentEl));
}

export function getScrollEventTarget() {
  const root = getScrollRoot();
  return root === document.documentElement ? window : root;
}
