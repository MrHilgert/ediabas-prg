//! Localization — two independent tables, each an `.ini` (`key = value`).
//!
//! 1. **UI chrome** — our own interface strings, keyed by a stable slug and looked
//!    up with [`t`]. Russian is just another locale file (`ui.ru.ini`), the base
//!    every language falls back to. A key with no translation shows the key itself,
//!    so a gap is visible rather than blank. Values may embed `{placeholder}`s that
//!    the caller fills with `str::replace`.
//!
//! 2. **ECU / `.prg` strings** — text that comes out of a BMW SGBD (`.prg`) or INPA
//!    `.ipo`, whose source language is **German**. Translated with [`tr_prg`], keyed
//!    by the German source string (`prg.<lang>.ini`: `deutsch = перевод`). Unknown
//!    strings pass through unchanged (the German original is shown).
//!
//! Both tables are plain `.ini`: `key = value` per line, `#`/`;` comments and
//! `[sections]` ignored, split on the first `=`. Missing files → empty (passthrough).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::lang::Lang;

struct Tables {
    ui_ru: HashMap<String, String>,
    ui_en: HashMap<String, String>,
    prg_ru: HashMap<String, String>,
    prg_en: HashMap<String, String>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| Tables {
        ui_ru: load_ini("locale/ui.ru.ini"),
        ui_en: load_ini("locale/ui.en.ini"),
        // ECU/`.prg` strings: the bulk comes from the translated `de-<lang>.tsv`
        // (German source → translation, tab-separated). A small hand-curated
        // `prg.<lang>.ini` is overlaid on top so manual fixes win over the batch.
        prg_ru: merge(load_tsv("locale/de-ru.tsv"), load_ini("locale/prg.ru.ini")),
        prg_en: merge(load_tsv("locale/de-en.tsv"), load_ini("locale/prg.en.ini")),
    })
}

/// Overlay `over` onto `base` (later keys win), returning the merged map.
fn merge(mut base: HashMap<String, String>, over: HashMap<String, String>) -> HashMap<String, String> {
    base.extend(over);
    base
}

/// Translate a UI chrome string by its stable key. Falls back to the Russian base
/// table, then to the key itself.
pub fn t(key: &str, lang: Lang) -> String {
    let tb = tables();
    let primary = match lang {
        Lang::Ru => &tb.ui_ru,
        Lang::En => &tb.ui_en,
    };
    primary
        .get(key)
        .or_else(|| tb.ui_ru.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// Translate a German ECU/`.prg` string into the UI language. Unknown strings pass
/// through unchanged (the German original).
pub fn tr_prg(german: &str, lang: Lang) -> String {
    let tb = tables();
    let table = match lang {
        Lang::Ru => &tb.prg_ru,
        Lang::En => &tb.prg_en,
    };
    table.get(german.trim()).cloned().unwrap_or_else(|| german.to_string())
}

/// Load a translation table from a tab-separated file (`german<TAB>translation`).
/// This is how the bulk ECU-string translations ship (`de-ru.tsv`, `de-en.tsv`):
/// tabs, not `=`, so German text containing `=`/`;`/`[` is preserved verbatim.
/// Absent file → empty map (passthrough). Keys are trimmed to match `tr_prg`'s
/// trimmed lookup; blank lines and lines without a tab are skipped.
fn load_tsv(rel: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(path) = resolve(rel) else { return map };
    let Ok(text) = std::fs::read_to_string(path) else { return map };
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('\t') {
            let k = k.trim();
            if !k.is_empty() {
                map.insert(k.to_string(), v.trim().to_string());
            }
        }
    }
    map
}

/// Load an `.ini` table (`key = value`). Absent file → empty map (passthrough).
fn load_ini(rel: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(path) = resolve(rel) else { return map };
    let Ok(text) = std::fs::read_to_string(path) else { return map };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('[') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

/// Locate `<subdir>/<file>` case-insensitively. On Linux (case-sensitive FS) the
/// staged ECU/SGDAT assets are often upper-case (`KOMBI.IPO`, `DDE40KW0.PRG`) while
/// the catalog/script names are lower-case (`kombi`), so an exact path misses. Try
/// the exact path first (fast, correct on Windows/macOS), then scan `<subdir>` for a
/// filename that matches case-insensitively.
pub fn resolve_ci(subdir: &str, file: &str) -> Option<PathBuf> {
    if let Some(p) = resolve(&format!("{subdir}/{file}")) {
        return Some(p);
    }
    let dir = resolve(subdir)?;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(file) {
            return Some(entry.path());
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::tr_prg;
    use crate::lang::Lang;

    /// The bulk ECU dictionary (`de-ru.tsv`) must actually load and match: a known German
    /// `.prg` string translates. Guards against a silent load failure (empty table →
    /// everything passes through untranslated, "no translations visible").
    #[test]
    fn prg_dictionary_loads_and_translates() {
        let out = tr_prg("% Tastverhältnis", Lang::Ru);
        assert_eq!(out, "% Скважность", "de-ru.tsv not loaded or key not matched (out={out:?})");
    }
}
