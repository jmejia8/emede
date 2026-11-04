/**
 * Diagrams: ```mermaid fences drawn by Pele (https://pele.run), which reads
 * Mermaid syntax with its own layout engine and visual style.
 *
 * Pele is vendored as its lazy ESM build (`src/vendor/pele/`, copied at
 * postinstall), so only the diagram types a document uses are ever loaded,
 * the same way `syntax.js` loads highlight.js grammars.
 *
 * Colors are not passed in. Every color in Pele's SVG is a `var(--pele-*)`,
 * and `styles.css` defines those on `:root` from the page's own `--color-*`
 * tokens, so a theme change restyles every diagram without a re-render, and
 * a diagram's own Mermaid `theme`/`themeVariables` cannot repaint it. Fonts
 * are the exception: labels are measured, so a font change has to re-lay the
 * diagram out (`refreshDiagrams`).
 *
 * Pele measures text on a canvas and reads the container's `clientWidth`,
 * neither of which sees the page sheet's zoom transform, so diagrams render
 * in place (Mermaid had to render outside the sheet).
 */

const PELE_URL = "/vendor/pele/lazy.js";

let pelePromise = null;

function loadPele() {
  pelePromise ??= import(PELE_URL).catch((err) => {
    pelePromise = null;
    console.warn("Failed to load Pele", err);
    return null;
  });
  return pelePromise;
}

/** Every diagram currently drawn, by its container: `{ diagram, fontSize }`. */
const mounted = new Map();

/**
 * Labels follow the reading size: 12pt body text is Pele's own 16px, and the
 * diagram is still scaled down to fit the column when it is wider.
 */
function mountOptions(contentEl) {
  const size = parseFloat(getComputedStyle(contentEl).fontSize);
  return {
    // emede has its own zoom viewer (`imagezoom.js`).
    zoom: false,
    fontSize: size > 0 ? Math.round(size * 100) / 100 : 16,
    linkRel: "noopener noreferrer",
    onError: (err) => console.warn("Diagram failed to render", err),
  };
}

function forgetDetached() {
  for (const [el, entry] of mounted) {
    if (el.isConnected) continue;
    entry.diagram.destroy();
    mounted.delete(el);
  }
}

/**
 * Replace each ```mermaid fence in `contentEl` with its diagram.
 *
 * The diagram types are loaded before any fence is touched, so the source
 * stays on screen until its drawing can replace it in the same frame. A fence
 * Pele cannot draw shows its error in place (`mount` never throws), with the
 * source kept for the next attempt.
 */
export async function renderDiagrams(contentEl) {
  forgetDetached();
  const fences = [...contentEl.querySelectorAll("pre > code.language-mermaid")];
  if (fences.length === 0) return;

  const pele = await loadPele();
  if (!pele) return;

  const sources = fences.map((code) => code.textContent);
  await Promise.all(
    sources.map((source) => pele.load(pele.detectType(source)).catch(() => false)),
  );

  const options = mountOptions(contentEl);
  fences.forEach((code, i) => {
    // The document may have been replaced while the types were loading.
    if (!code.isConnected) return;
    const div = document.createElement("div");
    // MathJax must leave `$` in labels alone: they are SVG text.
    div.className = "diagram mathjax_ignore";
    div.dataset.diagramSource = sources[i];
    code.parentElement.replaceWith(div);
    try {
      const diagram = pele.mount(div, sources[i], options);
      mounted.set(div, { diagram, fontSize: options.fontSize });
    } catch (err) {
      console.warn("Diagram failed to render", err);
    }
  });
}

/**
 * Re-lay diagrams out after a settings change. Colors never need this; fonts
 * and the reading size do, and `refresh` is a no-op when neither moved.
 */
export function refreshDiagrams(contentEl) {
  forgetDetached();
  if (mounted.size === 0) return;
  const options = mountOptions(contentEl);
  for (const entry of mounted.values()) {
    if (entry.fontSize === options.fontSize) {
      entry.diagram.refresh();
    } else {
      entry.fontSize = options.fontSize;
      entry.diagram.update(entry.diagram.source, options);
    }
  }
}

/** Put the fences back, for when diagrams are switched off. */
export function restoreDiagramSources(contentEl) {
  for (const div of contentEl.querySelectorAll("div.diagram[data-diagram-source]")) {
    mounted.get(div)?.diagram.destroy();
    mounted.delete(div);
    const pre = document.createElement("pre");
    const code = document.createElement("code");
    code.className = "language-mermaid";
    code.textContent = div.dataset.diagramSource;
    pre.append(code);
    div.replaceWith(pre);
  }
}
