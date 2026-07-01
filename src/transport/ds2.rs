// BMW DS2 K-line protocol transport.
//
// Response frame layout depends on EDIABAS concept:
//   Old-style (len_add=0): [ADDR][LEN][SVC][DATA...][CHK]             → len_offset=1
//                          [ADDR][SRC][LEN][SVC][DATA...][CHK]         → len_offset=2
//   BMW 4-byte (len_add=5): [FMT=B8][TGT][SRC][LEN][SVC][DATA...][CHK] → len_offset=3
//
// len_add = CommAnswerLen constant:
//   len_add=0 → LEN = total frame length (all bytes incl. header and CHK).
//   len_add=5 → LEN = payload count; total_frame = LEN + 5 (4 hdr + LEN payload + 1 CHK).
//
// General formula: remaining_after_header = LEN + len_add - hdr_len
//
// CHK = XOR of all bytes except CHK itself.
// DS2 ECUs connect immediately — no 5-baud init required.
use std::thread::sleep;
use std::time::Duration;
use crate::config::CommConfig;
use crate::driver::Driver;
use crate::error::{Error, Result};
use super::Transport;

pub struct Ds2Transport {
    driver: Box<dyn Driver>,
    /// true if the K-line adapter mirrors TX bytes back on RX (typical for single-wire K-line).
    pub echo: bool,
    /// Index of the LEN byte in ECU response: 1, 2, or 3.
    pub len_offset: usize,
    /// CommAnswerLen additive constant: 0 → LEN=total, 5 → LEN=payload_count (4-byte header).
    pub len_add: usize,
    timeout_std_ms: u64,
    regen_time_ms: u64,
    /// Half-duplex direction control via RTS.
    /// When Some(true):  RTS=HIGH before write, RTS=LOW after write (to release K-line for RX).
    /// When Some(false): RTS=LOW always (RX enabled from the start).
    /// When None: RTS is not touched after open_parity().
    pub rts_rx_low: Option<bool>,
    /// Extra delay (ms) between end of TX (and echo drain) and starting to read ECU response.
    pub regen_delay_ms: u64,
}

impl Ds2Transport {
    pub fn new(driver: Box<dyn Driver>) -> Self {
        Self {
            driver,
            echo: false,
            len_offset: 1,
            len_add: 0,
            timeout_std_ms: 2000,
            regen_time_ms: 20,
            rts_rx_low: None,
            regen_delay_ms: 0,
        }
    }

    /// Build a complete DS2 frame: [ecu_addr][LEN][payload...][CHK]
    pub fn build_frame(ecu_addr: u8, payload: &[u8]) -> Vec<u8> {
        let total = 2 + payload.len() + 1;
        let mut frame = Vec::with_capacity(total);
        frame.push(ecu_addr);
        frame.push(total as u8);
        frame.extend_from_slice(payload);
        let chk = frame.iter().fold(0u8, |a, &b| a ^ b);
        frame.push(chk);
        frame
    }

    /// Append XOR checksum to raw bytes (e.g. a TELEGRAM straight from .prg).
    pub fn append_checksum(raw: &[u8]) -> Vec<u8> {
        let chk = raw.iter().fold(0u8, |a, &b| a ^ b);
        let mut frame = raw.to_vec();
        frame.push(chk);
        frame
    }

    /// Send K-line fast-init wakeup: BREAK for `ms` ms, then idle for `ms` ms.
    /// Drains any noise bytes received during the break before returning.
    pub fn fast_wakeup(&mut self, ms: u64) -> Result<()> {
        self.driver.set_break(true)?;
        sleep(Duration::from_millis(ms));
        self.driver.set_break(false)?;
        sleep(Duration::from_millis(ms));
        // Drain noise received during break
        let _ = self.receive_raw(10, 5);
        Ok(())
    }

    /// Send raw bytes, optionally discarding TX echo.
    /// If `rts_rx_low` is configured, asserts RTS=HIGH before TX and lowers it after
    /// (half-duplex direction control for cables that use RTS as TX-enable).
    pub fn send_raw(&mut self, frame: &[u8]) -> Result<()> {
        // Half-duplex: raise RTS to enable TX driver before sending
        if self.rts_rx_low == Some(true) {
            let _ = self.driver.set_rts(true);
        }
        self.driver.write(frame)?;
        if self.echo {
            let mut echo_buf = vec![0u8; frame.len()];
            self.driver.read_exact(&mut echo_buf)?;
        }
        // Half-duplex: lower RTS to release bus for ECU response
        if self.rts_rx_low.is_some() {
            let _ = self.driver.set_rts(false);
        }
        if self.regen_delay_ms > 0 {
            sleep(Duration::from_millis(self.regen_delay_ms));
        }
        Ok(())
    }

