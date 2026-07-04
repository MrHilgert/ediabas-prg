//! Application state + eframe entry. Holds the selection/theme/lang state shared
//! across screens, applies theme visuals each frame, and routes to the active screen.

use std::time::Duration;

use crate::config::Settings;
use crate::data::{self, DATA};
use crate::ecu::Category;
use crate::lang::Lang;
use crate::screens;
use crate::theme::{self, Theme};
use crate::worker::Worker;

/// Register Inter as the UI font (modern, high-legibility, full Cyrillic),
/// ahead of egui's thin default. Used for both proportional and monospace text
/// so Cyrillic renders everywhere; the built-in fonts stay as emoji fallback.
fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "inter".to_owned(),
        FontData::from_static(include_bytes!("../assets/fonts/Inter.ttf")),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "inter".to_owned());
    ctx.set_fonts(fonts);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Chassis,
    Ecu,
    Session,
}

/// Connection lifecycle as one explicit state machine (replaces the old scattered
/// `connected`/`connect_pending`/`connect_attempted`/`connect_error` booleans). Exactly
/// one variant holds at a time, so the UI can never be e.g. "pending AND connected".
#[derive(Clone)]
pub enum Link {
    /// No link, nothing in flight.
    Idle,
    /// `Cmd::Connect` sent for `code`; awaiting `Connected`/`Error`.
    Connecting { code: &'static str },
    /// A live link to `code` (INITIALISIERUNG ran and the ECU answered).
    Connected { code: &'static str },
    /// The last connect attempt resolved without a link; `reason` is user-facing. (The
    /// failed module is whatever `ecu_sel` still points at — no need to duplicate it.)
    Failed { reason: String },
}

pub struct App {
    pub theme: Theme,
    pub lang: Lang,
    pub screen: Screen,

    // Chassis-select state
    pub series: char,
    pub chassis: Option<usize>, // index into DATA
    pub body: Option<usize>,    // index into the selected chassis' bodies
    pub query: String,

    // ECU-select state
    pub ecu_cat: Option<Category>,          // active category filter (always Some once on screen 2)
    pub ecu_sel: Option<&'static str>,      // selected module code

    // Session (screen 3): the ECU's parsed .ipo screen tree + navigation.
    pub module: Option<std::rc::Rc<inpa::ScreenModule>>, // loaded .ipo for the current ECU
    pub module_for: Option<&'static str>,   // which ECU code `module` was loaded for
    pub nav: Vec<crate::session::View>,     // view stack (menu + optional screen)
    pub fault_open: Option<String>,         // expanded fault code
    pub info_for: Option<usize>,            // TextInfo screen whose feeder we've read once
    pub info_tries: u8,                     // TextInfo feeder read attempts (bounded retry)
    pub info_busy: bool,                    // a TextInfo feeder job is in flight
    pub faults: Option<ediabas::JobResult>, // real FS_LESEN result (DDE)
    pub faults_busy: bool,                  // an FS_LESEN/FS_LOESCHEN is in flight
    pub faults_tries: u8,                   // auto-read attempts this visit (bounded retry)

    // Diagnostic worker (spun up on CONNECT). None until screen 2 connects.
    pub worker: Option<Worker>,
    pub status_msg: String,
    // Real connection / live data (screen 3, DDE via MW_SELECT_LESEN_NORM)
    pub link: Link,                        // connection state machine (see `Link`)
    pub streaming: bool,                   // a StartStream is active
    pub live: Option<ediabas::JobResult>,  // latest polled measurement set
    pub stream_poll: Option<(Vec<(String, String)>, u64)>, // active per-page poll: (reqs, interval_ms)
    // Comms-activity pulse for the header link dot.
    pub comms_seq: u64,                    // bumped on every completed exchange
    pub comms_seen: u64,                   // last seq the header observed
    pub comms_at: f64,                     // ctx time of that last change (for the fade)
    pub comms_miss: u32,                   // consecutive streaming poll misses (transient)

    // Settings (persisted to settings.ini)
    pub show_settings: bool,           // settings popup open
    pub port: Option<String>,          // chosen serial port (None = auto)
    pub ports: Vec<String>,            // last-enumerated available ports (for the picker)
    saved: Settings,                   // last-persisted snapshot (save-on-change)

    // Window
    pub fullscreen: bool,
    startup_frames: u8,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Restore persisted language/theme/port (or defaults on first run).
        let settings = Settings::load();
        let theme = settings.theme;
        install_fonts(&cc.egui_ctx);
        // Nudge everything up ~6% — a touch larger and easier to read, keeping
        // all the hand-tuned pixel layouts in proportion (scales on top of DPI).
        cc.egui_ctx.set_zoom_factor(1.06);
        cc.egui_ctx.set_visuals(theme::visuals(theme));

        // Default selection: E46 (the reference DDE chassis), first body.
        let e46 = DATA.iter().position(|c| c.code == "E46");

        Self {
            theme,
            lang: settings.lang,
            screen: Screen::Chassis,
            series: '3',
            chassis: e46,
            body: Some(0),
            query: String::new(),
            ecu_cat: Some(Category::Pwr),
            ecu_sel: None,
            module: None,
            module_for: None,
            nav: Vec::new(),
            fault_open: None,
            info_for: None,
            info_tries: 0,
            info_busy: false,
            faults: None,
            faults_busy: false,
            faults_tries: 0,
            worker: None,
            status_msg: String::new(),
            link: Link::Idle,
            streaming: false,
            live: None,
            stream_poll: None,
            comms_seq: 0,
            comms_seen: 0,
            comms_at: 0.0,
            comms_miss: 0,
            show_settings: false,
            port: settings.port.clone(),
            ports: Vec::new(),
            saved: settings,
            fullscreen: true,
            startup_frames: 3,
        }
    }

    /// Open the settings popup, refreshing the list of serial ports.
    pub fn open_settings(&mut self) {
        self.ports = ediabas::available_ports();
        self.show_settings = true;
    }

    /// Re-enumerate serial ports (settings "Refresh" button).
    pub fn refresh_ports(&mut self) {
        self.ports = ediabas::available_ports();
    }

    /// The serial port CONNECT will actually use: the chosen one, or the first
    /// available if set to auto. `None` only when auto and nothing is present.
    pub fn active_port(&self) -> Option<String> {
        self.port.clone().or_else(|| ediabas::available_ports().into_iter().next())
    }

    /// Current settings as reflected by live UI state.
    fn current_settings(&self) -> Settings {
        Settings { lang: self.lang, theme: self.theme, port: self.port.clone() }
    }

    /// Persist to `settings.ini` only when language/theme/port actually changed.
    fn persist_if_changed(&mut self) {
        let now = self.current_settings();
        if now != self.saved {
            now.save();
            self.saved = now;
        }
    }

    /// Currently selected chassis, if any.
    pub fn selected_chassis(&self) -> Option<&'static data::Chassis> {
        self.chassis.map(|i| &DATA[i])
    }

