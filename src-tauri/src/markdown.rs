use comrak::create_formatter;
use comrak::html::ChildRendering;
use comrak::nodes::NodeValue;
use comrak::{parse_document, Arena, Options};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::Read as _;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

#[derive(Serialize, Clone)]
pub struct RenderResult {
    pub html: String,
    pub title: String,
    pub path: String,
}

fn resolve_path(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(p)
    }
}

pub(crate) fn is_remote_url(src: &str) -> bool {
    let lower = src.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.starts_with("data:")
}

/// Resolve an image `src` relative to the directory containing `markdown_path`.
fn resolve_asset_path(markdown_path: &Path, src: &str) -> String {
    if is_remote_url(src) {
        return src.to_string();
    }

    let src_path = PathBuf::from(src);
    let resolved = if src_path.is_absolute() {
        src_path
    } else {
        markdown_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(src_path)
    };

    resolved
        .canonicalize()
        .unwrap_or(resolved)
        .to_string_lossy()
        .into_owned()
}

fn rewrite_img_tag_src(tag: &str, markdown_path: &Path) -> String {
    let lower = tag.to_ascii_lowercase();
    let Some(src_idx) = lower.find("src=") else {
        return tag.to_string();
    };

    let after_src = &tag[src_idx + 4..];
    let (quote, rest) = if let Some(rest) = after_src.strip_prefix('"') {
        ('"', rest)
    } else if let Some(rest) = after_src.strip_prefix('\'') {
        ('\'', rest)
    } else {
        return tag.to_string();
    };

    let Some(end_quote) = rest.find(quote) else {
        return tag.to_string();
    };

    let raw_src = &rest[..end_quote];
    let resolved = resolve_asset_path(markdown_path, raw_src);
    let prefix = &tag[..src_idx + 4 + 1];
    format!("{prefix}{resolved}{quote}{}", &rest[end_quote + 1..])
}

/// Rewrite `src` attributes on embedded `<img>` tags in raw HTML blocks.
fn rewrite_html_image_srcs(html: &str, markdown_path: &Path) -> String {
    let mut result = String::with_capacity(html.len());
    let lower_html = html.to_ascii_lowercase();
    let mut search_from = 0;

    while let Some(rel) = lower_html[search_from..].find("<img") {
        let start = search_from + rel;
        let Some(tag_end_rel) = html[start..].find('>') else {
            result.push_str(&html[search_from..]);
            return result;
        };
        let end = start + tag_end_rel + 1;

        result.push_str(&html[search_from..start]);
        let tag = &html[start..end];
        result.push_str(&rewrite_img_tag_src(tag, markdown_path));
        search_from = end;
    }

    result.push_str(&html[search_from..]);
    result
}

/// Sanitize rendered HTML to neutralize XSS from untrusted markdown
/// (scripts, event handlers, `javascript:` URLs) while preserving the
/// structural markup the reader relies on (heading ids for the TOC,
/// `language-*` code classes, task-list checkboxes, image alignment).
fn sanitize_html(html: &str) -> String {
    ammonia::Builder::default()
        .add_tags(["input"])
        .add_generic_attributes(["class", "id", "align"])
        .add_tag_attributes("input", ["type", "checked", "disabled"])
        // Drop `<script>`/`<style>` *contents* too — otherwise Ammonia keeps the
        // inner text of a disallowed tag, leaking script/CSS source as visible
        // text when we render a full HTML document.
        .clean_content_tags(HashSet::from(["script", "style"]))
        .url_relative(ammonia::UrlRelative::PassThrough)
        .clean(html)
        .to_string()
}

fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string()
}

fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{FEFF}').unwrap_or(s)
}

fn skip_leading_front_matter_prefix(content: &str) -> usize {
    let content = strip_bom(content);
    let mut pos = 0;
    loop {
        while pos < content.len() {
            let rest = &content[pos..];
            if rest.starts_with(' ') || rest.starts_with('\t') {
                pos += 1;
                continue;
            }
            break;
        }
        if pos >= content.len() {
            return pos;
        }
        if content[pos..].starts_with("<!--") {
            let rest = &content[pos..];
            let Some(comment_end) = rest.find("-->") else {
                return pos;
            };
            pos += comment_end + 3;
            continue;
        }
        let line_end = content[pos..]
            .find('\n')
            .map(|i| pos + i)
            .unwrap_or(content.len());
        let line = content[pos..line_end].trim_end().trim_end_matches('\r');
        if line.is_empty() {
            pos = if line_end < content.len() {
                line_end + 1
            } else {
                content.len()
            };
            continue;
        }
        break;
    }
    pos
}

fn split_front_matter(content: &str) -> Option<(&str, &str, &str)> {
    let content = strip_bom(content);
    let mut pos = skip_leading_front_matter_prefix(content);
    if pos >= content.len() {
        return None;
    }
    let line_end = content[pos..]
        .find('\n')
        .map(|i| pos + i)
        .unwrap_or(content.len());
    let first_line = content[pos..line_end].trim_end().trim_end_matches('\r');
    let (delimiter, lang) = match first_line {
        "---" => ("---", "yaml"),
        "~~~" => ("~~~", ""),
        _ => return None,
    };
    if line_end >= content.len() {
        return None;
    }
    pos = line_end + 1;

    let preamble_start = pos;
    let mut preamble_end = None;
    let mut body_start = None;

    while pos < content.len() {
        let line_end = content[pos..]
            .find('\n')
            .map(|i| pos + i)
            .unwrap_or(content.len());
        let line = content[pos..line_end].trim_end().trim_end_matches('\r');
        if line == delimiter {
            preamble_end = Some(pos);
            body_start = Some(if line_end < content.len() {
                line_end + 1
            } else {
                content.len()
            });
            break;
        }
        pos = if line_end < content.len() {
            line_end + 1
        } else {
            content.len()
        };
    }

    let preamble_end = preamble_end?;
    let body_start = body_start?;
    let preamble = content[preamble_start..preamble_end].trim_end();
    let body = &content[body_start..];

    Some((preamble, lang, body))
}

fn title_from_front_matter(content: &str) -> Option<String> {
    let (inner, _, _) = split_front_matter(content)?;
    for line in inner.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("title:") {
            let title = rest.trim().trim_matches('"').trim_matches('\'');
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

fn title_from_markdown(content: &str, path: &Path) -> String {
    if let Some(title) = title_from_front_matter(content) {
        return title;
    }

    let body = split_front_matter(content)
        .map(|(_, _, body)| body)
        .unwrap_or(content);

    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let plain = strip_inline_markdown(rest);
            if !plain.is_empty() {
                return plain;
            }
        }
    }

    title_from_path(path)
}

