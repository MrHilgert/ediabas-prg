# PROTO-KWP1281 — спецификация транспорта KWP1281 (концепты 0x0002 / 0x0003)

Байт-уровневая спецификация для реализации `src/transport/kwp1281.rs`.
Целевые ЭБУ: legacy BMW — старые DDE2.x diesel, ABS, IHKA (EDIABAS концепты
`0x0002` и `0x0003`). Физика — та же однопроводная K-line через FTDI/KDCAN,
что и DS2, но канальный уровень **принципиально другой**: это не
запрос/ответ-фреймы, а непрерывный *блочный диалог* с побайтовым
комплемент-подтверждением и общим счётчиком блоков.

Источники (см. §14): EdiabasLib `EdInterfaceObd.cs` (авторитетная реализация
концептов 2/3), прошивка `mnaberez/vwradio kwp1281_tool` (открытая, хорошо
документированная реализация того же канального уровня VAG KW1281),
`gmenounos/kw1281test`, `domnulvlad/KLineKWP1281Lib`.

---

## 1. Ключевой архитектурный факт

BEST/2-байткод (VM) строит телеграмму и вызывает `transport.exchange(frame)` —
он ожидает семантику «послал запрос → получил ответ». KWP1281 же — это
**сессия из чередующихся блоков**: после init стороны обязаны ходить строго
по очереди, каждый блок несёт инкрементирующийся счётчик, каждый байт
(кроме последнего) эхо-подтверждается комплементом, а при простое > ~1 с
сессия рвётся.

Отображение на наш `Transport`:

| Trait-метод | KWP1281-смысл |
|---|---|
| `configure(cfg)` | запомнить baud/таймауты; порт = 9600, **8N1 (parity None!)** |
| `init_connection()` | 5-baud init → 0x55 → KB1/KB2 → ~KB2 → приём ident-блоков ЭБУ (0xF6×N) с ACK'ами до ACK от ЭБУ. После этого сессия «в покое», ход за нами. |
| `exchange(frame)` | послать ОДИН блок-запрос (`frame` = `[title][data…]`), затем читать блоки ЭБУ, ACK'ая каждый data-блок, пока ЭБУ не ответит ACK (0x09). Вернуть склеенные payload'ы. |
| `disconnect()` | послать блок EndOutput (0x06), сбросить состояние. |

**Состояние, живущее МЕЖДУ вызовами `exchange`** (поля структуры):
- `block_counter: u8` — общий счётчик блоков (см. §5). Уже есть в стабе.
- `connected: bool` — init прошёл.
- `last_block_time: Instant` — момент конца последнего блока (keep-alive, §9).
- `last_tx_block: Vec<u8>` — последний посланный блок (повтор при NAK).

---

## 2. Физический уровень

- **Baud: 9600, 8 data bits, parity None, 1 stop** (EdiabasLib, концепты 2 и 3:
  `baud 9600, SerialParity.None`). Это отличие от DS2 (Even)! Наш
  `CommConfig::parse` уже даёт `Parity::None` для не-DS2 — ок.
- Baud берётся из `CommParameter[1]` .prg (обычно 9600; other — доверять .prg).
- **Эхо**: однопроводная K-line + KDCAN зеркалит каждый TX-байт в RX.
  КАЖДАЯ запись обязана вычитать своё эхо (`read_exact` 1 байт) и сверить его
  с посланным байтом — несовпадение = коллизия на шине → ошибка протокола.
- EdiabasLib для концептов 2/3 ставит `ParSendSetDtr = true` — при open
  дёрнуть `driver.set_dtr(true)` (для FTDI-кабелей, питающих K-line драйвер
  от DTR). Для нашего KDCAN, судя по DS2-опыту, не критично, но сделать.
- Адрес пробуждения (5-baud address) = `wake_addr` из `CommParameter[2]`
  (EdiabasLib: `ParWakeAddress = CommParameterProtected[2]`); поле
  `CommConfig::wake_addr` уже существует.

---

## 3. 5-baud init (в `init_connection`)

