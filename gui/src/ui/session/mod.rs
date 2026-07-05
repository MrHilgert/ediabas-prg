//! Конструктор экрана 3: строит его целиком из `inpa::ScreenModule` ЭБУ.
//!
//! Слой ОТРИСОВКИ: навигирует доменную модель экранов (`inpa`) ради разметки — меню,
//! страницы, F-клавиши, подписи строк — и рисует значения из view-моделей (`app.live`
//! типа `MeasFrame`, `app.faults` типа `FaultView`). Про коммуникацию с ЭБУ ничего не
//! знает: любые действия уходят как `model::Intent`, данные приходят уже декодированными.
//! Здесь нет ни `ediabas`, ни имён протокольных джобов.

mod faults;
mod widgets;

use std::rc::Rc;

use egui::{Key, RichText};
use inpa::model::{NavTarget, ScreenKind, ScreenModule};

use crate::app::{App, Link, Screen};
use crate::ecu::{mods_for, Module};
use crate::i18n::{t, tr_prg};
use crate::model::Intent;
use crate::ui::header;
use crate::ui::theme::palette;

/// UI-внутренние семантические токены F-клавиш страницы ошибок. Это НЕ имена джобов
/// ЭБУ (те живут только в `link/`), а маркеры намерения: `apply` превращает их в
/// `Intent::ReadFaults`/`ClearFaults`.
const TOK_READ: &str = "@read_faults";
const TOK_CLEAR: &str = "@clear_faults";

/// One navigation level: which menu drives the F-keys, and which screen (if any) is open.
#[derive(Clone, Copy)]
pub struct View {
    pub menu: usize,
    pub screen: Option<usize>,
}

/// An action produced by a click or F-key, applied at end of frame.
pub(super) enum Act {
    Open(View),
    /// Nav/activation job from the `.ipo` (job/arg are inpa domain data, passed through).
    RunJob { job: String, arg: String },
    /// Read fault memory (fault-page F1). Semantic — link maps it to the real job.
    ReadFaults,
    /// Clear fault memory (fault-page F2). Semantic — link maps it to the real job.
    ClearFaults,
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

    lifecycle(app, &m, &sm);

