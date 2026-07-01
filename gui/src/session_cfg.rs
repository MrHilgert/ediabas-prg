//! Config-driven ECU session model, ported from the prototype `screensFor()` /
//! `streamGroups()`. One templated screen renders any module from this data:
//! a page tree of typed blocks + positional F-keys, driven by a navigation stack.

use std::collections::HashMap;

use crate::ecu::{Category, Module};
use crate::lang::Lang;

/// MW_SELECT_LESEN_NORM selector string for the DDE live stream (10 channels) —
/// the known-good batch from the transport work.
pub const DDE_STREAM_JOB: &str = "MW_SELECT_LESEN_NORM";
pub const DDE_STREAM_ARGS: &str = "0F100F650F300F320F400F420F800F831F5D1F5E";
pub const DDE_STREAM_PAGE: &str = "stream_sg_dde";

/// Bilingual static label.
#[derive(Clone, Copy)]
pub struct Loc(pub &'static str, pub &'static str);
impl Loc {
    pub fn get(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::Ru => self.0,
            Lang::En => self.1,
        }
    }
}
fn loc(ru: &'static str, en: &'static str) -> Loc {
    Loc(ru, en)
}

/// What a positional F-key does.
#[derive(Clone)]
pub enum FAction {
    To(String),
    Back,
    Exit,
    Read,
    Clear,
    Noop,
}

#[derive(Clone)]
pub struct FKey {
    pub f: u8,
    pub label: Loc,
    pub act: FAction,
}

#[derive(Clone)]
pub struct MenuItem {
    pub label: Loc,
    pub to: String,
}

#[derive(Clone)]
pub struct StreamParam {
    pub label: Loc,
    pub unit: &'static str,
    pub base: f32,
    pub amp: f32,
    pub min: f32,
    pub max: f32,
    pub dec: u8,
    pub is_bool: bool,
    pub lo: Option<f32>,
    pub hi: Option<f32>,
    /// Real result base name (e.g. "STAT_dzmNmit"); value=`{bind}_WERT`,
    /// unit=`{bind}_EINH` from the polled `JobResult`. `None` = mock sine.
    pub bind: Option<&'static str>,
}

impl StreamParam {
    /// Bind this parameter to a real measurement result name.
    fn bound(mut self, base: &'static str) -> Self {
        self.bind = Some(base);
        self
    }
}

#[allow(clippy::too_many_arguments)]
fn sp(
    ru: &'static str,
    en: &'static str,
    unit: &'static str,
    base: f32,
    amp: f32,
    min: f32,
    max: f32,
    dec: u8,
    lo: Option<f32>,
    hi: Option<f32>,
    is_bool: bool,
) -> StreamParam {
    StreamParam { label: loc(ru, en), unit, base, amp, min, max, dec, is_bool, lo, hi, bind: None }
}

/// The DDE (diesel) live-data group — 10 real channels polled via
/// MW_SELECT_LESEN_NORM. Ranges are display-only; values/units come from the ECU.
fn dde_group() -> StreamGroup {
    StreamGroup {
        id: "sg_dde",
        name: loc("Двигатель (дизель)", "Engine (diesel)"),
        params: vec![
            sp("Обороты", "Engine speed", "1/min", 0.0, 0.0, 0.0, 6000.0, 0, None, Some(0.82), false).bound("STAT_dzmNmit"),
            sp("Напряжение АКБ", "Battery voltage", "V", 0.0, 0.0, 0.0, 16.0, 2, Some(0.72), None, false).bound("STAT_anmUBT"),
            sp("Расход воздуха, факт", "Air mass, act.", "mg/Hub", 0.0, 0.0, 0.0, 900.0, 0, None, None, false).bound("STAT_armM_List"),
            sp("Расход воздуха, задан", "Air mass, set", "mg/Hub", 0.0, 0.0, 0.0, 900.0, 0, None, None, false).bound("STAT_armM_Lsoll"),
            sp("Наддув, факт", "Boost, act.", "mbar", 0.0, 0.0, 0.0, 2500.0, 0, None, None, false).bound("STAT_ldmP_Llin"),
            sp("Наддув, задан", "Boost, set", "mbar", 0.0, 0.0, 0.0, 2500.0, 0, None, None, false).bound("STAT_ldmP_Lsoll"),
            sp("Давление в рейле, факт", "Rail pressure, act.", "bar", 0.0, 0.0, 0.0, 1800.0, 0, None, None, false).bound("STAT_zumP_RAIL"),
            sp("Давление в рейле, задан", "Rail pressure, set", "bar", 0.0, 0.0, 0.0, 1800.0, 0, None, None, false).bound("STAT_zumPQsoll"),
            sp("Кол-во топлива", "Injection qty", "mm^3", 0.0, 0.0, 0.0, 100.0, 1, None, None, false).bound("STAT_mrmM_EAKT"),
            sp("Положение педали", "Pedal position", "%", 0.0, 0.0, 0.0, 100.0, 1, None, None, false).bound("STAT_mrmPWGfi"),
        ],
    }
}