Основа — уже рабочий `Ds2Transport::kline_5baud_init` (src/transport/ds2.rs,
строки ~192-242): та же побитовая передача через `set_break`, тот же приём
0x55/KB1/KB2 и отправка ~KB2. **Отличия от ds2-версии помечены ⚠.**

Порт уже открыт на 9600 8N1. Последовательность:

```
1. driver.flush_rx()
2. Шина idle HIGH:      set_break(false); sleep(300 ms)
   ⚠ после НЕУДАЧНОЙ попытки или после disconnect ждать 2600 ms
     (EdiabasLib Kwp1281InitDelay = 2600) перед новым init.
3. Старт-бит LOW:       set_break(true);  sleep(200 ms)      // BIT_MS = 200
4. 8 бит адреса, LSB-first, по 200 ms каждый:
       for bit_pos in 0..8:
           bit = (addr >> bit_pos) & 1
           set_break(bit == 0)      // бит 0 = LOW = break ON
           sleep(200 ms)
5. Стоп-бит HIGH:       set_break(false); sleep(200 ms)
6. driver.flush_rx()    // break мог насыпать мусор (0x00/0xFF) в RX FIFO
7. set_timeout(3000); в цикле read_exact(1) до байта 0x55 (sync).
   Штатно 0x55 приходит через W1 = 60..300 ms после стоп-бита.
8. read_exact(KB1); read_exact(KB2)
   // W2 (0x55→KB1) = 5..20 ms, W3 (KB1→KB2) = 0..20 ms — просто читаем.
   Для KWP1281 ожидаем KB1 = 0x01, KB2 = 0x8A:
   protocol = ((KB2 & 0x7F) << 7) | (KB1 & 0x7F) = (0x0A << 7) | 0x01 = 1281.
   KB1/KB2 логировать; при других значениях — Error::Protocol
   (это не-1281 клавиатура, напр. 0x8F/0xE9 = KWP2000).
9. sleep(30 ms)                          // W4 = 25..50 ms (vwradio: 30)
10. write(&[!KB2]) = write(&[0x75]); read_exact(эхо 1 байт, == 0x75)
11. ⚠ В ОТЛИЧИЕ от ISO 9141-2 / ds2.rs: ЭБУ НЕ шлёт инвертированный адрес
    после ~KB2. Следующий байт на шине — это уже LEN-байт первого
    ident-блока ЭБУ. НЕ читать два «лишних» байта, как делает
    kline_5baud_init. (Толерантный вариант: если первый байт == !addr —
    съесть его и читать дальше; пометка для живой проверки, §13.)
12. Приём ident-блоков — см. §8.2.
```

Формат адреса в п.4: как в ds2.rs — 8 сырых бит. ⚠ VAG-классика шлёт адрес
как «7 бит + odd parity в бите 7»: `addr7 = addr & 0x7F;
tx = addr7 | (odd_parity(addr7) << 7)`. Для BMW-концепта-2 живой проверки
нет; реализовать сырые 8 бит (совместимо с ds2.rs), фолбэк с odd-parity —
флагом. См. §13.

После init никаких смен baud нет — диалог идёт на тех же 9600 8N1.

---

## 4. Структура блока

```
[LEN] [COUNTER] [TITLE] [DATA ...] [0x03]
  │       │        │       │         └─ block end, ВСЕГДА 0x03
  │       │        │       └─ 0..N байт данных
  │       │        └─ block title (= service ID), §7
  │       └─ счётчик блоков, §5
  └─ длина блока
```

**Семантика LEN**: LEN = число байт блока БЕЗ самого LEN-байта, но
ВКЛЮЧАЯ конечный 0x03. Эквивалентная формулировка (численно та же):
LEN = число байт ВКЛЮЧАЯ сам LEN, но БЕЗ 0x03.

- Всего байт на проводе: `LEN + 1`.
- Для блока с D байтами данных: `LEN = D + 3`.
- Минимальный блок (без данных, напр. ACK): `LEN = 0x03` →
  `03 <ctr> 09 03` — 4 байта на проводе.
- Пример: чтение группы 0x0F: `04 <ctr> 29 0F 03` (LEN=4, D=1).

Контрольной суммы НЕТ — целостность обеспечивает побайтовый
комплемент-хендшейк (§6) + проверка счётчика (§5) + фиксированный 0x03.

