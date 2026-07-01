# ediabas-prg — полноценный интерпретатор EDIABAS + K-line транспорт

Rust-реализация полноценного интерпретатора EDIABAS `.prg` файлов (BEST/2 байткод)  
с поддержкой всех транспортных протоколов BMW K-line.

**Долгосрочная цель (3 уровня):**
1. **`ediabas-prg`** (этот проект) — библиотека + CLI: интерпретатор `.prg` + все K-line протоколы
2. **GUI проект** (отдельный) — использует `ediabas-prg` как Rust crate-зависимость
3. **Итог:** нативный BMW диагностический инструмент, аналог INPA/EDIABAS, без Wine/Windows

> Архитектурное следствие: `vm.rs` и `ds2.rs` проектируются как публичное API библиотеки.  
> GUI будет управлять VM и получать `ResultSet` асинхронно.

**Текущий фокус:** публичное API библиотеки (`ediabas` crate) для использования из GUI —
`Session::open()` → `initialize()` → `run_job()` со структурированным `JobResult`.

## ВАЖНО — читай перед работой

**`docs/PLAN.md`** — обязательный документ. Содержит полный план разработки  
по этапам (0→4): от DS2 транспорта до GUI. Перед любой новой задачей:
1. Открой `docs/PLAN.md`
2. Найди текущий этап (чекбоксы `[ ]` / `[x]`)
3. Работай в рамках этапа, не перепрыгивай вперёд без завершения текущего

Прочие доки в `docs/`: `BEST2-DECODER.md` (спека декодера), `EDC15C-COMMS.md`,
`RESEARCH.md`. EDIABAS-тулзы (`xtract`, `bestinfo`, `Best2`) — в `Bin/` (gitignored).

## Контекст

- **ЭБУ:** BMW DDE4.0 diesel, адрес 0xB8
- **Адаптер:** KDCAN клон (FTDI), `/dev/ttyUSB0`, INPA на Windows с ним работает
- **Протокол:** BMW DS2 по K-line, 9600 baud, Even parity, XOR checksum
- **Файл:** `DDE40KW0.prg` (на машине пользователя, не в репозитории)
- **Цель-задание:** запустить job `STATUS_DZMNMIT`, получить обороты и прочие данные

## Архитектура (5 слоёв)

```
APP (CLI / GUI)
  ↓
API — Session: init_ecu(), run_job() → ResultSet    (этап 3)
  ↓
PRG — BEST/2 VM: выполняет байткод, управляет регистрами
  ↓
TRANSPORT — знает протокол: DS2, KWP1281, KWP2000, D-CAN
  ↓
DRIVER — только сырые байты: читает/пишет, set_break, set_timeout
         SerialDriver / MockDriver / (будущий TcpDriver, J2534Driver)
```

**Все слои через `Box<dyn Trait>`** — никаких generics, легко менять реализацию из GUI.

## Структура файлов

Crate = **lib + bin**: `[lib] name="ediabas"` (`src/lib.rs`) + `[[bin]] name="ediabas-prg"` (`src/main.rs`).
GUI/другие бинарники подключают `ediabas` как зависимость и работают через `Session`.

```
src/
  lib.rs           — корень библиотеки: pub use Session/JobResult/ResultSet/Value/PrgFile/Error
  session.rs       — ВЫСОКОУРОВНЕВОЕ API: Session::open → initialize → run_job → JobResult
  main.rs          — CLI (clap): команды info/jobs/sim/run/raw (использует ediabas::…)
  prg.rs           — парсер .prg файлов (хедер, job table, XOR decode 0xF7, таблицы данных)
  vm.rs            — BEST/2 VM (~2200 строк): Vm { transport: Box<dyn Transport> }
  error.rs         — Error enum, Result<T>
  config.rs        — CommConfig, Protocol enum
  driver/          — trait Driver + SerialDriver (FTDI/KDCAN) / MockDriver
  transport/       — trait Transport + Ds2Transport (DS2 K-line) / SimTransport / NullTransport
docs/              — PLAN.md, BEST2-DECODER.md, EDC15C-COMMS.md, RESEARCH.md
Bin/  (gitignored) — EDIABAS-тулзы: xtract.exe, bestinfo.exe, Best2.exe …
ecu/  (gitignored) — .prg файлы ЭБУ
```

