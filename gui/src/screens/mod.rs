//! Screens + shared chrome (header, status bar) and small paint helpers.

pub mod chassis_select;
pub mod ecu_select;
pub mod session;

use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::app::{clock, App};
use crate::data::Status;
use crate::lang::{dict, Lang};
use crate::theme::{palette, Colors, Theme};

/// A filled status dot of diameter `d`.
pub fn dot(ui: &mut egui::Ui, color: Color32, d: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(d), Sense::hover());
    ui.painter().circle_filled(rect.center(), d / 2.0, color);
}

pub fn status_color(c: &Colors, s: Status) -> Color32 {
    match s {
        Status::Ready => c.ok,
        Status::Partial => c.warn,
        Status::None => c.faint_dot,
    }
}

/// A two-option segmented toggle (RU/EN, Dark/Light). Returns true if `left` picked.
fn seg2(
    ui: &mut egui::Ui,
    c: &Colors,
    left: &str,
    right: &str,
    left_active: bool,
) -> Option<bool> {
    let mut picked = None;
    egui::Frame::none()
        .stroke(Stroke::new(1.0, c.stroke))
        .rounding(3.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (label, active, is_left) in
                [(left, left_active, true), (right, !left_active, false)]
            {
                let txt = RichText::new(label)
                    .size(11.0)
                    .color(if active { c.accent } else { c.fg_dim })
                    .strong();
                let btn = egui::Button::new(txt)
                    .fill(if active { c.panel2 } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .rounding(3.0);
                if ui.add(btn).clicked() {
                    picked = Some(is_left);
                }
            }
        });
    picked
}

/// Top header: logo, title, interface chip, RU/EN + theme toggles. Shared by screens.
pub fn header(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);
    let d = dict(app.lang);
    let frame = egui::Frame::none()
        .fill(c.panel)
        .inner_margin(egui::Margin::symmetric(14.0, 8.0));

    egui::TopBottomPanel::top("header")
        .exact_height(48.0)
        .frame(frame)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                // Logo placeholder "e/"
                egui::Frame::none()
                    .stroke(Stroke::new(1.0, c.stroke2))
                    .rounding(3.0)
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("e/").size(13.0).strong().color(c.accent));
                    });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new("eDIAG").size(14.0).strong().color(c.fg));
                    ui.label(RichText::new(d.title_sub).size(9.0).color(c.fg_dim));
                });

                // Right cluster
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Theme toggle (☾ / ☀)
                    if let Some(left) = seg2(ui, &c, "D", "L", app.theme == Theme::Dark) {
                        app.theme = if left { Theme::Dark } else { Theme::Light };
                    }
                    ui.add_space(8.0);
                    // Language toggle
                    if let Some(left) = seg2(ui, &c, "RU", "EN", app.lang == Lang::Ru) {
                        app.lang = if left { Lang::Ru } else { Lang::En };
                    }
                    ui.add_space(12.0);
                    // Interface chip (mock — TODO: real adapter/link state)
                    egui::Frame::none()
                        .fill(c.panel2)
                        .stroke(Stroke::new(1.0, c.stroke))
                        .rounding(3.0)
                        .inner_margin(egui::Margin::symmetric(11.0, 5.0))
                        .show(ui, |ui| {
                            ui.horizontal_centered(|ui| {
                                dot(ui, c.ok, 8.0);
                                ui.add_space(6.0);
                                ui.label(RichText::new(d.interface).size(10.0).color(c.fg_dim));
                                ui.label(RichText::new("· D-CAN ·").size(10.0).color(c.fg));
                                ui.label(RichText::new(d.connected).size(10.0).strong().color(c.ok));
                            });
                        });
                });
            });
        });
}

/// Bottom status bar. `vbat`/`ping` shown as "—" until an actual session exists.
pub fn status_bar(app: &App, ctx: &egui::Context) {
    let c = palette(app.theme);
    let d = dict(app.lang);
    let frame = egui::Frame::none()
        .fill(c.panel)
        .inner_margin(egui::Margin::symmetric(12.0, 0.0));

    egui::TopBottomPanel::bottom("status")
        .exact_height(26.0)
        .frame(frame)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let sep = |ui: &mut egui::Ui| {
                    ui.add_space(10.0);
                    ui.label(RichText::new("│").size(10.0).color(c.stroke2));
                    ui.add_space(10.0);
                };
                let dim = |s: String| RichText::new(s).size(10.0).color(c.fg_dim);
                ui.label(dim(format!("{}: OBDLINK EX", d.adapter)));
                sep(ui);
                ui.label(dim("PROTO: DS2".into()));
                sep(ui);
                ui.label(dim("KL15: —".into()));
                sep(ui);
                ui.label(dim("VBAT: — V".into()));
                sep(ui);
                ui.label(dim("PING: — ms".into()));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(dim(clock()));
                });
            });
        });
}
