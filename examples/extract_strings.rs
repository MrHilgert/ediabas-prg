//! Extract translatable German strings from every `.prg`/`.GRP` in `ecu/`.
//!
//! For each file it writes `strings/<name>.strings` — a structured, context-carrying
//! dump grouped by origin (SGBD comment fields, then each table column). Strings are
//! classified so the translation step only ever sees genuine human-readable German:
//!
//!   * `identifier` — communication keywords / names (`JOB_STATUS`, `OKAY`, `STAT_*`,
//!     hex, numbers). Dropped entirely: the software matches on these, they must NOT
//!     be translated.
//!   * `unit`       — measurement units (`°C`, `1/min`, `km/h`, `V`). Kept in a
//!     separate `[UNITS — do not translate]` section so nothing is lost, but excluded
//!     from translation (units are the same across languages / handled in the GUI).
//!   * `text`       — real German prose (fault descriptions, status texts, long names).
//!     THIS is what gets translated.
//!
//! It also writes two corpus-wide, de-duplicated masters that are the actual input to
//! the translators (translate each unique string once, apply everywhere):
//!   * `strings/_all_text.txt`  — every unique translatable German string, sorted.
//!   * `strings/_all_units.txt` — every unique unit, sorted (reference, not translated).
//!
//! Run: `cargo run --release --example extract_strings`

use ediabas::prg::PrgFile;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

#[derive(PartialEq)]
enum Kind {
    Skip,
    Unit,
    Text,
}

/// A curated set of units seen in BMW SGBDs (case-sensitive where it matters).
const KNOWN_UNITS: &[&str] = &[
    "°C", "°KW", "°kw", "°", "1/min", "U/min", "min-1", "min^-1", "km/h", "kmh", "V", "mV", "kV",
    "A", "mA", "µA", "%", "Nm", "bar", "mbar", "hPa", "kPa", "Pa", "Ohm", "kOhm", "MOhm", "ms",
    "µs", "ns", "s", "sec", "min", "h", "l/h", "l/100km", "g/s", "kg/h", "mg/Hub", "mg/hub",
    "mg/H", "mg", "kg", "g", "ml", "l", "Hz", "kHz", "MHz", "W", "kW", "kWh", "rpm", "U/L", "°C/s",
    "1/s", "K", "°F", "psi", "km", "m", "cm", "mm", "dl", "Vol%", "ppm", "mg/stroke",
];

/// Is this string a unit (do-not-translate, but keep for reference)?
fn is_unit(t: &str) -> bool {
    if KNOWN_UNITS.contains(&t) {
        return true;
    }
    // Short, space-free token containing a non-letter marker typical of units
    // (digit, slash, degree, percent, micro, caret). Excludes plain words.
    if t.len() <= 7 && !t.contains(' ') {
        let has_marker = t.chars().any(|c| matches!(c, '/' | '°' | '%' | 'µ' | '^' | '-' | '0'..='9'));
        let charset_ok = t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '°' | '%' | 'µ' | '^' | '-' | '.'));
        if has_marker && charset_ok {
            return true;
        }
    }
    false
}

/// A single (space-free) token that is a code identifier, not German prose:
/// hex literal, snake_case, camelCase, digit-bearing code, or vowel-less gibberish
/// (`STAT_x_WERT`, `Cpp_trqInrCor_3`, `0x22332F`, `deviceIdIC1020`, `ARG_0xD0C4`).
/// Multi-word strings (containing a space) are always prose, never identifiers.
fn is_code_identifier(t: &str) -> bool {
    if t.contains(' ') {
        return false;
    }
    let low = t.to_ascii_lowercase();
    if let Some(rest) = low.strip_prefix("0x") {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_hexdigit()) {
            return true;
        }
    }
    // snake_case or any embedded digit ⇒ code.
    if t.contains('_') || t.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    let chars: Vec<char> = t.chars().collect();
    // camelCase: a lowercase immediately followed by an uppercase.
    if chars.windows(2).any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase()) {
        return true;
    }
    // Interior uppercase cluster past position 0 (e.g. "...HSS...", "...IC...").
    if chars.iter().skip(1).filter(|c| c.is_ascii_uppercase()).count() >= 2 {
        return true;
    }
    // A real word has a vowel; vowel-less tokens are abbreviations/codes.
    if !t.chars().any(|c| "aeiouyäöüAEIOUY".contains(c)) {
        return true;
    }
    false
}

