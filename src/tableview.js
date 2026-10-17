/**
 * Table viewer overlay.
 *
 * Every table gets an "expand" button on hover. It opens a full-window viewer
 * holding a copy of the table, where it can be sorted by any column, filtered
 * by row text, and zoomed.
 *
 * Zoom is a font-size change on the copy, not a transform. Every table metric
 * in `.prose` is in `em`, so the table reflows at the new size: text stays
 * crisp, the stage's native scrollbars stay truthful, and the sticky header
 * keeps working. "Fit" solves for the size at which the table's width matches
 * the stage, which is linear in font size up to the 1px borders.
 *
 * Sorting is per `<tbody>` and stable, with the original order as the third
 * state of the header cycle (ascending → descending → original). A column
 * sorts numerically when every non-empty cell parses as a number, and
 * otherwise with a numeric-aware collator, so "item 10" follows "item 9".
 * Empty cells always sink to the bottom. Body cells that span rows or columns
 * make row order meaningless, so such tables are offered without sorting.
 *
 * The document is never touched: the overlay works on a clone, which loses its
 * `id`s so nothing in the document is shadowed. A click on a link in the copy
 * closes the viewer and is replayed on the original link, so every link rule
 * in `main.js` (local Markdown, footnotes, external) applies unchanged.
 */

const MIN_SCALE = 0.25;
const MAX_SCALE = 4;
const KEY_ZOOM = 1.15;
const WHEEL_STEP = 1.0015;
const KEY_SCROLL = 60;

const HINT =
  "Click a header to sort · / filter · Ctrl+wheel or +/− zoom · 0 fit · 1 actual size · Esc close";

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

function button(label, title, text) {
  const el = document.createElement("button");
  el.type = "button";
  el.className = "image-viewer-btn";
  el.title = title;
  el.setAttribute("aria-label", label);
  el.textContent = text;
  return el;
}

