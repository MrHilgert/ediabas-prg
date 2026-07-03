//! `inpa` — a headless parser for BMW INPA `.ipo` compiled screen scripts.
//!
//! An `.ipo` is the compiled form of a C-dialect INPA screen program. This crate decodes
//! its container + bytecode and (in later layers) extracts a typed [`model::ScreenModule`]
//! describing an ECU's pages, navigation, and result bindings — deterministically, as a
//! pure function of the file bytes. No I/O beyond an optional path helper; no egui.
//!
//! Layers: [`record`] (container) → [`opcode`] (instructions) → `extract` → [`model`].

pub mod extract;
pub mod ids;
pub mod model;
pub mod opcode;
pub mod record;
pub mod strings;

pub use model::{
    InfoLine, InfoValue, NavItem, NavTarget, Node, Row, Screen, ScreenKind, ScreenModule,
};
pub use record::{parse_module, Body, Const, Module, Record};

use std::fmt;
use std::path::Path;

/// Errors from decoding an `.ipo` file.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// File does not start with the `05 00` fingerprint.
    NoFingerprint,
    /// A `0x0a`-terminated string ran to EOF without its terminator.
    UnterminatedString(usize),
    /// A record header was cut off by EOF.
    Truncated(usize),
    /// A payload (globals / consts / code) extended past EOF.
    Overrun(&'static str),
    /// A constant-pool entry used an unknown type tag.
    UnknownConstTag(u8, usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::NoFingerprint => write!(f, "not an .ipo (no 05 00 fingerprint)"),
            Error::UnterminatedString(at) => write!(f, "unterminated string at {at:#x}"),
            Error::Truncated(at) => write!(f, "truncated record header at {at:#x}"),
            Error::Overrun(what) => write!(f, "{what} payload overruns end of file"),
            Error::UnknownConstTag(t, at) => write!(f, "unknown const tag {t:#04x} at {at:#x}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Read and parse an `.ipo` file from disk into its low-level [`Module`].
pub fn read_module(path: impl AsRef<Path>) -> Result<Module, Error> {
    let data = std::fs::read(path)?;
    parse_module(&data)
}

/// Parse an `.ipo` file into a typed [`ScreenModule`] (the GUI-facing model).
/// The script name is taken from the file stem.
pub fn parse(path: impl AsRef<Path>) -> Result<ScreenModule, Error> {
    let path = path.as_ref();
    let data = std::fs::read(path)?;
    let module = parse_module(&data)?;
    let script = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(extract::build(&module, script))
}

/// Parse `.ipo` bytes into a [`ScreenModule`] with an explicit script name.
pub fn parse_bytes(data: &[u8], script: impl Into<String>) -> Result<ScreenModule, Error> {
    let module = parse_module(data)?;
    Ok(extract::build(&module, script.into()))
}
