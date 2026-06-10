use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_DOCUMENTS: usize = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocViewState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_id: Option<String>,
    #[serde(default)]
    pub anchor_offset: f64,
    #[serde(default)]
    pub scroll_top: f64,
    #[serde(default)]
    pub scroll_fraction: f64,
    #[serde(default)]
    pub updated_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ViewStateFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    documents: HashMap<String, DocViewState>,
}

fn default_version() -> u32 {
    1
}

fn view_state_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("emede")
        .join("view_state.json")
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load_view_state_file() -> ViewStateFile {
    let path = view_state_path();
    if path.exists() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| ViewStateFile {
                version: default_version(),
                documents: HashMap::new(),
            })
    } else {
        ViewStateFile {
            version: default_version(),
            documents: HashMap::new(),
        }
    }
}

fn prune_documents(documents: &mut HashMap<String, DocViewState>) {
    if documents.len() <= MAX_DOCUMENTS {
        return;
    }

    let mut entries: Vec<(String, u64)> = documents
        .iter()
        .map(|(path, state)| (path.clone(), state.updated_at))
        .collect();
    entries.sort_by_key(|(_, updated_at)| *updated_at);

    let remove_count = documents.len() - MAX_DOCUMENTS;
    for (path, _) in entries.into_iter().take(remove_count) {
        documents.remove(&path);
    }
}

fn save_view_state_file(file: &ViewStateFile) -> Result<(), String> {
    let path = view_state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_view_state(path: String) -> Option<DocViewState> {
    let file = load_view_state_file();
    file.documents.get(&path).cloned()
}

#[tauri::command]
pub fn set_view_state(path: String, state: DocViewState) -> Result<(), String> {
    let mut file = load_view_state_file();
    let mut next = state;
    next.updated_at = now_unix_secs();
    file.documents.insert(path, next);
    prune_documents(&mut file.documents);
    save_view_state_file(&file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn with_temp_view_state_file(test_fn: impl FnOnce()) {
        let _guard = test_guard();
        let path = view_state_path();
        let backup = path.exists().then(|| fs::read(&path).ok()).flatten();

        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::remove_file(&path);

        test_fn();

        if let Some(bytes) = backup {
            let _ = fs::write(&path, bytes);
        } else {
            let _ = fs::remove_file(&path);
        }
    }

    #[test]
    fn stores_and_retrieves_view_state_by_path() {
        with_temp_view_state_file(|| {
            let state = DocViewState {
                anchor_id: Some("intro".into()),
                anchor_offset: 120.0,
                scroll_top: 840.0,
                scroll_fraction: 0.35,
                updated_at: 0,
            };

            set_view_state("/tmp/a.md".into(), state.clone()).expect("save a");
            set_view_state(
                "/tmp/b.md".into(),
                DocViewState {
                    anchor_id: Some("chapter-2".into()),
                    anchor_offset: 40.0,
                    scroll_top: 2100.0,
                    scroll_fraction: 0.81,
                    updated_at: 0,
                },
            )
            .expect("save b");

            let loaded_a = get_view_state("/tmp/a.md".into()).expect("load a");
            assert_eq!(loaded_a.anchor_id.as_deref(), Some("intro"));
            assert!((loaded_a.anchor_offset - 120.0).abs() < f64::EPSILON);
            assert!((loaded_a.scroll_top - 840.0).abs() < f64::EPSILON);
            assert!((loaded_a.scroll_fraction - 0.35).abs() < f64::EPSILON);
            assert!(loaded_a.updated_at > 0);

            let loaded_b = get_view_state("/tmp/b.md".into()).expect("load b");
            assert_eq!(loaded_b.anchor_id.as_deref(), Some("chapter-2"));
            assert!((loaded_b.scroll_fraction - 0.81).abs() < f64::EPSILON);
        });
    }

    #[test]
    fn updates_existing_entry() {
        with_temp_view_state_file(|| {
            set_view_state(
                "/tmp/doc.md".into(),
                DocViewState {
                    anchor_id: None,
                    anchor_offset: 0.0,
                    scroll_top: 100.0,
                    scroll_fraction: 0.1,
                    updated_at: 0,
                },
            )
            .expect("initial save");

            let first = get_view_state("/tmp/doc.md".into()).expect("first load");
            set_view_state(
                "/tmp/doc.md".into(),
                DocViewState {
                    anchor_id: Some("section".into()),
                    anchor_offset: 24.0,
                    scroll_top: 500.0,
                    scroll_fraction: 0.5,
                    updated_at: 0,
                },
            )
            .expect("update save");

            let updated = get_view_state("/tmp/doc.md".into()).expect("updated load");
            assert_eq!(updated.anchor_id.as_deref(), Some("section"));
            assert!((updated.scroll_top - 500.0).abs() < f64::EPSILON);
            assert!(updated.updated_at >= first.updated_at);
        });
    }
}
