/**
 * Change highlighting: paint the backend's diff onto the rendered document.
 *
 * The backend does all the diffing and hands over a list of blocks, each
 * identified by `(tag, sourcepos)` and carrying spans expressed as **token
 * indices**, not character offsets. This module's whole job is to turn those
 * indices back into DOM ranges, and to refuse to do so when it cannot prove the
 * two sides agree — see `expect` below.
 *
 * Ordering is not a nicety here, it is a precondition. `applyChanges` reads a
 * block's text out of the DOM and checks it against the backend's `expect`
 * string, so it must run while math is still literal `$…$` text and mermaid
 * diagrams are still `<pre><code>` — exactly like `stats.js`. Called after
 * MathJax has run, every block containing math would fail that check and
 * degrade to a block-level bar. In practice that means: call this from
 * `applyDocument` and nowhere else. To refresh marks later, re-render.
 */

import {
  normalizeTextNodes,
  collectTextSegments,
  resolveTextPosition,
  wrapRange,
  unwrap,
} from "./textrange.js";

/// Elements whose text is not ours to mark: `pre` because code blocks are
/// block-level only, the rest because their content is generated or replaced.
const OPAQUE = "pre, script, style, .mermaid, mjx-container, .katex";

/// Math regions must never be split by a mark. MathJax scans text nodes for
/// these delimiters, and a `<mark>` in the middle both breaks detection and,
/// once typesetting replaces the region with `<mjx-container>`, destroys any
/// mark inside it. Same expression `stats.js` uses.
const MATH_REGION = /\$\$[\s\S]+?\$\$|\$[^$\n]+?\$/g;

/// Must match `tokenize` in `src-tauri/src/changes.rs`. If these two disagree
/// about where a token starts, every block fails its `expect` check.
const TOKEN = /\S+/gu;

/**
 * Mark up every changed block in `container`.
 *
 * Returns the number of blocks that got some kind of mark. Blocks the backend
 * sent but that cannot be resolved or verified are counted as degraded, not as
 * failures — a coarse block-level bar is always correct, so falling back to it
 * is the designed behavior rather than an error path.
 */
export function applyChanges(container, changes) {
  clearChanges(container);
  if (!container || !changes?.length) return 0;

  // Resolve every block first. The ownership test below asks "is this text
  // node's nearest keyed-block ancestor *this* element?", which needs the full
  // set of keyed elements up front — a `<li>` is resolved before the nested
  // `<li>` whose text it must not claim.
  const resolved = [];
  const blockElements = new WeakSet();
  let unresolved = 0;

  for (const block of changes) {
    const el = resolveBlockElement(container, block);
    if (!el) {
      unresolved += 1;
      continue;
    }
    blockElements.add(el);
    resolved.push({ block, el });
  }

  let marked = 0;
  for (const { block, el } of resolved) {
    if (markBlock(el, block, blockElements)) marked += 1;
  }

  if (unresolved > 0) {
    // Once per document, never once per block: a comrak upgrade that shifts
    // the tight-paragraph rule would otherwise flood the console.
    console.debug(
      `emede: ${unresolved} of ${changes.length} changed blocks could not be located in the DOM`,
    );
  }
  return marked;
}

/** Remove every mark this module made, restoring the original inline markup. */
export function clearChanges(container) {
  if (!container) return;
  for (const el of container.querySelectorAll(".emede-change")) {
    unwrap(el);
  }
  for (const el of container.querySelectorAll(".emede-change-deleted")) {
    el.remove();
  }
  for (const el of container.querySelectorAll(".emede-change-block")) {
    el.classList.remove("emede-change-block", "emede-change-block--added");
  }
  normalizeTextNodes(container);
}

/**
 * Find the element a block was rendered as.
 *
 * The key is `(tag, sourcepos)`, not sourcepos alone: a single-item list gives
 * `<ul>` and `<li>` the same value. Two elements of the same tag cannot begin
 * at the same source position, so the pair is unique. Anything other than
 * exactly one hit is treated as unresolvable and skipped silently — zero means
 * the renderer moved on us, more than one means the assumption broke, and in
 * both cases guessing would put the highlight in the wrong place.
 */
function resolveBlockElement(container, block) {
  if (!block?.tag || !block?.sourcepos) return null;
  let found;
  try {
    found = container.querySelectorAll(
      `${block.tag}[data-sourcepos="${CSS.escape(block.sourcepos)}"]`,
    );
  } catch {
    return null;
  }
  return found.length === 1 ? found[0] : null;
}

/** Mark one block. Returns true if anything was marked. */
function markBlock(el, block, blockElements) {
  // Code blocks are block-level only — their text is deliberately not walked.
  if (block.tag === "pre") {
    return markWholeBlock(el, block);
  }

  normalizeTextNodes(el);
  const owns = (node) => ownsTextNode(el, node, blockElements);
  const { text } = collectTextSegments(el, (node) => !owns(node));
  const tokens = tokenizeWithOffsets(text);

  // The safety interlock. If the token stream the backend diffed is not the
  // token stream actually on screen, its indices mean nothing here — so mark
  // the block coarsely instead of placing spans somewhere plausible-looking
  // but wrong.
  if (tokens.map((t) => t.text).join(" ") !== block.expect) {
    return markWholeBlock(el, block);
  }

  const mathRanges = findMathRanges(text);
  // A whole-block change still shows its carets: those record blocks that were
  // deleted around it, which the block mark itself does not convey.
  const spans = (block.spans ?? []).filter(
    (span) => !block.whole || span.kind === "deleted",
  );

  const placeable = spans.filter((span) => {
    const range = spanCharRange(span, tokens, text);
    return range && !intersectsAny(range, mathRanges);
  });

  if (block.whole) markWholeBlock(el, block);

  // Back to front, so wrapping one span never shifts the offsets of the next.
  placeable.sort((a, b) => b.start - a.start || b.len - a.len);
  let placed = 0;
  for (const span of placeable) {
    if (applySpan(el, span, owns)) placed += 1;
  }

  // Every span dropped — all of them inside math, say. The block did change, so
  // fall back to the coarse mark rather than showing nothing at all.
  if (!block.whole && placed === 0 && spans.length > 0) {
    return markWholeBlock(el, block);
  }

  return block.whole || placed > 0;
}

