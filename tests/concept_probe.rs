//! End-to-end check that the static concept probe reads real `.prg` files correctly.
//! The `ecu/` corpus is gitignored, so each case skips (passes) when its file is
//! absent — the assertions only run on a machine that has the SGBDs staged.

use ediabas::config::{CommConfig, Protocol};
use ediabas::prg::PrgFile;
use std::path::Path;

fn cfg(name: &str) -> Option<CommConfig> {
    let p = Path::new("ecu").join(name);
    if !p.exists() {
        eprintln!("skip: {name} not present");
        return None;
    }
    let prg = PrgFile::open(&p).expect("parse prg");
    Some(prg.initial_comm_config().expect("has INITIALISIERUNG concept"))
}

fn probe(name: &str) -> Option<Protocol> {
    cfg(name).map(|c| c.protocol)
}

#[test]
fn dde40_is_ds2() {
    if let Some(proto) = probe("DDE40KW0.prg") {
        assert_eq!(proto, Protocol::Ds2, "DDE40KW0 must probe as DS2 (concept 0x0006)");
    }
}

#[test]
fn ms430_petrol_is_ds2() {
    if let Some(proto) = probe("MS430DS0.prg") {
        assert_eq!(proto, Protocol::Ds2);
    }
}

#[test]
fn old_m51_diesel_is_kwp1281() {
    if let Some(proto) = probe("DDE21K20.prg") {
        assert_eq!(proto, Protocol::Kwp1281, "DDE21 (M51) must probe as KWP1281 (concept 0x0002)");
    }
}

#[test]
fn modern_dcan_is_dcan_at_can_bitrate() {
    // A concept-0x0110 SGBD (u32-element CommParameter): D-CAN, bitrate 500000.
    if let Some(c) = cfg("MEVD172Y.prg") {
        assert_eq!(c.protocol, Protocol::DCan);
        assert!(c.protocol.is_can());
        assert_eq!(c.baud, 500_000, "D-CAN CommParameter declares the 500k CAN bitrate");
    }
}

#[test]
fn kwp2000_bmw_u32_baud() {
    // A concept-0x010C SGBD: KWP2000-BMW, K-line baud 10400 (u32 element[1]).
    if let Some(c) = cfg("03BMSC2.prg") {
        assert_eq!(c.protocol, Protocol::Kwp2000Bmw);
        assert_eq!(c.concept, 0x010C);
        assert_eq!(c.baud, 10_400, "KWP2000-BMW runs 10400 baud on K-line");
    }
}

#[test]
fn address_group_files_probe_and_carry_ident() {
    // The address-keyed group files (`D_<ADDR>.GRP`) drive INPA variant identification:
    // they must parse, resolve to a K-line DS2 transport (exactly as `Session::open`
    // does — via the INITIALISIERUNG concept, or the DS2 default when a group leaves it
    // implicit, e.g. D_005B), and define the IDENTIFIKATION job whose VARIANTE result
    // names the variant. D_0080 = 0x80 cluster (IKE/KOMBI39…), D_0012 = 0x12 engine.
    for name in ["D_0080.GRP", "D_0012.GRP", "D_00D0.GRP", "D_005B.GRP"] {
        let p = Path::new("ecu").join(name);
        if !p.exists() {
            eprintln!("skip: {name} not present");
            continue;
        }
        let prg = PrgFile::open(&p).expect("parse .GRP");
        // Mirror Session::open: concept from INITIALISIERUNG, else the DS2 default.
        let cfg = prg.initial_comm_config().unwrap_or_default();
        assert_eq!(cfg.protocol, Protocol::Ds2, "{name} is a K-line DS2 group");
        assert!(
            prg.job_code("IDENTIFIKATION").is_some(),
            "{name} must define IDENTIFIKATION (the variant-ident job)"
        );
    }
}