    /// Collect all response bytes using timeouts.
    /// `first_ms`  — how long to wait for the very first byte.
    /// `gap_ms`    — inter-byte silence that signals end of frame (typ. 20 ms).
    /// Returns empty Vec on first-byte timeout (no response).
    pub fn receive_raw(&mut self, first_ms: u64, gap_ms: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];

        self.driver.set_timeout(first_ms).ok();
        if self.driver.read_exact(&mut byte).is_err() {
            return buf;
        }
        buf.push(byte[0]);

        self.driver.set_timeout(gap_ms).ok();
        loop {
            match self.driver.read_some(&mut byte) {
                Ok(1) => buf.push(byte[0]),
                _     => break,
            }
        }
        buf
    }

    /// Read a framed DS2 response using `self.len_offset` and `self.len_add`.
    ///
    /// Formula: total_frame = LEN_byte + len_add
    ///   len_add=0 → LEN is the total frame length (old-style ECUs, len_offset=1 or 2)
    ///   len_add=5 → LEN is payload count; total = LEN+5 (4-byte BMW DS2 header, len_offset=3)
    ///
    /// Returns payload bytes (after header, before CHK).
    fn receive(&mut self) -> Result<Vec<u8>> {
        let hdr_len = self.len_offset + 1;
        let mut header = vec![0u8; hdr_len];
        self.driver.read_exact(&mut header)?;

        let len_byte = header[self.len_offset] as usize;
        let total = len_byte + self.len_add;
        if total < hdr_len + 1 {
            return Err(Error::Protocol(format!(
                "DS2: LEN={len_byte} + add={} = {total} too small (header={hdr_len})",
                self.len_add
            )));
        }

        let remaining = total - hdr_len;  // payload_bytes + CHK
        let mut rest = vec![0u8; remaining];
        self.driver.read_exact(&mut rest)?;

        let all: Vec<u8> = header.iter().chain(rest.iter()).copied().collect();
        let chk_idx = all.len() - 1;
        let expected = all[..chk_idx].iter().fold(0u8, |a, &b| a ^ b);
        let got = all[chk_idx];
        if expected != got {
            return Err(Error::Checksum { expected, got });
        }

        Ok(rest[..remaining - 1].to_vec())
    }

    /// K-line 5-baud initialization (ISO 9141-2).
    /// BMW DS2 ECUs do NOT require this — only for KWP1281/KWP2000.
    pub fn kline_5baud_init(&mut self, ecu_addr: u8) -> Result<()> {
        const BIT_MS: u64 = 200;

        eprintln!("  [5baud] idle HIGH...");
        self.driver.set_break(false)?;
        sleep(Duration::from_millis(300));

        eprintln!("  [5baud] start bit LOW");
        self.driver.set_break(true)?;
        sleep(Duration::from_millis(BIT_MS));

        for bit_pos in 0..8u8 {
            let bit = (ecu_addr >> bit_pos) & 1;
            eprintln!("  [5baud] bit {bit_pos} = {bit}");
            self.driver.set_break(bit == 0)?;
            sleep(Duration::from_millis(BIT_MS));
        }

        eprintln!("  [5baud] stop bit HIGH");
        self.driver.set_break(false)?;
        sleep(Duration::from_millis(BIT_MS));
        eprintln!("  [5baud] 5-baud address sent");

        self.driver.set_timeout(3000)?;
        eprintln!("  [5baud] waiting for sync byte 0x55...");
        let mut sync = [0u8; 1];
        loop {
            self.driver.read_exact(&mut sync)?;
            eprintln!("  [5baud] got byte: {:#04x}", sync[0]);
            if sync[0] == 0x55 { break; }
        }

        let mut w1 = [0u8; 1];
        let mut w2 = [0u8; 1];
        self.driver.read_exact(&mut w1)?;
        eprintln!("  [5baud] W1={:#04x}", w1[0]);
        self.driver.read_exact(&mut w2)?;
        eprintln!("  [5baud] W2={:#04x}", w2[0]);

        sleep(Duration::from_millis(25));
        let inv_w2 = !w2[0];
        eprintln!("  [5baud] sending ~W2={inv_w2:#04x}");
        self.driver.write(&[inv_w2])?;
        self.driver.read_exact(&mut sync)?;
        eprintln!("  [5baud] echo: {:#04x}", sync[0]);
        self.driver.read_exact(&mut sync)?;
        eprintln!("K-line init complete: ECU addr={:#04x}", sync[0]);

        self.driver.set_timeout(self.timeout_std_ms)?;
        Ok(())
    }
}