function markWholeBlock(el, block) {
  el.classList.add("emede-change-block");
  // Green — "this block is new" — only when the backend actually said so. A
  // block that merely failed verification is a modification we could not
  // localize, and colouring it as an addition would be a lie.
  if (block.whole && !block.previous) {
    el.classList.add("emede-change-block--added");
  }
  if (block.previous) {
    el.dataset.previous = block.previous;
    el.title = block.previous;
  }
  return true;
}

/**
 * Whether `node`'s text belongs to `el` rather than to a block nested inside it.
 *
 * `<li>` is a key holder and may contain a nested `<ul>`, so a block's text is
 * its subtree's text *minus* the subtrees of nested keyed blocks. The Rust
 * walker stops descending at keyed blocks for exactly the same reason; this is
 * the mirror of that rule, done by walking up instead of down so the DOM is
 * never mutated to mark ownership.
 */
function ownsTextNode(el, node, blockElements) {
  const parent = node.parentElement;
  if (!parent) return false;
  if (parent.closest(OPAQUE)) return false;

  for (let current = parent; current; current = current.parentElement) {
    if (blockElements.has(current)) return current === el;
    if (current === el) return true;
  }
  return false;
}

/** Split `text` into tokens, recording each one's `[start, end)` char range. */
function tokenizeWithOffsets(text) {
  const tokens = [];
  TOKEN.lastIndex = 0;
  let match;
  while ((match = TOKEN.exec(text)) !== null) {
    tokens.push({ text: match[0], start: match.index, end: match.index + match[0].length });
  }
  return tokens;
}

function findMathRanges(text) {
  const ranges = [];
  MATH_REGION.lastIndex = 0;
  let match;
  while ((match = MATH_REGION.exec(text)) !== null) {
    ranges.push({ start: match.index, end: match.index + match[0].length });
  }
  return ranges;
}

/** Translate a span's token indices into a character range in the flat text. */
function spanCharRange(span, tokens, text) {
  if (span.kind === "deleted") {
    // A caret sits between tokens, so it is a zero-width point: at the start of
    // the token it precedes, or past the end of the text when it trails.
    const at = span.start < tokens.length ? tokens[span.start].start : text.length;
    return { start: at, end: at };
  }
  if (span.len <= 0 || span.start >= tokens.length) return null;
  const last = Math.min(span.start + span.len, tokens.length) - 1;
  return { start: tokens[span.start].start, end: tokens[last].end };
}

function intersectsAny(range, mathRanges) {
  return mathRanges.some((math) => range.start < math.end && math.start < range.end);
}

/**
 * Place one span in the DOM.
 *
 * Segments are re-collected per span, exactly as `find.js` does: the previous
 * wrap split text nodes, so a mapping captured once would already be stale.
 * That makes this O(spans × nodes), which is fine — both are per-block and
 * small, and correctness is worth more than the cleverness here.
 */
function applySpan(el, span, owns) {
  normalizeTextNodes(el);
  const { text, segments } = collectTextSegments(el, (node) => !owns(node));
  if (segments.length === 0) return false;

  const tokens = tokenizeWithOffsets(text);
  const chars = spanCharRange(span, tokens, text);
  if (!chars) return false;

  const startPos = resolveTextPosition(segments, chars.start);
  const endPos = resolveTextPosition(segments, chars.end);
  if (!startPos || !endPos) return false;

  const range = document.createRange();
  range.setStart(startPos.node, startPos.offset);
  range.setEnd(endPos.node, endPos.offset);

  if (span.kind === "deleted") {
    const caret = document.createElement("span");
    caret.className = "emede-change-deleted";
    caret.setAttribute("role", "img");
    describe(caret, span.previous, "Deleted");
    range.collapse(true);
    range.insertNode(caret);
    return true;
  }

  const kind = span.kind === "changed" ? "changed" : "added";
  wrapRange(range, (mark) => {
    mark.className = `emede-change emede-change--${kind}`;
    describe(mark, span.previous, kind === "changed" ? "Was" : "Added");
  });
  return true;
}

/**
 * Attach the previous wording to a mark.
 *
 * Written from JS, so ammonia's attribute stripping never sees it — the same
 * reason `find.js` can rely on `data-find-match`.
 */
function describe(el, previous, label) {
  if (previous) {
    el.dataset.previous = previous;
    el.title = `${label}: ${previous}`;
    el.setAttribute("aria-label", `${label}: ${previous}`);
  } else {
    el.title = label;
    el.setAttribute("aria-label", label);
  }
}
