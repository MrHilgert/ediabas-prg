//! eDIAG — modern INPA-style BMW diagnostic GUI on top of the `ediabas` crate.
//!
//! Foundation: workspace `gui` crate, eframe app, design-token theming (dark/light),
//! RU/EN, ported chassis catalog, INPA-style ECU model, and a worker-thread bridge
//! to `ediabas::Session`. Screen 1 (Chassis Select) is UI-only; screen 2 (ECU Select)
//! is where CONNECT brings the transport online.

// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod data;
mod ecu;
mod lang;
mod screens;
mod session_cfg;
mod theme;
mod worker;

use app::App;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        // Start maximized (a real, sized window); the app forces true fullscreen at
        // runtime as an actual state transition — the builder `with_fullscreen` flag
        // is unreliable (borderless without a monitor-sized resize on some setups).
        viewport: egui::ViewportBuilder::default()
            .with_title("eDIAG")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 640.0])
            .with_maximized(true),
        ..Default::default()
    };
    eframe::run_native("eDIAG", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
