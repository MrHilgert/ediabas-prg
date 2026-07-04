//! Fault-memory page (`FS_LESEN` / `FS_LOESCHEN`): the pre-`.ipo` INPA-style DTC view —
//! expandable rows with status, cause, freeze-frame and raw record — chrome i18n'd via
//! `t()`. Read/Clear live in the F-zone (F1/F2); this only renders + toggles detail.

use egui::{Align2, Color32, FontId, Pos2, RichText, Sense, Stroke, Vec2};

use super::Act;
use crate::app::App;
use crate::i18n::t;
use crate::lang::Lang;
use crate::theme::Colors;

/// The whole fault page.
pub(super) fn render(ui: &mut egui::Ui, c: &Colors, app: &App, act: &mut Option<Act>, lang: Lang) {
    // Auto-read on open (bounded retry — a first FS_LESEN after streaming can time out).
    if app.connected_code().is_some() && app.faults_tries < 3 && app.faults.is_none() && !app.faults_busy {
        *act = Some(Act::RunJob { job: "FS_LESEN".into(), arg: String::new() });
    }
    ui.label(
        RichText::new(t("faults_fkeys", lang)).size(11.0).color(c.fg_faint),
    );
    ui.add_space(12.0);

    if app.faults_busy {
        ui.label(RichText::new(t("reading_faults", lang)).size(12.0).color(c.accent));
        return;
    }
    // A read error (e.g. transient timeout) — surface it instead of silence.
    if app.faults.is_none() && !app.status_msg.is_empty() {
        ui.label(RichText::new(app.status_msg.clone()).size(12.0).color(c.err));
        ui.add_space(6.0);
    }
    let Some(fr) = &app.faults else {
        ui.label(
            RichText::new(t("press_f1", lang)).size(12.0).color(c.fg_dim),
        );
        return;
    };
    let dtcs = parse_dtcs(fr);
    if dtcs.is_empty() {
        ui.label(RichText::new(t("faults_empty", lang)).size(13.0).strong().color(c.ok));
        return;
    }
    ui.label(
        RichText::new(format!("{} {}", dtcs.len(), t("faults_count", lang)))
            .size(11.0)
            .color(c.fg_faint),
    );
    ui.add_space(6.0);
    for d in &dtcs {
        let expanded = app.fault_open.as_deref() == Some(d.code.as_str());
        let (slabel, scol) = fault_status(c, d.present, d.sporadic, lang);
        if fault_row(ui, c, &d.code, &d.text, &slabel, scol, expanded) {
            *act = Some(Act::ToggleFault(d.code.clone()));
        }
        if expanded {
            real_fault_detail(ui, c, d, lang);
        }
    }
}

// ───────────────────────────────── model ─────────────────────────────────

struct Uw {
    text: String,
    val: String,
    unit: String,
}

/// One decoded DTC from `FS_LESEN`.
struct Dtc {
    code: String,        // e.g. "1E00"
    text: String,        // F_ORT_TEXT
    present: bool,       // F_VORHANDEN — currently active
    sporadic: bool,      // F_ART "sporadischer Fehler" flag
    raw: String,         // F_HEX_CODE
    hfk: i64,            // F_HFK  — occurrences
    lz: i64,             // F_LZ   — lifetime counter
    uw_satz: i64,        // F_UW_SATZ — number of freeze-frame snapshots
    uw: Vec<Uw>,         // F_UW1..N — freeze-frame conditions
    causes: Vec<String>, // F_ART{i}_TEXT != "--" — fault types/causes
}

