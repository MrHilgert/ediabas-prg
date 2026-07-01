use std::collections::VecDeque;
use crate::error::{Error, Result};
use super::Driver;

/// In-memory driver for unit tests — no real serial port required.
pub struct MockDriver {
    pub rx: VecDeque<u8>,
    pub tx_log: Vec<u8>,
}

impl MockDriver {
    pub fn new() -> Self {
        Self { rx: VecDeque::new(), tx_log: Vec::new() }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.rx.extend(bytes);
    }
}

impl Default for MockDriver {
    fn default() -> Self { Self::new() }
}

impl Driver for MockDriver {
    fn set_timeout(&mut self, _ms: u64) -> Result<()> { Ok(()) }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.tx_log.extend_from_slice(data);
        Ok(())
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        if self.rx.len() < buf.len() {
            return Err(Error::Timeout);
        }
        for b in buf.iter_mut() {
            *b = self.rx.pop_front().unwrap();
        }
        Ok(())
    }

    fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = buf.len().min(self.rx.len());
        if n == 0 {
            return Err(Error::Timeout);
        }
        for b in buf[..n].iter_mut() {
            *b = self.rx.pop_front().unwrap();
        }
        Ok(n)
    }

    fn set_break(&mut self, _on: bool) -> Result<()> { Ok(()) }
    fn set_rts(&mut self, _val: bool) -> Result<()> { Ok(()) }
    fn set_dtr(&mut self, _val: bool) -> Result<()> { Ok(()) }
    fn flush_rx(&mut self) { self.rx.clear(); }
}