Приём: читаем LEN → знаем, что дальше ровно LEN байт, последний обязан
быть 0x03 (иначе `Error::Protocol("KWP1281: bad block end")`).

---

## 5. Счётчик блоков (`block_counter`)

- Счётчик **ОДИН НА СЕССИЮ, ОБЩИЙ ДЛЯ ОБОИХ НАПРАВЛЕНИЙ**. Каждый блок —
  чей бы он ни был — увеличивает его на 1, `wrapping_add(1)` (mod 256,
  0xFF → 0x00).
- Инициализирует ЭБУ: его первый ident-блок после init несёт counter = 0x01.
- Правила реализации (как в vwradio: `buf[1] = ++kwp_block_counter` при TX,
  адоптирование при RX):
  - **TX**: `self.block_counter = self.block_counter.wrapping_add(1);`
    в блок пишется новое значение.
  - **RX**: ожидаем `received == self.block_counter.wrapping_add(1)`.
    При расхождении — залогировать и **адоптировать**
    (`self.block_counter = received`) — толерантность важнее строгости,
    старые ЭБУ иногда сбоят. При этом сам факт расхождения после ACK-обмена —
    признак рассинхрона, можно эскалировать после N подряд.
- В `init_connection` перед init: `self.block_counter = 0` (тогда первый
  RX-блок с counter=1 пройдёт проверку `0+1`).
- Поле уже заведено в стабе `Kwp1281Transport` — оно и персистит между
  вызовами `exchange`.

---

## 6. Комплемент-хендшейк (побайтовое подтверждение)

Правило: **каждый байт блока, КРОМЕ последнего (0x03), подтверждается
приёмником отправкой его побитового комплемента** `!b` (`b ^ 0xFF`).
Действует В ОБЕ СТОРОНЫ. Финальный 0x03 НЕ комплементируется никем.

### 6.1 `send_byte_ack(b)` — послать байт, дождаться комплемента

```
driver.write(&[b])
read_exact(&mut echo, 1)            // эхо KDCAN; echo != b → Error::Protocol("bus collision")
driver.set_timeout(55)              // EdiabasLib Kwp1281ByteTimeout = 55 ms
read_exact(&mut compl, 1)           // таймаут → байт не подтверждён
if compl != !b → Error::Protocol("KWP1281: bad complement")
sleep(interbyte_ms)                 // ≥1 ms перед следующим байтом (vwradio: 1)
```

### 6.2 `recv_byte_ack()` — принять байт, ответить комплементом

```
read_exact(&mut b, 1)               // таймаут: 55 ms внутри блока
sleep(1..2 ms)                      // ЭБУ нужна пауза до нашего комплемента;
                                    // окно W4: комплемент в течение 1..50 ms
driver.write(&[!b])
read_exact(&mut echo, 1)            // вычитать СВОЁ эхо комплемента (== !b)
return b
```

### 6.3 `send_raw_byte(b)` / `recv_raw_byte()` — для 0x03

Как выше, но без комплемент-фазы: write+эхо / read без ответа.

### 6.4 Отправка блока целиком

```
fn send_block(title, data):
    self.block_counter = self.block_counter.wrapping_add(1)
    buf = [ (data.len() + 3) as u8, self.block_counter, title ] ++ data
    for b in buf:  send_byte_ack(b)
    send_raw_byte(0x03)
    self.last_tx_block = buf        // для повтора при NAK
    self.last_block_time = now()
```

### 6.5 Приём блока целиком

```
fn recv_block(first_byte_timeout_ms) -> (title, data):
    driver.set_timeout(first_byte_timeout_ms)   // 1-й байт: timeout_std_ms
    len = recv_byte_ack()                       // LEN тоже комплементируется!
    if len < 3 → Error::Protocol
    driver.set_timeout(55)                      // остальные байты — по 55 ms
    ctr   = recv_byte_ack()                     // проверка счётчика, §5
    title = recv_byte_ack()
    data  = (0 .. len-3).map(|_| recv_byte_ack())
    end   = recv_raw_byte()                     // БЕЗ комплемента
    if end != 0x03 → Error::Protocol
    self.block_counter = ctr                    // адоптируем (§5)
    self.last_block_time = now()
    sleep(10 ms)          // межблочная пауза перед НАШИМ следующим блоком
                          // (vwradio KWP_INTERBLOCK_MS = 10; допустимо 5..50)
    return (title, data)
```

