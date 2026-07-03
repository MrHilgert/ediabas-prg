/// EDIABAS CommParameter protocol concept codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Ds2,        // concepts 0x0001, 0x0005, 0x0006
    Kwp1281,    // concepts 0x0002, 0x0003
    Kwp2000Bmw, // concepts 0x010C, 0x010D
    BmwFast,    // concept  0x010F
    DCan,       // concept  0x0110
}

impl Protocol {
    /// Map an EDIABAS CommParameter *concept* code to the transport family that
    /// implements it. Concepts we don't recognise fall back to DS2 (the classic
    /// BMW K-line framing) so an unknown-but-K-line ECU still gets a best effort.
    pub fn from_concept(concept: u16) -> Protocol {
        match concept {
            0x0001 | 0x0005 | 0x0006 => Protocol::Ds2,
            0x0002 | 0x0003          => Protocol::Kwp1281,
            0x010C | 0x010D          => Protocol::Kwp2000Bmw,
            0x010F                   => Protocol::BmwFast,
            0x0110                   => Protocol::DCan,
            _                        => Protocol::Ds2,
        }
    }

    /// Physical layer this protocol rides on: `true` = raw CAN (D-CAN), `false` =
    /// serial K-line (DS2, KWP1281, KWP2000, BMW-FAST). Drives adapter setup.
    pub fn is_can(self) -> bool {
        matches!(self, Protocol::DCan)
    }
}

/// Full set of EDIABAS CommParameter values derived from xconnect / INITIALISIERUNG.
#[derive(Debug, Clone)]
pub struct CommConfig {
    pub protocol:       Protocol,
    /// Raw EDIABAS concept code from CommParameter[0] (0x0006, 0x0110, …). Kept so a
    /// transport can distinguish sub-variants within one `Protocol` family.
    pub concept:        u16,
    pub baud:           u32,
    pub parity:         serialport::Parity,
    /// Index of the LEN byte in ECU response frame: 1=concept-0x0006, 2=concept-0x0001
    pub len_offset:     usize,
    /// Additive constant from CommAnswerLen[1]: total_frame = LEN_byte + len_add.
    /// 0 → LEN is total frame length (old-style ECUs).
    /// 5 → LEN is payload count, total = LEN + 5 (4-byte BMW DS2 header).
    pub len_add:        usize,
    pub timeout_std_ms: u64,
    pub regen_time_ms:  u64,
    pub timeout_tel_ms: u64,
    /// Inter-byte TX delay (ms). DDE4.0 and classic DS2 ECUs ignore frames sent
    /// without ~4-5 ms spacing between bytes (ParInterbyteTime). 0 = send at once.
    pub interbyte_ms:   u64,
    pub wake_addr:      Option<u8>,
    /// KWP2000/BMW-FAST: how many times to keep waiting on a `7F .. 78`
    /// (responsePending) reply before giving up (ParRetryNr78).
    pub retry_nr78:     u32,
    /// KWP2000/BMW-FAST: first-byte timeout while a `7F .. 78` is pending (P2* extended).
    pub timeout_nr78_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an 18-byte CommParameter block with a given concept + baud (LE u16s).
    fn block(concept: u16, baud: u16) -> Vec<u8> {
        let mut p = vec![0u8; 18];
        p[0..2].copy_from_slice(&concept.to_le_bytes());
        p[2..4].copy_from_slice(&baud.to_le_bytes());
        p
    }

    #[test]
    fn concept_maps_to_protocol() {
        assert_eq!(Protocol::from_concept(0x0006), Protocol::Ds2);
        assert_eq!(Protocol::from_concept(0x0001), Protocol::Ds2);
        assert_eq!(Protocol::from_concept(0x0002), Protocol::Kwp1281);
        assert_eq!(Protocol::from_concept(0x010C), Protocol::Kwp2000Bmw);
        assert_eq!(Protocol::from_concept(0x010D), Protocol::Kwp2000Bmw);
        assert_eq!(Protocol::from_concept(0x010F), Protocol::BmwFast);
        assert_eq!(Protocol::from_concept(0x0110), Protocol::DCan);
        assert!(Protocol::DCan.is_can());
        assert!(!Protocol::Ds2.is_can());
    }

