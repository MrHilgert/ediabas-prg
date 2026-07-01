# EDIABAS / BMW K-line — исследование протоколов

> Документ пополняется по мере изучения. Данные получены из: ifh.trc/api.trc трейсов INPA,
> бинарного дампа DDE40KW0.prg, документации EDIABAS и открытых проектов.

---

## 1. Структура .prg файла

### Заголовок (magic + указатели)

```
0x0000  "@EDIABAS OBJECT\0"  (16 байт, magic)
0x0010  version              (u32 LE)
0x0080  ptr_code             (u32 LE) → начало байткода (всегда 0xa0)
0x0084  ptr_results          (u32 LE) → секция результатов/параметров
0x0088  ptr_jobs             (u32 LE) → таблица job-записей
0x008C  ptr_params           (u32 LE) → доп. параметры
0x0090  ptr_sgbd             (u32 LE) → SGBD-метаданные (ECU, ORIGIN, REVISION...)
0x0094  ptr_vars             (u32 LE) → переменные/регистровая таблица
```

Все данные XOR-кодированы ключом **0xF7**, кроме заголовка (magic + указатели в 0x80-0x97).

### Job-запись (68 байт, XOR'd)

```
[0..63]  имя job (null-padded ASCII)
[64..67] code_offset (u32 LE, от начала файла)
```

### CommParameter — хранится в BEST/2 байткоде

CommParameter **не хранится** отдельно как бинарный массив — он закодирован прямо
в байткоде INITIALISIERUNG-джоба как непосредственные (immediate) значения.

Из DDE40KW0.prg, INITIALISIERUNG, offset 0x4B:
```
06 00  = CommParameter[0] = concept    = 6     (BMW DS2 4-byte header)
80 25  = CommParameter[1] = baud       = 9600  (0x2580)
B8 00  = CommParameter[2] = ecu_type  = 0xB8  (формат-байт кадра)
00 00  = CommParameter[3] = 0
00 00  = CommParameter[4] = 0
D0 07  = CommParameter[5] = timeout   = 2000 ms
64 00  = CommParameter[6] = regen     = 100 ms
32 00  = CommParameter[7] = tel_end   = 50 ms
04 00  = CommParameter[8] = hdr_len   = 4     (длина заголовка кадра)
```

CommAnswerLen (SETTELPARAMETER, хранятся тоже в байткоде, offset ~0x91):
```
FD = -3  → len_offset = 3 (позиция LEN-байта в ответном кадре)
FF = -1  → ???
05 =  5  → len_add = 5 (total_frame = LEN_byte + 5)
00 =  0  → padding
```

---

## 2. EDIABAS IFH внутренний протокол (OBD.DLL ↔ EDIABAS API)

Трейс `ifh.trc` — **не байты на K-line**. Это внутренний протокол между
`OBD.DLL` (драйвер адаптера) и EDIABAS API.

Формат пакета:
```
[CMD][LEN_LO][LEN_HI][DATA...]
```
`LEN` = полная длина пакета (включая 3-байтовый заголовок).

### Ключевые команды

| CMD  | Название        | Описание                          |
|------|-----------------|-----------------------------------|
| 0x03 | GETBATTERYVOLTAGE | Напряжение АКБ                  |
| 0x04 | GETIGNITIONVOLTAGE | Напряжение зажигания            |
| 0x05 | SETECUCOMM      | Настроить параметры ECU (18 байт) |
| 0x06 | SENDTELGRAM     | Отправить кадр и получить ответ  |
| 0x0E | STOPECUTELFREQUENT | Остановить периодический опрос |
| 0x12 | GETHANDLERTYPE  | Тип адаптера ("OBD\0" = 4F 42 44 00) |
| 0x14 | SETTELPARAMETER | CommAnswerLen (4 байта)          |

### SETECUCOMM (CMD=0x05, 18 байт данных = 9 × u16 LE)

```
[concept][baud][ecu_type][p3][p4][timeout_std_ms][regen_ms][tel_end_ms][hdr_len]
```

