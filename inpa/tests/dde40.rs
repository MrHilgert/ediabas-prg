//! Phase-3 golden test: known bindings extracted from DDE40.IPO across all archetypes.

use inpa::model::{NavTarget, Row, ScreenKind};
use inpa::Node;
use std::path::PathBuf;

fn dde40() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../SGDAT/DDE40.IPO")
}

fn screens(m: &inpa::ScreenModule) -> Vec<&inpa::Screen> {
    m.nodes
        .iter()
        .filter_map(|n| match n {
            Node::Screen(s) => Some(s),
            _ => None,
        })
        .collect()
}

#[test]
fn dde40_bindings() {
    let path = dde40();
    if !path.exists() {
        eprintln!("skip: {} not present (gitignored corpus)", path.display());
        return;
    }
    let m = inpa::parse(&path).expect("parse DDE40");
    // The .ipo tells us which PRG to load.
    assert_eq!(m.sgbd, "DDE40KW0", "SGBD extracted from the .ipo");
    let scr = screens(&m);

    // --- Logical row: s_digital, STAT_AC_EIN with On/Off + STATUS_DIGITAL feeder ---
    let digital = scr
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case("s_digital"))
        .expect("s_digital");
    assert_eq!(digital.kind, ScreenKind::DataStream);
    assert_eq!(digital.feeder.as_ref().map(|j| j.job.as_str()), Some("STATUS_DIGITAL"));
    let ac = digital
        .rows
        .iter()
        .find_map(|r| match r {
            Row::Logical { result, on, off, .. } if result == "STAT_AC_EIN" => Some((on, off)),
            _ => None,
        })
        .expect("STAT_AC_EIN logical row");
    assert_eq!(ac.0.trim(), "ON");
    assert_eq!(ac.1.trim(), "OFF");

    // --- Analog row: s_analog1, STAT_ZUOME_VE_WERT with unit + per-row job ---
    let analog = scr
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case("s_analog1"))
        .expect("s_analog1");
    let ve = analog
        .rows
        .iter()
        .find_map(|r| match r {
            Row::Analog { result, unit, scale, job, .. } if result == "STAT_ZUOME_VE_WERT" => {
                Some((unit.clone(), *scale, job.job.clone()))
            }
            _ => None,
        })
        .expect("STAT_ZUOME_VE_WERT analog row");
    assert!(ve.0.contains("mm"), "unit like mm2, got {:?}", ve.0);
    assert_eq!(ve.1.max, 20.0, "display max");
    assert_eq!(ve.2, "STATUS_ZUOME_VE", "per-row feeder job");

    // --- Text-info row: s_ident feeds ID_BMW_NR via ergebnisTextAusgabe ---
    let ident = scr
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case("s_ident"))
        .expect("s_ident");
    assert_eq!(ident.kind, ScreenKind::TextInfo);
    assert!(
        ident.rows.iter().any(|r| matches!(r, Row::Text { result, .. } if result == "ID_BMW_NR")),
        "ID_BMW_NR text row"
    );

    // --- Activation: a menu item runs STEUERN_LADEDRUCKSTELLER ---
    let has_activation = m.nodes.iter().any(|n| match n {
        Node::Menu(menu) => menu.items.iter().any(|it| {
            matches!(&it.target, NavTarget::Job(j) if j.job == "STEUERN_LADEDRUCKSTELLER")
        }),
        _ => false,
    });
    assert!(has_activation, "STEUERN_LADEDRUCKSTELLER activation item");
}

#[test]
fn dde40_variant_screen_resolution() {
    let path = dde40();
    if !path.exists() {
        eprintln!("skip: {} not present (gitignored corpus)", path.display());
        return;
    }
    let m = inpa::parse(&path).expect("parse DDE40");
    // The data menu item stores the GENERIC base screen (first SETSCREEN).
    let m_status = m
        .nodes
        .iter()
        .find_map(|n| match n {
            Node::Menu(mn) if mn.name.eq_ignore_ascii_case("m_status") => Some(mn),
            _ => None,
        })
        .expect("m_status menu");
    let analog1_id = m_status
        .items
        .iter()
        .find_map(|it| match &it.target {
            NavTarget::Screen(s) if m.as_screen(*s).is_some_and(|sc| sc.name.eq_ignore_ascii_case("s_analog1")) => {
                Some(*s)
            }
            _ => None,
        })
        .expect("Аналог #1 → generic s_analog1");

    // A base ECU keeps the generic screen; the M57 variant resolves to its `_d40m57a1` sibling.
    assert_eq!(m.screen_for_variant(analog1_id, "DDE40KW0"), analog1_id, "base ECU → generic");
    let variant_id = m.screen_for_variant(analog1_id, "D40M57A1");
    assert_eq!(
        m.as_screen(variant_id).map(|s| s.name.as_str()),
        Some("s_analog1_d40m57a1"),
        "M57 variant → variant screen"
    );
}
