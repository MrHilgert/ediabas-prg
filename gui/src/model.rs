//! Мост между слоем отрисовки (`ui/`) и слоем работы с ЭБУ (`link/`).
//!
//! Строго plain-data: этот модуль НЕ импортирует ни `egui`, ни `ediabas`. Он —
//! общий словарь двух зон: `ui/` рисует эти view-модели, `link/` их наполняет.
//! `Intent` идёт из UI в ЭБУ-слой (намерения, без имён джобов/протокола),
//! `Update` — обратно (декодированные данные, без единого типа `ediabas`).

/// Причина неудачного коннекта. Разделяет проблему адаптера/порта (ничего не
/// воткнуто, не тот порт, занят) от молчания ЭБУ на открытом порту.
/// (Перенесено из `worker::ConnectError`.)
pub enum ConnReason {
    /// Порт/tty недоступен — нет адаптера, не тот порт, исчез.
    NoPort,
    /// Порт есть, но открыть нельзя (занят другим приложением / нет прав).
    PortBusy,
    /// Порт открылся, но ЭБУ не ответил на реальный запрос (таймаут / нет на шине).
    NoResponse,
    /// Прочее (неожиданный I/O, ошибка разбора) — несёт сырой текст для лога.
    Other(String),
}

/// Одна декодированная ячейка живого экрана, привязанная к индексу строки экрана
/// (`row` = позиция в `screen.rows`). Форматирование (знаки, бар) — задача `ui/`.
pub struct MeasCell {
    pub row: usize,
    /// Числовое значение (Analog/Logical), если есть.
    pub num: Option<f64>,
    /// Текстовое значение (Text), если есть.
    pub text: Option<String>,
    /// Единица измерения (динамическая с ЭБУ либо статическая из строки экрана).
    pub unit: String,
}

/// Кадр живых значений открытого экрана — позиционно по строкам экрана.
pub struct MeasFrame {
    pub cells: Vec<MeasCell>,
    /// Имена результатов, реально пришедшие с ЭБУ — для диагностики пустого ответа
    /// (TextInfo показывает их, если ни одно имя не совпало).
    pub raw_names: Vec<String>,
}

impl MeasFrame {
    /// Значение для строки экрана по её индексу.
    pub fn cell(&self, row: usize) -> Option<&MeasCell> {
        self.cells.iter().find(|c| c.row == row)
    }
    /// Есть ли хоть одно осмысленное значение (иначе — «нет данных»).
    pub fn has_data(&self) -> bool {
        self.cells.iter().any(|c| c.num.is_some() || c.text.is_some())
    }
}

/// Одна запись условий стоп-кадра (freeze-frame) DTC.
pub struct UwView {
    pub text: String,
    pub val: String,
    pub unit: String,
}

/// Один декодированный код неисправности (DTC) из `FS_LESEN`.
pub struct DtcView {
    pub code: String,        // напр. "1E00"
    pub text: String,        // F_ORT_TEXT
    pub present: bool,       // F_VORHANDEN — активна сейчас
    pub sporadic: bool,      // F_ART «sporadischer Fehler»
    pub raw: String,         // F_HEX_CODE
    pub hfk: i64,            // F_HFK — число появлений
    pub lz: i64,             // F_LZ — счётчик пробега
    pub uw_satz: i64,        // F_UW_SATZ — число стоп-кадров
    pub uw: Vec<UwView>,     // F_UW1..N — условия стоп-кадра
    pub causes: Vec<String>, // F_ART{i}_TEXT — причины/типы
}

/// Результат чтения памяти ошибок.
pub struct FaultView {
    pub dtcs: Vec<DtcView>,
}

/// Намерение из UI в слой ЭБУ. Семантика, а не протокол: ни одного имени джоба,
/// которое UI придумал бы сам. (`NavJob` несёт job/arg, пришедшие из `.ipo` —
/// это доменные данные `inpa`, проходящие сквозь границу, а не знание UI о протоколе.)
pub enum Intent {
    /// Установить связь. `script` — имя .ipo-скрипта: ЭБУ-слой сам разбирает `.ipo`,
    /// извлекает SGBD и группы, резолвит вариант и открывает транспорт.
    Connect { script: String, port: String },
    /// Открыт живой экран данных (id = индекс `screen` в модуле) → начать опрос.
    SetLive(usize),
    /// Открыт одноразовый TextInfo-экран → прочитать его фид+джобы один раз.
    OpenInfo(usize),
    /// Остановить активный опрос/фид.
    StopLive,
    /// Nav-действие меню/активации: job/arg взяты из `inpa` (домен), не из UI-логики.
    NavJob { job: String, arg: String },
    ReadFaults,
    ClearFaults,
    RefreshPorts,
    Shutdown,
}

/// Событие из слоя ЭБУ в UI. Полезная нагрузка — только view-модели, ни одного
/// типа `ediabas`.
pub enum Update {
    /// Связь установлена (INITIALISIERUNG прошла, ЭБУ ответил). Метку интерфейса UI
    /// берёт из каталога — здесь полезной нагрузки нет.
    Connected,
    /// Свежий кадр живых значений открытого экрана.
    Live(MeasFrame),
    Faults(FaultView),
    Ports(Vec<String>),
    ConnectFailed(ConnReason),
    /// Опрос не дал данных (транзиентный сбой шины) — не фатально.
    PollMiss,
    /// Сообщение середины сессии (ошибка одного джоба) — связь держим.
    Notice(String),
}
