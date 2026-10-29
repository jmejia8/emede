//! Syntax themes for fenced code blocks.
//!
//! The frontend loads a theme's CSS itself (`src/syntax-themes/` for the
//! curated list, `syntax-auto.css` for Auto), so this module serves the two
//! places it cannot: themes the user imported, which live in the config dir,
//! and the LAN-shared or exported page, which must inline the chosen theme
//! even headless. The curated files are embedded by `build.rs`.
//!
//! A theme's `bg` and `fg` are read from its `.hljs` rule, the same way
//! `scripts/sync-syntax-themes.mjs` writes them into `index.json`: emede moves
//! them onto the `pre` (see `src/syntax.css`).

use serde::Serialize;
use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/syntax_themes.rs"));

/// Seats any theme inside emede's `pre`; always inlined.
pub const LAYOUT_CSS: &str = include_str!("../../src/syntax.css");
/// Token colors derived from the color theme, for `syntax_theme: None`.
pub const AUTO_CSS: &str = include_str!("../../src/syntax-auto.css");

/// Prefix of a `syntax_theme` value naming an imported theme.
const USER_PREFIX: &str = "user:";
/// Same cap as an imported color template (`read_color_template`).
const MAX_THEME_BYTES: u64 = 256 * 1024;
/// Below this contrast against the page, a block's edge is invisible.
const FLUSH_CONTRAST: f64 = 1.1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BaseColors {
    pub bg: String,
    pub fg: String,
}

/// An imported theme, as listed in the settings panel.
#[derive(Debug, Clone, Serialize)]
pub struct UserTheme {
    pub id: String,
    pub label: String,
    pub bg: String,
    pub fg: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserThemeCss {
    pub css: String,
    pub bg: String,
    pub fg: String,
}

/// A theme ready to inline: its CSS, and its colors (`None` for Auto, which
/// keeps the color theme's code background).
pub struct Resolved {
    pub id: Option<String>,
    pub css: Cow<'static, str>,
    pub colors: Option<BaseColors>,
}

pub fn curated_css(id: &str) -> Option<&'static str> {
    CURATED_THEMES
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, css)| *css)
}

/// The theme a `syntax_theme` setting names, or Auto when it is unset, unknown,
/// or unreadable.
pub fn resolve(theme: Option<&str>) -> Resolved {
    let auto = Resolved {
        id: None,
        css: Cow::Borrowed(AUTO_CSS),
        colors: None,
    };
    let Some(id) = theme.filter(|id| !id.is_empty()) else {
        return auto;
    };
    let css: Cow<'static, str> = if let Some(name) = id.strip_prefix(USER_PREFIX) {
        match user_theme_path(name).and_then(|path| read_capped(&path)) {
            Ok(css) => Cow::Owned(css),
            Err(err) => {
                eprintln!("syntax theme {id}: {err}; using Auto");
                return auto;
            }
        }
    } else {
        match curated_css(id) {
            Some(css) => Cow::Borrowed(css),
            None => {
                eprintln!("unknown syntax theme {id}; using Auto");
                return auto;
            }
        }
    };
    match base_colors(&css) {
        Some(colors) => Resolved {
            id: Some(id.to_string()),
            css,
            colors: Some(colors),
        },
        None => auto,
    }
}

/// Whether a block in this theme would be indistinguishable from the page.
pub fn is_flush(theme_bg: &str, page_bg: &str) -> bool {
    match (parse_color(theme_bg), parse_color(page_bg)) {
        (Some(a), Some(b)) => contrast(a, b) < FLUSH_CONTRAST,
        _ => false,
    }
}

/// `bg` and `fg` from a theme's `.hljs` / `code.hljs` rules (the last of each
/// wins, as in the cascade). `None` unless both are plain opaque colors.
pub fn base_colors(css: &str) -> Option<BaseColors> {
    let mut bg = None;
    let mut fg = None;
    for (selectors, body) in top_level_rules(css) {
        if !selectors
            .split(',')
            .map(str::trim)
            .any(|s| s == ".hljs" || s == "code.hljs")
        {
            continue;
        }
        for declaration in body.split(';') {
            let Some((property, value)) = declaration.split_once(':') else {
                continue;
            };
            let Some(color) = parse_color(value) else {
                continue;
            };
            match property.trim().to_ascii_lowercase().as_str() {
                "background" | "background-color" => bg = Some(to_hex(color)),
                "color" => fg = Some(to_hex(color)),
                _ => {}
            }
        }
    }
    Some(BaseColors { bg: bg?, fg: fg? })
}

