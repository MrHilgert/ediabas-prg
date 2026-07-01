// Protocol layer — sits on top of a Driver, knows about framing and init sequences.
// DS2, KWP1281, KWP2000 etc. all implement this trait.
pub mod ds2;
pub mod sim;

use crate::config::CommConfig;
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
