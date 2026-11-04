// Small controls for the settings panel: steppers, segmented radio groups, and
// "presets + custom" fields that pair the two.
//
// Each wraps a native element that stays the source of truth, so `main.js`
// keeps reading and writing `.value` / `.checked` as before: a stepper is a
// `<input type="number">` with −/+ buttons, a segmented control is a group of
// radios. Neither restyles anything it does not own.

function decimals(step) {
  const text = String(step);
  const dot = text.indexOf(".");
  return dot === -1 ? 0 : text.length - dot - 1;
}

/**
 * Clamp a number input to its `min`/`max` and round to its `step` grid, so a
 * typed `13.3` cm lands on `13.5` like a button press would. An empty or
 * unparsable entry falls back to `fallback`.
 */
function normalizeNumber(input, value, fallback) {
  const min = Number(input.min);
  const max = Number(input.max);
  const step = Number(input.step) || 1;
  let n = Number(value);
  if (value === "" || !Number.isFinite(n)) n = Number(fallback);
  if (!Number.isFinite(n)) n = min;
  n = Math.min(max, Math.max(min, n));
  // Snap relative to `min`, which is how the browser's own spin buttons count.
  n = min + Math.round((n - min) / step) * step;
  return Number(n.toFixed(decimals(step)));
}

/** Refresh a stepper's buttons after its input changed programmatically. */
export function syncStepper(root) {
  const input = root.querySelector("input");
  const value = Number(input.value);
  for (const button of root.querySelectorAll(".stepper-btn")) {
    const delta = Number(button.dataset.delta);
    button.disabled = delta < 0 ? value <= Number(input.min) : value >= Number(input.max);
  }
}

/**
 * Wire a `[data-stepper]`: −/+ buttons step its number input by `step`, and a
 * typed value is normalized when committed. Listeners should use `change`, not
 * `input`: typing fires `input` on every keystroke, including a half-typed or
 * empty field, while `change` only fires once the value is committed (and the
 * buttons dispatch it too).
 */
export function wireStepper(root) {
  const input = root.querySelector("input");
  let committed = input.value;

  const commit = (value) => {
    input.value = String(normalizeNumber(input, value, committed));
    committed = input.value;
    syncStepper(root);
  };

  for (const button of root.querySelectorAll(".stepper-btn")) {
    button.addEventListener("click", () => {
      const step = Number(input.step) || 1;
      const current = Number(input.value === "" ? committed : input.value);
      commit(current + Number(button.dataset.delta) * step);
      input.dispatchEvent(new Event("change", { bubbles: true }));
    });
  }

  // Registered before anything in `main.js`, so its listeners see the
  // normalized value.
  input.addEventListener("change", () => commit(input.value));
  input.addEventListener("focus", () => {
    committed = input.value;
    input.select();
  });
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") input.blur();
    if (event.key === "Escape" && input.value !== committed) {
      // Abandon the edit without letting Escape close the panel too.
      event.stopPropagation();
      input.value = committed;
      input.blur();
    }
  });
  // A scroll over the panel must never change a value it happens to cross.
  input.addEventListener("wheel", (event) => {
    if (document.activeElement === input) event.preventDefault();
  }, { passive: false });

  syncStepper(root);
}

/**
 * A segmented control backed by native radios (arrow keys, focus, and screen
 * reader semantics come for free). Exposes `value` and `addEventListener` like
 * the `<select>` it replaces, so existing code reads it the same way.
 */
export class RadioGroup {
  constructor(element) {
    this.element = element;
  }

  get radios() {
    return Array.from(this.element.querySelectorAll('input[type="radio"]'));
  }

  get value() {
    return this.radios.find((radio) => radio.checked)?.value ?? "";
  }

  set value(value) {
    for (const radio of this.radios) radio.checked = radio.value === String(value);
  }

  addEventListener(...args) {
    this.element.addEventListener(...args);
  }
}

/**
 * Pair a segmented preset group with a stepper for "Custom". Picking a preset
 * writes its value into the stepper's input and fires `change` on it; picking
 * Custom reveals the stepper. Custom stays selected while it is being used,
 * even when stepping lands on a preset's value, so the row under the pointer
 * never disappears mid-adjustment.
 *
 * @param {Element} groupEl the `role="radiogroup"` with a `value="custom"` radio
 * @param {HTMLInputElement} input the stepper's number input
 */
export function wirePresetField(groupEl, input) {
  const group = new RadioGroup(groupEl);
  const customRow = groupEl.parentElement.querySelector("[data-preset-custom]");
  let pinned = false;

  const show = (custom) => {
    customRow.hidden = !custom;
    group.value = custom ? "custom" : String(Number(input.value));
  };

  const sync = ({ unpin = false } = {}) => {
    if (unpin) pinned = false;
    const matches = group.radios.some(
      (radio) => radio.value !== "custom" && Number(radio.value) === Number(input.value),
    );
    show(pinned || !matches);
  };

  groupEl.addEventListener("change", (event) => {
    const radio = event.target;
    if (!(radio instanceof HTMLInputElement)) return;
    event.stopPropagation();
    if (radio.value === "custom") {
      pinned = true;
      show(true);
      return;
    }
    pinned = false;
    input.value = radio.value;
    input.dispatchEvent(new Event("change", { bubbles: true }));
    show(false);
  });
  // The radios are not settings themselves; only the number input is.
  groupEl.addEventListener("input", (event) => event.stopPropagation());
  input.addEventListener("change", () => sync());

  return { sync };
}