fn fmt_uw(v: f64) -> String {
    if v.fract().abs() < 0.05 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Parse the `FS_LESEN` result into a DTC list (one per set carrying `F_ORT_NR`).
fn parse_dtcs(fr: &ediabas::JobResult) -> Vec<Dtc> {
    let mut out = Vec::new();
    for set in fr.sets() {
        if !set.contains("F_ORT_NR") {
            continue; // e.g. the trailing JOB_STATUS set
        }
        let code_num = set.get_i64("F_ORT_NR").unwrap_or(0);
        let text = set.get_str("F_ORT_TEXT").unwrap_or_default().trim().to_string();
        if code_num == 0 && text.is_empty() {
            continue;
        }
        // Freeze-frame: F_UW{i}_TEXT/_WERT/_EINH, contiguous from 1.
        let mut uw = Vec::new();
        for i in 1..=64 {
            let tkey = format!("F_UW{i}_TEXT");
            if !set.contains(&tkey) {
                break;
            }
            let val = set
                .get_f64(&format!("F_UW{i}_WERT"))
                .map(fmt_uw)
                .or_else(|| set.get_str(&format!("F_UW{i}_WERT")))
                .unwrap_or_default();
            uw.push(Uw {
                text: set.get_str(&tkey).unwrap_or_default().trim().to_string(),
                val,
                unit: set.get_str(&format!("F_UW{i}_EINH")).unwrap_or_default().trim().to_string(),
            });
        }
        // Fault types/causes: F_ART{i}_TEXT. Skip "--" and the high-numbered status-flag
        // entries (NR >= 0xEB, already shown as Status). Keep real causes.
        let mut causes = Vec::new();
        let mut sporadic = false;
        for i in 1..=64 {
            let key = format!("F_ART{i}_TEXT");
            if !set.contains(&key) {
                break;
            }
            let nr = set.get_i64(&format!("F_ART{i}_NR")).unwrap_or(0);
            let txt = set.get_str(&key).unwrap_or_default().trim().to_string();
            if txt.to_lowercase().contains("sporadisch") {
                sporadic = true;
            }
            if nr > 0 && nr < 0xEB && !txt.is_empty() && txt != "--" {
                causes.push(txt);
            }
        }
        out.push(Dtc {
            code: format!("{:04X}", (code_num as u32) & 0xffff),
            text,
            present: set.get_i64("F_VORHANDEN").unwrap_or(0) != 0,
            sporadic,
            raw: set.get_str("F_HEX_CODE").unwrap_or_default().trim().to_string(),
            hfk: set.get_i64("F_HFK").unwrap_or(0),
            lz: set.get_i64("F_LZ").unwrap_or(0),
            uw_satz: set.get_i64("F_UW_SATZ").unwrap_or(0),
            uw,
            causes,
        });
    }
    out
}

// ──────────────────────────────── render ─────────────────────────────────

/// Three-state fault status (active / inactive / sporadic) → (label, colour).
fn fault_status(c: &Colors, present: bool, sporadic: bool, lang: Lang) -> (String, Color32) {
    if sporadic {
        (t("sporadic", lang), c.warn)
    } else if present {
        (t("active", lang), c.warn)
    } else {
        (t("inactive", lang), c.fg_dim)
    }
}

fn fault_row(
    ui: &mut egui::Ui,
    c: &Colors,
    code: &str,
    desc: &str,
    slabel: &str,
    scol: Color32,
    expanded: bool,
) -> bool {
    egui::Frame::none()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.stroke))
        .rounding(3.0)
        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
        .outer_margin(egui::Margin { bottom: 6.0, ..Default::default() })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(if expanded { "▾" } else { "▸" }).size(11.0).color(c.fg_faint));
                ui.add_space(8.0);
                ui.label(RichText::new(code).size(13.0).strong().color(c.fg));
                ui.add_space(10.0);
                ui.label(RichText::new(desc).size(11.0).color(c.fg_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(slabel).size(10.0).color(scol));
                });
            });
        })
        .response
        .interact(Sense::click())
        .clicked()
}

/// One compact key/value cell for the 2-column detail grid.
struct KvCell {
    label: String,
    value: String,
    unit: String,
    color: Color32,
}

fn kv(label: &str, value: String, unit: &str, color: Color32) -> KvCell {
    KvCell { label: label.to_string(), value, unit: unit.to_string(), color }
}