/// Strip inline markdown syntax (emphasis, code spans, links, etc.) from a
/// string, returning its plain-text content. Used so window titles derived
/// from a `# ` heading read as plain text instead of raw markdown.
fn strip_inline_markdown(text: &str) -> String {
    let arena = Arena::new();
    let options = Options::default();
    let root = parse_document(&arena, text, &options);

    fn collect<'a>(node: &'a comrak::nodes::AstNode<'a>, out: &mut String) {
        match &node.data.borrow().value {
            NodeValue::Text(t) => out.push_str(t),
            NodeValue::Code(c) => out.push_str(&c.literal),
            NodeValue::SoftBreak | NodeValue::LineBreak => out.push(' '),
            _ => {}
        }
        for child in node.children() {
            collect(child, out);
        }
    }

    let mut out = String::new();
    collect(root, &mut out);
    out.trim().to_string()
}

/// Wrap YAML/metadata preamble (`---` or `~~~`) in a fenced code block.
fn preprocess_front_matter(src: &str) -> String {
    let Some((preamble, lang, body)) = split_front_matter(src) else {
        return src.to_string();
    };

    let mut out = String::from("```");
    out.push_str(lang);
    out.push('\n');
    out.push_str(preamble);
    out.push_str("\n```\n\n");
    out.push_str(body);
    out
}

fn html_escape(s: &str) -> String {
    let mut out = String::new();
    comrak::html::escape(&mut out, s).expect("escape to string");
    out
}

/// Convert ` ```math ` fenced blocks to `$$...$$` display math.
fn preprocess_math_fences(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut lines = src.split_inclusive('\n').peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with("```math") {
            out.push_str("$$\n");
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    out.push_str("\n$$");
                    if inner.ends_with('\n') {
                        out.push('\n');
                    }
                    break;
                }
                out.push_str(inner);
            }
            continue;
        }
        out.push_str(line);
    }

    out
}

fn read_backtick_run(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut run = String::new();
    if chars.peek() == Some(&'`') {
        run.push(chars.next().unwrap());
        while chars.peek() == Some(&'`') {
            run.push(chars.next().unwrap());
        }
    }
    run
}

fn at_line_start(out: &str) -> bool {
    out.is_empty() || out.ends_with('\n')
}

fn try_open_fence(
    marker: &str,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    out: &mut String,
) -> bool {
    if marker.len() < 3 {
        return false;
    }
    let mut info = String::new();
    while let Some(&next) = chars.peek() {
        if next == '\n' || next == '\r' {
            break;
        }
        info.push(chars.next().unwrap());
    }
    if !matches!(chars.peek(), Some('\n') | Some('\r') | None) {
        out.push_str(&info);
        return false;
    }
    out.push_str(marker);
    out.push_str(&info);
    true
}

enum FenceCloseResult {
    Closed(String),
    NotClosing(String),
}

fn try_close_fence(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    fence_marker: &str,
) -> Option<FenceCloseResult> {
    let mut collected = String::new();
    while chars.peek() == Some(&'`') {
        collected.push(chars.next().unwrap());
    }
    if collected.len() < fence_marker.len() {
        return Some(FenceCloseResult::NotClosing(collected));
    }
    while matches!(chars.peek(), Some(' ') | Some('\t')) {
        collected.push(chars.next().unwrap());
    }
    if matches!(chars.peek(), Some('\n') | Some('\r') | None) {
        return Some(FenceCloseResult::Closed(collected));
    }
    Some(FenceCloseResult::NotClosing(collected))
}

/// Convert pandoc-style `\(...\)` / `\[...\]` to Comrak `$...$` / `$$...$$`,
/// skipping fenced and inline code.
fn preprocess_tex_delimiters(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_fence = false;
    let mut fence_marker = String::new();
    let mut inline_code_marker: Option<String> = None;

    while let Some(ch) = chars.next() {
        if in_fence {
            out.push(ch);
            if ch == '\n' {
                if let Some(result) = try_close_fence(&mut chars, &fence_marker) {
                    match result {
                        FenceCloseResult::Closed(closing) => {
                            out.push_str(&closing);
                            in_fence = false;
                            fence_marker.clear();
                        }
                        FenceCloseResult::NotClosing(literal) => out.push_str(&literal),
                    }
                }
            }
            continue;
        }

        if ch == '`' {
            let mut run = String::from("`");
            while chars.peek() == Some(&'`') {
                run.push(chars.next().unwrap());
            }
            if inline_code_marker.is_none()
                && at_line_start(&out)
                && try_open_fence(&run, &mut chars, &mut out)
            {
                in_fence = true;
                fence_marker = run;
                continue;
            }
            match inline_code_marker.as_ref() {
                Some(marker) if marker == &run => inline_code_marker = None,
                None => inline_code_marker = Some(run.clone()),
                Some(_) => {}
            }
            out.push_str(&run);
            continue;
        }

        if inline_code_marker.is_some() {
            out.push(ch);
            continue;
        }

        if ch == '\n' {
            out.push('\n');
            let marker = read_backtick_run(&mut chars);

            if marker.len() >= 3 && try_open_fence(&marker, &mut chars, &mut out) {
                in_fence = true;
                fence_marker = marker;
                continue;
            }

            out.push_str(&marker);
            continue;
        }

        if ch == '\\' {
            match chars.peek() {
                Some('(') => {
                    chars.next();
                    out.push('$');
                    loop {
                        match chars.next() {
                            None => break,
                            Some('\\') => {
                                if chars.peek() == Some(&')') {
                                    chars.next();
                                    out.push('$');
                                    break;
                                }
                                out.push('\\');
                            }
                            Some(c) => out.push(c),
                        }
                    }
                    continue;
                }
                Some('[') => {
                    chars.next();
                    out.push('$');
                    out.push('$');
                    loop {
                        match chars.next() {
                            None => break,
                            Some('\\') => {
                                if chars.peek() == Some(&']') {
                                    chars.next();
                                    out.push('$');
                                    out.push('$');
                                    break;
                                }
                                out.push('\\');
                            }
                            Some(c) => out.push(c),
                        }
                    }
                    continue;
                }
                _ => {
                    out.push(ch);
                }
            }
            continue;
        }

        out.push(ch);
    }

    out
}

fn comrak_options_for(markdown_path: &Path) -> Options<'static> {
    let doc_path = markdown_path.to_path_buf();
    let mut options = Options::default();
    options.extension.math_dollars = true;
    options.extension.math_code = true;
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.render.tasklist_classes = true;
    options.render.r#unsafe = true;
    options.extension.header_id_prefix = Some(String::new());
    options.extension.image_url_rewriter = Some(Arc::new(move |url: &str| {
        resolve_asset_path(&doc_path, url)
    }));
    options
}