/// Classify a raw cell/field value. Only `Text` is translated.
fn classify(raw: &str) -> Kind {
    let t = raw.trim();
    if t.len() < 2 || t.len() > 200 {
        return Kind::Skip;
    }
    let has_alpha = t.chars().any(|c| c.is_alphabetic());
    if !has_alpha {
        return Kind::Skip; // pure numbers / hex / symbols / separators
    }
    // Pure identifier / communication keyword: A-Z, digits, underscore only.
    // (JOB_STATUS, OKAY, STAT_DREHZAHL_WERT, ...)
    if t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        return Kind::Skip;
    }
    // Units first (some, like "km/h", would otherwise trip the identifier test).
    if is_unit(t) {
        return Kind::Unit;
    }
    // Code identifiers leaking through as mixed-case tokens.
    if is_code_identifier(t) {
        return Kind::Skip;
    }
    let has_lower = t.chars().any(|c| c.is_ascii_lowercase() || matches!(c, 'ä' | 'ö' | 'ü' | 'ß'));
    // Real prose has a lowercase letter, OR is an ALL-CAPS phrase with spaces
    // (e.g. "KEIN FEHLER"). A single ALL-CAPS token with no space was already
    // caught above as an identifier.
    if has_lower || t.contains(' ') {
        Kind::Text
    } else {
        Kind::Skip
    }
}

/// GUI-facing category of a table column, by its name. Only these two surface in
/// the GUI (error names/descriptions + parameter/detail labels); everything else
/// (SGBD comments, supplier/internal metadata) is not displayed and not translated.
#[derive(PartialEq, Clone, Copy)]
enum Cat {
    Detail, // measurement / parameter labels, status/mode texts — "детали"
    Fault,  // DTC names + fault descriptions — "названия/описания ошибок"
    Other,  // not displayed → excluded from the GUI translation set
}

