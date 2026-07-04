use std::io;
use std::path::Path;

const MAGIC: &[u8] = b"@EDIABAS OBJECT\0";
const XOR_KEY: u8 = 0xf7;

// Pointer table at offset 0x78 in the header (8 x u32 LE)
const PTR_CODE: usize = 0x80;     // start of bytecode (always 0xa0)
const PTR_JOBS: usize = 0x88;     // job name table
const PTR_RESULTS: usize = 0x84;  // results/parameters section
const PTR_SGBD: usize = 0x90;     // ECU + full SGBD metadata block

const JOB_ENTRY_SIZE: usize = 68;
const JOB_NAME_LEN: usize = 64; // first 64 bytes = name (null-padded)

fn xor_decode(buf: &[u8]) -> Vec<u8> {
    buf.iter().map(|b| b ^ XOR_KEY).collect()
}

/// Decode bytes as ISO-8859-1 (Latin-1): every byte maps 1:1 to U+0000..=U+00FF,
/// so it is lossless and never yields the U+FFFD replacement char. BMW SGBD text
/// is CP1252/Latin-1 (umlauts ä ö ü ß = 0xE4/0xF6/0xFC/0xDF); `from_utf8_lossy`
/// would destroy them. ASCII is unaffected.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Read a null-terminated string from `section` at `offset`.
/// Returns (string, offset_after_null).
fn read_cstring_at(section: &[u8], offset: usize) -> (String, usize) {
    let slice = section.get(offset..).unwrap_or(&[]);
    let rel_end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let s = latin1(&slice[..rel_end]);
    (s, offset + rel_end + 1)
}

/// Column names are all-uppercase ASCII letters, digits, and underscores.
fn is_col_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn read_cstring(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    latin1(&buf[..end])
}

/// Read a little-endian u32 at `offset`; out-of-bounds yields 0 (never panics).
/// Header pointers read this way are validated in `open`; truncated tables degrade
/// to "no more entries" rather than crashing.
fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    buf.get(offset..offset + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .unwrap_or(0)
}

// Parses key:value metadata lines from PTR_SGBD section.
// The section starts with 4 binary bytes, then XOR'd newline-delimited text.
fn parse_sgbd_fields(data: &[u8], off: usize) -> Vec<(String, String)> {
    if off + 4 >= data.len() {
        return vec![];
    }
    let raw = &data[off + 4..];
    let limit = raw.len().min(512 * 1024);
    let dec = xor_decode(&raw[..limit]);
    let text = latin1(&dec);

    text.split('\n')
        .map(|l| l.trim_end_matches('\r').trim_end_matches('\0'))
        // Keep printable lines (ASCII + Latin-1 letters like umlauts); drop binary
        // junk by rejecting C0/C1 control chars. The key check below is the real guard.
        .filter(|l| l.contains(':') && !l.chars().any(|c| c.is_control()))
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let k = k.trim();
            // Only accept keys that look like identifiers
            if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.is_empty() {
                Some((k.to_string(), v.trim().to_string()))
            } else {
                None
            }
        })
        .collect()
}

#[derive(Debug)]
pub struct JobEntry {
    pub name: String,
    pub code_offset: u32,
}

#[derive(Debug)]
pub struct PrgHeader {
    pub version: u32,
    pub ptr_code: u32,
    pub ptr_jobs: u32,
    pub ptr_results: u32,
    pub ptr_sgbd: u32,
}

/// Parse one table data block (already XOR-decoded): leading uppercase/digit/`_`
/// strings are the column header, then null-separated cells grouped by column
/// count are the rows. Stops at a non-printable byte or the block end.
fn parse_table_block(block: &[u8]) -> Option<Table> {
    let mut offset = 0usize;
    let mut columns: Vec<String> = Vec::new();
    while offset < block.len() {
        let (s, next) = read_cstring_at(block, offset);
        if s.is_empty() || !is_col_name(&s) {
            break;
        }
        columns.push(s);
        offset = next;
    }
    let n_cols = columns.len();
    if n_cols == 0 {
        return None;
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    while offset < block.len() {
        let b = block[offset];
        if b > 127 || (b < 32 && b != 0) {
            break;
        }
        let (s, next) = read_cstring_at(block, offset);
        cur.push(s);
        if cur.len() == n_cols {
            rows.push(std::mem::take(&mut cur));
        }
        offset = next;
    }
    Some(Table { columns, rows })
}

/// Parse a hex code string, tolerating an optional `0x`/`0X` prefix and
/// leading zeros. Returns `None` for non-hex (e.g. free text) so string
/// comparison stays authoritative for text columns.
fn parse_hexish(s: &str) -> Option<u64> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    if t.is_empty() || t.len() > 16 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(t, 16).ok()
}

