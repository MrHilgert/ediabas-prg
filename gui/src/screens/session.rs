//! Screen 3 — ECU Session. One templated screen renders any module from the
//! config page-tree (`session_cfg`): typed blocks + positional INPA F-keys,
//! driven by a navigation stack. Live stream values animate from time.

use egui::{Align2, Color32, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::{header, status_bar};
use crate::app::{App, Screen};
use crate::ecu::{code_hash, mock_fault_count, mods_for, Category, Module};
use crate::lang::{dict, Lang};
use crate::session_cfg::{fault_pool, screens_for, Block, FAction, Page, StreamParam};
use crate::theme::{palette, Colors};
use crate::worker::{Cmd, Event, Worker};

/// Everything a block renderer needs about the current module.
struct Em {
    code: &'static str,
    name: &'static str,
    bus: &'static str,
    addr: &'static str,
    cat: Category,
    hash: u32,
    faults: u32,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);
    let d = dict(app.lang);

    // Resolve the selected module.
    let module: Option<Module> = app.ecu_sel.and_then(|code| {
        app.selected_chassis()
            .map(mods_for)
            .and_then(|v| v.into_iter().find(|m| m.code == code))
    });
    let Some(m) = module else {
        header(app, ctx);
        status_bar(app, ctx);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(d.pick_module).color(c.fg_faint));
            });
        });
        return;
    };
    let em = Em {
        code: m.code,
        name: m.name(app.lang),
        bus: m.bus,
        addr: m.addr,
        cat: m.cat,
        hash: code_hash(m.code),
        faults: mock_fault_count(m.code),
    };

    let screens = screens_for(&m, &app.meas_groups);
    let cur_id = app.ses_stack.last().cloned().unwrap_or_else(|| "main".into());
    let page = screens.pages.get(&cur_id).or_else(|| screens.pages.get("main")).unwrap();

    // --- Real connection + live-stream lifecycle (any module with a real SGBD) ---
    let is_real = m.prg.is_some();
    // Drain worker events.
    if let Some(w) = &app.worker {
        for evt in w.poll() {
            match evt {
                Event::Connected { .. } => {
                    app.connected = app.ecu_sel;
                    app.connect_pending = false;
                    app.status_msg.clear();
                    // Build the .ipo-curated live measurement groups once for this connection.
                    app.meas_groups =
                        crate::session_cfg::build_meas_groups(app.ecu_sel.unwrap_or(""));
                }
                Event::JobDone { job, result } => {
                    if job == "FS_LESEN" {
                        app.faults = Some(result);
                        app.faults_read = true;
                        app.faults_busy = false;
                        app.faults_cleared = false;
                    } else if job == "FS_LOESCHEN" {
                        // Cleared — re-read to show the (now empty) memory.
                        app.faults = None;
                        if let Some(w) = &app.worker {
                            w.send(Cmd::RunJob { job: "FS_LESEN".into(), args: String::new() });
                        }
                    } else {
                        app.live = Some(result); // measurement stream (MW_SELECT_LESEN_NORM)
                    }
                }
                Event::Error(e) => {
                    app.connected = None;
                    app.connect_pending = false; // no longer in flight → banner shows the error
                    app.streaming = false;
                    app.status_msg = format!("нет связи: {e}");
                }
                Event::Disconnected => {
                    app.connected = None;
                    app.streaming = false;
                }
            }
        }
    }
    if is_real {
        let code = m.code;
        let prg = m.prg.unwrap().to_string();
        // Presence = a successful INITIALISIERUNG. One attempt per module (no spam):
        // `connect_attempted` gates retries, `connect_pending` = in flight now.
        if app.connected != Some(code) && !app.connect_attempted {
            let w = app.worker.get_or_insert_with(|| Worker::spawn(ctx.clone()));
            w.send(Cmd::Connect { port: "COM3".into(), baud: 9600, prg });
            app.connect_attempted = true;
            app.connect_pending = true;
        }
        // Stream whatever poll the current page declares — a measurement group polls
        // INPA's own request list (MW_SELECT_LESEN_NORM + STATUS_*). Start/stop/switch
        // on change.
        let desired = if app.connected == Some(code) {
            screens.polls.get(&cur_id).cloned()
        } else {
            None
        };
        if desired != app.stream_poll {
            if let Some(w) = &app.worker {
                match &desired {
                    Some((reqs, ms)) => {
                        w.send(Cmd::StartStream { reqs: reqs.clone(), interval_ms: *ms })
                    }
                    None => w.send(Cmd::StopStream),
                }
            }
            if desired.is_none() {
                app.live = None;
            }
            app.stream_poll = desired;
            app.streaming = app.stream_poll.is_some();
        }
    }

    // Keyboard: F1–F6 or 1–6 fire the matching positional key on the current page.
    let mut pending: Option<FAction> = ctx.input(|i| {
        for f in 1..=6u8 {
            let fkey = [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6][(f - 1) as usize];
            let num = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6][(f - 1) as usize];
            if i.key_pressed(fkey) || i.key_pressed(num) {
                if let Some(k) = page.keys.iter().find(|k| k.f == f) {
                    return Some(k.act.clone());
                }
            }
        }
        None
    });

    header(app, ctx);
    status_bar(app, ctx);

    // F-key panel (INPA), above the status bar.
    fkey_panel(app, ctx, &c, page, &mut pending);

    // Body
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(c.bg))
        .show(ctx, |ui| {
            // Context strip
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                if ui
                    .add(egui::Button::new(RichText::new(format!("‹ {}", d.select_ecu)).size(11.0).color(c.fg_dim))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0, c.stroke)))
                    .clicked()
                {
                    app.screen = Screen::Ecu;
                }
                ui.add_space(10.0);
                let chassis = app.selected_chassis().map(|ch| ch.code).unwrap_or("");
                ui.label(RichText::new(page.title.get(app.lang)).size(13.0).strong().color(c.fg));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(format!("{chassis} · {} · {}", em.code, em.cat.label(app.lang)))
                            .size(10.0)
                            .color(c.fg_faint),
                    );
                    // Breadcrumb of the page stack
                    let crumbs: Vec<&str> = app
                        .ses_stack
                        .iter()
                        .filter_map(|id| screens.pages.get(id).map(|p| p.title.get(app.lang)))
                        .collect();
                    ui.add_space(12.0);
                    ui.label(RichText::new(crumbs.join(" › ")).size(10.0).color(c.fg_dim));
                });
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // Connection status (real modules only): visible init/link state.
            if is_real {
                let (txt, col) = if app.connected == Some(em.code) {
                    if app.live.is_some() {
                        ("● связь установлена · поток".to_string(), c.ok)
                    } else {
                        ("● связь установлена · ожидание данных…".to_string(), c.ok)
                    }
                } else if app.connect_pending {
                    ("◌ INITIALISIERUNG…".to_string(), c.accent)
                } else {
                    (format!("● {}", app.status_msg), c.err)
                };
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(RichText::new(txt).size(10.0).color(col));
                });
                ui.add_space(4.0);
            }

            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::none().inner_margin(14.0).show(ui, |ui| {
                    for block in &page.blocks {
                        render_block(ui, app, ctx, &c, &em, block, &mut pending);
                        ui.add_space(12.0);
                    }
                });
            });
        });

    if let Some(act) = pending.take() {
        apply(app, act);
    }
}

