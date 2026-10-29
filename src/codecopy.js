/**
 * Copy button for code blocks.
 *
 * Like the image zoom and table viewer buttons, this is one floating button on
 * `<body>`, placed over the top-right corner of whichever `<pre>` is under the
 * pointer and hidden otherwise. The content DOM is never wrapped, so nothing
 * that walks or measures it (`stats.js`, `changes.js`, the contents panel) has
 * to know about the button.
 *
 * What is copied is the block's `textContent`: highlighting only adds spans and
 * change carets are empty, so that is exactly the fenced source, minus the
 * trailing newline comrak keeps.
 */

// Stroked with currentColor so the icons follow the theme.
const ICON_COPY =
  '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
  '<rect x="9" y="9" width="13" height="13" rx="2" ry="2"/>' +
  '<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
const ICON_DONE =
  '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
  '<path d="M20 6 9 17l-5-5"/></svg>';
const ICON_FAILED =
  '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
  '<path d="M18 6 6 18M6 6l12 12"/></svg>';

const LABEL = "Copy code";
const RESET_MS = 1600;

export function createCodeCopy({ contentEl }) {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.id = "code-copy-button";
  btn.className = "image-zoom-button code-copy-button hidden";
  btn.title = LABEL;
  btn.setAttribute("aria-label", LABEL);
  btn.innerHTML = ICON_COPY;
  document.body.append(btn);

  let target = null; // the <pre> under the pointer
  let frame = 0;
  let lastPointer = null;
  let resetTimer = null;

  function blockFrom(node) {
    if (!(node instanceof Element) || !contentEl.contains(node)) return null;
    const pre = node.closest("pre");
    // A plain-text document is one big `pre.plain-text`, read as prose.
    return pre && contentEl.contains(pre) && !pre.classList.contains("plain-text") ? pre : null;
  }

  function place() {
    if (!target?.isConnected) {
      hideHoverButton();
      return;
    }
    const rect = target.getBoundingClientRect();
    const right = Math.min(rect.right, window.innerWidth) - 4;
    // Clamped to the viewport, so it stays reachable on a long block.
    const top = Math.max(rect.top, 0) + 4;
    if (rect.bottom < top + 32 || right - 32 < rect.left) {
      btn.classList.add("hidden");
      return;
    }
    // Unhidden before measuring: a `display: none` button has no width, and the
    // first placement would land one button-width too far right.
    btn.classList.remove("hidden");
    btn.style.top = `${top}px`;
    btn.style.left = `${right - btn.offsetWidth}px`;
  }

  function resetIcon() {
    clearTimeout(resetTimer);
    resetTimer = null;
    btn.classList.remove("copied", "failed");
    btn.innerHTML = ICON_COPY;
    btn.title = LABEL;
  }

  function setTarget(pre) {
    if (pre !== target) resetIcon();
    target = pre;
    if (!pre) {
      btn.classList.add("hidden");
      return;
    }
    place();
  }

  function hideHoverButton() {
    setTarget(null);
  }

  function evaluatePointer() {
    frame = 0;
    if (!lastPointer) return;
    const under = document.elementFromPoint(lastPointer.x, lastPointer.y);
    if (under === btn || btn.contains(under)) {
      place();
      return;
    }
    setTarget(blockFrom(under));
  }

  function scheduleEvaluate() {
    if (!frame) frame = requestAnimationFrame(evaluatePointer);
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

  btn.addEventListener("click", async (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (!target) return;
    const text = target.textContent.replace(/\n$/, "");
    let ok = false;
    try {
      await navigator.clipboard.writeText(text);
      ok = true;
    } catch (err) {
      console.warn("Failed to copy code block", err);
    }
    clearTimeout(resetTimer);
    btn.classList.toggle("copied", ok);
    btn.classList.toggle("failed", !ok);
    btn.innerHTML = ok ? ICON_DONE : ICON_FAILED;
    btn.title = ok ? "Copied" : "Copy failed";
    resetTimer = setTimeout(resetIcon, RESET_MS);
  });

  return { hideHoverButton };
}