Ошибка комплемента / таймаут байта = ошибка всего блока → ретрай на уровне
диалога: до 3 повторов с паузой 150 ms (EdiabasLib `Kwp1281ErrorRetries = 3`,
`Kwp1281ErrorDelay = 150`). После 3 неудач — сессия считается потерянной
(`connected = false`), нужен новый `init_connection`.

---

## 7. Block titles (SID)

Запросы тестера (подтверждено vwradio kwp1281.h + EdiabasLib):

| Title | Значение | Ответ ЭБУ |
|---|---|---|
| `0x00` | Read identification | `0xF6` (ASCII data) ×N |
| `0x01` | Read RAM | data-блок |
| `0x03` | Read ROM/EEPROM | data-блок |
| `0x04` | Actuator/output test | `0xF5` (не подтверждено, §13) |
| `0x05` | Clear fault codes | `0x09` ACK |
| `0x06` | **End output / disconnect** | ACK или тишина |
| `0x07` | Read fault codes (DTC) | `0xFC` ×N |
| `0x08` | Single reading (channel) | data-блок |
| `0x09` | **ACK** — подтверждение/keep-alive | — |
| `0x0A` | **NAK** — повторить последний блок | — |
| `0x0C` | Write EEPROM | ACK |
| `0x10` | Recoding | `0xF6` |
| `0x19` | Read EEPROM | data-блок |
| `0x1B` | Custom / manufacturer specific | data-блок |
| `0x29` | Group reading (measuring blocks), data=[группа] | `0xE7` |
| `0x2B` | Login (data = 2 байта кода + 3 байта) | ACK |

Ответы ЭБУ: `0xF6` ASCII-данные (ident, коды), `0xFC` fault codes
(по 3 байта на DTC: 2 байта кода + 1 байт статуса), `0xE7` group reading
answer (по 3 байта на величину: id формулы + 2 байта), `0x09` ACK,
`0x0A` NAK.

Для BMW-SGBD реальный title приходит из .prg — транспорту таблица нужна
только для распознавания `0x09`/`0x0A`/`0xF6` в диалоге; остальное он
передаёт прозрачно.

---

## 8. Диалог

### 8.1 `exchange(frame)` — один запрос VM → блочный диалог

Контракт: `frame = [TITLE][DATA…]` (title + payload, БЕЗ LEN/counter/0x03 —
их добавляет транспорт). Возврат: склейка `[TITLE][DATA…]` всех data-блоков
ответа (без LEN/counter/0x03).

```
fn exchange(frame):
    if !connected → Error
    keep_alive_if_idle()                        // §9
    driver.flush_rx()
    send_block(frame[0], &frame[1..])           // с ретраями §6
    out = Vec::new()
    loop:
        (title, data) = recv_block(timeout_std_ms)
        match title:
            0x09 (ACK)  → break                 // ЭБУ подтвердил — диалог завершён
            0x0A (NAK)  → resend last_tx_block  // ≤3 раз, иначе Error
            _ (data)    → out.push(title); out.extend(data);
                          send_block(0x09, &[]) // ACK'аем, ЭБУ может слать ещё
    return out
```

Свойства:
- Многоблочные ответы (FS_LESEN → несколько 0xFC) собираются в один буфер.
- Одноблочные ответы стоят один лишний раунд ACK→ACK (~50-100 ms) — это
  цена универсального правила «читать до ACK». Оптимизация (сразу выходить
  после известного одноблочного title и слать следующий запрос как
  ответ-блок) возможна позже, но усложняет состояние «чей ход».
- После выхода из `exchange` ход снова за тестером — инвариант сессии.
- ⚠ Что именно ждёт BEST/2-байткод концепта-2 в телеграмме `xrecv`
  (только `[title][data]` или с LEN/counter) — сверить по concept-2 .prg
  из корпуса до фиксации контракта. См. §13.

### 8.2 Ident-блоки после init (внутри `init_connection`)