fn apply(app: &mut App, act: FAction) {
    match act {
        FAction::To(id) => app.ses_stack.push(id),
        FAction::Back => app.go_back(),
        FAction::Exit => app.screen = Screen::Ecu,
        FAction::Read => {
            // Real module + linked → run FS_LESEN; otherwise fall back to the mock list.
            if app.connected == app.ecu_sel && app.worker.is_some() {
                app.faults = None;
                app.faults_busy = true;
                app.faults_cleared = false;
                app.fault_open = None;
                app.worker.as_ref().unwrap()
                    .send(Cmd::RunJob { job: "FS_LESEN".into(), args: String::new() });
            } else {
                app.faults_read = true;
                app.faults_cleared = false;
            }
        }
        FAction::Clear => {
            if app.connected == app.ecu_sel && app.worker.is_some() {
                app.faults_busy = true;
                app.fault_open = None;
                app.worker.as_ref().unwrap()
                    .send(Cmd::RunJob { job: "FS_LOESCHEN".into(), args: String::new() });
            } else {
                app.faults_cleared = true;
            }
        }
        FAction::Noop => {}
    }
}

fn fkey_panel(
    app: &App,
    ctx: &egui::Context,
    c: &Colors,
    page: &Page,
    pending: &mut Option<FAction>,
) {
    let frame = egui::Frame::none()
        .fill(c.panel)
        .inner_margin(egui::Margin::symmetric(12.0, 8.0));
    egui::TopBottomPanel::bottom("fkeys")
        .exact_height(64.0)
        .frame(frame)
        .show(ctx, |ui| {
            ui.columns(6, |cols| {
                for (i, col) in cols.iter_mut().enumerate() {
                    let f = (i + 1) as u8;
                    let key = page.keys.iter().find(|k| k.f == f);
                    fkey_button(col, app, c, f, key, pending);
                }
            });
        });
}