/// A single SGBD lookup table loaded from the data section.
#[derive(Debug, Default, Clone)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn col_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.eq_ignore_ascii_case(name))
    }

    pub fn find_row(&self, col_idx: usize, value: &str) -> Option<usize> {
        // EDIABAS code tables store keys as `0x1E00` (uppercase, `0x`-prefixed,
        // byte-width zero-padded), while the search key comes from `fix2hex`
        // (bare `1E00`, minimal width). Match case-insensitively first, then fall
        // back to hex-numeric equivalence so `1E00`==`0x1E00` and `500`==`0x0500`.
        let want = parse_hexish(value);
        self.rows.iter().position(|r| {
            r.get(col_idx).map_or(false, |v| {
                v.eq_ignore_ascii_case(value)
                    || matches!((want, parse_hexish(v)), (Some(a), Some(b)) if a == b)
            })
        })
    }

    pub fn cell(&self, row: usize, col: usize) -> &str {
        self.rows.get(row).and_then(|r| r.get(col)).map_or("", |s| s.as_str())
    }
}

/// One measurable channel from the ECU's measurement table (BETRIEBSWTAB on DDE):
/// a mnemonic, its 2C10 selector (PID), unit and description. `MW_SELECT_LESEN_NORM`
/// with `selector` returns the value under `STAT_<name>_WERT`.
#[derive(Debug, Clone)]
pub struct Measurement {
    pub name: String,
    pub selector: String, // 4-hex-digit PID, e.g. "0F65"
    pub unit: String,
    pub label: String,    // long description (LNAME)
    pub fact_a: f64,
    pub fact_b: f64,
}

#[derive(Debug)]
pub struct PrgFile {
    data: Vec<u8>,
    header: PrgHeader,
}

