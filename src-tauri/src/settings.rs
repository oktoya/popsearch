use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const KEYRING_SERVICE: &str = "PopSearch";
const KEYRING_USER: &str = "anthropic-api-key";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub provider: String,
    pub web_search: bool,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_true")]
    pub capture_selection: bool,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default = "default_true")]
    pub autostart: bool,
}

fn default_model() -> String {
    "haiku".to_string()
}

fn normalize_default_model(m: &str) -> String {
    match m {
        "sonnet" | "opus" => m.to_string(),
        _ => "haiku".to_string(),
    }
}

fn default_true() -> bool {
    true
}

pub const DEFAULT_HOTKEY: &str = "CommandOrControl+Shift+Space";

fn default_hotkey() -> String {
    DEFAULT_HOTKEY.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            provider: "cli".to_string(),
            web_search: false,
            hotkey: default_hotkey(),
            capture_selection: true,
            default_model: default_model(),
            autostart: true,
        }
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    let Ok(path) = settings_path(&app) else {
        return Settings::default();
    };
    let mut s = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
        .unwrap_or_default();
    s.default_model = normalize_default_model(&s.default_model);
    s
}

#[tauri::command]
pub fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let provider = if settings.provider == "api" { "api" } else { "cli" };
    let normalized = Settings {
        provider: provider.to_string(),
        web_search: settings.web_search,
        hotkey: get_settings(app.clone()).hotkey,
        capture_selection: settings.capture_selection,
        default_model: normalize_default_model(&settings.default_model),
        autostart: settings.autostart,
    };
    write_settings(&app, &normalized)?;
    apply_autostart(&app, normalized.autostart);
    Ok(())
}

/// Make the OS autostart entry match the setting. Release builds only;
/// debug builds never touch the OS entry.
pub fn apply_autostart(app: &AppHandle, enabled: bool) {
    if cfg!(debug_assertions) {
        return;
    }
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let current = manager.is_enabled().unwrap_or(false);
    if enabled && !current {
        let _ = manager.enable();
    } else if !enabled && current {
        let _ = manager.disable();
    }
}

#[tauri::command]
pub fn is_release() -> bool {
    !cfg!(debug_assertions)
}

pub fn write_settings(app: &AppHandle, normalized: &Settings) -> Result<(), String> {
    let path = settings_path(app)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(normalized).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())
}

fn key_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_api_key() -> Option<String> {
    let entry = key_entry().ok()?;
    entry.get_password().ok().filter(|k| !k.is_empty())
}

#[tauri::command]
pub fn set_api_key(key: String) -> Result<(), String> {
    let entry = key_entry()?;
    if key.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    } else {
        entry.set_password(&key).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_without_hotkey_gets_default() {
        let s: Settings = serde_json::from_str(r#"{"provider":"api","web_search":true}"#).unwrap();
        assert_eq!(s.hotkey, "CommandOrControl+Shift+Space");
        assert_eq!(s.provider, "api");
        assert!(s.capture_selection);
    }

    #[test]
    fn settings_without_default_model_defaults_haiku() {
        let s: Settings = serde_json::from_str(r#"{"provider":"cli","web_search":false}"#).unwrap();
        assert_eq!(s.default_model, "haiku");
        assert_eq!(normalize_default_model("gpt"), "haiku");
        assert_eq!(normalize_default_model("opus"), "opus");
    }

    #[test]
    fn settings_without_autostart_defaults_true() {
        let s: Settings = serde_json::from_str(r#"{"provider":"cli","web_search":false}"#).unwrap();
        assert!(s.autostart);
    }

    #[test]
    fn settings_without_capture_selection_defaults_true() {
        let s: Settings = serde_json::from_str(r#"{"provider":"cli","web_search":false}"#).unwrap();
        assert!(s.capture_selection);
    }
}