    // Gate: a connectable ECU (real SGBD) must hold a LIVE link before its session
    // renders. While the handshake is in flight → blank backdrop only (the spinner is
    // `App::link_modal`, drawn above every screen). If it resolved WITHOUT a link
    // (Idle/Failed) → don't open a dead session: return to ECU-select, where the error
    // modal shows. Structure-only ECUs (no SGBD in the .ipo) skip the gate entirely.
    let connectable = !sm.sgbd.is_empty() || !sm.group_files.is_empty();
    if connectable && app.connected_code() != Some(m.code) {
        header(app, ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(c.bg))
            .show(ctx, |_| {});
        if !app.is_connecting() {
            // Not connecting and not connected → the attempt failed (or was never made);
            // leave the dead session. Any failure reason already lives in `app.link`.
            app.screen = Screen::Ecu;
            ctx.request_repaint();
        }
        return;
    }

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
pub(crate) fn displayable(it: &inpa::NavItem) -> bool {
    !it.label.trim().is_empty()
        && !matches!(it.target, NavTarget::Exit | NavTarget::Script(_) | NavTarget::Other(_))
}

/// Standard fault-page F-keys (INPA puts Read/Clear in the F-zone): F1 read, F2 clear,
/// F10 back out of the fault screen. The read/clear targets carry UI semantic tokens, not
/// ЭБУ job names — `item_action`/`apply` turn them into `Intent::ReadFaults`/`ClearFaults`.
fn fault_fkeys() -> Vec<inpa::NavItem> {
    use inpa::model::{JobCall, NavItem};
    let tok = |token: &str| {
        NavTarget::Job(JobCall { job: token.into(), arg: String::new(), selector: None })
    };
    vec![
        NavItem { fkey: 1, shifted: false, label: "Прочитать".into(), target: tok(TOK_READ) },
        NavItem { fkey: 2, shifted: false, label: "Удалить".into(), target: tok(TOK_CLEAR) },
        NavItem { fkey: 10, shifted: false, label: "Назад".into(), target: NavTarget::Back },
    ]
}

/// Render the already-filtered menu items as a clickable list, numbered by F-key.
fn render_menu(
    ui: &mut egui::Ui,
    c: &crate::ui::theme::Colors,
    items: &[inpa::NavItem],
    sm: &ScreenModule,
    view: &View,
    act: &mut Option<Act>,
    app: &App,
) {
    // Menu title is shown once, in the header (context_strip) — not repeated here.
    // Guard against a level with no navigable items rendering as a blank page: show a
    // placeholder. `go_back` skips such dead levels, so this is a last-resort safety net.
    if items.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new(t("no_menu_items", app.lang)).size(12.0).color(c.fg_faint));
        return;
    }
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
    c: &crate::ui::theme::Colors,
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
            // Fault-page F-keys carry UI semantic tokens; everything else is a real
            // inpa-defined nav/activation job passed through to the ЭБУ-слой.
            match j.job.as_str() {
                TOK_READ => Some(Act::ReadFaults),
                TOK_CLEAR => Some(Act::ClearFaults),
                _ => Some(Act::RunJob { job: j.job.clone(), arg: j.arg.clone() }),
            }
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
        Act::ReadFaults => {
            app.faults_busy = true; // show a reading state until the result lands
            app.faults_tries = app.faults_tries.saturating_add(1); // bounded auto-retry
            app.worker.send(Intent::ReadFaults);
        }
        Act::ClearFaults => {
            app.faults = None;
            app.faults_busy = true; // clearing → re-read is driven by the ЭБУ-слой
            app.worker.send(Intent::ClearFaults);
        }
        Act::RunJob { job, arg } => {
            app.worker.send(Intent::NavJob { job, arg });
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
/// case-sensitive (Linux) filesystem. This is the render-side copy for layout; the
/// ЭБУ-слой parses its own copy for polling/decoding.
fn load_ipo(script: &str) -> Option<ScreenModule> {
    let path = crate::i18n::resolve_ci("SGDAT", &format!("{script}.ipo"))?;
    inpa::parse(&path).ok()
}

/// Establish the transport link once. The UI only names the ECU (`script`/`code`) and the
/// chosen port; the ЭБУ-слой resolves SGBD/groups/variant and opens the transport. Only
/// ECUs whose `.ipo` names an SGBD get a connect attempt; the rest render structure-only.
fn lifecycle(app: &mut App, m: &Module, sm: &ScreenModule) {
    let connectable = !sm.sgbd.is_empty() || !sm.group_files.is_empty();
    // Send the ONE Connect for this session while fresh (Idle). A Failed attempt does NOT
    // auto-retry — the link modal's Retry re-arms it via `App::start_connect`.
    if connectable && matches!(app.link, Link::Idle) {
        // Script name = the .ipo stem the ЭБУ-слой will (re)parse to resolve the SGBD.
        let script = m.script.unwrap_or(m.code).to_string();
        let port = app.active_port(); // "" = auto (link resolves to first available)
        app.iface = Some(m.bus.to_string()); // header chip shows the catalog protocol family
        app.worker.send(Intent::Connect { script, port });
        app.link = Link::Connecting { code: m.code };
    }
}

/// Start/stop/switch the live poll for the open data-stream screen. The UI only tells the
/// ЭБУ-слой WHICH screen is live (`Intent::SetLive`); the poll list + decode live there.
fn update_poll(app: &mut App, m: &Module, sm: &ScreenModule, view: &View) {
    let desired: Option<usize> = if app.connected_code() == Some(m.code) {
        view.screen
            .and_then(|s| sm.as_screen(s).map(|sc| (s, sc.kind)))
            .filter(|(_, kind)| *kind == ScreenKind::DataStream)
            .map(|(id, _)| id)
    } else {
        None
    };
    if desired != app.live_screen {
        match desired {
            Some(id) => app.worker.send(Intent::SetLive(id)),
            None => {
                app.worker.send(Intent::StopLive);
                app.live = None;
            }
        }
        app.live_screen = desired;
        app.streaming = desired.is_some();
    }
}

/// One-shot loader for a TextInfo screen (ident / info / coding): its data is static, so
/// the ЭБУ-слой reads it once (`Intent::OpenInfo`) and the decoded frame lands in
/// `app.live`. A transient first-read miss is retried a bounded number of times.
fn update_textinfo(app: &mut App, m: &Module, sm: &ScreenModule, view: &View) {
    let target = view
        .screen
        .filter(|_| app.connected_code() == Some(m.code))
        .filter(|s| sm.as_screen(*s).map(|sc| sc.kind) == Some(ScreenKind::TextInfo));
    let Some(sid) = target else {
        app.info_for = None; // left the TextInfo screen — re-read on next open
        return;
    };
    // Читаем, если у экрана есть display-строки ИЛИ feeder-джоб: ident/status без строк
    // (`s_ident`) полагаются на авто-дамп всего результат-сета джоба (см. decode.rs).
    let fetchable = sm
        .as_screen(sid)
        .map(|sc| !sc.rows.is_empty() || sc.feeder.is_some())
        .unwrap_or(false);
    let no_data = app.live.as_ref().map_or(true, |f| !f.has_data());
    if app.info_for != Some(sid) {
        // A new TextInfo screen just opened → clear stale state and fetch this one once.
        app.info_for = Some(sid);
        app.info_tries = 0;
        app.info_busy = false;
        app.live = None;
        app.status_msg.clear();
        if fetchable {
            app.worker.send(Intent::OpenInfo(sid));
            app.info_busy = true;
            app.info_tries = 1;
        }
    } else if fetchable && no_data && !app.info_busy && app.info_tries < 3 {
        // The previous read finished without data (a transient timeout on the stream→job
        // transition can eat the first request) → retry, up to a small cap.
        app.worker.send(Intent::OpenInfo(sid));
        app.info_busy = true;
        app.info_tries += 1;
    }
}
