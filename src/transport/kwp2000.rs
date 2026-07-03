//! BMW-FAST telegram engine — the shared transport for four EDIABAS concepts:
//!   * 0x010F BMW-FAST      (K-line, no init)
//!   * 0x010C KWP2000-BMW   (K-line, fast-init + StartCommunication)
//!   * 0x010D KWP2000*       (K-line, XOR checksum + Even parity, no init)
//!   * 0x0110 D-CAN         (over the KDCAN cable, which bridges to CAN internally —
//!                           the PC speaks the *same* BMW-FAST telegrams at 115200)
//!
//! Division of labour (see `docs/PROTO-KWP2000.md`): the SGBD bytecode builds the
//! complete request telegram `[FMT][TGT][SRC](LEN)[SID][DATA…]` WITHOUT the trailing
//! checksum. This transport owns only: the checksum, the response framing, the
//! `7F..78` (responsePending) wait loop, and — for 0x010C — the fast-init handshake.
//!
//! The one correctness trap (verified against EdiabasLib `CalcChecksumBmwFast` vs
//! `CalcChecksumXor`): BMW-FAST / KWP2000-BMW use the ISO 14230 checksum = arithmetic
//! **sum of all bytes mod 256**, NOT XOR. Only DS2 and 0x010D use XOR. Using XOR on a
//! BMW-FAST ECU is a silent-failure bug — the ECU simply never answers.

use std::thread::sleep;
use std::time::Duration;

use crate::config::CommConfig;
use crate::driver::Driver;
use crate::error::{Error, Result};
use crate::trace::trace;
use super::Transport;

/// Checksum algorithm over a telegram (all bytes before the checksum byte).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Csum {
    /// ISO 14230 arithmetic sum mod 256 (BMW-FAST, KWP2000-BMW, D-CAN).
    Sum,
    /// XOR of all bytes (KWP2000*, concept 0x010D).
    Xor,
}

/// Response framing rule.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Framing {
    /// Variable BMW-FAST framing: short / long / extra-long forms keyed by FMT.
    BmwFast,
    /// Fixed 4-byte header, LEN always at byte [3] (concept 0x010D).
    Kwp2000S,
}

pub struct Kwp2000Transport {
    driver: Box<dyn Driver>,
    cfg: CommConfig,
    csum: Csum,
    framing: Framing,
    /// true if the adapter mirrors TX bytes back on RX (KDCAN/FTDI K-line + D-CAN cable).
    pub echo: bool,
}

impl Kwp2000Transport {
    pub fn new(driver: Box<dyn Driver>) -> Self {
        Self {
            driver,
            cfg: CommConfig::default(),
            csum: Csum::Sum,
            framing: Framing::BmwFast,
            echo: true,
        }
    }

    /// Compute the checksum byte for `data` (the frame without its checksum).
    fn checksum(csum: Csum, data: &[u8]) -> u8 {
        match csum {
            Csum::Sum => data.iter().fold(0u8, |a, &b| a.wrapping_add(b)),
            Csum::Xor => data.iter().fold(0u8, |a, &b| a ^ b),
        }
    }

    /// Total telegram length (EXCLUDING the trailing checksum) implied by the header
    /// of a BMW-FAST frame. `hdr` must hold at least 4 bytes; for the extra-long form
    /// it must hold 6. Returns the length in bytes of `[header..payload]`.
    fn tel_length_bmwfast(hdr: &[u8]) -> usize {
        let n = (hdr[0] & 0x3f) as usize;
        if n != 0 {
            n + 3 // short: FMT=0x80|N, TGT, SRC, N payload
        } else if hdr[3] != 0 {
            hdr[3] as usize + 4 // long: LEN at [3]
        } else {
            // extra-long (BMW extension): 16-bit big-endian length at [4][5]
            let len16 = ((hdr[4] as usize) << 8) | hdr[5] as usize;
            len16 + 6
        }
    }

    /// Byte offset where the diagnostic payload starts in a received frame (already
    /// minus checksum), for `7F..78` detection.
    fn payload_offset(&self, frame: &[u8]) -> usize {
        match self.framing {
            Framing::Kwp2000S => 4,
            Framing::BmwFast => {
                if frame.is_empty() {
                    return 0;
                }
                let n = frame[0] & 0x3f;
                if n != 0 {
                    3
                } else if frame.len() > 3 && frame[3] != 0 {
                    4
                } else {
                    6
                }
            }
        }
    }

