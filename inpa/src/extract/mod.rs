//! Extraction: [`record::Module`] → typed [`model::ScreenModule`].
//!
//! Deterministic and side-effect free. This module wires the passes together; the
//! archetype recipes live in the submodules ([`nodes`], and — Phase 3 — logical/analog/…).

mod eval;
mod nodes;
mod rows;

use crate::model::{Node, ScreenKind, ScreenModule};
use crate::record::Module;

/// Build the screen model for a parsed module. `script` is the INPA script name
/// (usually the `.ipo` file stem).
pub fn build(module: &Module, script: impl Into<String>) -> ScreenModule {
    nodes::build(module, script.into())
}

/// First-cut screen archetype from the node name. Content-based refinement happens
/// during row extraction (Phase 3).
pub(crate) fn kind_from_name(name: &str) -> ScreenKind {
    let n = name.to_ascii_lowercase();
    if n.starts_with("s_fehler") || n.starts_with("s_fault") || n.contains("fehler") {
        ScreenKind::Faults
    } else if n.starts_with("s_digital") || n.starts_with("s_status") || n.starts_with("s_analog") {
        ScreenKind::DataStream
    } else if n.starts_with("s_ident") || n.starts_with("s_info") || n.starts_with("s_code") {
        ScreenKind::TextInfo
    } else if n.starts_with("s_steuern") || n.starts_with("s_ansteuer") {
        ScreenKind::Activation
    } else {
        ScreenKind::Other
    }
}

/// Diagnostic: dump a code body's recovered call sites (target + args) as strings — used by
/// the `dumprec` example for `.ipo` reverse-engineering (which builtins/procs a screen calls).
pub fn dbg_calls(code: &[u8], consts: &[crate::record::Const]) -> Vec<String> {
    eval::call_sites(code, consts)
        .iter()
        .map(|cs| {
            let args: Vec<String> = cs.args.iter().map(|a| format!("{a:?}")).collect();
            format!("{:?}  {}", cs.target, args.join(", "))
        })
        .collect()
}

/// Convenience: how many screens/menus the model holds (for diagnostics/tests).
pub fn counts(m: &ScreenModule) -> (usize, usize) {
    let mut menus = 0;
    let mut screens = 0;
    for n in &m.nodes {
        match n {
            Node::Menu(_) => menus += 1,
            Node::Screen(_) => screens += 1,
        }
    }
    (menus, screens)
}
