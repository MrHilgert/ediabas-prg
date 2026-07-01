//! Static vehicle catalog, ported 1:1 from the Chassis Select handoff `DATA`,
//! `SNAME` and `BODY` dictionaries. Codes/years are language-neutral.

use crate::lang::Lang;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ready,
    Partial,
    None,
}

pub struct Chassis {
    pub code: &'static str,
    pub series: char, // '1','3','5','6','7','8','X','Z'
    pub ru: &'static str,
    pub en: &'static str,
    pub years: &'static str,
    pub bodies: &'static [&'static str],
    pub drive: &'static [&'static str],
    pub eng: &'static [&'static str],
    pub ecus: u32,
    pub status: Status,
}

impl Chassis {
    pub fn name(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::Ru => self.ru,
            Lang::En => self.en,
        }
    }
}

/// Series order for the left rail.
pub const SERIES: [char; 8] = ['1', '3', '5', '6', '7', '8', 'X', 'Z'];

pub fn series_name(lang: Lang, s: char) -> &'static str {
    match (lang, s) {
        (Lang::Ru, '1') => "1 серия",
        (Lang::Ru, '3') => "3 серия",
        (Lang::Ru, '5') => "5 серия",
        (Lang::Ru, '6') => "6 серия",
        (Lang::Ru, '7') => "7 серия",
        (Lang::Ru, '8') => "8 серия",
        (Lang::Ru, 'X') => "X модели",
        (Lang::Ru, 'Z') => "Z модели",
        (Lang::En, '1') => "1 Series",
        (Lang::En, '3') => "3 Series",
        (Lang::En, '5') => "5 Series",
        (Lang::En, '6') => "6 Series",
        (Lang::En, '7') => "7 Series",
        (Lang::En, '8') => "8 Series",
        (Lang::En, 'X') => "X Models",
        (Lang::En, 'Z') => "Z Models",
        _ => "",
    }
}

pub fn body_name(lang: Lang, code: &str) -> &'static str {
    match (lang, code) {
        (Lang::Ru, "SED") => "седан",
        (Lang::Ru, "TOU") => "универсал",
        (Lang::Ru, "CPE") => "купе",
        (Lang::Ru, "CAB") => "кабрио",
        (Lang::Ru, "HB3") => "хэтч 3д",
        (Lang::Ru, "HB5") => "хэтч 5д",
        (Lang::Ru, "SAV") => "кроссовер",
        (Lang::Ru, "ROD") => "родстер",
        (Lang::Ru, "CMP") => "компакт",
        (Lang::Ru, "LWB") => "дл. база",
        (Lang::En, "SED") => "Sedan",
        (Lang::En, "TOU") => "Touring",
        (Lang::En, "CPE") => "Coupé",
        (Lang::En, "CAB") => "Cabrio",
        (Lang::En, "HB3") => "3-dr Hatch",
        (Lang::En, "HB5") => "5-dr Hatch",
        (Lang::En, "SAV") => "SAV",
        (Lang::En, "ROD") => "Roadster",
        (Lang::En, "CMP") => "Compact",
        (Lang::En, "LWB") => "LWB",
        _ => "—",
    }
}

macro_rules! ch {
    ($code:literal, $s:literal, $ru:literal, $en:literal, $years:literal,
     [$($b:literal),*], [$($d:literal),*], [$($e:literal),*], $ecus:literal, $st:ident) => {
        Chassis {
            code: $code, series: $s, ru: $ru, en: $en, years: $years,
            bodies: &[$($b),*], drive: &[$($d),*], eng: &[$($e),*],
            ecus: $ecus, status: Status::$st,
        }
    };
}

