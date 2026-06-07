mod pandoc;
mod settings;

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Url, window::Color};
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
        let _ = app.opener().open_path(url.path(), None::<&str>);
        return false;
    }

    if is_app_url(url) {
        return is_app_entry(url);
    }

    scheme == "tauri" || scheme == "data"
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_cli::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("external-links")
                .on_navigation(|webview, url| handle_navigation(webview.app_handle(), url))
                .build(),
        )
        .manage(StartupFile(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            pandoc::render_markdown,
            settings::get_settings,
            settings::set_settings,
            get_startup_file,
        ])
        .setup(|app| {
            capture_cli_file(app.handle());

            if let Some(window) = app.get_webview_window("main") {
                let settings = settings::load_settings();
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

fn capture_cli_file(app: &AppHandle) {
    let matches = match app.cli().matches() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("CLI parse error: {e}");
            return;
        }
    };

    if let Some(arg) = matches.args.get("file") {
        if let Some(path) = arg.value.as_str() {
            let path = path.to_string();
            if let Ok(mut slot) = app.state::<StartupFile>().0.lock() {
                *slot = Some(path.clone());
            }
            let _ = app.emit("file-to-open", path);
        }
    }
}
