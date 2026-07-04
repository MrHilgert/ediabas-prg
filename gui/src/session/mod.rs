//! The GUI constructor: builds screen 3 entirely from an ECU's `inpa::ScreenModule`.
//!
//! Flow: chassis → ECU (hard-coded) → load the ECU's `.ipo` → extract its SGBD (`.prg`)
//! and screen tree → render generically and (where a transport exists) stream live data.
//! No per-ECU code: every menu, page, parameter, binding and F-key comes from the `.ipo`.
//!
//! Navigation mirrors INPA: a *view* is an active menu (the F-key bar) plus an optional
//! open screen (the content). Selecting a menu item pushes a new view.

mod faults;
mod widgets;

use std::rc::Rc;

use egui::{Key, RichText};
use inpa::model::{NavTarget, Row, ScreenKind, ScreenModule};

use crate::app::{App, Screen};
use crate::ecu::{mods_for, Module};
use crate::i18n::{t, tr_prg};
use crate::screens::header;
use crate::theme::palette;
use crate::worker::{Cmd, Event, Worker};

/// One navigation level: which menu drives the F-keys, and which screen (if any) is open.
#[derive(Clone, Copy)]
pub struct View {
    pub menu: usize,
    pub screen: Option<usize>,
}

/// An action produced by a click or F-key, applied at end of frame.
pub(super) enum Act {
    Open(View),
    RunJob { job: String, arg: String },
    ToggleFault(String),
    Back,
    Exit,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let c = palette(app.theme);

    // Resolve the selected module.
    let Some(m) = current_module(app) else {
        header(app, ctx);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(t("pick_module", app.lang)).color(c.fg_faint));
            });
        });
        return;
    };

    // Load this ECU's .ipo once (structure is available for ANY ECU with a script).
    ensure_module(app, &m, ctx);

    let Some(sm) = app.module.clone() else {
        header(app, ctx);
        egui::CentralPanel::default().frame(egui::Frame::none().fill(c.bg)).show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                let msg = t("no_ipo", app.lang).replace("{code}", m.code);
                ui.label(RichText::new(msg).color(c.fg_faint));
            });
        });
        return;
    };

    drain_worker(app);
    lifecycle(app, &m, &sm, ctx);

    // Current view (menu + optional screen).
    let view = app.nav.last().copied().unwrap_or(View { menu: sm.root, screen: None });

    let on_faults =
        view.screen.and_then(|s| sm.as_screen(s)).map(|s| s.kind) == Some(ScreenKind::Faults);

    // INPA-style Shift paging: unshifted items (F1..F10) by default, the shifted set
    // (Shift+F1..F10) while the Shift key is held. The F-key bar shows a fixed 10 slots
    // (empty ones inert); the central list shows only real functions.
    //
    // A fault page swaps in standard fault F-keys (F1 read / F2 clear) — INPA puts the
    // Read/Clear actions in the F-zone, not on the screen.
    let shift = ctx.input(|i| i.modifiers.shift);
    let page_items: Vec<inpa::NavItem> = if on_faults {
        fault_fkeys()
    } else {
        let all_items = sm.as_menu(view.menu).map(|mn| mn.items.clone()).unwrap_or_default();
        all_items.into_iter().filter(|it| it.shifted == shift).collect()
    };
    let list_items: Vec<inpa::NavItem> =
        page_items.iter().filter(|it| displayable(it)).cloned().collect();

    // Live-poll lifecycle for the open data-stream screen.
    update_poll(app, &m, &sm, &view);
    // TextInfo screens (ident / info / coding) read their feeder job(s) once on open.
    update_textinfo(app, &m, &sm, &view);

    // Reset the faults auto-read attempts whenever we're not on a fault page (so the next
    // visit reads again).
    if !on_faults {
        app.faults_tries = 0;
    }

    // Keyboard: F1..F10 fire the matching slot (INPA Exit → our native exit); Esc = Back.
    let mut act: Option<Act> = ctx.input(|i| {
        const KEYS: [Key; 10] = [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10];
        for (idx, k) in KEYS.iter().enumerate() {
            if i.key_pressed(*k) {
                if let Some(it) = page_items.iter().find(|it| it.fkey as usize == idx + 1) {
                    return item_action(&sm, app, &view, it);
                }
            }
        }
        None
    });

    header(app, ctx);
    widgets::fkey_panel(ctx, &c, &page_items, &mut act, &sm, app, &view, app.lang);

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(c.bg))
        .show(ctx, |ui| {
            widgets::context_strip(ui, app, &c, &m, &sm, &view);
            let open = view.screen.and_then(|s| sm.as_screen(s));
            // A data-stream page fills the whole viewport (bars stretch to fill height/width);
            // everything else scrolls at its natural size.
            let fill = matches!(open.map(|s| s.kind), Some(ScreenKind::DataStream));
            if fill {
                egui::Frame::none().inner_margin(16.0).show(ui, |ui| {
                    render_screen(ui, app, ctx, &c, open.unwrap(), &mut act);
                });
            } else {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Frame::none().inner_margin(16.0).show(ui, |ui| {
                        match open {
                            Some(screen) => render_screen(ui, app, ctx, &c, screen, &mut act),
                            None => render_menu(ui, &c, &list_items, &sm, &view, &mut act, app),
                        }
                    });
                });
            }
        });

    if let Some(a) = act.take() {
        apply(app, a);
    }
}

