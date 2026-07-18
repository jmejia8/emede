use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_RECENTS: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentFile {
    pub path: String,
    pub title: String,
    pub opened_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RecentsFile {
    #[serde(default)]
    files: Vec<RecentFile>,
}

fn recents_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("emede")
        .join("recents.json")
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load() -> RecentsFile {
    crate::persist::load_json_or_backup(&recents_path())
}

fn save(file: &RecentsFile) {
    match serde_json::to_string_pretty(file) {
        Ok(json) => {
            if let Err(e) = crate::persist::write_json_atomic(&recents_path(), &json) {
                eprintln!("emede: failed to save recent files: {e}");
            }
        }
        Err(e) => eprintln!("emede: failed to serialize recent files: {e}"),
    }
}

/// Record a successfully opened local document at the front of the recents
/// list, de-duplicating by path and capping the list length.
pub fn add_recent(path: &str, title: &str) {
    let mut file = load();
    file.files.retain(|f| f.path != path);
    file.files.insert(
        0,
        RecentFile {
            path: path.to_string(),
            title: title.to_string(),
            opened_at: now_unix_secs(),
        },
    );
    file.files.truncate(MAX_RECENTS);
    save(&file);
}

/// Return the recent files, dropping (and persisting the removal of) any whose
/// path no longer exists on disk.
#[tauri::command]
pub fn get_recent_files() -> Vec<RecentFile> {
    let mut file = load();
    let before = file.files.len();
    file.files.retain(|f| Path::new(&f.path).exists());
    if file.files.len() != before {
        save(&file);
    }
    file.files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, OnceLock};

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn with_temp_recents(test_fn: impl FnOnce()) {
        let _guard = test_guard();
        let path = recents_path();
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
    fn dedupes_and_moves_to_front() {
        with_temp_recents(|| {
            add_recent("/tmp/a.md", "A");
            add_recent("/tmp/b.md", "B");
            add_recent("/tmp/a.md", "A again");

            let list = load().files;
            assert_eq!(list.len(), 2);
            assert_eq!(list[0].path, "/tmp/a.md");
            assert_eq!(list[0].title, "A again");
            assert_eq!(list[1].path, "/tmp/b.md");
        });
    }

    #[test]
    fn caps_at_max() {
        with_temp_recents(|| {
            for i in 0..(MAX_RECENTS + 5) {
                add_recent(&format!("/tmp/file{i}.md"), &format!("F{i}"));
            }
            let list = load().files;
            assert_eq!(list.len(), MAX_RECENTS);
            // Most recent insert is at the front.
            assert_eq!(list[0].path, format!("/tmp/file{}.md", MAX_RECENTS + 4));
        });
    }

    #[test]
    fn filters_missing_files() {
        with_temp_recents(|| {
            let dir = std::env::temp_dir().join("emede-recents-test");
            let _ = fs::create_dir_all(&dir);
            let real = dir.join("real.md");
            fs::write(&real, "# hi").unwrap();

            add_recent(real.to_str().unwrap(), "Real");
            add_recent("/tmp/definitely-does-not-exist-xyz.md", "Ghost");

            let list = get_recent_files();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].title, "Real");

            let _ = fs::remove_dir_all(&dir);
        });
    }
}
