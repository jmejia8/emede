// Document statistics for the contents panel.
//
// Two different texts feed this module, on purpose:
//
//   * Prose stats (words, characters, sentences, read time) come from the
//     *rendered* DOM, so they describe what is actually on screen — no
//     markdown punctuation, no YAML front matter, no fenced code.
//   * The token estimate comes from the *raw* source, because that is the text
//     you would paste into an LLM; its syntax, delimiters and front matter all
//     cost tokens too.

/// Block elements whose text is prose. Everything outside these is either
/// structural or counted separately (see `ELEMENT_COUNTS`).
const PROSE_BLOCKS = "p, li, dt, dd, figcaption, blockquote, h1, h2, h3, h4, h5, h6";

/// Removed before prose is measured. `pre.plain-text` is deliberately spared:
/// emede wraps whole .txt documents in it, and that text really is the prose.
const NON_PROSE = "pre:not(.plain-text), table, .mermaid, mjx-container, .katex";

/// Words per minute used when no reading speed is configured. From Brysbaert
/// (2019), a meta-analysis of 190 studies: 238 wpm for silent non-fiction.
export const DEFAULT_READING_WPM = 238;

/// Abbreviations that end in a period without ending a sentence. Without these
/// a methods section reads as roughly twice as many sentences as it has.
const ABBREVIATIONS = new Set([
  "e.g", "i.e", "cf", "vs", "etc", "al", "ie", "eg",
  "fig", "figs", "eq", "eqs", "sec", "secs", "ch", "chap", "app",
  "approx", "resp", "est", "min", "max", "avg", "std", "var",
  "dr", "prof", "mr", "mrs", "ms", "st", "jr", "sr",
  "no", "vol", "pp", "ed", "eds", "ref", "refs", "inc", "ltd", "dept",
]);

/// Counts a run of text as words: whitespace-separated tokens holding at least
/// one letter or digit, so bare punctuation and list bullets do not inflate it.
function countWords(text) {
  const matches = text.match(/[^\s]*[\p{L}\p{N}][^\s]*/gu);
  return matches ? matches.length : 0;
}

