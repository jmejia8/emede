//! Change highlighting: what in this document is new since the baseline.
//!
//! The awkward part of the problem is that the diff is over *markdown source*
//! while the reader looks at *rendered HTML*, and the two do not share
//! coordinates — `**bold**` is eight source bytes and four rendered characters,
//! `[label](url)` collapses to its label entirely. Mapping source offsets onto
//! the DOM produces highlights that drift off the words they belong to.
//!
//! So nothing here works in source offsets. Both versions are parsed, each is
//! reduced to a sequence of *block* plain-texts that mirror what the browser's
//! `textContent` will be, blocks are paired against blocks, and each pair is
//! diffed word by word. Every coordinate that reaches the frontend is a
//! **whitespace-token index within a block**.
//!
//! That choice is what makes the feature safe rather than merely clever. Any
//! disagreement between this module's idea of a block's text and the DOM's that
//! is purely about whitespace — whether comrak puts a newline between table
//! cells, whether a soft break renders as a space — becomes unobservable. Only
//! a disagreement that adds or removes a whole token can misalign a span, and
//! [`BlockChange::expect`] catches exactly that, at which point the block
//! degrades to a coarse whole-block mark instead of a wrong inline one.

use comrak::nodes::{AstNode, NodeValue};
use comrak::{parse_document, Arena};
use serde::Serialize;
use similar::{capture_diff_slices, Algorithm, DiffOp};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// Documents above this size skip diffing entirely. Two full parses plus a
/// word diff on every debounced save is fine for a research note and not for a
/// megabyte of generated log.
const MAX_DIFF_BYTES: usize = 1024 * 1024;

/// Below this fraction of shared tokens a paired block is reported as one
/// wholly-changed block rather than as spans: a rewritten paragraph diffs into
/// confetti, which reads worse than a single bar in the margin.
const WHOLE_BLOCK_RATIO: f32 = 0.25;

// ── Wire format ────────────────────────────────────────────────────────────────

/// One rendered block that differs from the baseline.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BlockChange {
    /// Element name the block rendered as: `p`, `li`, `h2`, `td`, `th`, `pre`.
    pub tag: String,
    /// comrak's `data-sourcepos` value, e.g. `12:1-12:40`. Paired with `tag`
    /// this identifies the element — sourcepos alone does not, since a
    /// single-item list gives `<ul>` and `<li>` the same value.
    pub sourcepos: String,
    /// `tokens.join(" ")`. The frontend tokenizes the element's flattened text
    /// the same way and refuses to place spans unless the two match exactly.
    pub expect: String,
    /// The whole block was added or rewritten; mark it at block level and
    /// ignore the word-level spans.
    pub whole: bool,
    /// The baseline text this block replaced, or the text of a block deleted
    /// just before it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    pub spans: Vec<ChangeSpan>,
}

/// A run of tokens within a block, in the block's *new* token space.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChangeSpan {
    /// `"added"`, `"changed"` or `"deleted"`.
    pub kind: &'static str,
    /// First token index covered.
    pub start: usize,
    /// Number of tokens covered; always 0 for `"deleted"`, which is a caret
    /// between tokens rather than a range.
    pub len: usize,
    /// The baseline text this span replaced or removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
}

// ── Blocks ─────────────────────────────────────────────────────────────────────

/// How a block is addressed in the DOM: `(tag, sourcepos)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BlockKey {
    pub tag: String,
    pub sourcepos: String,
}

/// One text-bearing block of a parsed document.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Block {
    /// `None` for blocks that cannot be addressed in the DOM (raw HTML blocks,
    /// anything comrak gave no sourcepos). They still take part in pairing, so
    /// the block sequences stay aligned, but never produce output.
    pub key: Option<BlockKey>,
    /// Plain text approximating the element's `textContent`.
    pub text: String,
    /// `text` split on whitespace — the coordinate space for every span.
    pub tokens: Vec<String>,
    /// Whether word-level spans may be placed inside. False for code blocks,
    /// whose text the frontend deliberately does not walk.
    pub markable: bool,
}

impl Block {
    fn new(key: Option<BlockKey>, text: String, markable: bool) -> Self {
        let tokens = tokenize(&text);
        Self {
            key,
            text,
            tokens,
            markable,
        }
    }

    fn expect(&self) -> String {
        self.tokens.join(" ")
    }

