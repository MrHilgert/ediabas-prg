//! Одноразовый дампер сырых записей `.ipo`: tag/name/str2/str3 + начало кода.
//! Запуск: cargo run -p inpa --example dumprec -- SGDAT/LWR2A.IPO [filter]
use inpa::record::tag;

fn tagname(t: u8) -> &'static str {
    match t {
        tag::SCREEN => "SCREEN",
        tag::MENU => "MENU",
        tag::PROC => "PROC",
        tag::CODE => "CODE",
        tag::LINE => "LINE",
        tag::ITEM => "ITEM",
        tag::SPACER => "SPACER",
        tag::CONSTS => "CONSTS",
        tag::GLOBALS => "GLOBALS",
        _ => "?",
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: dumprec <path.ipo> [filter]");
    let filt = std::env::args().nth(2).unwrap_or_default().to_lowercase();
    // Current extractor output (rows the GUI actually gets) for the filtered screens.
    if let Ok(sm) = inpa::parse(&path) {
        for n in &sm.nodes {
            if let inpa::Node::Screen(s) = n {
                if !filt.is_empty() && !s.name.to_lowercase().contains(&filt) {
                    continue;
                }
                println!("\n>> PARSED SCREEN {} kind={:?} feeder={:?} rows={} info={}",
                    s.name, s.kind, s.feeder.as_ref().map(|j| &j.job), s.rows.len(), s.info.len());
                for row in &s.rows {
                    println!("     row label={:?} result={:?}", row.label(), row.result());
                }
            }
        }
        // Resolve each menu item's nav target to a screen NAME (which variant we actually open).
        for n in &sm.nodes {
            if let inpa::Node::Menu(mn) = n {
                if !filt.is_empty() && !mn.name.to_lowercase().contains(&filt) {
                    continue;
                }
                println!("\n>> MENU {} items:", mn.name);
                for it in &mn.items {
                    let tgt = match &it.target {
                        inpa::NavTarget::Screen(id) | inpa::NavTarget::ScreenAndMenu { screen: id, .. } => {
                            sm.as_screen(*id).map(|s| s.name.clone()).unwrap_or_default()
                        }
                        other => format!("{other:?}"),
                    };
                    println!("     [{}] {:?} -> {}", it.fkey, it.label, tgt);
                }
            }
        }
        println!("\n---- raw records ----");
    }

    let m = inpa::read_module(&path).expect("parse");
    let consts = m.const_pool().unwrap_or(&[]);
    let mut show = filt.is_empty();
    for r in &m.records {
        // A SCREEN record starts a new node; gate printing on the filter matching its name.
        if r.tag == tag::SCREEN || r.tag == tag::MENU || r.tag == tag::PROC {
            show = filt.is_empty() || r.name.to_lowercase().contains(&filt);
            if show {
                println!("\n== {} {} ==", tagname(r.tag), r.name);
            }
        }
        if !show {
            continue;
        }
        match r.tag {
            tag::LINE => {
                println!("  LINE str2={:?} str3={:?}", r.str2, r.str3);
                if let Some(code) = r.code() {
                    for call in inpa::extract::dbg_calls(code, consts) {
                        println!("      {call}");
                    }
                }
            }
            tag::ITEM => {
                println!("  ITEM#{} str2={:?}", r.item_no(), r.str2);
                if let Some(code) = r.code() {
                    for call in inpa::extract::dbg_calls(code, consts) {
                        println!("      {call}");
                    }
                }
            }
            tag::CODE => {
                let n = r.code().map(|c| c.len()).unwrap_or(0);
                println!("  CODE {n}b");
                if let Some(code) = r.code() {
                    for call in inpa::extract::dbg_calls(code, consts) {
                        println!("      {call}");
                    }
                }
            }
            _ => {}
        }
    }
}
