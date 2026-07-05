//! Screen content: feeder job, title, and the bound rows (recipes 3, 4, 5).
//!
//! A screen's records follow it in file order: an optional CODE (0x21) block that runs the
//! feeder job once, then LINE (0x22) blocks — one per displayed line, each pairing a
//! `text`/`ftextout` label with an `ergebnis*` helper call that names the bound result.

use std::collections::HashMap;

use super::eval::{call_sites, Target, Val};
use crate::ids::{builtin, helper};
use crate::model::{InfoLine, InfoValue, JobCall, Row, Scale};
use crate::record::{tag, Const, Record};

/// What [`extract_screen`] recovers for one screen node.
pub(super) struct Content {
    pub title: String,
    pub feeder: Option<JobCall>,
    pub rows: Vec<Row>,
    pub info: Vec<InfoLine>,
}

/// Extract a screen's content from its child records (CODE + LINE blocks).
pub(super) fn extract_screen(
    records: &[&Record],
    proc_names: &HashMap<u16, String>,
    consts: &[Const],
) -> Content {
    let mut title = String::new();
    let mut feeder: Option<JobCall> = None;
    let mut rows: Vec<Row> = Vec::new();
    let mut cur_job = JobCall::default();
    // Ordered `ftextout` tokens from the screen's CODE block (Some = a constant string such
    // as a label or ":"; None = a runtime value slot). Assembled into static info lines for
    // ftextout-only screens (e.g. `s_info`).
    let mut entries: Vec<Option<String>> = Vec::new();

    for (li, rec) in records.iter().enumerate() {
        // The LINE-record index is the group key: rows from one record share an INPA line.
        let line = li as u16;
        let is_code = rec.tag == tag::CODE;
        let Some(code) = rec.code() else { continue };
        // Labels/units accumulate within a single LINE block, reset per record.
        let mut texts: Vec<String> = Vec::new();

        for cs in call_sites(code, consts) {
            match cs.target {
                Target::Builtin(builtin::FTEXTOUT) => {
                    let s = last_str(&cs.args);
                    if is_code && title.is_empty() {
                        if let Some(t) = s {
                            title = t.to_string();
                        }
                    }
                    if !is_code {
                        // A LINE-record ftextout is a row label candidate.
                        if let Some(t) = s {
                            texts.push(t.to_string());
                        }
                    }
                    // Record every ftextout as a token (string or runtime slot). Only used to
                    // build static info lines for screens that end up with no job-bound rows.
                    entries.push(s.map(|t| t.to_string()));
                }
                Target::Builtin(builtin::TEXT) => {
                    if let Some(s) = last_str(&cs.args) {
                        texts.push(s.to_string());
                    }
                }
                Target::Builtin(builtin::INPA_JOB) => {
                    let job = job_from(&cs.args);
                    if is_code && feeder.is_none() {
                        feeder = Some(job.clone());
                    }
                    cur_job = job;
                }
                Target::Proc(idx) => {
                    let name = proc_names.get(&idx).map(String::as_str).unwrap_or("");
                    if let Some(row) = row_from(name, &cs.args, &texts, &cur_job, line) {
                        rows.push(row);
                        texts.clear();
                    }
                }
                _ => {}
            }
        }
    }

    // Static info lines only matter for ftextout-only screens (no job-bound rows).
    let info = if rows.is_empty() { build_info(&entries, &title) } else { Vec::new() };
    Content { title, feeder, rows, info }
}

/// Assemble ordered `ftextout` tokens into `label : value` info lines. A `label` followed
/// by a `":"` separator becomes a field (value = the next token: a runtime slot → dash, or
/// a constant); a label with no separator is a section heading. The page heading that just
/// repeats the screen title is dropped (it's already shown in the header).
fn build_info(entries: &[Option<String>], title: &str) -> Vec<InfoLine> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let Some(Some(s)) = entries.get(i).map(|e| e.as_ref().filter(|s| s.trim() != ":")) else {
            i += 1; // stray separator or runtime value with no preceding label
            continue;
        };
        if !looks_like_label(s) {
            i += 1; // INPA infrastructure constant (import signature / format string) — skip
            continue;
        }
        let label = s.trim().to_string();
        let mut j = i + 1;
        let had_sep = matches!(entries.get(j), Some(Some(x)) if x.trim() == ":");
        if had_sep {
            j += 1;
        }
        let value = if had_sep {
            match entries.get(j) {
                Some(None) => {
                    j += 1;
                    InfoValue::Runtime
                }
                Some(Some(v)) if v.trim() != ":" => {
                    let v = v.trim().to_string();
                    j += 1;
                    // An empty string literal is a runtime placeholder INPA fills at display
                    // time — show it as an (unresolved) dash, not a blank.
                    if v.is_empty() {
                        InfoValue::Runtime
                    } else {
                        InfoValue::Const(v)
                    }
                }
                _ => InfoValue::Runtime,
            }
        } else {
            InfoValue::Heading
        };
        let is_title_heading =
            value == InfoValue::Heading && label.eq_ignore_ascii_case(title.trim());
        if !label.is_empty() && !is_title_heading {
            out.push(InfoLine { label, value });
        }
        i = j;
    }
    out
}

