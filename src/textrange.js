/**
 * Shared DOM text-range primitives.
 *
 * Both in-page search (`find.js`) and change highlighting (`changes.js`) need
 * the same trick: locate a run of text inside rendered HTML by character offset
 * and wrap it in an element, without ever re-serializing the markup. These
 * helpers do that by walking text nodes and moving them, so inline structure
 * (`<em>`, `<code>`, links) survives being wrapped and unwrapped.
 */

/**
 * Merge adjacent text nodes so offsets stay stable across repeated collection.
 * Equivalent to `Node.normalize()`, but explicit about the traversal so a caller
 * can reason about when it happens — wrapping splits text nodes, so this has to
 * run again between wraps.
 */
export function normalizeTextNodes(root) {
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

/**
 * Flatten `container` into a single string plus an index mapping each global
 * character offset back to the text node it came from.
 */
export function collectTextSegments(container, shouldSkipTextNode) {
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

/** Translate a global offset from `collectTextSegments` into a `{node, offset}` pair. */
export function resolveTextPosition(segments, index) {
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

/**
 * Wrap `range` in a fresh element, calling `decorate` to brand it.
 *
 * `surroundContents` is the fast path but throws when the range only partially
 * selects a non-text node — a match spanning `foo <em>bar</em> baz`. The
 * fallback moves the whole fragment inside the wrapper instead, which is why
 * this never damages the markup it wraps.
 */
export function wrapRange(range, decorate, tagName = 'mark') {
  const wrapper = document.createElement(tagName);
  if (decorate) decorate(wrapper);

  try {
    range.surroundContents(wrapper);
  } catch {
    const fragment = range.extractContents();
    wrapper.appendChild(fragment);
    range.insertNode(wrapper);
  }

  return wrapper;
}

/**
 * Remove a wrapper, promoting its children back into the parent.
 *
 * Deliberately *not* `replaceChild(createTextNode(el.textContent), el)`: that
 * flattens away any inline markup the wrapper had swallowed via `wrapRange`'s
 * fallback path, and would also destroy nested wrappers belonging to the other
 * highlighting feature.
 */
export function unwrap(element) {
  const parent = element.parentNode;
  if (!parent) return;
  while (element.firstChild) {
    parent.insertBefore(element.firstChild, element);
  }
  parent.removeChild(element);
}
