// Search-as-you-type over the system's installed fonts, attached under a font
// `<select>` in the settings panel.
//
// The curated presets in `main.js` stay exactly as they are; this only adds a
// way to reach a font we never hardcoded. Nothing is listed until the user
// types — the backend answers a query with at most `LIMIT` matches and never
// ships the full font list across IPC.

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

function renderResults(list, fonts, { onPick }) {
  list.replaceChildren();
  if (!fonts.length) {
    const empty = document.createElement("li");
    empty.className = "font-search-empty";
    empty.textContent = "No matching fonts installed";
    list.appendChild(empty);
    list.hidden = false;
    return;
  }

  for (const font of fonts) {
    const item = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.className = "font-search-result";
    // Preview each candidate in its own face — the whole reason to pick by
    // sight rather than by name.
    button.style.fontFamily = font.stack;
    button.textContent = font.family;

    const tag = document.createElement("span");
    tag.className = "font-search-generic";
    tag.textContent = font.generic === "sans-serif" ? "sans" : font.generic;
    button.appendChild(tag);

    button.addEventListener("click", () => onPick(font));
    item.appendChild(button);
    list.appendChild(item);
  }
  list.hidden = false;
}

/**
 * Wire one `.font-search` block to the `<select>` named by its
 * `data-font-search` attribute.
 *
 * @param {Element} root the `.font-search` container
 * @param {(query: string, limit: number) => Promise<Array>} searchFonts backend query
 * @param {() => void} onChange called after a pick, to persist settings
 */
export function attachFontSearch(root, searchFonts, onChange) {
  const select = document.getElementById(root.dataset.fontSearch);
  const input = root.querySelector(".font-search-input");
  const list = root.querySelector(".font-search-results");
  if (!select || !input || !list) return;

  // The code slot only offers monospace faces; a proportional font there is
  // never what someone means.
  const monoOnly = root.dataset.fontSearchMono === "true";
  let timer = null;
  let seq = 0;

  const close = () => {
    list.hidden = true;
    list.replaceChildren();
  };

  const pick = (font) => {
    selectFontStack(select, font.stack, font.family);
    input.value = "";
    close();
    onChange();
  };

  const run = async (query) => {
    const mine = ++seq;
    let fonts = [];
    try {
      fonts = await searchFonts(query, monoOnly ? LIMIT * 3 : LIMIT);
    } catch {
      // No fontconfig, or the command failed: the curated presets are still
      // there, so degrade to silence rather than an error banner.
      close();
      return;
    }
    // A slower earlier query must not overwrite a newer one's results.
    if (mine !== seq) return;
    if (monoOnly) fonts = fonts.filter((font) => font.generic === "monospace");
    renderResults(list, fonts.slice(0, LIMIT), { onPick: pick });
  };

  input.addEventListener("input", () => {
    clearTimeout(timer);
    const query = input.value.trim();
    if (!query) {
      close();
      return;
    }
    timer = setTimeout(() => void run(query), DEBOUNCE_MS);
  });

  input.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !list.hidden) {
      // Swallow it, so closing the results does not also close the settings panel.
      event.stopPropagation();
      event.preventDefault();
      close();
      return;
    }
    if (event.key === "ArrowDown") {
      const first = list.querySelector(".font-search-result");
      if (first) {
        event.preventDefault();
        first.focus();
      }
    }
    if (event.key === "Enter") {
      event.preventDefault();
      const first = list.querySelector(".font-search-result");
      if (first) first.click();
    }
  });

  list.addEventListener("keydown", (event) => {
    const buttons = Array.from(list.querySelectorAll(".font-search-result"));
    const index = buttons.indexOf(document.activeElement);
    if (event.key === "ArrowDown" && index >= 0 && index < buttons.length - 1) {
      event.preventDefault();
      buttons[index + 1].focus();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      if (index > 0) buttons[index - 1].focus();
      else input.focus();
    } else if (event.key === "Escape") {
      event.stopPropagation();
      event.preventDefault();
      close();
      input.focus();
    }
  });

  root.addEventListener("focusout", () => {
    // Let focus settle first: moving from the input into a result is a focusout
    // on the input, and closing there would kill the click.
    setTimeout(() => {
      if (!root.contains(document.activeElement)) close();
    }, 0);
  });
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