create_formatter!(MathJaxFormatter, {
    NodeValue::Math(ref nm) => |context, entering| {
        if !entering {
            return Ok(ChildRendering::HTML);
        }
        let escaped = html_escape(&nm.literal);
        if nm.display_math {
            write!(context, "$$\n{escaped}\n$$").expect("write display math");
        } else {
            write!(context, "${escaped}$").expect("write inline math");
        }
        return Ok(ChildRendering::Skip);
    },
});

/// How a loaded resource should be interpreted before rendering. emede opens
/// arbitrary paths and URLs from its home screen, so the bytes are not always
/// Markdown; this drives the normalization in [`render_content`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentKind {
    Markdown,
    Html,
    Json,
    PlainText,
    /// A recognized-but-unsupported text format, shown in a fenced code block
    /// tagged with this language.
    Code(&'static str),
}

/// Map an HTTP `Content-Type` header to a [`ContentKind`]. The charset and
/// other parameters after `;` are ignored. Returns `None` for types we don't
/// recognize (callers then fall back to extension / sniffing).
fn kind_from_content_type(content_type: &str) -> Option<ContentKind> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let kind = match mime.as_str() {
        "text/markdown" | "text/x-markdown" => ContentKind::Markdown,
        "text/html" | "application/xhtml+xml" => ContentKind::Html,
        "application/json" => ContentKind::Json,
        "text/plain" => ContentKind::PlainText,
        "application/xml" | "text/xml" => ContentKind::Code("xml"),
        "text/csv" => ContentKind::Code("csv"),
        "text/yaml" | "application/yaml" | "application/x-yaml" => ContentKind::Code("yaml"),
        "application/toml" => ContentKind::Code("toml"),
        "application/javascript" | "text/javascript" => ContentKind::Code("javascript"),
        "text/css" => ContentKind::Code("css"),
        _ if mime.ends_with("+json") => ContentKind::Json,
        _ if mime.ends_with("+xml") => ContentKind::Code("xml"),
        _ => return None,
    };
    Some(kind)
}

/// Map a file extension to a [`ContentKind`]. Returns `None` for unknown
/// extensions (callers then fall back to content sniffing).
fn kind_from_extension(path: &Path) -> Option<ContentKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let kind = match ext.as_str() {
        "md" | "markdown" | "mdown" | "mkd" | "mkdn" => ContentKind::Markdown,
        "html" | "htm" | "xhtml" => ContentKind::Html,
        "json" => ContentKind::Json,
        "txt" | "text" | "log" => ContentKind::PlainText,
        "xml" => ContentKind::Code("xml"),
        "csv" => ContentKind::Code("csv"),
        "tsv" => ContentKind::Code("tsv"),
        "yaml" | "yml" => ContentKind::Code("yaml"),
        "toml" => ContentKind::Code("toml"),
        "ini" | "cfg" | "conf" => ContentKind::Code("ini"),
        "rs" => ContentKind::Code("rust"),
        "py" => ContentKind::Code("python"),
        "js" | "mjs" | "cjs" => ContentKind::Code("javascript"),
        "ts" => ContentKind::Code("typescript"),
        "jsx" => ContentKind::Code("jsx"),
        "tsx" => ContentKind::Code("tsx"),
        "c" | "h" => ContentKind::Code("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => ContentKind::Code("cpp"),
        "go" => ContentKind::Code("go"),
        "java" => ContentKind::Code("java"),
        "rb" => ContentKind::Code("ruby"),
        "php" => ContentKind::Code("php"),
        "sh" | "bash" | "zsh" => ContentKind::Code("bash"),
        "css" => ContentKind::Code("css"),
        "sql" => ContentKind::Code("sql"),
        _ => return None,
    };
    Some(kind)
}

/// Last-resort classification by inspecting the leading bytes: used only when
/// neither the `Content-Type` header nor a file extension is conclusive.
/// Defaults to Markdown, which is the historical behavior for prose.
fn sniff_kind(content: &str) -> ContentKind {
    let trimmed = strip_bom(content).trim_start();
    match trimmed.chars().next() {
        Some('<') => {
            let head = trimmed[..trimmed.len().min(64)].to_ascii_lowercase();
            if head.starts_with("<?xml") {
                ContentKind::Code("xml")
            } else {
                ContentKind::Html
            }
        }
        Some('{') | Some('[') if serde_json::from_str::<Value>(trimmed).is_ok() => {
            ContentKind::Json
        }
        _ => ContentKind::Markdown,
    }
}

/// Extract a [`ContentKind`] from a URL's path extension, ignoring the query
/// string and fragment (e.g. `.../data.json?v=2#top` → Json).
fn url_path_kind(url: &str) -> Option<ContentKind> {
    let path = url
        .split('#')
        .next()
        .unwrap_or(url)
        .split('?')
        .next()
        .unwrap_or(url);
    kind_from_extension(Path::new(path))
}

/// Decide how to interpret a fetched URL body. `Content-Type` wins when it maps
/// to a specific format, but the generic `text/plain` / `application/octet-stream`
/// types are treated as inconclusive so that, e.g., raw Markdown served as
/// `text/plain` still renders as Markdown. Falls back to the URL extension, then
/// content sniffing.
fn resolve_url_kind(content_type: Option<&str>, url: &str, body: &str) -> ContentKind {
    if let Some(ct) = content_type {
        let mime = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if mime != "text/plain" && mime != "application/octet-stream" {
            if let Some(kind) = kind_from_content_type(&mime) {
                return kind;
            }
        }
    }
    url_path_kind(url).unwrap_or_else(|| sniff_kind(body))
}

/// Decide how to interpret a local file: extension is authoritative, with
/// content sniffing as the fallback for extensionless files.
fn resolve_local_kind(path: &Path, content: &str) -> ContentKind {
    kind_from_extension(path).unwrap_or_else(|| sniff_kind(content))
}

/// Largest local markdown file (bytes) we will read and render.
const MAX_LOCAL_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// Decode file bytes as UTF-8 text, rejecting binary content. Returns `None`
/// for files that are not valid UTF-8 or that contain NUL bytes (the common
/// signature of binary formats like PDF, images, and archives), so callers can
/// surface a clear "not a valid text file" message instead of a raw decode error.
fn decode_text(bytes: Vec<u8>) -> Option<String> {
    let text = String::from_utf8(bytes).ok()?;
    if text.as_bytes().contains(&0) {
        return None;
    }
    Some(text)
}

