// Protocol layer — sits on top of a Driver, knows about framing and init sequences.
// DS2, KWP1281, KWP2000 etc. all implement this trait.
pub mod ds2;
pub mod kwp1281;
pub mod kwp2000;
pub mod sim;

use crate::config::{CommConfig, Protocol};
use crate::driver::Driver;
use crate::error::Result;

pub trait Transport: Send {
    /// Apply CommConfig (baud, parity, timeouts, len_offset) to this transport.
    fn configure(&mut self, cfg: &CommConfig) -> Result<()>;
    /// Perform protocol-level init handshake (5-baud init, fast init, etc.).
    /// DS2 ECUs connect immediately — this is a no-op for DS2.
    fn init_connection(&mut self) -> Result<()>;
    /// Send a frame, receive the response. Core operation.
    fn exchange(&mut self, frame: &[u8]) -> Result<Vec<u8>>;
    /// Close the protocol session.
    fn disconnect(&mut self) -> Result<()>;
}

/// Build the transport that implements `cfg.protocol`, pre-configured from `cfg`.
///
/// This is the single dispatch point that makes ECU support deterministic: whatever
/// concept the `.prg` declared (via [`crate::prg::PrgFile::initial_comm_config`]),
/// the matching transport is selected here. DS2-family ECUs get the proven
/// [`ds2::Ds2Transport`]; the other BMW concepts get their dedicated transport.
pub fn build(driver: Box<dyn Driver>, cfg: &CommConfig) -> Box<dyn Transport> {
    let mut t: Box<dyn Transport> = match cfg.protocol {
        Protocol::Ds2 => {
            let mut ds2 = ds2::Ds2Transport::new(driver);
            // KDCAN/FTDI K-line adapters mirror TX back on RX — always drain the echo.
            ds2.echo = true;
            Box::new(ds2)
        }
        // D-CAN through a KDCAN cable speaks the SAME BMW-FAST telegrams over the
        // 115200 serial link — the cable bridges to CAN/ISO-TP internally — so it
        // shares the KWP2000/BMW-FAST engine (see docs/PROTO-DCAN.md).
        Protocol::BmwFast | Protocol::Kwp2000Bmw | Protocol::DCan => {
            Box::new(kwp2000::Kwp2000Transport::new(driver))
        }
        Protocol::Kwp1281 => Box::new(kwp1281::Kwp1281Transport::new(driver)),
    };
    let _ = t.configure(cfg);
    t
}
