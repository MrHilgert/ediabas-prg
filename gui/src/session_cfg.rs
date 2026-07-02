//! Config-driven ECU session model, ported from the prototype `screensFor()` /
//! `streamGroups()`. One templated screen renders any module from this data:
//! a page tree of typed blocks + positional F-keys, driven by a navigation stack.

use std::collections::HashMap;

use crate::ecu::{Category, Module};
use crate::lang::Lang;

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

/// The page tree plus, for pages that show live data, the poll to run while active:
/// a request LIST `(reqs, interval_ms)` where each `req = (job, args)` mirrors INPA's
/// own call — `("MW_SELECT_LESEN_NORM", "<selector>")` or `("STATUS_*", "")`. The
/// worker runs every req per tick and merges the results into one view.
pub struct Screens {
    pub pages: HashMap<String, Page>,
    pub polls: HashMap<String, (Vec<(String, String)>, u64)>,
}

/// One INPA data-stream screen: the ordered params it displays (for the bars) plus
/// the exact per-parameter job calls INPA issues (`reqs`), all from the ECU's `.ipo`
/// (see `screens_gen::MeasScreen`). Built on connect by [`build_meas_groups`].
#[derive(Clone)]
pub struct MeasGroup {
    pub id: String,
    pub title: &'static str,
    pub params: Vec<StreamParam>,
    pub reqs: Vec<(String, String)>, // (job, arg) per parameter, in INPA's order
}