impl Transport for Ds2Transport {
    fn configure(&mut self, cfg: &CommConfig) -> Result<()> {
        self.len_offset = cfg.len_offset;
        self.len_add = cfg.len_add;
        self.timeout_std_ms = cfg.timeout_std_ms;
        self.regen_time_ms = cfg.regen_time_ms;
        self.driver.set_timeout(cfg.timeout_std_ms)
    }

    fn init_connection(&mut self) -> Result<()> {
        // DS2 ECUs respond immediately — no init handshake required.
        Ok(())
    }

    fn exchange(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        let with_chk = Self::append_checksum(frame);
        eprintln!("TX: {}", fmt_hex(&with_chk));
        self.send_raw(&with_chk)?;
        self.driver.set_timeout(self.timeout_std_ms)?;
        let resp = self.receive();
        match &resp {
            Ok(r)  => eprintln!("RX: {}", fmt_hex(r)),
            Err(e) => eprintln!("RX err: {e}"),
        }
        resp
    }

    fn disconnect(&mut self) -> Result<()> { Ok(()) }
}

fn fmt_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

/// Parse hex string "B812F1..." or "B8 12 F1 ..." into bytes.
pub fn parse_hex(s: &str) -> Option<Vec<u8>> {
    let clean = s.replace([' ', ':'], "");
    if clean.len() % 2 != 0 { return None; }
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::mock::MockDriver;

    fn make_transport() -> Ds2Transport {
        Ds2Transport::new(Box::new(MockDriver::new()))
    }

    #[test]
    fn build_frame_len_correct() {
        let frame = Ds2Transport::build_frame(0x12, &[0x0B, 0x03]);
        assert_eq!(frame[0], 0x12);
        assert_eq!(frame[1], 0x05);
        assert_eq!(frame[2], 0x0B);
        assert_eq!(frame[3], 0x03);
        assert_eq!(frame[4], 0x1F);
    }

    #[test]
    fn append_checksum_matches_build() {
        let via_append = Ds2Transport::append_checksum(&[0x12, 0x05, 0x0B, 0x03]);
        let via_build  = Ds2Transport::build_frame(0x12, &[0x0B, 0x03]);
        assert_eq!(via_append, via_build);
    }

    #[test]
    fn parse_hex_formats() {
        assert_eq!(parse_hex("12050B03"), Some(vec![0x12, 0x05, 0x0B, 0x03]));
        assert_eq!(parse_hex("B8 12 F1"), Some(vec![0xB8, 0x12, 0xF1]));
        assert_eq!(parse_hex("FF"),        Some(vec![0xFF]));
        assert_eq!(parse_hex("ZZ"),        None);
        assert_eq!(parse_hex("1"),         None);
    }

    #[test]
    fn checksum_known_frame() {
        // B8 12 F1 04 2C 10 0F 10 → CHK = 7C
        let frame = Ds2Transport::append_checksum(&[0xB8, 0x12, 0xF1, 0x04, 0x2C, 0x10, 0x0F, 0x10]);
        assert_eq!(*frame.last().unwrap(), 0x7C);
    }

    #[test]
    fn receive_concept6_frame() {
        // Concept 0x0006: [ADDR][LEN][SVC][DATA...][CHK], len_offset=1
        // Frame: B8 06 61 01 02 03 CHK
        let payload = [0xB8u8, 0x07, 0x61, 0x01, 0x02, 0x03];
        let chk = payload.iter().fold(0u8, |a, &b| a ^ b);
        let mut full = payload.to_vec();
        full.push(chk);

        let mut driver = MockDriver::new();
        driver.feed(&full);

        let mut t = Ds2Transport::new(Box::new(driver));
        t.len_offset = 1;
        let resp = t.receive().unwrap();
        // payload after header (2 bytes) and before CHK (1 byte)
        assert_eq!(resp, &[0x61, 0x01, 0x02, 0x03]);
    }
}
