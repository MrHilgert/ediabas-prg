//! Node tree, navigation/F-keys, and screen-content assembly.
//!
//! Records appear in file order: a MENU record is followed by its ITEM records; a SCREEN
//! record is followed by its CODE/LINE records. Object references use **per-table** indices
//! (`scr#N`, `mnu#N`, `proc#N` are separate spaces), so screens/menus/procs each resolve
//! through their own index map — this is what avoids the disassembler's index-collision bug.

use std::collections::HashMap;

use super::eval::{call_sites, Target, Val};
use super::rows;
use crate::ids::{self, builtin, helper};
use crate::model::{JobCall, Menu, NavItem, NavTarget, Node, Screen, ScreenKind, ScreenModule};
use crate::opcode::{self, mode, op};
use crate::record::{tag, Const, Module, Record};

pub(super) fn build(module: &Module, script: String) -> ScreenModule {
    let consts = module.const_pool().unwrap_or(&[]);
    let proc_names = proc_name_map(module);

    // Pass A — one Node per SCREEN/MENU; index maps; child records grouped by parent.
    let mut nodes: Vec<Node> = Vec::new();
    let mut screen_map: HashMap<u32, usize> = HashMap::new();
    let mut menu_map: HashMap<u32, usize> = HashMap::new();
    let mut item_groups: Vec<(usize, &Record)> = Vec::new();
    let mut screen_children: HashMap<usize, Vec<&Record>> = HashMap::new();
    let mut cur_menu: Option<usize> = None;
    let mut cur_screen: Option<usize> = None;

    for rec in &module.records {
        match rec.tag {
            tag::MENU => {
                let id = nodes.len();
                nodes.push(Node::Menu(Menu {
                    name: rec.name.clone(),
                    title: menu_title(rec, consts),
                    items: Vec::new(),
                }));
                menu_map.insert(rec.index, id);
                cur_menu = Some(id);
            }
            tag::SCREEN => {
                let id = nodes.len();
                nodes.push(Node::Screen(Screen {
                    name: rec.name.clone(),
                    title: String::new(),
                    kind: super::kind_from_name(&rec.name),
                    feeder: None,
                    rows: Vec::new(),
                    info: Vec::new(),
                }));
                screen_map.insert(rec.index, id);
                cur_screen = Some(id);
            }
            tag::ITEM => {
                if let Some(menu_id) = cur_menu {
                    item_groups.push((menu_id, rec));
                }
            }
            tag::CODE | tag::LINE | tag::SPACER => {
                if let Some(screen_id) = cur_screen {
                    screen_children.entry(screen_id).or_default().push(rec);
                }
            }
            _ => {}
        }
    }

    // Pass B — resolve nav items now that all nodes/maps exist.
    for (menu_id, rec) in item_groups {
        let item = decode_item(rec, &screen_map, &menu_map, &proc_names, consts);
        if let Node::Menu(m) = &mut nodes[menu_id] {
            m.items.push(item);
        }
    }

    // Pass C — fill each screen's title/feeder/rows and refine its kind.
    for (screen_id, recs) in &screen_children {
        let content = rows::extract_screen(recs, &proc_names, consts);
        if let Node::Screen(s) = &mut nodes[*screen_id] {
            if !content.title.is_empty() {
                s.title = content.title;
            }
            s.feeder = content.feeder;
            s.rows = content.rows;
            s.info = content.info;
            if s.kind == ScreenKind::Other {
                s.kind = kind_from_rows(&s.rows);
                // A screen with no job-bound rows but static ftextout text (e.g. `s_abgleich`
                // instructions, `s_info`) is a textual info page.
                if s.kind == ScreenKind::Other && !s.info.is_empty() {
                    s.kind = ScreenKind::TextInfo;
                }
            }
        }
    }
    for node in &mut nodes {
        if let Node::Screen(s) = node {
            if s.title.is_empty() {
                s.title = s.name.clone();
            }
        }
    }

    let root = menu_map
        .iter()
        .find(|(_, &id)| matches!(&nodes[id], Node::Menu(m) if m.name.eq_ignore_ascii_case("m_main")))
        .map(|(_, &id)| id)
        .or_else(|| menu_map.get(&0).copied())
        .or_else(|| nodes.iter().position(|n| matches!(n, Node::Menu(_))))
        .unwrap_or(0);

    ScreenModule { sgbd: sgbd_name(consts), group_files: group_files(consts), script, root, nodes }
}