/// Should a menu item be shown at all? Hides empty labels and INPA infrastructure items
/// (Select/Deselect = scriptselect, editor = callwin, Exit) — only real functions remain.
fn displayable(it: &inpa::NavItem) -> bool {
    !it.label.trim().is_empty()
        && !matches!(it.target, NavTarget::Exit | NavTarget::Script(_) | NavTarget::Other(_))
}

/// Standard fault-page F-keys (INPA puts Read/Clear in the F-zone): F1 read, F2 clear,
/// F10 back out of the fault screen.
fn fault_fkeys() -> Vec<inpa::NavItem> {
    use inpa::model::{JobCall, NavItem};
    let job = |name: &str| {
        NavTarget::Job(JobCall { job: name.into(), arg: String::new(), selector: None })
    };
    vec![
        NavItem { fkey: 1, shifted: false, label: "Прочитать".into(), target: job("FS_LESEN") },
        NavItem { fkey: 2, shifted: false, label: "Удалить".into(), target: job("FS_LOESCHEN") },
        NavItem { fkey: 10, shifted: false, label: "Назад".into(), target: NavTarget::Back },
    ]
}

/// Render the already-filtered menu items as a clickable list, numbered by F-key.
fn render_menu(
    ui: &mut egui::Ui,
    c: &crate::theme::Colors,
    items: &[inpa::NavItem],
    sm: &ScreenModule,
    view: &View,
    act: &mut Option<Act>,
    app: &App,
) {
    // Menu title is shown once, in the header (context_strip) — not repeated here.
    for it in items {
        let label = tr_prg(&it.label, app.lang);
        if widgets::menu_row(ui, c, it.fkey as usize, &label, &it.target) {
            *act = item_action(sm, app, view, it);
        }
    }
}

/// Render an open screen according to its archetype.
fn render_screen(
    ui: &mut egui::Ui,
    app: &App,
    ctx: &egui::Context,
    c: &crate::theme::Colors,
    screen: &inpa::Screen,
    act: &mut Option<Act>,
) {
    // Title is shown once, in the header (context_strip) — not repeated above the content.
    match screen.kind {
        ScreenKind::DataStream => {
            // While the poll is stalling, the shown values are held (old) → render greyer.
            let stale = app.comms_miss > 0;
            widgets::stream(ui, ctx, c, &screen.rows, app.live.as_ref(), stale, app.lang)
        }
        ScreenKind::TextInfo => {
            widgets::textinfo(ui, c, &screen.rows, &screen.info, app.live.as_ref(), app, app.lang)
        }
        ScreenKind::Activation => widgets::activation(ui, c, &screen.rows, app.lang),
        ScreenKind::Faults => faults::render(ui, c, app, act, app.lang),
        _ => {
            let n = screen.rows.len();
            let msg = t("screen_rows", app.lang).replace("{n}", &n.to_string());
            ui.label(RichText::new(msg).color(c.fg_dim));
        }
    }
}

