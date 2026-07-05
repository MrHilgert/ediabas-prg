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
        // A metadata-style LINE record has an EMPTY code body (`Body::Code` of length 0, so
        // `code()` is `Some(&[])`, not `None`) and carries its binding in `str2` (label) /
        // `str3` (result-id) — e.g. LCM activation screens list 54 in/outputs this way. Without
        // this the screen extracts zero rows and renders empty. Attach the current feeder job
        // (empty for pure selection screens like LCM → a static list; a real feeder → polls).
        let code = rec.code().unwrap_or(&[]);
        if code.is_empty() {
            if rec.tag == tag::LINE {
                metadata_rows(rec, &cur_job, line, &mut rows);
            }
            continue;
        }
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
                    // An empty literal — or an INPA infrastructure constant (a DLL import
                    // signature like `kernel32::GetPrivateProfileIntA:c.ssis%I`, which is really
                    // a runtime INI read) — is a placeholder INPA fills at display time. Show it
                    // as an (unresolved) dash, not the raw junk.
                    if v.is_empty() || is_infra_const(&v) {
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

/// Whether `s` reads as a human display label rather than INPA plumbing. `ftextout` operands
/// include DLL-import signatures (`user32::CharLowerA`), printf format strings (`c.s%S`), and
/// on-screen F-key navigation hints (`< F1 >  Read error memory`) — none of which are field
/// labels. A real label has a letter and is none of those.
fn looks_like_label(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && !is_infra_const(t) && !is_nav_hint(t) && t.chars().any(char::is_alphabetic)
}

/// An INPA infrastructure constant: a DLL import path (`module::func`) or a printf-style
/// format token (`%` followed by a format letter). These are runtime plumbing, not display text.
fn is_infra_const(s: &str) -> bool {
    if s.contains("::") {
        return true;
    }
    let b = s.as_bytes();
    b.iter()
        .enumerate()
        .any(|(i, &c)| c == b'%' && b.get(i + 1).is_some_and(u8::is_ascii_alphabetic))
}

/// An on-screen F-key navigation hint INPA draws on hub screens (`< F1 >  …`, `<F10>: Back`,
/// `<Shift> + < F2 >  …`). These describe the F-key menu, not the screen's data — rendering
/// them leaks INPA's own navigation text into our (already button+F-panel) UI. Detected by an
/// angle-bracketed `F<digit>` or a `<Shift>` combo prefix anywhere in the line.
fn is_nav_hint(s: &str) -> bool {
    let t = s.trim();
    for (i, _) in t.match_indices('<') {
        let rest = t[i + 1..].trim_start();
        if rest.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("shift")) {
            return true;
        }
        let mut ch = rest.chars();
        if matches!(ch.next(), Some('F' | 'f')) && ch.next().is_some_and(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

/// Emit rows from a metadata-style LINE record (`str2` = comma-joined labels, `str3` =
/// `;`-joined result-ids), used by screens that bind in record metadata rather than code
/// (LCM activation lists). Each label→id pair becomes a [`Row::Text`] carrying the current
/// feeder job (empty for pure selection screens → a static list).
fn metadata_rows(rec: &Record, job: &JobCall, line: u16, rows: &mut Vec<Row>) {
    if rec.str2.trim().is_empty() {
        return;
    }
    let ids: Vec<&str> = rec.str3.split(';').map(str::trim).collect();
    for (k, label) in rec.str2.split(',').map(str::trim).enumerate() {
        if label.is_empty() {
            continue;
        }
        let result = ids.get(k).copied().unwrap_or("").to_string();
        rows.push(Row::Text { label: label.to_string(), result, job: job.clone(), line });
    }
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


    #[test]
    fn label_filter_rejects_infra_constants() {
        // Real labels pass.
        assert!(super::looks_like_label("Rail-pressure"));
        assert!(super::looks_like_label("Давление в рампе"));
        assert!(super::looks_like_label("Duty 50%")); // trailing % (no format letter) is fine
        assert!(super::looks_like_label("Byte 1:")); // real coding-section heading
        // INPA infrastructure / format-string leaks are rejected.
        assert!(!super::looks_like_label("user32::CharLowerA:c.s%S"));
        assert!(!super::looks_like_label("c.s%S"));
        assert!(!super::looks_like_label("%d"));
        assert!(!super::looks_like_label("kernel32::GetProcAddress"));
        assert!(!super::looks_like_label(""));
        assert!(!super::looks_like_label("---")); // punctuation-only, no letter
        // On-screen F-key navigation hints are rejected (leak into our UI otherwise).
        assert!(!super::looks_like_label("< F1 >  Read error memory"));
        assert!(!super::looks_like_label("<F10>: Back"));
        assert!(!super::looks_like_label("<F2> : Analog status"));
        assert!(!super::looks_like_label("<Shift> + < F1 >  Exhaust valve front left"));
        assert!(!super::looks_like_label("<Shift> + < F10>  Exit"));
        // A real label that merely contains '<' as a comparison is NOT a nav hint.
        assert!(super::looks_like_label("Voltage < 5V"));
    }

    #[test]
    fn infra_const_detects_runtime_plumbing() {
        assert!(super::is_infra_const("kernel32::GetPrivateProfileIntA:c.ssis%I"));
        assert!(super::is_infra_const("c.s%S"));
        assert!(!super::is_infra_const("1.12")); // a real version value must survive
        assert!(!super::is_infra_const("Central Body Electronics IV E36"));
        assert!(!super::is_infra_const("50%")); // trailing % without format letter is fine
    }
}
