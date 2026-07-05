//! Скан конструктора попапа выбора: билтин 0x16 (22) в коде ITEM/CODE-записей. Печатает
//! распределение первых двух булевых аргументов (гипотеза: сингл vs мульти) и примеры.
//! Запуск: cargo run -p inpa --example scanselect -- SGDAT
use std::collections::BTreeMap;
use std::fs;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "SGDAT".into());
    let mut combos: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples: Vec<String> = Vec::new();
    for entry in fs::read_dir(&dir).expect("read dir") {
        let path = entry.expect("entry").path();
        if !path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("ipo")) {
            continue;
        }
        let Ok(m) = inpa::read_module(&path) else { continue };
        let consts = m.const_pool().unwrap_or(&[]);
        for r in &m.records {
            let Some(code) = r.code() else { continue };
            for call in inpa::extract::dbg_calls(code, consts) {
                if !call.starts_with("Builtin(22)") {
                    continue;
                }
                // Аргументы идут после "Builtin(22)  " — берём первые два токена.
                let args = call.splitn(2, "  ").nth(1).unwrap_or("");
                let key: String = args.split(", ").take(2).collect::<Vec<_>>().join(" , ");
                *combos.entry(key).or_default() += 1;
                if samples.len() < 25 {
                    samples.push(format!("{}  {}  {}", path.file_name().unwrap().to_string_lossy(), r.name, args));
                }
            }
        }
    }
    println!("== распределение первых двух аргументов Builtin(22) ==");
    for (k, n) in &combos {
        println!("  {n:5}  [{k}]");
    }
    println!("\n== примеры ==");
    for s in &samples {
        println!("  {s}");
    }
}
