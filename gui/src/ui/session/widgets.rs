//! Paint primitives for the session constructor — all driven by `inpa` model data,
//! rendered in eDIAG's own design (not INPA's pixel layout).

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use inpa::model::{NavItem, NavTarget, Row, ScreenModule};

use super::{item_action, Act, View};
use crate::app::{App, Screen};
use crate::ecu::Module;
use crate::i18n::{t, tr_prg};
use crate::lang::Lang;
use crate::model::MeasFrame;
use crate::ui::theme::Colors;

/// Bottom F-key bar: a fixed strip of exactly 10 slots (F1..F10), stretched evenly across
/// the full width. Each slot holds the menu item with that F-key (INPA Exit is remapped to
/// our native exit); positions without a real function are shown but inert. No extra keys.
#[allow(clippy::too_many_arguments)]
pub(super) fn fkey_panel(
    ctx: &egui::Context,
    c: &Colors,
    page_items: &[NavItem],
    act: &mut Option<Act>,
    sm: &ScreenModule,
    app: &App,
    view: &View,
    lang: Lang,
) {
    let frame = egui::Frame::none()
        .fill(c.panel)
        .inner_margin(egui::Margin::symmetric(10.0, 8.0));
    egui::TopBottomPanel::bottom("fkeys").exact_height(60.0).frame(frame).show(ctx, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
        ui.columns(10, |cols| {
            for (i, col) in cols.iter_mut().enumerate() {
                let f = (i + 1) as u8;
                // A slot is functional only if its item maps to a real action (nav/job/exit);
                // scriptselect/editor (→ None) and empty labels fall back to an inert slot.
                let item = page_items.iter().find(|it| it.fkey == f);
                match item.filter(|it| !it.label.trim().is_empty()) {
                    Some(it) => match item_action(sm, app, view, it) {
                        Some(a) => {
                            if fkey_button(col, c, f, &tr_prg(&it.label, lang), &it.target) {
                                *act = Some(a);
                            }
                        }
                        None => empty_slot(col, c, f),
                    },
                    None => empty_slot(col, c, f),
                }
            }
        });
    });
}

fn target_color(c: &Colors, t: &NavTarget) -> Color32 {
    match t {
        NavTarget::Job(_) => c.ok,
        NavTarget::Exit | NavTarget::Back => c.fg_faint,
        _ => c.accent,
    }
}

/// A full-width slot frame filling its column; `body` paints the two label lines.
fn slot(ui: &mut egui::Ui, c: &Colors, fill: Color32, body: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    egui::Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0, c.stroke))
        .rounding(3.0)
        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(32.0);
            ui.vertical(body);
        })
        .response
}

fn fkey_button(ui: &mut egui::Ui, c: &Colors, f: u8, label: &str, t: &NavTarget) -> bool {
    slot(ui, c, c.panel2, |ui| {
        ui.label(RichText::new(format!("F{f}")).size(10.0).strong().color(target_color(c, t)));
        ui.label(RichText::new(label).size(11.0).color(c.fg).text_style(egui::TextStyle::Body));
    })
    .interact(Sense::click())
    .clicked()
}

/// A fixed but unused F-key position: shown for a stable 10-slot layout, non-interactive.
fn empty_slot(ui: &mut egui::Ui, c: &Colors, f: u8) {
    slot(ui, c, c.panel, |ui| {
        ui.label(RichText::new(format!("F{f}")).size(10.0).color(c.fg_faint));
        ui.label(RichText::new(" ").size(11.0));
    });
}

/// Top context strip: back-to-ECU, current title, breadcrumb, and link status.
pub(super) fn context_strip(
    ui: &mut egui::Ui,
    app: &mut App,
    c: &Colors,
    m: &Module,
    sm: &ScreenModule,
    view: &View,
) {
    let lang = app.lang;
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        if ui
            .add(
                egui::Button::new(RichText::new(format!("‹ {}", t("ecu_selection", lang))).size(11.0).color(c.fg_dim))
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::new(1.0, c.stroke)),
            )
            .clicked()
        {
            app.screen = Screen::Ecu;
        }
        ui.add_space(10.0);
        let title = view
            .screen
            .and_then(|s| sm.as_screen(s))
            .map(|s| s.title.clone())
            .or_else(|| sm.as_menu(view.menu).map(|mn| mn.title.clone()))
            .unwrap_or_default();
        ui.label(RichText::new(tr_prg(&title, lang)).size(13.0).strong().color(c.fg));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(format!("{} · {} · {}", m.code, sm.sgbd, m.bus)).size(10.0).color(c.fg_faint));
        });
    });
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);
}