/// Address-keyed EDIABAS group files (`D_<ADDR>`) referenced by the script — the INPA
/// variant-identification entry points. The `.ipo` const pool names them directly
/// (e.g. `KOMBI.ipo` → `D_0080`; a cluster script spanning two diagnostic addresses
/// carries both `D_000D` and `D_0080`). Each names `ecu/<name>.GRP`, whose
/// `IDENTIFIKATION` job returns the concrete variant in result `VARIANTE`. Returned
/// in first-seen order (deduplicated); the connect flow tries them in turn until one
/// identifies. Empty when the script references no group (single-variant ECU).
fn group_files(consts: &[Const]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in consts {
        let Const::Str(s) = c else { continue };
        let s = s.trim();
        if is_group_file(s) && !out.iter().any(|x| x == s) {
            out.push(s.to_string());
        }
    }
    out
}

/// `D_` followed by exactly four hex digits (e.g. `D_0080`, `D_00D0`) — the EDIABAS
/// address-keyed group-file naming convention.
fn is_group_file(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() == 6 && b[0] == b'D' && b[1] == b'_' && b[2..].iter().all(u8::is_ascii_hexdigit)
}

/// Map proc-record index → proc name (tag 0x05 only — never SCREEN/MENU).
fn proc_name_map(module: &Module) -> HashMap<u16, String> {
    module
        .records
        .iter()
        .filter(|r| r.tag == tag::PROC)
        .map(|r| (r.index as u16, r.name.clone()))
        .collect()
}

/// Extract the primary SGBD name the screens drive — i.e. **which `.prg` to load**.
///
/// The `.ipo` carries an IDENT info-literal of the form `"NAME,VARIANT"` (e.g.
/// `"DDE40KW0,D40M57A1"`, or `"GS19/V1.06,GS19A/V3.01"` for versioned/multi-block ECUs).
/// The `NAME` head (before any `/` or space) is the SGBD → the `.prg` to load. This is how
/// the `.ipo` tells us which PRG to work with. Falls back to empty if none is present.
fn sgbd_name(consts: &[Const]) -> String {
    for c in consts {
        let Const::Str(s) = c else { continue };
        let Some((head, tail)) = s.split_once(',') else { continue };
        if tail.is_empty() {
            continue;
        }
        let cand = head.split(['/', ' ']).next().unwrap_or(head).trim();
        if is_sgbd_like(cand) {
            return cand.to_string();
        }
    }
    String::new()
}

/// Does this string look like an SGBD name (`DDE40KW0`, `MSV70`, `CAS`, …) as opposed to a
/// label word? Alphanumeric/underscore, 3..16 chars, and either contains a digit or is all
/// upper-case (rejects mixed-case prose like the head of a comma-joined label list).
fn is_sgbd_like(s: &str) -> bool {
    let s = s.trim();
    (3..=16).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && (s.bytes().any(|b| b.is_ascii_digit()) || !s.bytes().any(|b| b.is_ascii_lowercase()))
}

/// Extract a menu title from its `setmenutitle(const)` call.
fn menu_title(rec: &Record, consts: &[Const]) -> String {
    let Some(code) = rec.code() else { return String::new() };
    let mut last_const: Option<u16> = None;
    for ins in opcode::decode(code) {
        if ins.op == op::PUSH && ins.mode == mode::CONST {
            last_const = Some(ins.opnd);
        } else if ins.builtin() == Some(0x00) {
            if let Some(ci) = last_const {
                if let Some(Const::Str(s)) = consts.get(ci as usize) {
                    return s.clone();
                }
            }
        }
    }
    String::new()
}

