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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_bold: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_italic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_code_bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_border: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_muted: Option<String>,
    #[serde(default = "default_margin")]
    pub margin: String,
    #[serde(default = "default_window_frame")]
    pub window_frame: String,
    #[serde(default = "default_keybindings")]
    pub keybindings: String,
    #[serde(default = "default_gpu_acceleration")]
    pub gpu_acceleration: bool,
    #[serde(default)]
    pub justify_text: bool,
    #[serde(default = "default_mermaid_diagrams")]
    pub mermaid_diagrams: bool,
    /// Name shown as the host on LAN-shared pages. Defaults to the `USER`
    /// environment value; empty means "fall back to the env var / anonymous".
    #[serde(default = "default_share_username")]
    pub share_username: String,
}

fn default_window_frame() -> String {
    "emede".into()
}

fn default_keybindings() -> String {
    "default".into()
}

fn default_gpu_acceleration() -> bool {
    false
}

fn default_mermaid_diagrams() -> bool {
    true
}

/// Default shared-host footer text, e.g. "Shared by jesus". Shown verbatim on
/// LAN-shared pages. Empty if no OS username is available.
fn default_share_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .map(|u| format!("Shared by {u}"))
        .unwrap_or_default()
}

fn default_font_inherit() -> String {
    String::new()
}

fn default_font_code() -> String {
    "\"IBM Plex Mono\", \"JetBrains Mono\", \"Fira Code\", monospace".into()
}

fn default_margin() -> String {
    "16cm".into()
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
            color_title: None,
            color_bold: None,
            color_italic: None,
            color_quote: None,
            color_link: None,
            color_code: None,
            color_code_bg: None,
            color_border: None,
            color_muted: None,
            margin: default_margin(),
            window_frame: default_window_frame(),
            keybindings: default_keybindings(),
            gpu_acceleration: default_gpu_acceleration(),
            justify_text: false,
            mermaid_diagrams: default_mermaid_diagrams(),
            share_username: default_share_username(),
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
    crate::persist::load_json_or_backup(&settings_path())
}

pub fn save_settings(settings: &Settings) -> Result<(), String> {
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    crate::persist::write_json_atomic(&settings_path(), &json)
}

#[tauri::command]
pub fn get_settings() -> Settings {
    load_settings()
}

#[tauri::command]
pub fn set_settings(settings: Settings) -> Result<(), String> {
    save_settings(&settings)
}

#[tauri::command]
pub fn read_color_template(path: String) -> Result<String, String> {
    let path = PathBuf::from(path);
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;

    if metadata.len() > 256 * 1024 {
        return Err("CSS template is too large".into());
    }

    fs::read_to_string(path).map_err(|e| e.to_string())
}