impl PrgFile {
    pub fn open(path: &Path) -> io::Result<Self> {
        let data = std::fs::read(path)?;

        if data.len() < 0xa0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too small"));
        }
        if &data[..MAGIC.len()] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an EDIABAS .prg file (invalid magic)",
            ));
        }

        let version = read_u32_le(&data, 0x10);
        let ptr_code = read_u32_le(&data, PTR_CODE);
        let ptr_jobs = read_u32_le(&data, PTR_JOBS);
        let ptr_results = read_u32_le(&data, PTR_RESULTS);
        let ptr_sgbd = read_u32_le(&data, PTR_SGBD);

        // Header pointers are raw file offsets; a truncated/malformed .prg could point
        // out of bounds. Validate the code→jobs span (sliced in job_code) up front so
        // later access degrades to an error here instead of panicking deep in parsing.
        if ptr_code as usize > ptr_jobs as usize || ptr_jobs as usize > data.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "EDIABAS .prg: code/jobs pointers out of range",
            ));
        }

        Ok(Self { data, header: PrgHeader { version, ptr_code, ptr_jobs, ptr_results, ptr_sgbd } })
    }

    pub fn sgbd_fields(&self) -> Vec<(String, String)> {
        parse_sgbd_fields(&self.data, self.header.ptr_sgbd as usize)
    }

    /// Return the number of jobs (raw, no XOR decode).
    fn job_count(&self) -> usize {
        let off = self.header.ptr_jobs as usize;
        if off + 4 > self.data.len() { return 0; }
        read_u32_le(&self.data, off) as usize
    }

    /// Parse ALL SGBD lookup tables from the data section between the job
    /// entries and the results section.  Tables are returned keyed by their
    /// uppercase name (e.g. "BETRIEBSWTAB").
    ///
    /// Table format (XOR-decoded, from jobs_end+8):
    ///   first K strings (all uppercase + digits + '_') = column header row
    ///   subsequent strings in groups of K            = data rows
    ///
    /// The directory of named tables follows the row data, prefixed with
    /// non-printable bytes.  We stop parsing rows when we encounter a byte
    /// outside the printable-ASCII range.
    pub fn parse_tables(&self) -> std::collections::HashMap<String, Table> {
        let mut map = std::collections::HashMap::new();

        let ptr_jobs   = self.header.ptr_jobs    as usize;
        let dir_start  = self.header.ptr_results as usize; // [0x84] = table directory
        let ptr_sgbd   = self.header.ptr_sgbd    as usize;
        let end_region = ptr_sgbd.min(self.data.len());
        if dir_start <= ptr_jobs || dir_start >= end_region {
            return map;
        }

        let jobs_count = self.job_count();
        let jobs_end   = ptr_jobs + 4 + jobs_count * JOB_ENTRY_SIZE;
        if jobs_end >= dir_start {
            return map;
        }

        // XOR-decode the span holding the table DATA blocks and the DIRECTORY that
        // indexes them, so an absolute file offset `abs` maps to `decoded[abs - jobs_end]`.
        let decoded = xor_decode(&self.data[jobs_end..end_region]);
        let at = |abs: usize| abs - jobs_end;

        // 1) Directory: 80-byte entries — name (32B) @+0x04, data_ptr (abs offset) @+0x44.
        //    (Verified: BETRIEBSWTAB.data_ptr == jobs_end + 8.)
        const ENTRY: usize = 80;
        let mut entries: Vec<(String, usize)> = Vec::new();
        let mut e = dir_start;
        while e + ENTRY <= end_region {
            let base = at(e);
            let name_bytes = &decoded[base + 4..base + 4 + 32];
            let nlen = name_bytes.iter().position(|&b| b == 0).unwrap_or(32);
            let name = &name_bytes[..nlen];
            if name.is_empty() || !name.iter().all(|&b| b > 32 && b < 127) {
                break; // end of directory
            }
            let dp = read_u32_le(&decoded, base + 0x44) as usize;
            let name = String::from_utf8_lossy(name).to_uppercase();
            if dp >= jobs_end && dp < dir_start {
                entries.push((name, dp));
            }
            e += ENTRY;
        }
        if entries.is_empty() {
            return map;
        }

        // 2) Each table's data runs from its data_ptr to the next data_ptr (or the
        //    directory start for the last block) — parse columns then rows per block.
        let mut ptrs: Vec<usize> = entries.iter().map(|(_, p)| *p).collect();
        ptrs.sort_unstable();
        ptrs.dedup();
        for (name, dp) in &entries {
            let end = ptrs.iter().copied().find(|&p| p > *dp).unwrap_or(dir_start);
            if end <= *dp || at(end) > decoded.len() {
                continue;
            }
            if let Some(t) = parse_table_block(&decoded[at(*dp)..at(end)]) {
                map.entry(name.clone()).or_insert(t);
            }
        }

        map
    }

    /// Measurable channels from the SGBD's measurement table (identified by its
    /// NAME + ADR columns, `BETRIEBSWTAB` on DDE). Empty if the SGBD has none.
    pub fn measurements(&self) -> Vec<Measurement> {
        let tables = self.parse_tables();
        let t = tables.values().find(|t| {
            t.col_index("NAME").is_some()
                && t.col_index("ADR").is_some()
                && (t.col_index("MEAS").is_some() || t.col_index("TELEGRAM").is_some())
        });
        let Some(t) = t else { return vec![] };
        let c_name = t.col_index("NAME").unwrap();
        let c_adr = t.col_index("ADR").unwrap();
        let (c_meas, c_ln, c_fa, c_fb) =
            (t.col_index("MEAS"), t.col_index("LNAME"), t.col_index("FACT_A"), t.col_index("FACT_B"));
        let cell = |r: &[String], c: Option<usize>| {
            c.and_then(|i| r.get(i)).map_or(String::new(), |s| s.clone())
        };
        // FACT_A is a multiplier (identity = 1.0), FACT_B an offset (0.0). An empty or
        // unparsable factor must fall back to its IDENTITY, not a blanket 0.0 — a 0.0
        // FACT_A would silently zero out the whole channel (value = raw*0 + b).
        let num = |s: String, default: f64| {
            let t = s.trim().replace(',', ".");
            if t.is_empty() { default } else { t.parse::<f64>().unwrap_or(default) }
        };
        t.rows
            .iter()
            .filter_map(|r| {
                let name = r.get(c_name)?.clone();
                let selector = r
                    .get(c_adr)?
                    .trim()
                    .trim_start_matches("0x")
                    .trim_start_matches("0X")
                    .to_uppercase();
                if name.is_empty() || selector.is_empty() {
                    return None;
                }
                Some(Measurement {
                    name,
                    selector,
                    unit: cell(r, c_meas),
                    label: cell(r, c_ln),
                    fact_a: num(cell(r, c_fa), 1.0),
                    fact_b: num(cell(r, c_fb), 0.0),
                })
            })
            .collect()
    }

    pub fn jobs(&self) -> Vec<JobEntry> {
        let off = self.header.ptr_jobs as usize;
        if off + 4 > self.data.len() {
            return vec![];
        }
        let count = read_u32_le(&self.data, off) as usize;
        let entries_start = off + 4;

        (0..count)
            .filter_map(|i| {
                let entry_off = entries_start + i * JOB_ENTRY_SIZE;
                if entry_off + JOB_ENTRY_SIZE > self.data.len() {
                    return None;
                }
                let raw = &self.data[entry_off..entry_off + JOB_ENTRY_SIZE];
                let dec = xor_decode(raw);
                let name = read_cstring(&dec[..JOB_NAME_LEN]);
                let code_offset = read_u32_le(&dec, JOB_NAME_LEN);
                Some(JobEntry { name, code_offset })
            })
            .collect()
    }

    pub fn print_info(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("=== EDIABAS .prg file info ===");
        println!("Format version : {}", self.header.version);
        println!("Code start     : 0x{:08x}", self.header.ptr_code);
        println!("Job table      : 0x{:08x}", self.header.ptr_jobs);

        let top_keys = ["ECU", "ORIGIN", "REVISION", "AUTHOR", "ECUCOMMENT"];
        let stop_keys = ["JOBNAME", "RESULT", "JOBCOMMENT"];
        for (k, v) in self.sgbd_fields() {
            if stop_keys.contains(&k.as_str()) {
                break;
            }
            if top_keys.contains(&k.as_str()) {
                println!("{:<15}: {}", k, v);
            }
        }

        println!("Job count      : {}", self.jobs().len());
        Ok(())
    }

    pub fn print_jobs(&self) -> Result<(), Box<dyn std::error::Error>> {
        let jobs = self.jobs();
        println!("{:<40} {}", "Job name", "Code offset");
        println!("{}", "-".repeat(55));
        for job in &jobs {
            println!("{:<40} 0x{:08x}", job.name, job.code_offset);
        }
        println!("\nTotal: {} jobs", jobs.len());
        Ok(())
    }

    /// Return XOR-decoded bytecode for the named job (case-insensitive).
    /// Slices from the job's code offset to the next job's offset (or end of
    /// code section). Trailing 0xf7 padding bytes are included; the VM stops
    /// when it hits one.
    pub fn job_code(&self, name: &str) -> Option<Vec<u8>> {
        let ptr_code = self.header.ptr_code as usize;
        let ptr_jobs = self.header.ptr_jobs as usize;
        // Validated in `open`, but stay defensive: a bad span yields None, not a panic.
        let code_raw = self.data.get(ptr_code..ptr_jobs)?;

        // Build sorted list of (code_offset, name) pairs
        let mut entries: Vec<(usize, String)> = self
            .jobs()
            .into_iter()
            .map(|j| (j.code_offset as usize, j.name))
            .collect();
        entries.sort_by_key(|(off, _)| *off);

        let idx = entries
            .iter()
            .position(|(_, n)| n.eq_ignore_ascii_case(name))?;

        // A job whose code_offset is below ptr_code (corrupt table) → None, not underflow.
        let start = entries[idx].0.checked_sub(ptr_code)?;
        let end = entries
            .get(idx + 1)
            .and_then(|(off, _)| off.checked_sub(ptr_code))
            .unwrap_or(code_raw.len());

        Some(xor_decode(code_raw.get(start..end)?))
    }

    /// Statically determine the ECU's communication concept WITHOUT any I/O.
    ///
    /// The `INITIALISIERUNG` job configures the transport by executing `xsetpar`
    /// with an 18-byte CommParameter block. That block carries the concept
    /// (0x0006 DS2, 0x0110 D-CAN, 0x010F BMW-FAST, …), baud and timeouts — so we
    /// can pick the right transport before ever touching the bus. We scan the
    /// (already XOR-decoded) INITIALISIERUNG bytecode for the first
    /// `xsetpar` opcode (`28 80 <len_lo> <len_hi> <payload>`) and parse its payload.
    ///
    /// Returns `None` if there is no INITIALISIERUNG job or no `xsetpar` in it
    /// (caller should then fall back to the DS2 default).
    pub fn initial_comm_config(&self) -> Option<crate::config::CommConfig> {
        let code = self.job_code("INITIALISIERUNG")?;
        let mut ip = 0usize;
        while ip + 4 <= code.len() {
            // xsetpar: 28 80 <n_lo> <n_hi> <n bytes CommParameter>
            if code[ip] == 0x28 && code[ip + 1] == 0x80 {
                let n = (code[ip + 2] as usize) | ((code[ip + 3] as usize) << 8);
                if n >= 18 && ip + 4 + n <= code.len() {
                    if let Some(cfg) = crate::config::CommConfig::parse(&code[ip + 4..ip + 4 + n]) {
                        return Some(cfg);
                    }
                }
            }
            ip += 1;
        }
        None
    }
}