/// Turn a menu item into the action it triggers.
pub(super) fn item_action(sm: &ScreenModule, app: &App, view: &View, it: &inpa::NavItem) -> Option<Act> {
    match &it.target {
        NavTarget::Screen(s) => Some(Act::Open(View { menu: view.menu, screen: Some(*s) })),
        NavTarget::ScreenAndMenu { screen, menu } => {
            Some(Act::Open(View { menu: *menu, screen: Some(*screen) }))
        }
        NavTarget::Menu(mn) => Some(Act::Open(View { menu: *mn, screen: None })),
        NavTarget::Job(j) => {
            let _ = (sm, app);
            Some(Act::RunJob { job: j.job.clone(), arg: j.arg.clone() })
        }
        NavTarget::Back => Some(Act::Back),
        NavTarget::Exit => Some(Act::Exit),
        NavTarget::Script(_) | NavTarget::Other(_) => None,
    }
}

fn apply(app: &mut App, a: Act) {
    match a {
        // Switching the open screen within the SAME menu replaces (siblings, no history);
        // entering a different menu goes one level deeper (push).
        Act::Open(v) => match app.nav.last_mut() {
            Some(top) if top.menu == v.menu => top.screen = v.screen,
            _ => app.nav.push(v),
        },
        Act::ToggleFault(code) => {
            app.fault_open = if app.fault_open.as_deref() == Some(code.as_str()) {
                None
            } else {
                Some(code)
            };
        }
        Act::Back => app.go_back(),
        Act::Exit => app.screen = Screen::Ecu,
        Act::RunJob { job, arg } => {
            if job == "FS_LESEN" || job == "FS_LOESCHEN" {
                app.faults_busy = true; // show a reading/clearing state until the result lands
            }
            if job == "FS_LESEN" {
                app.faults_tries = app.faults_tries.saturating_add(1); // bounded auto-retry
            }
            if let Some(w) = &app.worker {
                w.send(Cmd::RunJob { job, args: arg });
            }
        }
    }
}

// ---------------------------------------------------------------- lifecycle --

fn current_module(app: &App) -> Option<Module> {
    app.ecu_sel.and_then(|code| {
        app.selected_chassis().map(mods_for).and_then(|v| v.into_iter().find(|m| m.code == code))
    })
}

/// Load (once per ECU) the `.ipo` for `m` into `app.module`, resetting the session view.
fn ensure_module(app: &mut App, m: &Module, _ctx: &egui::Context) {
    if app.module_for == Some(m.code) {
        return;
    }
    app.module_for = Some(m.code);
    // Try the mapped script name, falling back to the ECU code (often == the .ipo stem).
    app.module = m
        .script
        .or(Some(m.code))
        .and_then(load_ipo)
        .map(Rc::new);
    app.reset_session_view();
    if let Some(sm) = &app.module {
        app.nav = vec![View { menu: sm.root, screen: None }];
    }
}

/// Resolve and parse `SGDAT/<script>.ipo`. The lookup is fully case-insensitive
/// (name AND extension), so a catalog script `kombi` finds `KOMBI.IPO` on a
/// case-sensitive (Linux) filesystem.
fn load_ipo(script: &str) -> Option<ScreenModule> {
    let path = crate::i18n::resolve_ci("SGDAT", &format!("{script}.ipo"))?;
    inpa::parse(&path).ok()
}