/// Whether `s` reads as a human display label rather than an INPA infrastructure constant.
/// `ftextout` operands include DLL-import signatures (`user32::CharLowerA`) and printf-style
/// format strings (`c.s%S`) that must NOT leak into visible field labels. A real label has a
/// letter and neither a `::` module path nor a `%<letter>` format token.
fn looks_like_label(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.contains("::") {
        return false;
    }
    let bytes = t.as_bytes();
    let has_fmt = bytes
        .iter()
        .enumerate()
        .any(|(i, &b)| b == b'%' && bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic));
    if has_fmt {
        return false;
    }
    t.chars().any(char::is_alphabetic)
}

/// Build a [`Row`] from an `ergebnis*` helper call, if `name` is a recognised helper.
fn row_from(name: &str, args: &[Val], texts: &[String], job: &JobCall, line: u16) -> Option<Row> {
    let label = texts.first().cloned().unwrap_or_default();
    match name {
        helper::DIGITAL_OUT => Some(Row::Logical {
            label,
            result: arg_str(args, 0),
            on: arg_str(args, 3),
            off: arg_str(args, 4),
            job: job.clone(),
            line,
        }),
        helper::ANALOG_OUT => Some(Row::Analog {
            label,
            unit: clean_unit(texts.get(1)),
            result: arg_str(args, 0),
            scale: Scale { factor: 1.0, offset: 0.0, min: arg_num(args, 3), max: arg_num(args, 4) },
            job: job.clone(),
            line,
        }),
        helper::ANALOG_CONV_OUT => Some(Row::Analog {
            label,
            unit: clean_unit(texts.get(1)),
            result: arg_str(args, 0),
            scale: Scale {
                factor: arg_num_or(args, 1, 1.0),
                offset: arg_num(args, 2),
                min: arg_num(args, 3),
                max: arg_num(args, 4),
            },
            job: job.clone(),
            line,
        }),
        helper::TEXT_OUT => Some(Row::Text {
            label,
            result: arg_str(args, 0),
            job: job.clone(),
            line,
        }),
        _ => None,
    }
}

/// Build a [`JobCall`] from an `INPAapiJob(handle, job, arg, results)` arg list.
pub(super) fn job_from(args: &[Val]) -> JobCall {
    let job = arg_str(args, 1);
    let arg = arg_str(args, 2);
    let selector = (job == "MW_SELECT_LESEN_NORM" && !arg.is_empty()).then(|| arg.clone());
    JobCall { job, arg, selector }
}

fn arg_str(args: &[Val], i: usize) -> String {
    args.get(i).and_then(Val::as_str).unwrap_or("").to_string()
}
fn arg_num(args: &[Val], i: usize) -> f64 {
    args.get(i).and_then(Val::as_num).unwrap_or(0.0)
}
fn arg_num_or(args: &[Val], i: usize, dflt: f64) -> f64 {
    args.get(i).and_then(Val::as_num).unwrap_or(dflt)
}
fn last_str(args: &[Val]) -> Option<&str> {
    args.iter().rev().find_map(Val::as_str)
}

/// Strip the surrounding `[...]`/whitespace INPA wraps units in (`"[mbar]"` → `"mbar"`).
fn clean_unit(u: Option<&String>) -> String {
    u.map(|s| s.trim().trim_start_matches('[').trim_end_matches(']').trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::looks_like_label;

    #[test]
    fn label_filter_rejects_infra_constants() {
        // Real labels pass.
        assert!(looks_like_label("Rail-pressure"));
        assert!(looks_like_label("Давление в рампе"));
        assert!(looks_like_label("Duty 50%")); // trailing % (no format letter) is fine
        // INPA infrastructure / format-string leaks are rejected.
        assert!(!looks_like_label("user32::CharLowerA:c.s%S"));
        assert!(!looks_like_label("c.s%S"));
        assert!(!looks_like_label("%d"));
        assert!(!looks_like_label("kernel32::GetProcAddress"));
        assert!(!looks_like_label(""));
        assert!(!looks_like_label("---")); // punctuation-only, no letter
    }
}