/// Build the live-measurement groups for a module code from the generated,
/// `.ipo`-curated screens (`screens_gen::meas_screens`). Each INPA screen becomes one
/// group; we replicate INPA's exact requests and bind each bar to its `STAT_*` result.
/// Returns empty if the ECU has no extracted meas screens (caller falls back).
pub fn build_meas_groups(code: &str) -> Vec<MeasGroup> {
    crate::screens_gen::meas_screens(code)
        .iter()
        .enumerate()
        .map(|(gi, sc)| {
            let params = sc
                .reqs
                .iter()
                .map(|r| {
                    let (min, max, dec) = if r.lo == 0.0 && r.hi == 0.0 {
                        range_for(r.unit, r.label)
                    } else {
                        (r.lo, r.hi, if r.hi - r.lo <= 20.0 { 2 } else { 0 })
                    };
                    StreamParam {
                        label: Loc(r.label, r.label),
                        unit: r.unit,
                        base: 0.0,
                        amp: 0.0,
                        min,
                        max,
                        dec,
                        is_bool: false,
                        lo: None,
                        hi: None,
                        bind: Some(r.bind),
                    }
                })
                .collect();
            let reqs = sc
                .reqs
                .iter()
                .map(|r| (r.job.to_string(), r.arg.to_string()))
                .collect();
            MeasGroup { id: format!("stream_meas_{gi}"), title: sc.title, params, reqs }
        })
        .collect()
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

/// Build live-stream bars from an extracted INPA screen: each row-column becomes a
/// param bound to its result name (value + unit come from the polled `JobResult`;
/// no data yet → "—"). Display ranges are label heuristics until real ranges arrive
/// from the `.prg` measurement table (Phase 4).
fn sg_stream_params(sc: &'static crate::screens_gen::SgScreen) -> Vec<StreamParam> {
    let is_bool = matches!(sc.kind, crate::screens_gen::SgKind::Digital);
    let mut out = Vec::new();
    for row in sc.rows {
        let cols = row.labels.len().max(row.results.len());
        for i in 0..cols {
            let label = row
                .labels
                .get(i)
                .copied()
                .filter(|s| !s.is_empty())
                .or_else(|| row.results.get(i).copied())
                .unwrap_or("");
            // Always bound (never the mock-sine path): a real result name when the
            // screen has one, else "" so the bar shows "—" until the value is polled.
            let bind = Some(row.results.get(i).copied().unwrap_or(""));
            let (min, max, dec) = range_for_label(label, is_bool);
            out.push(StreamParam {
                label: Loc(label, label),
                unit: "",
                base: 0.0,
                amp: 0.0,
                min,
                max,
                dec,
                is_bool,
                lo: None,
                hi: None,
                bind,
            });
        }
    }
    out
}

/// Coarse display range for bar scaling (the numeric value shown is always the exact
/// polled value). Prefers the measurement unit, falls back to label keywords.
fn range_for(unit: &str, label: &str) -> (f32, f32, u8) {
    match unit.trim() {
        "1/min" => return (0.0, 7000.0, 0),
        "V" => return (0.0, 16.0, 1),
        "%" => return (0.0, 100.0, 1),
        "°C" | "grad C" | "GradC" => return (-40.0, 150.0, 0),
        "bar" => return (0.0, 2000.0, 0),
        "mbar" => return (0.0, 2500.0, 0),
        "km/h" => return (0.0, 300.0, 0),
        _ => {}
    }
    range_for_label(label, false)
}

/// Coarse display range from a measurement label (bar scaling only — the numeric
/// value shown is always the exact polled value). Real ranges/units come from the
/// `.prg` table later; this just keeps bars sensible meanwhile.
fn range_for_label(label: &str, is_bool: bool) -> (f32, f32, u8) {
    if is_bool {
        return (0.0, 1.0, 0);
    }
    let l = label.to_lowercase();
    let has = |ks: &[&str]| ks.iter().any(|k| l.contains(k));
    if has(&["speed", "drehzahl", "rpm", "1/min", "оборот"]) {
        (0.0, 7000.0, 0)
    } else if has(&["temp", "°c", "temperatur", "температ", "коленвал"]) {
        (-40.0, 150.0, 0)
    } else if has(&["rail", "boost", "ladedruck", "pressure", "druck", "давлен", "наддув"]) {
        (0.0, 2500.0, 0)
    } else if has(&["volt", "spannung", "напряж"]) {
        (0.0, 16.0, 1)
    } else if has(&["mass", "masse", "luftmasse", "расход возд"]) {
        (0.0, 900.0, 0)
    } else if has(&["%", "percent", "prozent", "position", "pedal", "load", "last", "нагруз", "педал", "положен"]) {
        (0.0, 100.0, 1)
    } else if has(&["angle", "winkel", "угол"]) {
        (-540.0, 540.0, 0)
    } else {
        (0.0, 100.0, 1)
    }
}

/// The full page tree for a module, plus per-page live polls. `meas_groups` are the
/// batched measurement groups built on connect (`build_meas_groups`); empty before
/// connect or for ECUs without a measurement table.
pub fn screens_for(m: &Module, meas_groups: &[MeasGroup]) -> Screens {
    let mut s = HashMap::new();
    let mut polls: HashMap<String, (Vec<(String, String)>, u64)> = HashMap::new();

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

    // --- Data-stream section ("Поток данных") --------------------------------
    // Our structure is populated by the ECU's real INPA display screens
    // (Digital / Analog n / Status) extracted from its `.ipo` — this is INPA's own
    // measurement layout, adapted into our page. The mock category groups are a
    // fallback only for ECUs with no extracted `.ipo`. The DDE reference additionally
    // keeps its live 10-channel group (MW_SELECT) until its analog rows bind (Phase 4).
    let back = || FKey { f: 6, label: loc("НАЗАД", "BACK"), act: FAction::Back };
    let have_meas = !meas_groups.is_empty();
    let mut stream_menu: Vec<MenuItem> = Vec::new();

    // 1) INPA's own curated, grouped data-stream screens (from the ECU's .ipo). Each
    //    group polls the exact per-parameter job calls INPA issues (a mix of batched
    //    MW_SELECT_LESEN_NORM and individual STATUS_*), merged into one view per tick.
    for g in meas_groups {
        stream_menu.push(MenuItem { label: Loc(g.title, g.title), to: g.id.clone() });
        s.insert(
            g.id.clone(),
            Page {
                title: Loc(g.title, g.title),
                blocks: vec![Block::Stream(g.params.clone())],
                keys: vec![back()],
            },
        );
        polls.insert(g.id.clone(), (g.reqs.clone(), 140));
    }

    // 2) INPA digital/status screens keep the ECU's own layout (bars, bound to results).
    //    Analog screens are shown only when there's no live measurement table (they're
    //    label-only until bound); otherwise the batched groups above supersede them.
    if let Some(sg) = crate::screens_gen::get(m.code) {
        use crate::screens_gen::SgKind;
        for (i, sc) in sg.screens.iter().enumerate() {
            let keep = match sc.kind {
                SgKind::Digital | SgKind::Status => true,
                SgKind::Analog => !have_meas,
                _ => false,
            };
            if !keep {
                continue;
            }
            let pid = format!("stream_inpa_{i}");
            let title = Loc(sc.title, sc.title);
            stream_menu.push(MenuItem { label: title, to: pid.clone() });
            s.insert(
                pid,
                Page { title, blocks: vec![Block::Stream(sg_stream_params(sc))], keys: vec![back()] },
            );
        }
    }

    // 3) Fallback: mock category groups only when there's neither a measurement table
    //    nor an extracted INPA layout.
    if !have_meas && crate::screens_gen::get(m.code).is_none() {
        for g in stream_groups_for(m) {
            let pid = format!("stream_{}", g.id);
            stream_menu.push(MenuItem { label: g.name, to: pid.clone() });
            s.insert(
                pid,
                Page { title: g.name, blocks: vec![Block::Stream(g.params)], keys: vec![back()] },
            );
        }
    }

    s.insert(
        "stream".into(),
        Page {
            title: loc("ПОТОК ДАННЫХ", "DATA STREAM"),
            blocks: vec![
                Block::Note(loc("Выберите группу параметров.", "Select a parameter group.")),
                Block::Menu(stream_menu),
            ],
            keys: vec![back()],
        },
    );

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

    Screens { pages: s, polls }
}
