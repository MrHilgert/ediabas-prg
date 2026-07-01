/// EDIABAS CommParameter protocol concept codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Ds2,        // concepts 0x0001, 0x0005, 0x0006
    Kwp1281,    // concepts 0x0002, 0x0003
    Kwp2000Bmw, // concept  0x010C
    BmwFast,    // concept  0x010F
    DCan,       // concept  0x0110
}

/// Full set of EDIABAS CommParameter values derived from xconnect / INITIALISIERUNG.
#[derive(Debug, Clone)]
pub struct CommConfig {
    pub protocol:       Protocol,
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
}

impl Default for CommConfig {
    fn default() -> Self {
        Self {
            protocol:       Protocol::Ds2,
            baud:           9600,
            parity:         serialport::Parity::Even,
            len_offset:     1,
            len_add:        0,
            timeout_std_ms: 2000,
            regen_time_ms:  20,
            timeout_tel_ms: 50,
            interbyte_ms:   5,
            wake_addr:      None,
        }
    }
}
