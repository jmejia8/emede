use comrak::create_formatter;
use comrak::html::ChildRendering;
use comrak::nodes::NodeValue;
use comrak::{parse_document, Arena, Options};
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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

fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string()
}

fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{FEFF}').unwrap_or(s)
}

fn split_front_matter(content: &str) -> Option<(&str, &str, &str)> {
    let content = strip_bom(content);
    let mut pos = 0;
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

/// Convert pandoc-style `\(...\)` / `\[...\]` to Comrak `$...$` / `$$...$$`,
/// skipping fenced and inline code.
fn preprocess_tex_delimiters(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_fence = false;
    let mut fence_marker = String::new();
    let mut inline_code = false;

    while let Some(ch) = chars.next() {
        if in_fence {
            out.push(ch);
            if ch == '\n' {
                let mut matched = true;
                let mut closing = String::new();
                for expected in fence_marker.chars() {
                    match chars.peek() {
                        Some(&c) if c == expected => {
                            chars.next();
                            closing.push(c);
                        }
                        _ => {
                            matched = false;
                            break;
                        }
                    }
                }
                if matched && !fence_marker.is_empty() {
                    while matches!(chars.peek(), Some(' ') | Some('\t')) {
                        closing.push(chars.next().unwrap());
                    }
                    if matches!(chars.peek(), Some('\n') | Some('\r') | None) {
                        out.push_str(&closing);
                        in_fence = false;
                        fence_marker.clear();
                    }
                }
            }
            continue;
        }

        if inline_code {
            out.push(ch);
            if ch == '`' {
                inline_code = false;
            }
            continue;
        }

        if ch == '`' {
            inline_code = true;
            out.push(ch);
            continue;
        }

        if ch == '\n' {
            out.push('\n');
            let mut marker = String::new();
            while chars.peek() == Some(&'`') {
                chars.next();
                marker.push('`');
            }

            if marker.len() >= 3 {
                let mut info = String::new();
                while let Some(&next) = chars.peek() {
                    if next == '\n' || next == '\r' {
                        break;
                    }
                    info.push(chars.next().unwrap());
                }

                if matches!(chars.peek(), Some('\n') | Some('\r')) {
                    in_fence = true;
                    fence_marker = marker.clone();
                    out.push_str(&marker);
                    out.push_str(&info);
                    continue;
                }

                out.push_str(&marker);
                out.push_str(&info);
            } else {
                out.push_str(&marker);
            }
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

fn comrak_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.math_dollars = true;
    options.extension.math_code = true;
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.header_id_prefix = Some(String::new());
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
    let options = comrak_options();
    let root = parse_document(&arena, &preprocessed, &options);

    let mut html = String::new();
    MathJaxFormatter::format_document(root, &options, &mut html)
        .map_err(|e| format!("Failed to render markdown: {e}"))?;

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
            !result.html.contains("<h1># development</h1>") && !result.html.contains("<h1>development</h1>"),
            "comment in code block became a heading; html snippet: {}",
            &result.html[..result.html.len().min(3000)]
        );
    }

    #[test]
    fn renders_fenced_code_blocks() {
        let src = "```bash\n# development\nnpm install\n```\n";
        let preprocessed = preprocess_tex_delimiters(&preprocess_math_fences(src));
        let arena = Arena::new();
        let options = comrak_options();
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
            let options = comrak_options();
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
        let preprocessed = preprocess_tex_delimiters(&preprocess_math_fences(&preprocess_front_matter(
            src,
        )));
        let arena = Arena::new();
        let options = comrak_options();
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
    fn renders_math_delimiters() {
        let html = {
            let src = "Inline $E=mc^2$ and display:\n\n$$x^2$$\n";
            let preprocessed = preprocess_tex_delimiters(&preprocess_math_fences(src));
            let arena = Arena::new();
            let options = comrak_options();
            let root = parse_document(&arena, &preprocessed, &options);
            let mut html = String::new();
            MathJaxFormatter::format_document(root, &options, &mut html).unwrap();
            html
        };
        assert!(html.contains("$E=mc^2$"));
        assert!(html.contains("$$\nx^2\n$$"));
    }
}
