//! KWP1281 (concepts 0x0002/0x0003) K-line transport.
//!
//! The classic slow-init, block-oriented protocol (legacy BMW: old DDE2.x diesel,
//! ABS, IHKA). Physically single-wire K-line at 9600 **8N1** (not DS2's Even parity).
//! Full byte-level spec: `docs/PROTO-KWP1281.md`.
//!
//! Link layer, in short:
//!   * 5-baud address init → sync 0x55 → key bytes KB1/KB2 (=0x01/0x8A for "1281") →
//!     we reply ~KB2 → the ECU streams ident blocks (title 0xF6) which we ACK.
//!   * Thereafter a strict block ping-pong: `[LEN][COUNTER][TITLE][DATA…][0x03]`.
//!   * EVERY byte except the trailing 0x03 is acknowledged by the receiver echoing
//!     its bitwise complement. One block counter is shared by both directions.
//!   * A single `exchange()` sends one request block and reads response block(s)
//!     until the ECU returns an ACK (0x09), ACKing each data block in between.
//!
//! Because the KDCAN cable mirrors TX on RX, every byte we write is first read back
//! as its own echo before the complement/response is expected.

use std::thread::sleep;
use std::time::Duration;

use crate::config::CommConfig;
use crate::driver::Driver;
use crate::error::{Error, Result};
use crate::trace::trace;
use super::Transport;

const ACK: u8 = 0x09; // block title: acknowledge / keep-alive
const NAK: u8 = 0x0A; // block title: negative ack — resend last block
const END: u8 = 0x03; // block end marker (never complemented)
const IDENT: u8 = 0xF6; // ECU ident/ASCII data block
const END_OUTPUT: u8 = 0x06; // request: end communication

pub struct Kwp1281Transport {
    driver: Box<dyn Driver>,
    cfg: CommConfig,
    /// Running block counter, shared across both directions (wraps at 256).
    block_counter: u8,
    /// Whether the slow-init handshake has completed.
    connected: bool,
    /// Last block we transmitted, for NAK-triggered resends.
    last_tx: Vec<u8>,
    /// Per-byte complement/echo timeout (EdiabasLib Kwp1281ByteTimeout).
    byte_timeout_ms: u64,
    /// Gathered ident ASCII from the post-init 0xF6 blocks (part no., name…).
    pub ident: Vec<u8>,
}

impl Kwp1281Transport {
    pub fn new(driver: Box<dyn Driver>) -> Self {
        Self {
            driver,
            cfg: CommConfig::default(),
            block_counter: 0,
            connected: false,
            last_tx: Vec::new(),
            byte_timeout_ms: 55,
            ident: Vec::new(),
        }
    }

    // ── byte-level handshake ────────────────────────────────────────────────

    /// Send one byte and wait for its complement ack. KDCAN echoes TX, so we first
    /// read our own echo, then the receiver's `!b`.
    fn send_byte(&mut self, b: u8) -> Result<()> {
        self.driver.write(&[b])?;
        let mut echo = [0u8; 1];
        self.driver.read_exact(&mut echo)?;
        if echo[0] != b {
            return Err(Error::Protocol(format!(
                "KWP1281: bus collision (sent {b:#04x}, echo {:#04x})",
                echo[0]
            )));
        }
        self.driver.set_timeout(self.byte_timeout_ms)?;
        let mut compl = [0u8; 1];
        self.driver.read_exact(&mut compl)?;
        if compl[0] != !b {
            return Err(Error::Protocol(format!(
                "KWP1281: bad complement for {b:#04x} (got {:#04x})",
                compl[0]
            )));
        }
        if self.cfg.interbyte_ms > 0 {
            sleep(Duration::from_millis(self.cfg.interbyte_ms));
        }
        Ok(())
    }

    /// Send one byte WITHOUT expecting a complement (the trailing 0x03), draining echo.
    fn send_raw(&mut self, b: u8) -> Result<()> {
        self.driver.write(&[b])?;
        let mut echo = [0u8; 1];
        self.driver.read_exact(&mut echo)?;
        Ok(())
    }

    /// Receive one byte and reply with its complement (draining our echo of it).
    fn recv_byte(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.driver.read_exact(&mut b)?;
        // Short pause so the ECU is ready to receive our complement.
        sleep(Duration::from_millis(self.cfg.interbyte_ms.max(1)));
        let compl = !b[0];
        self.driver.write(&[compl])?;
        let mut echo = [0u8; 1];
        self.driver.read_exact(&mut echo)?;
        Ok(b[0])
    }

    /// Receive one byte with NO complement reply (the trailing 0x03).
    fn recv_raw(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.driver.read_exact(&mut b)?;
        Ok(b[0])
    }

    // ── block level ─────────────────────────────────────────────────────────

