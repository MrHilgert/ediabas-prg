//! The BEST/2 register/result value type and its number formatting.

use super::decode::hex;

/// A value held in a BEST/2 register or emitted as a job result.
#[derive(Debug, Clone)]
pub enum Value {
    Long(i32),
    Str(String),
    Data(Vec<u8>),
    /// BEST/2 float register (stored in S-registers). 64-bit double.
    Float(f64),
}

impl Value {
    pub fn as_long(&self) -> i32 {
        match self {
            Value::Long(v) => *v,
            Value::Str(s) => s.trim().parse().unwrap_or(0),
            Value::Float(f) => *f as i32,
            Value::Data(_) => 0,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Value::Str(s) => s,
            _ => "",
        }
    }

    pub fn as_data(&self) -> &[u8] {
        match self {
            Value::Data(d) => d,
            Value::Str(s) => s.as_bytes(),
            Value::Long(_) | Value::Float(_) => &[],
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Long(v) => write!(f, "{v}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Data(d) => write!(f, "{}", hex(d)),
            Value::Float(v) => write!(f, "{}", fmt_flt(*v)),
        }
    }
}

/// Parse an ASCII number (BEST/2 a2flt source) to f64. Accepts ',' or '.' decimals.
pub(crate) fn parse_f64(s: &str) -> f64 {
    s.trim().replace(',', ".").parse().unwrap_or(0.0)
}

/// Format a float the way EDIABAS results read: drop trailing ".0" for integers.
pub(crate) fn fmt_flt(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.6}", v);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.to_string()
    }
}