/// Stream groups for a module: the real DDE diesel group when it's the DDE
/// reference (has a `.prg`), otherwise the category-based mock groups.
pub fn stream_groups_for(m: &Module) -> Vec<StreamGroup> {
    if m.prg == Some("DDE40KW0.prg") {
        vec![dde_group()]
    } else {
        stream_groups(m.cat)
    }
}

pub struct StreamGroup {
    pub id: &'static str,
    pub name: Loc,
    pub params: Vec<StreamParam>,
}

pub enum Block {
    Note(Loc),
    NoteOnline, // "Модуль {code} на связи…"
    Menu(Vec<MenuItem>),
    Kv,
    Faults,
    Stream(Vec<StreamParam>),
    Status,
}

pub struct Page {
    pub title: Loc,
    pub blocks: Vec<Block>,
    pub keys: Vec<FKey>,
}

/// Fault-description pool per category (prototype `FAULTDESC`).
pub fn fault_pool(cat: Category) -> &'static [Loc] {
    match cat {
        Category::Pwr => &[
            Loc("Пропуски воспламенения", "Misfire detected"),
            Loc("Датчик кислорода, банк 1", "O2 sensor, bank 1"),
            Loc("Клапан VANOS", "VANOS solenoid"),
            Loc("Форсунка, обрыв цепи", "Injector open circuit"),
            Loc("Датчик детонации", "Knock sensor"),
            Loc("Смесь слишком бедная", "Mixture too lean"),
        ],
        Category::Chs => &[
            Loc("Датчик скорости колеса", "Wheel speed sensor"),
            Loc("Датчик угла руля", "Steering angle sensor"),
            Loc("Датчик давления в тормозах", "Brake pressure sensor"),
            Loc("Клапан гидроблока", "Hydraulic valve"),
        ],
        Category::Saf => &[
            Loc("Цепь подушки водителя", "Driver airbag circuit"),
            Loc("Преднатяжитель ремня", "Belt pretensioner"),
            Loc("Датчик удара", "Crash sensor"),
        ],
        Category::Bdy => &[
            Loc("Обрыв цепи освещения", "Lighting circuit open"),
            Loc("Концевик двери", "Door contact switch"),
            Loc("Мотор стеклоподъёмника", "Window motor"),
            Loc("Датчик температуры салона", "Cabin temp sensor"),
        ],
        Category::Gwy => &[
            Loc("Потеря связи по шине", "Bus communication lost"),
            Loc("Рассогласование версий ПО", "Software version mismatch"),
        ],
        Category::Inf => &[
            Loc("Обрыв кольца MOST", "MOST ring break"),
            Loc("Динамик, обрыв цепи", "Speaker open circuit"),
            Loc("Неисправность антенны", "Antenna fault"),
        ],
    }
}

