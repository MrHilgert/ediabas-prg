//! Runtime localization for `.ipo`-sourced strings.
//!
//! Every displayed string from an ECU's `.ipo` is passed through [`tr`]: it returns a
//! translation from the external `locale/<lang>.txt` table if one exists, otherwise the
//! original string unchanged (passthrough). Tables are plain `key<TAB>value` lines
//! (UTF-8), keyed by the source string; missing files or keys just pass through.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::lang::Lang;

struct Locales {
    ru: HashMap<String, String>,
    en: HashMap<String, String>,
}

fn locales() -> &'static Locales {
    static L: OnceLock<Locales> = OnceLock::new();
    L.get_or_init(|| Locales { ru: load("ru"), en: load("en") })
}

/// Translate an `.ipo` string for the current UI language, or pass it through unchanged.
pub fn tr(s: &str, lang: Lang) -> String {
    let table = match lang {
        Lang::Ru => &locales().ru,
        Lang::En => &locales().en,
    };
    table.get(s.trim()).cloned().unwrap_or_else(|| s.to_string())
}

/// Load `locale/<lang>.txt` (`key<TAB>value` per line). Absent file → empty (passthrough).
fn load(lang: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(path) = resolve(&format!("locale/{lang}.txt")) else { return map };
    let Ok(text) = std::fs::read_to_string(path) else { return map };
    for line in text.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('\t') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

/// Locate an external asset by walking up from cwd and the exe dir (like the ECU/SGDAT dirs).
pub fn resolve(rel: &str) -> Option<PathBuf> {
    let direct = Path::new(rel);
    if direct.exists() {
        return Some(direct.to_path_buf());
    }
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            bases.push(dir.to_path_buf());
        }
    }
    for base in bases {
        let mut dir = Some(base.as_path());
        while let Some(d) = dir {
            let cand = d.join(rel);
            if cand.exists() {
                return Some(cand);
            }
            dir = d.parent();
        }
    }
    None
}
