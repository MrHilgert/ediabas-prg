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
    pub scan_pct: f32,                       // 0..100 module-scan animation
    pub connect_error: Option<String>,      // last connect failure → error popup on screen 2

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
    pub connected: Option<&'static str>,   // module code we've INITIALISIERUNG'd
    pub connect_pending: bool,             // a Connect is in flight right now
    pub connect_attempted: bool,           // tried once this session (no retry spam)
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
            scan_pct: 0.0,
            connect_error: None,
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
            connected: None,
            connect_pending: false,
            connect_attempted: false,
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

    /// Reset the connection lifecycle (no link established yet).
    fn reset_connection(&mut self) {
        self.connected = None;
        self.connect_pending = false;
        self.connect_attempted = false;
        self.connect_error = None;
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
            _ => self.connect_error = Some(crate::i18n::t("no_sgbd", self.lang)),
        }
    }

    /// Enter the ECU-select screen for the current chassis and kick off the scan.
    pub fn enter_ecu(&mut self) {
        self.screen = Screen::Ecu;
        self.ecu_cat = Some(Category::Pwr);
        self.ecu_sel = crate::ecu::mods_for(&DATA[self.chassis.unwrap_or(0)])
            .first()
            .map(|m| m.code);
        self.scan_pct = 0.0;
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

        match self.screen {
            Screen::Chassis => screens::chassis_select::show(self, ctx),
            Screen::Ecu => screens::ecu_select::show(self, ctx),
            Screen::Session => crate::session::show(self, ctx),
        }

        // Settings popup renders above any screen; persist any change it (or the
        // header toggles) made this frame.
        screens::settings_modal(self, ctx);
        self.persist_if_changed();
    }
}

