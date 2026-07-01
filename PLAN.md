# План разработки ediabas-prg

Цель: полноценная замена EDIABAS runtime на нативный Rust.  
Конечный результат — библиотека (`ediabas` crate) поверх которой строится GUI.

---

## Этап 0 — транспорт работает (текущий)

- [x] DS2 send/receive реализован
- [x] Echo off по умолчанию (KDCAN/FTDI)
- [x] `raw` команда для отладки транспорта
- [ ] **ЭБУ отвечает на DS2 фрейм** ← здесь сейчас

Команды для завершения этапа:
```bash
# 1. Прямой тест без init
sudo ./ediabas-prg raw --port /dev/ttyUSB0 --baud 9600 \
  "B8 12 F1 04 2C 10 0F 10"

# 2. С init фреймом
sudo ./ediabas-prg raw --port /dev/ttyUSB0 --baud 9600 \
  --init "B8 06 F1 01 A2 00" \
  "B8 12 F1 04 2C 10 0F 10"

# 3. Через VM с init job
sudo ./ediabas-prg run --prg DDE40KW0.prg STATUS_DZMNMIT \
  --port /dev/ttyUSB0 --baud 9600 --init-job INITIALISIERUNG
```

---

## Этап 1 — CommParameter внутри VM

Сейчас протокол (baud, parity, концепт, таймауты) задаётся CLI флагами.  
Должен настраиваться INITIALISIERUNG job через x-команды VM.

### Что реализовать

**`xconnect` (0x26)** — установить соединение, принять CommParameter:
```
CommParameter[0] = concept (0x0001, 0x0006, 0x010C, ...)
CommParameter[1] = baud rate
CommParameter[2] = wake address (для KWP1281/2000)
CommParameter[5] = timeout standard (ms)
CommParameter[6] = regen time (ms)
CommParameter[7] = timeout tel end (ms)
```

На основе концепта VM должна:
- выбрать протокол (DS2 / KWP1281 / KWP2000 / BMW-FAST / D-CAN)
- настроить порт (baud, parity)
- установить `len_offset` (1 для концепта 6, 2 для концепта 1)
- сохранить таймауты

**`xhangup` (0x27)** — закрыть соединение  
**`xsetpar` (0x28)** — изменить отдельный параметр  
**`xawlen` (0x29)** — установить ожидаемую длину ответа  
**`xsend` (0x2A)** — отправить TELEGRAM без ожидания ответа  
**`xsendf` (0x2B)** — отправить "frequent" telegram  
**`xrequf` (0x2C)** — отправить и получить ответ (основная команда)  
**`xstate` (0x2F)** — получить статус соединения  

### Структура изменений

```
src/
  protocol.rs  ← НОВЫЙ: enum Protocol, trait ProtocolHandler
  ds2.rs       — реализует ProtocolHandler для DS2
  kwp1281.rs   ← НОВЫЙ (этап 2)
  kwp2000.rs   ← НОВЫЙ (этап 2)
  vm.rs        — xconnect/xhangup/xsetpar хранят CommParameter,
                 вызывают protocol.configure() / protocol.exchange()
```

```rust
// protocol.rs
pub enum Protocol { Ds2, Kwp1281, Kwp2000Bmw, BmwFast, DCan }

pub struct CommConfig {
    pub protocol: Protocol,
    pub baud:     u32,
    pub parity:   serialport::Parity,
    pub len_offset: usize,
    pub timeout_std: u64,
    pub regen_time:  u64,
    pub wake_addr:   Option<u8>,
}

pub trait ProtocolHandler {
    fn configure(&mut self, cfg: &CommConfig) -> io::Result<()>;
    fn exchange(&mut self, frame: &[u8]) -> io::Result<Vec<u8>>;
    fn init_connection(&mut self, cfg: &CommConfig) -> io::Result<()>;
}
```

---

## Этап 2 — остальные протоколы K-line

Приоритет по частоте применения в BMW:

### DS2 (концепты 0x0001, 0x0005, 0x0006) — уже есть, доработать
- [ ] Авто-определение `len_offset` из концепта
- [ ] Regen time (пауза между запросами)
- [ ] Interbyte time