Сразу после ~KB2 ЭБУ шлёт 3-4 блока `0xF6` с ASCII (номер детали, название,
кодировка/WSC). Каждый принять `recv_block` и ответить `send_block(0x09)`;
цикл до тех пор, пока ЭБУ не пришлёт ACK-блок (0x09) в ответ на наш ACK:

```
loop:
    (title, data) = recv_block(1500)
    if title == 0x09 → break            // ЭБУ перешёл в idle — init завершён
    if title == 0xF6 → сохранить ASCII в self.ident (пригодится GUI)
    send_block(0x09, &[])
connected = true
```

Не выгребя ident-блоки, слать запрос нельзя — нарушится очерёдность.

### 8.3 `disconnect()`

```
send_block(0x06, &[])         // EndOutput; ответ (ACK) читать best-effort,
                              // тишину не считать ошибкой
connected = false; block_counter = 0
// до следующего init_connection выдержать ≥2600 ms тишины на шине
```

---

## 9. Keep-alive (критично!)

ЭБУ рвёт сессию, если между блоками пауза больше ~1.1 c. EdiabasLib
(`IdleKwp1281`, `Kwp1281StatusTimeout = 1000`) в простое гоняет пинг-понг
ACK↔ACK каждую ~1 с.

У нашего `Transport` нет фонового потока, поэтому:
1. В начале `exchange`: если `now() - last_block_time > ~900 ms` — сессия,
   возможно, уже мертва; всё равно пробуем, при ошибке — авто-reinit
   (однократный `init_connection` + повтор) либо честная ошибка наверх.
2. **Архитектурное требование к Session/GUI**: при открытой KWP1281-сессии
   вызывать `exchange(&[0x09])` (пустой ACK) не реже раза в секунду, если
   нет полезных запросов. Либо добавить в `Kwp1281Transport` inherent-метод
   `idle_tick()` и дергать его из воркер-цикла Session. (Для нашего GUI с
   постоянным поллингом датчиков это почти всегда выполняется само.)

---

## 10. Сводная таблица таймингов

| Параметр | Значение | Источник / примитив |
|---|---|---|
| 5-baud бит | 200 ms | ISO; ds2.rs `BIT_MS` |
| Idle перед init | 300 ms (первая попытка) | ds2.rs |
| Пауза перед re-init | 2600 ms | EdiabasLib `Kwp1281InitDelay` |
| W1: стоп-бит → 0x55 | 60–300 ms (код: `set_timeout(3000)`) | ISO 9141 |
| W2: 0x55 → KB1 | 5–20 ms | ISO 9141 |
| W3: KB1 → KB2 | 0–20 ms | ISO 9141 |
| W4: KB2 → тестер шлёт ~KB2 | 25–50 ms, использовать `sleep(30)` | ISO; vwradio `KWP_POSTKEYWORD_MS=30` |
| W4 побайтово: байт → комплемент | таймаут `set_timeout(55)` | EdiabasLib `Kwp1281ByteTimeout=55` |
| Наш комплемент после приёма байта | `sleep(1..2 ms)` до write | vwradio |
| Межбайтовая пауза TX (после комплемента) | ≥1 ms (`sleep(interbyte_ms)`, default 1–2) | vwradio `KWP_INTERBYTE_MS=1` |
| Межблочная пауза (конец RX-блока → наш блок) | 10 ms (допустимо 5–50) | vwradio `KWP_INTERBLOCK_MS=10` |
| 1-й байт ответного блока | `set_timeout(timeout_std_ms)` (деф. 1000–3000) | EdiabasLib 1000 / vwradio 3000 |
| Keep-alive при простое | ACK каждые ~1000 ms | EdiabasLib `Kwp1281StatusTimeout=1000` |
| Ретраи блока при ошибке | 3 × пауза 150 ms | EdiabasLib `Kwp1281ErrorRetries/ErrorDelay` |

`set_timeout` дёргается часто (55 ms внутри блока ↔ 1000+ ms на первый байт) —
это дешёвая операция у serialport, допустимо.

---

## 11. Отображение на код (итог)

