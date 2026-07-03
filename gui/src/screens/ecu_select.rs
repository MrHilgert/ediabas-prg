//! Screen 2 — ECU Select. Dense module table for the chosen chassis.
//! Single click selects (right card); double click / CONNECT → session (screen 3).

use egui::{Align2, Color32, FontId, Pos2, RichText, Rounding, Sense, Stroke, Vec2};

use super::header;
use crate::app::{App, Screen};
use crate::ecu::{mods_for, Category, Module};
use crate::i18n::t;
use crate::lang::Lang;
use crate::theme::{palette, Colors};
use crate::worker::Event;

pub fn show(app: &mut App, ctx: &egui::Context) {
    // A real connection attempt (INITIALISIERUNG) runs here, on screen 2: only a
    // successful link opens the session screen; a failure stays put with an error.
    poll_connect(app, ctx);
    header(app, ctx);
    category_rail(app, ctx);
    module_table(app, ctx);
    // Modal overlay: waiting-for-link spinner, or the connection error.
    connect_modal(app, ctx);
}

/// Watch the worker while a connection is in flight. `Connected` opens the
/// session screen; `Error` keeps us here and surfaces the reason. Events unrelated
/// to the connect handshake (job results) are ignored on this screen.
fn poll_connect(app: &mut App, ctx: &egui::Context) {
    if let Some(w) = &app.worker {
        for evt in w.poll() {
            match evt {
                Event::Connected { .. } => {
                    app.connected = app.ecu_sel;
                    app.connect_pending = false;
                    app.status_msg.clear();
                    app.screen = Screen::Session; // link established → open session
                }
                Event::Error(e) => {
                    app.connected = None;
                    app.connect_pending = false;
                    // Show a clean, user-facing reason (raw detail stays in logs).
                    app.connect_error = Some(humanize_connect_error(&e, app.lang));
                }
                Event::Disconnected => {
                    app.connected = None;
                    app.connect_pending = false;
                }
                Event::JobDone { .. } | Event::PollMiss(_) => {}
            }
        }
    }
    if app.connect_pending {
        ctx.request_repaint(); // keep spinning until the handshake resolves
    }
}

/// Turn a raw worker/transport error into a short, user-facing reason. The
/// technical string (VM/IO detail) stays in the trace; the popup shows only this.
fn humanize_connect_error(raw: &str, lang: Lang) -> String {
    let low = raw.to_ascii_lowercase();
    let key = if low.contains("timed out") || low.contains("timeout")
        || low.contains("no response") || low.contains("не отвеч")
    {
        "err_no_response"
    } else if low.contains("denied") || low.contains("отказано")
        || low.contains("in use") || low.contains("занят")
    {
        "err_port_busy"
    } else if low.contains("not found") || low.contains("cannot find")
        || low.contains("no such") || low.contains("не найд")
    {
        "err_no_adapter"
    } else {
        "err_generic"
    };
    t(key, lang)
}

fn mods(app: &App) -> Vec<Module> {
    match app.selected_chassis() {
        Some(ch) => mods_for(ch),
        None => Vec::new(),
    }
}

fn category_rail(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);
    let all = mods(app);
    let frame = egui::Frame::none().fill(c.panel);

    egui::SidePanel::left("categories")
        .exact_width(186.0)
        .resizable(false)
        .frame(frame)
        .show(ctx, |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(RichText::new(t("category", app.lang)).size(10.0).strong().color(c.fg_dim));
            });
            ui.add_space(9.0);
            ui.separator();
            ui.add_space(6.0);

            let row = |ui: &mut egui::Ui, active: bool, label: &str, count: usize| -> bool {
                // No fill — accent left bar marks the active category.
                egui::Frame::none()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(15.0, 8.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if active {
                                let (bar, _) =
                                    ui.allocate_exact_size(Vec2::new(3.0, 14.0), Sense::hover());
                                ui.painter().rect_filled(bar, 0.0, c.accent);
                                ui.add_space(4.0);
                            }
                            ui.label(
                                RichText::new(label)
                                    .size(11.0)
                                    .color(if active { c.fg } else { c.fg_dim }),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(count.to_string()).size(10.0).color(c.fg_faint));
                            });
                        });
                    })
                    .response
                    .interact(Sense::click())
                    .clicked()
            };

            for cat in Category::ALL {
                let n = all.iter().filter(|m| m.cat == cat).count();
                if row(ui, app.ecu_cat == Some(cat), cat.label(app.lang), n) {
                    app.ecu_cat = Some(cat);
                }
            }
        });
}