    /// Transmit a frame (checksum appended), draining the TX echo if present.
    fn transmit(&mut self, frame: &[u8]) -> Result<()> {
        let chk = Self::checksum(self.csum, frame);
        let mut out = frame.to_vec();
        out.push(chk);
        trace!("TX: {}", fmt_hex(&out));

        self.driver.flush_rx();
        if self.cfg.interbyte_ms > 0 {
            for (i, b) in out.iter().enumerate() {
                self.driver.write(&[*b])?;
                if i + 1 < out.len() {
                    sleep(Duration::from_millis(self.cfg.interbyte_ms));
                }
            }
        } else {
            self.driver.write(&out)?;
        }
        if self.echo {
            // KDCAN mirrors TX on RX — drain exactly the bytes we sent.
            self.driver.set_timeout(100)?;
            let mut echo = vec![0u8; out.len()];
            self.driver.read_exact(&mut echo)?;
        }
        Ok(())
    }

    /// Read one complete, checksum-valid response telegram; return it minus the
    /// trailing checksum (EDIABAS result byte-positions index into the full frame).
    fn receive_frame(&mut self, first_byte_ms: u64) -> Result<Vec<u8>> {
        // Header: always at least 4 bytes.
        self.driver.set_timeout(first_byte_ms)?;
        let mut hdr = vec![0u8; 4];
        self.driver.read_exact(&mut hdr)?;

        let (total_no_chk, hdr_read) = match self.framing {
            Framing::Kwp2000S => (hdr[3] as usize + 4, 4),
            Framing::BmwFast => {
                if hdr[0] & 0xc0 != 0x80 {
                    self.driver.flush_rx();
                    return Err(Error::Protocol(format!(
                        "BMW-FAST: bad response FMT {:#04x} (need 0x80..0xBF)",
                        hdr[0]
                    )));
                }
                // Extra-long form needs two more header bytes before length is known.
                if (hdr[0] & 0x3f) == 0 && hdr[3] == 0 {
                    self.driver.set_timeout(self.cfg.timeout_tel_ms.max(20))?;
                    let mut ext = vec![0u8; 2];
                    self.driver.read_exact(&mut ext)?;
                    hdr.extend_from_slice(&ext);
                    (Self::tel_length_bmwfast(&hdr), 6)
                } else {
                    (Self::tel_length_bmwfast(&hdr), 4)
                }
            }
        };

        if total_no_chk < hdr_read {
            return Err(Error::Protocol(format!(
                "BMW-FAST: telegram length {total_no_chk} < header {hdr_read}"
            )));
        }
        let remaining = total_no_chk - hdr_read + 1; // payload tail + CHK
        self.driver.set_timeout(self.cfg.timeout_tel_ms.max(20))?;
        let mut tail = vec![0u8; remaining];
        self.driver.read_exact(&mut tail)?;

        let mut all = hdr;
        all.extend_from_slice(&tail);
        let chk_idx = all.len() - 1;
        let expected = Self::checksum(self.csum, &all[..chk_idx]);
        let got = all[chk_idx];
        if expected != got {
            self.driver.flush_rx();
            return Err(Error::Checksum { expected, got });
        }
        all.truncate(chk_idx);
        trace!("RX: {}", fmt_hex(&all));
        Ok(all)
    }
}

impl Transport for Kwp2000Transport {
    fn configure(&mut self, cfg: &CommConfig) -> Result<()> {
        self.cfg = cfg.clone();
        // 0x010D is the XOR + fixed-header variant; everything else is BMW-FAST/SUM.
        if cfg.concept == 0x010D {
            self.csum = Csum::Xor;
            self.framing = Framing::Kwp2000S;
        } else {
            self.csum = Csum::Sum;
            self.framing = Framing::BmwFast;
        }
        self.driver.set_timeout(cfg.timeout_std_ms)
    }

    fn init_connection(&mut self) -> Result<()> {
        // 0x010F (BMW-FAST), 0x010D (KWP2000*) and 0x0110 (D-CAN over the bridging
        // cable) all listen permanently — no wake stimulation, exactly like DS2.
        // 0x010C needs a fast-init BREAK + StartCommunication telegram from the
        // CommParameter; that is a separate, live-hardware-gated step (see
        // docs/PROTO-KWP2000.md §3) not yet wired.
        if self.cfg.concept == 0x010C {
            return Err(Error::NotSupported(
                "KWP2000-BMW (0x010C) fast-init not yet wired — see docs/PROTO-KWP2000.md §3".into(),
            ));
        }
        Ok(())
    }

    fn exchange(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        // P3 regeneration gap before every request (ECU needs idle between telegrams).
        if self.cfg.regen_time_ms > 0 {
            sleep(Duration::from_millis(self.cfg.regen_time_ms));
        }
        self.transmit(frame)?;

        // responsePending (7F .. 78) loop: keep waiting on the extended timeout.
        let mut first_ms = self.cfg.timeout_std_ms;
        for _ in 0..=self.cfg.retry_nr78 {
            let resp = self.receive_frame(first_ms)?;
            let off = self.payload_offset(&resp);
            let is_nr78 = resp.len() >= off + 3
                && resp[off] == 0x7f
                && resp[off + 2] == 0x78;
            if is_nr78 {
                trace!("[kwp2000] responsePending (7F..78) — waiting");
                first_ms = self.cfg.timeout_nr78_ms;
                continue;
            }
            return Ok(resp);
        }
        Err(Error::Timeout)
    }