    /// Select a chassis by DATA index and reset the body to its first variant.
    pub fn select_chassis(&mut self, idx: usize) {
        self.chassis = Some(idx);
        self.body = Some(0);
    }

    /// Navigate one screen back (Esc / Back button). In a session, Esc pops the
    /// page stack first, only leaving to ECU-select when already at the root page.
    pub fn go_back(&mut self) {
        match self.screen {
            // Hierarchical back (not history): first close an open screen (→ its menu
            // list), then climb to the parent menu, then leave to ECU-select.
            Screen::Session => {
                if let Some(top) = self.nav.last_mut() {
                    if top.screen.is_some() {
                        top.screen = None;
                        return;
                    }
                }
                if self.nav.len() > 1 {
                    self.nav.pop();
                } else {
                    self.screen = Screen::Ecu;
                }
            }
            Screen::Ecu => self.screen = Screen::Chassis,
            Screen::Chassis => {}
        }
    }

    /// Reset the per-session view state (nav + cached results) for a fresh module.
    pub fn reset_session_view(&mut self) {
        self.nav.clear();
        self.fault_open = None;
        self.info_for = None;
        self.info_tries = 0;
        self.info_busy = false;
        self.faults = None;
        self.faults_busy = false;
        self.streaming = false;
        self.live = None;
        self.stream_poll = None;
        self.status_msg.clear();
    }

    /// Reset the connection lifecycle to `Idle` (no link, nothing in flight).
    fn reset_connection(&mut self) {
        self.link = Link::Idle;
    }