### Публичное API (для GUI) — `session.rs`

```rust
use ediabas::Session;

let mut s = Session::open("COM3", 9600, "ecu/DDE40KW0.prg")?;
s.initialize()?;                                   // запускает INITIALISIERUNG
let res = s.run_job("MW_SELECT_LESEN_NORM", "0F30")?;  // структурированный JobResult
let v = res.get_f64("STAT_armM_List_WERT");        // → Some(480.0)
for set in res.sets() {
    for (name, val) in set.iter() { /* … */ }
}
```

## Сборка и запуск

```bash
cargo build --release

# Информация о .prg файле
./ediabas-prg info DDE40KW0.prg

# Список jobs
./ediabas-prg jobs DDE40KW0.prg

# Тест транспорта без VM — главный инструмент отладки
sudo ./ediabas-prg raw \
  --port /dev/ttyUSB0 --baud 9600 \
  "B8 12 F1 04 2C 10 0F 10"

# С init-фреймом перед запросом
sudo ./ediabas-prg raw \
  --port /dev/ttyUSB0 --baud 9600 \
  --init "B8 06 F1 01 A2 00" \
  "B8 12 F1 04 2C 10 0F 10"

# Полный прогон через VM
sudo ./ediabas-prg run \
  --prg DDE40KW0.prg STATUS_DZMNMIT \
  --port /dev/ttyUSB0 --baud 9600 \
  --init-job INITIALISIERUNG
```

## Формат .prg файла

