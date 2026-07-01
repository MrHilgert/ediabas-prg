// EDIABAS .SIM file parser and ECU simulator.
//
// .SIM format:
//   [REQUEST]
//   job_name = B8,12,F1,01,A2
//   job_name = B8,12,F1,06,23,81,XX,XX,XX,XX   ← XX = wildcard byte
//
//   [RESPONSE]
//   job_name = B8,12,F1,30,...,CHK
use std::collections::HashMap;
use std::path::Path;
use crate::config::CommConfig;
use crate::error::{Error, Result};
use super::Transport;

#[derive(Clone)]
struct SimEntry {
    request:  Vec<BytePattern>,
    response: Vec<u8>,
}

#[derive(Clone)]
enum BytePattern {
    Exact(u8),
    Any,
}

pub struct SimTransport {
    entries: Vec<SimEntry>,
}

impl SimTransport {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(Error::Io)?;
        let mut requests: HashMap<String, Vec<BytePattern>> = HashMap::new();
        let mut responses: HashMap<String, Vec<u8>> = HashMap::new();
        let mut section = "";

        for line in text.lines() {
            let line = line.trim();
            if line.starts_with(';') || line.is_empty() { continue; }
            if line == "[REQUEST]"  { section = "req";  continue; }
            if line == "[RESPONSE]" { section = "resp"; continue; }
            if let Some((name, value)) = line.split_once('=') {
                let name = name.trim().to_lowercase();
                match section {
                    "req"  => { if let Some(p) = parse_pattern(value.trim()) { requests.insert(name, p); } }
                    "resp" => { if let Some(b) = parse_bytes(value.trim())   { responses.insert(name, b); } }
                    _ => {}
                }
            }
        }

        let entries = requests
            .into_iter()
            .filter_map(|(name, request)| {
                let response = responses.remove(&name)?;
                Some(SimEntry { request, response })
            })
            .collect();

        Ok(Self { entries })
    }

    /// Build a SimTransport from inline pairs (for tests / hardcoded scenarios).
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        let entries = pairs
            .iter()
            .filter_map(|(req, resp)| {
                Some(SimEntry {
                    request:  parse_pattern(req)?,
                    response: parse_bytes(resp)?,
                })
            })
            .collect();
        Self { entries }
    }

    fn find_response(&self, frame: &[u8]) -> Option<Vec<u8>> {
        'outer: for entry in &self.entries {
            if entry.request.len() != frame.len() { continue; }
            for (pat, &byte) in entry.request.iter().zip(frame.iter()) {
                match pat {
                    BytePattern::Exact(v) if *v != byte => continue 'outer,
                    _ => {}
                }
            }
            return Some(entry.response.clone());
        }
        None
    }
}

impl Transport for SimTransport {
    fn configure(&mut self, _cfg: &CommConfig) -> Result<()> { Ok(()) }
    fn init_connection(&mut self) -> Result<()> { Ok(()) }
    fn disconnect(&mut self) -> Result<()> { Ok(()) }

    fn exchange(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        self.find_response(frame).ok_or_else(|| {
            Error::Protocol(format!("SimTransport: no response for frame: {}", hex(frame)))
        })
    }
}

/// Transport that always errors — for jobs that never call xsend (INFO, metadata).
pub struct NullTransport;

impl Transport for NullTransport {
    fn configure(&mut self, _cfg: &CommConfig) -> Result<()> { Ok(()) }
    fn init_connection(&mut self) -> Result<()> { Ok(()) }
    fn disconnect(&mut self) -> Result<()> { Ok(()) }

    fn exchange(&mut self, _frame: &[u8]) -> Result<Vec<u8>> {
        Err(Error::NotSupported("NullTransport: no physical transport configured".into()))
    }
}

fn parse_pattern(s: &str) -> Option<Vec<BytePattern>> {
    s.split(',')
        .map(|tok| {
            let tok = tok.trim();
            if tok.eq_ignore_ascii_case("XX") {
                Some(BytePattern::Any)
            } else {
                u8::from_str_radix(tok, 16).ok().map(BytePattern::Exact)
            }
        })
        .collect()
}

fn parse_bytes(s: &str) -> Option<Vec<u8>> {
    s.split(',')
        .map(|tok| u8::from_str_radix(tok.trim(), 16).ok())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        let mut sim = SimTransport::from_pairs(&[
            ("B8,12,F1,01,A2", "B8,F1,12,01,61,98"),
        ]);
        let resp = sim.exchange(&[0xB8, 0x12, 0xF1, 0x01, 0xA2]).unwrap();
        assert_eq!(resp, vec![0xB8, 0xF1, 0x12, 0x01, 0x61, 0x98]);
    }

    #[test]
    fn wildcard_match() {
        let mut sim = SimTransport::from_pairs(&[
            ("B8,12,F1,06,23,81,XX,XX,XX,XX", "B8,F1,12,02,63,42,AA"),
        ]);
        let frame = vec![0xB8, 0x12, 0xF1, 0x06, 0x23, 0x81, 0x00, 0x00, 0x00, 0x01];
        assert!(sim.exchange(&frame).is_ok());
    }

    #[test]
    fn no_match() {
        let mut sim = SimTransport::from_pairs(&[
            ("B8,12,F1,01,A2", "B8,F1,12,01,61,98"),
        ]);
        let result = sim.exchange(&[0xB8, 0x12, 0xF1, 0x01, 0xFF]);
        assert!(result.is_err());
    }
}