    /// The module code we currently hold a LIVE link to, if any.
    pub fn connected_code(&self) -> Option<&'static str> {
        match self.link {
            Link::Connected { code } => Some(code),
            _ => None,
        }
    }

    /// True while a connect handshake is in flight (spinner shown).
    pub fn is_connecting(&self) -> bool {
        matches!(self.link, Link::Connecting { .. })
    }

    /// The user-facing failure reason when the last attempt failed (drives the modal).
    pub fn link_failure(&self) -> Option<&str> {
        match &self.link {
            Link::Failed { reason, .. } => Some(reason.as_str()),
            _ => None,
        }
    }

    /// Drain ONE worker event into app state — the single place connection + session
    /// events are handled (formerly split between `ecu_select::poll_connect` and
    /// `session::drain_worker`).
    pub fn apply_event(&mut self, evt: crate::worker::Event) {
        use crate::worker::{Cmd, Event};
        match evt {
            Event::Connected { .. } => {
                // A handshake completed → promote Connecting → Connected.
                if let Link::Connecting { code } = self.link {
                    self.link = Link::Connected { code };
                }
                self.status_msg.clear();
            }
            Event::PollMiss(_) => {
                // Transient bus glitch: hold the last values AND the link. Only after
                // several consecutive misses treat it as a real disconnect.
                const MISS_LIMIT: u32 = 8;
                self.comms_miss += 1;
                if self.comms_miss >= MISS_LIMIT {
                    if matches!(self.link, Link::Connected { .. }) {
                        self.link = Link::Failed { reason: crate::i18n::t("err_no_response", self.lang) };
                    }
                    self.streaming = false;
                    self.status_msg = crate::i18n::t("no_link", self.lang);
                }
            }
            Event::JobDone { job, result } => {
                self.comms_seq = self.comms_seq.wrapping_add(1); // a real exchange completed → pulse
                self.comms_miss = 0; // link is live again
                if job == "FS_LESEN" {
                    self.faults = Some(result);
                    self.faults_busy = false;
                } else if job == "FS_LOESCHEN" {
                    self.faults = None;
                    self.faults_busy = true; // re-read after clearing; keep the busy state
                    if let Some(w) = &self.worker {
                        w.send(Cmd::RunJob { job: "FS_LESEN".into(), args: String::new() });
                    }
                } else {
                    self.info_busy = false; // a TextInfo feeder read completed
                    // Overlay onto the held set so a value missing this tick keeps its last
                    // reading instead of blanking — steady display, no per-tick flicker.
                    match &mut self.live {
                        Some(cur) => cur.overlay(result),
                        None => self.live = Some(result),
                    }
                }
            }
            Event::Error(e) => {
                self.faults_busy = false;
                self.info_busy = false;
                match self.link {
                    // A connect attempt failed → no link; show a clean reason.
                    Link::Connecting { .. } => {
                        self.link = Link::Failed { reason: humanize_connect_error(&e, self.lang) };
                        self.streaming = false;
                    }
                    // A job failed mid-session (e.g. one FS_LESEN telegram) — keep the link,
                    // just surface it. A truly dead bus is caught by the stream miss-counter.
                    _ => {
                        self.status_msg = format!("{}: {e}", crate::i18n::t("job_error", self.lang));
                    }
                }
            }
            Event::Disconnected => {
                self.link = Link::Idle;
                self.streaming = false;
            }
        }
    }

    /// Connection modal, rendered above every screen: a spinner while `Connecting`, or
    /// the failure reason + Retry/Close while `Failed`. Nothing when `Idle`/`Connected`.
    /// This is the single connect overlay (formerly `ecu_select::connect_modal` +
    /// `session::connecting_modal`).
    pub fn link_modal(&mut self, ctx: &egui::Context) {
        let (connecting, failure) = (self.is_connecting(), self.link_failure().map(str::to_owned));
        if !connecting && failure.is_none() {
            return;
        }
        let c = theme::palette(self.theme);
        let lang = self.lang;
        let code = self.ecu_sel.unwrap_or("");
        let screen = ctx.screen_rect();

        // Dim, click-swallowing backdrop.
        egui::Area::new(egui::Id::new("link_backdrop"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
                ui.painter().rect_filled(rect, 0.0, egui::Color32::from_black_alpha(170));
            });

        let frame = egui::Frame::none()
            .fill(c.panel)
            .stroke(egui::Stroke::new(1.0, c.stroke2))
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(30.0, 26.0));

        egui::Window::new("link_modal")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .frame(frame)
            .show(ctx, |ui| {
                use egui::RichText;
                ui.set_width(300.0);
                ui.vertical_centered(|ui| match &failure {
                    None => {
                        ui.add(egui::Spinner::new().size(30.0).color(c.accent));
                        ui.add_space(16.0);
                        ui.label(RichText::new(crate::i18n::t("connecting", lang)).size(13.0).strong().color(c.fg));
                        ui.add_space(6.0);
                        ui.label(RichText::new(format!("{code} · INITIALISIERUNG")).size(10.0).color(c.fg_dim));
                    }
                    Some(msg) => {
                        ui.label(RichText::new("⚠").size(30.0).color(c.err));
                        ui.add_space(12.0);
                        ui.label(RichText::new(crate::i18n::t("connect_failed", lang)).size(13.0).strong().color(c.err));
                        ui.add_space(8.0);
                        ui.label(RichText::new(msg).size(10.0).color(c.fg_dim));
                        ui.add_space(20.0);
                        ui.horizontal(|ui| {
                            let retry = egui::Button::new(RichText::new(crate::i18n::t("retry", lang)).size(11.0).strong().color(c.accent_fg))
                                .fill(c.accent).rounding(3.0).min_size(egui::Vec2::new(140.0, 32.0));
                            if ui.add(retry).clicked() {
                                self.start_connect(ctx);
                            }
                            ui.add_space(10.0);
                            let close = egui::Button::new(RichText::new(crate::i18n::t("close", lang)).size(11.0).color(c.fg_dim))
                                .fill(c.panel2).stroke(egui::Stroke::new(1.0, c.stroke)).rounding(3.0).min_size(egui::Vec2::new(140.0, 32.0));
                            if ui.add(close).clicked() {
                                self.link = Link::Idle;
                            }
                        });
                    }
                });
            });

        if connecting {
            ctx.request_repaint(); // keep the spinner animating until the handshake resolves
        }
    }

    /// CONNECT pressed on screen 2. Every module backed by a real SGBD (`.prg`)
    /// must establish a link BEFORE the session screen opens: we open the port and
    /// run its actual `INITIALISIERUNG` here, staying on ECU-select; `ecu_select`
    /// watches for the result and only switches to `Screen::Session` on a
    /// successful `Connected` (a failed attempt keeps the user on screen 2 with an
    /// error popup). Modules without an SGBD can't be reached — report that plainly
    /// rather than opening a fake session.
    pub fn start_connect(&mut self, _ctx: &egui::Context) {
        self.reset_session_view();
        self.reset_connection();
        self.module_for = None; // force the session screen to (re)load this ECU's .ipo
        let module = self.ecu_sel.and_then(|code| {
            self.selected_chassis()
                .map(crate::ecu::mods_for)
                .and_then(|v| v.into_iter().find(|m| m.code == code))
        });
        match module {
            // Any ECU with a screen script (.ipo) can open the session — its structure
            // renders even without a transport; the link (if any) comes up in the session.
            Some(m) if m.script.is_some() || m.prg.is_some() => self.screen = Screen::Session,
            _ => self.link = Link::Failed { reason: crate::i18n::t("no_sgbd", self.lang) },
        }
    }

    /// Enter the ECU-select screen for the current chassis and kick off the scan.
    pub fn enter_ecu(&mut self) {
        self.screen = Screen::Ecu;
        self.ecu_cat = Some(Category::Pwr);
        self.ecu_sel = crate::ecu::mods_for(&DATA[self.chassis.unwrap_or(0)])
            .first()
            .map(|m| m.code);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Force fullscreen a couple of frames after the window is up, as a real
        // maximized→fullscreen transition (the first frame can be too early to honor).
        if self.startup_frames > 0 {
            self.startup_frames -= 1;
            if self.startup_frames == 0 && self.fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            }
            ctx.request_repaint();
        }
        // F11 toggles fullscreen.
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        // Esc = navigate back (like the Back button), NOT exit fullscreen.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.go_back();
        }

        // Theme can flip at runtime; applying every frame is cheap.
        ctx.set_visuals(theme::visuals(self.theme));
        // Keep the clock / pulse animating.
        ctx.request_repaint_after(Duration::from_secs(1));

        // Single worker-event drain for the whole app (connection + session events),
        // BEFORE any screen renders, so every screen sees a consistent `link`.
        let events = self.worker.as_ref().map(|w| w.poll()).unwrap_or_default();
        for evt in events {
            self.apply_event(evt);
        }

        match self.screen {
            Screen::Chassis => screens::chassis_select::show(self, ctx),
            Screen::Ecu => screens::ecu_select::show(self, ctx),
            Screen::Session => crate::session::show(self, ctx),
        }

        // The connect overlay (spinner / error) renders above every screen.
        self.link_modal(ctx);
        // Settings popup renders above any screen; persist any change it (or the
        // header toggles) made this frame.
        screens::settings_modal(self, ctx);
        self.persist_if_changed();
    }
}

/// Turn a raw worker/transport error into a short, user-facing reason. The technical
/// string (VM/IO detail) stays in the trace; the modal shows only this.
fn humanize_connect_error(raw: &str, lang: Lang) -> String {
    let low = raw.to_ascii_lowercase();
    let key = if low.contains("timed out") || low.contains("timeout")
        || low.contains("no response") || low.contains("не отвеч") || low.contains("did not answer")
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
    crate::i18n::t(key, lang)
}

