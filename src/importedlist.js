/**
 * The list of imported themes in the settings panel, one instance per kind
 * (code themes, color templates). It renders nothing until something has been
 * imported, so the panel looks the same for anyone who never imports.
 *
 * Removing a row is deferred rather than confirmed: the row offers Undo for
 * `UNDO_MS`, and only then does `commit` delete the file. `flush` ends every
 * undo window at once, for when the panel or window closes or another import
 * starts. If the app dies inside the window, the theme is simply still there
 * at the next launch.
 */

const UNDO_MS = 8000;

/**
 * @param {object} options
 * @param {HTMLElement} options.element the `<ul>` to fill
 * @param {string} options.glyph sample text for each row's swatch
 * @param {HTMLElement} options.fallbackFocus takes focus when a focused row goes away
 * @param {(item: object) => boolean} options.isActive whether the row is in use
 * @param {(item: object) => void} options.onApply a row was clicked
 * @param {(item: object) => any} options.onRemove the row was removed, and
 *   `available()` already leaves it out; what this returns goes to `onRestore`
 * @param {(item: object, context: any) => void} options.onRestore the removal was
 *   undone, or its delete failed, and `available()` lists the row again
 * @param {(item: object) => Promise<void>} options.commit delete the item for good
 * @param {(item: object, err: unknown) => void} options.onCommitError the delete
 *   failed, and the row is back in the list
 *
 * Items are `{ id, label, bg, fg }` plus whatever the callbacks need.
 */
export function createImportedList({
  element,
  glyph,
  fallbackFocus,
  isActive,
  onApply,
  onRemove,
  onRestore,
  commit,
  onCommitError,
}) {
  let items = [];
  /** Removed rows still in their undo window: id → `{ item, context, timer, committing }`. */
  const pending = new Map();

  function button(className, part, text, onClick) {
    const el = document.createElement("button");
    el.type = "button";
    el.className = className;
    el.dataset.part = part;
    el.textContent = text;
    el.addEventListener("click", onClick);
    return el;
  }

  function appliedRow(item) {
    const row = document.createElement("li");
    row.className = "imported-row";
    row.dataset.id = item.id;

    const apply = button("imported-apply", "apply", "", () => onApply(item));
    apply.title = item.label;
    const swatch = document.createElement("span");
    swatch.className = "imported-swatch";
    swatch.setAttribute("aria-hidden", "true");
    swatch.textContent = glyph;
    swatch.style.backgroundColor = item.bg;
    swatch.style.color = item.fg;
    const label = document.createElement("span");
    label.className = "imported-label";
    label.textContent = item.label;
    const check = document.createElement("span");
    check.className = "imported-check";
    check.setAttribute("aria-hidden", "true");
    check.textContent = "✓";
    apply.append(swatch, label, check);

    const remove = button("imported-remove", "remove", "×", () => removeItem(item));
    remove.title = `Remove ${item.label}`;
    remove.setAttribute("aria-label", `Remove ${item.label}`);

    row.append(apply, remove);
    return row;
  }

  function removedRow(item) {
    const row = document.createElement("li");
    row.className = "imported-row imported-row--removed";
    row.dataset.id = item.id;

    const label = document.createElement("span");
    label.className = "imported-label";
    label.setAttribute("role", "status");
    label.textContent = `Removed ${item.label}`;

    const undo = button("imported-undo", "undo", "Undo", () => undoRemoval(item));
    undo.setAttribute("aria-label", `Undo removing ${item.label}`);

    row.append(label, undo);
    return row;
  }

  function focusedControl() {
    const focused = document.activeElement;
    if (!element.contains(focused)) return null;
    return { id: focused.closest(".imported-row")?.dataset.id, part: focused.dataset.part };
  }

  function findControl({ id, part }) {
    for (const row of element.children) {
      if (row.dataset.id === id) return row.querySelector(`[data-part="${part}"]`);
    }
    return null;
  }

  /** Rebuild the rows. Focus only moves if it was already inside the list. */
  function render(focusTarget) {
    const focused = focusedControl();
    element.replaceChildren(
      ...items.map((item) => (pending.has(item.id) ? removedRow(item) : appliedRow(item))),
    );
    element.hidden = items.length === 0;
    syncActive();
    if (focused) {
      (findControl(focusTarget ?? focused) ?? fallbackFocus).focus();
    }
  }

  function syncActive() {
    for (const row of element.children) {
      // A row showing Undo has no apply control. It may be mid-restore, so go by
      // what is on screen and not by `pending`.
      const apply = row.querySelector('[data-part="apply"]');
      const item = items.find((candidate) => candidate.id === row.dataset.id);
      if (!apply || !item) continue;
      const active = isActive(item);
      row.classList.toggle("imported-row--active", active);
      if (active) apply.setAttribute("aria-current", "true");
      else apply.removeAttribute("aria-current");
    }
  }

  function removeItem(item) {
    if (pending.has(item.id)) return;
    const entry = {
      item,
      context: undefined,
      timer: setTimeout(() => void commitRemoval(item.id), UNDO_MS),
      committing: null,
    };
    pending.set(item.id, entry);
    entry.context = onRemove(item);
    render({ id: item.id, part: "undo" });
  }

  function undoRemoval(item) {
    const entry = pending.get(item.id);
    if (!entry || entry.committing) return;
    clearTimeout(entry.timer);
    pending.delete(item.id);
    onRestore(item, entry.context);
    render({ id: item.id, part: "remove" });
  }

  /** Delete for good. Resolves when done, and never rejects. */
  function commitRemoval(id) {
    const entry = pending.get(id);
    if (!entry) return Promise.resolve();
    entry.committing ??= (async () => {
      clearTimeout(entry.timer);
      let failure = null;
      try {
        await commit(entry.item);
        items = items.filter((item) => item.id !== id);
      } catch (err) {
        failure = { err };
      }
      pending.delete(id);
      if (failure) {
        onRestore(entry.item, entry.context);
        onCommitError(entry.item, failure.err);
      }
      render();
    })();
    return entry.committing;
  }

  return {
    /** Replace the rows with a fresh listing. Rows in an undo window stay in it. */
    setItems(next) {
      const listed = new Set(next.map((item) => item.id));
      items = [...next, ...[...pending.values()].map((e) => e.item).filter((i) => !listed.has(i.id))];
      items.sort((a, b) => a.label.toLowerCase().localeCompare(b.label.toLowerCase()));
      render();
    },
    /** The items not removed: what a picker beside this list may still offer. */
    available() {
      return items.filter((item) => !pending.has(item.id));
    },
    syncActive,
    /** End every undo window now, deleting those files. Never rejects. */
    flush() {
      return Promise.all([...pending.keys()].map(commitRemoval));
    },
  };
}
