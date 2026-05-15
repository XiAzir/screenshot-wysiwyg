use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Settings {
    pub save_to_file: bool,
    pub save_to_clipboard: bool,
    pub save_dir: String,
    pub hotkey: String,
    pub start_with_windows: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            save_to_file: true,
            save_to_clipboard: false,
            save_dir: default_save_dir(),
            hotkey: "Ctrl+Alt+B".to_string(),
            start_with_windows: false,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let mut settings = Settings::default();
        let path = config_path();
        let legacy_path = legacy_config_path();
        let path = if path.exists() || !legacy_path.exists() {
            path
        } else {
            legacy_path
        };
        let Ok(content) = fs::read_to_string(path) else {
            return settings;
        };

        for line in content.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "save_to_file" => settings.save_to_file = value.trim() == "true",
                "save_to_clipboard" => settings.save_to_clipboard = value.trim() == "true",
                "save_dir" => settings.save_dir = value.trim().to_string(),
                "hotkey" => settings.hotkey = value.trim().to_string(),
                "start_with_windows" => settings.start_with_windows = value.trim() == "true",
                _ => {}
            }
        }

        if settings.save_dir.trim().is_empty() {
            settings.save_dir = default_save_dir();
        }
        if settings.hotkey.trim().is_empty() {
            settings.hotkey = "Ctrl+Alt+B".to_string();
        }

        settings
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("创建配置目录失败")?;
        }

        let content = format!(
            "save_to_file={}\nsave_to_clipboard={}\nsave_dir={}\nhotkey={}\nstart_with_windows={}\n",
            self.save_to_file,
            self.save_to_clipboard,
            self.save_dir,
            self.hotkey,
            self.start_with_windows
        );

        fs::write(path, content).context("写入配置失败")
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    base.join("Kumokiri").join("settings.ini")
}

fn legacy_config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    base.join("WysiwygScreenshot").join("settings.ini")
}

pub fn default_save_dir() -> String {
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile)
            .join("Pictures")
            .join("Screenshots")
            .to_string_lossy()
            .to_string();
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .to_string_lossy()
        .to_string()
}
