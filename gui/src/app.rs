//! Application state + eframe entry. Holds the selection/theme/lang state shared
//! across screens, applies theme visuals each frame, and routes to the active screen.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::data::{self, DATA};
use crate::ecu::Category;
use crate::lang::Lang;
use crate::screens;
use crate::theme::{self, Theme};
use crate::worker::Worker;

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
    pub ecu_cat: Option<Category>,          // None = All
    pub ecu_sel: Option<&'static str>,      // selected module code
    pub scan_pct: f32,                       // 0..100 module-scan animation

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

    // Window
    pub fullscreen: bool,
    startup_frames: u8,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = Theme::Dark;
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
            ecu_cat: None,
            ecu_sel: None,
            scan_pct: 0.0,
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

    /// Enter the config-driven session for the selected module.
    pub fn enter_session(&mut self) {
        self.screen = Screen::Session;
        self.ses_stack = vec!["main".into()];
        self.faults_read = false;
        self.faults_cleared = false;
        self.fault_open = None;
        self.faults = None;
        self.faults_busy = false;
        // Fresh connection state for this module (a real one will INITIALISIERUNG).
        self.connected = None;
        self.connect_pending = false;
        self.connect_attempted = false;
        self.streaming = false;
        self.live = None;
        self.status_msg.clear();
    }

    /// Enter the ECU-select screen for the current chassis and kick off the scan.
    pub fn enter_ecu(&mut self) {
        self.screen = Screen::Ecu;
        self.ecu_cat = None;
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