Из трейса DDE40KW0 → DDE4.0:
```
06 00  concept    = 0x0006 (BMW DS2 4-byte)
80 25  baud       = 9600
B8 00  ecu_type   = 0x00B8 (кадр начинается с B8)
00 00  param3     = 0
00 00  param4     = 0
D0 07  timeout    = 2000 ms  (либо E8 03 = 1000 ms при первом вызове)
64 00  regen      = 100 ms
32 00  tel_end    = 50 ms
04 00  hdr_len    = 4
```

### SETTELPARAMETER (CMD=0x14, 4 байта)

CommAnswerLen для DDE4.0: `FD FF 05 00`
- `FD` = -3 → `len_offset` = 3
- `FF` = -1 → ???
- `05` = 5  → `len_add` = 5
- `00` = padding

CommAnswerLen для старого ECU (2-byte DS2): `FF FF 00 00`
- `FF` = -1 → `len_offset` = 1
- `FF` = -1 → ???
- `00` = 0  → `len_add` = 0
- `00` = padding

Вывод: `len_offset = -(CommAnswerLen[0])`, `len_add = CommAnswerLen[2]`

### Полная последовательность для DDE4.0 (из ifhold.trc, строки 3041-3085)

```
I→D: 05 15 00  06 00 80 25 B8 00 00 00 00 00 E8 03 64 00 32 00 04 00
                └─ SETECUCOMM (concept=6, baud=9600, ECU=B8, timeout=1000ms)
D→I: 01 03 00  (OK)

I→D: 14 06 00  FD FF 05
                └─ SETTELPARAMETER (CommAnswerLen: len_offset=3, len_add=5)
D→I: 01 03 00  (OK)

I→D: 06 08 00  B8 12 F1 01 A2
                └─ SENDTELGRAM (DS2 кадр без CHK, OBD.DLL добавляет CHK сам)
D→I: 01 33 00  B8 F1 12 2B E2 37 37...EC
                └─ Ответ ECU (полный кадр с CHK)
```

Байты на K-line: OBD.DLL добавляет CHK → `B8 12 F1 01 A2 F8`

---

## 3. BMW DS2 K-line протокол

### Кадр запроса (4-byte BMW DS2, concept 6)

```
[FMT=B8][TGT][SRC][LEN][SVC][DATA...][CHK_XOR]
```
- `FMT` = 0xB8 — идентификатор BMW DS2 4-byte формата (всегда B8)
- `TGT` = адрес ЭБУ (DDE4.0 = 0x12)
- `SRC` = адрес тестера (обычно 0xF1)
- `LEN` = количество payload-байт (SVC + DATA)
- `CHK` = XOR всех байт кадра кроме CHK

Пример — ReadEcuIdentification для DDE4.0:
```
B8 12 F1 01 A2 F8
│   │   │   │  │   └── CHK (B8^12^F1^01^A2)
│   │   │   │  └────── SVC = 0xA2 (DS2 ReadEcuIdentification)
│   │   │   └───────── LEN = 1 (1 payload byte: A2)
│   │   └───────────── SRC = 0xF1 (tester)
│   └───────────────── TGT = 0x12 (DDE4.0)
└───────────────────── FMT = 0xB8
```

### Кадр ответа (из трейса)

```
B8 F1 12 2B E2 37 37 38 39 33 37 36 35 33 30 31 31 ...
│   │   │   │   │
│   │   │   └── LEN = 0x2B = 43 (43 payload bytes)
│   │   └────── SRC = 0x12 (DDE4.0 отвечает)
│   └────────── TGT = 0xF1 (тестеру)
└────────────── FMT = 0xB8
```

`total_frame = LEN + len_add = 43 + 5 = 48 байт`
`remaining_after_header = total - hdr_len = 48 - 4 = 44 = 43 payload + 1 CHK`

### Старый DS2 (concept 1/5/6 для старых ECU)

```
[ADDR][LEN][SVC][DATA...][CHK]    (len_offset=1, len_add=0)
или
[ADDR][SRC][LEN][SVC][DATA...][CHK]  (len_offset=2, len_add=0)
```
Здесь LEN = полное кол-во байт кадра (включая ADDR и CHK).

### Физический слой

