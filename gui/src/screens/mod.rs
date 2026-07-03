//! Screens + shared chrome (header, status bar) and small paint helpers.

pub mod chassis_select;
pub mod ecu_select;

use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::app::App;
use crate::lang::{dict, Lang};
use crate::theme::{palette, Colors, Theme};

/// A filled status dot of diameter `d`.
pub fn dot(ui: &mut egui::Ui, color: Color32, d: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(d), Sense::hover());
    ui.painter().circle_filled(rect.center(), d / 2.0, color);
}

/// The connection lifecycle phase reflected by the header link chip.
#[derive(Clone, Copy)]
enum Link {
    /// No attempt yet / disconnected quietly.
    Idle,
    /// `INITIALISIERUNG` in flight.
    Pending,
    /// Linked — winks on real comms.
    Up,
    /// Linked but the last poll(s) missed — values held, link not yet dropped.
    Stall,
    /// Link lost / error.
    Down,
}

fn link_state(app: &App) -> Link {
    if app.connect_pending {
        Link::Pending
    } else if app.connected.is_some() {
        if app.comms_miss > 0 {
            Link::Stall
        } else {
            Link::Up
        }
    } else if !app.status_msg.is_empty() {
        Link::Down
    } else {
        Link::Idle
    }
}

/// Colour of the header link dot: grey when idle, a breathing amber during
/// INITIALISIERUNG, a bright green wink on each real exchange, solid red on loss.
fn link_dot(app: &mut App, ctx: &egui::Context, c: &Colors, link: Link) -> Color32 {
    let now = ctx.input(|i| i.time);
    match link {
        Link::Idle => c.fg_faint,
        Link::Down => c.err,
        Link::Stall => c.warn, // holding last values through a transient glitch

        Link::Pending => {
            let t = 0.5 + 0.5 * (now as f32 * 5.0).sin(); // slow breathe
            ctx.request_repaint();
            let bright = egui::Rgba::from(c.accent);
            Color32::from(egui::lerp(bright * 0.3..=bright, t))
        }
        Link::Up => {
            if app.comms_seq != app.comms_seen {
                app.comms_seen = app.comms_seq;
                app.comms_at = now;
            }
            // 1.0 right after an exchange → 0.0 after ~180 ms (a wink per completed poll).
            let age = (now - app.comms_at) as f32;
            let t = (1.0 - age / 0.18).clamp(0.0, 1.0);
            if age < 0.6 {
                ctx.request_repaint(); // keep the wink smooth while activity is fresh
            }
            let bright = egui::Rgba::from(c.ok);
            Color32::from(egui::lerp(bright * 0.28..=bright, t))
        }
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

/// A single icon toggle for the theme, painted with shapes so it needs no icon font
/// (native-safe): a moon while Dark is active, a sun while Light is. Click flips the theme.
fn theme_toggle(ui: &mut egui::Ui, c: &Colors, theme: Theme) -> Option<Theme> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(44.0, 32.0), Sense::click());
    let hovered = resp.hovered();
    let bg = if hovered { c.panel2 } else { c.panel };
    let p = ui.painter_at(rect);
    p.rect(rect.shrink(0.5), egui::Rounding::same(4.0), bg, Stroke::new(1.0, c.stroke));
    let ctr = rect.center();
    let col = c.accent;
    match theme {
        // Moon: a disc with a bite carved out by a second, background-coloured disc.
        Theme::Dark => {
            p.circle_filled(ctr, 8.0, col);
            p.circle_filled(ctr + Vec2::new(4.0, -3.0), 8.0, bg);
        }
        // Sun: a core disc with eight rays.
        Theme::Light => {
            p.circle_filled(ctr, 5.5, col);
            for k in 0..8 {
                let a = std::f32::consts::TAU * k as f32 / 8.0;
                let dir = Vec2::new(a.cos(), a.sin());
                p.line_segment([ctr + dir * 8.0, ctr + dir * 12.0], Stroke::new(1.8, col));
            }
        }
    }
    resp.clicked().then(|| if theme == Theme::Dark { Theme::Light } else { Theme::Dark })
}

/// Top header: wordmark, interface chip, RU/EN + theme toggles. Shared by screens.
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
                ui.label(RichText::new("eDIAG").size(15.0).strong().color(c.fg));

                // Right cluster
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Theme toggle — painted sun/moon (no icon font needed on native).
                    if let Some(t) = theme_toggle(ui, &c, app.theme) {
                        app.theme = t;
                    }
                    ui.add_space(8.0);
                    // Language toggle
                    if let Some(left) = seg2(ui, &c, "RU", "EN", app.lang == Lang::Ru) {
                        app.lang = if left { Lang::Ru } else { Lang::En };
                    }
                    ui.add_space(12.0);
                    // Interface chip — the single source of truth for link state: it winks
                    // with real comms activity and shows the current lifecycle phase.
                    // The dot alone conveys link state (grey idle / amber init / green
                    // wink on comms / red loss) — no redundant status text.
                    let link = link_state(app);
                    let dot_col = link_dot(app, ctx, &c, link);
                    egui::Frame::none()
                        .fill(c.panel2)
                        .stroke(Stroke::new(1.0, c.stroke))
                        .rounding(3.0)
                        .inner_margin(egui::Margin::symmetric(11.0, 5.0))
                        .show(ui, |ui| {
                            ui.horizontal_centered(|ui| {
                                dot(ui, dot_col, 8.0);
                                ui.add_space(6.0);
                                ui.label(RichText::new(d.interface).size(10.0).color(c.fg_dim));
                                ui.label(RichText::new("· D-CAN").size(10.0).color(c.fg));
                            });
                        });
                });
            });
        });
}

