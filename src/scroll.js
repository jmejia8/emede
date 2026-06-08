const readerEl = () => document.getElementById("reader");

export function getScrollRoot() {
  return document.body.classList.contains("frame-emede") ? readerEl() : document.documentElement;
}

export function getScrollViewportHeight() {
  const root = getScrollRoot();
  if (root === document.documentElement) return window.innerHeight;
  return root?.clientHeight ?? window.innerHeight;
}