/// `(selectors, declarations)` of each top-level rule; comments and at-rules
/// (`@media` blocks) are skipped.
fn top_level_rules(css: &str) -> Vec<(String, String)> {
    let mut text = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        text.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    text.push_str(rest);

    let mut rules = Vec::new();
    let mut depth = 0usize;
    let mut selector_start = 0;
    let mut body_start = 0;
    let mut selector = "";
    for (i, ch) in text.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    selector = text[selector_start..i].trim();
                    body_start = i + 1;
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if !selector.starts_with('@') {
                        rules.push((selector.to_string(), text[body_start..i].to_string()));
                    }
                    selector_start = i + 1;
                }
            }
            _ => {}
        }
    }
    rules
}

fn parse_color(value: &str) -> Option<[u8; 3]> {
    let v = value.trim().to_ascii_lowercase();
    let named = match v.as_str() {
        "white" => Some([255, 255, 255]),
        "black" => Some([0, 0, 0]),
        "navy" => Some([0, 0, 128]),
        "gray" | "grey" => Some([128, 128, 128]),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    if let Some(hex) = v.strip_prefix('#') {
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |s: &str| u8::from_str_radix(s, 16).ok();
        return match hex.len() {
            3 => {
                let mut rgb = [0; 3];
                for (i, c) in hex.chars().enumerate() {
                    rgb[i] = channel(&format!("{c}{c}"))?;
                }
                Some(rgb)
            }
            6 => Some([
                channel(&hex[0..2])?,
                channel(&hex[2..4])?,
                channel(&hex[4..6])?,
            ]),
            _ => None,
        };
    }
    let inner = v.strip_prefix("rgb(")?.strip_suffix(')')?;
    let parts: Vec<u8> = inner
        .split(',')
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    <[u8; 3]>::try_from(parts).ok()
}

fn to_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn luminance(rgb: [u8; 3]) -> f64 {
    let [r, g, b] = rgb.map(|c| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG contrast ratio.
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn user_themes_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("emede")
        .join("syntax-themes")
}

/// A file name for an imported theme: its stem, with anything outside a
/// conservative set replaced, so a name can never leave the themes dir.
fn sanitize_name(stem: &str) -> String {
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    if cleaned.is_empty() {
        "theme".into()
    } else {
        cleaned.into()
    }
}

fn user_theme_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || sanitize_name(name) != name {
        return Err(format!("invalid syntax theme name {name:?}"));
    }
    Ok(user_themes_dir().join(format!("{name}.css")))
}

fn read_capped(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.len() > MAX_THEME_BYTES {
        return Err("syntax theme is too large".into());
    }
    fs::read_to_string(path).map_err(|e| e.to_string())
}

fn no_base_colors() -> String {
    "This CSS has no .hljs rule with a plain background and color, so it is not a \
     highlight.js theme emede can use."
        .into()
}

/// Copy a highlight.js theme into the config dir, replacing one of the same
/// name, and return it as listed.
#[tauri::command]
pub fn import_syntax_theme(path: String) -> Result<UserTheme, String> {
    let source = PathBuf::from(&path);
    let css = read_capped(&source)?;
    let colors = base_colors(&css).ok_or_else(no_base_colors)?;
    let name = sanitize_name(
        &source
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let dir = user_themes_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(format!("{name}.css")), &css).map_err(|e| e.to_string())?;
    Ok(UserTheme {
        id: format!("{USER_PREFIX}{name}"),
        label: name,
        bg: colors.bg,
        fg: colors.fg,
    })
}

/// Imported themes, by name. Files that are not usable themes are left out.
#[tauri::command]
pub fn list_user_syntax_themes() -> Vec<UserTheme> {
    let Ok(entries) = fs::read_dir(user_themes_dir()) else {
        return Vec::new();
    };
    let mut themes: Vec<UserTheme> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "css" {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            let colors = base_colors(&read_capped(&user_theme_path(&name).ok()?).ok()?)?;
            Some(UserTheme {
                id: format!("{USER_PREFIX}{name}"),
                label: name,
                bg: colors.bg,
                fg: colors.fg,
            })
        })
        .collect();
    themes.sort_by_key(|theme| theme.label.to_lowercase());
    themes
}