    fn disconnect(&mut self) -> Result<()> {
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

    fn transport(concept: u16, driver: MockDriver) -> Kwp2000Transport {
        let mut t = Kwp2000Transport::new(Box::new(driver));
        let mut cfg = CommConfig::default();
        cfg.concept = concept;
        cfg.timeout_tel_ms = 20;
        t.echo = false; // MockDriver has no echo
        t.configure(&cfg).unwrap();
        t
    }

    #[test]
    fn sum_checksum_short_form() {
        // 82 12 F1 1A 80 → CHK = (82+12+F1+1A+80) mod 256 = 1F  (spec §1.3)
        let c = Kwp2000Transport::checksum(Csum::Sum, &[0x82, 0x12, 0xF1, 0x1A, 0x80]);
        assert_eq!(c, 0x1F);
    }

    #[test]
    fn sum_checksum_long_form() {
        // 80 12 F1 04 2C 10 0F 10 → CHK = E2  (spec §1.3)
        let c = Kwp2000Transport::checksum(
            Csum::Sum,
            &[0x80, 0x12, 0xF1, 0x04, 0x2C, 0x10, 0x0F, 0x10],
        );
        assert_eq!(c, 0xE2);
    }

    #[test]
    fn xor_checksum_kwp2000s() {
        // B8 12 F1 02 21 0B → XOR = 73  (spec §2)
        let c = Kwp2000Transport::checksum(Csum::Xor, &[0xB8, 0x12, 0xF1, 0x02, 0x21, 0x0B]);
        assert_eq!(c, 0x73);
    }

    #[test]
    fn tel_length_forms() {
        // short: FMT=0x82 → 2 payload + 3 header = 5
        assert_eq!(Kwp2000Transport::tel_length_bmwfast(&[0x82, 0x12, 0xF1, 0x1A]), 5);
        // long: FMT=0x80, LEN=4 → 4 + 4 = 8
        assert_eq!(Kwp2000Transport::tel_length_bmwfast(&[0x80, 0x12, 0xF1, 0x04]), 8);
        // extra-long: FMT=0x80, [3]=0, len16=0x0102 → 258 + 6 = 264
        assert_eq!(
            Kwp2000Transport::tel_length_bmwfast(&[0x80, 0x12, 0xF1, 0x00, 0x01, 0x02]),
            264
        );
    }

    #[test]
    fn receive_short_positive_response() {
        // ECU→tester: 84 F1 12 5A 80 41 42 | E4  (spec §1.3)
        let mut d = MockDriver::new();
        d.feed(&[0x84, 0xF1, 0x12, 0x5A, 0x80, 0x41, 0x42, 0xE4]);
        let mut t = transport(0x010F, d);
        let resp = t.receive_frame(1000).unwrap();
        // full frame minus CHK
        assert_eq!(resp, &[0x84, 0xF1, 0x12, 0x5A, 0x80, 0x41, 0x42]);
    }

    #[test]
    fn receive_rejects_bad_fmt() {
        let mut d = MockDriver::new();
        d.feed(&[0x00, 0xF1, 0x12, 0x5A, 0x00]);
        let mut t = transport(0x010F, d);
        assert!(t.receive_frame(1000).is_err());
    }

    #[test]
    fn kwp2000s_receive_xor() {
        // 0x010D: fixed header, LEN@3, XOR checksum. B8 F1 12 02 61 0B | CHK
        let frame = [0xB8u8, 0xF1, 0x12, 0x02, 0x61, 0x0B];
        let chk = frame.iter().fold(0u8, |a, &b| a ^ b);
        let mut full = frame.to_vec();
        full.push(chk);
        let mut d = MockDriver::new();
        d.feed(&full);
        let mut t = transport(0x010D, d);
        let resp = t.receive_frame(1000).unwrap();
        assert_eq!(resp, &frame);
    }

    #[test]
    fn detects_nr78_payload_offset() {
        // short-form negative response 83 F1 12 7F 1A 78 → payload at [3]
        let t = transport(0x010F, MockDriver::new());
        let frame = [0x83, 0xF1, 0x12, 0x7F, 0x1A, 0x78];
        let off = t.payload_offset(&frame);
        assert_eq!(off, 3);
        assert_eq!(frame[off], 0x7F);
        assert_eq!(frame[off + 2], 0x78);
    }
}