/// Counts sentences in a single block of prose.
///
/// A sentence ends at `.`, `!`, `?` or `…` (plus any closing quote/bracket)
/// followed by whitespace or the end of the block. Requiring that trailing
/// whitespace is what keeps `3.14`, `v1.2` and URLs from splitting. Preceding
/// abbreviations and single-letter initials ("J. M. Smith") are skipped.
///
/// A block with words but no terminator — a heading, a list item, a table
/// caption — counts as one sentence, matching how word processors report it.
function countSentencesInBlock(text) {
  let count = 0;
  const terminator = /([.!?…])["'”’)\]]*(?=\s|$)/gu;

  for (const match of text.matchAll(terminator)) {
    if (match[1] !== ".") {
      count += 1;
      continue;
    }
    const preceding = text.slice(0, match.index).match(/([\p{L}\p{N}.]+)$/u);
    const word = preceding?.[1]?.toLowerCase() ?? "";
    // "J." is an initial and "e.g." an abbreviation — neither ends a sentence.
    // A lone digit is not an initial, so "see Sec. 4." still terminates.
    if (/^\p{L}$/u.test(word) || ABBREVIATIONS.has(word.replace(/\.+$/, ""))) continue;
    count += 1;
  }

  if (count === 0 && countWords(text) > 0) return 1;
  return count;
}

/// Math survives into the rendered DOM as literal `$…$` / `$$…$$` delimiters —
/// MathJax has not run yet when stats are computed. Strip those spans out of
/// the prose text and report how many there were.
function extractMath(text) {
  let equations = 0;
  const stripped = text.replace(/\$\$[\s\S]+?\$\$|\$[^$\n]+?\$/g, () => {
    equations += 1;
    return " ";
  });
  return { equations, stripped };
}

/// The text an element contributes on its own, excluding any block it merely
/// wraps. Without this a `blockquote` would double-count its paragraphs, and a
/// list item introducing a sub-list would lose its own line to the filter.
function ownText(el) {
  const copy = el.cloneNode(true);
  for (const nested of copy.querySelectorAll(PROSE_BLOCKS)) nested.remove();
  return copy.textContent;
}

/// Measure the rendered document. Call this before MathJax and Mermaid run,
/// while the DOM still holds the plain rendering.
function proseStats(contentEl) {
  const clone = contentEl.cloneNode(true);
  for (const node of clone.querySelectorAll(NON_PROSE)) node.remove();

  let blocks = [...clone.querySelectorAll(PROSE_BLOCKS)].map(ownText);
  // Raw-HTML documents may have no recognizable block at all; treat the
  // remaining text as a single block rather than reporting nothing.
  if (blocks.length === 0) blocks = [clone.textContent];

  let words = 0;
  let sentences = 0;
  let characters = 0;
  let charactersNoSpaces = 0;
  let equations = 0;
  let paragraphs = 0;

  for (const block of blocks) {
    const raw = block.replace(/\s+/g, " ").trim();
    if (!raw) continue;
    const math = extractMath(raw);
    equations += math.equations;
    const text = math.stripped.replace(/\s+/g, " ").trim();
    if (!text) continue;
    paragraphs += 1;
    words += countWords(text);
    sentences += countSentencesInBlock(text);
    characters += text.length;
    charactersNoSpaces += text.replace(/\s/g, "").length;
  }

  return { words, sentences, characters, charactersNoSpaces, equations, paragraphs };
}

/// Structural elements reported alongside the prose counts.
function elementStats(contentEl) {
  const codeBlocks = contentEl.querySelectorAll(
    "pre:not(.plain-text) > code:not(.language-mermaid)",
  ).length;
  const diagrams = contentEl.querySelectorAll(
    "pre > code.language-mermaid, .mermaid",
  ).length;
  return {
    codeBlocks,
    diagrams,
    tables: contentEl.querySelectorAll("table").length,
    figures: contentEl.querySelectorAll("img").length,
  };
}

// ---------------------------------------------------------------------------
// Token estimation
// ---------------------------------------------------------------------------

/// Characters per token for ordinary English prose — the widely published
/// rule of thumb (1 token ≈ 4 characters ≈ 0.75 words).
const CHARS_PER_TOKEN_PROSE = 4;
/// Code, TeX and structured syntax tokenize denser: punctuation and symbols
/// rarely merge into multi-character tokens.
const CHARS_PER_TOKEN_DENSE = 3;
/// CJK text is the other extreme — often one or two tokens per character.
const CHARS_PER_TOKEN_CJK = 1.5;

/// Source spans that tokenize at the denser rate: fenced and inline code, TeX
/// in either delimiter style, and YAML front matter.
const DENSE_SPANS =
  /^---\n[\s\S]*?\n---\n|```[\s\S]*?```|~~~[\s\S]*?~~~|`[^`\n]+`|\$\$[\s\S]+?\$\$|\$[^$\n]+?\$|\\\([\s\S]*?\\\)|\\\[[\s\S]*?\\\]/g;

const CJK = /[　-ヿ㐀-䶿一-鿿豈-﫿＀-￯가-힯]/g;

/// Estimate the LLM token count of a raw source document.
///
/// This is a heuristic, not a tokenizer: no vocabulary is consulted, so treat
/// the result as ±10-20%. It improves on a flat chars/4 by splitting the text
/// into prose, dense syntax (code/TeX/front matter) and CJK, each of which has
/// a well-documented and quite different character-per-token ratio.
export function estimateTokens(source) {
  if (!source) return 0;

  let dense = 0;
  const prose = source.replace(DENSE_SPANS, (span) => {
    dense += span.length;
    return " ";
  });

  const cjk = (prose.match(CJK) ?? []).length;
  const rest = Math.max(0, prose.length - cjk);

  return Math.round(
    dense / CHARS_PER_TOKEN_DENSE +
      cjk / CHARS_PER_TOKEN_CJK +
      rest / CHARS_PER_TOKEN_PROSE,
  );
}

/// Full statistics for a document: prose measured from `contentEl`, tokens
/// estimated from `source`, read time derived from `wpm`.
export function computeDocStats(contentEl, source, wpm = DEFAULT_READING_WPM) {
  const prose = proseStats(contentEl);
  const speed = Number(wpm) > 0 ? Number(wpm) : DEFAULT_READING_WPM;
  return {
    ...prose,
    ...elementStats(contentEl),
    tokens: estimateTokens(source),
    readSeconds: (prose.words / speed) * 60,
    wpm: speed,
  };
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

const groups = new Intl.NumberFormat();

export function formatCount(n) {
  return groups.format(n);
}

/// "< 1 min", "8 min", "1 h 14 min".
export function formatReadTime(seconds) {
  if (seconds <= 0) return "—";
  const minutes = Math.round(seconds / 60);
  if (minutes < 1) return "< 1 min";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

/// Tokens are an estimate, so they are shown rounded — a digit-exact figure
/// would imply a precision this heuristic does not have.
export function formatTokens(tokens) {
  if (tokens <= 0) return "0";
  if (tokens < 1000) return `≈ ${Math.round(tokens / 10) * 10}`;
  return `≈ ${(tokens / 1000).toFixed(1)}k`;
}

/// The rows the contents panel displays, as `{ label, value, title }` objects.
/// A `null` entry marks the rule between the prose counts and the structural
/// element counts. Element rows that would read zero are dropped, so a plain
/// prose document shows no empty tail.
export function statsRows(stats) {
  const rows = [
    {
      label: "Words",
      value: formatCount(stats.words),
      title: "Prose only — code blocks, tables and math excluded",
    },
    {
      label: "Characters",
      value: formatCount(stats.characters),
      title: `${formatCount(stats.charactersNoSpaces)} without spaces`,
    },
    { label: "Sentences", value: formatCount(stats.sentences) },
    {
      label: "Read time",
      value: formatReadTime(stats.readSeconds),
      title: `At ${stats.wpm} words per minute`,
    },
    {
      label: "Tokens",
      value: formatTokens(stats.tokens),
      title: `Estimated from the source (~${formatCount(stats.tokens)} tokens); a heuristic, not a tokenizer`,
    },
  ];

  const elements = [
    ["Code blocks", stats.codeBlocks],
    ["Diagrams", stats.diagrams],
    ["Equations", stats.equations],
    ["Tables", stats.tables],
    ["Figures", stats.figures],
  ].filter(([, count]) => count > 0);

  if (elements.length > 0) {
    rows.push(null);
    for (const [label, count] of elements) {
      rows.push({ label, value: formatCount(count) });
    }
  }

  return rows;
}
