//! `.ipo` container: fingerprint + flat record stream.
//!
//! ```text
//! file   := 05 00 "TEST-Infotext" 0a  record*  [trailing 0a]
//! record := tag(1) str1 0a index(u32 LE) str2 0a str3 0a pad(00) count(u16 LE) payload
//! ```
//! Record tags: 01 SCREEN(`s_*`) 02 MENU(`m_*`) 03 STATEMACHINE(`sm_*`) 05 PROC
//! 11 GLOBALS(count type-bytes) 12 CONSTS(count pool entries) 21 screen-code
//! 22 LINE(str2=labels, str3=`;`-result-list) 23 spacer 24 ITEM(index>>16=item#)
//! 25 STATE(`Z_*`). Everything except 11/12 carries a `count`-instruction code body.

use crate::strings::decode_cp1251;
use crate::Error;

/// Record tag constants.
pub mod tag {
    pub const SCREEN: u8 = 0x01;
    pub const MENU: u8 = 0x02;
    pub const STATEMACHINE: u8 = 0x03;
    pub const PROC: u8 = 0x05;
    pub const GLOBALS: u8 = 0x11;
    pub const CONSTS: u8 = 0x12;
    pub const CODE: u8 = 0x21;
    pub const LINE: u8 = 0x22;
    pub const SPACER: u8 = 0x23;
    pub const ITEM: u8 = 0x24;
    pub const STATE: u8 = 0x25;
}

/// A constant-pool entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Bool(bool),
    Byte(u8),
    Int(i16),
    Long(i32),
    Real(f64),
    Str(String),
}

impl Const {
    /// Borrow the string payload, if this const is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Const::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// The tag-specific payload of a record.
#[derive(Debug, Clone)]
pub enum Body {
    /// Tag 0x11 — one type byte per global slot.
    Globals(Vec<u8>),
    /// Tag 0x12 — the constant pool.
    Consts(Vec<Const>),
    /// Any code-bearing record — raw bytes, `count * 4` long (decode via `opcode`).
    Code(Vec<u8>),
}

/// One parsed record.
#[derive(Debug, Clone)]
pub struct Record {
    pub tag: u8,
    /// str1 — node/proc name (`s_*`, `m_*`, `inpainit`, …). Empty for ITEM/LINE.
    pub name: String,
    /// str2 — for LINE = comma-joined column labels; for ITEM = the menu label.
    pub str2: String,
    /// str3 — for LINE = `;`-joined result names (co-located bindings).
    pub str3: String,
    pub pad: u8,
    /// u32 index. For ITEM, `index >> 16` is the item/F-key number.
    pub index: u32,
    pub count: u16,
    /// Byte offset of the record start (for diagnostics).
    pub off: usize,
    pub body: Body,
}

impl Record {
    /// The item/F-key number of an ITEM record (`index >> 16`).
    pub fn item_no(&self) -> u32 {
        self.index >> 16
    }

    /// Borrow the raw code bytes, if this is a code-bearing record.
    pub fn code(&self) -> Option<&[u8]> {
        match &self.body {
            Body::Code(c) => Some(c),
            _ => None,
        }
    }