pub fn render_markdown_inner(path: &str) -> Result<RenderResult, String> {
    let resolved = resolve_path(path);
    if !resolved.exists() {
        return Err(format!("File not found: {}", resolved.display()));
    }
    if !resolved.is_file() {
        return Err(format!("Not a file: {}", resolved.display()));
    }

    if let Ok(meta) = std::fs::metadata(&resolved) {
        if meta.len() > MAX_LOCAL_FILE_BYTES {
            return Err(format!(
                "File too large ({} MB). emede opens markdown files up to {} MB.",
                meta.len() / (1024 * 1024),
                MAX_LOCAL_FILE_BYTES / (1024 * 1024)
            ));
        }
    }

    let bytes = std::fs::read(&resolved).map_err(|e| e.to_string())?;
    let raw = decode_text(bytes).ok_or_else(|| {
        format!(
            "Not a valid text file: {}. emede opens Markdown and other text-based files, not binary files such as PDFs, images, or archives.",
            resolved.display()
        )
    })?;
    let kind = resolve_local_kind(&resolved, &raw);
    let source_id = resolved.to_string_lossy().into_owned();
    render_content(&raw, kind, &resolved, &source_id, true)
}

#[tauri::command]
pub fn render_markdown(path: String) -> Result<RenderResult, String> {
    let result = render_markdown_inner(&path)?;
    crate::recents::add_recent(&result.path, &result.title);
    Ok(result)
}

/// Normalize a raw resource of the given [`ContentKind`] into a rendered
/// [`RenderResult`]. `local` is true for on-disk files (enables resolving
/// relative `<img>` paths and using the filename for titles).
fn render_content(
    raw: &str,
    kind: ContentKind,
    source_path: &Path,
    source_id: &str,
    local: bool,
) -> Result<RenderResult, String> {
    match kind {
        ContentKind::Markdown => {
            let title = title_from_markdown(raw, source_path);
            render_markdown_core(raw, title, source_path, source_id, local)
        }
        ContentKind::Html => render_html_content(raw, source_path, source_id, local),
        ContentKind::Json => render_json_content(raw, source_path, source_id, local),
        ContentKind::PlainText => Ok(render_plain_text(raw, source_path, source_id)),
        ContentKind::Code(lang) => render_code_content(raw, lang, source_path, source_id, local),
    }
}

/// Core Markdown → HTML pipeline shared by every entry point.
fn render_markdown_core(
    content: &str,
    title: String,
    source_path: &Path,
    source_id: &str,
    rewrite_local_images: bool,
) -> Result<RenderResult, String> {
    let with_front_matter = preprocess_front_matter(content);
    let with_fences = preprocess_math_fences(&with_front_matter);
    let preprocessed = preprocess_tex_delimiters(&with_fences);

    let arena = Arena::new();
    let options = comrak_options_for(source_path);
    let root = parse_document(&arena, &preprocessed, &options);

    let mut html = String::new();
    MathJaxFormatter::format_document(root, &options, &mut html)
        .map_err(|e| format!("Failed to render markdown: {e}"))?;
    let html = if rewrite_local_images {
        rewrite_html_image_srcs(&html, source_path)
    } else {
        html
    };
    let html = sanitize_html(&html);

    Ok(RenderResult {
        html,
        title,
        path: source_id.to_string(),
    })
}

/// Choose a title for non-Markdown content: the source filename when available,
/// otherwise the last path segment of the source id (e.g. a URL).
fn fallback_title(source_path: &Path, source_id: &str) -> String {
    let from_path = title_from_path(source_path);
    if from_path != "Untitled" {
        return from_path;
    }
    let path = source_id
        .split('#')
        .next()
        .unwrap_or(source_id)
        .split('?')
        .next()
        .unwrap_or(source_id);
    path.trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Untitled".to_string())
}

/// A fenced-code delimiter guaranteed to be longer than any run of backticks in
/// `content`, so embedded backticks can't prematurely close the fence.
fn fence_for(content: &str) -> String {
    let mut max_run = 0usize;
    let mut cur = 0usize;
    for ch in content.chars() {
        if ch == '`' {
            cur += 1;
            max_run = max_run.max(cur);
        } else {
            cur = 0;
        }
    }
    "`".repeat(max_run.max(2) + 1)
}

/// Slice out the inner content of `<body>…</body>`, falling back to the whole
/// document when there is no `<body>` element.
fn extract_html_body(html: &str) -> &str {
    let lower = html.to_ascii_lowercase();
    let Some(open) = lower.find("<body") else {
        return html;
    };
    let Some(gt) = lower[open..].find('>') else {
        return html;
    };
    let start = open + gt + 1;
    let end = lower[start..]
        .find("</body>")
        .map(|i| start + i)
        .unwrap_or(html.len());
    &html[start..end]
}

/// Extract the trimmed text of the document's `<title>` element, if any.
fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let gt = lower[open..].find('>')? + open + 1;
    let close = lower[gt..].find("</title>")? + gt;
    let title = html[gt..close].trim();
    (!title.is_empty()).then(|| title.to_string())
}

/// Render an HTML document as "simple HTML": extract `<body>`, resolve local
/// image paths, and sanitize it for direct display.
fn render_html_content(
    raw: &str,
    source_path: &Path,
    source_id: &str,
    local: bool,
) -> Result<RenderResult, String> {
    let body = extract_html_body(raw);
    let rewritten;
    let body = if local {
        rewritten = rewrite_html_image_srcs(body, source_path);
        rewritten.as_str()
    } else {
        body
    };
    let html = sanitize_html(body);
    let title = html_title(raw).unwrap_or_else(|| fallback_title(source_path, source_id));
    Ok(RenderResult {
        html,
        title,
        path: source_id.to_string(),
    })
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// A one-line description of a JSON value's type, including collection sizes.
fn json_value_label(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("array ({} item{})", a.len(), plural(a.len())),
        Value::Object(o) => format!("object ({} key{})", o.len(), plural(o.len())),
        other => json_type_name(other).to_string(),
    }
}

/// Build the Markdown "structure summary" shown above the pretty-printed JSON.
fn json_summary_markdown(value: &Value) -> String {
    let mut out = String::new();
    match value {
        Value::Object(map) => {
            let _ = writeln!(out, "**JSON object — {} key{}**", map.len(), plural(map.len()));
            let _ = writeln!(out);
            for (k, v) in map {
                let _ = writeln!(out, "- `{}`: {}", k, json_value_label(v));
            }
        }
        Value::Array(arr) => {
            let _ = writeln!(out, "**JSON array — {} item{}**", arr.len(), plural(arr.len()));
            let _ = writeln!(out);
            let mut types: Vec<&'static str> = Vec::new();
            for v in arr {
                let t = json_type_name(v);
                if !types.contains(&t) {
                    types.push(t);
                }
            }
            if !types.is_empty() {
                let _ = writeln!(out, "- element type{}: {}", plural(types.len()), types.join(", "));
            }
        }
        other => {
            let _ = writeln!(out, "**JSON {}**", json_type_name(other));
        }
    }
    out
}