/// Parameter groups for the data stream, keyed by module category.
pub fn stream_groups(cat: Category) -> Vec<StreamGroup> {
    match cat {
        Category::Pwr => vec![
            StreamGroup {
                id: "sg_eng",
                name: loc("Двигатель", "Engine"),
                params: vec![
                    sp("Обороты", "Engine speed", "1/min", 815.0, 70.0, 0.0, 7000.0, 0, None, Some(0.82), false),
                    sp("Температура ОЖ", "Coolant temp", "°C", 88.0, 2.0, 0.0, 130.0, 0, None, Some(0.7), false),
                    sp("Расход воздуха", "Air mass", "kg/h", 12.6, 1.4, 0.0, 900.0, 1, None, None, false),
                    sp("Нагрузка", "Load", "%", 17.0, 3.0, 0.0, 100.0, 1, None, Some(0.85), false),
                    sp("Давление в рейле", "Rail pressure", "bar", 300.0, 15.0, 0.0, 1800.0, 0, None, None, false),
                    sp("Давление наддува", "Boost pressure", "mbar", 1002.0, 30.0, 0.0, 2500.0, 0, None, None, false),
                    sp("Температура впуска", "Intake temp", "°C", 30.0, 1.0, -40.0, 120.0, 0, None, None, false),
                    sp("Положение педали", "Pedal position", "%", 0.0, 2.0, 0.0, 100.0, 1, None, None, false),
                    sp("Напряжение АКБ", "Battery voltage", "V", 14.1, 0.08, 0.0, 16.0, 1, Some(0.75), Some(0.95), false),
                    sp("Температура топлива", "Fuel temp", "°C", 45.0, 1.0, -40.0, 120.0, 0, None, None, false),
                ],
            },
            StreamGroup {
                id: "sg_fuel",
                name: loc("Топливоподача", "Fuel system"),
                params: vec![
                    sp("Давление топлива", "Fuel pressure", "bar", 3.8, 0.08, 0.0, 6.0, 1, Some(0.5), None, false),
                    sp("Коррекция аддит.", "Additive trim", "%", 1.4, 0.8, -25.0, 25.0, 1, None, None, false),
                    sp("Коррекция мульт.", "Multipl. trim", "%", -0.6, 0.8, -25.0, 25.0, 1, None, None, false),
                    sp("Время впрыска", "Injection time", "ms", 3.1, 0.25, 0.0, 20.0, 2, None, None, false),
                ],
            },
            StreamGroup {
                id: "sg_elec",
                name: loc("Электрика", "Electrical"),
                params: vec![
                    sp("Напряжение АКБ", "Battery voltage", "V", 14.1, 0.08, 0.0, 16.0, 1, Some(0.75), Some(0.95), false),
                    sp("Температура впуска", "Intake temp", "°C", 30.0, 1.0, -40.0, 120.0, 0, None, None, false),
                    sp("Положение ДЗ", "Throttle pos.", "%", 3.1, 0.4, 0.0, 100.0, 1, None, None, false),
                ],
            },
        ],
        Category::Chs => vec![
            StreamGroup {
                id: "sg_whl",
                name: loc("Скорости колёс", "Wheel speeds"),
                params: vec![
                    sp("Переднее левое", "Front left", "km/h", 0.0, 0.0, 0.0, 300.0, 0, None, None, false),
                    sp("Переднее правое", "Front right", "km/h", 0.0, 0.0, 0.0, 300.0, 0, None, None, false),
                    sp("Заднее левое", "Rear left", "km/h", 0.0, 0.0, 0.0, 300.0, 0, None, None, false),
                    sp("Заднее правое", "Rear right", "km/h", 0.0, 0.0, 0.0, 300.0, 0, None, None, false),
                ],
            },
            StreamGroup {
                id: "sg_dyn",
                name: loc("Динамика", "Dynamics"),
                params: vec![
                    sp("Угол руля", "Steering angle", "°", 0.0, 3.0, -540.0, 540.0, 1, None, None, false),
                    sp("Поперечное уск.", "Lateral acc.", "m/s²", 0.0, 0.25, -12.0, 12.0, 2, None, None, false),
                    sp("Скорость рыскания", "Yaw rate", "°/s", 0.0, 1.0, -75.0, 75.0, 1, None, None, false),
                    sp("Давление тормоза", "Brake pressure", "bar", 0.0, 0.0, 0.0, 250.0, 1, None, None, false),
                ],
            },
        ],
        _ => vec![StreamGroup {
            id: "sg_gen",
            name: loc("Основные", "General"),
            params: vec![
                sp("Напряжение питания", "Supply voltage", "V", 13.9, 0.08, 0.0, 16.0, 1, None, None, false),
                sp("Внутр. температура", "Internal temp", "°C", 33.0, 1.0, -40.0, 120.0, 0, None, None, false),
                sp("Клемма 15", "Terminal 15", "", 1.0, 0.0, 0.0, 1.0, 0, None, None, true),
                sp("Клемма 30", "Terminal 30", "V", 12.4, 0.05, 0.0, 16.0, 1, None, None, false),
            ],
        }],
    }
}