/** Lowercased, accent-free, so "accion" finds "Acción". */
function foldText(value) {
  return value.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

const THOUSANDS = /^[+-]?\d{1,3}(?:,\d{3})+(?:\.\d+)?$/;

/** A cell's numeric value, or NaN. Tolerates currency, %, grouping and U+2212. */
export function parseCellNumber(raw) {
  let s = raw.replace(/[\s   ]/g, "").replace(/−/g, "-");
  s = s.replace(/^([+-]?)[$€£¥]/, "$1").replace(/%$/, "");
  if (THOUSANDS.test(s)) s = s.replace(/,/g, "");
  if (!/^[+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?$/i.test(s)) return NaN;
  return Number(s);
}

/** The header row (whose cells become sort buttons) and the body sections. */
function tableLayout(table) {
  const headRows = table.tHead ? [...table.tHead.rows] : [];
  let header = headRows.at(-1) ?? null;
  let bodies = [...table.tBodies];
  let skipFirst = false;
  if (!header) {
    const first = table.rows[0];
    if (first && first.cells.length && [...first.cells].every((c) => c.tagName === "TH")) {
      header = first;
      skipFirst = true;
    }
  }
  const sections = bodies.map((tbody) => {
    const rows = [...tbody.rows];
    if (skipFirst && rows[0] === header) rows.shift();
    return { tbody, rows };
  }).filter((section) => section.rows.length > 0);
  return { header, sections };
}

function spansCells(rows) {
  return rows.some((row) => [...row.cells].some((c) => c.rowSpan > 1 || c.colSpan > 1));
}

/** The cell occupying column `index` of `row`, honouring colspans. */
function cellAt(row, index) {
  let col = 0;
  for (const cell of row.cells) {
    if (index < col + cell.colSpan) return cell;
    col += cell.colSpan;
  }
  return null;
}

export function createTableView({ contentEl }) {
  // ---------------------------------------------------------------- hover ---

  const hoverBtn = document.createElement("button");
  hoverBtn.type = "button";
  hoverBtn.id = "table-view-button";
  hoverBtn.className = "image-zoom-button hidden";
  hoverBtn.title = "Open table viewer";
  hoverBtn.setAttribute("aria-label", "Open table viewer");
  hoverBtn.textContent = "⤢";
  document.body.append(hoverBtn);

  let hoverTarget = null; // the .table-wrapper under the pointer
  let hoverFrame = 0;
  let lastPointer = null;

  function wrapperFrom(node) {
    if (!(node instanceof Element) || !contentEl.contains(node)) return null;
    const wrapper = node.closest(".table-wrapper");
    return wrapper?.querySelector(":scope > table") ? wrapper : null;
  }

  function placeHoverButton() {
    if (!hoverTarget?.isConnected) {
      hideHoverButton();
      return;
    }
    const rect = hoverTarget.getBoundingClientRect();
    const table = hoverTarget.querySelector(":scope > table").getBoundingClientRect();
    // Hug the table when it is narrower than the column, as tables are centred.
    const right = Math.min(Math.min(rect.right, table.right), window.innerWidth) - 4;
    const top = Math.max(rect.top, 0) + 4;
    if (rect.bottom < top + 32 || right - 32 < rect.left) {
      hoverBtn.classList.add("hidden");
      return;
    }
    hoverBtn.style.top = `${top}px`;
    hoverBtn.style.left = `${right - hoverBtn.offsetWidth}px`;
    hoverBtn.classList.remove("hidden");
  }

  function setHoverTarget(wrapper) {
    hoverTarget = wrapper;
    if (!wrapper) {
      hoverBtn.classList.add("hidden");
      return;
    }
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
    setHoverTarget(wrapperFrom(under));
  }

  function scheduleEvaluate() {
    if (!hoverFrame) hoverFrame = requestAnimationFrame(evaluatePointer);
  }

  document.addEventListener("mousemove", (event) => {
    lastPointer = { x: event.clientX, y: event.clientY };
    scheduleEvaluate();
  }, { passive: true });
  document.addEventListener("scroll", scheduleEvaluate, { capture: true, passive: true });
  window.addEventListener("resize", scheduleEvaluate);
  document.documentElement.addEventListener("mouseleave", () => {
    lastPointer = null;
    hideHoverButton();
  });

  hoverBtn.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    const table = hoverTarget?.querySelector(":scope > table");
    if (table) open(table);
  });

  // --------------------------------------------------------------- viewer ---

  const overlay = document.createElement("div");
  overlay.id = "table-viewer";
  overlay.className = "image-viewer table-viewer hidden";
  overlay.setAttribute("role", "dialog");
  overlay.setAttribute("aria-modal", "true");
  overlay.setAttribute("aria-label", "Table viewer");

  const bar = document.createElement("div");
  bar.className = "image-viewer-toolbar table-viewer-bar";

  const filterInput = document.createElement("input");
  filterInput.type = "search";
  filterInput.className = "table-viewer-filter";
  filterInput.placeholder = "Filter rows";
  filterInput.setAttribute("aria-label", "Filter rows");
  filterInput.spellcheck = false;
  const countLabel = document.createElement("span");
  countLabel.className = "image-viewer-zoom table-viewer-count";
  const separator = document.createElement("span");
  separator.className = "image-viewer-separator";
  const zoomOutBtn = button("Zoom out", "Zoom out (-)", "−");
  const zoomLabel = document.createElement("span");
  zoomLabel.className = "image-viewer-zoom";
  const zoomInBtn = button("Zoom in", "Zoom in (+)", "+");
  const fitBtn = button("Fit width", "Fit width (0)", "⊡");
  const actualBtn = button("Actual size", "Actual size (1)", "1:1");
  const resetSortBtn = button("Original order", "Restore the original row order", "⇅");
  const closeBtn = button("Close", "Close (Esc)", "✕");
  bar.append(
    filterInput, countLabel, separator, resetSortBtn,
    zoomOutBtn, zoomLabel, zoomInBtn, fitBtn, actualBtn, closeBtn,
  );

  const stage = document.createElement("div");
  stage.className = "table-viewer-stage";
  stage.tabIndex = -1;
  const sheet = document.createElement("div");
  sheet.className = "prose visible table-viewer-sheet";
  stage.append(sheet);

  const hint = document.createElement("div");
  hint.className = "image-viewer-hint";
  hint.textContent = HINT;

  overlay.append(bar, stage, hint);
  document.body.append(overlay);

  let table = null;
  let layout = null; // { header, sections: [{ tbody, rows }] }
  let columnKeys = new Map(); // column index → { numeric, keys: Map<row, key> }
  let rowText = new Map(); // row → folded text
  let sortState = null; // { col, dir: 1 | -1 }
  let sortable = false;
  let scale = 1;
  let pinchBase = 1;
  let restoreFocus = null;
  let linkTargets = new WeakMap(); // cloned <a> → original <a>

  function isOpen() {
    return !overlay.classList.contains("hidden");
  }

  // ----------------------------------------------------------------- zoom ---

  function applyScale(next, anchor) {
    const clamped = Math.min(Math.max(next, MIN_SCALE), MAX_SCALE);
    const ratio = clamped / scale;
    const rect = stage.getBoundingClientRect();
    const ax = anchor ? anchor.x - rect.left : stage.clientWidth / 2;
    const ay = anchor ? anchor.y - rect.top : stage.clientHeight / 2;
    const left = (stage.scrollLeft + ax) * ratio - ax;
    const top = (stage.scrollTop + ay) * ratio - ay;
    scale = clamped;
    sheet.style.fontSize = `calc(var(--font-size) * ${scale})`;
    stage.scrollLeft = left;
    stage.scrollTop = top;
    zoomLabel.textContent = `${Math.round(scale * 100)}%`;
    zoomOutBtn.disabled = scale <= MIN_SCALE;
    zoomInBtn.disabled = scale >= MAX_SCALE;
  }

  function fitScale() {
    const style = getComputedStyle(sheet);
    const available = stage.clientWidth
      - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
    const width = table.getBoundingClientRect().width;
    return width > 0 ? scale * (available / width) : scale;
  }

  function fit() {
    // Two passes absorb the part of the width that does not scale (borders).
    applyScale(fitScale());
    applyScale(fitScale());
    stage.scrollLeft = 0;
  }

  // ---------------------------------------------------------------- sort ---

  function keysFor(col) {
    let entry = columnKeys.get(col);
    if (entry) return entry;
    const texts = new Map();
    for (const { rows } of layout.sections) {
      for (const row of rows) texts.set(row, (cellAt(row, col)?.textContent ?? "").trim());
    }
    const filled = [...texts.values()].filter(Boolean);
    const numeric = filled.length > 0 && filled.every((t) => !Number.isNaN(parseCellNumber(t)));
    const keys = new Map();
    for (const [row, text] of texts) {
      keys.set(row, text === "" ? null : numeric ? parseCellNumber(text) : text);
    }
    entry = { numeric, keys };
    columnKeys.set(col, entry);
    return entry;
  }

  function applySort() {
    for (const { tbody, rows } of layout.sections) {
      let ordered = rows;
      if (sortState) {
        const { numeric, keys } = keysFor(sortState.col);
        const dir = sortState.dir;
        ordered = rows
          .map((row, index) => ({ row, index, key: keys.get(row) }))
          .sort((a, b) => {
            if (a.key === null || b.key === null) {
              if (a.key === b.key) return a.index - b.index;
              return a.key === null ? 1 : -1;
            }
            const cmp = numeric ? a.key - b.key : collator.compare(a.key, b.key);
            return cmp * dir || a.index - b.index;
          })
          .map((entry) => entry.row);
      }
      tbody.append(...ordered);
    }
    for (const th of sortHeaders()) {
      const col = Number(th.dataset.tvCol);
      const state = sortState?.col === col ? (sortState.dir > 0 ? "ascending" : "descending") : "none";
      th.setAttribute("aria-sort", state);
    }
    resetSortBtn.disabled = !sortState;
  }

  function sortHeaders() {
    return layout?.header ? layout.header.querySelectorAll(":scope > [data-tv-col]") : [];
  }

  function cycleSort(col) {
    if (sortState?.col !== col) sortState = { col, dir: 1 };
    else if (sortState.dir === 1) sortState = { col, dir: -1 };
    else sortState = null;
    applySort();
  }

  // -------------------------------------------------------------- filter ---

  function applyFilter() {
    const query = foldText(filterInput.value.trim());
    let total = 0;
    let shown = 0;
    for (const { rows } of layout.sections) {
      for (const row of rows) {
        let text = rowText.get(row);
        if (text === undefined) {
          text = foldText(row.textContent);
          rowText.set(row, text);
        }
        const match = !query || text.includes(query);
        row.hidden = !match;
        total += 1;
        if (match) shown += 1;
      }
    }
    countLabel.textContent = query ? `${shown} / ${total} rows` : `${total} rows`;
  }

  // ---------------------------------------------------------- open/close ---

  function prepareClone(source) {
    const clone = source.cloneNode(true);
    const originals = source.querySelectorAll("a[href]");
    clone.querySelectorAll("a[href]").forEach((a, i) => linkTargets.set(a, originals[i]));
    for (const el of clone.querySelectorAll("[id]")) el.removeAttribute("id");
    clone.removeAttribute("id");
    return clone;
  }

  function open(source) {
    hideHoverButton();
    linkTargets = new WeakMap();
    table = prepareClone(source);
    layout = tableLayout(table);
    columnKeys = new Map();
    rowText = new Map();
    sortState = null;
    sortable = layout.header !== null
      && layout.sections.some((s) => s.rows.length > 1)
      && !spansCells(layout.sections.flatMap((s) => s.rows));

    if (sortable) {
      let col = 0;
      for (const th of layout.header.cells) {
        if (th.colSpan === 1) {
          th.dataset.tvCol = String(col);
          th.classList.add("tv-sortable");
          th.tabIndex = 0;
          th.title = "Sort by this column";
        }
        col += th.colSpan;
      }
    }
    sheet.replaceChildren(table);
    filterInput.value = "";
    filterInput.disabled = layout.sections.length === 0;
    resetSortBtn.hidden = !sortable;
    applySort();
    applyFilter();

    restoreFocus = document.activeElement;
    overlay.classList.remove("hidden");
    overlay.setAttribute("aria-hidden", "false");
    document.body.classList.add("has-table-viewer");
    scale = 1;
    applyScale(1);
    stage.scrollTo(0, 0);
    stage.focus({ preventScroll: true });
  }

  function close() {
    if (!isOpen()) return;
    overlay.classList.add("hidden");
    overlay.setAttribute("aria-hidden", "true");
    document.body.classList.remove("has-table-viewer");
    sheet.replaceChildren();
    table = null;
    layout = null;
    columnKeys = new Map();
    rowText = new Map();
    if (restoreFocus instanceof HTMLElement && restoreFocus.isConnected) {
      restoreFocus.focus({ preventScroll: true });
    }
    restoreFocus = null;
  }

  /** Touchpad pinch, forwarded by `main.js` while the viewer is up. */
  function pinch(payload) {
    if (payload.phase === "begin") {
      pinchBase = scale;
      return;
    }
    if (payload.phase === "cancel" || !(payload.scale > 0)) return;
    applyScale(pinchBase * payload.scale, lastPointer);
  }

  filterInput.addEventListener("input", applyFilter);
  zoomInBtn.addEventListener("click", () => applyScale(scale * KEY_ZOOM));
  zoomOutBtn.addEventListener("click", () => applyScale(scale / KEY_ZOOM));
  fitBtn.addEventListener("click", fit);
  actualBtn.addEventListener("click", () => applyScale(1));
  resetSortBtn.addEventListener("click", () => {
    sortState = null;
    applySort();
  });
  closeBtn.addEventListener("click", close);

  stage.addEventListener("click", (event) => {
    if (!(event.target instanceof Element)) return;
    const anchor = event.target.closest("a[href]");
    if (anchor) {
      event.preventDefault();
      const original = linkTargets.get(anchor);
      close();
      if (original?.isConnected) {
        original.dispatchEvent(new MouseEvent("click", {
          bubbles: true, cancelable: true, ctrlKey: event.ctrlKey, metaKey: event.metaKey,
        }));
      }
      return;
    }
    const th = event.target.closest("th.tv-sortable");
    if (!th) return;
    // A click that ended a text selection inside the header is not a sort.
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed && th.contains(selection.anchorNode)) return;
    cycleSort(Number(th.dataset.tvCol));
  });

  overlay.addEventListener("wheel", (event) => {
    if (!event.ctrlKey) return; // plain wheel scrolls the stage natively
    event.preventDefault();
    const delta = event.deltaMode === 1 ? event.deltaY * 16 : event.deltaY;
    applyScale(scale * Math.pow(WHEEL_STEP, -delta), { x: event.clientX, y: event.clientY });
  }, { passive: false });

  // The app's own context menu (open file, settings…) means nothing here, but
  // the native one is kept for a selection or the filter box, as elsewhere.
  overlay.addEventListener("contextmenu", (event) => {
    event.stopPropagation();
    const selection = window.getSelection();
    const hasSelection = selection && selection.toString().trim();
    if (!hasSelection && event.target !== filterInput) event.preventDefault();
  });

  // ------------------------------------------------------------ keyboard ---

  // Capture phase, so the reader keybindings never see a key while the viewer
  // is up. Typing in the filter box keeps its default action.
  window.addEventListener("keydown", (event) => {
    if (!isOpen()) return;
    event.stopImmediatePropagation();
    const key = event.key;

    if (event.target === filterInput) {
      if (key === "Escape" || key === "Enter") {
        event.preventDefault();
        if (key === "Escape" && filterInput.value) {
          filterInput.value = "";
          applyFilter();
        } else {
          stage.focus({ preventScroll: true });
        }
      }
      return;
    }

    let handled = true;
    switch (key) {
      case "Escape":
      case "q":
        close();
        break;
      case "/":
        filterInput.focus();
        filterInput.select();
        break;
      case "+":
      case "=":
        applyScale(scale * KEY_ZOOM);
        break;
      case "-":
      case "_":
        applyScale(scale / KEY_ZOOM);
        break;
      case "0":
        if (event.ctrlKey) applyScale(1);
        else fit();
        break;
      case "1":
        applyScale(1);
        break;
      case "h":
        stage.scrollBy(-KEY_SCROLL, 0);
        break;
      case "l":
        stage.scrollBy(KEY_SCROLL, 0);
        break;
      case "k":
        stage.scrollBy(0, -KEY_SCROLL);
        break;
      case "j":
        stage.scrollBy(0, KEY_SCROLL);
        break;
      case "g":
        stage.scrollTo({ top: 0 });
        break;
      case "G":
        stage.scrollTo({ top: stage.scrollHeight });
        break;
      case "Enter":
      case " ": {
        const th = event.target instanceof Element ? event.target.closest("th.tv-sortable") : null;
        if (th) cycleSort(Number(th.dataset.tvCol));
        // Otherwise let Enter activate a toolbar button and Space page the stage.
        else handled = false;
        break;
      }
      default:
        // Arrows, Page Up/Down, Home/End and Tab keep their native behaviour.
        handled = false;
    }
    if (handled) event.preventDefault();
  }, { capture: true });

  return { open, close, isOpen, hideHoverButton, pinch };
}
