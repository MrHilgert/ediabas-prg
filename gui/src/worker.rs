//! Worker thread that owns the blocking `ediabas::Session` and talks to the UI
//! over two mpsc channels. The UI thread NEVER blocks on serial I/O.
//!
//! Screen 1 (Chassis Select) is UI-only and doesn't touch this. Screen 2 spins a
//! worker up on "CONNECT"; screen 3 streams a measurement job (MW_SELECT_LESEN_NORM)
//! for live data. Events are drained per frame.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use ediabas::{JobResult, Session};

/// UI → worker.
pub enum Cmd {
    Connect { port: String, baud: u32, prg: String },
    RunJob { job: String, args: String },
    /// Poll `job(args)` every `interval_ms` and stream back `JobDone` events.
    StartStream { job: String, args: String, interval_ms: u64 },
    StopStream,
    Disconnect,
    Shutdown,
}

/// Worker → UI.
pub enum Event {
    Connected { jobs: Vec<String> },
    JobDone { job: String, result: JobResult },
    Error(String),
    Disconnected,
}

/// Handle held by the UI: send `Cmd`s, drain `Event`s (non-blocking).
pub struct Worker {
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
    _handle: thread::JoinHandle<()>,
}

impl Worker {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (cmd_tx, cmd_rx) = channel::<Cmd>();
        let (evt_tx, evt_rx) = channel::<Event>();
        let handle = thread::spawn(move || run(cmd_rx, evt_tx, ctx));
        Worker { tx: cmd_tx, rx: evt_rx, _handle: handle }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    /// Drain all pending events without blocking.
    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

fn run(cmd_rx: Receiver<Cmd>, evt_tx: Sender<Event>, ctx: egui::Context) {
    let mut session: Option<Session> = None;
    // (job, args, interval) while streaming.
    let mut stream: Option<(String, String, Duration)> = None;

    loop {
        let wait = stream
            .as_ref()
            .map(|(_, _, d)| *d)
            .unwrap_or(Duration::from_secs(3600));

        match cmd_rx.recv_timeout(wait) {
            Ok(cmd) => match cmd {
                Cmd::Connect { port, baud, prg } => {
                    stream = None;
                    match connect(&port, baud, &prg) {
                        Ok(s) => {
                            let jobs = s.jobs();
                            session = Some(s);
                            let _ = evt_tx.send(Event::Connected { jobs });
                        }
                        Err(e) => {
                            session = None;
                            let _ = evt_tx.send(Event::Error(e));
                        }
                    }
                    ctx.request_repaint();
                }
                Cmd::RunJob { job, args } => {
                    let evt = run_once(session.as_mut(), &job, &args);
                    let _ = evt_tx.send(evt);
                    ctx.request_repaint();
                }
                Cmd::StartStream { job, args, interval_ms } => {
                    stream = Some((job, args, Duration::from_millis(interval_ms.max(20))));
                }
                Cmd::StopStream => stream = None,
                Cmd::Disconnect => {
                    session = None;
                    stream = None;
                    let _ = evt_tx.send(Event::Disconnected);
                    ctx.request_repaint();
                }
                Cmd::Shutdown => break,
            },
            Err(RecvTimeoutError::Timeout) => {
                // Streaming tick: run one poll of the measurement job.
                if let Some((job, args, _)) = stream.clone() {
                    let evt = run_once(session.as_mut(), &job, &args);
                    let _ = evt_tx.send(evt);
                    ctx.request_repaint();
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn run_once(session: Option<&mut Session>, job: &str, args: &str) -> Event {
    match session {
        Some(s) => match s.run_job(job, args) {
            Ok(r) => Event::JobDone { job: job.to_string(), result: r },
            Err(e) => Event::Error(e.to_string()),
        },
        None => Event::Error("not connected".into()),
    }
}

fn connect(port: &str, baud: u32, prg: &str) -> Result<Session, String> {
    let prg_path = resolve_prg(prg);
    let mut s = Session::open(port, baud, &prg_path).map_err(|e| e.to_string())?;
    s.initialize().map_err(|e| e.to_string())?;
    Ok(s)
}

/// Locate `ecu/<name>` regardless of the current working directory: try cwd,
/// then walk up from both the cwd and the executable's directory (dev builds
/// live under `target/…`, with `ecu/` at the workspace root).
fn resolve_prg(name: &str) -> PathBuf {
    let rel = Path::new("ecu").join(name);
    if rel.exists() {
        return rel;
    }
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            bases.push(dir.to_path_buf());
        }
    }
    for base in bases {
        let mut dir = Some(base.as_path());
        while let Some(d) = dir {
            let cand = d.join("ecu").join(name);
            if cand.exists() {
                return cand;
            }
            dir = d.parent();
        }
    }
    rel // fall back to the relative path (Session::open will report a clear error)
}