- **Baud**: 9600
- **Format**: 8E1 (8 data, Even parity, 1 stop bit)
- **Interface**: K-line (ISO 9141-2) — single-wire, half-duplex
- **Echo**: TX байты видны на RX (аппаратный loopback трансивера). Для KDCAN — echo=false (адаптер НЕ loopback)
- **Init**: DS2 (concept 1/5/6) НЕ требует 5-baud init. ECU отвечает немедленно.

### Концепты EDIABAS

| Концепт | Протокол           | len_offset | len_add | Инициализация |
|---------|--------------------|------------|---------|---------------|
| 0x0001  | DS2 старый (2-byte)| 1          | 0       | нет           |
| 0x0005  | DS2 старый (2-byte)| 1          | 0       | нет           |
| 0x0006  | BMW DS2 (4-byte)   | 3          | 5       | нет           |
| 0x0002  | KWP1281            | ?          | ?       | 5-baud init   |
| 0x0003  | KWP1281            | ?          | ?       | 5-baud init   |
| 0x010C  | KWP2000 BMW        | ?          | ?       | fast/5-baud   |
| 0x010F  | BMW-FAST           | ?          | ?       | ?             |
| 0x0110  | D-CAN              | —          | —       | CAN init      |

### BMW DS2 сервисы (service byte)

| SVC  | Название                |
|------|-------------------------|
| 0xA2 | ReadEcuIdentification   |
| 0xA0 | StartDiagnosticSession  |
| 0x04 | ClearDiagnosticInfo     |
| 0x05 | ReadDiagnosticTroubleCodes |
| 0x07 | ReadStatusOfDiagnosticTroubleCodes |
| 0x08 | ReadFreezeFrameData     |
| 0x1A | ReadDataByLocalIdentifier |
| 0x2C | DynamicallyDefineDataIdentifier |
| 0x31 | StartRoutineByLocalIdentifier |
| 0x3E | TesterPresent           |

---

## 4. KWP1281 (концепты 0x0002 / 0x0003)

Используется старыми BMW ECU (E36, E38 ранние).

### Инициализация: 5-baud init

1. Линия HIGH (idle) ≥ 300 ms
2. Отправить адрес ECU по 5 бод (bit-bang через BREAK): 1 старт-бит + 8 бит + 1 стоп-бит
3. Ждать 0x55 (sync byte) от ECU
4. Читать W1, W2 (keyword bytes)
5. Пауза 25 ms, отправить `~W2` (инверт W2)
6. ECU должен вернуть эхо `~W2`, затем свой адрес

### Особенности протокола

- Обмен байт за байтом: каждый принятый байт надо ACK-нуть через ~25 ms
- ECU отключается если нет активности > ~2 секунд (keepalive TesterPresent)
- Кадры: `[LEN][CTR][SVC][DATA...][CHK_sum]` (CMR = счётчик кадров)

---

## 5. KWP2000 BMW (концепт 0x010C)

ISO 14230-4 с BMW-расширениями.

### Формат кадра

```
[0x80|flags][DST][SRC][LEN][SVC][DATA...][CHK_sum]
```

### Инициализация

Два варианта:
- **5-baud init**: аналогично KWP1281
- **Fast init**: 25 ms LOW + 25 ms HIGH (wakeup pulse), затем StartDiagnosticSession (0x10)

### Ключевые сервисы

- `0x10` StartDiagnosticSession — обязателен перед другими запросами
- `0x1A` ReadDataByLocalIdentifier
- `0x7F` NegativeResponse — ответ ECU при ошибке

---

## 6. BMW DS2 — что нужно для DDE4.0

### Правильная последовательность (согласно ifh.trc)

```
1. SETECUCOMM: concept=6, baud=9600, ecu_type=B8, timeout=1000ms, regen=100ms
2. SETTELPARAMETER: CommAnswerLen=[FD, FF, 05, 00]
3. SENDTELGRAM: B8 12 F1 01 A2  (без CHK — OBD.DLL добавляет)
4. Ответ: B8 F1 12 2B E2 37 38 39 33 37 36...EC
```

Байты реально на K-line:
```
TX: B8 12 F1 01 A2 F8   (с CHK = B8^12^F1^01^A2 = F8)
RX: B8 F1 12 2B E2 37...EC
```

