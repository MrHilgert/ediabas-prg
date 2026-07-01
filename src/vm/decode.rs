//! BEST/2 instruction/operand decoding helpers and small byte/string utilities.
//! Pure functions (no VM state) shared by the interpreter in [`super`].

use std::collections::HashMap;

use super::value::{fmt_flt, Value};

/// L0 register id — base of the flat register file (B/I overlap L).
const REG_L0: u8 = 0x18;

/// Read a numeric register value honouring the B/I → L register overlap
/// (byte regs = bytes of L, word regs = words of L, long regs = direct).
pub(crate) fn reg_val_from_map(regs: &HashMap<u8, Value>, reg: u8) -> i32 {
    match reg {
        0x00..=0x0f => {
            let long_reg = REG_L0 + reg / 4;
            let bp = (reg % 4) as u32;
            (regs.get(&long_reg).map(Value::as_long).unwrap_or(0) >> (bp * 8)) & 0xff
        }
        0x10..=0x17 => {
            let idx = reg - 0x10;
            let long_reg = REG_L0 + idx / 2;
            let wp = (idx % 2) as u32;
            (regs.get(&long_reg).map(Value::as_long).unwrap_or(0) >> (wp * 16)) & 0xffff
        }
        _ => regs.get(&reg).map(Value::as_long).unwrap_or(0),
    }
}

/// Read a numeric operand by MODE nibble, returning (value, next_pos).
pub(crate) fn read_long_at(regs: &HashMap<u8, Value>, nibble: u8, code: &[u8], pos: usize) -> (i32, usize) {
    match nibble {
        0 => (0, pos),
        1..=4 => {
            if pos < code.len() {
                (reg_val_from_map(regs, code[pos]), pos + 1)
            } else { (0, pos + 1) }
        }
        5 => if pos < code.len() { (code[pos] as i32, pos + 1) } else { (0, pos + 1) },
        6 => if pos + 1 < code.len() {
            (u16::from_le_bytes([code[pos], code[pos+1]]) as i32, pos + 2)
        } else { (0, pos + 2) },
        7 => if pos + 3 < code.len() {
            (i32::from_le_bytes([code[pos], code[pos+1], code[pos+2], code[pos+3]]), pos + 4)
        } else { (0, pos + 4) },
        8 => {
            if pos + 1 < code.len() {
                let n = code[pos] as usize | ((code[pos+1] as usize) << 8);
                (0, pos + 2 + n)
            } else { (0, pos) }
        }
        // Extended indexed modes — value not evaluated, but advance the correct size.
        0x9 => (0, pos + 3),
        0xa => (0, pos + 2),
        0xb => (0, pos + 4),
        0xc => (0, pos + 5),
        0xd | 0xe => (0, pos + 4),
        _   => (0, pos + 3),
    }
}

/// Read a string operand by MODE nibble, returning (string, next_pos).
pub(crate) fn read_str_at(regs: &HashMap<u8, Value>, nibble: u8, code: &[u8], pos: usize) -> (String, usize) {
    match nibble {
        0 => (String::new(), pos),
        1..=4 => {
            if pos < code.len() {
                let r = code[pos];
                let s = match regs.get(&r) {
                    Some(Value::Str(s)) => s.clone(),
                    Some(Value::Long(v)) => v.to_string(),
                    Some(Value::Data(d)) => blat(d),
                    Some(Value::Float(f)) => fmt_flt(*f),
                    None => String::new(),
                };
                (s, pos + 1)
            } else { (String::new(), pos + 1) }
        }
        5 => if pos < code.len() { ((code[pos] as char).to_string(), pos + 1) } else { (String::new(), pos + 1) },
        6 => if pos + 1 < code.len() { (String::new(), pos + 2) } else { (String::new(), pos + 2) },
        7 => if pos + 3 < code.len() { (String::new(), pos + 4) } else { (String::new(), pos + 4) },
        8 => {
            if pos + 1 < code.len() {
                let n = code[pos] as usize | ((code[pos+1] as usize) << 8);
                let end = (pos + 2 + n).min(code.len());
                let s = cstr(&code[pos+2..end]);
                (s, pos + 2 + n)
            } else { (String::new(), pos) }
        }
        9 | 0xa => (String::new(), pos + 2),
        0xb     => (String::new(), pos + 3),
        _       => (String::new(), pos + 3),
    }
}

