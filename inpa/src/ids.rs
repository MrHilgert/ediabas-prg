//! Builtin-id map and the names of the standard INPA helper procedures.
//!
//! Builtin ids (`call.builtin #id`) index the INPA runtime's extern table. The subset
//! below is empirically confirmed (Rosetta: `startger.ips`↔`startger.ipo` + `BMW_STD.H`
//! source bodies + DDE40 anchors) and covers every call the screen extractor needs.
//!
//! Helper procs (`ergebnis*`, `ansteuerung`) are resolved by **name** against the
//! module's PROC records, not by a fixed index — indices are per-module.

/// Return the name of a known builtin, if recognised.
pub fn builtin_name(id: u16) -> Option<&'static str> {
    Some(match id {
        0x00 => "setmenutitle",
        0x01 => "setmenu",
        0x02 => "setitem",
        0x03 => "settitle",
        0x04 => "setscreen",
        0x0C => "exit",
        0x0E => "scriptselect",
        0x20 => "inttostring",
        0x24 => "strlen",
        0x25 => "midstr",
        0x28 => "bytetoint",
        0x29 => "inttolong",
        0x3E => "getinputstate",
        0x43 => "input2text",
        0x48 => "text",
        0x4A => "ftextout",
        0x4B => "digitalout",
        0x4C => "analogout",
        0x52 => "messagebox",
        0x5B => "callwin",
        0x5D => "viewclose",
        0x60 => "INPAapiInit",
        0x61 => "INPAapiEnd",
        0x62 => "INPAapiJob",
        0x63 => "INPAapiResultText",
        0x66 => "INPAapiResultDigital",
        0x67 => "INPAapiResultAnalog",
        0x69 => "INPAapiCheckJobStatus",
        0x72 => "INP1apiResultInt",
        _ => return None,
    })
}

/// Builtin ids referenced by the extractor, as named constants.
pub mod builtin {
    pub const SETMENU: u16 = 0x01;
    pub const SETSCREEN: u16 = 0x04;
    pub const SCRIPTSELECT: u16 = 0x0E;
    pub const TEXT: u16 = 0x48;
    pub const FTEXTOUT: u16 = 0x4A;
    pub const DIGITALOUT: u16 = 0x4B;
    pub const ANALOGOUT: u16 = 0x4C;
    pub const INPA_JOB: u16 = 0x62;
    pub const INPA_RESULT_TEXT: u16 = 0x63;
    pub const INPA_RESULT_DIGITAL: u16 = 0x66;
    pub const INPA_RESULT_ANALOG: u16 = 0x67;
}

/// Names of the standard INPA library helper procs the extractor recognises.
pub mod helper {
    pub const TEXT_OUT: &str = "ergebnisTextAusgabe";
    pub const DIGITAL_OUT: &str = "ergebnisDigitalAusgabe";
    pub const ANALOG_OUT: &str = "ergebnisAnalogAusgabe";
    pub const ANALOG_CONV_OUT: &str = "ergebnisUmrechnungAnalogAusgabe";
    pub const ANSTEUERUNG: &str = "ansteuerung";
}
