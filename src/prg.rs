use std::io;
use std::path::Path;

const MAGIC: &[u8] = b"@EDIABAS OBJECT\0";
const XOR_KEY: u8 = 0xf7;

// Pointer table at offset 0x78 in the header (8 x u32 LE)
const _PTR_SIG: usize = 0x78;     // signature/checksum section
const _PTR_JOB2: usize = 0x7c;    // secondary job-table reference
const PTR_CODE: usize = 0x80;     // start of bytecode (always 0xa0)
const _PTR_RESULTS: usize = 0x84; // result/parameter table
const PTR_JOBS: usize = 0x88;     // job name table
const _PTR_PARAMS: usize = 0x8c;  // parameter table
const PTR_RESULTS: usize = 0x84;  // results/parameters section
const PTR_SGBD: usize = 0x90;     // ECU + full SGBD metadata block
const _PTR_VARS: usize = 0x94;    // variable/register table (binary)

const JOB_ENTRY_SIZE: usize = 68;
const JOB_NAME_LEN: usize = 64; // first 64 bytes = name (null-padded)

fn xor_decode(buf: &[u8]) -> Vec<u8> {
    buf.iter().map(|b| b ^ XOR_KEY).collect()
}

/// Read a null-terminated string from `section` at `offset`.
/// Returns (string, offset_after_null).
fn read_cstring_at(section: &[u8], offset: usize) -> (String, usize) {
    let end = section[offset..].iter().position(|&b| b == 0)
        .map(|i| offset + i)
        .unwrap_or(section.len());
    let s = String::from_utf8_lossy(&section[offset..end]).into_owned();
    (s, end + 1)
}

/// Column names are all-uppercase ASCII letters, digits, and underscores.
fn is_col_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn read_cstring(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap())
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
    let text = String::from_utf8_lossy(&dec);

    text.split('\n')
        .map(|l| l.trim_end_matches('\r').trim_end_matches('\0'))
        .filter(|l| l.contains(':') && l.chars().all(|c| c.is_ascii()))
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
        self.rows.iter().position(|r| {
            r.get(col_idx).map_or(false, |v| v.eq_ignore_ascii_case(value))
        })
    }

    pub fn cell(&self, row: usize, col: usize) -> &str {
        self.rows.get(row).and_then(|r| r.get(col)).map_or("", |s| s.as_str())
    }
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

        let ptr_jobs    = self.header.ptr_jobs    as usize;
        let ptr_results = self.header.ptr_results as usize;
        if ptr_results <= ptr_jobs || ptr_results > self.data.len() {
            return map;
        }

        let jobs_count = self.job_count();
        let jobs_end   = ptr_jobs + 4 + jobs_count * JOB_ENTRY_SIZE;
        if jobs_end + 8 >= ptr_results {
            return map;
        }

        // XOR-decode the whole data section once
        let raw_section = &self.data[jobs_end..ptr_results];
        let section = xor_decode(raw_section);

        // Data starts 8 bytes in (skip 8-byte null header)
        let mut offset = 8usize;

        // Step 1 – read column header (uppercase/digit/underscore strings)
        let mut columns: Vec<String> = Vec::new();
        while offset < section.len() {
            let (s, next) = read_cstring_at(&section, offset);
            if s.is_empty() || !is_col_name(&s) {
                break;
            }
            columns.push(s);
            offset = next;
        }
        if columns.is_empty() {
            return map;
        }
        let n_cols = columns.len();

        // Step 2 – read data rows until we hit non-printable bytes (directory)
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut cur_row: Vec<String> = Vec::new();
        while offset < section.len() {
            // Peek at next byte; non-printable means we hit the binary directory
            if section[offset] > 127 || (section[offset] < 32 && section[offset] != 0) {
                break;
            }
            let (s, next) = read_cstring_at(&section, offset);
            cur_row.push(s);
            if cur_row.len() == n_cols {
                rows.push(std::mem::take(&mut cur_row));
            }
            offset = next;
        }

        // Build the main table (always present; named "BETRIEBSWTAB" in DDE40*)
        let main_table = Table { columns: columns.clone(), rows };
        // Register it under common aliases
        map.insert("BETRIEBSWTAB".into(), main_table.clone());

        // Step 3 – parse the binary directory to register other table names
        // Each 80-byte entry: [0-7] flags, [8-11] count, [12-43] 32-byte name,
        // [44-75] zeros, [76-79] data_ptr (abs file offset, currently unused).
        //
        // For now we only have one data block, so every table name in the
        // directory maps to the same Table object (the one we just parsed).
        let dir_start = offset;
        let mut d = dir_start;
        while d + 80 <= section.len() {
            // Name starts at byte 12 within the 80-byte entry
            let name_slice = &section[d + 12..d + 44];
            let end = name_slice.iter().position(|&b| b == 0).unwrap_or(32);
            let name_bytes = &name_slice[..end];
            if name_bytes.is_empty() || !name_bytes.iter().all(|&b| b > 32 && b < 128) {
                break; // end of directory
            }
            let table_name = String::from_utf8_lossy(name_bytes).to_string();
            map.entry(table_name).or_insert_with(|| main_table.clone());
            d += 80;
        }

        map
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
        let code_raw = &self.data[ptr_code..ptr_jobs];

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

        let start = entries[idx].0 - ptr_code;
        let end = entries
            .get(idx + 1)
            .map(|(off, _)| off - ptr_code)
            .unwrap_or(code_raw.len());

        Some(xor_decode(&code_raw[start..end]))
    }
}