**Почему INPA работает, а наш инструмент — нет** — возможные причины (по вероятности):

1. **Эхо не дренируется** (наиболее вероятно).
   K-line всегда отражает TX байты обратно на RX. Без `--echo` флага наш инструмент
   принимает собственные TX байты как начало ответа ECU, checksum не сходится, ответ "не получен".
   **Фикс**: всегда передавать `--echo` если адаптер имеет однопроводной K-line.

2. **DTR не взводится** (для ADS-style адаптеров).
   Некоторые K-line кабели (ADS-style, L9637D трансивер) требуют DTR=HIGH для включения
   K-line-драйвера. INPA взводит DTR. Наш инструмент взводит DTR в serial.rs,
   но надо убедиться что это так и происходит.

3. **KDCAN ожидает STD:OBD, а не сырой K-line**.
   Если KDCAN-кабель имеет прошивку STD:OBD, он ожидает команды SETECUCOMM/SENDTELGRAM.
   Сырые K-line байты он просто игнорирует. INPA использует STD:OBD. Наш `raw` обходит его.
   Но пользователь сказал "адаптер тупой ретранслятор" — то есть это НЕ STD:OBD.

4. **INITIALISIERUNG не запущен**.
   DDE4.0 может требовать специфического первого запроса перед ответом на запросы данных.
   Без `INITIALISIERUNG` ECU может не реагировать.

5. **Inter-frame timing**.
   После предыдущей коммуникации ECU нужно ≥100ms idle. regen_time=20ms может быть мало.

---

## 7. Готовые проекты-аналоги на GitHub

| Проект | URL | Язык | Протоколы |
|--------|-----|------|-----------|
| **EdiabasLib** | https://github.com/uholeschak/ediabaslib | C# (.NET) | DS1/DS2, KWP1281, KWP2000 BMW, BMW-FAST, D-CAN |
| **ediabasx** | https://github.com/emdzej/ediabasx | TypeScript+C11 | KWP2000, UDS, DoIP, HSFZ; 184 BEST/2 опкода |
| **bmw-best2-vm** | https://github.com/lpcvoid/bmw-best2-vm | C++ | только BEST/2 VM (без транспорта) |
| **bmw-coding** | https://github.com/oleavr/bmw-coding | Python | DS2 (прямой K-line) |
| **ds2 (Arduino)** | https://github.com/handmade0octopus/ds2 | C++ Arduino | DS2 с L9637D трансивером |
| **bmwe46oil** | https://github.com/tomicooler/bmwe46oil | C++/QML | DS2 |
| **pydiabas** | https://github.com/BembelBytes/pydiabas | Python | обёртка над Win32 EDIABAS API |

### EdiabasLib — ключевые детали (uholeschak)

Самый полный аналог. Реализует весь стек от BEST/2 до транспорта.

**CommParameter для DS2 (concept 0x0006)**:
```
[0] concept         = 0x0006
[1] baud rate       (int)
[2] не используется  для DS2
[3] не используется
[4] не используется
[5] ParTimeoutStd   = таймаут ответа (ms)
[6] ParRegenTime    = пауза между запросами (ms)
[7] ParTimeoutTelEnd = таймаут конца телеграммы (ms)
[8] ParInterbyteTime = пауза между байтами TX (необязательно)
```

**Эхо (echo) в EdiabasLib**:
```csharp
if (concept == 6)  // DS2
{
    ParSendSetDtr = !HasAdapterEcho;
}
```
- `HasAdapterEcho = true`  → однопроводной K-line → DTR не нужен
- `HasAdapterEcho = false` → ADS-стиль (раздельные TX/RX) → DTR **обязателен** при передаче

**DS2 frame format** (подтверждено всеми источниками):
```
[ADDR][LEN][SVC][DATA...][CHK_XOR]
```
- `LEN` = полная длина кадра (все байты включая ADDR, LEN, CHK)
- `CHK` = XOR всех байт кроме CHK

Из `oleavr/bmw-coding` — `_execute()`:
1. Записывает кадр
2. Читает N байт эхо (echo drain)
3. Ставит timeout=5 секунд
4. Читает ответ ECU

