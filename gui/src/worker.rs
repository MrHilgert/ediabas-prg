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

use ediabas::{JobResult, Measurement, Session};

/// UI → worker.
pub enum Cmd {
    Connect { port: String, baud: u32, prg: String },
    RunJob { job: String, args: String },
    /// Poll a LIST of `(job, args)` requests every `interval_ms`, merge their results
    /// into one `JobResult`, and stream it back as a `JobDone` event. Mirrors INPA's
    /// per-screen mix of batched `MW_SELECT_LESEN_NORM` and individual `STATUS_*` jobs.
    StartStream { reqs: Vec<(String, String)>, interval_ms: u64 },
    StopStream,
    Disconnect,
    Shutdown,
}

/// Worker → UI.
pub enum Event {
    Connected { jobs: Vec<String>, measurements: Vec<Measurement> },
    JobDone { job: String, result: JobResult },
    /// A streaming poll produced no data (transient bus glitch). NOT fatal — the UI holds
    /// the last values and the link, dropping only after several consecutive misses.
    PollMiss(String),
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
    // (reqs, interval) while streaming — reqs are (job, args) run+merged per tick.
    let mut stream: Option<(Vec<(String, String)>, Duration)> = None;

    loop {
        let wait = stream
            .as_ref()
            .map(|(_, d)| *d)
            .unwrap_or(Duration::from_secs(3600));

        match cmd_rx.recv_timeout(wait) {
            Ok(cmd) => match cmd {
                Cmd::Connect { port, baud, prg } => {
                    stream = None;
                    session = None; // drop any prior session so it releases the port
                    match connect(&port, baud, &prg) {
                        Ok(s) => {
                            let jobs = s.jobs();
                            let measurements = s.measurements();
                            session = Some(s);
                            let _ = evt_tx.send(Event::Connected { jobs, measurements });
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
                Cmd::StartStream { reqs, interval_ms } => {
                    stream = Some((reqs, Duration::from_millis(interval_ms.max(20))));
                    // Fire an immediate first poll so the page fills without waiting a tick.
                    if let Some((reqs, _)) = stream.clone() {
                        let _ = evt_tx.send(poll_reqs(session.as_mut(), &reqs));
                        ctx.request_repaint();
                    }
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
                // Streaming tick: run every request of the current screen and merge.
                if let Some((reqs, _)) = stream.clone() {
                    let _ = evt_tx.send(poll_reqs(session.as_mut(), &reqs));
                    ctx.request_repaint();
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Run every `(job, args)` request in order and merge their result sets into one
/// `JobResult` (INPA polls a screen's params via a mix of MW_SELECT and STATUS_*).
/// Errors on individual requests are skipped; only an all-failed poll surfaces an error.
fn poll_reqs(session: Option<&mut Session>, reqs: &[(String, String)]) -> Event {
    let Some(s) = session else { return Event::Error("not connected".into()) };
    let mut merged: Option<JobResult> = None;
    let mut last_err: Option<String> = None;
    for (job, args) in reqs {
        match s.run_job(job, args) {
            Ok(r) => match &mut merged {
                Some(acc) => acc.extend(r),
                None => merged = Some(r),
            },
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    match merged {
        Some(result) => Event::JobDone { job: "STREAM".to_string(), result },
        // A streaming poll that yielded nothing is a transient miss, not a disconnect.
        None => Event::PollMiss(last_err.unwrap_or_else(|| "no data".into())),
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
    // DS2 INITIALISIERUNG only configures comm parameters locally (xsetpar/xawlen)
    // — it never sends a telegram, so it "succeeds" even with nothing on the bus.
    // Prove the ECU is actually there by running a real identification job: it
    // sends a request and a transport timeout ⇒ module absent / no link.
    probe_presence(&mut s)?;
    Ok(s)
}

/// Liveness probe: run the first available identification job and require the ECU
/// to answer for real. `Ok(())` only if a telegram round-trip actually succeeded;
/// a timeout (no response) or an error status means the module isn't there.
fn probe_presence(s: &mut Session) -> Result<(), String> {
    // Standard BMW SGBD identification jobs, in preference order.
    let job = ["IDENT", "IDENTIFIKATION", "INFO"]
        .into_iter()
        .find(|&j| s.has_job(j));
    let Some(job) = job else {
        // No known ident job to probe with — can't cheaply verify; accept the init.
        return Ok(());
    };
    match s.run_job(job, "") {
        Ok(r) => {
            // Present if the SGBD reports OKAY, or (no JOB_STATUS) returned data.
            let present = r
                .job_status()
                .map(|st| st.to_ascii_uppercase().contains("OKAY"))
                .unwrap_or_else(|| !r.is_empty());
            if present {
                Ok(())
            } else {
                Err(format!(
                    "ЭБУ не отвечает ({job}: {})",
                    r.job_status().unwrap_or_else(|| "нет данных".into())
                ))
            }
        }
        Err(e) => Err(format!("ЭБУ не отвечает: {e}")),
    }
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
