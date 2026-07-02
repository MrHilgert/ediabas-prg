//! Application state + eframe entry. Holds the selection/theme/lang state shared
//! across screens, applies theme visuals each frame, and routes to the active screen.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::data::{self, DATA};
use crate::ecu::Category;
use crate::lang::Lang;
use crate::screens;
use crate::theme::{self, Theme};
use crate::worker::{Cmd, Worker};

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

    // Session (screen 3) navigation state
    pub ses_stack: Vec<String>,             // page-id stack; "main" at bottom
    pub faults_read: bool,
    pub faults_cleared: bool,
    pub fault_open: Option<String>,         // expanded fault code
    pub faults: Option<ediabas::JobResult>, // real FS_LESEN result (DDE)
    pub faults_busy: bool,                  // an FS_LESEN/FS_LOESCHEN is in flight

    // Diagnostic worker (spun up on CONNECT). None until screen 2 connects.
    pub worker: Option<Worker>,
    pub status_msg: String,
    // Real connection / live data (screen 3, DDE via MW_SELECT_LESEN_NORM)
    pub connected: Option<&'static str>,   // module code we've INITIALISIERUNG'd
    pub connect_pending: bool,             // a Connect is in flight right now
    pub connect_attempted: bool,           // tried once this session (no retry spam)
    pub streaming: bool,                   // a StartStream is active
    pub live: Option<ediabas::JobResult>,  // latest polled measurement set
    pub meas_groups: Vec<crate::session_cfg::MeasGroup>, // .ipo-curated live groups (built on connect)
    pub stream_poll: Option<(Vec<(String, String)>, u64)>, // active per-page poll: (reqs, interval_ms)

    // Window
    pub fullscreen: bool,
    startup_frames: u8,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = Theme::Dark;
        install_fonts(&cc.egui_ctx);
        // Nudge everything up ~6% — a touch larger and easier to read, keeping
        // all the hand-tuned pixel layouts in proportion (scales on top of DPI).
        cc.egui_ctx.set_zoom_factor(1.06);
        cc.egui_ctx.set_visuals(theme::visuals(theme));

        // Default selection: E46 (the reference DDE chassis), first body.
        let e46 = DATA.iter().position(|c| c.code == "E46");

        Self {
            theme,
            lang: Lang::Ru,
            screen: Screen::Chassis,
            series: '3',
            chassis: e46,
            body: Some(0),
            query: String::new(),
            ecu_cat: Some(Category::Pwr),
            ecu_sel: None,
            scan_pct: 0.0,
            connect_error: None,
            ses_stack: vec!["main".into()],
            faults_read: false,
            faults_cleared: false,
            fault_open: None,
            faults: None,
            faults_busy: false,
            worker: None,
            status_msg: String::new(),
            connected: None,
            connect_pending: false,
            connect_attempted: false,
            streaming: false,
            live: None,
            meas_groups: Vec::new(),
            stream_poll: None,
            fullscreen: true,
            startup_frames: 3,
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
            Screen::Session => {
                if self.ses_stack.len() > 1 {
                    self.ses_stack.pop();
                } else {
                    self.screen = Screen::Ecu;
                }
            }
            Screen::Ecu => self.screen = Screen::Chassis,
            Screen::Chassis => {}
        }
    }

    /// Reset the per-session view state (page stack + cached results) for a fresh module.
    fn reset_session_view(&mut self) {
        self.ses_stack = vec!["main".into()];
        self.faults_read = false;
        self.faults_cleared = false;
        self.fault_open = None;
        self.faults = None;
        self.faults_busy = false;
        self.streaming = false;
        self.live = None;
        self.meas_groups.clear();
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
    pub fn start_connect(&mut self, ctx: &egui::Context) {
        self.reset_session_view();
        self.reset_connection();
        let module = self.ecu_sel.and_then(|code| {
            self.selected_chassis()
                .map(crate::ecu::mods_for)
                .and_then(|v| v.into_iter().find(|m| m.code == code))
        });
        match module.and_then(|m| m.prg) {
            Some(prg) => {
                let w = self.worker.get_or_insert_with(|| Worker::spawn(ctx.clone()));
                w.send(Cmd::Connect { port: "COM3".into(), baud: 9600, prg: prg.to_string() });
                self.connect_attempted = true;
                self.connect_pending = true;
                // Stay on Screen::Ecu — the session opens on the Connected event.
            }
            None => {
                // No SGBD on disk → nothing to open. Surface it honestly.
                self.connect_error = Some(crate::lang::dict(self.lang).no_sgbd.to_string());
            }
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
            Screen::Session => screens::session::show(self, ctx),
        }
    }
}

/// UTC HH:MM:SS for the status-bar clock (local-tz formatting would need a dep).
pub fn clock() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}")
}
