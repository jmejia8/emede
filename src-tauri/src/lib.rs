mod markdown;
mod persist;
mod recents;
mod settings;
mod share;
mod view_state;
mod watcher;

use std::sync::Mutex;
use tauri::{window::Color, AppHandle, Emitter, Manager, Url};
use tauri_plugin_window_state::StateFlags;
use tauri_plugin_cli::CliExt;
use tauri_plugin_opener::OpenerExt;

struct StartupFile(Mutex<Option<String>>);

fn is_app_url(url: &Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => matches!(
            url.host_str(),
            Some("tauri.localhost") | Some("localhost") | Some("127.0.0.1")
        ),
        _ => false,
    }
}

fn is_app_entry(url: &Url) -> bool {
    matches!(url.path(), "" | "/" | "/index.html")
}

fn handle_navigation(app: &AppHandle, url: &Url) -> bool {
    let scheme = url.scheme();

    if matches!(scheme, "http" | "https" | "mailto" | "tel") {
        if is_app_url(url) {
            return is_app_entry(url);
        }
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
        return false;
    }

    if scheme == "file" {
        // Don't hand arbitrary local paths to the OS opener. If a link points at
        // a markdown file, open it inside emede; otherwise block it entirely.
        let path = url.path();
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".md") || lower.ends_with(".markdown") {
            let _ = app.emit("file-to-open", path.to_string());
        }
        return false;
    }

    if is_app_url(url) {
        return is_app_entry(url);
    }

    scheme == "tauri" || scheme == "data"
}

pub fn apply_gpu_setting() {
    #[cfg(target_os = "linux")]
    {
        let settings = settings::load_settings();
        if !settings.gpu_acceleration {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::new()
                // Keep visibility under frontend control so the window stays hidden
                // until the first frame is painted.
                .with_state_flags(
                    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .plugin(tauri_plugin_cli::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("external-links")
                .on_navigation(|webview, url| handle_navigation(webview.app_handle(), url))
                .build(),
        )
        .manage(StartupFile(Mutex::new(None)))
        .manage(share::ShareState(Mutex::new(share::ShareStateInner::default())))
        .manage(watcher::WatcherState::default())
        .invoke_handler(tauri::generate_handler![
            markdown::render_markdown,
            markdown::render_markdown_url,
            settings::get_settings,
            settings::set_settings,
            settings::read_color_template,
            view_state::get_view_state,
            view_state::set_view_state,
            share::start_share,
            share::stop_share,
            share::stop_share_note,
            share::get_share_status,
            share::get_note_share_info,
            share::generate_share_qr,
            watcher::watch_document,
            watcher::unwatch_document,
            recents::get_recent_files,
            get_startup_file,
            get_app_version,
            restart_app,
        ])
        .setup(|app| {
            capture_cli_file(app.handle());

            if let Some(window) = app.get_webview_window("main") {
                let settings = settings::load_settings();
                let use_system_frame = settings.window_frame == "system";
                let _ = window.set_decorations(use_system_frame);
                if let Some(color) = hex_to_color(&settings.color_bg) {
                    let _ = window.set_background_color(Some(color));
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
fn get_startup_file(state: tauri::State<StartupFile>) -> Option<String> {
    state.0.lock().ok()?.clone()
}

#[tauri::command]
fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn restart_app() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to get current exe path: {e}"))?;
    let args: Vec<String> = std::env::args().collect();
    std::process::Command::new(exe)
        .args(args.iter().skip(1))
        .spawn()
        .map_err(|e| format!("failed to restart app: {e}"))?;
    std::process::exit(0);
}

fn hex_to_color(hex: &str) -> Option<Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color(r, g, b, 255))
}

fn file_arg_path(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(path) if !path.is_empty() => Some(path.clone()),
        serde_json::Value::Array(parts) => {
            let segments: Vec<&str> = parts.iter().filter_map(|part| part.as_str()).collect();
            if segments.is_empty() {
                None
            } else {
                Some(segments.join(" "))
            }
        }
        _ => None,
    }
}

fn capture_cli_file(app: &AppHandle) {
    let matches = match app.cli().matches() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("CLI parse error: {e}");
            return;
        }
    };

    if let Some(arg) = matches.args.get("file") {
        if let Some(path) = file_arg_path(&arg.value) {
            if let Ok(mut slot) = app.state::<StartupFile>().0.lock() {
                *slot = Some(path.clone());
            }
            let _ = app.emit("file-to-open", path);
        }
    }
}
