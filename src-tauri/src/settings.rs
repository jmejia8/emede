use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub font_family: String,
    #[serde(default = "default_font_inherit")]
    pub font_title: String,
    #[serde(default = "default_font_code", alias = "font_mono")]
    pub font_code: String,
    pub font_size: String,
    pub color_fg: String,
    pub color_bg: String,
    #[serde(default = "default_margin")]
    pub margin: String,
}

fn default_font_inherit() -> String {
    String::new()
}

fn default_font_code() -> String {
    "\"IBM Plex Mono\", \"JetBrains Mono\", \"Fira Code\", monospace".into()
}

fn default_margin() -> String {
    "10%".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_family: "\"Literata\", \"Source Serif 4\", \"Noto Serif\", serif".into(),
            font_title: default_font_inherit(),
            font_code: default_font_code(),
            font_size: "12pt".into(),
            color_fg: "#2c2c2c".into(),
            color_bg: "#faf8f5".into(),
            margin: default_margin(),
        }
    }
}

fn settings_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("emede")
        .join("settings.json")
}

pub fn load_settings() -> Settings {
    let path = settings_path();
    if path.exists() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        Settings::default()
    }
}

pub fn save_settings(settings: &Settings) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_settings() -> Settings {
    load_settings()
}

#[tauri::command]
pub fn set_settings(settings: Settings) -> Result<(), String> {
    save_settings(&settings)
}