/// Drain worker events into app state.
fn drain_worker(app: &mut App) {
    let Some(w) = &app.worker else { return };
    for evt in w.poll() {
        match evt {
            Event::Connected { .. } => {
                app.connected = app.ecu_sel;
                app.connect_pending = false;
                app.status_msg.clear();
            }
            Event::PollMiss(_) => {
                // Transient bus glitch: keep the last drawn values AND the link. Only after
                // several consecutive misses do we treat it as a real disconnect.
                const MISS_LIMIT: u32 = 8;
                app.comms_miss += 1;
                if app.comms_miss >= MISS_LIMIT {
                    app.connected = None; // update_poll then stops the stream and clears live
                    app.streaming = false;
                    app.status_msg = "нет связи".into();
                }
            }
            Event::JobDone { job, result } => {
                app.comms_seq = app.comms_seq.wrapping_add(1); // a real exchange completed → pulse
                app.comms_miss = 0; // link is live again
                if job == "FS_LESEN" {
                    app.faults = Some(result);
                    app.faults_busy = false;
                } else if job == "FS_LOESCHEN" {
                    app.faults = None;
                    app.faults_busy = true; // re-read after clearing; keep the busy state
                    if let Some(w) = &app.worker {
                        w.send(Cmd::RunJob { job: "FS_LESEN".into(), args: String::new() });
                    }
                } else {
                    app.info_busy = false; // a TextInfo feeder read completed
                    // Overlay onto the held set so a value missing this tick keeps its last
                    // reading instead of blanking — steady display, no per-tick flicker.
                    match &mut app.live {
                        Some(cur) => cur.overlay(result),
                        None => app.live = Some(result),
                    }
                }
            }
            Event::Error(e) => {
                app.faults_busy = false;
                app.info_busy = false;
                if app.connect_pending || app.connected.is_none() {
                    // A connect attempt failed → no link.
                    app.connected = None;
                    app.connect_pending = false;
                    app.streaming = false;
                    app.status_msg = format!("нет связи: {e}");
                } else {
                    // A job failed mid-session (e.g. one FS_LESEN telegram) — keep the link,
                    // just surface it. A truly dead bus is caught by the stream miss-counter.
                    app.status_msg = format!("ошибка задания: {e}");
                }
            }
            Event::Disconnected => {
                app.connected = None;
                app.streaming = false;
            }
        }
    }
}

/// Establish the transport link once, using the PRG the `.ipo` named. Only ECUs with a
/// real SGBD on a built transport (DDE/DS2 today) connect; the rest render structure-only.
fn lifecycle(app: &mut App, m: &Module, sm: &ScreenModule, ctx: &egui::Context) {
    let connectable = m.prg.is_some() && !sm.sgbd.is_empty();
    if connectable && app.connected != Some(m.code) && !app.connect_attempted {
        let prg = format!("{}.prg", sm.sgbd); // fallback SGBD from the .ipo
        // Port from settings (or first available if set to auto); fall back to COM3.
        let port = app.active_port().unwrap_or_else(|| "COM3".into());
        // The `.ipo` names address-keyed group files (`D_<ADDR>`); the worker runs each
        // group's IDENTIFIKATION → VARIANTE to resolve the concrete variant SGBD before
        // opening the real session (INPA variant identification).
        let groups = sm.group_files.clone();
        let w = app.worker.get_or_insert_with(|| Worker::spawn(ctx.clone()));
        w.send(Cmd::Connect { port, baud: 9600, prg, groups });
        app.connect_attempted = true;
        app.connect_pending = true;
    }
}

