use std::path::{Path, PathBuf};

fn main() {
    embed_syntax_themes();
    tauri_build::build()
}

/// Write `$OUT_DIR/syntax_themes.rs`: every curated theme in
/// `src/syntax-themes/` as `(id, css)`, so `--share` and `--export` can inline
/// the chosen one headless. The id is the path without `.css`, which is what
/// `index.json` and `settings.json` use (`base16/dracula`).
fn embed_syntax_themes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/syntax-themes");
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut files = Vec::new();
    collect_css(&dir, &mut files);
    files.sort();

    let mut table = String::from("pub static CURATED_THEMES: &[(&str, &str)] = &[\n");
    for file in &files {
        let id = file
            .strip_prefix(&dir)
            .unwrap()
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        let path = file.canonicalize().unwrap();
        table.push_str(&format!("    ({id:?}, include_str!({:?})),\n", path));
    }
    table.push_str("];\n");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("syntax_themes.rs");
    std::fs::write(out, table).unwrap();
}

fn collect_css(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_css(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "css") {
            files.push(path);
        }
    }
}