fn fkey_button(
    ui: &mut egui::Ui,
    app: &App,
    c: &Colors,
    f: u8,
    key: Option<&crate::session_cfg::FKey>,
    pending: &mut Option<FAction>,
) {
    let (label, tag_col, enabled) = match key {
        Some(k) => {
            let col = match k.act {
                FAction::To(_) => c.accent,
                FAction::Back | FAction::Exit => c.fg_faint,
                FAction::Read => c.ok,
                FAction::Clear => c.warn,
                FAction::Noop => c.fg_dim,
            };
            (k.label.get(app.lang), col, true)
        }
        None => ("", c.fg_faint, false),
    };
    let fill = if enabled { c.panel2 } else { c.panel };
    let stroke = Stroke::new(1.0, c.stroke);
    let resp = egui::Frame::none()
        .fill(fill)
        .stroke(stroke)
        .rounding(3.0)
        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width()); // fill the whole column
            ui.set_min_height(46.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("F{f}")).size(11.0).strong().color(tag_col));
                });
                ui.label(
                    RichText::new(label)
                        .size(11.0)
                        .color(if enabled { c.fg } else { c.fg_faint }),
                );
            });
        })
        .response
        .interact(Sense::click());
    if enabled && resp.clicked() {
        if let Some(k) = key {
            *pending = Some(k.act.clone());
        }
    }
}

fn render_block(
    ui: &mut egui::Ui,
    app: &mut App,
    ctx: &egui::Context,
    c: &Colors,
    em: &Em,
    block: &Block,
    pending: &mut Option<FAction>,
) {
    let lang = app.lang;
    match block {
        Block::Note(l) => {
            ui.label(RichText::new(l.get(lang)).size(12.0).color(c.fg_dim));
        }
        Block::NoteOnline => {
            let t = match lang {
                Lang::Ru => format!("Модуль {} на связи. Выберите функцию клавишами F1–F6 или мышью.", em.code),
                Lang::En => format!("Module {} online. Select a function with F1–F6 or click.", em.code),
            };
            ui.label(RichText::new(t).size(12.0).color(c.fg_dim));
        }
        Block::Menu(items) => {
            for (idx, it) in items.iter().enumerate() {
                if menu_row(ui, c, idx + 1, it.label.get(lang)) {
                    *pending = Some(FAction::To(it.to.clone()));
                }
            }
        }
        Block::Kv => kv_block(ui, c, em, lang),
        Block::Faults => faults_block(ui, app, c, em),
        Block::Stream(params) => stream_block(ui, ctx, c, params, app.live.as_ref(), lang),
        Block::Status => status_block(ui, c, em, lang),
    }
}

