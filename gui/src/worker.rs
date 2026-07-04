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

use ediabas::{Error, JobResult, Measurement, Session};

/// UI → worker.
pub enum Cmd {
    /// Open a session. `prg` is the fallback SGBD (from the `.ipo`). `groups` are the
    /// address-keyed EDIABAS group files the `.ipo` referenced (`D_<ADDR>`); when
    /// present, the concrete SGBD is resolved by running each group's IDENTIFIKATION →
    /// VARIANTE (INPA variant identification), in order, until one identifies. `prg` is
    /// used if no group identifies or none are present.
    Connect { port: String, baud: u32, prg: String, groups: Vec<String> },
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
    /// A CONNECT attempt failed, classified so the UI can tell "no adapter" (the serial
    /// port / tty isn't there) from a real ECU timeout (port opened, module silent).
    ConnectFailed(ConnectError),
    Disconnected,
}

/// Why a connect attempt failed. Distinguishes an adapter/port problem (nothing plugged
/// in, wrong port, busy) from the ECU never answering on an open port.
pub enum ConnectError {
    /// Serial port / tty is unavailable — no adapter, wrong port, or it vanished.
    NoPort,
    /// Port exists but can't be opened (in use by another app, permission denied).
    PortBusy,
    /// Port opened fine, but the ECU never answered a real request (diagnostic timeout
    /// or the module simply isn't on the bus).
    NoResponse,
    /// Anything else (unexpected I/O, parse error) — carries the raw detail for the log.
    Other(String),
}

/// Classify a `Session::open` / `initialize` failure into a [`ConnectError`]. Port-open
/// failures surface as `io::Error`; a bus timeout is `Error::Timeout`.
fn classify_open(e: Error) -> ConnectError {
    use std::io::ErrorKind;
    match e {
        Error::Io(io) => match io.kind() {
            ErrorKind::PermissionDenied | ErrorKind::AddrInUse => ConnectError::PortBusy,
            // NotFound / NotConnected / BrokenPipe / and most serialport open failures on a
            // missing tty land here — treat as "no adapter / port unavailable".
            _ => ConnectError::NoPort,
        },
        Error::Timeout => ConnectError::NoResponse,
        other => ConnectError::Other(other.to_string()),
    }
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
                Cmd::Connect { port, baud, prg, groups } => {
                    stream = None;
                    session = None; // drop any prior session so it releases the port
                    match connect(&port, baud, &prg, &groups) {
                        Ok(s) => {
                            let jobs = s.jobs();
                            let measurements = s.measurements();
                            session = Some(s);
                            let _ = evt_tx.send(Event::Connected { jobs, measurements });
                        }
                        Err(e) => {
                            session = None;
                            let _ = evt_tx.send(Event::ConnectFailed(e));
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

fn connect(port: &str, baud: u32, prg: &str, groups: &[String]) -> Result<Session, ConnectError> {
    // Phase 1 (INPA variant identification): if the .ipo referenced EDIABAS group
    // files, run each group's IDENTIFIKATION to learn the concrete variant SGBD;
    // otherwise fall back to `prg` (the .ipo's own SGBD).
    let target = resolve_variant(port, baud, groups, prg);

    // Phase 2: open the concrete SGBD for the real session. `Session::open` /
    // `initialize` fail with an I/O error when the port isn't there → NoPort/PortBusy.
    let prg_path = resolve_prg(&target);
    let mut s = Session::open(port, baud, &prg_path).map_err(classify_open)?;
    s.initialize().map_err(classify_open)?;
    // DS2 INITIALISIERUNG only configures comm parameters locally (xsetpar/xawlen)
    // — it never sends a telegram, so it "succeeds" even with nothing on the bus.
    // Prove the ECU is actually there by running a real identification job: it sends a
    // request, and a transport timeout here (port is open) ⇒ module absent / no answer.
    s.probe_presence().map_err(|_| ConnectError::NoResponse)?;
    Ok(s)
}

/// INPA variant identification (phase 1). Try each address-keyed group file the
/// `.ipo` referenced (`D_<ADDR>` → `ecu/D_<ADDR>.GRP`) in order: open it, run its
/// `IDENTIFIKATION` job and, on the first that reports a variant, return
/// `<VARIANTE>.prg` — the concrete SGBD the ECU is. A group whose file is missing, or
/// that doesn't answer (wrong diagnostic address for this car — e.g. `D_000D` on an
/// E39 whose cluster lives at `D_0080`), is skipped. Falls back to `fallback_prg`
/// (the `.ipo`'s own SGBD) when nothing identifies. Each group session is dropped
/// before the next attempt so it releases the serial port.
fn resolve_variant(port: &str, baud: u32, groups: &[String], fallback_prg: &str) -> String {
    for g in groups {
        let grp_path = resolve_prg(&format!("{g}.GRP"));
        if !grp_path.exists() {
            continue;
        }
        let identified = Session::open(port, baud, &grp_path).and_then(|mut s| {
            s.initialize()?;
            s.identify_variant()
        });
        if let Ok(Some(variant)) = identified {
            return format!("{variant}.prg");
        }
    }
    fallback_prg.to_string()
}

/// Locate `ecu/<name>` regardless of the current working directory and filename
/// case: walks up from cwd and the exe dir (dev builds live under `target/…`, with
/// `ecu/` at the workspace root) and matches the filename case-insensitively, so
/// `KOMBI31.prg`/`D_0080.GRP` resolve on a case-sensitive (Linux) filesystem even
/// when the catalog/variant name differs in case. Falls back to the plain relative
/// path so `Session::open` still reports a clear error when the file is truly absent.
fn resolve_prg(name: &str) -> PathBuf {
    crate::i18n::resolve_ci("ecu", name).unwrap_or_else(|| Path::new("ecu").join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error as IoError, ErrorKind};

    #[test]
    fn classify_open_splits_port_from_ecu() {
        // Missing/unavailable adapter → NoPort.
        assert!(matches!(classify_open(Error::Io(IoError::from(ErrorKind::NotFound))), ConnectError::NoPort));
        // Busy / access-denied port → PortBusy.
        assert!(matches!(classify_open(Error::Io(IoError::from(ErrorKind::PermissionDenied))), ConnectError::PortBusy));
        assert!(matches!(classify_open(Error::Io(IoError::from(ErrorKind::AddrInUse))), ConnectError::PortBusy));
        // Real ECU timeout (port opened, no answer) → NoResponse.
        assert!(matches!(classify_open(Error::Timeout), ConnectError::NoResponse));
        // Anything else → Other.
        assert!(matches!(classify_open(Error::Vm("x".into())), ConnectError::Other(_)));
    }
}
