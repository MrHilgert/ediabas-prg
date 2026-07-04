use std::io::{Read, Write};
use std::time::Duration;
use crate::error::{Error, Result};
use super::Driver;

/// Map a `serialport` error into `Error::Io` PRESERVING its kind, so callers can tell a
/// missing device (`NotFound`) from a busy/denied port (`PermissionDenied`) — the flat
/// `ErrorKind::Other` used before erased that distinction.
fn serial_io_err(e: serialport::Error) -> Error {
    use serialport::ErrorKind as SK;
    let kind = match e.kind() {
        SK::NoDevice => std::io::ErrorKind::NotFound,
        SK::InvalidInput => std::io::ErrorKind::InvalidInput,
        SK::Io(k) => k,
        SK::Unknown => std::io::ErrorKind::Other,
    };
    Error::Io(std::io::Error::new(kind, e))
}

pub struct SerialDriver {
    port: Box<dyn serialport::SerialPort>,
}

impl SerialDriver {
    pub fn open(device: &str, baud: u32) -> Result<Self> {
        Self::open_parity(device, baud, serialport::Parity::Even)
    }

    pub fn open_parity(device: &str, baud: u32, parity: serialport::Parity) -> Result<Self> {
        let mut port = serialport::new(device, baud)
            .data_bits(serialport::DataBits::Eight)
            .parity(parity)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .timeout(Duration::from_millis(2000))
            .open()
            .map_err(serial_io_err)?;
        // Assert RTS and DTR — some K-line transceivers use these as enable signals.
        port.write_request_to_send(true).map_err(serial_io_err)?;
        port.write_data_terminal_ready(true).map_err(serial_io_err)?;
        Ok(Self { port })
    }
}

impl Driver for SerialDriver {
    fn set_timeout(&mut self, ms: u64) -> Result<()> {
        self.port
            .set_timeout(Duration::from_millis(ms))
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.port.write_all(data).map_err(Error::Io)?;
        self.port.flush().map_err(Error::Io)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.port.read_exact(buf).map_err(Error::Io)
    }

    fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.port.read(buf).map_err(Error::Io)
    }

    fn set_break(&mut self, on: bool) -> Result<()> {
        let r = if on { self.port.set_break() } else { self.port.clear_break() };
        r.map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))
    }

    fn set_rts(&mut self, val: bool) -> Result<()> {
        self.port
            .write_request_to_send(val)
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))
    }

    fn set_dtr(&mut self, val: bool) -> Result<()> {
        self.port
            .write_data_terminal_ready(val)
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))
    }

    fn flush_rx(&mut self) {
        // Drain whatever is sitting in the adapter FIFO before we start reading.
        let _ = self.port.clear(serialport::ClearBuffer::Input);
    }
}