fn menu_row(ui: &mut egui::Ui, c: &Colors, idx: usize, label: &str) -> bool {
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
                    ui.label(RichText::new("›").size(12.0).color(c.fg_faint));
                });
            });
        })
        .response
        .interact(Sense::click())
        .clicked()
}

fn kv_block(ui: &mut egui::Ui, c: &Colors, em: &Em, lang: Lang) {
    let h = em.hash;
    let sup = ["Bosch", "Siemens VDO", "Continental", "Magneti Marelli", "Valeo"][(h % 5) as usize];
    let yr = 1998 + (h % 16);
    let wk = 1 + (h % 52);
    let rows: Vec<(String, String)> = vec![
        (t(lang, "Блок", "Module"), format!("{} · {}", em.code, em.name)),
        (t(lang, "Номер детали", "Part number"), format!("6512 {:04}", h % 10000)),
        (t(lang, "Версия HW / SW", "HW / SW version"), format!("0x{:02X} / 0x{:02X}", h % 256, (h >> 4) % 256)),
        (t(lang, "Индекс диагноза", "Diagnostic index"), format!("0x{:02X}", (h >> 2) % 256)),
        (t(lang, "Индекс кодирования", "Coding index"), format!("0x{:02X}", (h >> 5) % 256)),
        (t(lang, "Шина / адрес", "Bus / address"), format!("{} / {}", em.bus, em.addr)),
        (t(lang, "Поставщик", "Supplier"), sup.to_string()),
        (t(lang, "Дата выпуска", "Manufactured"), format!("{wk:02} / {yr}")),
        (t(lang, "VIN автомобиля", "Vehicle VIN"), format!("WBA{}V{:05}", h % 9, h % 100000)),
    ];
    for (k, v) in rows {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
            let p = ui.painter_at(rect);
            p.text(Pos2::new(rect.left(), rect.center().y), Align2::LEFT_CENTER, k, FontId::monospace(11.0), c.fg_dim);
            p.text(Pos2::new(rect.right(), rect.center().y), Align2::RIGHT_CENTER, v, FontId::monospace(12.0), c.fg);
            p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, c.stroke));
        });
    }
}

fn faults_block(ui: &mut egui::Ui, app: &mut App, c: &Colors, em: &Em) {
    let lang = app.lang;
    let real = app.connected == app.ecu_sel && app.worker.is_some();

    if real {
        real_faults_block(ui, app, c, lang);
        return;
    }

    // ── mock catalog (non-DDE modules) ──
    if !app.faults_read {
        ui.label(RichText::new(t(lang, "Нажмите F1 — читать память ошибок", "Press F1 — read fault memory")).size(12.0).color(c.fg_dim));
        return;
    }
    if app.faults_cleared || em.faults == 0 {
        ui.label(RichText::new(t(lang, "Память ошибок пуста", "Fault memory is empty")).size(12.0).color(c.ok));
        return;
    }
    let pool = fault_pool(em.cat);
    ui.label(RichText::new(format!("{} {}", em.faults, t(lang, "ошибок в памяти", "faults stored"))).size(11.0).color(c.fg_faint));
    ui.add_space(6.0);
    for i in 0..em.faults {
        let hh = em.hash.wrapping_mul(i + 7) % 65536;
        let present = i % 2 == 0;
        let code = format!("{hh:04X}");
        let desc = pool[((em.hash + i) as usize) % pool.len()].get(lang);
        let expanded = app.fault_open.as_deref() == Some(code.as_str());
        let (slabel, scol) = if present {
            (t(lang, "присутствует", "present"), c.warn)
        } else {
            (t(lang, "сохранён", "stored"), c.fg_dim)
        };
        let clicked = fault_row(ui, c, &code, desc, &slabel, scol, expanded);
        if clicked {
            app.fault_open = if expanded { None } else { Some(code.clone()) };
        }
        if expanded {
            fault_detail(ui, c, hh, present, lang);
        }
    }
}

