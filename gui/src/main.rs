//! eDIAG — modern INPA-style BMW diagnostic GUI on top of the `ediabas` crate.
//!
//! Foundation: workspace `gui` crate, eframe app, design-token theming (dark/light),
//! RU/EN, ported chassis catalog, INPA-style ECU model, and a worker-thread bridge
//! to `ediabas::Session`. Screen 1 (Chassis Select) is UI-only; screen 2 (ECU Select)
//! is where CONNECT brings the transport online.

// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod catalog;
mod config;
mod data;
mod ecu;
mod i18n;
mod lang;
mod screens;
mod session;
mod theme;
mod worker;

use app::App;

/// Turn the `ediabas` protocol trace ON for the GUI so a failed connect leaves a log
/// to inspect (TX/RX telegrams, protocol config, table lookups). Writes to
/// `ediabas-trace.log` in the working directory — same file the CLI uses. An explicit
/// `EDIABAS_TRACE` from the environment (e.g. `=2` for the per-opcode firehose, or a
/// custom path) is respected and left untouched.
fn init_trace() {
    if std::env::var_os("EDIABAS_TRACE").is_some() {
        return;
    }
    std::env::set_var("EDIABAS_TRACE", "1"); // → ediabas-trace.log in the cwd
    let where_ = std::env::current_dir()
        .map(|d| d.join("ediabas-trace.log").display().to_string())
        .unwrap_or_else(|_| "ediabas-trace.log".to_string());
    eprintln!("eDIAG: протокольный трейс включён → {where_}");
}

fn main() -> eframe::Result<()> {
    init_trace();
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
