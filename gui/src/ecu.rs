//! ECU model for screen 2 (ECU Select), ported from the prototype `CATS` / `MODS` +
//! `modsFor()` (year-overlap + AWD filter). Module status is real-only now: a
//! module is connectable iff it has a real SGBD (`.prg`); no fabricated health.

use crate::data::Chassis;
use crate::lang::Lang;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Pwr,
    Chs,
    Saf,
    Bdy,
    Gwy,
    Inf,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Pwr,
        Category::Chs,
        Category::Saf,
        Category::Bdy,
        Category::Gwy,
        Category::Inf,
    ];
    pub fn label(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Category::Pwr, Lang::Ru) => "СИЛОВОЙ АГРЕГАТ",
            (Category::Chs, Lang::Ru) => "ШАССИ",
            (Category::Saf, Lang::Ru) => "БЕЗОПАСНОСТЬ",
            (Category::Bdy, Lang::Ru) => "КУЗОВ",
            (Category::Gwy, Lang::Ru) => "ШЛЮЗ",
            (Category::Inf, Lang::Ru) => "МУЛЬТИМЕДИА",
            (Category::Pwr, Lang::En) => "POWERTRAIN",
            (Category::Chs, Lang::En) => "CHASSIS",
            (Category::Saf, Lang::En) => "SAFETY",
            (Category::Bdy, Lang::En) => "BODY",
            (Category::Gwy, Lang::En) => "GATEWAY",
            (Category::Inf, Lang::En) => "INFOTAINMENT",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Module {
    pub code: &'static str,
    pub ru: &'static str,
    pub en: &'static str,
    pub cat: Category,
    pub bus: &'static str,
    pub addr: &'static str,
    pub from: u32,
    pub to: u32,
    pub awd_only: bool,
    /// INPA `.ipo` screen-script name (hard-coded per ECU, the `SGDAT/<script>.ipo` stem).
    /// The screen tree AND the SGBD/`.prg` to load are both derived from this `.ipo`.
    pub script: Option<&'static str>,
    /// Concrete SGBD (.prg) mapped from the CFGDAT ENTRY script. `Some` = a real
    /// SGBD exists on disk, so this module can be opened and initialised for real.
    pub prg: Option<&'static str>,
    /// Confirmed connectable over an implemented transport (DS2, tested live).
    /// Informational only now — real I/O is driven by `prg`, not this flag.
    pub validated: bool,
}

impl Module {
    pub fn name(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::Ru => self.ru,
            Lang::En => self.en,
        }
    }

    /// Can this module be opened for real? True iff it has a real SGBD (`.prg`)
    /// on disk — the session then runs its actual `INITIALISIERUNG`.
    pub fn connectable(&self) -> bool {
        self.prg.is_some()
    }

}

macro_rules! m {
    ($code:literal, $ru:literal, $en:literal, $cat:ident, $bus:literal, $addr:literal, $from:literal, $to:literal, awd) => {
        Module { code: $code, ru: $ru, en: $en, cat: Category::$cat, bus: $bus, addr: $addr,
                 from: $from, to: $to, awd_only: true, script: None, prg: None, validated: false }
    };
    ($code:literal, $ru:literal, $en:literal, $cat:ident, $bus:literal, $addr:literal, $from:literal, $to:literal) => {
        Module { code: $code, ru: $ru, en: $en, cat: Category::$cat, bus: $bus, addr: $addr,
                 from: $from, to: $to, awd_only: false, script: None, prg: None, validated: false }
    };
}

/// Non-engine static modules. The engine block itself is expanded per-chassis
/// into its real engine markings (see `engine_module` / `mods_for`).
pub const MODS: &[Module] = &[
    m!("EGS",  "АКПП",              "Transmission",    Pwr, "PT-CAN", "0x32", 1991, 9999),
    m!("VGSG", "Раздаточная коробка","Transfer case",  Pwr, "PT-CAN", "0x39", 2003, 9999, awd),

    m!("DSC",  "Динамич. стабилизация", "Stability control", Chs, "PT-CAN", "0x56", 1998, 9999),
    m!("ABS",  "Антиблок. тормоза",  "Anti-lock brakes", Chs, "DS2",    "0x56", 1988, 2002),
    m!("EPS",  "Усилитель руля",     "Power steering",   Chs, "PT-CAN", "0x7F", 2004, 9999),
    m!("EHC",  "Пневмоподвеска",     "Air suspension",   Chs, "PT-CAN", "0x6D", 2001, 9999),

    m!("MRS",  "Система Airbag",     "Airbag (MRS)",     Saf, "K-bus",  "0xA4", 1990, 2007),
    m!("ACSM", "Датчики удара",      "Crash safety",     Saf, "F-CAN",  "0x63", 2007, 9999),

    m!("CAS",  "Доступ в автомобиль","Car Access",       Bdy, "K-CAN",  "0x40", 2004, 9999),
    m!("EWS",  "Иммобилайзер",       "Immobiliser",      Bdy, "K-bus",  "0x44", 1994, 2006),
    m!("GM",   "Электрика кузова",   "Body electronics", Bdy, "K-bus",  "0x00", 1994, 2008),
    m!("LCM",  "Блок света",         "Light control",    Bdy, "K-bus",  "0xD0", 1994, 2006),
    m!("FRM",  "Модуль света (FRM)", "Footwell/Light (FRM)", Bdy, "K-CAN", "0x72", 2004, 9999),
    m!("IHKA", "Климат-контроль",    "Climate control",  Bdy, "K-bus",  "0x5B", 1994, 9999),
    m!("SZL",  "Датчик угла руля",   "Steering column",  Bdy, "K-CAN",  "0x57", 2001, 9999),
    m!("PDC",  "Парктроник",         "Park distance",    Bdy, "K-bus",  "0x60", 1999, 9999),
    m!("RLS",  "Датчик дождя/света", "Rain-light sensor",Bdy, "K-CAN",  "0x71", 2001, 9999),
    m!("SM",   "Модуль сидений",     "Seat module",      Bdy, "K-bus",  "0x76", 1998, 2008),

    m!("ZGM",  "Центральный шлюз",   "Central gateway",  Gwy, "CAN",    "0x10", 2001, 9999),
    m!("KOMBI","Приборная панель",   "Instrument cluster",Gwy, "K-bus", "0x80", 1990, 9999),
    m!("DWA",  "Сигнализация",       "Alarm system",     Gwy, "K-bus",  "0x29", 1998, 9999),

    m!("RAD",  "Магнитола",          "Radio",            Inf, "I-bus",  "0x68", 1994, 2004),
    m!("CCC",  "Головное устройство","Head unit (CCC/CIC)",Inf,"MOST",   "0x60", 2004, 9999),
    m!("NAV",  "Навигация",          "Navigation",       Inf, "I-bus",  "0x7E", 1996, 2008),
    m!("DSP",  "Аудиоусилитель",     "DSP amplifier",    Inf, "I-bus",  "0x6A", 1998, 2008),
    m!("TEL",  "Телефон",            "Telephone",        Inf, "I-bus",  "0x46", 1997, 2008),
];

