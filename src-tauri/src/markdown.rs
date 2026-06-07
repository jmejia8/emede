use comrak::create_formatter;
use comrak::html::ChildRendering;
use comrak::nodes::NodeValue;
use comrak::{parse_document, Arena, Options};
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

use crate::watcher::WatcherState;

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

fn title_from_markdown(content: &str, path: &Path) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }

    title_from_path(path)
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
                for expected in fence_marker.chars() {
                    match chars.peek() {
                        Some(&c) if c == expected => {
                            chars.next();
                        }
                        _ => {
                            matched = false;
                            break;
                        }
                    }
                }
                if matched && !fence_marker.is_empty() {
                    in_fence = false;
                    fence_marker.clear();
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
            let mut marker = String::from("`");
            while chars.peek() == Some(&'`') {
                chars.next();
                marker.push('`');
            }

            if marker.len() >= 3 {
                if let Some(&next) = chars.peek() {
                    if next == '\n' || next == '\r' {
                        in_fence = true;
                        fence_marker = marker.clone();
                        out.push_str(&marker);
                        continue;
                    }
                }
            }

            out.push('\n');
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
    let with_fences = preprocess_math_fences(&raw);
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
pub fn render_markdown(
    path: String,
    app: AppHandle,
    watcher: State<WatcherState>,
) -> Result<RenderResult, String> {
    let result = render_markdown_inner(&path)?;
    crate::watcher::watch_file(app, PathBuf::from(&result.path), watcher)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

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