/// Render JSON as a structure summary followed by pretty-printed JSON in a
/// fenced block. Malformed JSON falls back to a plain code block.
fn render_json_content(
    raw: &str,
    source_path: &Path,
    source_id: &str,
    local: bool,
) -> Result<RenderResult, String> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return render_code_content(raw, "json", source_path, source_id, local);
    };
    let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| raw.to_string());
    let fence = fence_for(&pretty);
    let md = format!(
        "{summary}\n{fence}json\n{pretty}\n{fence}\n",
        summary = json_summary_markdown(&value),
    );
    let title = fallback_title(source_path, source_id);
    render_markdown_core(&md, title, source_path, source_id, local)
}

/// Render plain text verbatim: HTML-escaped inside a wrapping `<pre>` block.
fn render_plain_text(raw: &str, source_path: &Path, source_id: &str) -> RenderResult {
    let html = format!("<pre class=\"plain-text\">{}</pre>", html_escape(raw));
    RenderResult {
        html,
        title: fallback_title(source_path, source_id),
        path: source_id.to_string(),
    }
}

/// Render arbitrary source in a fenced code block tagged with `lang`.
fn render_code_content(
    raw: &str,
    lang: &str,
    source_path: &Path,
    source_id: &str,
    local: bool,
) -> Result<RenderResult, String> {
    let fence = fence_for(raw);
    let md = format!("{fence}{lang}\n{raw}\n{fence}\n");
    let title = fallback_title(source_path, source_id);
    render_markdown_core(&md, title, source_path, source_id, local)
}

/// Largest remote document (bytes) we will fetch and render.
const MAX_URL_BYTES: u64 = 10 * 1024 * 1024;

/// Reject any address that could reach the loopback interface, a private/local
/// network, or cloud metadata endpoints. Applied to every resolved address —
/// including each redirect hop — to block SSRF via user-supplied URLs.
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                // 0.0.0.0/8 "this network"
                || o[0] == 0
                // 100.64.0.0/10 carrier-grade NAT / shared address space
                || (o[0] == 100 && (o[1] & 0xc0) == 0x40))
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            let seg = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 unique local addresses
                || (seg[0] & 0xfe00) == 0xfc00
                // fe80::/10 link-local
                || (seg[0] & 0xffc0) == 0xfe80)
        }
    }
}

/// Shared HTTP agent with hard timeouts and an SSRF-blocking resolver.
fn http_agent() -> ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT
        .get_or_init(|| {
            ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(10))
                .timeout_read(Duration::from_secs(30))
                .timeout(Duration::from_secs(60))
                .resolver(|netloc: &str| -> std::io::Result<Vec<SocketAddr>> {
                    let filtered: Vec<SocketAddr> = netloc
                        .to_socket_addrs()?
                        .filter(|a| is_public_ip(a.ip()))
                        .collect();
                    if filtered.is_empty() {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "refusing to fetch a private or local address",
                        ))
                    } else {
                        Ok(filtered)
                    }
                })
                .build()
        })
        .clone()
}

/// Fetch a remote `http(s)` URL under strict limits and render it.
fn fetch_and_render_url(url: &str) -> Result<RenderResult, String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("Only http and https URLs can be opened.".to_string());
    }

    let response = http_agent()
        .get(url)
        .call()
        .map_err(|e| format!("Failed to fetch URL: {e}"))?;

    // Capture the content type before consuming the body so we can pick the
    // right renderer (Markdown / HTML / JSON / plain text / code).
    let content_type = response.header("Content-Type").map(|s| s.to_string());

    // Cap the body: read one byte past the limit so we can detect overflow
    // instead of silently truncating (ureq's into_string caps at 10 MB quietly).
    let mut content = String::new();
    response
        .into_reader()
        .take(MAX_URL_BYTES + 1)
        .read_to_string(&mut content)
        .map_err(|e| format!("Failed to read response body: {e}"))?;
    if content.len() as u64 > MAX_URL_BYTES {
        return Err(format!(
            "Remote document is too large (over {} MB).",
            MAX_URL_BYTES / (1024 * 1024)
        ));
    }

    let kind = resolve_url_kind(content_type.as_deref(), url, &content);
    render_content(&content, kind, Path::new("."), url, false)
}

/// Render a local file path or a remote URL, routing to the right backend.
pub fn render_markdown_any(path_or_url: &str) -> Result<RenderResult, String> {
    if is_remote_url(path_or_url) {
        fetch_and_render_url(path_or_url)
    } else {
        render_markdown_inner(path_or_url)
    }
}

