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

use crate::model::{ConnReason, Intent, Update};

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
                    let _ = tx.send(Update::Notice(e.to_string()));
                }
            }
        }
        Intent::ReadFaults => run_faults(st, tx),
        Intent::ClearFaults => {
            if let Some(s) = st.session.as_mut() {
                if let Err(e) = s.run_job("FS_LOESCHEN", "") {
                    let _ = tx.send(Update::Notice(e.to_string()));
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
        Err(e) => {
            let _ = tx.send(Update::Notice(e.to_string()));
        }
    }
}

/// Потоковый тик: опросить открытый экран, наложить на накопитель, декодировать, отдать
/// `Live` (или `PollMiss`, если тик пуст — прошлые значения UI держит сам).
fn poll_stream(st: &mut WorkerState, tx: &Sender<Update>, id: usize) {
    let reqs = match &st.module {
        Some(sm) => sm.as_screen(id).map(poll_reqs).unwrap_or_default(),
        None => return,
    };
    match run_reqs(st.session.as_mut(), &reqs) {
        Some(tick) => {
            match &mut st.held {
                Some(h) => h.overlay(tick),
                None => st.held = Some(tick),
            }
            emit_live(st, tx, id);
        }
        None => {
            let _ = tx.send(Update::PollMiss);
        }
    }
}

/// Одноразовый опрос экрана (TextInfo): сбросить накопитель, прочитать, декодировать.
fn poll_once(st: &mut WorkerState, tx: &Sender<Update>, id: usize) {
    let reqs = match &st.module {
        Some(sm) => sm.as_screen(id).map(poll_reqs).unwrap_or_default(),
        None => return,
    };
    match run_reqs(st.session.as_mut(), &reqs) {
        Some(tick) => {
            st.held = Some(tick);
            emit_live(st, tx, id);
        }
        None => {
            let _ = tx.send(Update::PollMiss);
        }
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

/// Прогнать список `(job, arg)` и слить наборы результатов в один `JobResult`.
/// `None` — если ни один запрос не дал данных (транзиентный промах).
fn run_reqs(session: Option<&mut Session>, reqs: &[(String, String)]) -> Option<JobResult> {
    let s = session?;
    let mut merged: Option<JobResult> = None;
    for (job, arg) in reqs {
        if let Ok(r) = s.run_job(job, arg) {
            match &mut merged {
                Some(acc) => acc.extend(r),
                None => merged = Some(r),
            }
        }
    }
    merged
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

/// Открыть сессию для ЭБУ, названного `.ipo`-скриптом. Разбирает `.ipo` (→ SGBD и
/// address-keyed группы), собирает SGBD-кандидатов и открывает ПЕРВОГО, кто реально
/// присутствует на шине (доказано IDENT-джобом), а не первого «опознанного». Возвращает
/// сессию и разобранную модель экранов (для последующего опроса/декода).
///
/// Присутствие — арбитр выбора: неверный/несуществующий вариант, чужой концепт или пустой
/// фолбэк отбрасываются, а не принимаются вслепую. Так порядок групп в `.ipo` перестаёт
/// быть фатальным, а `sgbd=""` не превращается в `open(".prg")`.
fn connect(port: &str, script: &str) -> Result<(Session, ScreenModule), ConnReason> {
    let sm = load_ipo(script)
        .ok_or_else(|| ConnReason::Other(format!("не удалось разобрать .ipo для {script}")))?;
    let port = resolve_port(port);

    // Кандидаты в порядке приоритета INPA: варианты, опознанные группами `.ipo`, затем
    // непустой фолбэк-SGBD из `.ipo` (дедуп сохраняет порядок).
    let candidates = collect_candidates(&port, &sm);

    // Сбой ОТКРЫТИЯ ПОРТА (нет адаптера / занят) — не «ЭБУ молчит»: запоминаем, чтобы
    // вернуть осмысленную причину, если ни один кандидат не откроется.
    let mut port_err: Option<ConnReason> = None;
    let picked = pick_present(&candidates, |cand| {
        let prg_path = resolve_prg(cand);
        if !prg_path.exists() {
            return None;
        }
        let mut s = match Session::open(&port, BAUD, &prg_path) {
            Ok(s) => s,
            Err(e) => {
                port_err = Some(classify_open(e));
                return None;
            }
        };
        // DS2 INITIALISIERUNG лишь настраивает параметры локально (xsetpar/xawlen) —
        // телеграмму не шлёт, поэтому «успешна» даже при пустой шине. Присутствие ЭБУ
        // доказываем реальным IDENT-джобом: не ответил ⇒ это не тот SGBD / модуль молчит.
        if s.initialize().is_err() {
            return None;
        }
        s.probe_presence().is_ok().then_some(s)
    });

    match picked {
        Some(s) => Ok((s, sm)),
        // Ни один кандидат не присутствует: ошибка адаптера важнее «нет ответа».
        None => Err(port_err.unwrap_or(ConnReason::NoResponse)),
    }
}

/// Собрать SGBD-кандидатов в порядке приоритета INPA: сперва варианты, опознанные
/// групповыми файлами `.ipo` (`D_<ADDR>.GRP` → `IDENTIFIKATION` → `VARIANTE`) в порядке
/// перечисления, затем непустой фолбэк-SGBD из `.ipo`. Имена — `<name>.prg`, дедуп с
/// сохранением порядка. Отсутствующий/не опознавший групповой файл просто не даёт кандидата.
fn collect_candidates(port: &str, sm: &ScreenModule) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in &sm.group_files {
        let grp_path = resolve_prg(&format!("{g}.GRP"));
        if !grp_path.exists() {
            continue;
        }
        let identified = Session::open(port, BAUD, &grp_path).and_then(|mut s| {
            s.initialize()?;
            s.identify_variant()
        });
        if let Ok(Some(variant)) = identified {
            let name = format!("{variant}.prg");
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    if !sm.sgbd.is_empty() {
        let name = format!("{}.prg", sm.sgbd);
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Выбрать первого присутствующего кандидата: для каждого непустого имени в порядке
/// приоритета вызвать `try_open`; вернуть первый `Some` (в боевом пути — открытую сессию,
/// доказавшую присутствие). Пустые имена пропускаются, `try_open` для них не вызывается.
/// Чистая логика выбора — тестируется без железа.
fn pick_present<T>(candidates: &[String], mut try_open: impl FnMut(&str) -> Option<T>) -> Option<T> {
    for cand in candidates {
        if cand.is_empty() {
            continue;
        }
        if let Some(v) = try_open(cand) {
            return Some(v);
        }
    }
    None
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

    #[test]
    fn pick_present_takes_first_present_skips_absent_and_empty() {
        let cands = vec!["KOMBI31.prg".to_string(), String::new(), "IKE.prg".to_string()];
        // Присутствует только верный вариант IKE; неверный фолбэк KOMBI31 отклоняется
        // присутствием, пустой кандидат вовсе не пробуется.
        let picked = pick_present(&cands, |c| (c == "IKE.prg").then(|| c.to_string()));
        assert_eq!(picked.as_deref(), Some("IKE.prg"));

        // Никто не присутствует → None; пустое имя не передаётся в проверку.
        let mut probed: Vec<String> = Vec::new();
        let none = pick_present(&cands, |c| {
            probed.push(c.to_string());
            None::<()>
        });
        assert!(none.is_none());
        assert_eq!(probed, vec!["KOMBI31.prg".to_string(), "IKE.prg".to_string()]);
    }
}