    /// The text shown in the "what was here before" popup.
    fn display(&self) -> String {
        let trimmed = self.text.trim();
        if trimmed.is_empty() {
            String::new()
        } else {
            trimmed.to_string()
        }
    }
}

/// True for the characters JavaScript's `\s` matches.
///
/// Deliberately not `char::is_whitespace`: Rust follows the Unicode
/// `White_Space` property, JS does not (it excludes U+0085 and includes
/// U+FEFF). The frontend tokenizes with `/\S+/gu`, and the two sides have to
/// agree on token boundaries or every block would degrade.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{b}'
            | '\u{c}'
            | '\r'
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(is_js_space)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Parse `content` and reduce it to the sequence of blocks a diff runs over.
///
/// `content` is raw document source; the same preprocessing the renderer
/// applies runs here, so sourcepos values line up with the emitted HTML.
pub(crate) fn document_blocks(content: &str, source_path: &Path) -> Vec<Block> {
    let preprocessed = crate::markdown::preprocess_all(content);
    let options = crate::markdown::comrak_options_ext(source_path, true);
    let arena = Arena::new();
    let root = parse_document(&arena, &preprocessed, &options);

    let mut blocks = Vec::new();
    collect_blocks(root, &mut blocks);
    blocks
}

fn sourcepos_key(node: &AstNode<'_>) -> Option<String> {
    let ast = node.data.borrow();
    // comrak omits the attribute entirely when the start line is 0, so such a
    // node has nothing to key on.
    if ast.sourcepos.start.line == 0 {
        return None;
    }
    Some(ast.sourcepos.to_string())
}

fn key_for(node: &AstNode<'_>, tag: &str) -> Option<BlockKey> {
    Some(BlockKey {
        tag: tag.to_string(),
        sourcepos: sourcepos_key(node)?,
    })
}

/// comrak's exact predicate for "this paragraph renders without a `<p>`".
///
/// Copied from `render_paragraph` in comrak 0.52's `src/html.rs`. It is the
/// reason list items are keyed on `<li>`: in a tight list — the most commonly
/// edited kind of content there is — the paragraph has no element of its own
/// and therefore no sourcepos to address. [`tripwire`] tests below fail loudly
/// if a comrak upgrade changes this.
fn paragraph_is_tight(node: &AstNode<'_>) -> bool {
    let grandparent_tight = node
        .parent()
        .and_then(|n| n.parent())
        .is_some_and(|n| match n.data.borrow().value {
            NodeValue::List(ref nl) => nl.tight,
            NodeValue::DescriptionItem(ref ndi) => ndi.tight,
            _ => false,
        });

    grandparent_tight
        || node
            .parent()
            .is_some_and(|n| matches!(n.data.borrow().value, NodeValue::DescriptionTerm))
}

/// Nearest enclosing list item, whose `<li>` a tight paragraph is keyed on.
/// Task items replace `Item` in the AST but still render as `<li>`.
fn item_ancestor<'a>(node: &'a AstNode<'a>) -> Option<&'a AstNode<'a>> {
    let mut current = node.parent();
    while let Some(n) = current {
        if matches!(
            n.data.borrow().value,
            NodeValue::Item(_) | NodeValue::TaskItem(_)
        ) {
            return Some(n);
        }
        current = n.parent();
    }
    None
}

/// The `lang` comrak derives from a fence's info string.
fn code_block_lang(info: &str) -> &str {
    info.split_whitespace().next().unwrap_or("")
}

/// What [`collect_blocks`] decided about one node, computed while the node's
/// `RefCell` is borrowed and acted on after that borrow is released.
enum Visit {
    /// Emit this block and stop descending.
    Emit(Block),
    /// Contributes nothing and holds nothing that does.
    Skip,
    /// A container: its children are the blocks.
    Recurse,
}