```
configure(cfg):
    cfg сохранить; driver.set_timeout(cfg.timeout_std_ms)
    // порт должен быть открыт 9600 8N1 (cfg.parity=None уже из parse())

init_connection():
    set_dtr(true); flush_rx()
    block_counter = 0
    5-baud init по §3 (адрес = cfg.wake_addr, иначе Error::Protocol
        "KWP1281: wake address is required")
    проверить KB1/KB2 == 0x01/0x8A (лог + мягкая проверка)
    приём ident-блоков §8.2 → connected = true

exchange(frame):     // frame = [title][data…]
    §8.1; персистентно: block_counter, last_tx_block, last_block_time

disconnect():
    §8.3
```

Ошибки — существующий `Error::Protocol(String)`; `Error::Checksum` не
используется (контрольной суммы нет).

---

## 12. Концепт 0x0003 — отличие

По EdiabasLib концепт 3 имеет ту же физику и init (9600, parity None,
key bytes), но ДРУГОЙ обмен: `TransConcept3` / `IdleConcept3` /
`FrequentConcept3` — «frequent mode», ЭБУ после init сам циклически шлёт
данные (broadcast), без блочного пинг-понга KW1281. Наш
`Protocol::from_concept` сейчас маппит 0x0003 → `Kwp1281` — для первой
итерации допустимо (init одинаков), но полноценный концепт-3 потребует
отдельного receive-only режима. Зафиксировать в PLAN.md как отдельный пункт.

---

## 13. Требует подтверждения на живом железе / корпусе

1. **Формат 5-baud адреса**: сырые 8 бит (как ds2.rs) vs VAG-стиль
   «7 бит + odd parity в бите 7». Реализовать 8-битный вариант, фолбэк
   с parity — под флагом.
2. **Байт после ~KB2**: убедиться, что BMW-концепт-2 ЭБУ НЕ шлёт
   инвертированный адрес (реализовать толерантно: если первый байт == !addr —
   проглотить).
3. **Контракт `exchange` с VM**: что кладёт EDIABAS в телеграмму для
   концепта-2 jobs — `[title][data]` или сырой блок с LEN/counter. Проверить
   по concept-2 SGBD из корпуса (ecu/: старые DDE2/IHKA/ABS .prg — найти
   через `CommParameter` с концептом 0x0002) ДО фиксации формата возврата.
4. Ответный title актуаторного теста (0xF5) — не подтверждён.
5. Точный порог обрыва сессии по простою у BMW-ЭБУ (принято ~1.1 c,
   EdiabasLib пингует раз в 1 c).
6. KB1/KB2 у конкретных BMW legacy ЭБУ (ожидаем 0x01/0x8A; при 0x8F/0xE9 —
   это KWP2000, маршрутизировать в другой транспорт).

---

## 14. Источники

- EdiabasLib — https://github.com/uholeschak/ediabaslib,
  `EdiabasLib/EdiabasLib/EdInterfaceObd.cs`: концепты 2/3 (9600/None,
  `TransKwp1281`/`IdleKwp1281`/`FinishKwp1281`), константы
  `Kwp1281ByteTimeout=55`, `Kwp1281StatusTimeout=1000`,
  `Kwp1281ErrorDelay=150`, `Kwp1281ErrorRetries=3`, `Kwp1281InitDelay=2600`,
  `Kwp1281Ack=0x09`, `Kwp1281Nack=0x0A`, `Kwp1281EndOutput=0x06`,
  `ParWakeAddress=CommParameter[2]`.
- vwradio KWP1281 tool — https://github.com/mnaberez/vwradio
  (`kwp1281_tool/firmware/kwp1281.c/.h`): семантика LEN, `buf[1] =
  ++kwp_block_counter`, комплемент всех байт кроме 0x03, таблица titles,
  `KWP_INTERBYTE_MS=1`, `KWP_INTERBLOCK_MS=10`, `KWP_POSTKEYWORD_MS=30`.
- kw1281test — https://github.com/gmenounos/kw1281test (референс диалога).
- KLineKWP1281Lib — https://github.com/domnulvlad/KLineKWP1281Lib.
- Наш рабочий референс 5-baud: `src/transport/ds2.rs::kline_5baud_init`.
