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
use crate::trace::trace;
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
    /// Open adapter on `port` and load the ECU's `.prg` from `prg_path`.
    ///
    /// The transport is chosen **deterministically from the `.prg` itself**: we
    /// statically read the ECU's communication concept from its `INITIALISIERUNG`
    /// job ([`PrgFile::initial_comm_config`]) and build the matching transport
    /// (DS2 / KWP2000 / BMW-FAST / KWP1281 / D-CAN) at the concept's own baud and
    /// parity. `baud` is only a fallback used when the `.prg` declares none.
    ///
    /// This does not talk to the ECU yet — call [`Session::initialize`] next.
    pub fn open(port: &str, baud: u32, prg_path: impl AsRef<Path>) -> Result<Self> {
        trace!("═══ OPEN {} @ {port} ═══", prg_path.as_ref().display());
        let prg = PrgFile::open(prg_path.as_ref()).map_err(|e| Error::Prg(e.to_string()))?;
        let tables = prg.parse_tables();

        // Static concept probe → pick transport + serial params before any I/O.
        let mut cfg = prg.initial_comm_config().unwrap_or_default();
        if cfg.baud == 0 {
            cfg.baud = baud;
        }
        if !crate::config::Protocol::is_recognized(cfg.concept) {
            trace!(
                "[open] ECU declares unrecognized concept {:#06x} — using DS2 framing as best effort",
                cfg.concept
            );
        }

        // K-line families open the serial at the concept's baud/parity. D-CAN reaches
        // the CAN bus through the adapter's own control link (the D-CAN transport sets
        // the CAN bitrate itself), so open the FTDI at its standard control baud.
        let (serial_baud, parity) = if cfg.protocol.is_can() {
            (115_200, serialport::Parity::None)
        } else {
            (cfg.baud, cfg.parity)
        };
        let serial = SerialDriver::open_parity(port, serial_baud, parity)?;

        let transport = crate::transport::build(Box::new(serial), &cfg);
        let vm = Vm::new(transport, tables, cfg);
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
        trace!("── JOB {job} ──");
        let code = self
            .prg
            .job_code(job)
            .ok_or_else(|| Error::Vm(format!("job '{job}' not found in .prg")))?;
        self.vm.set_args(args.to_vec());
        let sets = self.vm.run_job(&code).map_err(Error::Vm)?;
        // Presence gates on the ECU's OWN wake address answering, not merely "something
        // on the bus answered": a group IDENTIFIKATION probes several addresses, and a
        // reply from a different module must not count as this ECU being present. If the
        // CommParameter declared no wake address, fall back to "any address answered".
        let comm_ok = match self.vm.wake_addr() {
            Some(addr) => self.vm.responded_from(addr),
            None => self.vm.got_response(),
        };
        Ok(JobResult::new(
            sets.into_iter().map(ResultSet::from_map).collect(),
        )
        .with_comm(comm_ok))
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

    /// EDIABAS **variant identification**: when this session was opened on an
    /// address-keyed *group* file (`Gruppendatei`, `ecu/D_<ADDR>.GRP` — the `.ipo`
    /// names it, e.g. `KOMBI.ipo` → `D_0080`), run its identification job and return
    /// the concrete variant SGBD name from the `VARIANTE` result (e.g. `IKE`,
    /// `LCM_III`). The caller then re-opens on `<VARIANTE>.prg` for the actual session.
    ///
    /// This is exactly how INPA/EDIABAS pick the right SGBD: the group's ident job
    /// reads the ECU and reports which variant it is. The BMW group files name the job
    /// `IDENTIFIKATION`; `IDENT` is accepted as a fallback for the odd variant that
    /// self-identifies. Returns:
    /// - `Ok(Some(name))` — the group identified the variant,
    /// - `Ok(None)` — no ident job, or no `VARIANTE` result: the loaded `.prg` is
    ///   already the concrete SGBD, use it as-is.
    ///
    /// Runs a real telegram, so [`Session::initialize`] must have succeeded first.
    pub fn identify_variant(&mut self) -> Result<Option<String>> {
        let Some(job) = self.prg.ident_job() else {
            return Ok(None);
        };
        let variant_field = self.prg.variant_result();
        let res = self.run_job(job, "")?;
        Ok(res
            .get_str(variant_field)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()))
    }

    /// Liveness/presence probe: run the ECU's identification job and require a REAL bus
    /// answer — not merely `JOB_STATUS=OKAY`, which a job can report after every
    /// telegram timed out (comm errors are non-fatal so multi-address ident jobs can
    /// branch). `Ok(())` only when a telegram round-trip actually reached this ECU.
    /// If the `.prg` has no identification job there is nothing cheap to probe with, so
    /// the caller's successful `initialize` is accepted (`Ok(())`).
    ///
    /// Callers that localize errors should treat any `Err` as "not present" rather than
    /// parsing the message text.
    pub fn probe_presence(&mut self) -> Result<()> {
        let Some(job) = self.prg.ident_job() else {
            return Ok(());
        };
        let r = self.run_job(job, "")?;
        if !r.comm_ok() {
            // Diagnostic (trace-only): show whether the ECU answered at all and from which
            // address vs the CommParameter wake_addr. A non-empty `responders` that does
            // NOT contain `wake_addr` means the module DID reply, just from a different
            // diagnostic address than the static wake_addr expects → presence mis-gates.
            trace!(
                "[presence] '{job}' comm_ok=false — wake_addr={:?} responders={:02X?} status={:?}",
                self.vm.wake_addr(),
                self.vm.responders(),
                r.job_status()
            );
            return Err(Error::Vm("ECU did not answer on the bus".into()));
        }
        let ok = r
            .job_status()
            .map(|st| st.to_ascii_uppercase().contains("OKAY"))
            .unwrap_or_else(|| !r.is_empty());
        if ok {
            Ok(())
        } else {
            Err(Error::Vm(format!(
                "ECU answered but job status is {}",
                r.job_status().unwrap_or_else(|| "unknown".into())
            )))
        }
    }

    /// Whether the ECU produced a real bus answer within `within` (time-based liveness).
    /// Unlike a per-job flag, this survives across polls, so a GUI can distinguish a
    /// still-connected ECU from one that went silent mid-stream.
    pub fn link_alive(&self, within: std::time::Duration) -> bool {
        self.vm
            .last_response_at()
            .is_some_and(|t| t.elapsed() <= within)
    }

    /// Wall-clock instant of the most recent real bus answer, if any.
    pub fn last_response_at(&self) -> Option<std::time::Instant> {
        self.vm.last_response_at()
    }
}
