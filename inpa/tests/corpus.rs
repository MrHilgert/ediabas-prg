//! Corpus regression: every fingerprinted `.ipo` in SGDAT must parse cleanly, and every
//! decoded instruction must be a known encoding. This is the determinism guardrail — it
//! mirrors the Phase-0 `ipo_disasm.py` census (99.999% clean over the corpus).

use inpa::opcode;
use inpa::record::{has_fingerprint, parse_module, Body};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn sgdat() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../SGDAT")
}

#[test]
fn corpus_parses_clean() {
    let dir = sgdat();
    if !dir.exists() {
        eprintln!("skip: {} not present (gitignored corpus)", dir.display());
        return;
    }

    // Dedup by lowercased name (Windows FS is case-insensitive; *.IPO and *.ipo overlap).
    let mut seen = BTreeSet::new();
    let mut fingerprinted = 0usize;
    let mut non_fp = 0usize;
    let mut parsed = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut unknown_instrs: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(&dir).expect("read SGDAT") {
        let path = entry.unwrap().path();
        let is_ipo = path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("ipo"))
            .unwrap_or(false);
        if !is_ipo {
            continue;
        }
        let key = path.file_name().unwrap().to_string_lossy().to_lowercase();
        if !seen.insert(key.clone()) {
            continue;
        }

        let data = std::fs::read(&path).unwrap();
        if !has_fingerprint(&data) {
            non_fp += 1; // older non-screen container (job/coding libs) — out of scope
            continue;
        }
        fingerprinted += 1;

        match parse_module(&data) {
            Ok(m) => {
                parsed += 1;
                for rec in &m.records {
                    if let Body::Code(code) = &rec.body {
                        for (i, ins) in opcode::decode(code).iter().enumerate() {
                            if !ins.is_known() && unknown_instrs.len() < 40 {
                                unknown_instrs.push(format!(
                                    "{key}@{}+{i}: {:#04x} {:#04x}",
                                    rec.name, ins.op, ins.mode
                                ));
                            }
                        }
                    }
                }
            }
            Err(e) => failures.push(format!("{key}: {e}")),
        }
    }

    eprintln!(
        "corpus: {fingerprinted} fingerprinted ({parsed} parsed), {non_fp} non-fingerprint (skipped)"
    );
    assert!(fingerprinted > 0, "no fingerprinted .ipo found in corpus");
    assert!(failures.is_empty(), "structural parse failures: {failures:#?}");
    assert!(
        unknown_instrs.is_empty(),
        "unknown instruction encodings: {unknown_instrs:#?}"
    );
}