### KWP1281 (концепт 0x0002, 0x0003)
- [ ] 5-baud init (уже есть как `kline_5baud_init`)
- [ ] Byte-by-byte обмен с ACK (KWP1281 специфика)
- [ ] Idle keepalive (ECU отключается без активности)

### KWP2000 BMW (концепт 0x010C)
- [ ] 5-baud init ИЛИ fast init (25ms LOW + 25ms HIGH)
- [ ] ISO14230 фреймы: `[0x80|flags][DST][SRC][LEN][SVC][DATA][CHK]`
- [ ] StartDiagnosticSession (0x10) перед другими сервисами
- [ ] Negative response обработка (0x7F)

### BMW-FAST (концепт 0x010F)
- [ ] Расширение KWP2000 с BMW-специфичными сервисами

### D-CAN (концепт 0x0110)
- [ ] CAN фреймы через KDCAN в CAN-режиме
- [ ] ISO-TP (15765-2) для длинных сообщений
- [ ] Отдельная секция (потребует разбора KDCAN CAN API)

---

## Этап 3 — разделение на библиотеку

Перед GUI проектом `ediabas-prg` должен стать `lib + bin`.

### Структура

```
Cargo.toml:
  [lib]  name = "ediabas"  path = "src/lib.rs"
  [[bin]] name = "ediabas-prg"  path = "src/main.rs"

src/
  lib.rs      ← НОВЫЙ: pub use, публичный API
  error.rs    ← НОВЫЙ: Error enum
  session.rs  ← НОВЫЙ: Session struct (порт + VM + .prg)
  main.rs     — CLI, использует lib
```

### Публичный API (то что вызывает GUI)

```rust
use ediabas::{Session, PrgFile, Value};

// Открыть сессию
let mut session = Session::open("/dev/ttyUSB0", 9600)?;

// Загрузить .prg, выполнить инициализацию
let prg = PrgFile::open("DDE40KW0.prg")?;
session.init(&prg)?;  // запускает INITIALISIERUNG, настраивает протокол

// Запустить job
let results = session.run(&prg, "STATUS_DZMNMIT")?;

// Получить значения
let rpm   = results.get_long("STAT_DREHZAHL_WERT")?;
let temp  = results.get_float("STAT_KUEHLWASSER_WERT")?;
let dtcs  = results.get_string("STAT_FEHLER")?;
```

### Error тип

```rust
pub enum Error {
    Io(io::Error),
    Protocol(ProtocolError),
    Vm(VmError),
    Prg(PrgError),
    Timeout,
    ChecksumMismatch { expected: u8, got: u8 },
    EcuError { code: u8 },
}
```

### Для GUI — асинхронность

Опции (выбрать одну):
- **A. `std::sync::mpsc` канал** — VM в отдельном треде, шлёт события в GUI (проще)
- **B. `tokio` async** — если GUI тоже async (egui + tokio — рабочая связка)
- **C. callback** — `session.on_progress(|step| ...)` (наименее гибко)

Рекомендация: **вариант A** (канал) — не тянет async runtime, работает с любым GUI фреймворком.

---

## Этап 4 — GUI проект (отдельный репозиторий)

Зависимость в `Cargo.toml` GUI проекта:
```toml
[dependencies]
ediabas = { path = "../ediabas_prg" }
# или после публикации:
# ediabas = "0.1"
```

GUI фреймворк: **egui / eframe** (нативный Rust, кроссплатформенный, простой).

Минимальные экраны:
- Выбор порта + `.prg` файла
- Список jobs
- Запуск job → таблица результатов
- DTC список (ошибки ЭБУ)
- Live данные (обороты, температура, нагрузка)

---

## Приоритеты

```
[0] ЭБУ отвечает на DS2         ← сейчас
[1] CommParameter в VM          ← следующий большой этап
[2] KWP1281                     ← нужен для многих ЭБУ
[3] lib/bin разделение          ← перед GUI
[4] KWP2000 BMW                 ← параллельно с GUI
[5] GUI проект                  ← отдельный репо
[6] D-CAN                       ← позже
```
