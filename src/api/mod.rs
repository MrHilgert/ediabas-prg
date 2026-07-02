//! High-level diagnostic API — the surface a GUI or another binary uses.
//!
//! [`Session`] owns an open K-line adapter, a loaded ECU description (`.prg`) and
//! the BEST/2 VM. Typical flow: [`Session::open`] → [`Session::initialize`] →
//! [`Session::run_job`], reading a structured [`JobResult`] back.

mod result;

pub use result::{JobResult, ResultSet};

use std::path::Path;

use crate::driver::serial::SerialDriver;
use crate::error::{Error, Result};
use crate::prg::PrgFile;
use crate::transport::ds2::Ds2Transport;
use crate::vm::Vm;

/// A live diagnostic session against one ECU over a serial K-line adapter.
///
/// Create with [`Session::open`], call [`Session::initialize`] once, then run any
/// number of jobs with [`Session::run_job`].
pub struct Session {
    prg: PrgFile,
    vm: Vm,
}

impl Session {
    /// Open serial port `port` at `baud` and load the ECU's `.prg` from `prg_path`.
    ///
    /// This does not talk to the ECU yet — call [`Session::initialize`] next. The
    /// DS2 transport is configured for KDCAN/FTDI adapters (TX echo is drained).
    pub fn open(port: &str, baud: u32, prg_path: impl AsRef<Path>) -> Result<Self> {
        let prg = PrgFile::open(prg_path.as_ref()).map_err(|e| Error::Prg(e.to_string()))?;
        let tables = prg.parse_tables();

        let serial = SerialDriver::open_parity(port, baud, serialport::Parity::Even)?;
        let mut ds2 = Ds2Transport::new(Box::new(serial));
        // KDCAN/FTDI K-line adapters mirror TX back on RX — always drain the echo.
        ds2.echo = true;
        // Sensible pre-init inter-byte spacing; INITIALISIERUNG overrides it with
        // the value from the .prg (ParInterbyteTime).
        ds2.interbyte_ms = 5;

        let vm = Vm::new(Box::new(ds2), tables);
        Ok(Session { prg, vm })
    }

    /// Run the ECU's `INITIALISIERUNG` job to establish the protocol parameters
    /// (concept, baud, frame-length layout, inter-byte timing). Present on nearly
    /// every BMW ECU; call once after [`Session::open`] before any data job.
    pub fn initialize(&mut self) -> Result<JobResult> {
        self.exec("INITIALISIERUNG", &[])
    }

    /// Run diagnostic job `job` with an optional text argument string
    /// (EDIABAS ArgString). For jobs that take several selectors in one telegram
    /// (e.g. `MW_SELECT_LESEN_NORM`), concatenate them: `"0F300F65"`.
    ///
    /// Pass `""` for jobs that take no arguments.
    pub fn run_job(&mut self, job: &str, args: &str) -> Result<JobResult> {
        self.exec(job, args.as_bytes())
    }

    /// Run job `job` with raw binary arguments (EDIABAS `apiJobData` semantics).
    pub fn run_job_data(&mut self, job: &str, args: &[u8]) -> Result<JobResult> {
        self.exec(job, args)
    }

    fn exec(&mut self, job: &str, args: &[u8]) -> Result<JobResult> {
        let code = self
            .prg
            .job_code(job)
            .ok_or_else(|| Error::Vm(format!("job '{job}' not found in .prg")))?;
        self.vm.set_args(args.to_vec());
        let sets = self.vm.run_job(&code).map_err(Error::Vm)?;
        Ok(JobResult::new(
            sets.into_iter().map(ResultSet::from_map).collect(),
        ))
    }

    /// Names of all jobs defined in the loaded `.prg`.
    pub fn jobs(&self) -> Vec<String> {
        self.prg.jobs().into_iter().map(|j| j.name).collect()
    }

    /// Measurable channels (mnemonic → 2C10 selector, unit, description) from the
    /// SGBD's measurement table. Use these to batch up to 10 selectors into one
    /// `MW_SELECT_LESEN_NORM` request instead of one `STATUS_*` job per parameter.
    pub fn measurements(&self) -> Vec<crate::prg::Measurement> {
        self.prg.measurements()
    }

    /// Whether the `.prg` defines a job with this name.
    pub fn has_job(&self, name: &str) -> bool {
        self.prg.job_code(name).is_some()
    }
}
