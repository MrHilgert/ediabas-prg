//! ECU model for screen 2 (ECU Select), ported from the prototype `CATS` / `MODS` +
//! `modsFor()` (year-overlap + AWD filter) and `enrich()` (deterministic status/faults
//! from a code hash — mock, until a real scan replaces it).

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
    pub fn id(self) -> &'static str {
        match self {
            Category::Pwr => "PWR",
            Category::Chs => "CHS",
            Category::Saf => "SAF",
            Category::Bdy => "BDY",
            Category::Gwy => "GWY",
            Category::Inf => "INF",
        }
    }
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModStatus {
    Ok,
    Fault,
    NoLink,
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
    /// Concrete SGBD for modules we can really talk to (currently DS2 only).
    /// `Some(..)` marks a real, connectable module; `None` = mock catalog entry.
    pub prg: Option<&'static str>,
}

impl Module {
    pub fn name(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::Ru => self.ru,
            Lang::En => self.en,
        }
    }

    /// Real modules (with a `.prg`) are deterministically OK/connectable;
    /// the rest use the hash-based mock status.
    pub fn status(&self) -> (ModStatus, u32) {
        if self.prg.is_some() {
            (ModStatus::Ok, 0)
        } else {
            enrich(self.code)
        }
    }
}

macro_rules! m {
    ($code:literal, $ru:literal, $en:literal, $cat:ident, $bus:literal, $addr:literal, $from:literal, $to:literal, awd) => {
        Module { code: $code, ru: $ru, en: $en, cat: Category::$cat, bus: $bus, addr: $addr,
                 from: $from, to: $to, awd_only: true, prg: None }
    };
    ($code:literal, $ru:literal, $en:literal, $cat:ident, $bus:literal, $addr:literal, $from:literal, $to:literal) => {
        Module { code: $code, ru: $ru, en: $en, cat: Category::$cat, bus: $bus, addr: $addr,
                 from: $from, to: $to, awd_only: false, prg: None }
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
                 prg: if ds2 { Some("DDE40KW0.prg") } else { None } }
    } else {
        Module { code: mark, ru: "Двигатель · бензин", en: "Engine · petrol",
                 cat: Category::Pwr, bus: "PT-CAN", addr: "0x12", from: 0, to: 9999,
                 awd_only: false, prg: None }
    }
}

/// Modules present on a chassis: the engine block expanded into this chassis'
/// engine markings, then the non-engine static modules (year/AWD filtered).
pub fn mods_for(ch: &Chassis) -> Vec<Module> {
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

/// Stable hash of a module code (used for deterministic mock ident/faults).
pub fn code_hash(code: &str) -> u32 {
    let mut h: u32 = 0;
    for ch in code.bytes() {
        h = (h.wrapping_mul(31).wrapping_add(ch as u32)) % 99991;
    }
    h
}

/// Deterministic mock status + fault count from the module code hash
/// (mirrors the prototype `enrich()`; real values come from an actual scan later).
pub fn enrich(code: &str) -> (ModStatus, u32) {
    let h = code_hash(code);
    let r = h % 12;
    if r == 0 {
        (ModStatus::NoLink, 0)
    } else if r < 3 {
        (ModStatus::Fault, (h % 3) + 1)
    } else {
        (ModStatus::Ok, 0)
    }
}