/// A clickable menu row (list item in a menu view).
pub(super) fn menu_row(ui: &mut egui::Ui, c: &Colors, idx: usize, label: &str, t: &NavTarget) -> bool {
    let glyph = match t {
        NavTarget::Job(_) => "▶",
        NavTarget::Exit => "⏻",
        _ => "›",
    };
    egui::Frame::none()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.stroke))
        .rounding(3.0)
        .inner_margin(egui::Margin::symmetric(12.0, 9.0))
        .outer_margin(egui::Margin { bottom: 6.0, ..Default::default() })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{idx:02}")).size(11.0).color(c.accent));
                ui.add_space(10.0);
                ui.label(RichText::new(label).size(12.0).color(c.fg));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(glyph).size(12.0).color(target_color(c, t)));
                });
            });
        })
        .response
        .interact(Sense::click())
        .clicked()
}

/// A computed data-stream cell: an analog bar OR a logical On/Off state.
enum Cell {
    /// Analog / numeric — drawn as a scaled bar with value + unit.
    Bar { label: String, value: String, unit: String, pct: f32 },
    /// Logical — drawn as an On/Off state pill, never a bar. `on = None` = no data yet.
    State { label: String, text: String, on: Option<bool> },
}

/// Group consecutive rows that share an `.ipo` LINE index (INPA drew them on one line).
/// Returns `[start, end)` ranges so callers keep each row's GLOBAL index — the key into
/// the decoded `MeasFrame` (values are positional by `screen.rows` index).
fn group_ranges(rows: &[Row]) -> Vec<(usize, usize)> {
    let mut groups = Vec::new();
    let mut start = 0;
    for i in 1..=rows.len() {
        if i == rows.len() || rows[i].line() != rows[start].line() {
            groups.push((start, i));
            start = i;
        }
    }
    groups
}

/// Data-stream page: logical (On/Off pills) + analog (bars). Rows are laid out two-up,
/// preserving INPA's per-line pairing (`.ipo` LINE records), and stretched to fill the
/// whole viewport — the rows divide the available height evenly, bars span the full width.
pub(super) fn stream(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    c: &Colors,
    rows: &[Row],
    live: Option<&MeasFrame>,
    stale: bool,
    lang: Lang,
) {
    ctx.request_repaint();
    // Build the two-up rows first so we know how many there are.
    let cells_per_group: Vec<Vec<Cell>> = group_ranges(rows)
        .iter()
        .map(|&(s, e)| (s..e).filter_map(|i| cell_of(i, &rows[i], live, lang)).collect())
        .collect();
    let rows2: Vec<&[Cell]> = cells_per_group.iter().flat_map(|c| c.chunks(2)).collect();
    if rows2.is_empty() {
        return;
    }
    // Bars keep a FIXED height; the leftover space is spread as equal gaps *between* rows,
    // so the set fills the viewport top-to-bottom without stretching the bars themselves.
    // Keep a bottom margin so the last row doesn't touch the F-key bar.
    const BOTTOM_PAD: f32 = 24.0;
    let avail = (ui.available_height() - BOTTOM_PAD).max(ROW_H * rows2.len() as f32);
    let slack = avail - ROW_H * rows2.len() as f32;
    let gap = if rows2.len() > 1 { (slack / (rows2.len() as f32 - 1.0)).clamp(8.0, 400.0) } else { 0.0 };
    ui.spacing_mut().item_spacing.x = 24.0;
    for (i, chunk) in rows2.iter().enumerate() {
        if i > 0 {
            ui.add_space(gap);
        }
        ui.columns(2, |cols| {
            for (j, col) in cols.iter_mut().enumerate() {
                if let Some(cell) = chunk.get(j) {
                    draw_cell(col, c, ROW_H, stale, cell);
                }
            }
        });
    }
}