/// The full page tree for a module.
pub fn screens_for(m: &Module) -> HashMap<String, Page> {
    let mut s = HashMap::new();
    let groups = stream_groups_for(m);

    s.insert(
        "main".to_string(),
        Page {
            title: loc("ГЛАВНОЕ МЕНЮ", "MAIN MENU"),
            blocks: vec![
                Block::NoteOnline,
                Block::Menu(vec![
                    MenuItem { label: loc("Идентификация блока", "Module identification"), to: "ident".into() },
                    MenuItem { label: loc("Память ошибок", "Fault memory"), to: "faults".into() },
                    MenuItem { label: loc("Поток данных", "Data stream"), to: "stream".into() },
                    MenuItem { label: loc("Состояние блока", "Module status"), to: "status".into() },
                    MenuItem { label: loc("Активация компонентов", "Component activation"), to: "act".into() },
                ]),
            ],
            keys: vec![
                FKey { f: 1, label: loc("ИДЕНТ", "IDENT"), act: FAction::To("ident".into()) },
                FKey { f: 2, label: loc("ОШИБКИ", "FAULTS"), act: FAction::To("faults".into()) },
                FKey { f: 3, label: loc("ПОТОК", "STREAM"), act: FAction::To("stream".into()) },
                FKey { f: 4, label: loc("СТАТУС", "STATUS"), act: FAction::To("status".into()) },
                FKey { f: 5, label: loc("АКТИВ.", "ACTIV."), act: FAction::To("act".into()) },
                FKey { f: 6, label: loc("ВЫХОД", "EXIT"), act: FAction::Exit },
            ],
        },
    );

    s.insert(
        "ident".into(),
        Page {
            title: loc("ИДЕНТИФИКАЦИЯ", "IDENTIFICATION"),
            blocks: vec![Block::Kv],
            keys: vec![FKey { f: 6, label: loc("НАЗАД", "BACK"), act: FAction::Back }],
        },
    );

    s.insert(
        "faults".into(),
        Page {
            title: loc("ПАМЯТЬ ОШИБОК", "FAULT MEMORY"),
            blocks: vec![Block::Faults],
            keys: vec![
                FKey { f: 1, label: loc("ЧИТАТЬ", "READ"), act: FAction::Read },
                FKey { f: 2, label: loc("СБРОС", "CLEAR"), act: FAction::Clear },
                FKey { f: 6, label: loc("НАЗАД", "BACK"), act: FAction::Back },
            ],
        },
    );

    let stream_menu: Vec<MenuItem> = groups
        .iter()
        .map(|g| MenuItem { label: g.name, to: format!("stream_{}", g.id) })
        .collect();
    s.insert(
        "stream".into(),
        Page {
            title: loc("ПОТОК ДАННЫХ", "DATA STREAM"),
            blocks: vec![
                Block::Note(loc("Выберите группу параметров.", "Select a parameter group.")),
                Block::Menu(stream_menu),
            ],
            keys: vec![FKey { f: 6, label: loc("НАЗАД", "BACK"), act: FAction::Back }],
        },
    );
    for g in groups {
        s.insert(
            format!("stream_{}", g.id),
            Page {
                title: g.name,
                blocks: vec![Block::Stream(g.params)],
                keys: vec![FKey { f: 6, label: loc("НАЗАД", "BACK"), act: FAction::Back }],
            },
        );
    }

    s.insert(
        "status".into(),
        Page {
            title: loc("СОСТОЯНИЕ", "STATUS"),
            blocks: vec![Block::Status],
            keys: vec![FKey { f: 6, label: loc("НАЗАД", "BACK"), act: FAction::Back }],
        },
    );

    s.insert(
        "act".into(),
        Page {
            title: loc("АКТИВАЦИЯ", "ACTIVATION"),
            blocks: vec![
                Block::Note(loc("Выберите компонент для активации.", "Select a component to activate.")),
                Block::Menu(vec![
                    MenuItem { label: loc("Реле топливного насоса", "Fuel pump relay"), to: "act_run".into() },
                    MenuItem { label: loc("Вентилятор радиатора", "Radiator fan"), to: "act_run".into() },
                    MenuItem { label: loc("Лампа Check-Engine", "Check-engine lamp"), to: "act_run".into() },
                ]),
            ],
            keys: vec![FKey { f: 6, label: loc("НАЗАД", "BACK"), act: FAction::Back }],
        },
    );
    s.insert(
        "act_run".into(),
        Page {
            title: loc("АКТИВАЦИЯ", "ACTIVATION"),
            blocks: vec![Block::Note(loc(
                "F1 — запустить активацию. Двигатель на холостом ходу, стояночный тормоз включён.",
                "F1 — run activation. Engine idling, parking brake engaged.",
            ))],
            keys: vec![
                FKey { f: 1, label: loc("ПУСК", "RUN"), act: FAction::Noop },
                FKey { f: 6, label: loc("НАЗАД", "BACK"), act: FAction::Back },
            ],
        },
    );

    s
}
