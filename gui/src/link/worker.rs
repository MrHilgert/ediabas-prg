//! Поток, владеющий блокирующей `ediabas::Session`, и мост к UI по двум mpsc-каналам.
//! UI-поток НИКОГДА не блокируется на serial I/O.
//!
//! UI шлёт `Intent` (намерения), поток отвечает `Update` (view-модели). Вся протокольная
//! логика — резолв варианта, INITIALISIERUNG, сборка poll-батчей `MW_SELECT_LESEN_NORM`,
//! маппинг ReadFaults→`FS_LESEN` — живёт здесь, не в `ui/`. Поток держит собственную
//! копию `inpa::ScreenModule` (для опроса/декода) и будит перерисовку через wake-замыкание,
//! не называя `egui`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use ediabas::{Error, JobResult, Session};
use inpa::model::ScreenModule;

use crate::model::{ConnReason, Intent, NoticeKind, Update};

use super::decode;

/// DS2 baud по умолчанию. Реальный baud берётся из CommParameter самого `.prg` внутри
/// `Session::open`; это лишь стартовое значение (константа ЭБУ-слоя, не UI).
const BAUD: u32 = 9600;

/// Классифицировать сбой `Session::open`/`initialize` в [`ConnReason`]. Сбой открытия
/// порта приходит как `io::Error` (kind сохранён в `driver::serial`); таймаут шины —
/// `Error::Timeout`.
fn classify_open(e: Error) -> ConnReason {
    use std::io::ErrorKind;
    match e {
        Error::Io(io) => match io.kind() {
            ErrorKind::PermissionDenied | ErrorKind::AddrInUse => ConnReason::PortBusy,
            // NotFound / NotConnected / BrokenPipe и большинство сбоев открытия на
            // отсутствующем tty — «нет адаптера / порт недоступен».
            _ => ConnReason::NoPort,
        },
        Error::Timeout => ConnReason::NoResponse,
        other => ConnReason::Other(other.to_string()),
    }
}

/// Хэндл, за который держится UI: шлёт `Intent`, дренирует `Update` (без блокировки).
pub struct Worker {
    tx: Sender<Intent>,
    rx: Receiver<Update>,
    _handle: thread::JoinHandle<()>,
}

impl Worker {
    /// Запустить поток. `wake` будит перерисовку UI (обычно `ctx.request_repaint()`),
    /// но сам поток про `egui` ничего не знает — только зовёт замыкание.
    pub fn spawn(wake: Box<dyn Fn() + Send + Sync>) -> Self {
        let (cmd_tx, cmd_rx) = channel::<Intent>();
        let (evt_tx, evt_rx) = channel::<Update>();
        let handle = thread::spawn(move || run(cmd_rx, evt_tx, wake));
        Worker { tx: cmd_tx, rx: evt_rx, _handle: handle }
    }

    pub fn send(&self, intent: Intent) {
        let _ = self.tx.send(intent);
    }

    /// Дренировать все накопленные события без блокировки.
    pub fn poll(&self) -> Vec<Update> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Intent::Shutdown);
    }
}

/// Внутреннее состояние потока.
struct WorkerState {
    session: Option<Session>,
    /// Разобранный `.ipo` активного ЭБУ — для сборки опроса и декода.
    module: Option<ScreenModule>,
    /// Открытый живой экран данных (id `screen`), опрашивается каждый тик.
    stream: Option<usize>,
    /// Накопитель значений (overlay между тиками): значение, пропавшее в этом тике,
    /// держит прошлое показание, а не бланкуется.
    held: Option<JobResult>,
}

