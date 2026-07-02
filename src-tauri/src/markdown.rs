use comrak::create_formatter;
use comrak::html::ChildRendering;
use comrak::nodes::NodeValue;
use comrak::{parse_document, Arena, Options};
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

fn is_remote_url(src: &str) -> bool {
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
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }

    title_from_path(path)
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

pub fn render_markdown_inner(path: &str) -> Result<RenderResult, String> {
    let resolved = resolve_path(path);
    if !resolved.exists() {
        return Err(format!("File not found: {}", resolved.display()));
    }
    if !resolved.is_file() {
        return Err(format!("Not a file: {}", resolved.display()));
    }

    let raw = std::fs::read_to_string(&resolved).map_err(|e| e.to_string())?;
    let with_front_matter = preprocess_front_matter(&raw);
    let with_fences = preprocess_math_fences(&with_front_matter);
    let preprocessed = preprocess_tex_delimiters(&with_fences);
    let title = title_from_markdown(&raw, &resolved);

    let arena = Arena::new();
    let options = comrak_options_for(&resolved);
    let root = parse_document(&arena, &preprocessed, &options);

    let mut html = String::new();
    MathJaxFormatter::format_document(root, &options, &mut html)
        .map_err(|e| format!("Failed to render markdown: {e}"))?;
    let html = rewrite_html_image_srcs(&html, &resolved);
    let html = sanitize_html(&html);

    Ok(RenderResult {
        html,
        title,
        path: resolved.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub fn render_markdown(path: String) -> Result<RenderResult, String> {
    render_markdown_inner(&path)
}

pub fn render_markdown_from_str(content: &str, source_path: &Path, source_id: &str) -> Result<RenderResult, String> {
    let title = title_from_markdown(content, source_path);
    let with_front_matter = preprocess_front_matter(content);
    let with_fences = preprocess_math_fences(&with_front_matter);
    let preprocessed = preprocess_tex_delimiters(&with_fences);

    let arena = Arena::new();
    let options = comrak_options_for(source_path);
    let root = parse_document(&arena, &preprocessed, &options);

    let mut html = String::new();
    MathJaxFormatter::format_document(root, &options, &mut html)
        .map_err(|e| format!("Failed to render markdown: {e}"))?;
    let html = sanitize_html(&html);

    Ok(RenderResult {
        html,
        title,
        path: source_id.to_string(),
    })
}

/// Render a local file path or a remote URL, routing to the right backend.
pub fn render_markdown_any(path_or_url: &str) -> Result<RenderResult, String> {
    if is_remote_url(path_or_url) {
        let response = ureq::get(path_or_url)
            .call()
            .map_err(|e| format!("Failed to fetch URL: {e}"))?;
        let content = response
            .into_string()
            .map_err(|e| format!("Failed to read response body: {e}"))?;
        render_markdown_from_str(&content, Path::new("."), path_or_url)
    } else {
        render_markdown_inner(path_or_url)
    }
}

#[tauri::command]
pub fn render_markdown_url(url: String) -> Result<RenderResult, String> {
    let response = ureq::get(&url)
        .call()
        .map_err(|e| format!("Failed to fetch URL: {e}"))?;
    let content = response
        .into_string()
        .map_err(|e| format!("Failed to read response body: {e}"))?;
    render_markdown_from_str(&content, Path::new("."), &url)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            preprocess_tex_delimiters(&preprocess_math_fences(&preprocess_front_matter(&raw)));
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
}