fn collect_blocks<'a>(node: &'a AstNode<'a>, out: &mut Vec<Block>) {
    // Emitting a block means "stop descending": a block's text is its subtree's
    // text *minus* the subtrees of any nested block. `<li>` holds keys and can
    // contain a nested `<ul>`, so this exclusion is load-bearing, and the
    // frontend mirrors it by rejecting text nodes it does not own.
    let visit = match &node.data.borrow().value {
        NodeValue::Paragraph => {
            let key = if paragraph_is_tight(node) {
                item_ancestor(node).and_then(|item| key_for(item, "li"))
            } else {
                key_for(node, "p")
            };
            Visit::Emit(Block::new(key, inline_text(node), true))
        }
        NodeValue::Heading(nh) => {
            let tag = format!("h{}", nh.level);
            Visit::Emit(Block::new(key_for(node, &tag), inline_text(node), true))
        }
        NodeValue::TableCell => {
            let in_header = node
                .parent()
                .is_some_and(|row| matches!(row.data.borrow().value, NodeValue::TableRow(true)));
            let tag = if in_header { "th" } else { "td" };
            Visit::Emit(Block::new(key_for(node, tag), inline_text(node), true))
        }
        NodeValue::CodeBlock(ncb) => {
            // Mermaid fences are replaced wholesale by the frontend renderer,
            // so any mark placed in one is destroyed. Skipping them from both
            // sides of the diff keeps the sequences aligned.
            if code_block_lang(&ncb.info) == "mermaid" {
                Visit::Skip
            } else {
                // Block-level marking only, mirroring the `pre` exclusion that
                // find-in-page and the statistics panel both already make.
                Visit::Emit(Block::new(key_for(node, "pre"), ncb.literal.clone(), false))
            }
        }
        // Unaddressable, so never marked — but it holds text a reader can see,
        // and dropping it would let a deleted HTML block slide the two block
        // sequences out of step. Pairing only.
        NodeValue::HtmlBlock(nhb) => Visit::Emit(Block::new(None, strip_tags(&nhb.literal), false)),
        // Zero text; an entry here would just be an empty slot for the block
        // diff to trip over.
        NodeValue::ThematicBreak => Visit::Skip,
        // Containers. `Item`/`TaskItem` land here too, even though they may
        // hold a tight paragraph's key — that is looked up upward, not down.
        _ => Visit::Recurse,
    };

    match visit {
        Visit::Emit(block) => out.push(block),
        Visit::Skip => {}
        Visit::Recurse => {
            for child in node.children() {
                collect_blocks(child, out);
            }
        }
    }
}

/// Build the plain text a block contributes, as the browser will see it.
fn inline_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut out = String::new();
    inline_text_into(node, &mut out);
    out
}

fn inline_text_into<'a>(node: &'a AstNode<'a>, out: &mut String) {
    for child in node.children() {
        // `recurse` rather than calling through under the borrow: emphasis and
        // links descend into the same `RefCell` graph.
        let mut recurse = false;
        match &child.data.borrow().value {
            NodeValue::Text(t) => out.push_str(t),
            // `<code>literal</code>` — the literal is the text.
            NodeValue::Code(c) => out.push_str(&c.literal),
            NodeValue::Math(nm) => {
                // `MathJaxFormatter` writes HTML-*escaped* `$…$` delimiters;
                // the browser decodes those back, so `&amp;` in the markup is
                // `&` in `textContent`. Pushing the literal unescaped is what
                // matches the DOM — do not call `html_escape` here.
                if nm.display_math {
                    out.push_str("$$\n");
                    out.push_str(&nm.literal);
                    out.push_str("\n$$");
                } else {
                    out.push('$');
                    out.push_str(&nm.literal);
                    out.push('$');
                }
            }
            NodeValue::SoftBreak | NodeValue::LineBreak => out.push('\n'),
            // `alt` is an attribute, so an image contributes no text at all —
            // and its children hold exactly that alt text. Do not recurse.
            NodeValue::Image(_) => {}
            // The tag itself is invisible; any text around it arrives as
            // sibling `Text` nodes.
            NodeValue::HtmlInline(_) | NodeValue::Raw(_) => {}
            // Emphasis, links, strikethrough and friends contribute their
            // label and no syntax.
            _ => recurse = true,
        }
        if recurse {
            inline_text_into(child, out);
        }
    }
}

/// Crude tag stripper for raw HTML blocks. Never used for marking — only to
/// give a raw block *some* text so the block sequence keeps its shape.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

// ── Diff ───────────────────────────────────────────────────────────────────────

/// Diff `baseline` against `current`, both raw document source.
pub(crate) fn diff_documents(baseline: &str, current: &str, source_path: &Path) -> Vec<BlockChange> {
    if baseline.len() > MAX_DIFF_BYTES || current.len() > MAX_DIFF_BYTES {
        return Vec::new();
    }

    let old_blocks = document_blocks(baseline, source_path);
    let new_blocks = document_blocks(current, source_path);
    pair_blocks(&old_blocks, &new_blocks)
}

/// Accumulates the changes for one new-document block until every op that
/// touches it has been seen.
#[derive(Default)]
struct Pending {
    whole: bool,
    previous: Option<String>,
    spans: Vec<ChangeSpan>,
}

