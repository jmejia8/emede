/**
 * Image zoom overlay.
 *
 * Diagrams and images that the reading column cannot show at full detail get
 * an "expand" button on hover. It opens a full-window viewer with two tools:
 * with the zoom tool dragging selects a rectangle to zoom into, with the hand
 * tool dragging pans. Space+drag or middle-drag pans under either tool, the
 * wheel zooms around the cursor, and Back pops a zoom-history stack.
 *
 * Which images qualify is decided at hover time, so a resize or a font/margin
 * change needs no bookkeeping:
 *   - every Mermaid diagram and every SVG image, since vectors always reward zoom
 *   - a bitmap only when it is shown below its natural size, or does not fit
 *     in the window. One at 100% or more that fits gets nothing.
 *
 * Everything in the viewer is an `<svg>`, so every zoom is a `viewBox` change.
 * Vector content therefore re-renders crisply at every level, and a rectangle
 * selection maps exactly onto the next `viewBox`. A bitmap goes through an SVG
 * `<image>`, which switches to `pixelated` past 1:1 so pixels stay honest
 * instead of being smeared.
 *
 * The content DOM is never restructured (no wrappers), since `changes.js`,
 * `find.js` and `stats.js` all walk it. The hover button is a single fixed
 * element placed over whichever target is under the pointer.
 */

const SVG_NS = "http://www.w3.org/2000/svg";
const FIT_MARGIN = 0.04;
const MIN_RECT_PX = 6;
const MAX_SCALE_FACTOR = 200;
const WHEEL_STEP = 1.0015;
const KEY_ZOOM = 1.4;
const KEY_PAN = 0.12;
const WHEEL_SESSION_MS = 400;

