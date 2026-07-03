//! Ground-truth decode test against the smallest complete module, S_FUNK_I.IPO
//! (674 bytes, fully hand-verified). See scratchpad `S_FUNK_I.annotated.txt`.

use inpa::opcode::{self, op};
use inpa::record::{tag, Const};
use inpa::{read_module, Body};
use std::path::PathBuf;

fn sgdat() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../SGDAT")
}

#[test]
fn s_funk_i_decodes_exactly() {
    let path = sgdat().join("S_FUNK_I.IPO");
    if !path.exists() {
        eprintln!("skip: {} not present (gitignored corpus)", path.display());
        return;
    }
    let m = read_module(&path).expect("parse S_FUNK_I");

    assert_eq!(m.title, "TEST-Infotext");
    assert_eq!(m.records.len(), 6, "4 PROC + GLOBALS + CONSTS");

    // Record identities.
    let by_name = |n: &str| m.records.iter().find(|r| r.name == n).unwrap();
    assert_eq!(by_name("inpainit").tag, tag::PROC);
    assert_eq!(by_name("inpainit").index, 2);
    assert_eq!(by_name("__inpa_startup__").index, 0);
    assert!(matches!(by_name("Global Data").body, Body::Globals(_)));
    assert!(matches!(by_name("Constant Data").body, Body::Consts(_)));

    // Constant pool: 20 entries, first two known strings.
    let consts = m.const_pool().expect("const pool");
    assert_eq!(consts.len(), 20);
    assert_eq!(consts[0], Const::Str("inpa.h".into()));
    assert_eq!(consts[1], Const::Str("Remote adapter initialisation".into()));

    // inpainit body: 64 instructions, all recognised.
    let instrs = opcode::decode(by_name("inpainit").code().unwrap());
    assert_eq!(instrs.len(), 64);
    assert!(instrs.iter().all(|i| i.is_known()), "no unknown instructions");
    assert_eq!(instrs[0].op, op::DECL);
    assert_eq!(instrs[0].mode, 0x55, "declare local string");
    assert_eq!(instrs[54].builtin(), Some(0x62), "call INPAapiJob");
    assert_eq!(instrs[63].op, op::RET);

    // __inpa_startup__ ends by calling proc #2 (inpainit).
    let startup = opcode::decode(by_name("__inpa_startup__").code().unwrap());
    assert_eq!(startup.last().unwrap().proc_call(), Some(2));
}