    /// Borrow the constant pool, if this is the CONSTS record.
    pub fn consts(&self) -> Option<&[Const]> {
        match &self.body {
            Body::Consts(c) => Some(c),
            _ => None,
        }
    }
}

/// A fully parsed `.ipo` module.
#[derive(Debug, Clone)]
pub struct Module {
    pub title: String,
    pub records: Vec<Record>,
    pub size: usize,
}

impl Module {
    /// The module's single constant pool (tag 0x12), if present.
    pub fn const_pool(&self) -> Option<&[Const]> {
        self.records.iter().find_map(|r| r.consts())
    }
}

/// True if `data` begins with the INPA screen-script fingerprint.
pub fn has_fingerprint(data: &[u8]) -> bool {
    data.starts_with(b"\x05\x00TEST-Infotext")
}

/// Read a CP1251, `0x0a`-terminated string starting at `i`; returns (string, next).
fn read_str(data: &[u8], i: usize) -> Result<(String, usize), Error> {
    if i > data.len() {
        return Err(Error::UnterminatedString(i));
    }
    match data[i..].iter().position(|&b| b == 0x0A) {
        Some(rel) => {
            let j = i + rel;
            Ok((decode_cp1251(&data[i..j]), j + 1))
        }
        None => Err(Error::UnterminatedString(i)),
    }
}

fn parse_consts(data: &[u8], mut k: usize, count: usize) -> Result<(Vec<Const>, usize), Error> {
    let n = data.len();
    let mut consts = Vec::with_capacity(count);
    for _ in 0..count {
        if k >= n {
            return Err(Error::Overrun("const pool"));
        }
        let t = data[k];
        match t {
            0x06 => {
                let (s, next) = read_str(data, k + 1)?;
                consts.push(Const::Str(s));
                k = next;
            }
            0x03 => {
                if k + 3 > n {
                    return Err(Error::Overrun("const int"));
                }
                let v = i16::from_le_bytes([data[k + 1], data[k + 2]]);
                consts.push(Const::Int(v));
                k += 3;
            }
            0x02 => {
                if k + 2 > n {
                    return Err(Error::Overrun("const byte"));
                }
                consts.push(Const::Byte(data[k + 1]));
                k += 2;
            }
            0x01 => {
                if k + 2 > n {
                    return Err(Error::Overrun("const bool"));
                }
                consts.push(Const::Bool(data[k + 1] != 0));
                k += 2;
            }
            0x04 => {
                if k + 5 > n {
                    return Err(Error::Overrun("const long"));
                }
                let v = i32::from_le_bytes([data[k + 1], data[k + 2], data[k + 3], data[k + 4]]);
                consts.push(Const::Long(v));
                k += 5;
            }
            0x05 => {
                if k + 9 > n {
                    return Err(Error::Overrun("const real"));
                }
                let mut b = [0u8; 8];
                b.copy_from_slice(&data[k + 1..k + 9]);
                consts.push(Const::Real(f64::from_le_bytes(b)));
                k += 9;
            }
            other => return Err(Error::UnknownConstTag(other, k)),
        }
    }
    Ok((consts, k))
}

/// Parse a whole `.ipo` file into a [`Module`]. Deterministic: pure function of `data`.
pub fn parse_module(data: &[u8]) -> Result<Module, Error> {
    if !data.starts_with(&[0x05, 0x00]) {
        return Err(Error::NoFingerprint);
    }
    let (title, mut i) = read_str(data, 2)?;
    let n = data.len();
    let mut records = Vec::new();

    while i < n {
        // Allow trailing 0x0a filler at EOF.
        if data[i] == 0x0A && n - i <= 2 {
            break;
        }
        let tag = data[i];
        let (name, j) = read_str(data, i + 1)?;
        if j + 4 > n {
            return Err(Error::Truncated(i));
        }
        let index = u32::from_le_bytes([data[j], data[j + 1], data[j + 2], data[j + 3]]);
        let (str2, j2) = read_str(data, j + 4)?;
        let (str3, j3) = read_str(data, j2)?;
        if j3 + 3 > n {
            return Err(Error::Truncated(i));
        }
        let pad = data[j3];
        let count = u16::from_le_bytes([data[j3 + 1], data[j3 + 2]]);
        let body_at = j3 + 3;

        let (body, next) = match tag {
            tag::GLOBALS => {
                let end = body_at + count as usize;
                if end > n {
                    return Err(Error::Overrun("globals"));
                }
                (Body::Globals(data[body_at..end].to_vec()), end)
            }
            tag::CONSTS => {
                let (consts, k) = parse_consts(data, body_at, count as usize)?;
                (Body::Consts(consts), k)
            }
            _ => {
                let end = body_at + count as usize * 4;
                if end > n {
                    return Err(Error::Overrun("code"));
                }
                (Body::Code(data[body_at..end].to_vec()), end)
            }
        };
        records.push(Record { tag, name, str2, str3, pad, index, count, off: i, body });
        i = next;
    }

    Ok(Module { title, records, size: n })
}
