//! Screens + shared chrome (header, status bar) and small paint helpers.

pub mod chassis_select;
pub mod ecu_select;

use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::app::App;
use crate::i18n::t;
use crate::lang::Lang;
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
    if app.is_connecting() {
        Link::Pending
    } else if app.connected_code().is_some() {
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

/// A settings button, painted as an "adjustments" icon (three sliders with knobs) so
/// it needs no icon font. Returns true when clicked.
fn gear_button(ui: &mut egui::Ui, c: &Colors) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(38.0, 32.0), Sense::click());
    let hovered = resp.hovered();
    let bg = if hovered { c.panel2 } else { c.panel };
    let p = ui.painter_at(rect);
    p.rect(rect.shrink(0.5), egui::Rounding::same(4.0), bg, Stroke::new(1.0, c.stroke));
    let col = if hovered { c.accent } else { c.fg_dim };
    let cx = rect.center().x;
    let (x0, x1) = (cx - 8.0, cx + 8.0);
    // Three horizontal rails, each with a knob at a different position.
    for (i, knob_x) in [cx + 3.0, cx - 4.0, cx + 5.0].into_iter().enumerate() {
        let y = rect.center().y + (i as f32 - 1.0) * 7.0;
        p.line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], Stroke::new(1.6, col));
        p.circle_filled(egui::pos2(knob_x, y), 2.6, col);
    }
    resp.clicked()
}

/// Top header: wordmark, interface chip, settings + theme toggles. Shared by screens.
pub fn header(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);
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
                    // Settings — gear opens the popup (language, theme, COM port).
                    if gear_button(ui, &c) {
                        app.open_settings();
                    }
                    ui.add_space(8.0);
                    // Theme toggle — painted sun/moon (no icon font needed on native).
                    if let Some(t) = theme_toggle(ui, &c, app.theme) {
                        app.theme = t;
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
                                ui.label(
                                    RichText::new(t("interface", app.lang)).size(10.0).color(c.fg_dim),
                                );
                                ui.label(RichText::new("D-CAN").size(10.0).color(c.fg));
                            });
                        });
                });
            });
        });
}

/// A dimmed field label in the settings grid's left column, right-aligned so the
/// dropdowns line up in the right column.
fn field_label(ui: &mut egui::Ui, c: &Colors, text: &str) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(RichText::new(text).size(12.0).color(c.fg_dim));
    });
}

/// A flat text button whose fill and text clearly react to hover and press (accent
/// on press). Painted so the click feedback is unmistakable, unlike a fixed-fill
/// `egui::Button`. Returns true when clicked.
fn flat_button(ui: &mut egui::Ui, c: &Colors, label: &str) -> bool {
    let font = egui::FontId::proportional(11.0);
    let measure = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), c.fg);
    let size = measure.size() + Vec2::new(24.0, 14.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let (bg, fg) = if resp.is_pointer_button_down_on() {
        (c.accent, c.accent_fg)
    } else if resp.hovered() {
        (c.panel2, c.accent)
    } else {
        (c.panel, c.fg_dim)
    };
    let p = ui.painter_at(rect);
    p.rect(rect, egui::Rounding::same(3.0), bg, Stroke::new(1.0, c.stroke));
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, fg);
    p.galley(rect.center() - galley.size() * 0.5, galley, fg);
    resp.clicked()
}

/// A small ✕ close button for the top-right corner of a popup. Highlights on hover
/// and press. Returns true when clicked.
fn close_cross(ui: &mut egui::Ui, c: &Colors) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    let (bg, col) = if resp.is_pointer_button_down_on() {
        (c.accent, c.accent_fg)
    } else if resp.hovered() {
        (c.panel2, c.fg)
    } else {
        (Color32::TRANSPARENT, c.fg_dim)
    };
    let p = ui.painter_at(rect);
    p.rect(rect, egui::Rounding::same(4.0), bg, Stroke::NONE);
    let ctr = rect.center();
    let r = 5.0;
    p.line_segment([ctr + Vec2::new(-r, -r), ctr + Vec2::new(r, r)], Stroke::new(1.6, col));
    p.line_segment([ctr + Vec2::new(-r, r), ctr + Vec2::new(r, -r)], Stroke::new(1.6, col));
    resp.clicked()
}