/// One freeze-frame environmental condition (F_UW{i}_*).
struct Uw {
    text: String,
    val: String,
    unit: String,
}

/// One decoded DTC from FS_LESEN.
struct Dtc {
    code: String,    // e.g. "1E00"
    text: String,    // F_ORT_TEXT
    present: bool,   // F_VORHANDEN — currently active
    sporadic: bool,  // F_ART "sporadischer Fehler" flag
    raw: String,     // F_HEX_CODE
    hfk: i64,          // F_HFK  — occurrences
    lz: i64,           // F_LZ   — lifetime counter
    uw_satz: i64,      // F_UW_SATZ — number of freeze-frame snapshots
    uw: Vec<Uw>,       // F_UW1..N — freeze-frame conditions
    causes: Vec<String>, // F_ART{i}_TEXT != "--" — fault types/causes
}

/// Format an environmental value: integer if whole, else one decimal.
fn fmt_uw(v: f64) -> String {
    if v.fract().abs() < 0.05 { format!("{v:.0}") } else { format!("{v:.1}") }
}

/// Parse the FS_LESEN JobResult into a DTC list (one per set carrying F_ORT_NR).
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
        // Fault types/causes: F_ART{i}_TEXT. Skip "--" slots and the status-flag
        // entries (ARTNR >= 0xEB = "Fehler momentan vorhanden"/"sporadisch"/… —
        // those live in a separate high-numbered block and are already shown as
        // Status). Keep only real causes (short/open/implausible/…).
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

fn real_faults_block(ui: &mut egui::Ui, app: &mut App, c: &Colors, lang: Lang) {
    if app.faults_busy {
        ui.label(RichText::new(t(lang, "Чтение памяти ошибок…", "Reading fault memory…")).size(12.0).color(c.accent));
        return;
    }
    let Some(fr) = &app.faults else {
        ui.label(RichText::new(t(lang, "Нажмите F1 — читать память ошибок", "Press F1 — read fault memory")).size(12.0).color(c.fg_dim));
        return;
    };
    let dtcs = parse_dtcs(fr);
    if dtcs.is_empty() {
        ui.label(RichText::new(t(lang, "Память ошибок пуста", "Fault memory is empty")).size(12.0).color(c.ok));
        return;
    }
    ui.label(RichText::new(format!("{} {}", dtcs.len(), t(lang, "ошибок в памяти", "faults stored"))).size(11.0).color(c.fg_faint));
    ui.add_space(6.0);
    for d in &dtcs {
        let expanded = app.fault_open.as_deref() == Some(d.code.as_str());
        let (slabel, scol) = fault_status(c, d.present, d.sporadic, lang);
        if fault_row(ui, c, &d.code, &d.text, &slabel, scol, expanded) {
            app.fault_open = if expanded { None } else { Some(d.code.clone()) };
        }
        if expanded {
            real_fault_detail(ui, c, d, lang);
        }
    }
}

/// One compact key/value cell for the 2-column detail grid.
struct KvCell {
    label: String,
    value: String,
    unit: String,
    color: egui::Color32,
}

fn kv(label: &str, value: String, unit: &str, color: egui::Color32) -> KvCell {
    KvCell { label: label.to_string(), value, unit: unit.to_string(), color }
}

