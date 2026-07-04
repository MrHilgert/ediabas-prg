//! Screen 2 — ECU Select. Dense module table for the chosen chassis.
//! Single click selects (right card); double click / CONNECT → session (screen 3).

use egui::{Align2, Color32, FontId, Pos2, RichText, Rounding, Sense, Stroke, Vec2};

use super::header;
use crate::app::{App, Screen};
use crate::ecu::{mods_for, Category, Module};
use crate::i18n::t;
use crate::ui::theme::{palette, Colors};

pub fn show(app: &mut App, ctx: &egui::Context) {
    // Worker events (incl. connect result) are drained once in `App::update`; the
    // connect overlay (spinner / error) is `App::link_modal`, rendered above every
    // screen. This screen just lists modules and starts a connect on CONNECT / dbl-click.
    header(app, ctx);
    category_rail(app, ctx);
    module_table(app, ctx);
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