/// Parse "1998–2006" (en-dash) into (start, end).
fn parse_years(s: &str) -> (u32, u32) {
    let mut it = s.split('–').map(|p| p.trim().parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(9999))
}

/// Is an engine marking a diesel? (BMW M/N diesel families.)
pub fn is_diesel(mark: &str) -> bool {
    matches!(mark, "M41" | "M47" | "M51" | "M57" | "M67" | "N47" | "N57")
}

/// Build an engine module for one marking on a given chassis. Only DDE4.0 diesels
/// (M47/M57) on the DS2 K-line chassis are really connectable today (DDE40KW0.prg);
/// everything else is a catalog entry (`prg: None`, mock status) until its
/// transport (D-CAN/KWP2000/DME) lands.
fn engine_module(mark: &'static str, chassis: &str) -> Module {
    if is_diesel(mark) {
        // DS2 DDE4.0 era: E38 730d, E39 525/530d, E46 320/330d, E53/E83 3.0d/2.0d.
        let ds2 = matches!(chassis, "E38" | "E39" | "E46" | "E53" | "E83")
            && matches!(mark, "M47" | "M57");
        Module { code: mark, ru: "Двигатель · дизель", en: "Engine · diesel",
                 cat: Category::Pwr, bus: if ds2 { "DS2" } else { "D-CAN" }, addr: "0x12",
                 from: 0, to: 9999, awd_only: false,
                 // The DDE4.0 diesel screen script; its .ipo yields SGBD DDE40KW0.
                 script: if ds2 { Some("DDE40") } else { None },
                 prg: if ds2 { Some("DDE40KW0.prg") } else { None }, validated: ds2 }
    } else {
        Module { code: mark, ru: "Двигатель · бензин", en: "Engine · petrol",
                 cat: Category::Pwr, bus: "PT-CAN", addr: "0x12", from: 0, to: 9999,
                 awd_only: false, script: None, prg: None, validated: false }
    }
}

/// Build a Module from a CFGDAT catalog entry. `bus`/`addr` come from the ECU's own
/// `.prg` CommParameter (static concept probe, emitted by the generator) — no longer a
/// validated-DS2 placeholder.
fn catalog_module(e: &crate::catalog::CatEntry) -> Module {
    Module {
        code: e.code, ru: e.ru, en: e.en, cat: e.cat,
        bus: e.bus, addr: e.addr,
        // The CFGDAT catalog `code` IS the INPA script name (e.g. "DDE40", "ascdsc46") →
        // its `SGDAT/<code>.ipo` is the screen source.
        from: 0, to: 9999, awd_only: false, script: Some(e.code), prg: e.prg,
        validated: e.validated,
    }
}

/// Modules present on a chassis. Prefer the real INPA CFGDAT catalog (categories,
/// ECU list, engine markings, .prg mapping); fall back to the engine-marking
/// generator + static modules for chassis not covered by CFGDAT.
pub fn mods_for(ch: &Chassis) -> Vec<Module> {
    let cat = crate::catalog::entries_for(ch.code);
    if !cat.is_empty() {
        return cat.iter().map(catalog_module).collect();
    }
    let (y0, y1) = parse_years(ch.years);
    let awd = ch.drive.iter().any(|d| *d == "AWD");
    let mut out: Vec<Module> = ch.eng.iter().map(|&mark| engine_module(mark, ch.code)).collect();
    out.extend(
        MODS.iter()
            .filter(|m| m.from <= y1 && m.to >= y0 && (!m.awd_only || awd))
            .copied(),
    );
    out
}

