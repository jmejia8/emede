/**
 * Syntax highlighting for fenced code blocks, by highlight.js.
 *
 * Only a block that names its language is highlighted. Auto-detection is never
 * used: on short snippets it guesses wrong often enough that a plain block is
 * the better default, and an unnamed fence is the author saying "just text".
 * A name highlight.js does not know also stays plain.
 *
 * Nothing is loaded until a document has such a block. Then the core and only
 * the grammars in use are imported from `vendor/highlight/` (copied there by
 * `npm postinstall`, so this works offline), along with any grammar those embed:
 * HTML highlights its `<script>` through the JavaScript grammar. Which file a
 * fence name maps to, and what each grammar embeds, comes from the `index.json`
 * written at the same time.
 *
 * Highlighting only wraps text in `<span class="hljs-…">`, so `textContent` is
 * unchanged: `stats.js` and `changes.js` (which treats `pre` as opaque) see the
 * same document either way, and the order relative to them does not matter.
 * Colors come from the code theme (`syntaxtheme.js`), seated by `syntax.css`.
 */

const VENDOR = "/vendor/highlight";

/// Fences that are owned by another renderer or that mean "no highlighting".
const SKIP = new Set(["mermaid", "math", "text", "txt", "plain", "plaintext"]);

let indexPromise = null;
let corePromise = null;
const grammarPromises = new Map();

function loadIndex() {
  indexPromise ??= fetch(`${VENDOR}/index.json`)
    .then((response) => {
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      return response.json();
    })
    .catch((err) => {
      console.warn("Syntax highlighting unavailable", err);
      indexPromise = null;
      return null;
    });
  return indexPromise;
}

function loadCore() {
  corePromise ??= import(`${VENDOR}/core.min.js`).then((module) => module.default);
  return corePromise;
}

/** Import and register one grammar. Resolves to false when it failed to load. */
function loadGrammar(hljs, name) {
  if (!grammarPromises.has(name)) {
    grammarPromises.set(
      name,
      import(`${VENDOR}/languages/${name}.min.js`).then(
        (module) => {
          hljs.registerLanguage(name, module.default);
          return true;
        },
        (err) => {
          console.warn(`Failed to load the ${name} grammar`, err);
          return false;
        },
      ),
    );
  }
  return grammarPromises.get(name);
}

/** The fence name comrak put on a `<code>`, as `language-<name>`. */
function fenceName(code) {
  for (const cls of code.classList) {
    if (cls.startsWith("language-")) return cls.slice("language-".length).toLowerCase();
  }
  return null;
}

/** `names` plus everything they embed, transitively. */
function withEmbedded(names, requires) {
  const all = new Set();
  const visit = (name) => {
    if (all.has(name)) return;
    all.add(name);
    for (const dep of requires[name] ?? []) visit(dep);
  };
  names.forEach(visit);
  return all;
}

/**
 * Highlight every fenced block in `container` that names a known language.
 *
 * Safe to call more than once on the same document: highlighted blocks carry
 * the `hljs` class and are skipped. Never rejects; on any failure the blocks
 * simply stay plain.
 */
export async function highlightCode(container) {
  const blocks = [];
  for (const code of container.querySelectorAll("pre > code[class*='language-']")) {
    const fence = fenceName(code);
    if (fence && !SKIP.has(fence) && !code.classList.contains("hljs")) {
      blocks.push({ code, fence });
    }
  }
  if (!blocks.length) return;

  const index = await loadIndex();
  if (!index) return;

  const targets = blocks
    .map((block) => ({ ...block, language: index.aliases[block.fence] }))
    .filter((block) => block.language);
  if (!targets.length) return;

  try {
    const hljs = await loadCore();
    const needed = withEmbedded(new Set(targets.map((t) => t.language)), index.requires);
    await Promise.all([...needed].map((name) => loadGrammar(hljs, name)));

    for (const { code, language } of targets) {
      // A re-render may have replaced the document while grammars loaded.
      if (!code.isConnected || !hljs.getLanguage(language)) continue;
      const { value } = hljs.highlight(code.textContent, { language, ignoreIllegals: true });
      code.innerHTML = value;
      code.classList.add("hljs");
    }
  } catch (err) {
    console.warn("Syntax highlighting failed", err);
  }
}
