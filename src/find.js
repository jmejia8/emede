import { collectTextSegments } from "./textrange.js";
import { getScrollRoot, getScrollViewportHeight } from "./scroll.js";

const MATCH_HIGHLIGHT = "emede-find-match";
const ACTIVE_HIGHLIGHT = "emede-find-active";

/**
 * Resolve monotonically increasing document offsets in one forward pass.
 *
 * Search hits never overlap, so separate cursors for their starts and ends
 * avoid an O(matches x text-nodes) walk on large documents.
 */
function positionResolver(segments) {
  let cursor = 0;

  return (index) => {
    while (cursor < segments.length - 1 && index >= segments[cursor].end) {
      cursor++;
    }

    const segment = segments[cursor];
    if (!segment || index < segment.start || index > segment.end) return null;
    return { node: segment.node, offset: index - segment.start };
  };
}

function supportsCustomHighlights() {
  return typeof globalThis.Highlight === "function" && Boolean(globalThis.CSS?.highlights);
}

export class FindInPage {
  constructor(container) {
    this.container = container;
    this.matches = [];
    this.activeIndex = -1;
    this.query = "";
    this.matchHighlight = null;
    this.activeHighlight = null;
    this.fadeTimer = null;
  }

  stop({ fade = false } = {}) {
    clearTimeout(this.fadeTimer);
    this.fadeTimer = null;
    const hadHighlights = Boolean(this.matchHighlight || this.activeHighlight);

    this.matches = [];
    this.activeIndex = -1;
    this.query = "";

    if (fade && hadHighlights) {
      this.container.classList.add("find-highlights-closing");
      this.fadeTimer = setTimeout(() => {
        this.fadeTimer = null;
        this.clearHighlights();
      }, 180);
      return;
    }

    this.clearHighlights();
  }

  clearHighlights() {
    const hadHighlights = Boolean(this.matchHighlight || this.activeHighlight);

    // Clear the ranges before unregistering them. WebKit can otherwise retain
    // the old highlight paint until an unrelated invalidation, such as scroll.
    this.matchHighlight?.clear();
    this.activeHighlight?.clear();
    globalThis.CSS?.highlights?.delete(MATCH_HIGHLIGHT);
    globalThis.CSS?.highlights?.delete(ACTIVE_HIGHLIGHT);
    this.matchHighlight = null;
    this.activeHighlight = null;
    this.container.classList.remove("find-highlights-closing");

    if (hadHighlights) this.flushHighlightPaint();
  }

  flushHighlightPaint() {
    // A temporary compositing transform invalidates paint without changing
    // layout. Keep it through one frame so WebKit cannot coalesce it away.
    const previous = this.container.style.transform;
    const forced = previous ? `${previous} translateZ(0)` : "translateZ(0)";
    this.container.style.transform = forced;

    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (this.container.style.transform === forced) {
          this.container.style.transform = previous;
        }
      });
    });
  }

  shouldSkipTextNode(node) {
    const parent = node.parentElement;
    if (!parent) return true;
    return Boolean(parent.closest("script, style, pre"));
  }

  find(query) {
    this.stop();

    query = query.trim();
    if (!query) return;

    this.query = query;
    if (!supportsCustomHighlights()) {
      console.warn("Find highlighting requires the CSS Custom Highlight API");
      return;
    }

    const needle = query.toLowerCase();
    const { text, segments } = collectTextSegments(
      this.container,
      (node) => this.shouldSkipTextNode(node),
    );

    if (!text || segments.length === 0) return;

    const lower = text.toLowerCase();
    const resolveStart = positionResolver(segments);
    const resolveEnd = positionResolver(segments);
    const highlight = new Highlight();
    let from = 0;
    let index;

    while ((index = lower.indexOf(needle, from)) !== -1) {
      const start = resolveStart(index);
      // Use the source query's length for the DOM endpoint. Lowercasing a few
      // Unicode characters can expand them, but DOM offsets address the
      // original text rather than the folded search string.
      const end = resolveEnd(index + query.length);

      if (start && end) {
        const range = document.createRange();
        range.setStart(start.node, start.offset);
        range.setEnd(end.node, end.offset);
        this.matches.push(range);
        highlight.add(range);
      }

      from = index + needle.length;
    }

    if (this.matches.length === 0) return;

    this.matchHighlight = highlight;
    this.activeHighlight = new Highlight();
    // Give the active match precedence when it overlaps the all-matches layer.
    this.activeHighlight.priority = 1;
    globalThis.CSS.highlights.set(MATCH_HIGHLIGHT, this.matchHighlight);
    globalThis.CSS.highlights.set(ACTIVE_HIGHLIGHT, this.activeHighlight);
    this.activeIndex = 0;
    this.scrollToActive();
  }

  next() {
    if (this.matches.length === 0) return;
    this.activeIndex = (this.activeIndex + 1) % this.matches.length;
    this.scrollToActive();
  }

  prev() {
    if (this.matches.length === 0) return;
    this.activeIndex = (this.activeIndex - 1 + this.matches.length) % this.matches.length;
    this.scrollToActive();
  }

  scrollToActive() {
    const active = this.matches[this.activeIndex];
    if (!active || !this.activeHighlight) return;

    this.activeHighlight.clear();
    this.activeHighlight.add(active);

    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const behavior = reduceMotion ? "auto" : "smooth";
    const startElement = active.startContainer.parentElement;
    const block = startElement?.closest(".prose > *");

    // content-visibility gives off-screen ranges no box. Materialize the
    // containing block first, then center the exact match on the next frame.
    if (active.getBoundingClientRect().height === 0 && block) {
      block.scrollIntoView({ block: "center", behavior: "auto" });
      requestAnimationFrame(() => {
        if (this.matches[this.activeIndex] === active) {
          this.centerRange(active, behavior);
        }
      });
      return;
    }

    this.centerRange(active, behavior);
  }

  centerRange(range, behavior) {
    const rect = range.getBoundingClientRect();
    if (rect.height === 0) return;

    // Wide tables scroll independently from the reader. Keep a hit in a
    // clipped cell visible without moving or styling the table itself.
    const horizontalScroller = range.startContainer.parentElement
      ?.closest(".table-wrapper");
    if (horizontalScroller) {
      const scrollerRect = horizontalScroller.getBoundingClientRect();
      const horizontalOffset = rect.left + rect.width / 2
        - scrollerRect.left - horizontalScroller.clientWidth / 2;
      horizontalScroller.scrollBy({ left: horizontalOffset, behavior });
    }

    const root = getScrollRoot();
    const viewportTop = root === document.documentElement
      ? 0
      : root.getBoundingClientRect().top;
    const offset = rect.top + rect.height / 2
      - viewportTop - getScrollViewportHeight() / 2;

    if (root === document.documentElement) {
      window.scrollBy({ top: offset, behavior });
    } else {
      root.scrollBy({ top: offset, behavior });
    }
  }

  get matchCount() {
    return this.matches.length;
  }

  get currentMatchNumber() {
    return this.matches.length > 0 ? this.activeIndex + 1 : 0;
  }
}