/// Render key/value cells in a 2-column grid: label left, value right-aligned
/// within a narrow cell (so the two stay close), unit dimmed, hairline underline.
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
            if idx >= cells.len() { break; }
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
            ui.label(RichText::new(t(lang, "СВЕДЕНИЯ", "DETAILS")).size(9.0).color(c.fg_faint));
            ui.add_space(2.0);
            kv_grid(ui, c, &[
                kv(&t(lang, "Статус", "Status"), status, "", scol),
                kv(&t(lang, "Код места", "Location code"), format!("0x{}", d.code), "", c.fg),
                kv(&t(lang, "Повторений", "Occurrences"), d.hfk.to_string(), "", c.fg),
                kv(&t(lang, "Счётчик (LZ)", "Counter (LZ)"), d.lz.to_string(), "", c.fg),
            ]);

            // Cause / fault types (F_ART*).
            if !d.causes.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(t(lang, "ПРИЧИНА", "CAUSE")).size(9.0).color(c.fg_faint));
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

            // Freeze frame: F_UW_SATZ snapshots of equal size, if it divides evenly.
            if !d.uw.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(t(lang, "СТОП-КАДР ПАРАМЕТРОВ", "FREEZE FRAME")).size(9.0).color(c.fg_faint));
                let satz = d.uw_satz.max(1) as usize;
                let per = if satz > 1 && d.uw.len() % satz == 0 { d.uw.len() / satz } else { d.uw.len() };
                let mut i = 0;
                while i < d.uw.len() {
                    let snap = i / per + 1;
                    if satz > 1 {
                        if snap > 1 {
                            // Divider between snapshots so they don't blend together.
                            ui.add_space(6.0);
                            let w = (2.0f32 * 430.0 + 46.0).min(ui.available_width());
                            let (r, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
                            ui.painter().hline(r.left()..=r.right(), r.center().y, Stroke::new(1.0, c.fg_faint));
                            ui.add_space(4.0);
                        } else {
                            ui.add_space(3.0);
                        }
                        ui.label(RichText::new(format!("{} {}", t(lang, "Снимок", "Snapshot"), snap)).size(9.0).color(c.accent));
                    }
                    let end = (i + per).min(d.uw.len());
                    let cells: Vec<KvCell> = d.uw[i..end].iter()
                        .map(|u| kv(&u.text, u.val.clone(), &u.unit, c.fg))
                        .collect();
                    kv_grid(ui, c, &cells);
                    i = end;
                }
            }

            if !d.raw.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(t(lang, "СЫРАЯ ЗАПИСЬ", "RAW RECORD")).size(9.0).color(c.fg_faint));
                ui.label(RichText::new(&d.raw).size(10.0).monospace().color(c.fg_dim));
            }
        });
}

/// Three-state fault status (active / inactive / sporadic) → (label, colour).
fn fault_status(c: &Colors, present: bool, sporadic: bool, lang: Lang) -> (String, egui::Color32) {
    if sporadic {
        (t(lang, "спорадическая", "sporadic"), c.warn)
    } else if present {
        (t(lang, "активна", "active"), c.warn)
    } else {
        (t(lang, "неактивна", "inactive"), c.fg_dim)
    }
}

