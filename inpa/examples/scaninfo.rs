//! Скан утечки нав-текста: экраны с непустым `info`, но без feeder и rows — кандидаты, где
//! `textinfo` рисует статический ftextout-текст как контент. Запуск:
//!   cargo run -p inpa --example scaninfo -- SGDAT
use std::fs;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "SGDAT".into());
    let mut total = 0usize;
    let mut hits = 0usize;
    for entry in fs::read_dir(&dir).expect("read dir") {
        let path = entry.expect("entry").path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("ipo") {
            continue;
        }
        let Ok(sm) = inpa::parse(&path) else { continue };
        for n in &sm.nodes {
            let inpa::Node::Screen(s) = n else { continue };
            total += 1;
            // Утечка-кандидат: есть статический info-текст, но ни джоба, ни строк.
            if !s.info.is_empty() && s.feeder.is_none() && s.rows.is_empty() {
                hits += 1;
                let sample: Vec<String> =
                    s.info.iter().take(3).map(|l| format!("{:?}={:?}", l.label, l.value)).collect();
                println!(
                    "{}  {}  kind={:?} info={}  {}",
                    path.file_name().unwrap().to_string_lossy(),
                    s.name,
                    s.kind,
                    s.info.len(),
                    sample.join(" | ")
                );
            }
        }
    }
    eprintln!("\nscreens total={total}  info-only(feeder=None,rows=0)={hits}");
}