Из `handmade0octopus/ds2`:
> "if you send any command on DS2 it responds with whatever was send due to way K-line is constructed"

---

## 8. Параметры DDE4.0 (BMW E38/39/46 дизель M57)

- **ECU addr**: 0x12 (в поле TGT BMW DS2 кадра)
- **Протокол**: BMW DS2 (concept 6), 9600 8E1
- **Кадр-идентификатор**: FMT=0xB8
- **Источник (SRC)**: 0xF1 (тестер)
- **ReadEcuIdentification**: сервис 0xA2
- **Пример идентификации** (из api.trc):
  - `ID_MOTOR` = "M57_DDE40"
  - `ID_BMW_NR` = "7789376"
  - `ID_ZYKLUS` = "C029"
- **CommParameter из INITIALISIERUNG**:
  - concept=6, baud=9600, ecu_type=0xB8
  - timeout_std=2000ms, regen=100ms, tel_end=50ms
  - CommAnswerLen: len_offset=3, len_add=5

---

## 8b. CommParameter — форматы в зависимости от концепта

### K-line (концепты 0x0001...0x010F): 18 байт = 9 × u16 LE

```rust
struct CommParam18 {
    concept:      u16,  // [0] 0x0001, 0x0005, 0x0006, 0x010C, 0x010F
    baud:         u16,  // [1] 9600
    ecu_type:     u16,  // [2] 0xB8 для DDE4, 0x80 для IKE, 0x0D для KMB-E36
    _unused:      u16,  // [3] всегда 0
    _unused2:     u16,  // [4] всегда 0
    timeout_std:  u16,  // [5] ms, стандартный таймаут ответа
    regen:        u16,  // [6] ms, пауза между кадрами
    tel_end:      u16,  // [7] ms, таймаут окончания телеграммы
    hdr_len:      u16,  // [8] 4 для 4-byte DS2, 0 для старого DS2
}
```

### D-CAN (концепт 0x0110): 120 байт = 30 × u32 LE

```rust
struct CommParam120 {
    concept:      u32,  // [0] 0x0110 (D-CAN)
    baud:         u32,  // [1] 500000 (500 kbps CAN)
    timeout_std:  u32,  // [2] ms
    // [3..29] дополнительные параметры CAN, ISO-TP, UDS
    // [9] = 5000ms (длинный таймаут для тяжёлых операций)
}
```

---

## 8c. Как EDIABAS жонглирует между протоколами

### Распределение протоколов по ECU

| Концепт | Протокол         | Примеры ECU                                    | Кол-во .prg |
|---------|------------------|------------------------------------------------|-------------|
| 0x0001  | DS2 (2-byte)     | KMB/BC E36, ABS-MK4, ASC-MK4, E31 ECU         | 49          |
| 0x0002  | KWP1281          | EML12 (M70, 4800 baud)                         | 1           |
| 0x0005  | DS1              | CCM E38, LME E38, ZKE4 E36                     | 9           |
| 0x0006  | BMW DS2 (4-byte) | DDE4.0, IKE/KOMBI E39+, TELIBUS, большинство   | 402         |
| 0x0110  | D-CAN            | DSC, MSD87, все F-series/G-series ECU          | 426         |

### Физический уровень

```
K-line (DS2/KWP):
  OBD pin 8 → KDCAN в режиме K-line → /dev/ttyUSB0 → сериальный порт 9600 8E1
  Все K-line ECU делят одну шину, адресуются полем TGT в DS2 кадре

D-CAN:
  OBD pin 6/14 → KDCAN в режиме CAN → другой USB endpoint (SLCAN или FT-CAN)
  CAN bus 500kbps, протокол ISO-TP (ISO 15765-2) + UDS (ISO 14229)
```

**Физический переключатель на KDCAN кабеле** выбирает между K-line и CAN режимами.

### Механизм переключения внутри EDIABAS

