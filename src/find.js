function normalizeTextNodes(root) {
  const elements = [root];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);

  let element;
  while ((element = walker.nextNode())) {
    elements.push(element);
  }

  for (const parent of elements) {
    let child = parent.firstChild;
    while (child) {
      if (child.nodeType === Node.TEXT_NODE) {
        let next = child.nextSibling;
        while (next && next.nodeType === Node.TEXT_NODE) {
          child.textContent += next.textContent;
          const remove = next;
          next = next.nextSibling;
          parent.removeChild(remove);
        }
      }
      child = child.nextSibling;
    }
  }
}

function collectTextSegments(container, shouldSkipTextNode) {
  const segments = [];
  let text = '';

  const walker = document.createTreeWalker(
    container,
    NodeFilter.SHOW_TEXT,
    {
      acceptNode: (node) => (
        shouldSkipTextNode(node)
          ? NodeFilter.FILTER_REJECT
          : NodeFilter.FILTER_ACCEPT
      ),
    },
  );

  let node;
  while ((node = walker.nextNode())) {
    const content = node.textContent;
    if (!content) continue;

    const start = text.length;
    text += content;
    segments.push({ node, start, end: start + content.length });
  }

  return { text, segments };
}

function resolveTextPosition(segments, index) {
  for (const segment of segments) {
    const length = segment.node.textContent.length;
    const segmentEnd = segment.start + length;
    if (index < segmentEnd || (index === segmentEnd && segment === segments.at(-1))) {
      return { node: segment.node, offset: index - segment.start };
    }
    if (index === segmentEnd) {
      continue;
    }
  }

  const last = segments.at(-1);
  if (!last) return null;
  return { node: last.node, offset: last.node.textContent.length };
}

function wrapRange(range) {
  const mark = document.createElement('mark');
  mark.setAttribute('data-find-match', '');

  try {
    range.surroundContents(mark);
  } catch {
    const fragment = range.extractContents();
    mark.appendChild(fragment);
    range.insertNode(mark);
  }

  return mark;
}

export class FindInPage {
  constructor(container) {
    this.container = container;
    this.matches = [];
    this.activeIndex = -1;
    this.query = '';
  }

  stop() {
    const marks = this.container.querySelectorAll('mark[data-find-match]');
    for (const mark of marks) {
      const text = document.createTextNode(mark.textContent);
      mark.parentNode.replaceChild(text, mark);
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
      this.matches.unshift(wrapRange(range));
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