pub const DATA: &[Chassis] = &[
    ch!("E81", '1', "1 серия", "1 Series", "2007–2012", ["HB3"], ["RWD"], ["N43","N45","N47","N54"], 31, Partial),
    ch!("E82", '1', "1 серия", "1 Series", "2007–2013", ["CPE","CAB"], ["RWD"], ["N51","N52","N54","N55"], 34, Ready),
    ch!("E87", '1', "1 серия", "1 Series", "2004–2011", ["HB5"], ["RWD"], ["N43","N45","N46","N47"], 30, Ready),
    ch!("E88", '1', "1 серия", "1 Series", "2008–2013", ["CAB"], ["RWD"], ["N43","N52","N54"], 33, Partial),

    ch!("E21", '3', "3 серия", "3 Series", "1975–1983", ["SED"], ["RWD"], ["M10","M20"], 4, None),
    ch!("E30", '3', "3 серия", "3 Series", "1982–1994", ["SED","TOU","CPE","CAB"], ["RWD","AWD"], ["M10","M20","M40","M42","S14"], 9, Partial),
    ch!("E36", '3', "3 серия", "3 Series", "1990–2000", ["SED","TOU","CPE","CAB","CMP"], ["RWD"], ["M43","M50","M52","M44","S50","S52"], 18, Ready),
    ch!("E46", '3', "3 серия", "3 Series", "1998–2006", ["SED","TOU","CPE","CAB"], ["RWD","AWD"], ["M43","M52","M54","M47","M57","S54"], 28, Ready),
    ch!("E90", '3', "3 серия", "3 Series", "2005–2013", ["SED","TOU","CPE","CAB"], ["RWD","AWD"], ["N43","N46","N52","N53","N54","N55","N47","M57","S65"], 41, Ready),

    ch!("E12", '5', "5 серия", "5 Series", "1972–1981", ["SED"], ["RWD"], ["M10","M30"], 3, None),
    ch!("E28", '5', "5 серия", "5 Series", "1981–1988", ["SED"], ["RWD"], ["M10","M20","M30","M21"], 6, None),
    ch!("E34", '5', "5 серия", "5 Series", "1988–1996", ["SED","TOU"], ["RWD","AWD"], ["M20","M40","M50","M60","M30","S38"], 14, Partial),
    ch!("E39", '5', "5 серия", "5 Series", "1995–2003", ["SED","TOU"], ["RWD"], ["M52","M54","M62","M57","S62"], 24, Ready),
    ch!("E60", '5', "5 серия", "5 Series", "2003–2010", ["SED","TOU"], ["RWD","AWD"], ["N52","N53","N54","M54","N62","M57","S85"], 38, Ready),

    ch!("E24", '6', "6 серия", "6 Series", "1976–1989", ["CPE"], ["RWD"], ["M30","M88","S38"], 5, None),
    ch!("E63", '6', "6 серия", "6 Series", "2003–2010", ["CPE","CAB"], ["RWD"], ["N62","N52","S85"], 36, Ready),

    ch!("E23", '7', "7 серия", "7 Series", "1977–1986", ["SED"], ["RWD"], ["M30"], 5, None),
    ch!("E32", '7', "7 серия", "7 Series", "1986–1994", ["SED","LWB"], ["RWD"], ["M30","M60","M70"], 12, Partial),
    ch!("E38", '7', "7 серия", "7 Series", "1994–2001", ["SED","LWB"], ["RWD"], ["M52","M62","M57","M73"], 26, Ready),
    ch!("E65", '7', "7 серия", "7 Series", "2001–2008", ["SED","LWB"], ["RWD"], ["N62","N52","M67","N73"], 44, Ready),

    ch!("E31", '8', "8 серия", "8 Series", "1989–1999", ["CPE"], ["RWD"], ["M60","M62","M70","M73","S70"], 16, Partial),

    ch!("E53", 'X', "X5", "X5", "1999–2006", ["SAV"], ["AWD"], ["M54","M62","N62","M57"], 27, Ready),
    ch!("E70", 'X', "X5", "X5", "2006–2013", ["SAV"], ["AWD"], ["N52","N55","N62","M57","S63"], 42, Ready),
    ch!("E83", 'X', "X3", "X3", "2003–2010", ["SAV"], ["AWD"], ["M54","N52","M47","N47"], 29, Partial),
    ch!("E71", 'X', "X6", "X6", "2008–2014", ["SAV"], ["AWD"], ["N54","N55","N63","M57","S63"], 43, Ready),

    ch!("E36/7", 'Z', "Z3", "Z3", "1995–2002", ["ROD","CPE"], ["RWD"], ["M43","M44","M52","M54","S50","S52","S54"], 17, Partial),
    ch!("E85", 'Z', "Z4", "Z4", "2002–2008", ["ROD","CPE"], ["RWD"], ["N46","M54","N52","S54"], 30, Ready),
    ch!("E52", 'Z', "Z8", "Z8", "1999–2003", ["ROD"], ["RWD"], ["S62"], 19, Partial),
];

/// Number of chassis in a series.
pub fn series_count(s: char) -> usize {
    DATA.iter().filter(|c| c.series == s).count()
}

/// Case-insensitive match of a chassis against a search query (code / ru / en).
pub fn matches_query(c: &Chassis, q: &str) -> bool {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    c.code.to_lowercase().contains(&q)
        || c.ru.to_lowercase().contains(&q)
        || c.en.to_lowercase().contains(&q)
}