    #[test]
    fn parse_ds2_concept6() {
        let cfg = CommConfig::parse(&block(0x0006, 9600)).unwrap();
        assert_eq!(cfg.protocol, Protocol::Ds2);
        assert_eq!(cfg.concept, 0x0006);
        assert_eq!(cfg.baud, 9600);
        assert_eq!(cfg.parity, serialport::Parity::Even);
        assert_eq!((cfg.len_offset, cfg.len_add), (3, 5)); // 4-byte BMW header
    }

    #[test]
    fn parse_dcan_concept() {
        let cfg = CommConfig::parse(&block(0x0110, 500)).unwrap();
        assert_eq!(cfg.protocol, Protocol::DCan);
        assert_eq!(cfg.parity, serialport::Parity::None);
    }

    #[test]
    fn parse_rejects_short_block() {
        assert!(CommConfig::parse(&[0u8; 4]).is_none());
    }
}

impl CommConfig {
    /// Parse an EDIABAS 18-byte CommParameter block (as passed to `xsetpar`) into
    /// a `CommConfig`. Layout (9× u16 LE):
    ///   [0] concept  [1] baud  [2] ecu_type  [3] -  [4] -  [5] ParTimeoutStd
    ///   [6] ParRegenTime  [7] ParTimeoutTelEnd  [8] ParInterbyteTime
    ///
    /// Field [8] is the inter-byte TX time (KWP2000 P4). Verified across the corpus:
    /// it is 0 for most concept-6 ECUs and non-zero (2..5) only for slow ones like
    /// DDE4.0 (=4). Returns `None` if the block is too short to hold a concept.
    pub fn parse(p: &[u8]) -> Option<CommConfig> {
        if p.len() < 18 {
            return None;
        }
        // The concept's low 16 bits sit at byte 0 in BOTH element encodings, so we can
        // read it before deciding the element width.
        let concept = u16::from_le_bytes([p[0], p[1]]);
        // EDIABAS's classic concepts (< 0x0100: DS2, KWP1281) store CommParameter as
        // little-endian u16 elements; the 32-bit concepts (>= 0x0100: KWP2000-BMW,
        // BMW-FAST, D-CAN) store them as u32 elements. Verified on the corpus:
        // a 0x010C block is 136 bytes (34× u32); a DS2 block is 18 bytes (9× u16).
        if concept >= 0x0100 {
            CommConfig::parse_u32(concept, p)
        } else {
            CommConfig::parse_u16(concept, p)
        }
    }

    /// Classic u16-element CommParameter (DS2, KWP1281).
    fn parse_u16(concept: u16, p: &[u8]) -> Option<CommConfig> {
        let e = |i: usize| -> u16 {
            let off = i * 2;
            if off + 1 < p.len() { u16::from_le_bytes([p[off], p[off + 1]]) } else { 0 }
        };
        let baud           = e(1) as u32;
        let timeout_std_ms = e(5) as u64;
        let regen_time_ms  = e(6) as u64;
        let timeout_tel_ms = e(7) as u64;
        let interbyte_ms   = e(8) as u64;

        // len_offset / len_add follow the DS2 frame concept (xawlen may refine later).
        // concept 0x0006 = BMW 4-byte header [FMT][TGT][SRC][LEN] → offset 3, add 5.
        // concept 0x0001/0x0005 = 2-byte DS2 [ADDR][LEN] → offset 1, add 0.
        let (len_offset, len_add) = match concept {
            0x0006 => (3, 5),
            _      => (1, 0),
        };

        let protocol = Protocol::from_concept(concept);
        let parity = match protocol {
            Protocol::Ds2 => serialport::Parity::Even,
            _             => serialport::Parity::None, // KWP1281 = 8N1
        };

        let mut cfg = CommConfig::default();
        cfg.protocol       = protocol;
        cfg.concept        = concept;
        cfg.baud           = if baud > 0 { baud } else { cfg.baud };
        cfg.parity         = parity;
        cfg.len_offset     = len_offset;
        cfg.len_add        = len_add;
        cfg.timeout_std_ms = if timeout_std_ms > 0 { timeout_std_ms } else { 2000 };
        cfg.regen_time_ms  = if regen_time_ms  > 0 { regen_time_ms  } else { 20 };
        cfg.timeout_tel_ms = if timeout_tel_ms > 0 { timeout_tel_ms } else { 50 };
        cfg.interbyte_ms   = interbyte_ms;
        // Element [2] is the ECU/wake address (e.g. 0xB8 on DDE). KWP1281's 5-baud
        // init needs it; DS2 ignores it.
        cfg.wake_addr      = match e(2) { 0 => None, a => Some(a as u8) };
        Some(cfg)
    }

