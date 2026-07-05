//! Metadata-style activation extraction: LCM `s_steuern` binds its selectable in/outputs in
//! LINE `str2`/`str3` (no code body). The extractor must surface them as rows, else the
//! activation screen renders empty (grievance #5, safe first increment).

use inpa::model::{Row, ScreenKind};
use inpa::Node;
use std::path::PathBuf;

fn lcm() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../SGDAT/LCM.IPO")
}

#[test]
fn lcm_activation_list_from_metadata() {
    let path = lcm();
    if !path.exists() {
        eprintln!("skip: {} not present (gitignored corpus)", path.display());
        return;
    }
    let m = inpa::parse(&path).expect("parse LCM");
    let steuern = m
        .nodes
        .iter()
        .find_map(|n| match n {
            Node::Screen(s) if s.name.eq_ignore_ascii_case("s_steuern") => Some(s),
            _ => None,
        })
        .expect("s_steuern screen");

    assert_eq!(steuern.kind, ScreenKind::Activation, "stays an activation screen");
    assert!(steuern.rows.len() > 20, "in/output list extracted from str2/str3, got {}", steuern.rows.len());

    // A known item: label "Clamp 30A" bound to id "Kl30A".
    let clamp = steuern.rows.iter().find_map(|r| match r {
        Row::Text { label, result, .. } if label == "Clamp 30A" => Some(result.as_str()),
        _ => None,
    });
    assert_eq!(clamp, Some("Kl30A"), "metadata row carries label→id binding");
}