#[tauri::command]
pub fn render_markdown_url(url: String) -> Result<RenderResult, String> {
    fetch_and_render_url(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_private_and_local_ips() {
        let blocked = [
            "127.0.0.1",
            "0.0.0.0",
            "10.0.0.5",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254", // cloud metadata
            "100.64.0.1",      // CGNAT
            "::1",
            "fe80::1",
            "fc00::1",
            "fd00::1",
            "::ffff:127.0.0.1", // ipv4-mapped loopback
        ];
        for ip in blocked {
            assert!(
                !is_public_ip(ip.parse().unwrap()),
                "{ip} should be blocked"
            );
        }

        let allowed = ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:2800:220:1::1"];
        for ip in allowed {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} should be allowed");
        }
    }

    #[test]
    fn decode_text_accepts_utf8_and_rejects_binary() {
        assert_eq!(decode_text(b"# hello".to_vec()).as_deref(), Some("# hello"));
        // PDF header followed by a NUL byte, as in a real binary file.
        assert!(decode_text(b"%PDF-1.7\x00\x01binary".to_vec()).is_none());
        // Invalid UTF-8 sequence.
        assert!(decode_text(vec![0xff, 0xfe, 0x00]).is_none());
    }

    #[test]
    fn rejects_binary_file_with_clear_message() {
        let dir = std::env::temp_dir();
        let path = dir.join("emede_test_binary.pdf");
        std::fs::write(&path, b"%PDF-1.7\x00\x01\x02 binary payload").unwrap();
        let result = render_markdown_inner(&path.to_string_lossy());
        std::fs::remove_file(&path).ok();
        let err = match result {
            Ok(_) => panic!("binary file should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.contains("Not a valid text file"),
            "unexpected error message: {err}"
        );
    }

    #[test]
    fn renders_readme_fenced_code_blocks() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../README.md");
        let result = render_markdown_inner(path).expect("render README");
        assert!(
            result.html.contains("<pre>"),
            "expected fenced code blocks in README, got: {}",
            &result.html[..result.html.len().min(2000)]
        );
        assert!(
            !result.html.contains("<h1># development</h1>")
                && !result.html.contains("<h1>development</h1>"),
            "comment in code block became a heading; html snippet: {}",
            &result.html[..result.html.len().min(3000)]
        );
    }

    #[test]
    fn renders_readme_embedded_html() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../README.md");
        let result = render_markdown_inner(path).expect("render README");
        assert!(
            !result.html.contains("raw HTML omitted"),
            "embedded HTML was stripped from README, got: {}",
            &result.html[..result.html.len().min(2000)]
        );
        let expected_img = resolve_asset_path(Path::new(path), "src-tauri/icons/128x128.png");
        assert!(
            result
                .html
                .contains(&format!(r#"<img src="{expected_img}""#))
                && result.html.contains(r#"<h1 align="center">emede</h1>"#),
            "expected README header image and title HTML, got: {}",
            &result.html[..result.html.len().min(2000)]
        );
    }

    #[test]
    fn renders_fenced_code_blocks() {
        let src = "```bash\n# development\nnpm install\n```\n";
        let preprocessed = preprocess_tex_delimiters(&preprocess_math_fences(src));
        let arena = Arena::new();
        let options = comrak_options_for(Path::new("test.md"));
        let root = parse_document(&arena, &preprocessed, &options);
        let mut html = String::new();
        MathJaxFormatter::format_document(root, &options, &mut html).unwrap();
        assert!(
            html.contains("<pre>"),
            "expected fenced code block, got: {html}"
        );
        assert!(
            !html.contains("<h1"),
            "comment should not become a heading, got: {html}"
        );
    }

    #[test]
    fn renders_heading_ids() {
        let html = {
            let src = "# Hello World\n\n## Section Two\n";
            let preprocessed = preprocess_tex_delimiters(&preprocess_math_fences(src));
            let arena = Arena::new();
            let options = comrak_options_for(Path::new("test.md"));
            let root = parse_document(&arena, &preprocessed, &options);
            let mut html = String::new();
            MathJaxFormatter::format_document(root, &options, &mut html).unwrap();
            html
        };
        assert!(
            html.contains("id=\"hello-world\""),
            "expected hello-world heading id, got: {html}"
        );
        assert!(
            html.contains("id=\"section-two\""),
            "expected section-two heading id, got: {html}"
        );
    }

    #[test]
    fn wraps_yaml_front_matter_in_code_block() {
        let src = "---\ntitle: My Doc\nauthor: Alice\n---\n\n# Hello\n";
        let preprocessed =
            preprocess_tex_delimiters(&preprocess_math_fences(&preprocess_front_matter(src)));
        let arena = Arena::new();
        let options = comrak_options_for(Path::new("test.md"));
        let root = parse_document(&arena, &preprocessed, &options);
        let mut html = String::new();
        MathJaxFormatter::format_document(root, &options, &mut html).unwrap();
        assert!(
            html.contains("title: My Doc") && html.contains("<pre>"),
            "expected preamble content in code block, got: {html}"
        );
        assert!(
            !html.contains("<hr"),
            "preamble delimiters should not become horizontal rules, got: {html}"
        );
        assert!(html.contains("Hello</h1>"));
    }

    #[test]
    fn wraps_tilde_front_matter_in_code_block() {
        let src = "~~~\ntitle = \"My Doc\"\n~~~\n\nBody text.\n";
        let preprocessed = preprocess_front_matter(src);
        assert!(preprocessed.starts_with("```\n"));
        assert!(preprocessed.contains("title = \"My Doc\""));
        assert!(preprocessed.contains("Body text."));
        assert!(!preprocessed.starts_with("~~~"));
    }

    #[test]
    fn title_from_yaml_front_matter() {
        let src = "---\ntitle: Lecture Notes\n---\n\n# Ignored Heading\n";
        assert_eq!(
            title_from_markdown(src, Path::new("notes.md")),
            "Lecture Notes"
        );
    }

    #[test]
    fn title_strips_inline_markdown() {
        let src = "# Title **bold** -- text `code`\n";
        assert_eq!(
            title_from_markdown(src, Path::new("notes.md")),
            "Title bold -- text code"
        );
    }

    #[test]
    fn title_strips_link_markup() {
        let src = "# See [the docs](https://example.com) now\n";
        assert_eq!(
            title_from_markdown(src, Path::new("notes.md")),
            "See the docs now"
        );
    }

    #[test]
    fn ignores_unclosed_front_matter() {
        let src = "---\ntitle: Broken\n\n# Still Works\n";
        let preprocessed = preprocess_front_matter(src);
        assert_eq!(preprocessed, src);
    }

    #[test]
    fn preprocess_preserves_fenced_code_at_start() {
        let src = "```bash\n# development\nnpm install\n```\n";
        let out = preprocess_tex_delimiters(&preprocess_math_fences(src));
        assert_eq!(out, src, "preprocessed output drifted");
    }

    #[test]
    fn wraps_front_matter_after_html_comment() {
        let src = "<!-- plan-id -->\n---\ntitle: Plan\n---\n\n# Body\n";
        let preprocessed = preprocess_front_matter(src);
        assert!(preprocessed.starts_with("```yaml\n"));
        assert!(preprocessed.contains("title: Plan"));
        assert!(preprocessed.contains("# Body\n"));
    }

    #[test]
    fn preserves_fenced_code_after_html_block() {
        let src = "```html\n<span>\\(...\\)</span>\n```\n\n**New listener** in `boot()`:\n\n```javascript\nlisten(\"document-updated\");\n```\n";
        let preprocessed = preprocess_tex_delimiters(&preprocess_math_fences(src));
        assert!(preprocessed.contains("```javascript\nlisten(\"document-updated\");\n```"));
        let arena = Arena::new();
        let options = comrak_options_for(Path::new("test.md"));
        let root = parse_document(&arena, &preprocessed, &options);
        let mut html = String::new();
        MathJaxFormatter::format_document(root, &options, &mut html).unwrap();
        assert!(
            html.contains("listen(&quot;document-updated&quot;)"),
            "expected javascript code block, got: {html}"
        );
        assert!(
            html.contains("<strong>New listener</strong>"),
            "expected bold listener heading text, got: {html}"
        );
    }

    #[test]
    fn preserves_inline_code_with_nested_backticks() {
        let src = "- `extension.math_code = true` (for `` $`...`$ `` and ` ```math ` blocks)\n\n```bash\nnpm test\n```\n";
        let preprocessed = preprocess_tex_delimiters(&preprocess_math_fences(src));
        assert!(preprocessed.contains("```bash\nnpm test\n```"));
        let arena = Arena::new();
        let options = comrak_options_for(Path::new("test.md"));
        let root = parse_document(&arena, &preprocessed, &options);
        let mut html = String::new();
        MathJaxFormatter::format_document(root, &options, &mut html).unwrap();
        assert!(
            html.contains("npm test"),
            "expected bash code block after inline backtick examples, got: {html}"
        );
    }

    #[test]
    fn plan_preprocess_has_javascript_fence() {
        let raw = "**New listener** in `boot()`:\n\n```javascript\nlisten(\"document-updated\", (event) => applyDocument(event.payload, { reload: true }));\n```\n";
        let preprocessed =
            preprocess_tex_delimiters(&preprocess_math_fences(&preprocess_front_matter(raw)));
        assert!(
            preprocessed.contains("```javascript\nlisten(\"document-updated\""),
            "missing javascript fence in preprocessed output around: {}",
            preprocessed
                .find("New listener")
                .map(|idx| &preprocessed[idx..(idx + 250).min(preprocessed.len())])
                .unwrap_or("New listener section missing")
        );
    }

    #[test]
    fn renders_plan_style_code_blocks() {
        let dir = std::env::temp_dir().join("emede-plan-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let md_path = dir.join("plan.md");
        std::fs::write(
            &md_path,
            "**New listener** in `boot()`:\n\n```javascript\nlisten(\"document-updated\", (event) => applyDocument(event.payload, { reload: true }));\n```\n",
        )
        .expect("write temp markdown");

        let result =
            render_markdown_inner(md_path.to_str().unwrap()).expect("render plan");
        assert!(
            result.html.contains("language-javascript") && result.html.contains("document-updated"),
            "javascript block missing; html snippet: {}",
            &result.html[result.html.len().saturating_sub(2000)..]
        );
        assert!(
            result.html.contains("<strong>New listener</strong>"),
            "expected bold listener heading text"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn renders_math_delimiters() {
        let html = {
            let src = "Inline $E=mc^2$ and display:\n\n$$x^2$$\n";
            let preprocessed = preprocess_tex_delimiters(&preprocess_math_fences(src));
            let arena = Arena::new();
            let options = comrak_options_for(Path::new("test.md"));
            let root = parse_document(&arena, &preprocessed, &options);
            let mut html = String::new();
            MathJaxFormatter::format_document(root, &options, &mut html).unwrap();
            html
        };
        assert!(html.contains("$E=mc^2$"));
        assert!(html.contains("$$\nx^2\n$$"));
    }

    #[test]
    fn resolves_markdown_image_paths_relative_to_file() {
        let html = {
            let src = "![logo](images/logo.png)\n";
            let preprocessed = preprocess_tex_delimiters(&preprocess_math_fences(src));
            let arena = Arena::new();
            let options = comrak_options_for(Path::new("/home/asdf/foo/file.md"));
            let root = parse_document(&arena, &preprocessed, &options);
            let mut html = String::new();
            MathJaxFormatter::format_document(root, &options, &mut html).unwrap();
            html
        };
        assert!(
            html.contains(r#"<img src="/home/asdf/foo/images/logo.png""#),
            "expected markdown image resolved relative to file, got: {html}"
        );
    }

    #[test]
    fn resolves_embedded_html_image_paths_relative_to_file() {
        let html = rewrite_html_image_srcs(
            r#"<img src="images/image.png" alt="test">"#,
            Path::new("/home/asdf/foo/file.md"),
        );
        assert_eq!(
            html,
            r#"<img src="/home/asdf/foo/images/image.png" alt="test">"#
        );
    }

    #[test]
    fn resolves_raw_html_images_but_not_html_code_blocks() {
        let dir = std::env::temp_dir().join("emede-img-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let md_path = dir.join("file.md");
        std::fs::write(
            &md_path,
            "Raw HTML:\n\n<img src=\"images/raw.png\" alt=\"raw\">\n\n```html\n<img src=\"images/code.png\" alt=\"code\">\n```\n",
        )
        .expect("write temp markdown");

        let result =
            render_markdown_inner(md_path.to_str().unwrap()).expect("render temp markdown");
        let expected_raw = resolve_asset_path(&md_path, "images/raw.png");

        assert!(
            result
                .html
                .contains(&format!(r#"<img src="{expected_raw}""#)),
            "expected raw HTML image to resolve, got: {}",
            &result.html[..result.html.len().min(2000)]
        );
        assert!(
            result.html.contains("images/code.png")
                && result.html.contains("&lt;img src=")
                && !result.html.contains(r#"<img src="images/code.png""#),
            "expected fenced html code block to keep image path as escaped literal text, got: {}",
            &result.html[..result.html.len().min(2000)]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_remote_image_urls_unchanged() {
        let html = rewrite_html_image_srcs(
            r#"<img src="https://example.com/pic.png" alt="remote">"#,
            Path::new("/home/asdf/foo/file.md"),
        );
        assert_eq!(
            html,
            r#"<img src="https://example.com/pic.png" alt="remote">"#
        );
    }

    #[test]
    fn sanitizes_dangerous_html() {
        let dirty = r#"<p>hello</p><script>alert(1)</script><img src="x" onerror="alert(2)"><a href="javascript:alert(3)">link</a>"#;
        let clean = sanitize_html(dirty);
        assert!(clean.contains("<p>hello</p>"), "kept benign content");
        assert!(!clean.contains("<script"), "script tag stripped: {clean}");
        assert!(!clean.contains("onerror"), "event handler stripped: {clean}");
        assert!(!clean.contains("javascript:"), "js scheme stripped: {clean}");
    }

    #[test]
    fn sanitize_preserves_structural_markup() {
        let dirty = r#"<h1 id="title" align="center">T</h1><pre><code class="language-rust">fn main(){}</code></pre><ul class="contains-task-list"><li class="task-list-item"><input type="checkbox" disabled="" checked="" class="task-list-item-checkbox">done</li></ul>"#;
        let clean = sanitize_html(dirty);
        assert!(clean.contains(r#"id="title""#), "heading id kept: {clean}");
        assert!(clean.contains(r#"align="center""#), "align kept: {clean}");
        assert!(
            clean.contains("language-rust"),
            "code language class kept: {clean}"
        );
        assert!(
            clean.contains("task-list-item-checkbox") && clean.contains("checked"),
            "task list checkbox kept: {clean}"
        );
    }

    #[test]
    fn renders_tasklist_with_classes() {
        let html = {
            let src = "- [ ] Todo\n- [x] Done\n";
            let arena = Arena::new();
            let options = comrak_options_for(Path::new("test.md"));
            let root = parse_document(&arena, src, &options);
            let mut html = String::new();
            MathJaxFormatter::format_document(root, &options, &mut html).unwrap();
            html
        };
        assert!(
            html.contains("class=\"contains-task-list\""),
            "expected task list class on ul, got: {html}"
        );
        assert!(
            html.contains("class=\"task-list-item\""),
            "expected task list item class, got: {html}"
        );
        assert!(
            html.contains("class=\"task-list-item-checkbox\""),
            "expected task list checkbox class, got: {html}"
        );
        assert!(
            html.contains("checked=\"\""),
            "expected checked attribute on completed item, got: {html}"
        );
    }

    #[test]
    fn content_type_maps_to_kind() {
        assert_eq!(
            kind_from_content_type("text/html; charset=utf-8"),
            Some(ContentKind::Html)
        );
        assert_eq!(
            kind_from_content_type("application/json"),
            Some(ContentKind::Json)
        );
        assert_eq!(
            kind_from_content_type("application/vnd.api+json"),
            Some(ContentKind::Json)
        );
        assert_eq!(
            kind_from_content_type("text/plain"),
            Some(ContentKind::PlainText)
        );
        assert_eq!(
            kind_from_content_type("text/xml"),
            Some(ContentKind::Code("xml"))
        );
        assert_eq!(kind_from_content_type("application/octet-stream"), None);
    }

    #[test]
    fn extension_maps_to_kind() {
        assert_eq!(
            kind_from_extension(Path::new("a.md")),
            Some(ContentKind::Markdown)
        );
        assert_eq!(
            kind_from_extension(Path::new("a.html")),
            Some(ContentKind::Html)
        );
        assert_eq!(
            kind_from_extension(Path::new("a.json")),
            Some(ContentKind::Json)
        );
        assert_eq!(
            kind_from_extension(Path::new("a.txt")),
            Some(ContentKind::PlainText)
        );
        assert_eq!(
            kind_from_extension(Path::new("a.rs")),
            Some(ContentKind::Code("rust"))
        );
        assert_eq!(kind_from_extension(Path::new("a.unknownext")), None);
        assert_eq!(kind_from_extension(Path::new("noext")), None);
    }

    #[test]
    fn sniff_classifies_leading_bytes() {
        assert_eq!(sniff_kind("<!doctype html><html></html>"), ContentKind::Html);
        assert_eq!(sniff_kind("  <div>hi</div>"), ContentKind::Html);
        assert_eq!(sniff_kind("<?xml version=\"1.0\"?>"), ContentKind::Code("xml"));
        assert_eq!(sniff_kind("{\"a\": 1}"), ContentKind::Json);
        assert_eq!(sniff_kind("[1, 2, 3]"), ContentKind::Json);
        // JSON-looking but invalid → falls back to Markdown.
        assert_eq!(sniff_kind("{not json"), ContentKind::Markdown);
        assert_eq!(sniff_kind("# Heading\n\ntext"), ContentKind::Markdown);
    }

    #[test]
    fn url_kind_prefers_specific_type_but_sniffs_generic() {
        // Raw Markdown is commonly served as text/plain; must stay Markdown.
        assert_eq!(
            resolve_url_kind(Some("text/plain; charset=utf-8"), "https://x/readme", "# Hi"),
            ContentKind::Markdown
        );
        // ...but a text/plain URL that ends in .txt is plain text.
        assert_eq!(
            resolve_url_kind(Some("text/plain"), "https://x/notes.txt", "hello"),
            ContentKind::PlainText
        );
        // A specific content type wins outright.
        assert_eq!(
            resolve_url_kind(Some("application/json"), "https://x/api", "{}"),
            ContentKind::Json
        );
        // No content type at all: fall back to URL extension.
        assert_eq!(
            resolve_url_kind(None, "https://x/data.json?v=2#top", "{}"),
            ContentKind::Json
        );
    }

    #[test]
    fn renders_html_document_as_sanitized_body() {
        let raw = "<html><head><title>My Page</title><style>body{color:red}</style></head><body><h1>Hello</h1><p>World</p><script>alert(1)</script></body></html>";
        let result =
            render_content(raw, ContentKind::Html, Path::new("."), "page.html", false).unwrap();
        assert_eq!(result.title, "My Page");
        assert!(result.html.contains("<h1>Hello</h1>"), "kept heading: {}", result.html);
        assert!(result.html.contains("<p>World</p>"), "kept paragraph");
        assert!(!result.html.contains("<script"), "script stripped");
        assert!(!result.html.contains("alert(1)"), "script content stripped: {}", result.html);
        assert!(!result.html.contains("color:red"), "style content stripped: {}", result.html);
    }

    #[test]
    fn renders_json_with_structure_summary() {
        let raw = r#"{"name": "x", "tags": [1, 2, 3], "meta": {"a": 1, "b": 2}}"#;
        let result =
            render_content(raw, ContentKind::Json, Path::new("data.json"), "data.json", true)
                .unwrap();
        assert!(
            result.html.contains("JSON object") && result.html.contains("3 keys"),
            "expected object summary, got: {}",
            result.html
        );
        assert!(
            result.html.contains("array (3 items)") && result.html.contains("object (2 keys)"),
            "expected value labels, got: {}",
            result.html
        );
        assert!(
            result.html.contains("language-json"),
            "expected json code block, got: {}",
            result.html
        );
    }

    #[test]
    fn malformed_json_falls_back_to_code_block() {
        let raw = "{ this is not valid json ";
        let result =
            render_content(raw, ContentKind::Json, Path::new("data.json"), "data.json", true)
                .unwrap();
        assert!(
            result.html.contains("language-json") && !result.html.contains("JSON object"),
            "expected raw code block fallback, got: {}",
            result.html
        );
    }

    #[test]
    fn renders_plain_text_verbatim() {
        let raw = "# not a heading\n* not a list\n<b>literal</b>";
        let result =
            render_content(raw, ContentKind::PlainText, Path::new("notes.txt"), "notes.txt", true)
                .unwrap();
        assert!(
            result.html.contains(r#"<pre class="plain-text">"#),
            "expected plain-text pre, got: {}",
            result.html
        );
        assert!(!result.html.contains("<h1"), "hash not a heading: {}", result.html);
        assert!(
            result.html.contains("&lt;b&gt;literal&lt;/b&gt;"),
            "literal html escaped: {}",
            result.html
        );
        assert_eq!(result.title, "notes");
    }

    #[test]
    fn renders_code_content_in_fenced_block() {
        let raw = "SELECT * FROM t;";
        let result =
            render_content(raw, ContentKind::Code("sql"), Path::new("q.sql"), "q.sql", true)
                .unwrap();
        assert!(
            result.html.contains("language-sql") && result.html.contains("SELECT"),
            "expected sql code block, got: {}",
            result.html
        );
    }

    #[test]
    fn code_content_with_embedded_backticks_stays_fenced() {
        let raw = "run ```code``` here";
        let result =
            render_content(raw, ContentKind::Code(""), Path::new("a.txt"), "a.txt", true).unwrap();
        assert!(
            result.html.contains("```code```") || result.html.contains("code"),
            "embedded backticks preserved, got: {}",
            result.html
        );
        assert!(
            result.html.contains("<pre"),
            "content stays inside a code block, got: {}",
            result.html
        );
    }
}
