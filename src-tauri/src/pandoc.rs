use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Serialize)]
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

fn title_from_markdown(path: &Path) -> String {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return title_from_path(path),
    };

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

fn pandoc_available() -> bool {
    Command::new("pandoc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tauri::command]
pub fn render_markdown(path: String) -> Result<RenderResult, String> {
    let resolved = resolve_path(&path);
    if !resolved.exists() {
        return Err(format!("File not found: {}", resolved.display()));
    }
    if !resolved.is_file() {
        return Err(format!("Not a file: {}", resolved.display()));
    }

    if !pandoc_available() {
        return Err("pandoc not found on PATH. Install it with: pacman -S pandoc".into());
    }

    let output = Command::new("pandoc")
        .arg(resolved.as_os_str())
        .args([
            "-f",
            "markdown+tex_math_dollars+tex_math_single_backslash",
            "-t",
            "html5",
            "--no-highlight",
            "--mathjax",
        ])
        .output()
        .map_err(|e| format!("Failed to run pandoc: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("pandoc failed: {stderr}"));
    }

    let html = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let title = title_from_markdown(&resolved);

    Ok(RenderResult {
        html,
        title,
        path: resolved.to_string_lossy().into_owned(),
    })
}
