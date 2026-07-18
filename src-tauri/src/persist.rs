use serde::de::DeserializeOwned;
use std::fs;
use std::path::Path;

/// Write `json` to `path` atomically: write to a temporary file in the same
/// directory, then rename it into place. Same-directory rename is atomic on
/// every mainstream filesystem, so a reader never sees a half-written file and
/// an interrupted write leaves the previous contents intact.
///
/// The temp file name carries the current pid so that multiple emede processes
/// writing the same shared file (e.g. `active_shares.json`) never collide on
/// the temporary path. Concurrent writers still race on the final rename, which
/// resolves to last-writer-wins — acceptable for these config files.
pub fn write_json_atomic(path: &Path, json: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "invalid config path".to_string())?;
    let tmp = path.with_file_name(format!("{file_name}.tmp.{}", std::process::id()));

    if let Err(e) = fs::write(&tmp, json) {
        let _ = fs::remove_file(&tmp);
        return Err(e.to_string());
    }

    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.to_string());
    }

    Ok(())
}

/// Load and deserialize JSON from `path`, tolerating both absence and
/// corruption. A missing or unreadable file yields `T::default()`. A file that
/// exists but fails to parse is first copied to `{path}.corrupt` (so the user's
/// data is not silently destroyed on the next save) and then falls back to
/// `T::default()`.
pub fn load_json_or_backup<T: DeserializeOwned + Default>(path: &Path) -> T {
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return T::default(),
    };

    match serde_json::from_str(&contents) {
        Ok(value) => value,
        Err(e) => {
            let backup = path.with_extension(format!(
                "{}.corrupt",
                path.extension().and_then(|s| s.to_str()).unwrap_or("json")
            ));
            eprintln!(
                "emede: failed to parse {} ({e}); backing up to {} and using defaults",
                path.display(),
                backup.display()
            );
            let _ = fs::write(&backup, contents);
            T::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        count: u32,
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("emede-persist-test-{}-{}", std::process::id(), name))
    }

    #[test]
    fn atomic_write_roundtrips() {
        let path = temp_path("roundtrip.json");
        let _ = fs::remove_file(&path);

        let value = Sample {
            name: "hello".into(),
            count: 7,
        };
        let json = serde_json::to_string_pretty(&value).unwrap();
        write_json_atomic(&path, &json).expect("write");

        let loaded: Sample = load_json_or_backup(&path);
        assert_eq!(loaded, value);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn missing_file_yields_default() {
        let path = temp_path("does-not-exist.json");
        let _ = fs::remove_file(&path);
        let loaded: Sample = load_json_or_backup(&path);
        assert_eq!(loaded, Sample::default());
    }

    #[test]
    fn corrupt_file_backs_up_and_defaults() {
        let path = temp_path("corrupt.json");
        let backup = path.with_extension("json.corrupt");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&backup);

        fs::write(&path, "{ this is not json").unwrap();

        let loaded: Sample = load_json_or_backup(&path);
        assert_eq!(loaded, Sample::default());
        assert!(backup.exists(), "corrupt file should be backed up");
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "{ this is not json"
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&backup);
    }
}
