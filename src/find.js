import {
  normalizeTextNodes,
  collectTextSegments,
  resolveTextPosition,
  wrapRange,
  unwrap,
} from "./textrange.js";

/** Brands a wrapper as a find hit, so `stop()` can find it again. */
function markFindHit(mark) {
  mark.setAttribute('data-find-match', '');
}

export class FindInPage {
  constructor(container) {
    this.container = container;
    this.matches = [];
    this.activeIndex = -1;
    this.query = '';
  }

  stop() {
    // Unwrap rather than flatten to a text node: a hit that spans inline markup
    // (`foo <em>bar</em>`) is wrapped via `wrapRange`'s `extractContents`
    // fallback, so its children are real elements — and change highlighting may
    // have its own marks nested inside.
    const marks = this.container.querySelectorAll('mark[data-find-match]');
    for (const mark of marks) {
      unwrap(mark);
    }
    normalizeTextNodes(this.container);
    this.matches = [];
    this.activeIndex = -1;
    this.query = '';
  }

  shouldSkipTextNode(node) {
    const parent = node.parentElement;
    if (!parent) return true;
    return Boolean(parent.closest('script, style, pre, mark[data-find-match]'));
  }

  find(query) {
    this.stop();

    query = query.trim();
    if (!query) return;

    this.query = query;
    normalizeTextNodes(this.container);

    const needle = query.toLowerCase();
    const { text, segments } = collectTextSegments(
      this.container,
      (node) => this.shouldSkipTextNode(node),
    );

    if (!text || segments.length === 0) return;

    const lower = text.toLowerCase();
    const hits = [];
    let from = 0;
    let idx;

    while ((idx = lower.indexOf(needle, from)) !== -1) {
      hits.push({ start: idx, end: idx + query.length });
      from = idx + needle.length;
    }

    for (let i = hits.length - 1; i >= 0; i--) {
      const { start, end } = hits[i];
      normalizeTextNodes(this.container);
      const current = collectTextSegments(
        this.container,
        (node) => this.shouldSkipTextNode(node),
      );
      const startPos = resolveTextPosition(current.segments, start);
      const endPos = resolveTextPosition(current.segments, end);
      if (!startPos || !endPos) continue;

      const range = document.createRange();
      range.setStart(startPos.node, startPos.offset);
      range.setEnd(endPos.node, endPos.offset);
      this.matches.unshift(wrapRange(range, markFindHit));
    }

    if (this.matches.length > 0) {
      this.activeIndex = 0;
      this.scrollToActive();
    }
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
    for (let i = 0; i < this.matches.length; i++) {
      this.matches[i].classList.toggle('find-active', i === this.activeIndex);
    }
    const active = this.matches[this.activeIndex];
    if (active) {
      const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      active.scrollIntoView({ block: 'center', behavior: reduceMotion ? 'auto' : 'smooth' });
    }
  }

  get matchCount() {
    return this.matches.length;
  }

  get currentMatchNumber() {
    return this.matches.length > 0 ? this.activeIndex + 1 : 0;
  }
}