fn fault_row(ui: &mut egui::Ui, c: &Colors, code: &str, desc: &str, slabel: &str, scol: egui::Color32, expanded: bool) -> bool {
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

fn fault_detail(ui: &mut egui::Ui, c: &Colors, hh: u32, present: bool, lang: Lang) {
    let rr = |seed: u32, mn: f32, mx: f32| -> f32 {
        let x = ((hh.wrapping_mul(seed)) % 997) as f32 / 997.0;
        mn + (mx - mn) * x
    };
    egui::Frame::none()
        .inner_margin(egui::Margin { left: 24.0, right: 8.0, top: 0.0, bottom: 8.0 })
        .show(ui, |ui| {
            let status = if present { t(lang, "присутствует", "present") } else { t(lang, "сохранён", "stored") };
            ui.label(RichText::new(t(lang, "СВЕДЕНИЯ", "DETAILS")).size(9.0).color(c.fg_faint));
            for (k, v) in [
                (t(lang, "Статус", "Status"), status.to_string()),
                (t(lang, "Появлений", "Occurrences"), format!("{}", 1 + hh % 40)),
                (t(lang, "Пробег при фиксации", "Mileage at set"), format!("{} km", ((hh % 280) + 20) * 1000)),
                (t(lang, "Приоритет", "Priority"), format!("{}", 1 + hh % 9)),
            ] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k).size(10.0).color(c.fg_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(v).size(11.0).color(c.fg));
                    });
                });
            }
            ui.add_space(6.0);
            ui.label(RichText::new(t(lang, "СТОП-КАДР ПАРАМЕТРОВ", "FREEZE FRAME")).size(9.0).color(c.fg_faint));
            for (name, val, unit) in [
                (t(lang, "Обороты", "Engine speed"), format!("{:.0}", rr(3, 700.0, 4500.0)), "1/min"),
                (t(lang, "Температура ОЖ", "Coolant temp"), format!("{:.0}", rr(5, 60.0, 112.0)), "°C"),
                (t(lang, "Скорость", "Vehicle speed"), format!("{:.0}", rr(7, 0.0, 140.0)), "km/h"),
                (t(lang, "Нагрузка", "Load"), format!("{:.1}", rr(11, 5.0, 95.0)), "%"),
                (t(lang, "Напряжение АКБ", "Battery"), format!("{:.1}", rr(13, 11.8, 14.6)), "V"),
                (t(lang, "Темп. впуска", "Intake temp"), format!("{:.0}", rr(17, -10.0, 60.0)), "°C"),
            ] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).size(10.0).color(c.fg_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{val} {unit}")).size(11.0).color(c.fg));
                    });
                });
            }
        });
}

/// One computed bar's render data.
struct BarRow {
    label: &'static str,
    val_txt: String,
    unit: String,
    pct: f32,
    over: bool,
    lo: Option<f32>,
    hi: Option<f32>,
}

fn stream_block(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    c: &Colors,
    params: &[StreamParam],
    live: Option<&ediabas::JobResult>,
    lang: Lang,
) {
    ctx.request_repaint(); // keep values live
    let t = ctx.input(|i| i.time) as f32;

    // Compute every row up front.
    let rows: Vec<BarRow> = params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            // Bound params take value + unit from the polled JobResult; unbound
            // (mock catalog) animate a sine. Bound with no data yet → "—".
            let (v_opt, unit): (Option<f32>, String) = match p.bind {
                Some(b) => {
                    let val = live.and_then(|l| l.get_f64(&format!("{b}_WERT"))).map(|x| x as f32);
                    let u = live
                        .and_then(|l| l.get_str(&format!("{b}_EINH")))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| p.unit.to_string());
                    (val, u)
                }
                None => {
                    let phase = t * 1.2 + idx as f32 * 1.3;
                    let mut v = p.base;
                    if p.amp != 0.0 {
                        v += p.amp * phase.sin() + p.amp * 0.15 * (t * 5.0 + idx as f32 * 2.1).sin();
                    }
                    (Some(v.clamp(p.min, p.max)), p.unit.to_string())
                }
            };
            let pct = match v_opt {
                Some(v) if p.max > p.min => ((v - p.min) / (p.max - p.min)).clamp(0.0, 1.0),
                _ => 0.0,
            };
            let over = v_opt.is_some()
                && (p.hi.map_or(false, |hi| pct > hi) || p.lo.map_or(false, |lo| pct < lo));
            let val_txt = match v_opt {
                None => "—".to_string(),
                Some(v) if p.is_bool => {
                    if v >= 0.5 { t2(lang, "ВКЛ", "ON") } else { t2(lang, "ВЫКЛ", "OFF") }.to_string()
                }
                Some(v) => format!("{:.*}", p.dec as usize, v),
            };
            BarRow { label: p.label.get(lang), val_txt, unit, pct, over, lo: p.lo, hi: p.hi }
        })
        .collect();

    // Grid (row-major). Three columns keep each bar narrow while still filling the
    // wide screen — 2 columns left the right half of each column empty.
    const COLS: usize = 3;
    ui.spacing_mut().item_spacing.x = 24.0;
    let mut i = 0;
    while i < rows.len() {
        ui.columns(COLS, |cols| {
            for j in 0..COLS {
                if i + j < rows.len() {
                    draw_bar(&mut cols[j], c, &rows[i + j]);
                }
            }
        });
        ui.add_space(8.0);
        i += COLS;
    }
}