fn cell_of(idx: usize, row: &Row, live: Option<&MeasFrame>, lang: Lang) -> Option<Cell> {
    let label = tr_prg(row.label(), lang);
    let cell = live.and_then(|f| f.cell(idx));
    match row {
        // Logical: On/Off state, NOT a bar.
        Row::Logical { on, off, .. } => {
            let v = cell.and_then(|c| c.num);
            let (text, state) = match v {
                Some(x) if x >= 0.5 => (on.trim().to_string(), Some(true)),
                Some(_) => (off.trim().to_string(), Some(false)),
                None => ("—".to_string(), None),
            };
            Some(Cell::State { label, text, on: state })
        }
        // Analog: scaled bar.
        Row::Analog { scale, unit, .. } => {
            let v = cell.and_then(|c| c.num);
            // Unit: decoded (dynamic `_EINH` else the row's static unit) when a frame is
            // present; the row's own static unit before the first frame lands.
            let unit = cell
                .map(|c| c.unit.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| unit.clone());
            let (value, pct) = match v {
                Some(x) => {
                    let pct = if scale.max > scale.min {
                        ((x - scale.min) / (scale.max - scale.min)).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    (fmt_num(x), pct)
                }
                None => ("—".to_string(), 0.0),
            };
            Some(Cell::Bar { label, value, unit, pct })
        }
        _ => None,
    }
}

fn fmt_num(v: f64) -> String {
    let a = v.abs();
    if a >= 100.0 {
        format!("{v:.0}")
    } else if a >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

fn draw_cell(ui: &mut egui::Ui, c: &Colors, h: f32, stale: bool, cell: &Cell) {
    match cell {
        Cell::Bar { label, value, unit, pct } => draw_bar(ui, c, h, stale, label, value, unit, *pct),
        Cell::State { label, text, on } => draw_state(ui, c, h, stale, label, text, *on),
    }
}

/// Width reserved to the right of a bar for its numeric value.
const VAL_W: f32 = 120.0;
/// Fixed row height (caption + bar); rows never stretch — the gaps between them do.
const ROW_H: f32 = 74.0;
/// Fixed bar-track height.
const BAR_H: f32 = 34.0;
/// Caption (label/unit) font size.
const CAP_SZ: f32 = 22.0;
/// Value / state font size.
const VAL_SZ: f32 = 28.0;

fn draw_bar(ui: &mut egui::Ui, c: &Colors, h: f32, stale: bool, label: &str, value: &str, unit: &str, pct: f32) {
    let w = ui.available_width();
    let (cell, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let pr = ui.painter_at(cell);
    let left = cell.left();
    let bar_w = (w - VAL_W).max(120.0);
    let ty = cell.top() + 15.0;
    pr.text(Pos2::new(left, ty), Align2::LEFT_CENTER, label, FontId::monospace(CAP_SZ), c.fg);
    if !unit.is_empty() {
        pr.text(Pos2::new(left + bar_w, ty), Align2::RIGHT_CENTER, unit, FontId::monospace(CAP_SZ), c.fg_dim);
    }
    // Fixed-height bar just below the caption.
    let top = cell.top() + 34.0;
    let track = Rect::from_min_max(Pos2::new(left, top), Pos2::new(left + bar_w, top + BAR_H));
    pr.rect_filled(track, 3.0, c.stroke);
    let fill = if stale { c.fg_faint } else { c.accent };
    pr.rect_filled(Rect::from_min_size(track.min, Vec2::new(bar_w * pct, track.height())), 3.0, fill);
    let vcol = if stale { c.fg_dim } else { c.fg }; // held (old) values read a touch greyer
    pr.text(Pos2::new(left + bar_w + 14.0, track.center().y), Align2::LEFT_CENTER, value, FontId::monospace(VAL_SZ), vcol);
}

/// Logical state: label + a coloured On/Off pill (indicator dot + text). Never a bar.
fn draw_state(ui: &mut egui::Ui, c: &Colors, h: f32, stale: bool, label: &str, text: &str, on: Option<bool>) {
    let w = ui.available_width();
    let (cell, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let pr = ui.painter_at(cell);
    let left = cell.left();
    let cy = cell.center().y;
    pr.text(Pos2::new(left, cy), Align2::LEFT_CENTER, label, FontId::monospace(CAP_SZ), c.fg);
    // Right-aligned pill: coloured dot + state text (greyer while the value is held/old).
    let fg = if stale { c.fg_dim } else { c.fg };
    let (dot, txt) = match on {
        Some(true) => (c.ok, fg),
        Some(false) => (c.fg_faint, fg),
        None => (c.stroke, c.fg_dim),
    };
    let right = left + w;
    pr.text(Pos2::new(right, cy), Align2::RIGHT_CENTER, text, FontId::monospace(VAL_SZ), txt);
    let tw = text.chars().count() as f32 * 15.0;
    pr.circle_filled(Pos2::new(right - tw - 16.0, cy), 7.0, dot);
}

/// Text-info page (ident/coding): each row = label ↔ result value, laid out two-up by
/// INPA line-grouping (`.ipo` LINE records) so paired ident fields sit on one line.
pub(super) fn textinfo(
    ui: &mut egui::Ui,
    c: &Colors,
    rows: &[Row],
    info: &[inpa::InfoLine],
    live: Option<&MeasFrame>,
    app: &App,
    lang: Lang,
) {
    // An ftextout-only screen (e.g. `s_info`) has no job-bound rows — render its static
    // "about the SGBD" lines instead.
    if rows.is_empty() && !info.is_empty() {
        for line in info {
            info_row(ui, c, line, lang);
        }
        return;
    }
    // Data page (ident / coding): surface why it's empty rather than showing silent dashes.
    let have_data = live.is_some_and(|f| f.has_data());
    if !have_data {
        if app.info_busy {
            ui.label(RichText::new(t("reading_data", lang)).size(12.0).color(c.accent));
        } else if !app.status_msg.is_empty() {
            ui.label(RichText::new(app.status_msg.clone()).size(12.0).color(c.err));
        } else if let Some(f) = live {
            // The job answered but none of our result names matched — surface what it
            // actually returned so a name mismatch is diagnosable at a glance.
            if f.raw_names.is_empty() {
                ui.label(RichText::new(t("empty_response", lang)).size(12.0).color(c.warn));
            } else {
                ui.label(RichText::new(t("no_fields", lang)).size(12.0).color(c.warn));
                ui.label(RichText::new(f.raw_names.join(", ")).size(11.0).monospace().color(c.fg_dim));
            }
        } else if app.info_tries > 0 {
            ui.label(RichText::new(t("no_data", lang)).size(12.0).color(c.warn));
        }
        ui.add_space(6.0);
    }
    for (s, e) in group_ranges(rows) {
        let items: Vec<(usize, &Row)> =
            (s..e).filter(|&i| matches!(rows[i], Row::Text { .. })).map(|i| (i, &rows[i])).collect();
        for chunk in items.chunks(2) {
            ui.columns(2, |cols| {
                for (j, col) in cols.iter_mut().enumerate() {
                    if let Some((idx, row)) = chunk.get(j) {
                        kv_line(col, c, *idx, row, live, lang);
                    }
                }
            });
        }
    }
}

/// One static info line: a bold heading, or a `label ……… value` field (runtime SGBD
/// values we can't resolve statically show as a dash).
fn info_row(ui: &mut egui::Ui, c: &Colors, line: &inpa::InfoLine, lang: Lang) {
    let label = tr_prg(&line.label, lang);
    if matches!(line.value, inpa::InfoValue::Heading) {
        ui.add_space(8.0);
        ui.label(RichText::new(label).size(13.0).strong().color(c.fg));
        ui.add_space(2.0);
        return;
    }
    let value = match &line.value {
        inpa::InfoValue::Const(v) => tr_prg(v, lang),
        _ => "—".to_string(),
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.text(Pos2::new(rect.left(), rect.center().y), Align2::LEFT_CENTER, &label, FontId::proportional(12.5), c.fg_dim);
    p.text(Pos2::new(rect.right(), rect.center().y), Align2::RIGHT_CENTER, &value, FontId::monospace(12.5), c.fg);
    p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, c.stroke));
}

/// One `label ……… value` line with an underline.
fn kv_line(ui: &mut egui::Ui, c: &Colors, idx: usize, row: &Row, live: Option<&MeasFrame>, lang: Lang) {
    let Row::Text { .. } = row else { return };
    let label = tr_prg(row.label(), lang);
    let value = live
        .and_then(|f| f.cell(idx))
        .and_then(|cell| cell.text.clone())
        .unwrap_or_else(|| "—".into());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.text(Pos2::new(rect.left(), rect.center().y), Align2::LEFT_CENTER, &label, FontId::proportional(11.5), c.fg_dim);
    p.text(Pos2::new(rect.right(), rect.center().y), Align2::RIGHT_CENTER, &value, FontId::monospace(12.0), c.fg);
    p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, c.stroke));
}

/// Activation page: list the activation actions (label + job). Running is via the menu/F-keys.
pub(super) fn activation(ui: &mut egui::Ui, c: &Colors, rows: &[Row], lang: Lang) {
    ui.label(RichText::new(t("activations_hint", lang)).size(11.0).color(c.fg_faint));
    ui.add_space(6.0);
    for row in rows {
        let Row::Action { job, .. } = row else { continue };
        let label = tr_prg(row.label(), lang);
        ui.horizontal(|ui| {
            ui.label(RichText::new("▶").size(11.0).color(c.ok));
            ui.add_space(8.0);
            ui.label(RichText::new(label).size(12.0).color(c.fg));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(&job.job).size(10.0).monospace().color(c.fg_faint));
            });
        });
    }
}
