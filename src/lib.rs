//! `ediabas` — a native Rust EDIABAS BEST/2 interpreter with BMW DS2 / K-line
//! transport. No Wine, no Windows EDIABAS runtime.
//!
//! The high-level entry point is [`Session`]:
//!
//! ```no_run
//! use ediabas::Session;
//!
//! let mut s = Session::open("COM3", 9600, "ecu/DDE40KW0.prg")?;
//! s.initialize()?;                                       // run INITIALISIERUNG
//! let res = s.run_job("MW_SELECT_LESEN_NORM", "0F30")?;  // structured result
//!
//! println!("status: {:?}", res.job_status());
//! if let Some(v) = res.get_f64("STAT_armM_List_WERT") {
//!     println!("value = {v}");
//! }
//! for set in res.sets() {
//!     for (name, val) in set.iter() {
//!         println!("{name} = {val}");
//!     }
//! }
//! # Ok::<(), ediabas::Error>(())
//! ```
//!
//! Lower-level building blocks are also public for advanced use and for the CLI
//! binary: [`prg`] (parse `.prg` files), [`vm`] (the BEST/2 VM), [`transport`]
//! (DS2 / SIM), [`driver`] (serial / mock).

pub mod api;
pub mod config;
pub mod driver;
pub mod error;
pub mod prg;
pub mod transport;
pub mod vm;

mod trace;

pub use api::{JobResult, ResultSet, Session};
pub use driver::available_ports;
pub use error::{Error, Result};
pub use prg::{Measurement, PrgFile};
pub use vm::Value;
