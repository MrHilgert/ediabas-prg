//! Screen 2 — ECU Select. Dense module table for the chosen chassis.
//! Single click selects (right card); double click / CONNECT → session (screen 3).

use egui::{Align2, Color32, FontId, Pos2, RichText, Rounding, Sense, Stroke, Vec2};

use super::{dot, header, status_bar};
use crate::app::{App, Screen};
use crate::ecu::{mods_for, Category, ModStatus, Module};
use crate::lang::{dict, Lang};
use crate::theme::{palette, Colors};
use crate::worker::Event;

pub fn show(app: &mut App, ctx: &egui::Context) {
    // A real connection attempt (INITIALISIERUNG) runs here, on screen 2: only a
    // successful link opens the session screen; a failure stays put with an error.
    poll_connect(app, ctx);
    // Advance the scan animation.
    if app.scan_pct < 100.0 {
        app.scan_pct = (app.scan_pct + 2.5).min(100.0);
        ctx.request_repaint();
    }
    header(app, ctx);
    status_bar(app, ctx);
    category_rail(app, ctx);
    module_detail(app, ctx);
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
                    // The Connected event is consumed HERE, before the session screen
                    // opens — build the .ipo-curated live groups now so the session's
                    // data-stream pages exist on first frame.
                    app.meas_groups =
                        crate::session_cfg::build_meas_groups(app.ecu_sel.unwrap_or(""));
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
                Event::JobDone { .. } => {}
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
    let d = dict(lang);
    let low = raw.to_ascii_lowercase();
    let msg = if low.contains("timed out") || low.contains("timeout")
        || low.contains("no response") || low.contains("не отвеч")
    {
        d.err_no_response
    } else if low.contains("denied") || low.contains("отказано")
        || low.contains("in use") || low.contains("занят")
    {
        d.err_port_busy
    } else if low.contains("not found") || low.contains("cannot find")
        || low.contains("no such") || low.contains("не найд")
    {
        d.err_no_adapter
    } else {
        d.err_generic
    };
    msg.to_string()
}

fn mods(app: &App) -> Vec<Module> {
    match app.selected_chassis() {
        Some(ch) => mods_for(ch),
        None => Vec::new(),
    }
}

fn st_color(c: &Colors, s: ModStatus) -> Color32 {
    match s {
        ModStatus::Ready => c.fg_dim,   // SGBD present, health unknown until scanned
        ModStatus::NoSgbd => c.faint_dot,
    }
}

fn category_rail(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);
    let d = dict(app.lang);
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
                ui.label(RichText::new(d.category).size(10.0).strong().color(c.fg_dim));
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

            // Status legend (bottom).
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.add_space(11.0);
                for (col, label) in [
                    (c.ok, d.lg2_ok),
                    (c.warn, d.lg2_fault),
                    (c.faint_dot, d.lg2_nolink),
                ] {
                    ui.horizontal(|ui| {
                        ui.add_space(15.0);
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

fn module_table(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);
    let d = dict(app.lang);
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
                    egui::Button::new(RichText::new(format!("‹ {}", d.back)).size(11.0).color(c.fg_dim))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0, c.stroke)),
                )
                .clicked()
            {
                app.screen = Screen::Chassis;
            }
            ui.add_space(10.0);
            ui.label(RichText::new(d.select_ecu).size(12.0).strong().color(c.fg));
            ui.label(RichText::new(format!("/ {chassis_code}")).size(11.0).color(c.fg_faint));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(14.0);
                // Scan progress / button
                if app.scan_pct < 100.0 {
                    ui.label(
                        RichText::new(format!("{} {:.0}%", d.scanning, app.scan_pct))
                            .size(10.0)
                            .color(c.accent),
                    );
                } else if ui
                    .add(
                        egui::Button::new(RichText::new(format!("⟳ {}", d.scan_all)).size(10.0).color(c.fg_dim))
                            .fill(c.panel2)
                            .stroke(Stroke::new(1.0, c.stroke)),
                    )
                    .clicked()
                {
                    app.scan_pct = 0.0;
                }
                ui.add_space(12.0);
                if sgbd_total > 0 {
                    ui.label(
                        RichText::new(format!("{sgbd_total} SGBD"))
                            .size(10.0)
                            .color(c.fg_faint),
                    );
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

/// One ECU as a card: code + system name only (no dot / bus / addr / status).
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
    let p = ui.painter_at(rect);
    let l = rect.left() + 12.0;
    // code
    p.text(Pos2::new(l, rect.top() + 11.0), Align2::LEFT_TOP, m.code, FontId::monospace(17.0), c.fg);
    // system name
    p.text(Pos2::new(l, rect.top() + 39.0), Align2::LEFT_TOP, m.name(app.lang), FontId::proportional(11.0), c.fg_dim);
    resp
}

