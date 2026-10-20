/**
 * Link target preview.
 *
 * Resting the pointer on a link (or focusing it from the keyboard) shows where
 * it goes in a small muted pill just under the link. Nothing is permanent, so
 * the reading surface stays bare:
 *   - it appears only after a short dwell, so a pointer drifting across a
 *     paragraph never makes anything flicker
 *   - it sits under the link, where the eye already is, not in a screen corner
 *   - it vanishes at once on leave, scroll, click or any key
 *   - it is skipped when it would add nothing: `#anchor` links (the contents
 *     panel covers those) and autolinks, whose text already is the URL
 *
 * A markdown link title (`[x](url "title")`) is shown above the URL. The
 * `title` attribute is moved to `data-link-title` on first hover, otherwise
 * WebKitGTK would also pop its own unstyled tooltip.
 *
 * Like the image zoom button, the pill is one fixed element on `<body>`; the
 * content DOM is never restructured.
 */

const DWELL_MS = 450;
const GAP_PX = 6;
const EDGE_PX = 8;
const MAX_CHARS = 72;

function safeDecode(value) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

// Strip the parts a reader never needs to judge a link: scheme, `www.`, and a
// trailing slash. Also used to recognise autolinks.
function normalize(text) {
  return text
    .trim()
    .replace(/^[a-z][a-z0-9+.-]*:(\/\/)?/i, "")
    .replace(/^www\./i, "")
    .replace(/\/$/, "");
}

// Keep the host and the last path segment, eliding the middle: those are the
// two parts that tell you where a link goes.
function elide(text) {
  if (text.length <= MAX_CHARS) return text;
  const slash = text.indexOf("/");
  const last = text.lastIndexOf("/");
  if (slash > 0 && last > slash) {
    const shortened = `${text.slice(0, slash)}/…${text.slice(last)}`;
    if (shortened.length <= MAX_CHARS) return shortened;
  }
  return `${text.slice(0, MAX_CHARS - 1)}…`;
}

/** What to show for `href`, or null when the preview should be skipped. */
export function describeHref(href, linkText) {
  if (!href || href.startsWith("#")) return null;

  let url;
  try {
    url = new URL(href);
  } catch {
    // Relative: a local file, shown as written.
    return elide(safeDecode(href));
  }

  let shown;
  if (url.protocol === "mailto:" || url.protocol === "tel:") {
    shown = safeDecode(url.pathname);
  } else if (url.protocol === "http:" || url.protocol === "https:") {
    shown = normalize(
      url.host + safeDecode(url.pathname) + url.search + url.hash,
    );
  } else {
    shown = safeDecode(href);
  }

  if (normalize(linkText) === normalize(shown)) return null;
  return elide(shown);
}

export function createLinkPreview({ contentEl }) {
  const pill = document.createElement("div");
  pill.className = "link-preview hidden";
  pill.setAttribute("role", "tooltip");
  pill.id = "link-preview";
  const titleEl = document.createElement("div");
  titleEl.className = "link-preview-title";
  const urlEl = document.createElement("div");
  urlEl.className = "link-preview-url";
  pill.append(titleEl, urlEl);
  document.body.appendChild(pill);

  let anchor = null;
  let timer = 0;
  let pointer = null;

  function hide() {
    clearTimeout(timer);
    timer = 0;
    if (anchor) anchor.removeAttribute("aria-describedby");
    anchor = null;
    pill.classList.add("hidden");
  }

  function takeTitle(a) {
    const title = a.getAttribute("title");
    if (title !== null) {
      a.dataset.linkTitle = title;
      a.removeAttribute("title");
    }
    return a.dataset.linkTitle ?? "";
  }

  // A link that wraps has several boxes; anchor to the one under the pointer.
  function anchorRect(a) {
    const rects = [...a.getClientRects()];
    if (pointer) {
      const hit = rects.find(
        (r) =>
          pointer.x >= r.left &&
          pointer.x <= r.right &&
          pointer.y >= r.top &&
          pointer.y <= r.bottom,
      );
      if (hit) return hit;
    }
    return rects[0] ?? a.getBoundingClientRect();
  }

  function show(a) {
    if (!a.isConnected) return;
    const text = describeHref(a.getAttribute("href"), a.textContent ?? "");
    const title = a.dataset.linkTitle ?? "";
    if (!text && !title) return;

    titleEl.textContent = title;
    titleEl.hidden = !title;
    urlEl.textContent = text ?? "";
    urlEl.hidden = !text;
    pill.classList.remove("hidden");
    a.setAttribute("aria-describedby", pill.id);

    const rect = anchorRect(a);
    const { offsetWidth: w, offsetHeight: h } = pill;
    const maxLeft = window.innerWidth - w - EDGE_PX;
    const left = Math.max(EDGE_PX, Math.min(rect.left, maxLeft));
    let top = rect.bottom + GAP_PX;
    if (top + h > window.innerHeight - EDGE_PX) top = rect.top - GAP_PX - h;
    pill.style.left = `${left}px`;
    pill.style.top = `${Math.max(EDGE_PX, top)}px`;
  }

  function schedule(a, delay) {
    if (a === anchor) return;
    hide();
    takeTitle(a);
    anchor = a;
    timer = setTimeout(() => {
      timer = 0;
      if (anchor === a) show(a);
    }, delay);
  }

  function linkFrom(target) {
    const a = target instanceof Element ? target.closest("a[href]") : null;
    return a && contentEl.contains(a) ? a : null;
  }

  contentEl.addEventListener("pointerover", (event) => {
    pointer = { x: event.clientX, y: event.clientY };
    const a = linkFrom(event.target);
    if (a) schedule(a, DWELL_MS);
  });

  contentEl.addEventListener("pointermove", (event) => {
    pointer = { x: event.clientX, y: event.clientY };
  });

  contentEl.addEventListener("pointerout", (event) => {
    if (!anchor) return;
    if (event.relatedTarget instanceof Node && anchor.contains(event.relatedTarget)) return;
    hide();
  });

  contentEl.addEventListener("focusin", (event) => {
    const a = linkFrom(event.target);
    if (a && a.matches(":focus-visible")) {
      pointer = null;
      schedule(a, 0);
    }
  });

  contentEl.addEventListener("focusout", hide);
  contentEl.addEventListener("pointerdown", hide);
  window.addEventListener("scroll", hide, { capture: true, passive: true });
  window.addEventListener("keydown", hide, true);
  window.addEventListener("blur", hide);

  return { hide };
}
