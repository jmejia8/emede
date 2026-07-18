use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

/// How long to keep draining filesystem events after the first one before
/// re-rendering, so a burst of writes (editors often write in several steps)
/// collapses into a single reload.
const DEBOUNCE: Duration = Duration::from_millis(200);

pub struct WatcherState(pub Mutex<Option<ActiveWatch>>);

impl Default for WatcherState {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

pub struct ActiveWatch {
    /// Canonicalized path currently being watched (used to dedupe re-arm calls).
    path: PathBuf,
    /// Dropping the watcher stops notify; that closes the event channel, which
    /// makes the debounce thread exit on its own.
    _watcher: RecommendedWatcher,
}

/// Start watching `path` for changes and emit `document-updated` (a rendered
/// [`crate::markdown::RenderResult`]) when it changes. Watches the parent
/// directory so editor save patterns (rename-then-replace, delete-then-create)
/// still trigger without any re-arming.
#[tauri::command]
pub fn watch_document(
    path: String,
    app: AppHandle,
    state: State<WatcherState>,
) -> Result<(), String> {
    let raw = PathBuf::from(&path);
    let target = raw.canonicalize().unwrap_or(raw);

    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if guard.as_ref().is_some_and(|active| active.path == target) {
        return Ok(());
    }
    // Drop any previous watch (stops its notify watcher and debounce thread).
    *guard = None;

    let parent = target
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot watch a path with no parent directory".to_string())?;
    let file_name = target
        .file_name()
        .map(OsStr::to_os_string)
        .ok_or_else(|| "invalid file path".to_string())?;

    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .map_err(|e| e.to_string())?;
    watcher
        .watch(&parent, RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;

    // Render/emit using the exact path string the frontend opened with, so the
    // emitted `path` matches `currentDocPath` there even when it differs from
    // the canonical form (relative paths, symlinks).
    let thread_app = app.clone();
    std::thread::spawn(move || run_watch_loop(rx, thread_app, path, file_name));

    *guard = Some(ActiveWatch {
        path: target,
        _watcher: watcher,
    });
    Ok(())
}

/// Stop watching the current document, if any.
#[tauri::command]
pub fn unwatch_document(state: State<WatcherState>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    *guard = None;
    Ok(())
}

fn event_touches_file(res: &notify::Result<Event>, file_name: &OsStr) -> bool {
    let Ok(event) = res else {
        return false;
    };
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return false;
    }
    event
        .paths
        .iter()
        .any(|p| p.file_name() == Some(file_name))
}

fn run_watch_loop(
    rx: Receiver<notify::Result<Event>>,
    app: AppHandle,
    render_path: String,
    file_name: OsString,
) {
    loop {
        // Block until an event arrives or the channel closes (watcher dropped).
        let first = match rx.recv() {
            Ok(ev) => ev,
            Err(_) => return,
        };
        let mut relevant = event_touches_file(&first, &file_name);

        // Debounce: keep draining until a quiet window elapses.
        loop {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(ev) => relevant |= event_touches_file(&ev, &file_name),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    if relevant {
                        emit_update(&app, &render_path);
                    }
                    return;
                }
            }
        }

        if relevant {
            emit_update(&app, &render_path);
        }
    }
}

/// Re-render the watched file and emit the result. Retries once after a short
/// delay to ride out the brief window where an editor has removed the file but
/// not yet written the replacement.
fn emit_update(app: &AppHandle, render_path: &str) {
    for attempt in 0..2 {
        match crate::markdown::render_markdown_inner(render_path) {
            Ok(result) => {
                let _ = app.emit("document-updated", &result);
                return;
            }
            Err(_) if attempt == 0 => std::thread::sleep(Duration::from_millis(150)),
            Err(_) => return,
        }
    }
}