    /// 32-bit-concept CommParameter (KWP2000-BMW 0x010C/D, BMW-FAST 0x010F, D-CAN 0x0110).
    /// Element layout (u32, per EdiabasLib + `docs/PROTO-KWP2000.md` §7):
    ///   [0]=concept [1]=baud [2]=TimeoutStd [3]=RegenTime [4]=TimeoutTelEnd
    ///   0x010F/0x0110:  [5]=RetryNr78 [6]=TimeoutNr78
    ///   0x010C/0x010D:  [5]=InterbyteTime [6]=RetryNr78 [7]=TimeoutNr78
    fn parse_u32(concept: u16, p: &[u8]) -> Option<CommConfig> {
        let e = |i: usize| -> u32 {
            let off = i * 4;
            if off + 3 < p.len() {
                u32::from_le_bytes([p[off], p[off + 1], p[off + 2], p[off + 3]])
            } else {
                0
            }
        };
        let baud           = e(1);
        let timeout_std_ms = e(2) as u64;
        let regen_time_ms  = e(3) as u64;
        let timeout_tel_ms = e(4) as u64;

        let has_interbyte = matches!(concept, 0x010C | 0x010D);
        let interbyte_ms  = if has_interbyte { e(5) as u64 } else { 0 };
        let nr78_base     = if has_interbyte { 6 } else { 5 };
        let retry_nr78    = e(nr78_base);
        let timeout_nr78  = e(nr78_base + 1) as u64;

        // 0x010D ("KWP2000*") is the XOR + Even-parity variant with a fixed 4-byte
        // header (LEN@3); everything else in this group is BMW-FAST framing (None parity).
        let parity = if concept == 0x010D {
            serialport::Parity::Even
        } else {
            serialport::Parity::None
        };
        let (len_offset, len_add) = if concept == 0x010D { (3, 5) } else { (0, 0) };

        let mut cfg = CommConfig::default();
        cfg.protocol        = Protocol::from_concept(concept);
        cfg.concept         = concept;
        cfg.baud            = if baud > 0 { baud } else { cfg.baud };
        cfg.parity          = parity;
        cfg.len_offset      = len_offset;
        cfg.len_add         = len_add;
        cfg.timeout_std_ms  = if timeout_std_ms > 0 { timeout_std_ms } else { 2000 };
        cfg.regen_time_ms   = if regen_time_ms  > 0 { regen_time_ms  } else { 20 };
        cfg.timeout_tel_ms  = if timeout_tel_ms > 0 { timeout_tel_ms } else { 20 };
        cfg.interbyte_ms    = interbyte_ms;
        cfg.retry_nr78      = if retry_nr78 > 0 && retry_nr78 < 100 { retry_nr78 } else { 3 };
        cfg.timeout_nr78_ms = if timeout_nr78 > 0 { timeout_nr78 } else { 3000 };
        Some(cfg)
    }
}

impl Default for CommConfig {
    fn default() -> Self {
        Self {
            protocol:       Protocol::Ds2,
            concept:        0x0006,
            baud:           9600,
            parity:         serialport::Parity::Even,
            len_offset:     1,
            len_add:        0,
            timeout_std_ms: 2000,
            regen_time_ms:  20,
            timeout_tel_ms: 50,
            interbyte_ms:   5,
            wake_addr:      None,
            retry_nr78:     3,
            timeout_nr78_ms: 3000,
        }
    }
}
