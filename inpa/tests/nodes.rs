//! Phase-2 test: node tree, menu items and F-key/target resolution on DDE40.IPO.

use inpa::model::{NavTarget, ScreenKind};
use inpa::{extract, Node};
use std::path::PathBuf;

fn dde40() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../SGDAT/DDE40.IPO")
}

#[test]
fn dde40_node_tree() {
    let path = dde40();
    if !path.exists() {
        eprintln!("skip: {} not present (gitignored corpus)", path.display());
        return;
    }
    let m = inpa::parse(&path).expect("parse DDE40");
    assert_eq!(m.script, "DDE40");

    // Root menu is m_main and has a title.
    let root = m.root_menu();
    assert_eq!(root.name, "m_main");
    assert!(!root.items.is_empty(), "m_main has items");

    // Item 10 = exit (fkey 10, unshifted).
    let exit = root
        .items
        .iter()
        .find(|i| matches!(i.target, NavTarget::Exit))
        .expect("an Exit item");
    assert_eq!(exit.fkey, 10);
    assert!(!exit.shifted);

    // Item 4 opens a detail screen and swaps the menu.
    assert!(
        root.items
            .iter()
            .any(|i| matches!(i.target, NavTarget::ScreenAndMenu { .. })),
        "a ScreenAndMenu item"
    );

    // Every screen target resolves to an actual Screen node.
    for it in &root.items {
        match &it.target {
            NavTarget::Screen(s) | NavTarget::ScreenAndMenu { screen: s, .. } => {
                assert!(matches!(m.node(*s), Node::Screen(_)), "screen target is a Screen");
            }
            NavTarget::Menu(mn) => {
                assert!(matches!(m.node(*mn), Node::Menu(_)), "menu target is a Menu");
            }
            _ => {}
        }
    }

    // Classification produced both a data-stream and a text-info screen.
    let (menus, screens) = extract::counts(&m);
    assert!(menus > 0 && screens > 0, "has menus and screens");
    let kinds: Vec<ScreenKind> = m
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Screen(s) => Some(s.kind),
            _ => None,
        })
        .collect();
    assert!(kinds.contains(&ScreenKind::DataStream), "a DataStream screen");
    assert!(kinds.contains(&ScreenKind::TextInfo), "a TextInfo screen");
}