fn col_category(col: &str) -> Cat {
    let c = col.to_uppercase();
    let has = |k: &str| c.contains(k);
    if has("FEHLER") || has("FTEXT") || has("STOER") || has("DTC") || has("FCODE")
        || has("ORTTEXT") || has("UWTEXT") || has("ARTTEXT") || has("ERROR")
    {
        return Cat::Fault;
    }
    if has("LNAME") || has("LTEXT") || has("BEZEICHN") || has("BENENN") || has("LABEL")
        || has("ANZEIGE") || has("STATUS_TEXT") || has("MODE_TEXT") || has("BEZ")
        || c == "NAME" || c == "TEXT" || c == "INFO"
    {
        return Cat::Detail;
    }
    Cat::Other
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ecu_dir = Path::new("ecu");
    let out_dir = Path::new("strings");
    fs::create_dir_all(out_dir)?;

    let mut all_text: BTreeSet<String> = BTreeSet::new();
    let mut all_units: BTreeSet<String> = BTreeSet::new();
    // GUI-displayed translation targets (deduped across the whole corpus).
    let mut gui_details: BTreeSet<String> = BTreeSet::new();
    let mut gui_faults: BTreeSet<String> = BTreeSet::new();

    let mut files: Vec<_> = fs::read_dir(ecu_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            ext.eq_ignore_ascii_case("prg") || ext.eq_ignore_ascii_case("grp")
        })
        .collect();
    files.sort();

    let (mut ok, mut failed) = (0usize, 0usize);
    for path in &files {
        let prg = match PrgFile::open(path) {
            Ok(p) => p,
            Err(_) => {
                failed += 1;
                continue;
            }
        };
        ok += 1;

        // Per-file collections, deduped, insertion-order-ish via BTreeSet for stability.
        let mut file_text: BTreeSet<String> = BTreeSet::new();
        let mut file_units: BTreeSet<String> = BTreeSet::new();
        let mut body = String::new();

        // 1) SGBD comment fields (ECUCOMMENT, JOBCOMMENT, ...): German prose only.
        let fields = prg.sgbd_fields();
        let mut field_lines: Vec<(String, String)> = Vec::new();
        for (k, v) in &fields {
            let ku = k.to_uppercase();
            if ku.contains("COMMENT") || ku.contains("INFO") || ku.contains("TEXT") {
                if classify(v) == Kind::Text {
                    field_lines.push((k.clone(), v.trim().to_string()));
                }
            }
        }
        if !field_lines.is_empty() {
            let _ = writeln!(body, "[FIELDS]");
            for (k, v) in field_lines {
                let _ = writeln!(body, "{k}\t{v}");
                file_text.insert(v);
            }
            let _ = writeln!(body);
        }

        // 2) Tables: classify each cell per column.
        let tables = prg.parse_tables();
        let mut table_names: Vec<&String> = tables.keys().collect();
        table_names.sort();
        for tname in table_names {
            let t = &tables[tname];
            // Which columns hold any translatable text? Emit those, per column.
            for (ci, col) in t.columns.iter().enumerate() {
                let mut col_text: BTreeSet<String> = BTreeSet::new();
                let mut col_units: BTreeSet<String> = BTreeSet::new();
                for row in &t.rows {
                    let Some(cell) = row.get(ci) else { continue };
                    match classify(cell) {
                        Kind::Text => {
                            col_text.insert(cell.trim().to_string());
                        }
                        Kind::Unit => {
                            col_units.insert(cell.trim().to_string());
                        }
                        Kind::Skip => {}
                    }
                }
                if !col_text.is_empty() {
                    let _ = writeln!(body, "[TABLE {tname} col={col}]");
                    for s in &col_text {
                        let _ = writeln!(body, "{s}");
                    }
                    let _ = writeln!(body);
                    match col_category(col) {
                        Cat::Detail => gui_details.extend(col_text.iter().cloned()),
                        Cat::Fault => gui_faults.extend(col_text.iter().cloned()),
                        Cat::Other => {}
                    }
                    file_text.extend(col_text);
                }
                file_units.extend(col_units);
            }
        }

        // 3) Units section (kept, not translated).
        if !file_units.is_empty() {
            let _ = writeln!(body, "[UNITS — do not translate]");
            for u in &file_units {
                let _ = writeln!(body, "{u}");
            }
            let _ = writeln!(body);
        }

        all_text.extend(file_text.iter().cloned());
        all_units.extend(file_units.iter().cloned());

        // Only write a file if it produced anything.
        if !file_text.is_empty() || !file_units.is_empty() {
            let stem = path.file_name().and_then(|s| s.to_str()).unwrap_or("unknown");
            let header = format!("# {stem}\n# translatable text: {}\n\n", file_text.len());
            fs::write(out_dir.join(format!("{stem}.strings")), format!("{header}{body}"))?;
        }
    }

    // A string seen under a detail column takes priority over a fault column, so a
    // term is translated once and never appears in both GUI masters.
    let faults_only: Vec<String> =
        gui_faults.iter().filter(|s| !gui_details.contains(*s)).cloned().collect();

    let write_lines = |name: &str, set: &[String]| -> std::io::Result<()> {
        fs::write(out_dir.join(name), set.join("\n") + "\n")
    };

    // Corpus-wide masters (the real translation input).
    write_lines("_all_text.txt", &all_text.iter().cloned().collect::<Vec<_>>())?;
    write_lines("_all_units.txt", &all_units.iter().cloned().collect::<Vec<_>>())?;
    // GUI translation targets.
    write_lines("_gui_details.txt", &gui_details.iter().cloned().collect::<Vec<_>>())?;
    write_lines("_gui_faults.txt", &faults_only)?;

    eprintln!("files: {ok} parsed, {failed} failed");
    eprintln!("unique translatable text strings: {}", all_text.len());
    eprintln!("unique units (excluded): {}", all_units.len());
    eprintln!("GUI details (parameter/detail labels): {}", gui_details.len());
    eprintln!("GUI faults (names + descriptions):     {}", faults_only.len());
    eprintln!("GUI total to translate: {}", gui_details.len() + faults_only.len());
    Ok(())
}