/// Operand byte count by MODE nibble (EdiabasLib GetOpArg — authoritative).
pub(crate) fn nibble_size(nibble: u8, code: &[u8], pos: usize) -> usize {
    match nibble {
        0 => 0,
        1|2|3|4|5 => 1,
        6 => 2,
        7 => 4,
        8 => {
            if pos + 1 < code.len() {
                let n = code[pos] as usize | ((code[pos+1] as usize) << 8);
                2 + n
            } else { 2 }
        }
        0x9 => 3,         // IdxImm       reg + u16 idx
        0xa => 2,         // IdxReg       reg + reg
        0xb => 4,         // IdxRegImm    reg + reg + u16 inc
        0xc => 5,         // IdxImmLenImm reg + u16 idx + u16 len
        0xd => 4,         // IdxImmLenReg reg + u16 idx + reg len
        0xe => 4,         // IdxRegLenImm reg + reg + u16 len
        _   => 3,         // IdxRegLenReg reg + reg + reg
    }
}

/// Read an immediate operand (Imm8/16/32 addressing modes 5/6/7) as u32.
pub(crate) fn read_imm_u32(code: &[u8], pos: usize, nib: u8) -> u32 {
    match nib {
        5 => code.get(pos).copied().unwrap_or(0) as u32,
        6 => {
            if pos + 1 < code.len() { u16::from_le_bytes([code[pos], code[pos + 1]]) as u32 }
            else { 0 }
        }
        7 => {
            if pos + 3 < code.len() {
                u32::from_le_bytes([code[pos], code[pos + 1], code[pos + 2], code[pos + 3]])
            } else { 0 }
        }
        _ => 0,
    }
}

/// Operand data length (bytes) by addressing-mode nibble, for numeric operands.
/// RegAB/Imm8 = 1, RegI/Imm16 = 2, RegL/Imm32 = 4; non-numeric modes = 0.
pub(crate) fn nib_width(nib: u8) -> usize {
    match nib {
        2 | 5 => 1,
        3 | 6 => 2,
        4 | 7 => 4,
        _ => 0,
    }
}

/// Skip an instruction using the MODE byte to determine length.
pub(crate) fn skip_instr(code: &[u8], ip: usize) -> usize {
    if ip + 1 >= code.len() { return ip + 1; }
    let mode = code[ip + 1];
    let hi = mode >> 4;
    let lo = mode & 0xf;
    let mut pos = ip + 2;
    pos += nibble_size(hi, code, pos);
    pos += nibble_size(lo, code, pos);
    pos
}

/// Parse a hex string like "B812F104" into a byte vec.
pub(crate) fn parse_hex_str(s: &str) -> Vec<u8> {
    let s = s.trim();
    (0..s.len())
        .step_by(2)
        .filter(|&i| i + 1 < s.len())
        .filter_map(|i| u8::from_str_radix(&s[i..i+2], 16).ok())
        .collect()
}

/// Read a null-terminated Latin-1 string from `buf`.
pub(crate) fn cstr(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    blat(&buf[..end])
}

/// Decode raw bytes into a String Latin-1 style (each byte → one char, 0..=255).
/// Byte-preserving: `unlat(blat(b)) == b`, so binary telegram data survives a trip
/// through string registers without UTF-8 corruption.
pub(crate) fn blat(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Encode a Latin-1 String back to raw bytes (inverse of `blat`).
pub(crate) fn unlat(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u8).collect()
}

/// Space-separated lowercase hex dump.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}