    /// Send a full block `[LEN][COUNTER][TITLE][DATA…][0x03]`.
    fn send_block(&mut self, title: u8, data: &[u8]) -> Result<()> {
        self.block_counter = self.block_counter.wrapping_add(1);
        let len = (data.len() + 3) as u8; // COUNTER + TITLE + 0x03, plus DATA
        let mut buf = Vec::with_capacity(data.len() + 3);
        buf.push(len);
        buf.push(self.block_counter);
        buf.push(title);
        buf.extend_from_slice(data);
        trace!("KWP1281 TX block: {}", fmt_hex(&buf));
        for &b in &buf {
            self.send_byte(b)?;
        }
        self.send_raw(END)?;
        self.last_tx = buf;
        Ok(())
    }

    /// Receive a full block; returns `(title, data)`.
    fn recv_block(&mut self, first_byte_ms: u64) -> Result<(u8, Vec<u8>)> {
        self.driver.set_timeout(first_byte_ms)?;
        let len = self.recv_byte()?;
        if len < 3 {
            return Err(Error::Protocol(format!("KWP1281: block LEN {len} < 3")));
        }
        self.driver.set_timeout(self.byte_timeout_ms)?;
        let ctr = self.recv_byte()?;
        let title = self.recv_byte()?;
        let mut data = Vec::with_capacity((len - 3) as usize);
        for _ in 0..(len - 3) {
            data.push(self.recv_byte()?);
        }
        let end = self.recv_raw()?;
        if end != END {
            return Err(Error::Protocol(format!(
                "KWP1281: bad block end {end:#04x} (want 0x03)"
            )));
        }
        // Adopt the ECU's counter (tolerant to occasional drift; see spec §5).
        self.block_counter = ctr;
        // Inter-block gap before our next block.
        sleep(Duration::from_millis(10));
        Ok((title, data))
    }

    /// 5-baud slow init + key-byte handshake + drain the ECU's ident blocks.
    fn slow_init(&mut self) -> Result<()> {
        const BIT_MS: u64 = 200;
        let addr = self
            .cfg
            .wake_addr
            .ok_or_else(|| Error::Protocol("KWP1281: wake address is required".into()))?;

        let _ = self.driver.set_dtr(true);
        self.driver.flush_rx();
        self.block_counter = 0;
        self.ident.clear();

        // Bus idle, then the 5-baud address byte, LSB first, framed by start/stop bits.
        self.driver.set_break(false)?;
        sleep(Duration::from_millis(300));
        self.driver.set_break(true)?; // start bit (LOW)
        sleep(Duration::from_millis(BIT_MS));
        for pos in 0..8u8 {
            let bit = (addr >> pos) & 1;
            self.driver.set_break(bit == 0)?; // 0 → LOW (break on)
            sleep(Duration::from_millis(BIT_MS));
        }
        self.driver.set_break(false)?; // stop bit (HIGH)
        sleep(Duration::from_millis(BIT_MS));
        self.driver.flush_rx(); // discard BREAK framing artefacts

        // Sync byte 0x55, then two key bytes.
        self.driver.set_timeout(3000)?;
        let mut b = [0u8; 1];
        loop {
            self.driver.read_exact(&mut b)?;
            if b[0] == 0x55 {
                break;
            }
        }
        let mut kb1 = [0u8; 1];
        let mut kb2 = [0u8; 1];
        self.driver.read_exact(&mut kb1)?;
        self.driver.read_exact(&mut kb2)?;
        let protocol = (((kb2[0] & 0x7f) as u16) << 7) | (kb1[0] & 0x7f) as u16;
        trace!("KWP1281 key bytes: {:#04x} {:#04x} → protocol {protocol}", kb1[0], kb2[0]);
        if protocol != 1281 {
            return Err(Error::Protocol(format!(
                "KWP1281: unexpected key bytes {:#04x}/{:#04x} (protocol {protocol}, want 1281)",
                kb1[0], kb2[0]
            )));
        }

        // W4 pause, then reply with the complement of KB2.
        sleep(Duration::from_millis(30));
        self.send_raw(!kb2[0])?;

        // Some ECUs echo the inverted address before the first ident block — swallow it.
        // Then drain ident blocks (0xF6) until the ECU ACKs, ACKing each in turn.
        self.connected = true; // send_block/recv_block need the link considered up
        loop {
            let (title, data) = self.recv_block(1500)?;
            match title {
                ACK => break,
                IDENT => self.ident.extend_from_slice(&data),
                _ => {}
            }
            self.send_block(ACK, &[])?;
        }
        Ok(())
    }
}

impl Transport for Kwp1281Transport {
    fn configure(&mut self, cfg: &CommConfig) -> Result<()> {
        self.cfg = cfg.clone();
        // Port is opened 9600 8N1 by Session (parse() gives Parity::None for KWP1281).
        self.driver.set_timeout(cfg.timeout_std_ms)
    }

    fn init_connection(&mut self) -> Result<()> {
        if self.connected {
            return Ok(());
        }
        self.slow_init()
    }