fn draw_bar(ui: &mut egui::Ui, c: &Colors, r: &BarRow) {
    // Stacked cell: label on top, then a fixed-width track with the value to its
    // right (kept narrow — the column is wider than the readout needs).
    let w = ui.available_width();
    let (cell, _) = ui.allocate_exact_size(Vec2::new(w, 42.0), Sense::hover());
    let pr = ui.painter_at(cell);
    let left = cell.left();
    let bar_w = (w - 96.0).clamp(120.0, 560.0);
    let ty = cell.top() + 10.0;

    // top row: label left, unit right (above the track)
    pr.text(Pos2::new(left, ty), Align2::LEFT_CENTER, r.label, FontId::monospace(11.0), c.fg_dim);
    if !r.unit.is_empty() {
        pr.text(Pos2::new(left + bar_w, ty), Align2::RIGHT_CENTER, &r.unit, FontId::monospace(11.0), c.fg_dim);
    }

    // track (bottom)
    let by = cell.top() + 29.0;
    let track = Rect::from_min_max(Pos2::new(left, by - 6.0), Pos2::new(left + bar_w, by + 6.0));
    pr.rect_filled(track, 2.0, c.stroke);
    let fill_col = if r.over { c.err } else { c.accent };
    pr.rect_filled(Rect::from_min_size(track.min, Vec2::new(bar_w * r.pct, track.height())), 2.0, fill_col);
    // threshold markers
    for m in [r.lo, r.hi].into_iter().flatten() {
        let mx = left + bar_w * m;
        pr.rect_filled(Rect::from_min_max(Pos2::new(mx - 1.0, track.min.y), Pos2::new(mx + 1.0, track.max.y)), 0.0, c.err);
    }
    // value (number only — unit is shown top-right), right of the track
    let vcol = if r.over { c.err } else { c.fg };
    pr.text(Pos2::new(left + bar_w + 12.0, by), Align2::LEFT_CENTER, &r.val_txt, FontId::monospace(13.0), vcol);
}

fn status_block(ui: &mut egui::Ui, c: &Colors, em: &Em, lang: Lang) {
    let mut rows = vec![
        (t(lang, "Клемма 15", "Terminal 15"), t(lang, "ВКЛ", "ON"), c.ok),
        (t(lang, "Связь с блоком", "Comm link"), t(lang, "УСТАНОВЛЕНА", "ESTABLISHED"), c.ok),
        (t(lang, "Кодирование", "Coding"), t(lang, "КОРРЕКТНО", "VALID"), c.ok),
    ];
    if em.faults > 0 {
        rows.push((t(lang, "Память ошибок", "Fault memory"), format!("{} {}", em.faults, t(lang, "зап.", "entr.")), c.warn));
    } else {
        rows.push((t(lang, "Память ошибок", "Fault memory"), t(lang, "пусто", "empty"), c.ok));
    }
    for (k, label, col) in rows {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
            let p = ui.painter_at(rect);
            p.text(Pos2::new(rect.left(), rect.center().y), Align2::LEFT_CENTER, k, FontId::monospace(11.0), c.fg_dim);
            p.text(Pos2::new(rect.right(), rect.center().y), Align2::RIGHT_CENTER, label, FontId::monospace(11.0), col);
            p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, c.stroke));
        });
    }
}

fn t(lang: Lang, ru: &str, en: &str) -> String {
    match lang {
        Lang::Ru => ru,
        Lang::En => en,
    }
    .to_string()
}
fn t2(lang: Lang, ru: &'static str, en: &'static str) -> &'static str {
    match lang {
        Lang::Ru => ru,
        Lang::En => en,
    }
}