- **Magic:** `@EDIABAS OBJECT\0` (16 байт)
- **XOR ключ:** `0xF7` (все данные XOR'ятся)
- **Указатели** в хедере (offset → смысл):
  - `0x80` → PTR_CODE (начало байткода, обычно 0xA0)
  - `0x84` → PTR_RESULTS
  - `0x88` → PTR_JOBS (таблица job'ов)
  - `0x90` → PTR_SGBD (метаданные ECU)
- **Job entry:** 68 байт: 64-байт имя (null-padded) + u32 LE code_offset
- **Таблицы SGBD:** XOR-decoded строки между PTR_JOBS и PTR_RESULTS

## BEST/2 VM — ключевые детали

### Регистры
```
B0-BF  = 0x00-0x0F  (байтовые)
I0-I7  = 0x10-0x17  (16-bit)
L0-L3  = 0x18-0x1B  (32-bit)
S0-S23 = 0x1C-0x33  (строки/данные)
```
- B0-B3 → байты 0-3 регистра L0
- B4-B7 → байты 0-3 регистра L1

### Типы операндов (MODE байт: hi=dst, lo=src)
```
0=None  1=RegS  2=RegAB  3=RegI  4=RegL
5=Imm8  6=Imm16  7=Imm32  8=ImmStr
9=IdxImm(base+imm8)  A=IdxReg(base+reg)  B=IdxRegImm
```

### Критически важные опкоды

**`comp` (0x02)** — НЕ ДЕСТРУКТИВЕН. Только выставляет флаги, L0 НЕ меняет:
```rust
flags.zero  = dst - src == 0;
flags.minus = dst - src < 0;
flags.carry = (dst as u32) < (src as u32);
```

**`test` (0x6A)** — AND операция, только флаги, регистры не меняет:
```rust
let result = dst & src;
flags.zero  = result == 0;
flags.minus = result < 0;
```

**`jz/jnz/jmi/jpl` (0x10/0x11/0x14/0x15)** — проверяют `flags.zero`/`flags.minus`, **НЕ регистр L0**.
- Это был главный баг: старая реализация проверяла L0 вместо флагов → неправильный переход при `test I1, #1; jnz`

**`move` (0x00)** — базовый копирующий присвоение.

### Индексация строк (IdxReg, тип 0xA)
`S1[I1]` кодируется как `[opcode][mode_with_A_nibble][base_reg][idx_reg]`

### Алгоритм hex-строка → байты (STATUS_DZMNMIT)
Job читает hex-строку `"B812F1042C100F10"` и конвертирует в S1:
```
test I1, #1  → jnz → нечётный индекс → B5 (младший nibble)
             ↓ чётный → B6 (старший nibble)
B6 <<= 4; S1[i] = B6; S1[i] += B5
```
Результат: `S1 = B8 12 F1 04 2C 10 0F 10` ✓

## DS2 транспорт

### Формат кадра
```
[ECU_ADDR] [LEN?] [...] [CHK_XOR]
```

Позиция LEN в ответе зависит от EDIABAS концепта:
- Концепт 0x0006: `[ADDR][LEN][SVC][DATA][CHK]` → `len_offset=1`
- Концепт 0x0001: `[ADDR][SRC][LEN][SVC][DATA][CHK]` → `len_offset=2`

LEN = общее кол-во байт в кадре (включая ADDR и CHK).

### Ключевые выводы из анализа EdiabasLib

1. **DS2 (концепты 0x0001, 0x0005, 0x0006) НЕ требует 5-baud init.** `EcuConnected = true` устанавливается сразу. 5-baud init нужен только для KWP1281/KWP2000.

2. **KDCAN/FTDI не даёт эхо на USB RX.** EdiabasLib концепт 0x0001 явно отвергает адаптеры с эхом. `echo` по умолчанию `false`.

3. **Parity=Even** — правильно для DS2 (концепты 0x0001 и 0x0006 оба используют Even).

4. **Baud rate** берётся из `CommParameterProtected[1]` в .prg (через INITIALISIERUNG).

### Структура `Ds2Transport`
```rust
pub struct Ds2Transport {
    driver: Box<dyn Driver>,
    pub echo: bool,         // false для KDCAN (default)
    pub len_offset: usize,  // 1 для концепта-6, 2 для концепта-1
    timeout_std_ms: u64,
    regen_time_ms: u64,
}
```

### `receive_raw()` — отладочный режим
Собирает ВСЕ байты ответа по таймауту, без разбора структуры.  
`raw` команда автоматически пробует разобрать ответ с LEN@[1] и LEN@[2].

## Текущее состояние

### Что работает ✓
- Парсинг .prg файлов (хедер, jobs, таблицы, декодирование)
- BEST/2 VM: все основные опкоды, флаги (после фикса jz/jnz)
- Корректное построение TELEGRAM: `B8 12 F1 04 2C 10 0F 10` + CHK=`7C`
- Команда `raw` для прямой отладки транспорта

### Что отлаживается
**ЭБУ не отвечает.** Возможные причины:
1. Нужен INITIALISIERUNG job перед STATUS_DZMNMIT (установить DS2 сессию)
2. TELEGRAM содержит 0x12 на позиции [1] — не понятно LEN это или SRC_ADDR
3. len_offset неправильный (1 vs 2) для DDE4

### Следующие шаги
```bash
# 1. Проверить без init, без echo
sudo ./ediabas-prg raw --port /dev/ttyUSB0 --baud 9600 \
  "B8 12 F1 04 2C 10 0F 10"

# 2. С init-фреймом (INITIALISIERUNG обычно service 0x01, sub 0xA2)
sudo ./ediabas-prg raw --port /dev/ttyUSB0 --baud 9600 \
  --init "B8 06 F1 01 A2 00" \
  "B8 12 F1 04 2C 10 0F 10"

# 3. Посмотреть концепт из SGBD метаданных .prg
./ediabas-prg info DDE40KW0.prg

# 4. Если есть ответ — посмотреть len_offset
sudo ./ediabas-prg run --prg DDE40KW0.prg STATUS_DZMNMIT \
  --port /dev/ttyUSB0 --baud 9600 \
  --init-job INITIALISIERUNG --len-offset 2
```

## Roadmap

### Интерпретатор BEST/2 (vm.rs)

| Статус | Область |
|--------|---------|
| ✓ Готово | Арифметика (move, comp, add, sub, mul, div, and, or, xor, not) |
| ✓ Готово | Ветвления (jz, jnz, jmi, jpl, jc, jae, jv, jnv, jg, jge, jl, jle, ja, jbe) |
| ✓ Готово | Стек (push, pop, pushf, popf), вызовы (jtsr, ret, pcall) |
| ✓ Готово | Строки (scmp, scat, scut, slen, spaste, serase, stoken, srevrs) |
| ✓ Готово | Результаты (etag, enewset, ergb/w/d/i/r/s/y) |
| ✓ Готово | Таблицы (tabset, tabseek, tabget, tabline, tabcols, tabrows) |
| ✓ Готово | Hex/числа (fix2hex, fix2dez, a2fix, fix2flt, flt2a, hex2y) |
| ✓ Готово | Флоаты (fadd, fsub, fmul, fdiv, fcomp, a2flt, flt2fix) |
| ⚡ Частично | Коммуникация (xconnect, xhangup, xsetpar, xsend, xrecv, xrequf) |
| ✗ Нужно | CommParameter разбор внутри VM (концепт, baud, таймауты из INITIALISIERUNG) |
| ✗ Нужно | xstate, xtype, xvers, xboot, xreset — статусные команды |
| ✗ Нужно | Файловые операции (fopen, fread, freadln, fclose) — низкий приоритет |

### Транспортный уровень (ds2.rs + новые модули)

| Статус | Протокол | Концепт EDIABAS | Применение |
|--------|----------|-----------------|------------|
| ⚡ В работе | **DS2** K-line | 0x0001, 0x0005, 0x0006 | DDE4, DME, EGS и др. |
| ✗ Нужно | **KWP1281** K-line | 0x0002, 0x0003 | Старые ЭБУ |
| ✗ Нужно | **KWP2000 BMW** K-line | 0x010C | Новые K-line ЭБУ |
| ✗ Нужно | **BMW-FAST** K-line | 0x010F | Расширенный протокол |
| ✗ Нужно | **D-CAN** | 0x0110 | CAN шина через KDCAN |
| ✗ Нужно | **TP2.0 / ISO-TP** | CAN подпротоколы | CAN транспорт |

### Адаптеры

| Статус | Адаптер |
|--------|---------|
| ⚡ Тестируется | KDCAN (FTDI USB, `/dev/ttyUSB*`) |
| ✗ Нужно | ELM327 (WiFi/BT/USB) |
| ✗ Нужно | PassThru / J2534 |

### Архитектурные задачи

- [ ] `CommParameter` — разбор параметров протокола внутри VM (сейчас хардкод 9600/Even)
- [ ] `len_offset` авто-определение из концепта (сейчас ручной флаг `--len-offset`)
- [ ] Модульный транспорт: `trait Transport` расширить под разные init-стратегии
- [ ] Логирование на уровне протокола (как EdiabasLib IFH лог)
- [ ] Тест-suite против реальных `.SIM` файлов от INPA

## Исправленные баги

| Баг | Симптом | Фикс |
|-----|---------|------|
| `jz/jnz` проверяли L0, а не `flags.zero` | TELEGRAM 15 байт вместо 8 | Теперь читают `self.flags.zero` |
| `comp` писал результат в L0 | Флаги не сохранялись | `comp` только флаги, L0 не трогает |
| `echo = true` по умолчанию | ЭБУ ответ принимался за эхо | `echo = false` по умолчанию |
| `receive()` всегда LEN@[1] | Таймаут если концепт 0x0001 | Добавлен `len_offset` поле |

## Зависимости

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
serialport = "4"
```

Нужны права на `/dev/ttyUSB0`: `sudo` или `usermod -aG dialout $USER` (потом re-login).
