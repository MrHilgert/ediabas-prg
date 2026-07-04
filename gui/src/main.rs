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
mod link;
mod model;
mod ui;

use app::App;

/// The `ediabas` protocol trace is OPT-IN via the `EDIABAS_TRACE` env var (same as
/// the CLI): unset → off. `EDIABAS_TRACE=1` → `ediabas-trace.log` in the cwd; `=2` →
/// per-opcode firehose; `=<path>` → a custom file. We don't force it on — just print
/// where the log goes when the user did enable it, so a failed connect is easy to find.
fn init_trace() {
    match std::env::var("EDIABAS_TRACE") {
        Ok(v) if !v.is_empty() => {
            let where_ = match v.as_str() {
                "1" | "true" | "on" | "2" | "verbose" | "all" => std::env::current_dir()
                    .map(|d| d.join("ediabas-trace.log").display().to_string())
                    .unwrap_or_else(|_| "ediabas-trace.log".to_string()),
                path => path.to_string(),
            };
            eprintln!("eDIAG: протокольный трейс включён (EDIABAS_TRACE={v}) → {where_}");
        }
        _ => {}
    }
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