/// The CSS and colors of an imported theme, by its `user:<name>` id.
#[tauri::command]
pub fn read_user_syntax_theme(id: String) -> Result<UserThemeCss, String> {
    let name = id
        .strip_prefix(USER_PREFIX)
        .ok_or_else(|| format!("not an imported syntax theme: {id}"))?;
    let css = read_capped(&user_theme_path(name)?)?;
    let colors = base_colors(&css).ok_or_else(no_base_colors)?;
    Ok(UserThemeCss {
        css,
        bg: colors.bg,
        fg: colors.fg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct IndexEntry {
        id: String,
        bg: String,
        fg: String,
    }

    fn index() -> Vec<IndexEntry> {
        serde_json::from_str(include_str!("../../src/syntax-themes/index.json")).unwrap()
    }

    #[test]
    fn every_indexed_theme_is_embedded_with_matching_colors() {
        let index = index();
        assert_eq!(index.len(), 30, "the curated list has 30 themes");
        for entry in &index {
            let css = curated_css(&entry.id)
                .unwrap_or_else(|| panic!("{} is indexed but not embedded", entry.id));
            assert_eq!(
                base_colors(css),
                Some(BaseColors {
                    bg: entry.bg.clone(),
                    fg: entry.fg.clone()
                }),
                "{}: Rust and the sync script disagree on its colors",
                entry.id
            );
        }
        assert_eq!(CURATED_THEMES.len(), index.len(), "an embedded theme is not indexed");
    }

    #[test]
    fn base_colors_reads_hljs_rules_only() {
        let css = "/* .hljs { background: red } */\n\
                   pre code.hljs { display: block; padding: 1em }\n\
                   .hljs { background: white; }\n\
                   .hljs, .hljs-subst { color: #abc }\n\
                   .hljs-keyword { color: #123456 }\n\
                   @media print { .hljs { background: #000 } }";
        assert_eq!(
            base_colors(css),
            Some(BaseColors {
                bg: "#ffffff".into(),
                fg: "#aabbcc".into()
            })
        );
        assert_eq!(base_colors(".hljs-keyword { color: red }"), None);
        assert_eq!(base_colors(".hljs { background: var(--x); color: #000 }"), None);
    }

    #[test]
    fn resolve_falls_back_to_auto() {
        let auto = resolve(None);
        assert!(auto.colors.is_none() && auto.css.contains("--syntax-keyword"));
        assert!(resolve(Some("no-such-theme")).colors.is_none());
        assert!(resolve(Some("user:../settings")).colors.is_none());

        let dracula = resolve(Some("base16/dracula"));
        assert_eq!(dracula.id.as_deref(), Some("base16/dracula"));
        assert_eq!(dracula.colors.unwrap().bg, "#282936");
    }

    #[test]
    fn flush_only_when_the_backgrounds_match() {
        // Dracula preset page vs base16/dracula; Light page vs stackoverflow-light.
        assert!(is_flush("#282936", "#282a37"));
        assert!(is_flush("#1e1e2e", "#1e1e2e"));
        assert!(!is_flush("#f6f6f6", "#000000"));
        assert!(!is_flush("#2b2b2b", "#ffffff"));
        assert!(!is_flush("#2b2b2b", "not a color"));
    }

    #[test]
    fn user_theme_names_stay_inside_the_themes_dir() {
        assert_eq!(sanitize_name("my theme.v2"), "my theme.v2");
        assert_eq!(sanitize_name("../../etc/passwd"), "-..-etc-passwd");
        assert_eq!(sanitize_name("..."), "theme");
        assert!(user_theme_path("../x").is_err());
        assert!(user_theme_path("a/b").is_err());
        assert!(user_theme_path("").is_err());
        assert!(user_theme_path("Solarized").is_ok());
    }
}