function isSvgImageSrc(src) {
  if (!src) return false;
  if (/^data:image\/svg\+xml/i.test(src)) return true;
  const path = src.split(/[?#]/, 1)[0];
  return /\.svg$/i.test(path) || /\.svg$/i.test(safeDecode(path));
}

function safeDecode(value) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

// Stroked with currentColor so the tool icons follow the theme, which a ✋
// glyph would not (it renders as a colour emoji).
const ICON_ZOOM_RECT =
  '<svg viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true">' +
  '<rect x="2.5" y="2.5" width="10" height="10" stroke-dasharray="2.2 1.8"/>' +
  '<circle cx="12.5" cy="12.5" r="3.2"/><path d="M14.8 14.8 18 18"/></svg>';
const ICON_HAND =
  '<svg viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
  '<path d="M6.5 10V4.2a1.2 1.2 0 0 1 2.4 0V9.5M8.9 9V3a1.2 1.2 0 0 1 2.4 0v6M11.3 9V3.8a1.2 1.2 0 0 1 2.4 0V10"/>' +
  '<path d="M13.7 10V6.2a1.2 1.2 0 0 1 2.4 0v5.3c0 3.3-2.3 6-5.4 6-2.2 0-3.3-.8-4.6-2.6L3.8 11.5a1.2 1.2 0 0 1 1.9-1.4l.8 1"/></svg>';

const HINTS = {
  zoom: "Drag to zoom · Space or middle-drag to pan · Wheel to zoom · M hand · Backspace back · 0 fit · Esc close",
  pan: "Drag to pan · Wheel to zoom · Z rectangle zoom · Backspace back · 0 fit · Esc close",
};

function button(label, title, text) {
  const el = document.createElement("button");
  el.type = "button";
  el.className = "image-viewer-btn";
  el.title = title;
  el.setAttribute("aria-label", label);
  el.textContent = text;
  return el;
}

export function createImageZoom({ contentEl }) {
  // ---------------------------------------------------------------- hover ---

  const hoverBtn = document.createElement("button");
  hoverBtn.type = "button";
  hoverBtn.id = "image-zoom-button";
  hoverBtn.className = "image-zoom-button hidden";
  hoverBtn.title = "Zoom";
  hoverBtn.setAttribute("aria-label", "Open in zoom viewer");
  hoverBtn.textContent = "⤢";
  document.body.append(hoverBtn);

  let hoverTarget = null;
  let hoverFrame = 0;
  let lastPointer = null;

  /** The element a pointer event is over, if it is a zoom candidate at all. */
  function candidateFrom(node) {
    if (!(node instanceof Element) || !contentEl.contains(node)) return null;
    const mermaid = node.closest(".mermaid");
    if (mermaid) {
      const svg = mermaid.querySelector(":scope > svg");
      return svg ? { kind: "mermaid", el: svg, box: mermaid } : null;
    }
    const img = node.closest("img");
    if (img && img.closest(".prose") && !img.closest(".image-fallback")) {
      return { kind: "img", el: img, box: img };
    }
    return null;
  }

  function isEligible(candidate) {
    if (candidate.kind === "mermaid") return true;
    const img = candidate.el;
    if (!img.complete || img.naturalWidth === 0) return false;
    if (isSvgImageSrc(img.getAttribute("src"))) return true;
    const rect = img.getBoundingClientRect();
    // One CSS pixel of slack: a fractional layout width is not a downscale.
    const downscaled =
      img.naturalWidth > rect.width + 1 || img.naturalHeight > rect.height + 1;
    const overflows = rect.width > window.innerWidth || rect.height > window.innerHeight;
    return downscaled || overflows;
  }

  function placeHoverButton() {
    if (!hoverTarget?.el.isConnected) {
      hideHoverButton();
      return;
    }
    const rect = hoverTarget.el.getBoundingClientRect();
    const top = Math.max(rect.top, 0) + 8;
    const right = Math.min(rect.right, window.innerWidth) - 8;
    if (rect.bottom < top + 32 || right - 32 < rect.left) {
      hoverBtn.classList.add("hidden");
      return;
    }
    hoverBtn.style.top = `${top}px`;
    hoverBtn.style.left = `${right - hoverBtn.offsetWidth}px`;
    hoverBtn.classList.remove("hidden");
  }

  function setHoverTarget(candidate) {
    if (hoverTarget && hoverTarget.el !== candidate?.el) {
      hoverTarget.box.classList.remove("zoomable");
    }
    hoverTarget = candidate;
    if (!candidate) {
      hoverBtn.classList.add("hidden");
      return;
    }
    candidate.box.classList.add("zoomable");
    placeHoverButton();
  }

  function hideHoverButton() {
    setHoverTarget(null);
  }

  function evaluatePointer() {
    hoverFrame = 0;
    if (!lastPointer || isOpen()) return;
    const under = document.elementFromPoint(lastPointer.x, lastPointer.y);
    if (under === hoverBtn) {
      placeHoverButton();
      return;
    }
    const candidate = candidateFrom(under);
    setHoverTarget(candidate && isEligible(candidate) ? candidate : null);
  }

  function scheduleEvaluate() {
    if (!hoverFrame) hoverFrame = requestAnimationFrame(evaluatePointer);
  }

  document.addEventListener("mousemove", (event) => {
    lastPointer = { x: event.clientX, y: event.clientY };
    scheduleEvaluate();
  }, { passive: true });

  // Scrolling moves the target out from under a still pointer, and a resize can
  // change whether it qualifies.
  document.addEventListener("scroll", scheduleEvaluate, { capture: true, passive: true });
  window.addEventListener("resize", scheduleEvaluate);
  document.documentElement.addEventListener("mouseleave", () => {
    lastPointer = null;
    hideHoverButton();
  });

  hoverBtn.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (hoverTarget) open(hoverTarget);
  });

  // Clicking the image itself opens it too, unless it is a link (the link
  // wins, and the hover button stays available) or the click ended a text
  // selection.
  contentEl.addEventListener("click", (event) => {
    if (event.button !== 0 || event.defaultPrevented) return;
    if (event.target instanceof Element && event.target.closest("a")) return;
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed) return;
    const candidate = candidateFrom(event.target);
    if (!candidate || !isEligible(candidate)) return;
    event.preventDefault();
    open(candidate);
  });

  // --------------------------------------------------------------- viewer ---

  const overlay = document.createElement("div");
  overlay.id = "image-viewer";
  overlay.className = "image-viewer hidden";
  overlay.setAttribute("role", "dialog");
  overlay.setAttribute("aria-modal", "true");
  overlay.setAttribute("aria-label", "Image viewer");
  overlay.tabIndex = -1;

  const stage = document.createElement("div");
  stage.className = "image-viewer-stage";

  const selectionEl = document.createElement("div");
  selectionEl.className = "image-viewer-selection hidden";

  const toolbar = document.createElement("div");
  toolbar.className = "image-viewer-toolbar";
  const backBtn = button("Back", "Back to previous zoom (Backspace)", "↶");
  const fitBtn = button("Fit", "Fit to window (0)", "⊡");
  const rectToolBtn = button("Rectangle zoom", "Rectangle zoom: drag to zoom (Z)", "");
  rectToolBtn.innerHTML = ICON_ZOOM_RECT;
  const handToolBtn = button("Hand", "Hand: drag to move (M)", "");
  handToolBtn.innerHTML = ICON_HAND;
  for (const el of [rectToolBtn, handToolBtn]) el.classList.add("image-viewer-tool");
  const separator = document.createElement("span");
  separator.className = "image-viewer-separator";
  const zoomOutBtn = button("Zoom out", "Zoom out (-)", "−");
  const zoomLabel = document.createElement("span");
  zoomLabel.className = "image-viewer-zoom";
  const zoomInBtn = button("Zoom in", "Zoom in (+)", "+");
  const closeBtn = button("Close", "Close (Esc)", "✕");
  toolbar.append(
    rectToolBtn, handToolBtn, separator, backBtn, fitBtn, zoomOutBtn, zoomLabel, zoomInBtn, closeBtn,
  );

  const hint = document.createElement("div");
  hint.className = "image-viewer-hint";

  overlay.append(stage, selectionEl, toolbar, hint);
  document.body.append(overlay);

  let svg = null;
  let bitmap = null; // the <image> when the content is a raster
  let content = null; // { w, h } in user units
  let view = null; // { x, y, w, h } in user units, aspect = stage aspect
  let history = [];
  let restoreFocus = null;
  let spaceDown = false;
  let tool = "zoom"; // what a plain left drag does: "zoom" (rectangle) or "pan"; kept across opens
  let drag = null;
  let lastWheelAt = 0;
  let appliedStageW = 1;

  function isOpen() {
    return !overlay.classList.contains("hidden");
  }

  function stageSize() {
    const rect = stage.getBoundingClientRect();
    return { w: Math.max(rect.width, 1), h: Math.max(rect.height, 1), left: rect.left, top: rect.top };
  }

  /** CSS pixels per user unit. */
  function scaleOf(v) {
    return stageSize().w / v.w;
  }

  function fitView() {
    const s = stageSize();
    const usableW = s.w * (1 - 2 * FIT_MARGIN);
    const usableH = s.h * (1 - 2 * FIT_MARGIN);
    // Never blow a small bitmap past 1:1 just to fill the window.
    let scale = Math.min(usableW / content.w, usableH / content.h);
    if (bitmap) scale = Math.min(scale, 1);
    return viewAround(content.w / 2, content.h / 2, scale);
  }

  function viewAround(cx, cy, scale) {
    const s = stageSize();
    const w = s.w / scale;
    const h = s.h / scale;
    return { x: cx - w / 2, y: cy - h / 2, w, h };
  }

  function clampScale(scale) {
    const fit = scaleOf(fitView());
    return Math.min(Math.max(scale, fit * 0.5), Math.max(fit * MAX_SCALE_FACTOR, 64));
  }

  function applyView() {
    appliedStageW = stageSize().w;
    svg.setAttribute("viewBox", `${view.x} ${view.y} ${view.w} ${view.h}`);
    const scale = scaleOf(view);
    if (bitmap) {
      bitmap.style.imageRendering = scale * window.devicePixelRatio > 1.01 ? "pixelated" : "auto";
    }
    zoomLabel.textContent = `${Math.round(scale * 100)}%`;
    backBtn.disabled = history.length === 0;
  }

  function setView(next, { remember = false } = {}) {
    if (remember) history.push(view);
    view = next;
    applyView();
  }

  /** Zoom by `factor` keeping the user point under (px, py) on screen fixed. */
  function zoomAt(factor, px, py, { remember = false } = {}) {
    const s = stageSize();
    const scale = clampScale(scaleOf(view) * factor);
    const ux = view.x + ((px - s.left) / s.w) * view.w;
    const uy = view.y + ((py - s.top) / s.h) * view.h;
    const w = s.w / scale;
    const h = s.h / scale;
    setView(
      { x: ux - ((px - s.left) / s.w) * w, y: uy - ((py - s.top) / s.h) * h, w, h },
      { remember },
    );
  }

  function zoomCenter(factor) {
    const s = stageSize();
    zoomAt(factor, s.left + s.w / 2, s.top + s.h / 2, { remember: true });
  }

  function panBy(dxPx, dyPx) {
    const scale = scaleOf(view);
    setView({ ...view, x: view.x - dxPx / scale, y: view.y - dyPx / scale });
  }

  function back() {
    if (history.length === 0) return;
    view = history.pop();
    applyView();
  }

  function reset() {
    history = [];
    setView(fitView());
  }

  function buildMermaid(source) {
    const clone = source.cloneNode(true);
    const vb = source.viewBox?.baseVal;
    let w = vb?.width;
    let h = vb?.height;
    if (!w || !h) {
      const rect = source.getBoundingClientRect();
      w = rect.width;
      h = rect.height;
      clone.setAttribute("viewBox", `0 0 ${w} ${h}`);
    }
    // Normalise to a 0,0 origin so the content box is simply (0, 0, w, h).
    const ox = vb?.width ? vb.x : 0;
    const oy = vb?.height ? vb.y : 0;
    // The clone keeps its id: mermaid scopes its embedded <style> by it.
    clone.removeAttribute("style");
    clone.setAttribute("x", "0");
    clone.setAttribute("y", "0");
    clone.setAttribute("width", String(w));
    clone.setAttribute("height", String(h));
    clone.setAttribute("viewBox", `${ox} ${oy} ${w} ${h}`);
    const root = document.createElementNS(SVG_NS, "svg");
    root.append(clone);
    return { root, w, h, raster: null };
  }

  function buildImage(img) {
    const rect = img.getBoundingClientRect();
    let w = img.naturalWidth;
    let h = img.naturalHeight;
    if (!w || !h) {
      w = rect.width;
      h = rect.height;
    }
    const root = document.createElementNS(SVG_NS, "svg");
    const image = document.createElementNS(SVG_NS, "image");
    image.setAttribute("href", img.currentSrc || img.src);
    image.setAttribute("width", String(w));
    image.setAttribute("height", String(h));
    root.append(image);
    const vector = isSvgImageSrc(img.getAttribute("src"));
    return { root, w, h, raster: vector ? null : image };
  }

  function open(candidate) {
    const built = candidate.kind === "mermaid" ? buildMermaid(candidate.el) : buildImage(candidate.el);
    hideHoverButton();
    stage.replaceChildren(built.root);
    svg = built.root;
    svg.setAttribute("class", "image-viewer-svg");
    svg.setAttribute("preserveAspectRatio", "xMidYMid meet");
    bitmap = built.raster;
    content = { w: built.w, h: built.h };
    history = [];

    restoreFocus = document.activeElement;
    overlay.classList.remove("hidden");
    overlay.setAttribute("aria-hidden", "false");
    document.body.classList.add("has-image-viewer");
    overlay.focus({ preventScroll: true });
    view = fitView();
    applyView();
  }

  function close() {
    if (!isOpen()) return;
    overlay.classList.add("hidden");
    overlay.setAttribute("aria-hidden", "true");
    document.body.classList.remove("has-image-viewer");
    endDrag();
    spaceDown = false;
    stage.replaceChildren();
    svg = null;
    bitmap = null;
    if (restoreFocus instanceof HTMLElement && restoreFocus.isConnected) {
      restoreFocus.focus({ preventScroll: true });
    }
    restoreFocus = null;
  }

  backBtn.addEventListener("click", back);
  fitBtn.addEventListener("click", reset);
  zoomInBtn.addEventListener("click", () => zoomCenter(KEY_ZOOM));
  zoomOutBtn.addEventListener("click", () => zoomCenter(1 / KEY_ZOOM));
  closeBtn.addEventListener("click", close);
  rectToolBtn.addEventListener("click", () => setTool("zoom"));
  handToolBtn.addEventListener("click", () => setTool("pan"));

  function setTool(next) {
    tool = next;
    rectToolBtn.setAttribute("aria-pressed", String(tool === "zoom"));
    handToolBtn.setAttribute("aria-pressed", String(tool === "pan"));
    hint.textContent = HINTS[tool];
    syncCursor();
  }

  // ------------------------------------------------------------- pointer ---

  function syncCursor() {
    stage.classList.toggle("is-panning", !!drag && drag.mode === "pan");
    stage.classList.toggle("can-pan", (spaceDown || tool === "pan") && !drag);
  }

  function endDrag() {
    drag = null;
    selectionEl.classList.add("hidden");
    syncCursor();
  }

  stage.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 && event.button !== 1) return;
    event.preventDefault();
    const pan = event.button === 1 || spaceDown || tool === "pan";
    drag = { mode: pan ? "pan" : "rect", x0: event.clientX, y0: event.clientY, x: event.clientX, y: event.clientY };
    stage.setPointerCapture(event.pointerId);
    syncCursor();
  });

  stage.addEventListener("pointermove", (event) => {
    if (!drag) return;
    if (drag.mode === "pan") {
      panBy(event.clientX - drag.x, event.clientY - drag.y);
      drag.x = event.clientX;
      drag.y = event.clientY;
      return;
    }
    drag.x = event.clientX;
    drag.y = event.clientY;
    // The overlay may start below the titlebar, so place the box in its coordinates.
    const origin = overlay.getBoundingClientRect();
    const left = Math.min(drag.x0, drag.x) - origin.left;
    const top = Math.min(drag.y0, drag.y) - origin.top;
    selectionEl.style.left = `${left}px`;
    selectionEl.style.top = `${top}px`;
    selectionEl.style.width = `${Math.abs(drag.x - drag.x0)}px`;
    selectionEl.style.height = `${Math.abs(drag.y - drag.y0)}px`;
    selectionEl.classList.remove("hidden");
  });

  function finishRect() {
    const s = stageSize();
    const wPx = Math.abs(drag.x - drag.x0);
    const hPx = Math.abs(drag.y - drag.y0);
    if (wPx < MIN_RECT_PX || hPx < MIN_RECT_PX) return;
    const cxPx = (drag.x + drag.x0) / 2;
    const cyPx = (drag.y + drag.y0) / 2;
    const ux = view.x + ((cxPx - s.left) / s.w) * view.w;
    const uy = view.y + ((cyPx - s.top) / s.h) * view.h;
    const current = scaleOf(view);
    // Fit the selection: the tighter of its two sides decides the scale.
    const scale = clampScale(current * Math.min(s.w / wPx, s.h / hPx));
    setView(viewAround(ux, uy, scale), { remember: true });
  }

  stage.addEventListener("pointerup", (event) => {
    if (!drag) return;
    if (stage.hasPointerCapture(event.pointerId)) stage.releasePointerCapture(event.pointerId);
    if (drag.mode === "rect") finishRect();
    endDrag();
  });

  stage.addEventListener("pointercancel", endDrag);
  stage.addEventListener("auxclick", (event) => event.preventDefault());
  stage.addEventListener("dblclick", reset);
  overlay.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    event.stopPropagation();
  });

  overlay.addEventListener("wheel", (event) => {
    event.preventDefault();
    if (!svg) return;
    const now = performance.now();
    // One wheel gesture is one history step, not one per notch.
    const remember = now - lastWheelAt > WHEEL_SESSION_MS;
    lastWheelAt = now;
    const delta = event.deltaMode === 1 ? event.deltaY * 16 : event.deltaY;
    zoomAt(Math.pow(WHEEL_STEP, -delta), event.clientX, event.clientY, { remember });
  }, { passive: false });

  window.addEventListener("resize", () => {
    if (!isOpen()) return;
    // Keep each view's centre and scale, take the new aspect.
    const previousW = appliedStageW;
    const reshape = (v) => viewAround(v.x + v.w / 2, v.y + v.h / 2, previousW / v.w);
    view = reshape(view);
    history = history.map(reshape);
    applyView();
  });

  // ------------------------------------------------------------ keyboard ---

  // Capture phase, so the reader keybindings never see a key while the viewer
  // is up: it is modal, and `j` must not scroll the document behind it.
  window.addEventListener("keydown", (event) => {
    if (!isOpen()) return;
    event.stopImmediatePropagation();
    const key = event.key;
    let handled = true;
    switch (key) {
      case "Escape":
      case "q":
        close();
        break;
      case " ":
        spaceDown = true;
        syncCursor();
        break;
      case "z":
        setTool("zoom");
        break;
      case "m":
        setTool("pan");
        break;
      case "Backspace":
      case "u":
        back();
        break;
      case "0":
        reset();
        break;
      case "+":
      case "=":
        zoomCenter(KEY_ZOOM);
        break;
      case "-":
      case "_":
        zoomCenter(1 / KEY_ZOOM);
        break;
      case "ArrowLeft":
      case "h":
        panBy(stageSize().w * KEY_PAN, 0);
        break;
      case "ArrowRight":
      case "l":
        panBy(-stageSize().w * KEY_PAN, 0);
        break;
      case "ArrowUp":
      case "k":
        panBy(0, stageSize().h * KEY_PAN);
        break;
      case "ArrowDown":
      case "j":
        panBy(0, -stageSize().h * KEY_PAN);
        break;
      case "Tab":
        handled = false;
        break;
      default:
        // Enter still activates a focused toolbar button.
        handled = !(key === "Enter" && event.target instanceof HTMLButtonElement);
    }
    if (handled) event.preventDefault();
  }, { capture: true });

  window.addEventListener("keyup", (event) => {
    if (event.key !== " ") return;
    spaceDown = false;
    if (isOpen()) {
      event.stopImmediatePropagation();
      event.preventDefault();
      syncCursor();
    }
  }, { capture: true });

  window.addEventListener("blur", () => {
    spaceDown = false;
    if (isOpen()) syncCursor();
  });

  setTool(tool);

  return { open, close, isOpen, hideHoverButton };
}
