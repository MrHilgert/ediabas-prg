// Physical connection layer — knows nothing about DS2, KWP, or any protocol.
// Add a new impl (TcpDriver, BluetoothDriver, J2534Driver) to support a new adapter.
pub mod mock;
pub mod serial;

use crate::error::Result;

pub trait Driver: Send {
    /// Change baud rate and parity without re-opening the connection.
    fn set_timeout(&mut self, ms: u64) -> Result<()>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()>;
    fn read_some(&mut self, buf: &mut [u8]) -> Result<usize>;
    /// Toggle K-line BREAK condition (used by 5-baud init and fast init).
    fn set_break(&mut self, on: bool) -> Result<()>;
    /// Set RTS modem line (used by some K-line cables as TX-enable / direction control).
    fn set_rts(&mut self, val: bool) -> Result<()>;
    /// Set DTR modem line.
    fn set_dtr(&mut self, val: bool) -> Result<()>;
    /// Discard any bytes already sitting in the adapter's RX FIFO.
    fn flush_rx(&mut self);
}
