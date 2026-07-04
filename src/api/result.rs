//! Structured job results returned by [`crate::Session`].

use std::collections::HashMap;

use crate::vm::Value;

/// All result sets produced by one job run (EDIABAS "Sätze"). The system set
/// carries `JOB_STATUS`; data sets carry the measured/decoded results.
#[derive(Debug, Clone, Default)]
pub struct JobResult {
    sets: Vec<ResultSet>,
    /// True if the job actually received a real bus response (see [`Vm::got_response`]).
    /// A presence probe must check this, not just `JOB_STATUS`.
    comm_ok: bool,
}

impl JobResult {
    pub(crate) fn new(sets: Vec<ResultSet>) -> Self {
        JobResult { sets, comm_ok: false }
    }

    /// Record whether the run reached the ECU (a real telegram round-trip happened).
    pub(crate) fn with_comm(mut self, comm_ok: bool) -> Self {
        self.comm_ok = comm_ok;
        self
    }

    /// True if the job actually got a response from the ECU. Unlike `JOB_STATUS=OKAY`
    /// (which a job may set even when every send timed out), this reflects a real link.
    pub fn comm_ok(&self) -> bool {
        self.comm_ok
    }

    /// All result sets, in order.
    pub fn sets(&self) -> &[ResultSet] {
        &self.sets
    }

    /// Append another result's sets (used to merge several polled jobs into one view).
    pub fn extend(&mut self, other: JobResult) {
        self.comm_ok |= other.comm_ok; // any real response counts as a live link
        self.sets.extend(other.sets);
    }

    /// Overlay another result's values onto this one — **newest values win**, collapsing to
    /// a single set. Used to hold last-known values across streaming polls: a name missing
    /// from `other` keeps its previous value, and repeated polls don't grow the set list.
    pub fn overlay(&mut self, other: JobResult) {
        self.comm_ok |= other.comm_ok;
        let mut flat: HashMap<String, Value> = HashMap::new();
        for set in self.sets.drain(..) {
            flat.extend(set.map);
        }
        for set in other.sets {
            flat.extend(set.map); // later inserts overwrite → newest wins
        }
        self.sets = vec![ResultSet { map: flat }];
    }

    /// Iterate over the result sets.
    pub fn iter(&self) -> std::slice::Iter<'_, ResultSet> {
        self.sets.iter()
    }

    /// True if no set contains any result.
    pub fn is_empty(&self) -> bool {
        self.sets.iter().all(ResultSet::is_empty)
    }

    /// The `JOB_STATUS` string (usually `"OKAY"` on success), across all sets.
    pub fn job_status(&self) -> Option<String> {
        self.get_str("JOB_STATUS")
    }

    /// First result named `name` across all sets, as a raw [`Value`].
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.sets.iter().find_map(|s| s.get(name))
    }

    /// First result named `name` as an integer.
    pub fn get_i64(&self, name: &str) -> Option<i64> {
        self.sets.iter().find_map(|s| s.get_i64(name))
    }

    /// First result named `name` as a float.
    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.sets.iter().find_map(|s| s.get_f64(name))
    }

    /// First result named `name` as a display string.
    pub fn get_str(&self, name: &str) -> Option<String> {
        self.sets.iter().find_map(|s| s.get_str(name))
    }

    /// First result named `name` as raw bytes.
    pub fn get_bytes(&self, name: &str) -> Option<Vec<u8>> {
        self.sets.iter().find_map(|s| s.get_bytes(name))
    }
}

impl<'a> IntoIterator for &'a JobResult {
    type Item = &'a ResultSet;
    type IntoIter = std::slice::Iter<'a, ResultSet>;
    fn into_iter(self) -> Self::IntoIter {
        self.sets.iter()
    }
}

/// One EDIABAS result set (Satz): named results mapped to [`Value`]s.
#[derive(Debug, Clone, Default)]
pub struct ResultSet {
    map: HashMap<String, Value>,
}

impl ResultSet {
    pub(crate) fn from_map(map: HashMap<String, Value>) -> Self {
        ResultSet { map }
    }

    /// Raw value of result `name`. Matching is **case-insensitive**, as in EDIABAS:
    /// the SGBD (`.prg`) defines results in upper-case, but callers (`.ipo` screens,
    /// jobs) may reference them in any case (e.g. `aif_fg_nr` vs `AIF_FG_NR`).
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.map.get(name).or_else(|| {
            self.map.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v)
        })
    }

    /// Whether this set contains a result named `name` (case-insensitive, as EDIABAS).
    pub fn contains(&self, name: &str) -> bool {
        self.map.contains_key(name) || self.map.keys().any(|k| k.eq_ignore_ascii_case(name))
    }

    /// Number of results in the set.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the set has no results.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Result names in this set.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    /// Iterate over `(name, value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Result `name` as an integer (parses strings, truncates floats). Returns `None`
    /// when the value is binary or an unparsable string — NOT a silent `0`, so a real
    /// zero is distinguishable from "no numeric value" (and `JobResult` search skips to
    /// the next set instead of stopping on a non-numeric hit).
    pub fn get_i64(&self, name: &str) -> Option<i64> {
        self.get(name).and_then(|v| match v {
            Value::Long(n) => Some(*n as i64),
            Value::Float(f) => Some(*f as i64),
            Value::Str(s) => s.trim().parse().ok(),
            Value::Data(_) => None,
        })
    }

    /// Result `name` as a float (parses strings; accepts `,` or `.` decimals). Returns
    /// `None` for binary/unparsable values rather than a silent `0.0`.
    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.get(name).and_then(|v| match v {
            Value::Float(f) => Some(*f),
            Value::Long(n) => Some(*n as f64),
            Value::Str(s) => s.trim().replace(',', ".").parse().ok(),
            Value::Data(_) => None,
        })
    }

    /// Result `name` as a display string.
    pub fn get_str(&self, name: &str) -> Option<String> {
        self.get(name).map(|v| v.to_string())
    }

    /// Result `name` as raw bytes (for binary results; others are stringified).
    pub fn get_bytes(&self, name: &str) -> Option<Vec<u8>> {
        self.get(name).map(|v| match v {
            Value::Data(d) => d.clone(),
            other => other.to_string().into_bytes(),
        })
    }
}
