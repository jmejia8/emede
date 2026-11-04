// The font fields of the settings panel: one combobox per font slot, over a
// hidden `<select>` that holds the curated presets and the current value.
//
// The curated presets in `main.js` stay exactly as they are; typing adds a way
// to reach a font we never hardcoded. System fonts are only listed for a query:
// the backend answers with at most `LIMIT` matches and never ships the full
// font list across IPC.

const LIMIT = 12;
const DEBOUNCE_MS = 120;

// Where a picked system font lands in the `<select>`, so an arbitrary family
// can be the element's value (a `<select>` cannot hold a value with no option).
const CUSTOM_GROUP_LABEL = "From your system";

/// Put `stack` into `select` as the current value, adding an option for it when
/// it is not one of the curated presets. Exported because restoring saved
/// settings hits the same problem: the stored value may name a system font.
export function selectFontStack(select, stack, label) {
  if (!stack) return false;
  const existing = Array.from(select.options).find((option) => option.value === stack);
  if (existing) {
    select.value = stack;
    return true;
  }

  let group = select.querySelector(`optgroup[label="${CUSTOM_GROUP_LABEL}"]`);
  if (!group) {
    group = document.createElement("optgroup");
    group.label = CUSTOM_GROUP_LABEL;
    select.appendChild(group);
  }
  // Only ever one custom entry per select: the previous one is either a preset
  // (still in its own group) or a system font the user has now replaced.
  group.replaceChildren();

  const option = document.createElement("option");
  option.value = stack;
  option.textContent = label || stack.split(",")[0].replace(/"/g, "");
  option.style.fontFamily = stack;
  group.appendChild(option);
  select.value = stack;
  return true;
}

// Fold case and accents, so "noto" finds "Noto" and "cafe" finds "Café".
function fold(text) {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

// The family a stack names first, which is what a system search result is.
function firstFamily(stack) {
  return stack.split(",")[0].trim().replace(/^["']|["']$/g, "");
}

/**
 * Wire one `.font-combo` to the hidden `<select>` named by its
 * `data-font-select` attribute: a single text field that both shows the
 * current font and searches for another one.
 *
 * The `<select>` stays the source of truth (`main.js` reads its `value`,
 * `markUnavailableFonts` annotates its options, `selectFontStack` adds system
 * fonts to it), and this only renders it. Focusing the field lists the
 * curated presets; typing filters them and also asks the backend for system
 * fonts, which never ships the full font list across IPC.
 *
 * @param {Element} root the `.font-combo` container
 * @param {(query: string, limit: number) => Promise<Array>} searchFonts backend query
 * @param {() => void} onChange called after a pick, to persist settings
 * @returns {{ sync: () => void }} refreshes the field from the select
 */
export function attachFontCombobox(root, searchFonts, onChange) {
  const select = document.getElementById(root.dataset.fontSelect);
  const input = root.querySelector(".font-combo-input");
  const list = root.querySelector(".font-combo-list");
  if (!select || !input || !list) return { sync() {} };

  // The code slot only offers monospace faces; a proportional font there is
  // never what someone means.
  const monoOnly = root.dataset.fontMono === "true";
  let timer = null;
  let seq = 0;
  let items = [];
  let active = -1;

  const currentLabel = () => select.selectedOptions[0]?.textContent ?? "";

  const sync = () => {
    if (!list.hidden) return;
    input.value = currentLabel();
    // Show the field in its own face, as long as there is one to show.
    input.style.fontFamily = select.value || "";
  };

  const setActive = (index) => {
    const options = list.querySelectorAll("[role=option]");
    options[active]?.classList.remove("is-active");
    active = Math.max(-1, Math.min(items.length - 1, index));
    const el = options[active];
    if (el) {
      el.classList.add("is-active");
      el.scrollIntoView({ block: "nearest" });
      input.setAttribute("aria-activedescendant", el.id);
    } else {
      input.removeAttribute("aria-activedescendant");
    }
  };

  // Curated options straight from the select, keeping its optgroups as
  // headings. An option outside any group ("Same as body") gets none.
  const curated = (query) => {
    const q = fold(query);
    const groups = [];
    for (const child of select.children) {
      const options = child.tagName === "OPTGROUP" ? Array.from(child.children) : [child];
      const matched = options
        .filter((option) => !q || fold(option.textContent).includes(q))
        .map((option) => ({
          label: option.textContent,
          value: option.value,
          font: option.value,
          muted: option.classList.contains("font-missing") ||
            option.classList.contains("font-substituted"),
        }));
      if (matched.length) {
        groups.push({ label: child.tagName === "OPTGROUP" ? child.label : "", items: matched });
      }
    }
    return groups;
  };

  const render = (groups, { empty = "", hint = "" } = {}) => {
    list.replaceChildren();
    items = [];
    active = -1;
    input.removeAttribute("aria-activedescendant");
    for (const group of groups) {
      if (group.label) {
        const heading = document.createElement("li");
        heading.className = "font-combo-heading";
        heading.setAttribute("role", "presentation");
        heading.textContent = group.label;
        list.appendChild(heading);
      }
      for (const item of group.items) {
        const li = document.createElement("li");
        li.id = `${list.id}-${items.length}`;
        li.className = "font-combo-option";
        li.setAttribute("role", "option");
        li.setAttribute("aria-selected", String(!item.system && item.value === select.value));
        li.dataset.index = String(items.length);
        // Preview each candidate in its own face, the whole reason to pick by
        // sight rather than by name.
        if (item.font) li.style.fontFamily = item.font;
        li.classList.toggle("is-muted", Boolean(item.muted));
        li.textContent = item.label;
        if (item.generic) {
          const tag = document.createElement("span");
          tag.className = "font-combo-generic";
          tag.textContent = item.generic === "sans-serif" ? "sans" : item.generic;
          li.appendChild(tag);
        }
        list.appendChild(li);
        items.push(item);
      }
    }
    for (const text of [empty && !items.length ? empty : "", hint]) {
      if (!text) continue;
      const note = document.createElement("li");
      note.className = "font-combo-note";
      note.setAttribute("role", "presentation");
      note.textContent = text;
      list.appendChild(note);
    }
  };

  const open = () => {
    render(curated(""), { hint: "Type to search your system fonts" });
    list.hidden = false;
    input.setAttribute("aria-expanded", "true");
    setActive(items.findIndex((item) => item.value === select.value));
  };

  const close = () => {
    clearTimeout(timer);
    seq++;
    list.hidden = true;
    list.replaceChildren();
    items = [];
    input.setAttribute("aria-expanded", "false");
    input.removeAttribute("aria-activedescendant");
    sync();
  };

  const pick = (item) => {
    if (!item) return;
    if (item.system) selectFontStack(select, item.value, item.label);
    else select.value = item.value;
    close();
    onChange();
  };

  const search = async (query) => {
    const mine = ++seq;
    let fonts = [];
    try {
      fonts = await searchFonts(query, monoOnly ? LIMIT * 3 : LIMIT);
    } catch {
      // No fontconfig, or the command failed: the curated presets are still
      // listed, so degrade to silence rather than an error.
      fonts = [];
    }
    // A slower earlier query must not overwrite a newer one's results.
    if (mine !== seq || list.hidden) return;
    if (monoOnly) fonts = fonts.filter((font) => font.generic === "monospace");

    const groups = curated(query);
    const known = new Set(
      groups.flatMap((group) => group.items.map((item) => fold(firstFamily(item.value || "")))),
    );
    const system = fonts
      .filter((font) => !known.has(fold(font.family)))
      .slice(0, LIMIT)
      .map((font) => ({
        label: font.family,
        value: font.stack,
        font: font.stack,
        generic: font.generic,
        system: true,
      }));
    // A system font picked earlier already sits in the select's own "From your
    // system" group; the results join it rather than repeat the heading.
    const own = groups.find((group) => group.label === CUSTOM_GROUP_LABEL);
    if (own) own.items.push(...system);
    else if (system.length) groups.push({ label: CUSTOM_GROUP_LABEL, items: system });
    render(groups, { empty: "No matching fonts installed" });
    setActive(0);
  };

  input.addEventListener("focus", () => {
    open();
    input.select();
  });

  input.addEventListener("mousedown", () => {
    if (list.hidden && document.activeElement === input) open();
  });

  input.addEventListener("input", () => {
    if (list.hidden) {
      list.hidden = false;
      input.setAttribute("aria-expanded", "true");
    }
    clearTimeout(timer);
    const query = input.value.trim();
    render(curated(query), { hint: query ? "Searching your system…" : "" });
    setActive(query ? 0 : -1);
    if (!query) return;
    timer = setTimeout(() => void search(query), DEBOUNCE_MS);
  });

  input.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (list.hidden) {
        open();
        return;
      }
      setActive(active + (event.key === "ArrowDown" ? 1 : -1));
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (!list.hidden) pick(items[active] ?? items[0]);
    } else if (event.key === "Escape" && !list.hidden) {
      // Swallow it, so closing the list does not also close the settings panel.
      event.preventDefault();
      event.stopPropagation();
      close();
      input.select();
    } else if (event.key === "Tab") {
      close();
    }
  });

  // Keep focus in the field while clicking an option, so the click lands.
  list.addEventListener("mousedown", (event) => event.preventDefault());
  list.addEventListener("click", (event) => {
    const option = event.target.closest("[role=option]");
    if (option) pick(items[Number(option.dataset.index)]);
  });

  input.addEventListener("blur", () => close());

  sync();
  return { sync };
}

/**
 * Annotate curated preset options with what they will *actually* render as.
 *
 * A preset is a stack, not a family, so a missing first family is not
 * necessarily a broken preset: `"Literata", "Noto Serif", serif` still reads
 * fine when Noto Serif is installed. Three outcomes, then — renders as asked
 * (no annotation), falls back to a later family in its own stack (say which),
 * or resolves to nothing we named and lands on a WebKit default (mark it).
 *
 * @param {HTMLSelectElement[]} selects
 * @param {(families: string[]) => Promise<Record<string, boolean>>} checkFonts
 */
export async function markUnavailableFonts(selects, checkFonts) {
  const GENERIC = /^(serif|sans-serif|monospace|system-ui|ui-serif|ui-monospace)$/i;
  const families = new Set();
  const entries = [];

  for (const select of selects) {
    for (const option of select.options) {
      if (!option.value) continue;
      const stack = option.value
        .split(",")
        .map((part) => part.trim().replace(/^["']|["']$/g, ""))
        .filter((family) => family && !GENERIC.test(family));
      if (!stack.length) continue;
      for (const family of stack) families.add(family);
      // Remember the pristine label so repeated runs annotate, never stack up.
      if (option.dataset.baseLabel === undefined) option.dataset.baseLabel = option.textContent;
      entries.push({ option, stack });
    }
  }
  if (!families.size) return;

  let available;
  try {
    available = await checkFonts(Array.from(families));
  } catch {
    // No fontconfig: leave every label as authored rather than guessing.
    return;
  }

  for (const { option, stack } of entries) {
    const base = option.dataset.baseLabel ?? option.textContent;
    const resolved = stack.find((family) => available[family] !== false);

    if (resolved === stack[0]) {
      option.textContent = base;
      option.classList.remove("font-missing", "font-substituted");
    } else if (resolved) {
      option.textContent = `${base} \u2192 ${resolved}`;
      option.classList.remove("font-missing");
      option.classList.add("font-substituted");
    } else {
      option.textContent = `${base} (not installed)`;
      option.classList.remove("font-substituted");
      option.classList.add("font-missing");
    }
  }
}