1. INPA загружает `DDE40KW0.prg` → запускает INITIALISIERUNG
2. INITIALISIERUNG вызывает `xconnect` с CommParam = [concept=6, baud=9600, ecu=B8, ...]
3. IFH перенастраивает COM-порт: 9600 8E1, тип кадра = BMW DS2 4-byte
4. Все последующие `xrequf` шлют `B8 [TGT] F1 LEN SVC...` на K-line

5. Переключение на KOMBI E39:
6. INPA загружает `KOMBI39.prg` → запускает INITIALISIERUNG  
7. `xhangup` (при необходимости) → `xconnect` с CommParam = [concept=6, baud=9600, ecu=0x80, ...]
8. **Тот же COM-порт, тот же протокол** — только другой адрес ECU (0x80 vs 0x12)
9. Кадры идут: `80 [IKE_ADDR] F1 LEN SVC...`

10. Переключение на новый ECU (концепт D-CAN):
11. INPA загружает `DSC_01.PRG` → INITIALISIERUNG с CommParam120 = [concept=0x110, baud=500000, ...]
12. IFH переключает KDCAN в CAN-режим, ISO-TP транспорт

### ecu_type (CommParameter[2]) для концепта 6

Это первый байт в BMW DS2 4-byte кадре (поле FMT):
```
0xB8 = 0b10111000 → DDE4.0, другие engine ECU
0x80 = 0b10000000 → IKE, KOMBI E39/E38
0xC8 = TELIBUS
0xD0 = LME E38
0x0D = KOMBI E36 (это концепт 0x0001, 2-byte формат)
```

Формат кадра для concept 6, ecu_type=FMT:
```
[FMT][TGT][SRC][LEN][SVC][DATA...][CHK]
│    └── физический адрес ECU (12 для DDE4, 80 для IKE...)
└── тип кадра (B8 для большинства, 80 для некоторых)
```

---

## 9. BEST/2 VM — xconnect и инициализация соединения

Опкод `0x26` = xconnect. Выполняет:
1. Устанавливает CommParameter в транспорт
2. Открывает соединение (для DS2 — тривиально, для KWP — init sequence)
3. Устанавливает CommAnswerLen (len_offset, len_add)

Опкод `0x27` = xhangup. Закрывает соединение.

Из дизассемблера INITIALISIERUNG DDE40KW0.prg:
```
0x0000: xconnect   ← сразу при входе в INITIALISIERUNG
...
0x4B:   [move CommParameter data] = 06 00 80 25 B8 00 ... D0 07 64 00 32 00 04 00
0x91:   [move CommAnswerLen data] = FD FF 05 00
```

INITIALISIERUNG не просто "инициализирует" — он НАСТРАИВАЕТ протокол.
Без правильного выполнения INITIALISIERUNG raw-send не будет работать корректно.

### Текущее состояние vm.rs (проблема)

```rust
// vm.rs line 1303-1304 — СЛОМАНО: xconnect и xhangup просто пропускаются!
[0x26, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }  // xconnect SKIP
[0x27, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }  // xhangup  SKIP
[0x2c, ..] if ip + 1 < code.len() => {                               // xrequf: только лог
    eprintln!("[xrequf@{ip:#x}] raw bytes...");
    ip = next;
}
```

Также на строке 126: грубый хак "если код начинается с 0x26, прыгнуть на +17".
Это неправильно — не читает CommParameter, транспорт не настраивается.

### Что нужно реализовать

```rust
// xconnect (0x26): читает CommParameter из следующего RegisterS/ImmStr
// CommParameter хранится как move S-reg, ImmStr (78 00 [data])
// Затем: concept → выбрать транспорт, baud → переоткрыть порт, timeouts → установить
[0x26, ..] => {
    // 1. Прочитать CommParameter из следующей инструкции (ImmStr)
    // 2. u16[0]=concept, u16[1]=baud, u16[2]=ecu_type, u16[5]=timeout...
    // 3. Вызвать transport.configure(&CommConfig {...})
}

// xrequf (0x2C): ОТПРАВИТЬ кадр и получить ответ — это ГЛАВНАЯ команда
[0x2c, ..] => {
    // Читать данные телеграммы из регистра или ImmStr
    // Вызвать transport.exchange(telegram)
    // Сохранить ответ в указанный регистр
}
```
