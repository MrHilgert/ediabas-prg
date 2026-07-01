//! Screen 1 — Chassis Select. UI-only (no Session/serial).
//! Per the updated handoff: NO right panel. Single click highlights a card;
//! double click opens the chassis (→ ECU Select).

use egui::{Align2, Color32, FontId, Pos2, RichText, Rounding, Sense, Stroke, Vec2};

use super::{dot, header, status_bar, status_color};
use crate::app::App;
use crate::data::{self, DATA, SERIES};
use crate::lang::dict;
use crate::theme::palette;

pub fn show(app: &mut App, ctx: &egui::Context) {
    header(app, ctx);
    status_bar(app, ctx);
    series_rail(app, ctx);
    chassis_grid(app, ctx);
}

fn series_rail(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);
    let d = dict(app.lang);
    let searching = !app.query.trim().is_empty();
    let frame = egui::Frame::none().fill(c.panel);

    egui::SidePanel::left("series")
        .exact_width(186.0)
        .resizable(false)
        .frame(frame)
        .show(ctx, |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(RichText::new(d.series).size(10.0).strong().color(c.fg_dim));
            });
            ui.add_space(9.0);
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(6.0);
                for s in SERIES {
                    let active = !searching && app.series == s;
                    if series_row(ui, &c, s, data::series_name(app.lang, s),
                                  data::series_count(s), active)
                    {
                        app.series = s;
                        app.query.clear();
                        if let Some(idx) = DATA.iter().position(|ch| ch.series == s) {
                            app.select_chassis(idx);
                        }
                    }
                }
            });

            // Legend pinned to the bottom.
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.add_space(11.0);
                for (col, label) in [
                    (c.ok, d.lg_ready),
                    (c.warn, d.lg_partial),
                    (c.faint_dot, d.lg_none),
                ] {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        dot(ui, col, 8.0);
                        ui.add_space(7.0);
                        ui.label(RichText::new(label).size(10.0).color(c.fg_dim));
                    });
                    ui.add_space(7.0);
                }
                ui.separator();
            });
        });
}

fn series_row(
    ui: &mut egui::Ui,
    c: &crate::theme::Colors,
    s: char,
    name: &str,
    count: usize,
    active: bool,
) -> bool {
    // No fill — the accent left bar is the (subtle) active indicator.
    let inner = egui::Frame::none()
        .fill(Color32::TRANSPARENT)
        .inner_margin(egui::Margin::symmetric(11.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (bar, _) = ui.allocate_exact_size(Vec2::new(3.0, 18.0), Sense::hover());
                if active {
                    ui.painter().rect_filled(bar, 0.0, c.accent);
                }
                ui.add_space(5.0);
                ui.label(
                    RichText::new(s.to_string())
                        .size(15.0)
                        .strong()
                        .color(if active { c.fg } else { c.fg_dim }),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(name)
                        .size(11.0)
                        .color(if active { c.fg } else { c.fg_dim }),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(count.to_string()).size(10.0).color(c.fg_faint));
                });
            });
        });
    inner.response.interact(Sense::click()).clicked()
}

fn chassis_grid(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);
    let d = dict(app.lang);
    let frame = egui::Frame::none().fill(c.bg);

    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        // Toolbar
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(d.select_chassis).size(12.0).strong().color(c.fg));
            ui.label(RichText::new(format!("/ {}", d.e_series)).size(11.0).color(c.fg_faint));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(14.0);
                let n = filtered(app).len();
                ui.label(RichText::new(format!("{n} · {}", d.results)).size(10.0).color(c.fg_faint));
                ui.add_space(10.0);
                ui.label(RichText::new(d.open_hint).size(10.0).color(c.fg_faint));
                ui.add_space(12.0);
                egui::Frame::none()
                    .fill(c.panel2)
                    .stroke(Stroke::new(1.0, c.stroke))
                    .rounding(3.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new("›").size(12.0).color(c.accent));
                        ui.add_space(7.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut app.query)
                                .desired_width(190.0)
                                .frame(false)
                                .hint_text(RichText::new(d.search_ph).color(c.fg_faint)),
                        );
                    });
            });
        });
        ui.add_space(8.0);
        ui.separator();

        // Cards: single click = highlight, double click = open.
        let matches = filtered(app);
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::none().inner_margin(14.0).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(10.0);
                for idx in matches {
                    let resp = chassis_card(ui, app, idx);
                    if resp.double_clicked() {
                        app.select_chassis(idx);
                        app.enter_ecu();
                    } else if resp.clicked() {
                        app.select_chassis(idx);
                    }
                }
            });
            });
        });
    });
}

/// Indices of chassis matching the current series/search filter.
fn filtered(app: &App) -> Vec<usize> {
    let searching = !app.query.trim().is_empty();
    DATA.iter()
        .enumerate()
        .filter(|(_, ch)| {
            if searching {
                data::matches_query(ch, &app.query)
            } else {
                ch.series == app.series
            }
        })
        .map(|(i, _)| i)
        .collect()
}

/// Fully manual card paint on a fixed-size rect. `horizontal_wrapped` staggers
/// Frame-based items; allocating an exact rect and painting into it guarantees
/// identical, top-aligned cards regardless of content.
fn chassis_card(ui: &mut egui::Ui, app: &App, idx: usize) -> egui::Response {
    const W: f32 = 154.0;
    const H: f32 = 106.0;
    let c = palette(app.theme);
    let ch = &DATA[idx];
    let selected = app.chassis == Some(idx);

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(W, H), Sense::click());
    // Selection = accent outline only (no fill) — subtle.
    let stroke = Stroke::new(1.0, if selected { c.accent } else { c.stroke });
    // Draw the frame with the UNCLIPPED painter (shrink 0.5px so the 1px stroke
    // sits crisply inside) — clipping to `rect` would shave the outer half of the
    // border and make it look crooked. Text uses the clipped painter below.
    ui.painter().rect(rect.shrink(0.5), Rounding::same(3.0), c.panel, stroke);
    let p = ui.painter_at(rect); // clips text to the card

    let l = rect.left() + 11.0;
    let r = rect.right() - 11.0;
    let mono = |sz: f32| FontId::monospace(sz);

    // Header: code + status dot
    p.text(Pos2::new(l, rect.top() + 9.0), Align2::LEFT_TOP, ch.code, mono(21.0), c.fg);
    p.circle_filled(Pos2::new(r - 4.0, rect.top() + 15.0), 4.0, status_color(&c, ch.status));
    // Model + years
    p.text(Pos2::new(l, rect.top() + 38.0), Align2::LEFT_TOP, ch.name(app.lang), mono(11.0), c.fg_dim);
    p.text(Pos2::new(l, rect.top() + 54.0), Align2::LEFT_TOP, ch.years, mono(11.0), c.fg_faint);
    // Bottom row + separator above it
    let sep_y = rect.bottom() - 26.0;
    p.line_segment([Pos2::new(l, sep_y), Pos2::new(r, sep_y)], Stroke::new(1.0, c.stroke));
    let by = rect.bottom() - 17.0;
    p.text(Pos2::new(l, by), Align2::LEFT_TOP, ch.bodies.join("·"), mono(10.0), c.fg_dim);
    p.text(Pos2::new(r, by), Align2::RIGHT_TOP, format!("{} ECU", ch.ecus), mono(10.0), c.fg_faint);

    resp
}