fn kind_from_rows(rows: &[crate::model::Row]) -> ScreenKind {
    use crate::model::Row::*;
    match rows.first() {
        Some(Logical { .. } | Analog { .. }) => ScreenKind::DataStream,
        Some(Text { .. }) => ScreenKind::TextInfo,
        Some(Action { .. }) => ScreenKind::Activation,
        None => ScreenKind::Other,
    }
}

/// Decode one ITEM record into a [`NavItem`].
fn decode_item(
    rec: &Record,
    screen_map: &HashMap<u32, usize>,
    menu_map: &HashMap<u32, usize>,
    proc_names: &HashMap<u16, String>,
    consts: &[Const],
) -> NavItem {
    let item_no = rec.item_no().max(1);
    let fkey = (((item_no - 1) % 10) + 1) as u8;
    let shifted = item_no > 10;
    let target = decode_target(rec, screen_map, menu_map, proc_names, consts);
    NavItem { fkey, shifted, label: rec.str2.clone(), target }
}

fn decode_target(
    rec: &Record,
    screen_map: &HashMap<u32, usize>,
    menu_map: &HashMap<u32, usize>,
    proc_names: &HashMap<u16, String>,
    consts: &[Const],
) -> NavTarget {
    let Some(code) = rec.code() else { return NavTarget::Other("empty".into()) };

    let mut screen_t: Option<usize> = None;
    let mut menu_t: Option<usize> = None;
    let mut job: Option<JobCall> = None;
    let mut is_exit = false;
    let mut is_script = false;
    let mut other: Option<String> = None;

    let arg = |args: &[Val], i: usize| args.get(i).and_then(Val::as_str).unwrap_or("").to_string();

    for cs in call_sites(code, consts) {
        match cs.target {
            Target::Builtin(b) => match b {
                builtin::SETSCREEN => {
                    // A data item issues TWO SETSCREENs: the generic screen first, then a
                    // variant override (`s_analog1` then `s_analog1_d40m57a1`). Keep the FIRST
                    // (generic base) — the runtime picks the variant sibling by the connected
                    // SGBD via `ScreenModule::screen_for_variant`. Taking the last would pin
                    // every ECU to one variant's result names (e.g. `_IST_WERT`), so a base ECU
                    // returning `_WERT` shows dashes on exactly the diverging rows.
                    if screen_t.is_none() {
                        if let Some(Val::ScreenRef(n)) = cs.args.first() {
                            screen_t = screen_map.get(&(*n as u32)).copied();
                        }
                    }
                }
                builtin::SETMENU => {
                    if let Some(Val::MenuRef(n)) = cs.args.first() {
                        menu_t = menu_map.get(&(*n as u32)).copied();
                    }
                }
                builtin::INPA_JOB => {
                    let j = rows::job_from(&cs.args);
                    if j.job.starts_with("STEUERN") || j.job.starts_with("ANSTEUERN") {
                        job = Some(j);
                    }
                }
                0x0C => is_exit = true, // exit
                builtin::SCRIPTSELECT | 0x10 | 0x11 => is_script = true,
                _ => {
                    if other.is_none() {
                        other = Some(
                            ids::builtin_name(b)
                                .map(str::to_string)
                                .unwrap_or_else(|| format!("builtin_{b:#04x}")),
                        );
                    }
                }
            },
            Target::Proc(idx) => {
                if proc_names.get(&idx).map(String::as_str) == Some(helper::ANSTEUERUNG) {
                    job = Some(JobCall {
                        job: arg(&cs.args, 0),
                        arg: arg(&cs.args, 1),
                        selector: None,
                    });
                }
            }
            Target::Import => {}
        }
    }

    if let Some(j) = job {
        return NavTarget::Job(j);
    }
    match (screen_t, menu_t) {
        (Some(screen), Some(menu)) => NavTarget::ScreenAndMenu { screen, menu },
        (Some(screen), None) => NavTarget::Screen(screen),
        (None, Some(menu)) => NavTarget::Menu(menu),
        (None, None) => {
            if is_exit {
                NavTarget::Exit
            } else if is_script {
                NavTarget::Script(rec.str2.clone())
            } else {
                NavTarget::Other(other.unwrap_or_else(|| "?".into()))
            }
        }
    }
}
