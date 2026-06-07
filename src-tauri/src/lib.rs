mod pandoc;
mod settings;

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, webview::PageLoadEvent, window::Color};
use tauri_plugin_cli::CliExt;

struct StartupFile(Mutex<Option<String>>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_cli::init())
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
        .on_page_load(|webview, payload| {
            if payload.event() == PageLoadEvent::Finished {
                if let Some(window) = webview.get_webview_window(webview.label()) {
                    let _ = window.show();
                }
            }
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