/// Render key/value cells in a 2-column grid.
fn kv_grid(ui: &mut egui::Ui, c: &Colors, cells: &[KvCell]) {
    const CELL_W: f32 = 430.0;
    const GAP: f32 = 46.0;
    const ROW_H: f32 = 30.0;
    let grid_w = (2.0 * CELL_W + GAP).min(ui.available_width());
    let mut i = 0;
    while i < cells.len() {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(grid_w, ROW_H), Sense::hover());
        let pr = ui.painter_at(rect);
        for col in 0..2 {
            let idx = i + col;
            if idx >= cells.len() {
                break;
            }
            let cell = &cells[idx];
            let cx = rect.left() + col as f32 * (CELL_W + GAP);
            let cr = cx + CELL_W;
            let cy = rect.center().y;
            pr.text(Pos2::new(cx, cy), Align2::LEFT_CENTER, &cell.label, FontId::proportional(11.5), c.fg_dim);
            let vr = pr.text(Pos2::new(cr, cy), Align2::RIGHT_CENTER, &cell.value, FontId::monospace(12.5), cell.color);
            if !cell.unit.is_empty() {
                pr.text(Pos2::new(vr.left() - 5.0, cy), Align2::RIGHT_CENTER, &cell.unit, FontId::monospace(9.5), c.fg_faint);
            }
            pr.line_segment(
                [Pos2::new(cx, rect.bottom() - 1.0), Pos2::new(cr, rect.bottom() - 1.0)],
                Stroke::new(1.0, c.stroke),
            );
        }
        i += 2;
    }
}

fn real_fault_detail(ui: &mut egui::Ui, c: &Colors, d: &Dtc, lang: Lang) {
    egui::Frame::none()
        .inner_margin(egui::Margin { left: 24.0, right: 8.0, top: 4.0, bottom: 10.0 })
        .show(ui, |ui| {
            let (status, scol) = fault_status(c, d.present, d.sporadic, lang);
            ui.label(RichText::new(t("details", lang)).size(9.0).color(c.fg_faint));
            ui.add_space(2.0);
            kv_grid(ui, c, &[
                kv(&t("status", lang), status, "", scol),
                kv(&t("location_code", lang), format!("0x{}", d.code), "", c.fg),
                kv(&t("repetitions", lang), d.hfk.to_string(), "", c.fg),
                kv(&t("counter_lz", lang), d.lz.to_string(), "", c.fg),
            ]);

            if !d.causes.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(t("cause", lang)).size(9.0).color(c.fg_faint));
                ui.add_space(2.0);
                for cause in &d.causes {
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.label(RichText::new("•").size(11.0).color(c.accent));
                        ui.add_space(6.0);
                        ui.label(RichText::new(cause).size(11.5).color(c.fg));
                    });
                }
            }

            if !d.uw.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(t("freeze_frame", lang)).size(9.0).color(c.fg_faint));
                let satz = d.uw_satz.max(1) as usize;
                let per = if satz > 1 && d.uw.len() % satz == 0 { d.uw.len() / satz } else { d.uw.len() };
                let mut i = 0;
                while i < d.uw.len() {
                    let snap = i / per + 1;
                    if satz > 1 {
                        if snap > 1 {
                            ui.add_space(6.0);
                            let w = (2.0f32 * 430.0 + 46.0).min(ui.available_width());
                            let (r, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
                            ui.painter().hline(r.left()..=r.right(), r.center().y, Stroke::new(1.0, c.fg_faint));
                            ui.add_space(4.0);
                        } else {
                            ui.add_space(3.0);
                        }
                        ui.label(RichText::new(format!("{} {}", t("snapshot", lang), snap)).size(9.0).color(c.accent));
                    }
                    let end = (i + per).min(d.uw.len());
                    let cells: Vec<KvCell> =
                        d.uw[i..end].iter().map(|u| kv(&u.text, u.val.clone(), &u.unit, c.fg)).collect();
                    kv_grid(ui, c, &cells);
                    i = end;
                }
            }

            if !d.raw.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(t("raw_record", lang)).size(9.0).color(c.fg_faint));
                ui.label(RichText::new(&d.raw).size(10.0).monospace().color(c.fg_dim));
            }
        });
}