fn module_detail(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);
    let d = dict(app.lang);
    let sel = app.ecu_sel.and_then(|code| mods(app).into_iter().find(|m| m.code == code));
    let frame = egui::Frame::none().fill(c.panel);

    egui::SidePanel::right("ecu_detail")
        .exact_width(346.0)
        .resizable(false)
        .frame(frame)
        .show(ctx, |ui| {
            let Some(m) = sel else {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(d.pick_module).size(11.0).color(c.fg_faint));
                });
                return;
            };
            let status = m.status();

            ui.add_space(15.0);
            ui.horizontal(|ui| {
                ui.add_space(15.0);
                ui.label(RichText::new(m.code).size(27.0).strong().color(c.fg));
                ui.add_space(8.0);
                ui.label(RichText::new(m.name(app.lang)).size(12.0).color(c.fg_dim));
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(15.0);
                dot(ui, st_color(&c, status), 8.0);
                ui.add_space(8.0);
                let (label, col) = match status {
                    ModStatus::Ready => (d.st_ready.to_string(), c.fg_dim),
                    ModStatus::NoSgbd => (d.st_nosgbd.to_string(), c.faint_dot),
                };
                ui.label(RichText::new(label).size(10.0).strong().color(col));
            });
            ui.add_space(12.0);
            ui.separator();

            meta(ui, &c, d.category, m.cat.label(app.lang));
            meta(ui, &c, d.bus, m.bus);
            meta(ui, &c, d.address, m.addr);

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                ui.add_space(15.0);
                let btn = egui::Button::new(
                    RichText::new(format!("▸  {}", d.connect)).size(12.0).strong().color(c.accent_fg),
                )
                .fill(c.accent)
                .rounding(3.0)
                .min_size(Vec2::new(316.0, 34.0));
                // Only modules with a real SGBD can be opened.
                if ui.add_enabled(m.connectable(), btn).clicked() {
                    app.start_connect(ctx);
                }
            });
        });
}

/// Modal popup shown on screen 2 while a link is being established (spinner) or
/// after it failed (error + retry/close). Dims and blocks the screen behind it,
/// so no other action is possible until the handshake resolves or is dismissed.
fn connect_modal(app: &mut App, ctx: &egui::Context) {
    if !app.connect_pending && app.connect_error.is_none() {
        return;
    }
    let c = palette(app.theme);
    let d = dict(app.lang);
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
                        ui.label(RichText::new(d.connecting).size(13.0).strong().color(c.fg));
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
                        ui.label(RichText::new(d.connect_failed).size(13.0).strong().color(c.err));
                        ui.add_space(8.0);
                        ui.label(RichText::new(msg).size(10.0).color(c.fg_dim));
                        ui.add_space(20.0);
                        ui.horizontal(|ui| {
                            let retry = egui::Button::new(
                                RichText::new(d.retry).size(11.0).strong().color(c.accent_fg),
                            )
                            .fill(c.accent)
                            .rounding(3.0)
                            .min_size(Vec2::new(140.0, 32.0));
                            if ui.add(retry).clicked() {
                                app.start_connect(ctx);
                            }
                            ui.add_space(10.0);
                            let close = egui::Button::new(
                                RichText::new(d.close).size(11.0).color(c.fg_dim),
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

fn meta(ui: &mut egui::Ui, c: &Colors, label: &str, value: &str) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(15.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(RichText::new(label).size(9.0).color(c.fg_faint));
            ui.label(RichText::new(value).size(13.0).color(c.fg));
        });
    });
}