/// Settings popup: language, theme, and COM-port selection — three aligned
/// dropdowns. Rendered above every screen from `App::update`. Dims and blocks the
/// screen behind it; the backdrop or the top-right ✕ dismisses it. Persistence
/// happens in `App::update` (save-on-change): this only mutates lang/theme/port.
pub fn settings_modal(app: &mut App, ctx: &egui::Context) {
    if !app.show_settings {
        return;
    }
    let c = palette(app.theme);
    let lang = app.lang;
    let screen = ctx.screen_rect();
    const COMBO_W: f32 = 190.0;

    // Dim backdrop; a click outside the window closes settings.
    let backdrop = egui::Area::new(egui::Id::new("settings_backdrop"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(170));
            resp
        });
    if backdrop.inner.clicked() {
        app.show_settings = false;
    }

    let frame = egui::Frame::none()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.stroke2))
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(28.0, 24.0));

    egui::Window::new("settings_modal")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(380.0);

            // Title row: heading left, ✕ close top-right.
            ui.horizontal(|ui| {
                ui.label(RichText::new(t("settings", lang)).size(15.0).strong().color(c.fg));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if close_cross(ui, &c) {
                        app.show_settings = false;
                    }
                });
            });
            ui.add_space(20.0);

            let auto = t("auto", lang);
            egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([16.0, 16.0])
                .show(ui, |ui| {
                    // Language — shown with its own endonym (never translated).
                    field_label(ui, &c, &t("language", lang));
                    let lang_txt = match app.lang {
                        Lang::Ru => "Русский",
                        Lang::En => "English",
                    };
                    egui::ComboBox::from_id_salt("set_lang")
                        .selected_text(RichText::new(lang_txt).size(12.0).color(c.fg))
                        .width(COMBO_W)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut app.lang, Lang::Ru, "Русский");
                            ui.selectable_value(&mut app.lang, Lang::En, "English");
                        });
                    ui.end_row();

                    // Theme.
                    field_label(ui, &c, &t("theme", lang));
                    let theme_txt = if app.theme == Theme::Dark {
                        t("theme_dark", lang)
                    } else {
                        t("theme_light", lang)
                    };
                    egui::ComboBox::from_id_salt("set_theme")
                        .selected_text(RichText::new(theme_txt).size(12.0).color(c.fg))
                        .width(COMBO_W)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut app.theme, Theme::Dark, t("theme_dark", lang));
                            ui.selectable_value(&mut app.theme, Theme::Light, t("theme_light", lang));
                        });
                    ui.end_row();

                    // COM port — cross-platform list (Windows COMx / Linux /dev/tty*).
                    field_label(ui, &c, &t("com_port", lang));
                    ui.horizontal(|ui| {
                        let selected = app.port.clone().unwrap_or_else(|| auto.clone());
                        egui::ComboBox::from_id_salt("set_port")
                            .selected_text(RichText::new(selected).size(12.0).color(c.fg))
                            .width(COMBO_W)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut app.port, None, auto.as_str());
                                for p in app.ports.clone() {
                                    ui.selectable_value(&mut app.port, Some(p.clone()), p.as_str());
                                }
                            });
                        ui.add_space(8.0);
                        if flat_button(ui, &c, &t("refresh", lang)) {
                            app.refresh_ports();
                        }
                    });
                    ui.end_row();
                });

            // Hint: what "auto" resolves to, or that no ports are present.
            ui.add_space(8.0);
            let hint = if app.ports.is_empty() {
                t("no_ports", lang)
            } else if app.port.is_none() {
                format!("{} → {}", auto, app.ports.first().cloned().unwrap_or_default())
            } else {
                String::new()
            };
            if !hint.is_empty() {
                ui.label(RichText::new(hint).size(10.0).color(c.fg_faint));
            }
        });
}

