//! Color templates the user imported, kept as files in
//! `~/.config/emede/color-templates/` and keyed by file name, the same way
//! `syntax_themes` keeps imported code themes. So they survive a restart, and
//! importing a file again replaces the earlier copy.
//!
//! Only the frontend parses a template (`extractColorTemplate`), so a listing
//! returns the raw CSS and the frontend skips anything it cannot read.

use crate::settings::read_color_template;
use crate::syntax_themes::{remove_user_css, sanitize_name, user_css_path, USER_PREFIX};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct UserColorTemplate {
    pub id: String,
    pub label: String,
    pub css: String,
}

fn templates_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("emede")
        .join("color-templates")
}

fn entry(name: String, css: String) -> UserColorTemplate {
    UserColorTemplate {
        id: format!("{USER_PREFIX}{name}"),
        label: name,
        css,
    }
}

fn import_into(dir: &Path, source: &Path) -> Result<UserColorTemplate, String> {
    let css = read_color_template(source.to_string_lossy().into_owned())?;
    let name = sanitize_name(
        &source
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(format!("{name}.css")), &css).map_err(|e| e.to_string())?;
    Ok(entry(name, css))
}

fn list_in(dir: &Path) -> Vec<UserColorTemplate> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut templates: Vec<UserColorTemplate> = entries
        .flatten()
        .filter_map(|file| {
            let path = file.path();
            if path.extension()? != "css" {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            let safe = user_css_path(dir, &name).ok()?;
            let css = read_color_template(safe.to_string_lossy().into_owned()).ok()?;
            Some(entry(name, css))
        })
        .collect();
    templates.sort_by_key(|template| template.label.to_lowercase());
    templates
}

fn delete_in(dir: &Path, id: &str) -> Result<(), String> {
    let name = id
        .strip_prefix(USER_PREFIX)
        .ok_or_else(|| format!("not an imported color template: {id}"))?;
    remove_user_css(&user_css_path(dir, name)?)
}

/// Copy a template into the config dir, replacing one of the same name, and
/// return it as listed.
#[tauri::command]
pub fn import_color_template(path: String) -> Result<UserColorTemplate, String> {
    import_into(&templates_dir(), Path::new(&path))
}

/// Imported templates, by name.
#[tauri::command]
pub fn list_user_color_templates() -> Vec<UserColorTemplate> {
    list_in(&templates_dir())
}

/// Delete an imported template by its `user:<name>` id.
#[tauri::command]
pub fn delete_user_color_template(id: String) -> Result<(), String> {
    delete_in(&templates_dir(), &id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("emede-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn importing_a_file_twice_keeps_one_entry_with_the_latest_css() {
        let root = temp_dir("template-reimport");
        let source = root.join("My Theme.css");
        let store = root.join("store");

        fs::write(&source, ":root { --color-fg: #000; }").unwrap();
        let first = import_into(&store, &source).unwrap();
        assert_eq!(first.id, "user:My Theme");
        assert_eq!(first.label, "My Theme");

        fs::write(&source, ":root { --color-fg: #111; }").unwrap();
        import_into(&store, &source).unwrap();

        let listed = list_in(&store);
        assert_eq!(listed.len(), 1);
        assert!(listed[0].css.contains("#111"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn listing_skips_what_is_not_a_usable_template_file() {
        let store = temp_dir("template-list");
        fs::write(store.join("b.css"), "b").unwrap();
        fs::write(store.join("A.css"), "a").unwrap();
        fs::write(store.join("notes.txt"), "not css").unwrap();
        fs::write(store.join("bad$name.css"), "name a user could not import").unwrap();

        let labels: Vec<String> = list_in(&store).into_iter().map(|t| t.label).collect();
        assert_eq!(labels, ["A", "b"]);
        assert!(list_in(&store.join("missing")).is_empty());
        fs::remove_dir_all(&store).unwrap();
    }

    #[test]
    fn delete_removes_the_file_and_tolerates_one_already_gone() {
        let store = temp_dir("template-delete");
        fs::write(store.join("a.css"), "a").unwrap();

        delete_in(&store, "user:a").unwrap();
        assert!(!store.join("a.css").exists());
        delete_in(&store, "user:a").unwrap();
        fs::remove_dir_all(&store).unwrap();
    }

    #[test]
    fn delete_cannot_leave_the_templates_dir() {
        let root = temp_dir("template-escape");
        let store = root.join("store");
        fs::create_dir_all(&store).unwrap();
        let outside = root.join("outside.css");
        fs::write(&outside, "keep me").unwrap();

        assert!(delete_in(&store, "user:../outside").is_err());
        assert!(delete_in(&store, "outside").is_err());
        assert!(delete_in(&store, "user:").is_err());
        assert!(outside.exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