    fn exchange(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        if !self.connected {
            return Err(Error::Protocol("KWP1281: not connected (init first)".into()));
        }
        if frame.is_empty() {
            return Err(Error::Protocol("KWP1281: empty request".into()));
        }
        self.driver.flush_rx();
        // frame = [TITLE][DATA…]; LEN/COUNTER/0x03 are added by send_block.
        self.send_block(frame[0], &frame[1..])?;

        let mut out = Vec::new();
        let mut resends = 0u8;
        loop {
            let (title, data) = self.recv_block(self.cfg.timeout_std_ms)?;
            match title {
                ACK => break, // ECU done — dialogue complete
                NAK => {
                    if resends >= 3 {
                        return Err(Error::Protocol("KWP1281: too many NAKs".into()));
                    }
                    resends += 1;
                    let buf = std::mem::take(&mut self.last_tx);
                    for &b in &buf {
                        self.send_byte(b)?;
                    }
                    self.send_raw(END)?;
                    self.last_tx = buf;
                }
                _ => {
                    // A data block: collect [title][data] and ACK so the ECU may send more.
                    out.push(title);
                    out.extend_from_slice(&data);
                    self.send_block(ACK, &[])?;
                }
            }
        }
        Ok(out)
    }

    fn disconnect(&mut self) -> Result<()> {
        if self.connected {
            let _ = self.send_block(END_OUTPUT, &[]);
        }
        self.connected = false;
        self.block_counter = 0;
        Ok(())
    }
}

fn fmt_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::mock::MockDriver;

    /// Box the MockDriver and return the transport plus a STABLE pointer to the
    /// driver inside its heap box (coercing `Box<MockDriver>` → `Box<dyn Driver>`
    /// does not move the heap allocation), so tests can inspect `tx_log` soundly.
    fn transport(driver: MockDriver) -> (Kwp1281Transport, *const MockDriver) {
        let boxed = Box::new(driver);
        let ptr: *const MockDriver = &*boxed;
        let mut t = Kwp1281Transport::new(boxed);
        let mut cfg = CommConfig::default();
        cfg.concept = 0x0002;
        cfg.interbyte_ms = 0; // no sleeps in tests
        t.cfg = cfg;
        (t, ptr)
    }

    fn tx_log(ptr: *const MockDriver) -> Vec<u8> {
        unsafe { (*ptr).tx_log.clone() }
    }

    /// Feed the RX stream a transmitted block expects: for each block byte, the
    /// echo (KDCAN mirror) then the receiver's complement; then the 0x03 echo.
    fn tx_echo_stream(block: &[u8]) -> Vec<u8> {
        let mut rx = Vec::new();
        for &b in block {
            rx.push(b); // echo
            rx.push(!b); // complement ack
        }
        rx.push(END); // echo of the trailing 0x03 (send_raw)
        rx
    }

    #[test]
    fn send_block_frames_len_counter_and_end() {
        let mut d = MockDriver::new();
        // Expected on-wire block for title=0x07 data=[] counter→1: [03,01,07,03]
        d.feed(&tx_echo_stream(&[0x03, 0x01, 0x07]));
        let (mut t, ptr) = transport(d);
        t.connected = true;
        t.send_block(0x07, &[]).unwrap();
        // We must have transmitted exactly the block bytes + 0x03 terminator.
        assert_eq!(tx_log(ptr), vec![0x03, 0x01, 0x07, 0x03]);
        assert_eq!(t.block_counter, 1);
    }

    #[test]
    fn send_block_computes_len_with_data() {
        let mut d = MockDriver::new();
        // title=0x29 data=[0x0F] counter→1: [04,01,29,0F,03]
        d.feed(&tx_echo_stream(&[0x04, 0x01, 0x29, 0x0F]));
        let (mut t, ptr) = transport(d);
        t.connected = true;
        t.send_block(0x29, &[0x0F]).unwrap();
        assert_eq!(tx_log(ptr), vec![0x04, 0x01, 0x29, 0x0F, 0x03]);
    }

    #[test]
    fn recv_block_reads_and_complements() {
        // ECU sends block [04,05,F6,41,03]; we complement every byte except 0x03.
        // RX stream interleaves: byte, then echo-of-our-complement.
        let block = [0x04u8, 0x05, 0xF6, 0x41];
        let mut rx = Vec::new();
        for &b in &block {
            rx.push(b); // the ECU byte
            rx.push(!b); // echo of the complement we write
        }
        rx.push(END); // trailing 0x03, no complement
        let mut d = MockDriver::new();
        d.feed(&rx);
        let (mut t, ptr) = transport(d);
        t.connected = true;
        let (title, data) = t.recv_block(1000).unwrap();
        assert_eq!(title, 0xF6);
        assert_eq!(data, vec![0x41]);
        assert_eq!(t.block_counter, 0x05); // adopted from the block
        // We wrote a complement for LEN, COUNTER, TITLE, DATA (4 bytes), none for 0x03.
        assert_eq!(tx_log(ptr), vec![!0x04, !0x05, !0xF6, !0x41]);
    }

    #[test]
    fn bad_complement_is_error() {
        let mut d = MockDriver::new();
        d.feed(&[0x03, 0xFF]); // echo ok (0x03), but complement 0xFF != !0x03(=0xFC)
        let (mut t, _ptr) = transport(d);
        t.connected = true;
        assert!(t.send_byte(0x03).is_err());
    }
}
