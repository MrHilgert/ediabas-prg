//! Optional diagnostic trace log, controlled by the `EDIABAS_TRACE` env var.
//!
//! * unset / empty → off; every [`trace!`]/[`vtrace!`] is a cheap no-op and stdout
//!   stays clean (callers print only job results).
//! * `EDIABAS_TRACE=1` (or `true`) → **level 1**: protocol/job trace (TX/RX telegrams,
//!   protocol config, table lookups, warnings) to `ediabas-trace.log`.
//! * `EDIABAS_TRACE=2` (or `verbose`) → **level 2**: level 1 *plus* the per-instruction
//!   firehose (every opcode, register/stack/byte dumps).
//! * `EDIABAS_TRACE=<path>` → level 1 to that file instead of `ediabas-trace.log`.
//!
//! Use [`trace!`] for level-1 messages and [`vtrace!`] for the noisy per-opcode ones.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Mutex, OnceLock};

type Writer = Mutex<BufWriter<File>>;

static STATE: OnceLock<(u8, Option<Writer>)> = OnceLock::new();

fn state() -> &'static (u8, Option<Writer>) {
    STATE.get_or_init(|| {
        let (level, path) = match std::env::var("EDIABAS_TRACE") {
            Ok(v) if !v.is_empty() => {
                let low = v.to_ascii_lowercase();
                match low.as_str() {
                    "1" | "true" | "on" => (1u8, None),
                    "2" | "verbose" | "all" => (2u8, None),
                    _ => (1u8, Some(v)), // treat the value as a log-file path
                }
            }
            _ => (0u8, None),
        };
        if level == 0 {
            return (0, None);
        }
        let file = path.unwrap_or_else(|| "ediabas-trace.log".to_string());
        match File::create(&file) {
            Ok(f) => (level, Some(Mutex::new(BufWriter::new(f)))),
            Err(_) => (0, None),
        }
    })
}

/// Active trace level: 0 = off, 1 = protocol/job, 2 = per-instruction. Cheap.
#[inline]
pub fn level() -> u8 {
    state().0
}

/// Level ≥ 1 (protocol/job trace active).
#[inline]
pub fn enabled() -> bool {
    level() >= 1
}

/// Level ≥ 2 (per-instruction / verbose trace active).
#[inline]
pub fn verbose() -> bool {
    level() >= 2
}

/// Write one line to the trace log. Flushes immediately so the log survives an
/// aborted / killed run.
pub fn line(args: std::fmt::Arguments<'_>) {
    if let Some(lock) = &state().1 {
        if let Ok(mut w) = lock.lock() {
            let _ = writeln!(w, "{args}");
            let _ = w.flush();
        }
    }
}

/// Log a level-1 (protocol/job) line when `EDIABAS_TRACE` is set. Usage mirrors
/// `println!`: `trace!("TX: {telegram}")`.
macro_rules! trace {
    ($($arg:tt)*) => {
        if $crate::trace::enabled() {
            $crate::trace::line(format_args!($($arg)*));
        }
    };
}

/// Log a level-2 (verbose / per-instruction) line — only when `EDIABAS_TRACE=2`.
macro_rules! vtrace {
    ($($arg:tt)*) => {
        if $crate::trace::verbose() {
            $crate::trace::line(format_args!($($arg)*));
        }
    };
}

pub(crate) use {trace, vtrace};
