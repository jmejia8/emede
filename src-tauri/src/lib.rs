mod pandoc;
mod settings;

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
fn get_startup_file(state: tauri::State<StartupFile>) -> Option<String> {
    state.0.lock().ok()?.clone()
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