fn pair_blocks(old_blocks: &[Block], new_blocks: &[Block]) -> Vec<BlockChange> {
    let old_texts: Vec<&str> = old_blocks.iter().map(|b| b.text.as_str()).collect();
    let new_texts: Vec<&str> = new_blocks.iter().map(|b| b.text.as_str()).collect();
    let ops = capture_diff_slices(Algorithm::Myers, &old_texts, &new_texts);

    let mut pending: HashMap<usize, Pending> = HashMap::new();
    // Blocks removed with nothing to replace them. A deletion has no place of
    // its own in the new document, so it is anchored as a caret on whatever
    // block now stands where it used to be.
    let mut orphaned: Vec<String> = Vec::new();

    for op in &ops {
        match *op {
            DiffOp::Equal { new_index, .. } => {
                flush_orphans(&mut orphaned, &mut pending, new_index);
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                flush_orphans(&mut orphaned, &mut pending, new_index);
                for i in new_index..new_index + new_len {
                    pending.entry(i).or_default().whole = true;
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for block in &old_blocks[old_index..old_index + old_len] {
                    push_orphan(&mut orphaned, block);
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                flush_orphans(&mut orphaned, &mut pending, new_index);
                let paired = old_len.min(new_len);
                for offset in 0..paired {
                    let old = &old_blocks[old_index + offset];
                    let new = &new_blocks[new_index + offset];
                    let entry = pending.entry(new_index + offset).or_default();
                    apply_block_diff(old, new, entry);
                }
                // More blocks than before: the surplus is new outright.
                for i in new_index + paired..new_index + new_len {
                    pending.entry(i).or_default().whole = true;
                }
                // Fewer blocks than before: the surplus is gone, and becomes a
                // caret on whatever follows.
                for block in &old_blocks[old_index + paired..old_index + old_len] {
                    push_orphan(&mut orphaned, block);
                }
            }
        }
    }

    // Anything deleted off the end of the document lands as a caret past the
    // last token of the last block.
    if !orphaned.is_empty() {
        if let Some(last) = new_blocks.len().checked_sub(1) {
            let at = new_blocks[last].tokens.len();
            let entry = pending.entry(last).or_default();
            for text in orphaned.drain(..) {
                entry.spans.push(deleted_span(at, text));
            }
        }
    }

    // In document order, so the frontend's "could not locate N blocks" tally and
    // any debugging read the same way as the page does.
    let mut changes: Vec<BlockChange> = Vec::new();
    for (index, block) in new_blocks.iter().enumerate() {
        let Some(entry) = pending.remove(&index) else {
            continue;
        };
        if !entry.whole && entry.spans.is_empty() {
            continue;
        }
        // Unaddressable blocks (raw HTML) took part in pairing but cannot be
        // marked. Their carets are dropped with them.
        let Some(key) = block.key.clone() else {
            continue;
        };
        changes.push(BlockChange {
            tag: key.tag,
            sourcepos: key.sourcepos,
            expect: block.expect(),
            whole: entry.whole,
            previous: entry.previous,
            spans: entry.spans,
        });
    }
    changes
}

fn push_orphan(orphaned: &mut Vec<String>, block: &Block) {
    let text = block.display();
    if !text.is_empty() {
        orphaned.push(text);
    }
}

/// Attach every pending deletion as a caret at the start of block `index`.
fn flush_orphans(orphaned: &mut Vec<String>, pending: &mut HashMap<usize, Pending>, index: usize) {
    if orphaned.is_empty() {
        return;
    }
    let entry = pending.entry(index).or_default();
    for text in orphaned.drain(..) {
        entry.spans.push(deleted_span(0, text));
    }
}

fn deleted_span(at: usize, previous: String) -> ChangeSpan {
    ChangeSpan {
        kind: "deleted",
        start: at,
        len: 0,
        previous: Some(previous),
    }
}

/// Diff one paired block, deciding between word-level spans and a whole-block
/// mark.
fn apply_block_diff(old: &Block, new: &Block, entry: &mut Pending) {
    if old.tokens == new.tokens {
        return;
    }

    // Code blocks are marked at block level only: the frontend does not walk
    // text inside `<pre>`, so there is nowhere to put a span.
    if !new.markable || old.tokens.is_empty() || new.tokens.is_empty() {
        entry.whole = true;
        entry.previous = Some(old.display());
        return;
    }

    let ops = capture_diff_slices(Algorithm::Myers, &old.tokens, &new.tokens);

    let shared: usize = ops
        .iter()
        .map(|op| match *op {
            DiffOp::Equal { len, .. } => len,
            _ => 0,
        })
        .sum();
    let ratio = (2.0 * shared as f32) / (old.tokens.len() + new.tokens.len()) as f32;
    if ratio < WHOLE_BLOCK_RATIO {
        entry.whole = true;
        entry.previous = Some(old.display());
        return;
    }

    for op in &ops {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Insert {
                new_index, new_len, ..
            } => entry.spans.push(ChangeSpan {
                kind: "added",
                start: new_index,
                len: new_len,
                previous: None,
            }),
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => entry.spans.push(deleted_span(
                new_index,
                old.tokens[old_index..old_index + old_len].join(" "),
            )),
            // A deletion butted up against an insertion is a reword, which is
            // exactly what `Replace` already groups for us.
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => entry.spans.push(ChangeSpan {
                kind: "changed",
                start: new_index,
                len: new_len,
                previous: Some(old.tokens[old_index..old_index + old_len].join(" ")),
            }),
        }
    }
}

// ── Baselines ──────────────────────────────────────────────────────────────────

/// Per-document baseline state, kept for the process lifetime.
///
/// Managed alongside `WatcherState` rather than hung off `ActiveWatch`, which
/// `unwatch_document` drops and which never exists at all on the headless and
/// `--print` paths.
#[derive(Default)]
pub struct BaselineState(pub Mutex<HashMap<String, DocBaseline>>);

/// What a single open document is diffed against.
#[derive(Default)]
pub struct DocBaseline {
    /// Content as it was the first time emede rendered this path. Used only
    /// when the file has no committed version to compare against, so
    /// highlights accumulate over the session and reset on reopen.
    snapshot: Option<String>,
    git: Option<crate::baseline::GitCache>,
}

/// The key a document's baseline is stored under: its canonical path, so the
/// same file opened by a relative path or through a symlink shares one entry.
fn baseline_key(resolved: &Path) -> String {
    resolved
        .canonicalize()
        .unwrap_or_else(|_| resolved.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Forget a document's baseline, so reopening it starts from a clean page.
pub fn forget_baseline(state: &BaselineState, path: &Path) {
    if let Ok(mut guard) = state.0.lock() {
        guard.remove(&baseline_key(path));
    }
}

/// Resolve the text this document should be diffed against.
///
/// A tracked file is compared against `HEAD`, so highlights are exactly the
/// uncommitted changes and committing clears them. Anything else — untracked,
/// outside a repo, or in a repo with no commits yet — falls back to the
/// in-memory snapshot taken when emede first rendered it.
fn resolve_baseline(state: &BaselineState, doc: &crate::markdown::LocalDocument) -> Option<String> {
    let mut guard = state.0.lock().ok()?;
    let entry = guard.entry(baseline_key(&doc.resolved)).or_default();

    if let Some(blob) = crate::baseline::head_blob(&doc.resolved, &mut entry.git) {
        return Some(blob);
    }

    Some(
        entry
            .snapshot
            .get_or_insert_with(|| doc.raw.clone())
            .clone(),
    )
}

// ── Render entry point ─────────────────────────────────────────────────────────

/// Render a local document, filling in [`crate::markdown::RenderResult::changes`]
/// when change highlighting applies.
///
/// This is the single place changes are computed. Every other render path —
/// `--share`, `--export`, URL fetches — keeps calling
/// `render_markdown_inner` and gets `changes: None` for free.
pub fn render_markdown_tracked(
    path: &str,
    state: &BaselineState,
    app: &tauri::AppHandle,
) -> Result<crate::markdown::RenderResult, String> {
    use tauri::Manager as _;

    // `--print` runs the full windowed app, so it reaches this command like any
    // other render; without this check the exported PDF would carry highlights.
    let print_mode = app
        .state::<crate::PrintTarget>()
        .0
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);

    let enabled = !print_mode
        && !crate::markdown::is_remote_url(path)
        && crate::settings::load_settings().change_highlighting;

    if !enabled {
        // Byte-for-byte what emede rendered before this feature existed: no
        // second parse, no diff, no git subprocess, no `data-sourcepos`.
        return crate::markdown::render_markdown_inner(path);
    }

    let doc = crate::markdown::read_local_document(path)?;
    if !doc.is_markdown() {
        return crate::markdown::render_local_document(&doc, false);
    }

    let baseline = resolve_baseline(state, &doc);
    let mut result = crate::markdown::render_local_document(&doc, true)?;
    result.changes = Some(changes_against(&baseline, &doc));
    Ok(result)
}

fn changes_against(
    baseline: &Option<String>,
    doc: &crate::markdown::LocalDocument,
) -> Vec<BlockChange> {
    match baseline {
        Some(baseline) if *baseline != doc.raw => {
            diff_documents(baseline, &doc.raw, &doc.resolved)
        }
        _ => Vec::new(),
    }
}

/// A recomputed diff for a document that is already on screen.
#[derive(Serialize)]
pub struct ChangesUpdate {
    /// The source the diff was computed against. The frontend applies the
    /// result only if this matches the source it rendered from — the span keys
    /// address *that* render's `data-sourcepos` values, so applying them to a
    /// document rendered from different bytes would put marks in the wrong
    /// place.
    pub source: String,
    pub changes: Vec<BlockChange>,
}

/// Recompute a document's changes without re-rendering it.
///
/// Committing in a terminal moves the baseline without touching the file, so
/// the filesystem watcher never fires and the highlights go stale. Re-rendering
/// on window focus would fix that, but at the cost of throwing away the typeset
/// math and rendered diagrams on every alt-tab. This does the diff alone and
/// lets the frontend repaint just the marks.
#[tauri::command]
pub fn get_document_changes(
    path: String,
    baselines: tauri::State<'_, BaselineState>,
    app: tauri::AppHandle,
) -> Result<Option<ChangesUpdate>, String> {
    use tauri::Manager as _;

    let print_mode = app
        .state::<crate::PrintTarget>()
        .0
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);

    if print_mode
        || crate::markdown::is_remote_url(&path)
        || !crate::settings::load_settings().change_highlighting
    {
        return Ok(None);
    }

    let doc = crate::markdown::read_local_document(&path)?;
    if !doc.is_markdown() {
        return Ok(None);
    }

    let baseline = resolve_baseline(&baselines, &doc);
    Ok(Some(ChangesUpdate {
        changes: changes_against(&baseline, &doc),
        source: doc.raw,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn doc_path() -> PathBuf {
        PathBuf::from("/tmp/emede-test/note.md")
    }

    fn blocks(src: &str) -> Vec<Block> {
        document_blocks(src, &doc_path())
    }

    fn keys(src: &str) -> Vec<(String, String)> {
        blocks(src)
            .iter()
            .filter_map(|b| b.key.as_ref())
            .map(|k| (k.tag.clone(), k.sourcepos.clone()))
            .collect()
    }

    fn texts(src: &str) -> Vec<String> {
        blocks(src).iter().map(|b| b.text.clone()).collect()
    }

    fn render(src: &str) -> String {
        let preprocessed = crate::markdown::preprocess_all(src);
        let options = crate::markdown::comrak_options_ext(&doc_path(), true);
        let arena = Arena::new();
        let root = parse_document(&arena, &preprocessed, &options);
        let mut html = String::new();
        comrak::format_html(root, &options, &mut html).expect("render html");
        html
    }

    fn changes(old: &str, new: &str) -> Vec<BlockChange> {
        diff_documents(old, new, &doc_path())
    }

    // ── Block extraction ───────────────────────────────────────────────────────

    #[test]
    fn paragraph_text_drops_markup_and_keeps_labels() {
        assert_eq!(
            texts("A **bold** word, a [label](http://example.com) and `code()`.\n"),
            vec!["A bold word, a label and code()."]
        );
    }

    #[test]
    fn image_contributes_no_text() {
        // `alt` is an attribute, so it never appears in `textContent`.
        assert_eq!(texts("Before ![alt text](x.png) after.\n"), vec!["Before  after."]);
    }

    #[test]
    fn heading_is_keyed_by_level() {
        assert_eq!(keys("## Results\n"), vec![("h2".into(), "1:1-1:10".into())]);
    }

    #[test]
    fn tight_list_paragraphs_are_keyed_on_the_list_item() {
        // comrak emits no `<p>` inside a tight list, so `<li>` is the only
        // element there is to address.
        let keys = keys("- alpha\n- beta\n");
        assert_eq!(
            keys,
            vec![
                ("li".into(), "1:1-1:7".into()),
                ("li".into(), "2:1-2:6".into())
            ]
        );
    }

    #[test]
    fn loose_list_paragraphs_are_keyed_on_the_paragraph() {
        let keys = keys("- alpha\n\n- beta\n");
        assert!(keys.iter().all(|(tag, _)| tag == "p"), "{keys:?}");
    }

    #[test]
    fn nested_list_text_is_not_folded_into_the_parent_item() {
        // The outer `<li>`'s text must stop where the nested list begins, or
        // the frontend's ownership test and this one would disagree.
        assert_eq!(texts("- outer\n  - inner\n"), vec!["outer", "inner"]);
    }

    #[test]
    fn task_items_are_keyed_on_the_list_item() {
        let keys = keys("- [x] done\n- [ ] pending\n");
        assert!(keys.iter().all(|(tag, _)| tag == "li"), "{keys:?}");
        assert_eq!(texts("- [x] done\n"), vec!["done"]);
    }

    #[test]
    fn table_cells_are_individually_keyed() {
        let src = "| a | b |\n|---|---|\n| 1 | 2 |\n";
        let tags: Vec<String> = blocks(src)
            .iter()
            .filter_map(|b| b.key.as_ref())
            .map(|k| k.tag.clone())
            .collect();
        assert_eq!(tags, vec!["th", "th", "td", "td"]);
        assert_eq!(texts(src), vec!["a", "b", "1", "2"]);
    }

    #[test]
    fn inline_math_keeps_its_delimiters() {
        // `MathJaxFormatter` re-emits math as `$…$` for MathJax to find, so the
        // delimiters really are in the DOM text.
        assert_eq!(texts("Let $x_1 < y$ hold.\n"), vec!["Let $x_1 < y$ hold."]);
    }

    #[test]
    fn code_blocks_are_keyed_on_pre_and_not_markable() {
        let bs = blocks("```rust\nfn main() {}\n```\n");
        assert_eq!(bs.len(), 1);
        assert_eq!(bs[0].key.as_ref().unwrap().tag, "pre");
        assert!(!bs[0].markable);
    }

    #[test]
    fn mermaid_fences_are_skipped_entirely() {
        // The frontend replaces the whole `<pre>` with a rendered diagram, so
        // any mark inside it would be destroyed.
        assert!(blocks("```mermaid\ngraph TD;\nA-->B;\n```\n").is_empty());
    }

    #[test]
    fn front_matter_becomes_a_keyed_code_block_and_shifts_line_numbers() {
        let src = "---\ntitle: Note\n---\n\nBody text.\n";
        let bs = blocks(src);
        assert_eq!(bs[0].key.as_ref().unwrap().tag, "pre");
        // The preamble is rewritten as a fence, so the body's sourcepos is
        // relative to the *preprocessed* document, not the file on disk. The
        // frontend keys against the same preprocessed render, so this is
        // consistent rather than merely tolerated.
        let body = bs.iter().find(|b| b.text.starts_with("Body")).unwrap();
        assert_eq!(body.key.as_ref().unwrap().tag, "p");
    }

    #[test]
    fn thematic_breaks_produce_no_block() {
        assert_eq!(texts("a\n\n---\n\nb\n"), vec!["a", "b"]);
    }

    #[test]
    fn raw_html_blocks_pair_but_carry_no_key() {
        let bs = blocks("<div class=\"x\">hi</div>\n");
        assert_eq!(bs.len(), 1);
        assert!(bs[0].key.is_none());
    }

    // ── comrak tripwires ───────────────────────────────────────────────────────
    //
    // The keying scheme rests on two structural properties of comrak's
    // renderer. If a version bump changes either, these fail loudly here rather
    // than silently producing no highlights in the app.

    #[test]
    fn tripwire_tight_list_paragraphs_render_without_a_p_element() {
        let html = render("- alpha\n- beta\n");
        assert!(html.contains("<li data-sourcepos"), "{html}");
        assert!(!html.contains("<p data-sourcepos"), "{html}");
    }

    #[test]
    fn tripwire_every_emitted_key_matches_exactly_one_element() {
        let src = "\
# Title

Some **prose** with a [link](http://example.com).

- alpha
- beta with `code`

| a | b |
|---|---|
| 1 | 2 |

> quoted paragraph

```rust
fn main() {}
```
";
        let html = render(src);
        for block in blocks(src) {
            let Some(key) = block.key else { continue };
            let needle = format!("<{} data-sourcepos=\"{}\"", key.tag, key.sourcepos);
            let hits = html.matches(&needle).count();
            assert_eq!(hits, 1, "key {key:?} matched {hits} elements in:\n{html}");
        }
    }

    // ── Diff ───────────────────────────────────────────────────────────────────

    #[test]
    fn identical_documents_produce_no_changes() {
        let src = "# Title\n\nSome prose.\n";
        assert!(changes(src, src).is_empty());
    }

    #[test]
    fn rewrapping_a_paragraph_is_not_a_change() {
        // The whole point of tokenizing: where the line breaks fall is not
        // something a reader can see.
        let old = "One two three four five six.\n";
        let new = "One two three\nfour five six.\n";
        assert!(changes(old, new).is_empty());
    }

    #[test]
    fn pure_insertion_is_an_added_span() {
        let out = changes("alpha beta\n", "alpha inserted beta\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag, "p");
        assert!(!out[0].whole);
        assert_eq!(
            out[0].spans,
            vec![ChangeSpan {
                kind: "added",
                start: 1,
                len: 1,
                previous: None,
            }]
        );
        assert_eq!(out[0].expect, "alpha inserted beta");
    }

    #[test]
    fn pure_deletion_is_a_zero_width_caret() {
        let out = changes("alpha removed beta\n", "alpha beta\n");
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].spans,
            vec![ChangeSpan {
                kind: "deleted",
                start: 1,
                len: 0,
                previous: Some("removed".into()),
            }]
        );
    }

    #[test]
    fn a_reword_is_classified_as_changed_and_carries_the_old_words() {
        let out = changes(
            "The algorithm converges slowly on this instance.\n",
            "The algorithm converges quickly on this instance.\n",
        );
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].spans,
            vec![ChangeSpan {
                kind: "changed",
                start: 3,
                len: 1,
                previous: Some("slowly".into()),
            }]
        );
    }

    #[test]
    fn a_wholly_rewritten_paragraph_degrades_to_one_block_mark() {
        let out = changes(
            "Alpha beta gamma delta epsilon.\n",
            "Completely different wording here entirely.\n",
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].whole);
        assert!(out[0].spans.is_empty());
        assert_eq!(out[0].previous.as_deref(), Some("Alpha beta gamma delta epsilon."));
    }

    #[test]
    fn a_block_inserted_mid_document_marks_only_that_block() {
        let out = changes(
            "First para.\n\nThird para.\n",
            "First para.\n\nSecond para.\n\nThird para.\n",
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].whole);
        assert_eq!(out[0].expect, "Second para.");
        assert!(out[0].previous.is_none());
    }

    #[test]
    fn a_deleted_block_anchors_a_caret_on_the_block_that_follows() {
        let out = changes(
            "First para.\n\nSecond para.\n\nThird para.\n",
            "First para.\n\nThird para.\n",
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].expect, "Third para.");
        assert_eq!(
            out[0].spans,
            vec![ChangeSpan {
                kind: "deleted",
                start: 0,
                len: 0,
                previous: Some("Second para.".into()),
            }]
        );
    }

    #[test]
    fn a_block_deleted_off_the_end_anchors_past_the_last_token() {
        let out = changes("Only para.\n\nTrailing para.\n", "Only para.\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].expect, "Only para.");
        assert_eq!(out[0].spans[0].kind, "deleted");
        assert_eq!(out[0].spans[0].start, 2);
    }

    #[test]
    fn an_edited_list_item_is_reported_against_its_li() {
        let out = changes("- alpha one\n- beta\n", "- alpha two\n- beta\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag, "li");
        assert_eq!(out[0].expect, "alpha two");
    }

    #[test]
    fn an_edited_table_cell_is_reported_against_its_td() {
        let out = changes(
            "| a | b |\n|---|---|\n| 1 | 2 |\n",
            "| a | b |\n|---|---|\n| 1 | 9 |\n",
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag, "td");
        assert!(out[0].whole, "a single-token cell is replaced wholesale");
    }

    #[test]
    fn an_edited_code_block_is_marked_at_block_level_only() {
        let out = changes(
            "```rust\nfn main() {}\n```\n",
            "```rust\nfn main() { work(); }\n```\n",
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag, "pre");
        assert!(out[0].whole);
        assert!(out[0].spans.is_empty());
    }

    #[test]
    fn oversized_documents_are_not_diffed() {
        let big = "x ".repeat(MAX_DIFF_BYTES);
        assert!(changes(&big, "small\n").is_empty());
    }
}