/// Start/stop/switch the live poll for the open data-stream screen.
fn update_poll(app: &mut App, m: &Module, sm: &ScreenModule, view: &View) {
    let desired = if app.connected == Some(m.code) {
        view.screen
            .and_then(|s| sm.as_screen(s))
            .filter(|s| s.kind == ScreenKind::DataStream)
            // 0 = poll as fast as the transport allows (worker floors the tick to 20 ms);
            // the real rate is bound by the DS2 exchange time, not an artificial delay.
            .map(|s| (poll_reqs(s), 0u64))
            .filter(|(reqs, _)| !reqs.is_empty())
    } else {
        None
    };
    if desired != app.stream_poll {
        if let Some(w) = &app.worker {
            match &desired {
                Some((reqs, ms)) => w.send(Cmd::StartStream { reqs: reqs.clone(), interval_ms: *ms }),
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

/// One-shot loader for a TextInfo screen (ident / info / coding): its data is static,
/// so instead of streaming we run the screen's feeder + row jobs a single time when it
/// opens, letting the results land in `app.live` (same overlay path as the stream).
fn update_textinfo(app: &mut App, m: &Module, sm: &ScreenModule, view: &View) {
    let target = view
        .screen
        .filter(|_| app.connected == Some(m.code))
        .filter(|s| sm.as_screen(*s).map(|sc| sc.kind) == Some(ScreenKind::TextInfo));
    let Some(sid) = target else {
        app.info_for = None; // left the TextInfo screen — re-read on next open
        return;
    };
    let reqs = sm.as_screen(sid).map(poll_reqs).unwrap_or_default();
    let send = |app: &mut App| {
        if let Some(w) = &app.worker {
            for (job, arg) in &reqs {
                w.send(Cmd::RunJob { job: job.clone(), args: arg.clone() });
            }
        }
        app.info_busy = true;
    };
    if app.info_for != Some(sid) {
        // A new TextInfo screen just opened → clear stale state and fetch this one once.
        app.info_for = Some(sid);
        app.info_tries = 0;
        app.info_busy = false;
        app.live = None;
        app.status_msg.clear();
        if !reqs.is_empty() {
            send(app);
            app.info_tries = 1;
        }
    } else if !reqs.is_empty() && app.live.is_none() && !app.info_busy && app.info_tries < 3 {
        // The previous read finished without data (a transient timeout on the stream→job
        // transition can eat the first request) → retry, up to a small cap.
        send(app);
        app.info_tries += 1;
    }
}

/// The per-tick request list for a data-stream screen: batch `MW_SELECT_LESEN_NORM`
/// selectors (≤10 per request), run other jobs individually, plus the feeder once.
fn poll_reqs(screen: &inpa::Screen) -> Vec<(String, String)> {
    let mut selectors: Vec<String> = Vec::new();
    let mut jobs: Vec<(String, String)> = Vec::new();
    let mut push_job = |job: &str, arg: &str| {
        if !job.is_empty() && !jobs.iter().any(|(j, a)| j == job && a == arg) {
            jobs.push((job.to_string(), arg.to_string()));
        }
    };
    if let Some(f) = &screen.feeder {
        if f.job != "MW_SELECT_LESEN_NORM" {
            push_job(&f.job, &f.arg);
        }
    }
    for row in &screen.rows {
        let j = row.job();
        match &j.selector {
            Some(sel) if j.job == "MW_SELECT_LESEN_NORM" => {
                if !selectors.contains(sel) {
                    selectors.push(sel.clone());
                }
            }
            _ => push_job(&j.job, &j.arg),
        }
    }
    let mut reqs: Vec<(String, String)> = selectors
        .chunks(10)
        .map(|c| ("MW_SELECT_LESEN_NORM".to_string(), c.concat()))
        .collect();
    reqs.extend(jobs);
    reqs
}

/// Read a row's live value + unit from the polled `JobResult`.
pub(crate) fn row_value(row: &Row, live: Option<&ediabas::JobResult>) -> (Option<f64>, String) {
    let Some(res) = row.result() else { return (None, String::new()) };
    let v = live.and_then(|l| l.get_f64(res));
    let unit = match row {
        Row::Analog { unit, .. } => res
            .strip_suffix("_WERT")
            .map(|b| format!("{b}_EINH"))
            .and_then(|k| live.and_then(|l| l.get_str(&k)))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| unit.clone()),
        _ => String::new(),
    };
    (v, unit)
}