fn module_table(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);
    let all = mods(app);
    let chassis_code = app.selected_chassis().map(|ch| ch.code).unwrap_or("");
    let visible: Vec<&Module> = all
        .iter()
        .filter(|m| app.ecu_cat.map_or(true, |cat| m.cat == cat))
        .collect();
    let sgbd_total = all.iter().filter(|m| m.connectable()).count();
    let frame = egui::Frame::none().fill(c.bg);

    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        // Toolbar
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            if ui
                .add(
                    egui::Button::new(RichText::new(format!("‹ {}", t("back_chassis", app.lang))).size(11.0).color(c.fg_dim))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0, c.stroke)),
                )
                .clicked()
            {
                app.screen = Screen::Chassis;
            }
            ui.add_space(10.0);
            ui.label(RichText::new(t("select_ecu", app.lang)).size(12.0).strong().color(c.fg));
            ui.label(RichText::new(format!("/ {chassis_code}")).size(11.0).color(c.fg_faint));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(14.0);
                // CONNECT the selected module (the mock detail panel was removed; a
                // double-click on a card connects too).
                let connectable = app
                    .ecu_sel
                    .and_then(|code| all.iter().find(|m| m.code == code))
                    .map_or(false, |m| m.connectable());
                let btn = egui::Button::new(
                    RichText::new(format!("▸ {}", t("connect", app.lang))).size(11.0).strong().color(c.accent_fg),
                )
                .fill(c.accent)
                .rounding(3.0)
                .min_size(Vec2::new(128.0, 26.0));
                if ui.add_enabled(connectable, btn).clicked() {
                    app.start_connect(ctx);
                }
                ui.add_space(12.0);
                if sgbd_total > 0 {
                    ui.label(RichText::new(format!("{sgbd_total} SGBD")).size(10.0).color(c.fg_faint));
                }
            });
        });
        ui.add_space(8.0);
        ui.separator();

        // Card grid: single click selects (right detail), double click connects.
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::none().inner_margin(14.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(10.0);
                    for m in &visible {
                        let resp = module_card(ui, app, &c, m);
                        if resp.double_clicked() {
                            app.ecu_sel = Some(m.code);
                            app.start_connect(ctx);
                        } else if resp.clicked() && app.ecu_sel != Some(m.code) {
                            app.ecu_sel = Some(m.code);
                            app.status_msg.clear(); // drop any stale error from another module
                        }
                    }
                });
            });
        });
    });
}

/// One ECU as a card: the human-readable system name as the title (wrapped to ≤2 rows
/// with an ellipsis so it never spills past the card) over the code as a secondary line.
/// Selection = accent outline.
fn module_card(ui: &mut egui::Ui, app: &App, c: &Colors, m: &Module) -> egui::Response {
    const W: f32 = 228.0;
    const H: f32 = 72.0;
    let selected = app.ecu_sel == Some(m.code);

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(W, H), Sense::click());
    let stroke = Stroke::new(1.0, if selected { c.accent } else { c.stroke });
    let fill = if selected { c.panel2 } else { c.panel };
    // Frame on the unclipped painter (0.5px inset for a crisp border); text clipped.
    ui.painter().rect(rect.shrink(0.5), Rounding::same(3.0), fill, stroke);
    if resp.hovered() && !selected {
        ui.painter()
            .rect_stroke(rect.shrink(0.5), Rounding::same(3.0), Stroke::new(1.0, c.stroke2));
    }
    let l = rect.left() + 12.0;
    // Title = system name, wrapped to at most two rows and ellipsised on overflow so a
    // long description never runs past the card edge.
    let mut job = egui::text::LayoutJob::simple(
        m.name(app.lang).to_owned(),
        FontId::proportional(13.0),
        c.fg,
        W - 24.0,
    );
    job.wrap.max_rows = 2;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts(|f| f.layout_job(job));
    let p = ui.painter_at(rect);
    p.galley(Pos2::new(l, rect.top() + 9.0), galley, c.fg);
    // Code = secondary, pinned to the bottom-left.
    p.text(Pos2::new(l, rect.bottom() - 9.0), Align2::LEFT_BOTTOM, m.code, FontId::monospace(12.0), c.fg_dim);
    resp
}

/// Modal popup shown on screen 2 while a link is being established (spinner) or
/// after it failed (error + retry/close). Dims and blocks the screen behind it,
/// so no other action is possible until the handshake resolves or is dismissed.
fn connect_modal(app: &mut App, ctx: &egui::Context) {
    if !app.connect_pending && app.connect_error.is_none() {
        return;
    }
    let c = palette(app.theme);
    let lang = app.lang;
    let screen = ctx.screen_rect();

    // Dim backdrop (above panels), also swallowing any click to the screen behind.
    egui::Area::new(egui::Id::new("connect_backdrop"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(170));
        });

    let frame = egui::Frame::none()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.stroke2))
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(30.0, 26.0));

    egui::Window::new("connect_modal")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(300.0);
            ui.vertical_centered(|ui| {
                let code = app.ecu_sel.unwrap_or("");
                match app.connect_error.clone() {
                    None => {
                        ui.add(egui::Spinner::new().size(30.0).color(c.accent));
                        ui.add_space(16.0);
                        ui.label(RichText::new(t("connecting", lang)).size(13.0).strong().color(c.fg));
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!("{code} · INITIALISIERUNG"))
                                .size(10.0)
                                .color(c.fg_dim),
                        );
                    }
                    Some(msg) => {
                        ui.label(RichText::new("⚠").size(30.0).color(c.err));
                        ui.add_space(12.0);
                        ui.label(RichText::new(t("connect_failed", lang)).size(13.0).strong().color(c.err));
                        ui.add_space(8.0);
                        ui.label(RichText::new(msg).size(10.0).color(c.fg_dim));
                        ui.add_space(20.0);
                        ui.horizontal(|ui| {
                            let retry = egui::Button::new(
                                RichText::new(t("retry", lang)).size(11.0).strong().color(c.accent_fg),
                            )
                            .fill(c.accent)
                            .rounding(3.0)
                            .min_size(Vec2::new(140.0, 32.0));
                            if ui.add(retry).clicked() {
                                app.start_connect(ctx);
                            }
                            ui.add_space(10.0);
                            let close = egui::Button::new(
                                RichText::new(t("close", lang)).size(11.0).color(c.fg_dim),
                            )
                            .fill(c.panel2)
                            .stroke(Stroke::new(1.0, c.stroke))
                            .rounding(3.0)
                            .min_size(Vec2::new(140.0, 32.0));
                            if ui.add(close).clicked() {
                                app.connect_error = None;
                            }
                        });
                    }
                }
            });
        });

    if app.connect_pending {
        ctx.request_repaint(); // keep the spinner animating
    }
}