fn run(cmd_rx: Receiver<Intent>, evt_tx: Sender<Update>, wake: Box<dyn Fn() + Send + Sync>) {
    let mut st = WorkerState { session: None, module: None, stream: None, held: None };

    loop {
        let wait = if st.stream.is_some() {
            Duration::from_millis(20) // тик потоковой отрисовки — как можно быстрее
        } else {
            Duration::from_secs(3600)
        };

        match cmd_rx.recv_timeout(wait) {
            Ok(intent) => {
                if !handle_intent(&mut st, &evt_tx, intent) {
                    break; // Shutdown
                }
                wake();
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(id) = st.stream {
                    poll_stream(&mut st, &evt_tx, id);
                    wake();
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Отправить ошибку джоба в UI, разведя её природу. Транспорт (шина/ЭБУ) — тихо: сама
/// телеграмма и `RX err` уже в ediabas-трейсе, UI покажет это лёгкой строкой. Внутренняя
/// (баг интерпретатора / данных SGBD / пробел фичи) — громко в stderr, ибо это дефект
/// инструмента, а не «машина молчит»; UI выделит её отдельно. Связь в обоих случаях цела.
fn notice_err(tx: &Sender<Update>, e: &Error) {
    let kind = if e.is_transport() {
        NoticeKind::Bus
    } else {
        eprintln!("eDIAG: внутренняя ошибка джоба — {e}");
        NoticeKind::Internal
    };
    let _ = tx.send(Update::Notice { kind, msg: e.to_string() });
}

/// Обработать одно намерение. Возвращает `false` только на `Shutdown`.
fn handle_intent(st: &mut WorkerState, tx: &Sender<Update>, intent: Intent) -> bool {
    match intent {
        Intent::Connect { script, port } => {
            st.stream = None;
            st.held = None;
            st.session = None; // отпустить прежнюю сессию (освободить порт)
            st.module = None;
            match connect(&port, &script) {
                Ok((s, sm)) => {
                    st.session = Some(s);
                    st.module = Some(sm);
                    let _ = tx.send(Update::Connected);
                }
                Err(e) => {
                    let _ = tx.send(Update::ConnectFailed(e));
                }
            }
        }
        Intent::SetLive(id) => {
            st.stream = Some(id);
            st.held = None;
            poll_stream(st, tx, id); // немедленный первый опрос — экран заполняется сразу
        }
        Intent::OpenInfo(id) => {
            // Одноразовое чтение статического экрана (ident/coding): не потоково.
            st.stream = None;
            st.held = None;
            poll_once(st, tx, id);
        }
        Intent::StopLive => {
            st.stream = None;
            st.held = None;
        }
        Intent::NavJob { job, arg } => {
            // Активация / джоб меню: запустить и забыть; ошибку — показать.
            if let Some(s) = st.session.as_mut() {
                if let Err(e) = s.run_job(&job, &arg) {
                    notice_err(tx, &e);
                }
            }
        }
        Intent::ReadFaults => run_faults(st, tx),
        Intent::ClearFaults => {
            if let Some(s) = st.session.as_mut() {
                if let Err(e) = s.run_job("FS_LOESCHEN", "") {
                    notice_err(tx, &e);
                    return true;
                }
            }
            run_faults(st, tx); // после очистки перечитать
        }
        Intent::RefreshPorts => {
            let _ = tx.send(Update::Ports(super::ports::available_ports()));
        }
        Intent::Shutdown => return false,
    }
    true
}

/// Прочитать `FS_LESEN` → `Update::Faults` (или `Notice` при ошибке).
fn run_faults(st: &mut WorkerState, tx: &Sender<Update>) {
    let Some(s) = st.session.as_mut() else { return };
    match s.run_job("FS_LESEN", "") {
        Ok(fr) => {
            let _ = tx.send(Update::Faults(decode::decode_faults(&fr)));
        }
        Err(e) => notice_err(tx, &e),
    }
}

/// Исход одного опроса экрана. Разводит «нечего спрашивать» и «внутренняя ошибка» от
/// настоящего молчания шины — только последнее двигает счётчик промахов к обрыву линка.
enum PollOutcome {
    /// Пришли данные — накладываем и рисуем.
    Data(JobResult),
    /// У экрана нет джобов опроса (нет живого источника) ИЛИ нет сессии. Это НЕ сбой связи:
    /// линк держим, счётчик промахов не трогаем (иначе статичный экран рвал бы связь).
    Nothing,
    /// Все запросы упали транспортом / вернули пусто — шину спросили, она молчит. Транзиентно.
    BusMiss,
    /// Джоб упал внутренней (не транспортной) ошибкой — дефект, а не мёртвая шина.
    Internal(String),
}

/// Потоковый тик: опросить открытый экран, наложить на накопитель, декодировать, отдать
/// `Live`. `PollMiss` шлём ТОЛЬКО на реальном молчании шины (см. [`PollOutcome`]).
fn poll_stream(st: &mut WorkerState, tx: &Sender<Update>, id: usize) {
    let reqs = match &st.module {
        Some(sm) => sm.as_screen(id).map(poll_reqs).unwrap_or_default(),
        None => return,
    };
    match run_reqs(st.session.as_mut(), &reqs) {
        PollOutcome::Data(tick) => {
            match &mut st.held {
                Some(h) => h.overlay(tick),
                None => st.held = Some(tick),
            }
            emit_live(st, tx, id);
        }
        PollOutcome::BusMiss => { let _ = tx.send(Update::PollMiss); }
        PollOutcome::Internal(msg) => {
            eprintln!("eDIAG: внутренняя ошибка опроса — {msg}");
            let _ = tx.send(Update::Notice { kind: NoticeKind::Internal, msg });
        }
        PollOutcome::Nothing => {} // нечего опрашивать — не промах, линк не трогаем
    }
}

/// Одноразовый опрос экрана (TextInfo): сбросить накопитель, прочитать, декодировать.
fn poll_once(st: &mut WorkerState, tx: &Sender<Update>, id: usize) {
    let reqs = match &st.module {
        Some(sm) => sm.as_screen(id).map(poll_reqs).unwrap_or_default(),
        None => return,
    };
    match run_reqs(st.session.as_mut(), &reqs) {
        PollOutcome::Data(tick) => {
            st.held = Some(tick);
            emit_live(st, tx, id);
        }
        PollOutcome::BusMiss => { let _ = tx.send(Update::PollMiss); }
        PollOutcome::Internal(msg) => {
            eprintln!("eDIAG: внутренняя ошибка опроса — {msg}");
            let _ = tx.send(Update::Notice { kind: NoticeKind::Internal, msg });
        }
        PollOutcome::Nothing => {}
    }
}

/// Декодировать накопитель против экрана `id` и отдать `Update::Live`.
fn emit_live(st: &WorkerState, tx: &Sender<Update>, id: usize) {
    if let (Some(sm), Some(held)) = (&st.module, &st.held) {
        if let Some(screen) = sm.as_screen(id) {
            let _ = tx.send(Update::Live(decode::decode_live(held, screen)));
        }
    }
}

/// Прогнать список `(job, arg)` и слить наборы результатов в один `JobResult`. Различает
/// исходы (см. [`PollOutcome`]): нет сессии/запросов → `Nothing`; есть данные → `Data`;
/// джоб упал внутренней ошибкой → `Internal` (первую и запоминаем); иначе (спросили, но
/// пусто/таймаут) → `BusMiss`. Транспортные ошибки отдельных джобов молча пропускаем —
/// это нормальный шум мульти-адресного опроса, а не дефект.
fn run_reqs(session: Option<&mut Session>, reqs: &[(String, String)]) -> PollOutcome {
    let Some(s) = session else { return PollOutcome::Nothing };
    if reqs.is_empty() {
        return PollOutcome::Nothing;
    }
    let mut merged: Option<JobResult> = None;
    let mut internal: Option<String> = None;
    for (job, arg) in reqs {
        match s.run_job(job, arg) {
            Ok(r) => match &mut merged {
                Some(acc) => acc.extend(r),
                None => merged = Some(r),
            },
            Err(e) if e.is_transport() => {} // транзиентный шум шины — ждём следующий тик
            Err(e) => { internal.get_or_insert_with(|| e.to_string()); }
        }
    }
    match (merged, internal) {
        (Some(data), _) => PollOutcome::Data(data),
        (None, Some(msg)) => PollOutcome::Internal(msg),
        (None, None) => PollOutcome::BusMiss,
    }
}

/// Список запросов на тик для экрана данных: батчи `MW_SELECT_LESEN_NORM` (≤10
/// селекторов), прочие джобы поштучно, плюс фид один раз. Перенос `session::poll_reqs`.
fn poll_reqs(screen: &inpa::Screen) -> Vec<(String, String)> {
    let mut selectors: Vec<String> = Vec::new();
    let mut jobs: Vec<(String, String)> = Vec::new();
    let mut push_job = |job: &str, arg: &str| {
        if !job.is_empty() && !jobs.iter().any(|(j, a)| j == job && a == arg) {
            jobs.push((job.to_string(), arg.to_string()));
        }
    };
    if let Some(f) = &screen.feeder {
        if f.job != "MW_SELECT_LESEN_NORM" {
            push_job(&f.job, &f.arg);
        }
    }
    for row in &screen.rows {
        let j = row.job();
        match &j.selector {
            Some(sel) if j.job == "MW_SELECT_LESEN_NORM" => {
                if !selectors.contains(sel) {
                    selectors.push(sel.clone());
                }
            }
            _ => push_job(&j.job, &j.arg),
        }
    }
    let mut reqs: Vec<(String, String)> = selectors
        .chunks(10)
        .map(|c| ("MW_SELECT_LESEN_NORM".to_string(), c.concat()))
        .collect();
    reqs.extend(jobs);
    reqs
}

// ------------------------------------------------------------------- connect --

/// Один SGBD-кандидат на открытие. `group_proven` = групповой `IDENTIFIKATION` уже получил
/// реальный ответ с СОБСТВЕННОГО wake-адреса этого варианта, т.е. блок доказанно на шине —
/// даже если его личный IDENT не умеет само-опроситься (идёт через мастер-ЭБУ, как LWR2A
/// через LCM). Фолбэк-SGBD из `.ipo` — всегда `group_proven=false` (его никто не подтверждал).
struct Candidate {
    prg: String,
    group_proven: bool,
}

/// Открыть сессию для ЭБУ, названного `.ipo`-скриптом. Разбирает `.ipo` (→ SGBD и
/// address-keyed группы), собирает SGBD-кандидатов и выбирает ЛУЧШЕГО присутствующего:
/// сперва тот, кто сам ответил на IDENT (`probe`-proven); если такого нет — тот, чьё
/// присутствие уже доказала группа (`group_proven`). Возвращает сессию и модель экранов.
///
/// Так неверный/несуществующий вариант, чужой концепт или пустой фолбэк отбрасываются, а
/// LWR-класс (IDENT через мастер-ЭБУ) всё же коннектится. Порт один — держим не больше одной
/// открытой сессии; поэтому найденный `group_proven` держим открытым как запасной и роняем
/// только ради попытки следующего кандидата.
fn connect(port: &str, script: &str) -> Result<(Session, ScreenModule), ConnReason> {
    let sm = load_ipo(script)
        .ok_or_else(|| ConnReason::Other(format!("не удалось разобрать .ipo для {script}")))?;
    // Свежий диагностический контекст: сбрасываем EDIABAS shared memory (shmset/shmget), чтобы
    // кэш состояния прошлого ЭБУ не «протёк» в этот коннект. Внутри одного коннекта слот пишет
    // INITIALISIERUNG варианта и читают его же джобы (так LWR отдаёт данные через мастер-LCM).
    ediabas::clear_shared_memory();
    let port = resolve_port(port);
    let candidates = collect_candidates(&port, &sm);

    // Решение (какой .prg победит) — см. чистую `select_present`; здесь тот же контракт, но
    // с одним открытием на кандидата (без повторного INITIALISIERUNG — у LWR это физический
    // тест фар). `held` — открытая group_proven-сессия, `held_name` — её имя для редкого
    // переоткрытия, если пришлось уронить ради следующего кандидата.
    let mut held: Option<Session> = None;
    let mut held_name: Option<String> = None;
    // Сбой ОТКРЫТИЯ ПОРТА (нет адаптера / занят) — не «ЭБУ молчит»: важнее «нет ответа».
    let mut port_err: Option<ConnReason> = None;

    for cand in &candidates {
        let prg_path = resolve_prg(&cand.prg);
        if !prg_path.exists() {
            continue;
        }
        held = None; // один порт — отпускаем прежнюю сессию перед новым открытием
        let mut s = match Session::open(&port, BAUD, &prg_path) {
            Ok(s) => s,
            Err(e) => {
                port_err = Some(classify_open(e));
                continue;
            }
        };
        if s.initialize().is_err() {
            continue;
        }
        // DS2 INITIALISIERUNG лишь настраивает параметры локально — присутствие доказываем
        // реальным IDENT-джобом. Ответил сам → это лучший кандидат, берём немедленно.
        if s.probe_presence().is_ok() {
            return Ok((s, sm));
        }
        // Сам не ответил, но группа уже видела его wake-адрес → держим как запасной.
        if cand.group_proven {
            if held_name.is_none() {
                held_name = Some(cand.prg.clone());
            }
            held = Some(s);
        }
    }

    // Ни один не прошёл IDENT. Принять group_proven (группа доказала присутствие).
    if let Some(s) = held {
        return Ok((s, sm)); // ещё открыт (был последним кандидатом) — без переоткрытия
    }
    if let Some(name) = held_name {
        // group_proven был уронён ради следующего (тоже неудачного) кандидата — переоткрыть.
        if let Ok(mut s) = Session::open(&port, BAUD, &resolve_prg(&name)) {
            if s.initialize().is_ok() {
                return Ok((s, sm));
            }
        }
    }
    Err(port_err.unwrap_or(ConnReason::NoResponse))
}

/// Собрать SGBD-кандидатов в порядке приоритета INPA: сперва варианты, опознанные
/// групповыми файлами `.ipo` (`D_<ADDR>.GRP` → `IDENTIFIKATION` → `VARIANTE`) в порядке
/// перечисления, затем непустой фолбэк-SGBD из `.ipo`. Дедуп по имени с сохранением порядка.
/// Для каждого варианта помечаем `group_proven`: ответил ли его собственный wake-адрес среди
/// адресов, откликнувшихся группе при опознании.
fn collect_candidates(port: &str, sm: &ScreenModule) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for g in &sm.group_files {
        let grp_path = resolve_prg(&format!("{g}.GRP"));
        if !grp_path.exists() {
            continue;
        }
        let identified = Session::open(port, BAUD, &grp_path).and_then(|mut s| {
            s.initialize()?;
            let variant = s.identify_variant()?;
            Ok((variant, s.responders()))
        });
        if let Ok((Some(variant), responders)) = identified {
            let prg = format!("{variant}.prg");
            if !out.iter().any(|c| c.prg == prg) {
                let group_proven =
                    variant_wake(&prg).is_some_and(|w| responders.contains(&w));
                out.push(Candidate { prg, group_proven });
            }
        }
    }
    if !sm.sgbd.is_empty() {
        let prg = format!("{}.prg", sm.sgbd);
        if !out.iter().any(|c| c.prg == prg) {
            out.push(Candidate { prg, group_proven: false });
        }
    }
    out
}

/// Статически прочитать wake-адрес варианта из его `.prg` (CommParameter), без I/O к ЭБУ.
fn variant_wake(prg: &str) -> Option<u8> {
    ediabas::PrgFile::open(&resolve_prg(prg)).ok()?.initial_comm_config()?.wake_addr
}

/// Чистый контракт выбора (тестируется без железа): первый `Present`; иначе первый
/// `GroupOnly`; иначе `None`. `connect` реализует ровно это, но с одним открытием сессии.
#[cfg_attr(not(test), allow(dead_code))]
enum Probe {
    Present,
    GroupOnly,
    Absent,
}

#[cfg_attr(not(test), allow(dead_code))]
fn select_present(cands: &[Candidate], probe: impl Fn(&Candidate) -> Probe) -> Option<String> {
    let mut group_only: Option<&Candidate> = None;
    for c in cands {
        match probe(c) {
            Probe::Present => return Some(c.prg.clone()),
            Probe::GroupOnly if group_only.is_none() => group_only = Some(c),
            _ => {}
        }
    }
    group_only.map(|c| c.prg.clone())
}

/// Разобрать `SGDAT/<script>.ipo` (регистронезависимо по имени и расширению).
fn load_ipo(script: &str) -> Option<ScreenModule> {
    let path = crate::i18n::resolve_ci("SGDAT", &format!("{script}.ipo"))?;
    inpa::parse(&path).ok()
}

/// Разрешить порт: пустая строка = «авто» → первый доступный (резолв авто — забота
/// ЭБУ-слоя, чтобы UI не звал `available_ports`).
fn resolve_port(port: &str) -> String {
    if !port.is_empty() {
        return port.to_string();
    }
    super::ports::available_ports().into_iter().next().unwrap_or_default()
}

/// Найти `ecu/<name>` независимо от cwd и регистра имени (dev-сборки лежат под `target/`,
/// а `ecu/` — в корне воркспейса). Иначе — обычный относительный путь.
fn resolve_prg(name: &str) -> PathBuf {
    crate::i18n::resolve_ci("ecu", name).unwrap_or_else(|| Path::new("ecu").join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error as IoError, ErrorKind};

    #[test]
    fn classify_open_splits_port_from_ecu() {
        // Отсутствующий/недоступный адаптер → NoPort.
        assert!(matches!(classify_open(Error::Io(IoError::from(ErrorKind::NotFound))), ConnReason::NoPort));
        // Занятый / запрещённый порт → PortBusy.
        assert!(matches!(classify_open(Error::Io(IoError::from(ErrorKind::PermissionDenied))), ConnReason::PortBusy));
        assert!(matches!(classify_open(Error::Io(IoError::from(ErrorKind::AddrInUse))), ConnReason::PortBusy));
        // Реальный таймаут ЭБУ (порт открыт, ответа нет) → NoResponse.
        assert!(matches!(classify_open(Error::Timeout), ConnReason::NoResponse));
        // Прочее → Other.
        assert!(matches!(classify_open(Error::Vm("x".into())), ConnReason::Other(_)));
    }

    fn cand(prg: &str, group_proven: bool) -> Candidate {
        Candidate { prg: prg.to_string(), group_proven }
    }

    #[test]
    fn select_prefers_probe_present_over_group_and_absent() {
        // KMB: неверный вариант KOMBI32 (не group_proven) отклоняется, фолбэк IKI отвечает сам.
        let kmb = [cand("KOMBI32.prg", false), cand("IKI.prg", false)];
        let picked = select_present(&kmb, |c| match c.prg.as_str() {
            "IKI.prg" => Probe::Present,
            _ => Probe::Absent,
        });
        assert_eq!(picked.as_deref(), Some("IKI.prg"));

        // LCM: вариант LCM_III присутствует по группе, но его IDENT молчит; фолбэк LCM
        // отвечает сам → probe-proven важнее group-proven.
        let lcm = [cand("LCM_III.prg", true), cand("LCM.prg", false)];
        let picked = select_present(&lcm, |c| match c.prg.as_str() {
            "LCM.prg" => Probe::Present,
            "LCM_III.prg" => Probe::GroupOnly,
            _ => Probe::Absent,
        });
        assert_eq!(picked.as_deref(), Some("LCM.prg"));
    }

    #[test]
    fn select_accepts_group_proven_when_no_probe_present() {
        // LWR: единственный кандидат LWR2A сам не отвечает (IDENT через LCM), но группа
        // достучалась до его адреса → принимаем по группе.
        let lwr = [cand("LWR2A.prg", true)];
        let picked = select_present(&lwr, |_| Probe::GroupOnly);
        assert_eq!(picked.as_deref(), Some("LWR2A.prg"));

        // Никто не присутствует и не подтверждён группой → None.
        let none = select_present(&lwr, |_| Probe::Absent);
        assert!(none.is_none());
    }
}
