use crate::markdown::render_markdown_inner;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use notify_debouncer_full::notify::{RecommendedWatcher, RecursiveMode};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub struct WatcherState(pub Mutex<Option<Debouncer<RecommendedWatcher, RecommendedCache>>>);

fn paths_match_event(watched: &Path, event_paths: &[PathBuf]) -> bool {
    event_paths.iter().any(|p| p == watched)
}

pub fn watch_file(
    app: AppHandle,
    path: PathBuf,
    state: tauri::State<WatcherState>,
) -> Result<(), String> {
    let watch_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let watched_path = path.clone();
    let app_handle = app.clone();

    let debouncer = new_debouncer(
        Duration::from_millis(300),
        None,
        move |result: DebounceEventResult| {
            let Ok(events) = result else {
                return;
            };

            let relevant = events.iter().any(|event| {
                paths_match_event(&watched_path, &event.paths)
            });

            if !relevant {
                return;
            }

            let path_str = watched_path.to_string_lossy().into_owned();
            match render_markdown_inner(&path_str) {
                Ok(rendered) => {
                    let _ = app_handle.emit("document-updated", rendered);
                }
                Err(err) => {
                    let _ = app_handle.emit("document-error", err);
                }
            }
        },
    )
    .map_err(|e| format!("Failed to create file watcher: {e}"))?;

    let mut debouncer = debouncer;
    debouncer
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch file: {e}"))?;

    if let Ok(mut slot) = state.0.lock() {
        *slot = Some(debouncer);
    }

    Ok(())
}
