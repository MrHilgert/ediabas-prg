//! Persistent user settings — hand-rolled `.ini` next to the executable.
//!
//! No serde/ini/toml dependency (the codebase hand-rolls its parsers). The file is
//! a flat `key = value` list; unknown keys and `[sections]` are ignored, missing or
//! malformed files fall back to [`Settings::default`]. Written best-effort — a write
//! failure is silently ignored so a read-only install still runs.

use std::path::PathBuf;

use crate::lang::Lang;
use crate::ui::theme::Theme;

/// Everything we remember between runs.
#[derive(Clone, PartialEq, Eq)]
pub struct Settings {
    pub lang: Lang,
    pub theme: Theme,
    /// Serial port for CONNECT. `None` = auto (first available port).
    pub port: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { lang: Lang::Ru, theme: Theme::Dark, port: None }
    }
}

impl Settings {
    /// Read `settings.ini` from the exe dir (fallback: a hand-placed file found by the
    /// asset resolver). Absent or unparseable → defaults.
    pub fn load() -> Settings {
        let path = config_path().filter(|p| p.exists()).or_else(|| crate::i18n::resolve("settings.ini"));
        let Some(path) = path else { return Settings::default() };
        let Ok(text) = std::fs::read_to_string(path) else { return Settings::default() };
        Settings::parse(&text)
    }

    /// Write `settings.ini` next to the exe. Best-effort: errors are swallowed.
    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        let _ = std::fs::write(path, self.serialize());
    }

    fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            // Drop inline `# comment` and surrounding whitespace.
            let v = v.split('#').next().unwrap_or("").trim();
            match k.trim().to_ascii_lowercase().as_str() {
                "language" | "lang" => {
                    s.lang = match v.to_ascii_lowercase().as_str() {
                        "en" => Lang::En,
                        _ => Lang::Ru,
                    }
                }
                "theme" => {
                    s.theme = match v.to_ascii_lowercase().as_str() {
                        "light" => Theme::Light,
                        _ => Theme::Dark,
                    }
                }
                "port" => s.port = (!v.is_empty()).then(|| v.to_string()),
                _ => {}
            }
        }
        s
    }

    fn serialize(&self) -> String {
        let lang = match self.lang {
            Lang::Ru => "ru",
            Lang::En => "en",
        };
        let theme = match self.theme {
            Theme::Dark => "dark",
            Theme::Light => "light",
        };
        let port = self.port.as_deref().unwrap_or("");
        format!(
            "# eDIAG settings\n\
             language = {lang}   # ru | en\n\
             theme    = {theme}   # dark | light\n\
             port     = {port}   # COM3 (Windows) | /dev/ttyUSB0 (Linux); empty = auto\n"
        )
    }
}

/// Location we read from and write to: `<exe dir>/settings.ini` (fallback: cwd).
fn config_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return Some(dir.join("settings.ini"));
        }
    }
    std::env::current_dir().ok().map(|d| d.join("settings.ini"))
}
