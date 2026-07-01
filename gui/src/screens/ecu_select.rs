//! Screen 2 — ECU Select. Dense module table for the chosen chassis.
//! Single click selects (right card); double click / CONNECT → session (screen 3).

use egui::{Align2, Color32, FontId, Pos2, RichText, Rounding, Sense, Stroke, Vec2};

use super::{dot, header, status_bar};
use crate::app::{App, Screen};
use crate::ecu::{mods_for, Category, ModStatus, Module};
use crate::lang::dict;
use crate::theme::{palette, Colors};

pub fn show(app: &mut App, ctx: &egui::Context) {
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
}

fn mods(app: &App) -> Vec<Module> {
    match app.selected_chassis() {
        Some(ch) => mods_for(ch),
        None => Vec::new(),
    }
}

fn st_color(c: &Colors, s: ModStatus) -> Color32 {
    match s {
        ModStatus::Ok => c.ok,
        ModStatus::Fault => c.warn,
        ModStatus::NoLink => c.faint_dot,
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

            let mut row = |ui: &mut egui::Ui, active: bool, label: &str, count: usize| -> bool {
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

            if row(ui, app.ecu_cat.is_none(), d.all, all.len()) {
                app.ecu_cat = None;
            }
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
    let fault_total = all
        .iter()
        .filter(|m| matches!(m.status().0, ModStatus::Fault))
        .count();
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
                if fault_total > 0 {
                    ui.label(
                        RichText::new(format!("{fault_total} {}", d.with_faults))
                            .size(10.0)
                            .color(c.warn),
                    );
                }
            });
        });
        ui.add_space(8.0);
        ui.separator();

        // Column header
        ui.add_space(6.0);
        table_header(ui, &c, &d);
        ui.add_space(2.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for m in visible {
                let selected = app.ecu_sel == Some(m.code);
                // module_row handles single-click selection internally.
                if module_row(ui, app, &c, m, selected).double_clicked() {
                    app.ecu_sel = Some(m.code);
                    app.enter_session();
                }
            }
        });
    });
}

// Column widths: dot 14 · code 62 · system 1fr · bus 78 · addr 58 · status 78
const COL_DOT: f32 = 18.0;
const COL_CODE: f32 = 62.0;
const COL_BUS: f32 = 78.0;
const COL_ADDR: f32 = 58.0;
const COL_STATUS: f32 = 90.0;

fn table_header(ui: &mut egui::Ui, c: &Colors, d: &crate::lang::Dict) {
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        let lbl = |ui: &mut egui::Ui, w: f32, s: &str| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 16.0), Sense::hover());
            ui.painter().text(
                Pos2::new(rect.left(), rect.center().y),
                Align2::LEFT_CENTER,
                s,
                FontId::monospace(9.0),
                c.fg_faint,
            );
        };
        lbl(ui, COL_DOT, "");
        lbl(ui, COL_CODE, d.col_ecu);
        // system stretches
        let avail = ui.available_width() - (COL_BUS + COL_ADDR + COL_STATUS + 20.0);
        lbl(ui, avail.max(80.0), d.col_system);
        lbl(ui, COL_BUS, d.col_bus);
        lbl(ui, COL_ADDR, d.col_addr);
        lbl(ui, COL_STATUS, d.col_status);
    });
}

fn module_row(
    ui: &mut egui::Ui,
    app: &mut App,
    c: &Colors,
    m: &Module,
    selected: bool,
) -> egui::Response {
    let (status, faults) = m.status();
    let d = dict(app.lang);
    let row_h = 30.0;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::click());
    let p = ui.painter_at(rect);
    // Subtle: hover = faint fill, selected = accent left bar only (no fill).
    if resp.hovered() && !selected {
        p.rect_filled(rect, Rounding::ZERO, c.panel);
    }
    if selected {
        p.rect_filled(
            egui::Rect::from_min_size(rect.left_top(), Vec2::new(2.0, row_h)),
            Rounding::ZERO,
            c.accent,
        );
    }
    p.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, c.stroke),
    );

    let mut x = rect.left() + 14.0;
    let cy = rect.center().y;
    let mono = |s| FontId::monospace(s);
    // status dot
    p.circle_filled(Pos2::new(x + 4.0, cy), 4.0, st_color(c, status));
    x += COL_DOT;
    // code
    p.text(Pos2::new(x, cy), Align2::LEFT_CENTER, m.code, mono(12.0), c.fg);
    x += COL_CODE;
    // system name (stretch)
    let sys_w = rect.right() - 14.0 - (COL_BUS + COL_ADDR + COL_STATUS) - x;
    let _ = sys_w;
    p.text(Pos2::new(x, cy), Align2::LEFT_CENTER, m.name(app.lang), mono(11.0), c.fg_dim);
    // right-anchored columns
    let mut rx = rect.right() - 14.0;
    // status text
    let (stxt, scol) = match status {
        ModStatus::Ok => (d.lg2_ok.to_string(), c.ok),
        ModStatus::Fault => (format!("{faults} DTC"), c.warn),
        ModStatus::NoLink => (d.st_nolink.to_string(), c.faint_dot),
    };
    p.text(Pos2::new(rx, cy), Align2::RIGHT_CENTER, stxt, mono(10.0), scol);
    rx -= COL_STATUS;
    p.text(Pos2::new(rx, cy), Align2::RIGHT_CENTER, m.addr, mono(10.0), c.fg_dim);
    rx -= COL_ADDR;
    p.text(Pos2::new(rx, cy), Align2::RIGHT_CENTER, m.bus, mono(10.0), c.fg_faint);

    if resp.clicked() {
        app.ecu_sel = Some(m.code);
    }
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
            let (status, faults) = m.status();

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
                    ModStatus::Ok => (d.lg2_ok.to_string(), c.ok),
                    ModStatus::Fault => (format!("{faults} {}", d.lg2_fault), c.warn),
                    ModStatus::NoLink => (d.st_nolink.to_string(), c.faint_dot),
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
                if ui.add(btn).clicked() {
                    app.enter_session();
                }
            });
        });
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
