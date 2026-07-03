# PROTO-DCAN — D-CAN transport (EDIABAS concept 0x0110) — implementation spec

Target: `src/transport/dcan.rs` (`DCanTransport`), driven through the existing
`Driver` / `Transport` traits. This document is the byte-level contract a Rust
developer implements from — no Rust in here, only wire formats, algorithms and
pseudocode.

Primary source: **EdiabasLib** (github.com/uholeschak/ediabaslib, `master` as of
2026-07), which is the authoritative open reimplementation of BMW EDIABAS. Every
claim below is tagged:

- **[CONFIRMED]** — read directly out of EdiabasLib source (file + line cited) or ISO 15765-2.
- **[LIKELY]** — strongly implied by EdiabasLib / community knowledge, not directly executable-verified.
- **[UNVERIFIED]** — needs live-hardware confirmation with our KDCAN clone on a D-CAN car.

Cited files (fetched 2026-07-03, kept line-accurate):
`EdiabasLib/EdiabasLib/EdInterfaceObd.cs`, `EdInterfaceBase.cs`,
`EdCustomAdapterCommon.cs`, `EdElmInterface.cs`, `EdFtdiInterfaceAndroid.cs`.

---

## 0. TL;DR — the finding that changes the design

**The PC never builds CAN frames when talking D-CAN through a K+DCAN cable.**

EdiabasLib handles concept 0x0110 with the *same* transmit function as BMW-FAST
K-line (`ParTransmitFunc = TransBmwFast`), at **115200 baud, 8 data bits, no
parity**, over the plain FTDI serial link (`EdInterfaceObd.cs:828-845`).
**[CONFIRMED]**

The K+DCAN cable contains a microcontroller that transparently bridges:

```
PC serial 115200 8N1                 CAN bus 500 kbit/s, 11-bit IDs
BMW-FAST telegram        <──MCU──>   ISO-TP (ISO 15765-2), extended addressing
[hdr][tgt][src][data][cs]            TX id 0x6F1, RX id 0x600|ecu_addr
```

The ISO-TP state machine (segmentation, flow control, timing) runs **inside the
cable firmware**, not on the PC. **[LIKELY** for clone cables — this is the
biggest open risk, see §6.1 and §10**]**

Consequently `DCanTransport` v1 is *simpler* than DS2 in framing (no ISO-TP on
our side), but we still spec the full ISO-TP layer (§5) because:
1. you cannot debug the cable without knowing what it does on the bus,
2. the ELM327/STN fallback path (§6.2) and a future SocketCAN/J2534 driver do
   need PC-side ISO-TP,
3. if the clone's MCU turns out not to bridge (risk §6.1), ISO-TP-on-PC via
   ELM327-style adapter is the fallback.

Three adapter paths, in recommended implementation order:

| Path | Adapter | PC-side work | Status |
|------|---------|--------------|--------|
| **A** (§6.1, implement first) | K+DCAN FTDI cable (what the user owns) | BMW-FAST telegrams @ 115200, zero extra framing | serial side [CONFIRMED], clone bridging [UNVERIFIED] |
| **B** (§6.2, fallback) | ELM327 v1.4+/STN11xx | AT commands + PC-side ISO-TP over "protocol B" raw CAN | [CONFIRMED] from EdElmInterface.cs |
| **C** (§6.3, do not implement) | Deep-OBD custom firmware adapter | proprietary `0x00`-header CAN telegrams | [CONFIRMED] but requires reflashed adapter |

---

## 1. Where D-CAN sits

- EDIABAS concept **0x0110**, `Protocol::DCan` in `src/config.rs`. Largest SGBD
  family (672 SGBDs): E90/E60/E87 (from ~03/2007), all F-series K-line-less cars.
- Physical: **CAN 500 kbit/s** on OBD pins 6 (CAN-H) / 14 (CAN-L). 11-bit
  identifiers only. **[CONFIRMED]** (`EdCustomAdapterCommon.cs:442-467` accepts
  only 500000/100000; 100 kbit/s is the older PT-CAN diag variant, not needed for v1).
- Diagnostic protocol carried on top: UDS (ISO 14229) / KWP2000 services — the
  SGBD bytecode decides; the transport does not interpret service bytes except
  for the negative-response codes in §7.

### 1.1 What the VM hands to `exchange()` — read this before anything else

For 0x0110 SGBDs the telegram built by the bytecode and passed to `xsend` is a
**complete BMW-FAST telegram including the 3/4/6-byte header** (format byte,
target = ECU address, source = 0xF1) and **excluding the checksum**. The
transport appends the checksum. This is exactly how `TransKwp2000` consumes it:
it reads target/source/length out of `sendData` itself and appends the checksum
when `!ParChecksumByUser` (`EdInterfaceObd.cs:4119-4124`). **[CONFIRMED]**

So the contract matches our DS2 transport: `exchange(frame)` gets a
checksum-less telegram, returns the full response telegram minus its checksum
(EDIABAS result byte-positions index the full telegram, header included — same
convention as `Ds2Transport::receive`).

There is **no XOR checksum and no DS2 LEN heuristics** here — BMW-FAST framing
(§3) replaces all of that.

---

## 2. Serial link parameters (Path A)

| Parameter | Value | Source |
|-----------|-------|--------|
| Baud | **115200** — hardcoded per concept, NOT taken from CommParameter | `EdInterfaceObd.cs:834` **[CONFIRMED]** |
| Format | 8 data bits, **no parity**, 1 stop | `EdInterfaceObd.cs:835` **[CONFIRMED]** |
| RTS | held **false** (never toggled) | `EdInterfaceObd.cs:479,892` **[CONFIRMED]** |
| DTR | held **true** the whole session when the adapter echoes (plain cable default `AdapterEcho = true`, `EdInterfaceObd.cs:2782-2786`); never pulsed during TX in that case (`ParSendSetDtr = !HasAdapterEcho` → false, line 843) | **[CONFIRMED]** in EdiabasLib; whether the clone *cares* is **[UNVERIFIED]** |
| Inter-byte TX delay | **0** (`setInterByteTimeFunc(0)`, line 897-905) — do NOT apply the DS2 5 ms spacing | **[CONFIRMED]** |
| TX echo | **expected and verified**: after writing N bytes, read N bytes back and compare; mismatch = comm error (`EdInterfaceObd.cs:4137-4156`) | **[CONFIRMED]** for EDIABAS behaviour; clone echo behaviour in D-CAN mode **[UNVERIFIED]** — make it a flag like `Ds2Transport::echo`, default `true` |

DTR note: for echo-less adapters EDIABAS instead pulses DTR high for the exact
duration of the TX (`EdInterfaceObd.cs:3342-3356`, `EdFtdiInterfaceAndroid.cs:436-447`
— busy-waits `byte_time × len + DtrTimeCorr`, then drops DTR). We only need
this if the cable turns out to be echo-less; spec'd here so the knob exists.

---

## 3. BMW-FAST telegram format (the bytes on the serial link)

### 3.1 Layout — three forms, selected by payload length

`LEN` below = number of *data* bytes (the UDS/KWP service bytes).

**Short form** (1 ≤ LEN ≤ 0x3F):
```
[0x80|LEN] [TGT] [SRC] [DATA × LEN] [CS]
```

**Long form 1** (0x40 ≤ LEN ≤ 0xFF):
```
[0x80] [TGT] [SRC] [LEN8] [DATA × LEN] [CS]        ; LEN8 != 0
```

**Long form 2** (LEN > 0xFF, up to 0xFFFF):
```
[0x80] [TGT] [SRC] [0x00] [LENH] [LENL] [DATA × LEN] [CS]
```

- `TGT` — target address: ECU address for requests (e.g. 0x12 DDE, 0x40 CAS,
  0x60 KOMBI), **0xF1** in responses.
- `SRC` — source address: **0xF1** (tester) in requests, ECU address in responses.
- `CS` — **8-bit additive sum** (`sum of all preceding bytes, mod 256`) —
  `EdInterfaceBase.cs:933-941`. NOT XOR. **[CONFIRMED]**
- Format byte high bits: `0x80` = physical addressing. `(b0 & 0xC0) == 0xC0` =
  functional/broadcast telegram (see §3.4). A response whose
  `(b0 & 0xC0) != 0x80` is invalid (`EdInterfaceObd.cs:4171`). **[CONFIRMED]**

### 3.2 Telegram length algorithm (receiver side)

Faithful port of `TelLengthBmwFast` (`EdInterfaceBase.cs:881-905`,
returns length **without** checksum): **[CONFIRMED]**

```
fn tel_length(buf) -> usize:          # buf holds at least 4 bytes (6 for long form 2)
    len = buf[0] & 0x3F
    if len == 0:
        if buf[3] == 0:  return (buf[4] << 8) + buf[5] + 6    # long form 2
        else:            return buf[3] + 4                    # long form 1
    else:                return len + 3                       # short form
```

Data offset correspondingly: 3 (short), 4 (long 1), 6 (long 2)
(`DataLengthBmwFast`, `EdInterfaceBase.cs:907-931`).

### 3.3 Worked example — UDS ReadDataByIdentifier 0xF190 (VIN) to DDE (0x12)

VM hands `exchange()`:      `83 12 F1 22 F1 90`
Transport appends CS:       sum = 0x83+0x12+0xF1+0x22+0xF1+0x90 = 0x329 → `29`
Serial TX @115200:          `83 12 F1 22 F1 90 29`
Serial RX (echo):           `83 12 F1 22 F1 90 29`   (drain + verify)
Serial RX (response, e.g.): `94 F1 12 62 F1 90 <17 VIN bytes> CS`
`exchange()` returns:       `94 F1 12 62 F1 90 …` (checksum stripped, header kept)

### 3.4 Functional addressing

Functional telegrams have `(b0 & 0xC0) == 0xC0`; BMW uses functional target
0xEF **[LIKELY]**. EdiabasLib does not broadcast them on ISO-TP paths — it
*expands* a functional request into one physical request per known ECU address
(`EdElmInterface.cs:448-468`, header rewritten `b0 = (b0 & ~0xC0) | 0x80`,
`b1 = addr`). **[CONFIRMED]** For v1: reject functional telegrams with a clear
error; physical addressing covers all normal jobs.

---

## 4. `exchange()` algorithm — faithful port of `TransBmwFast`/`TransKwp2000`

Source: `EdInterfaceObd.cs:4110-4264`. **[CONFIRMED]** This runs identically for
BMW-FAST K-line (0x010F) and D-CAN (0x0110); only the timing constants differ.

```
fn exchange(telegram):                     # telegram = BMW-FAST telegram, no CS
    # -- send ------------------------------------------------------------
    wait until (now - last_response_time) >= regen_time      # ParRegenTime
    flush_rx()
    frame = telegram + [sum8(telegram)]
    write(frame)                                             # no inter-byte delay
    if echo:
        buf = read_exact(len(frame))       within echo_timeout
        if buf != frame: drain_rx(); return Err(EchoMismatch)
    nr78_pending = {}                      # set of ECU source addresses
    nr_retry_count = 0

    # -- receive loop (handles 0x7F..0x78 / ..0x21 / ..0x23) -------------
    loop:
        timeout = if nr78_pending.non_empty() { timeout_nr78 } else { timeout_std }

        head = read_exact(4)               first byte within `timeout`,
                                           rest within timeout_tel_end (=10 ms)
        if (head[0] & 0xC0) != 0x80: drain_rx(); return Err(BadHeader)
        if (head[0] & 0x3F) == 0 and head[3] == 0:
            head += read_exact(2)                            # long form 2
        total = tel_length(head)                             # §3.2
        rest  = read_exact(total - len(head) + 1)            # +1 = checksum,
                                                             # inter-byte timeout_tel_end
        tel = head + rest
        if sum8(tel[..total]) != tel[total]: drain_rx(); return Err(Checksum)

        (data, off) = data_and_offset(tel)                   # §3.2
        src = tel[2]                                         # responding ECU

        if len(data) == 3 and data[0] == 0x7F:
            nrc = data[2]
            if nrc == 0x21 or nrc == 0x23:                   # busyRepeatRequest /
                if nr_retry_count < retry_nr21_23:           # routineNotComplete
                    sleep(request_time_nr21_23); nr_retry_count += 1
                    goto send (retransmit whole frame)
                # retries exhausted → fall through, return the 7F telegram
            if nrc == 0x78:                                  # responsePending
                nr78_pending.add(src)                        # bounded: abort after
                continue                                     # retry_nr78 per ECU
        nr78_pending.remove(src)
        break                                                # final answer

    last_response_time = now
    return tel[..total]                                      # header kept, CS stripped
```

Key semantics **[CONFIRMED]**:
- **0x7F xx 0x78 (responsePending)**: do NOT retransmit. Keep reading with the
  longer `timeout_nr78` until a non-0x78 telegram arrives from that ECU. Each
  0x78 from the same ECU counts against `retry_nr78`; exceeding it aborts.
  Tracking is *per source address* (dictionary) because functional expansion can
  have several ECUs pending (`EdInterfaceObd.cs:4246-4258`).
- **0x7F xx 0x21 / 0x23**: sleep the configured request time, then *retransmit
  the request*, bounded by the retry counters (`EdInterfaceObd.cs:4218-4243`).
- End-of-telegram is length-driven (§3.2), not silence-driven — silence
  (`timeout_tel_end` = 10 ms) is only the inter-byte watchdog within one telegram.

---

## 5. The CAN layer — what happens on the bus

Needed for debugging Path A, and executed on the PC in Path B. All frame maths
below is what the cable firmware does (Path A) or what we do (Path B).

### 5.1 Addressing — BMW D-CAN, 11-bit, ISO-TP **extended addressing**

**[CONFIRMED]** (`EdElmInterface.cs:505,575` — `canHeader = 0x600 | sourceAddr`;
receiver at :733-734 takes the partner address from CAN-ID low byte and data[0]):

| Direction | CAN ID | data[0] (extended address byte) |
|-----------|--------|---------------------------------|
| tester → ECU | `0x600 \| 0xF1` = **0x6F1** | ECU address (0x12, 0x40, …) |
| ECU → tester | `0x600 \| ecu_addr` (DDE 0x12 → **0x612**) | 0xF1 |

- The BMW-FAST `TGT`/`SRC` bytes map 1:1: CAN-ID low byte = sender, data[0] = receiver.
- RX filter: accept `(id & 0x700) == 0x600` (EdiabasLib ELM init `ATCF600` +
  `ATCM700`, `EdElmInterface.cs:62-63`). **[CONFIRMED]**
- All frames padded to DLC 8 with 0x00 (EdiabasLib allocates zeroed 8-byte
  buffers, `EdElmInterface.cs:585,675,796`). **[CONFIRMED]**

Because the extended-address byte occupies data[0], every PCI capacity is one
byte smaller than "textbook" ISO-TP: **SF carries ≤ 6, FF carries 5, CF carries 6.**

### 5.2 PCI layouts (data[1] = PCI byte; data[0] = address byte)

```
SF  Single Frame       data[1] = 0x0N            N = payload len, 1..6
                       data[2..2+N] = payload

FF  First Frame        data[1] = 0x10 | (LEN>>8) LEN = total payload len, 12-bit
                       data[2] = LEN & 0xFF      (7..4095)
                       data[3..8] = first 5 payload bytes

CF  Consecutive Frame  data[1] = 0x20 | SN       SN = sequence number mod 16,
                       data[2..8] = next ≤6 bytes    first CF after FF has SN=1

FC  Flow Control       data[1] = 0x30 | FS       FS: 0=CTS  1=WAIT  2=OVERFLOW
                       data[2] = BS              block size, 0 = "send all, no more FC"
                       data[3] = STmin           separation time, see below
```

STmin encoding (ISO 15765-2): `0x00-0x7F` = 0-127 ms; `0xF1-0xF9` = 100-900 µs;
other values reserved → treat as 127 ms. BMW tester side sends **BS=0,
STmin=0** (`EdElmInterface.cs:521-522, 799-800`: `Elm327CanBlockSize = 3` for
the ELM RX path, `blockSize 0x00 / sepTime 0x00` in the FC it *configures* for
the ECU). **[CONFIRMED]**

### 5.3 Send-long-message algorithm (tester → ECU)

Port of `Elm327CanSender` (`EdElmInterface.cs:542-707`). **[CONFIRMED]**

```
fn isotp_send(ecu_addr, payload):
    if len(payload) <= 6:
        tx(0x6F1, [ecu_addr, 0x00|len(payload)] + payload)          # SF, pad to 8
        return

    tx(0x6F1, [ecu_addr, 0x10|(len>>8), len&0xFF] + payload[0..5])  # FF
    sent = 5; sn = 1; wait_fc = true; bs = 0; stmin = 0

    while sent < len(payload):
        if wait_fc:
            loop:                                                    # N_Bs timeout
                f = rx_match(id == 0x600|ecu_addr, f.data[0] == 0xF1,
                             (f.data[1] & 0xF0) == 0x30)  within N_Bs (1000 ms)
                fs = f.data[1] & 0x0F
                if fs == 0: bs = f.data[2]; stmin = f.data[3]; break # CTS
                if fs == 1: continue                                 # WAIT → keep waiting
                else:       return Err(FlowControl)                  # OVERFLOW/invalid
        wait_fc = false
        if bs > 0:
            if bs == 1: wait_fc = true                               # block exhausted
            bs -= 1
        chunk = payload[sent .. sent+min(6, remaining)]
        tx(0x6F1, [ecu_addr, 0x20|(sn & 0x0F)] + chunk)              # CF
        sent += len(chunk); sn += 1
        if sent < len(payload) and not wait_fc:
            sleep(decode_stmin(stmin))                               # ≥ STmin gap
```

### 5.4 Receive-long-message algorithm (ECU → tester)

Port of `Elm327CanReceiver` (`EdElmInterface.cs:709-929`). **[CONFIRMED]**

```
fn isotp_receive(ecu_addr) -> payload:
    f = rx_match(id == 0x600|ecu_addr, f.data[0] == 0xF1)  within N_Ar/timeout_std
    match f.data[1] >> 4:
        0x0:  n = f.data[1] & 0x0F                                   # SF
              return f.data[2 .. 2+n]
        0x1:  total = ((f.data[1] & 0x0F) << 8) | f.data[2]          # FF
              buf = f.data[3..8]        # 5 bytes
              tx(0x6F1, [ecu_addr, 0x30, BS, STmin])                 # FC CTS
              fc_countdown = BS                                      # our BS=3, STmin=0
              sn = 1
              while len(buf) < total:                                # N_Cr per CF
                  f = rx_match(id, data[0]==0xF1, data[1]>>4 == 0x2) within N_Cr
                  if (f.data[1] & 0x0F) != (sn & 0x0F): return Err(Sequence)
                  buf += f.data[2 .. 2+min(6, total-len(buf))]
                  sn += 1
                  if BS > 0 and len(buf) < total:
                      fc_countdown -= 1
                      if fc_countdown == 0:
                          tx(0x6F1, [ecu_addr, 0x30, BS, STmin]); fc_countdown = BS
              return buf
        else: ignore frame, keep reading                             # stray CF/FC
```

The reassembled `payload` is then wrapped back into a BMW-FAST telegram
(short/long form chosen by length, `TGT=0xF1`, `SRC=ecu_addr`, sum checksum) —
exactly what `EdElmInterface.cs:888-926` does — so that upper layers see the
same bytes regardless of adapter path. **[CONFIRMED]**

### 5.5 CAN-layer timing (ISO 15765-2 names, values EdiabasLib uses)

| Timer | Meaning | Value |
|-------|---------|-------|
| N_Bs | wait for FC after FF/CF-block | ISO: 1000 ms; EdiabasLib uses 2000 ms data timeout (`Elm327DataTimeout`, `EdElmInterface.cs:90`) |
| N_Cr | wait for next CF | ISO: 150 ms (safe: 250 ms); EdiabasLib folds it into the same 2000 ms |
| STmin (we send in FC) | min CF gap we ask of ECU | 0 (`Elm327CanSepTime = 0`, line 94) |
| BS (we send in FC) | CFs per FC block we ask of ECU | 3 for ELM path (line 93); 0 (= unlimited) is what the FC-config for genuine ISO-TP uses (`EdElmInterface.cs:521`) |
| CF pacing (we transmit) | gap between our CFs when ECU asks STmin=0 | EdiabasLib sleeps `max(STmin, 50 ms)` in the ELM path (line 698) — ELM-latency artifact, real CAN needs only STmin |

---

## 6. Adapter paths

### 6.1 Path A — K+DCAN FTDI cable (implement first)

**Serial contract**: everything in §2-§4 — that is the whole implementation.
There are **no mode-entry command bytes, no bitrate command, no filter command
on the serial link in EdiabasLib for this adapter** — the concept-0x0110 branch
only reconfigures the UART to 115200 8N1 and sets DTR/RTS
(`EdInterfaceObd.cs:828-906`). **[CONFIRMED]**

How the cable knows to use D-CAN rather than K-line — evidence & hypotheses:

- **[CONFIRMED]** EdiabasLib sends nothing protocol-specific to select the mode.
  Whatever switching exists is inside the cable (and in the hardware "K-line /
  D-CAN" slide switch some cables have, which reroutes OBD pins 7/8, not CAN).
- **[LIKELY]** The MCU keys off the UART baud rate and/or telegram format:
  9600-10400 → K-line pass-through; 115200 BMW-FAST telegrams → D-CAN bridge.
  Consistent with the custom-adapter firmware doing exactly this
  (`EdCustomAdapterCommon.cs:966` — `RawMode || CurrentBaudRate == 115200` →
  "BMW-FAST" branch, telegram forwarded raw).
- **[UNVERIFIED]** Some clones ship in "K-line only" state and need the vendor
  "DCAN utility" once (persists a flag in the FT232 EEPROM / MCU); others
  auto-switch. Community reports both. Our clone works with INPA on a DS2 car —
  says nothing about its D-CAN path.
- **[UNVERIFIED]** Whether the clone echoes TX bytes in D-CAN mode (its MCU sits
  between FTDI and the bus, unlike the passive K-line loop). Keep `echo`
  configurable; auto-detect is cheap: after first TX, if the first bytes read
  back equal the TX, treat as echo.

**Failure signature if the clone does not bridge**: TX succeeds, echo (if any)
returns, then silence — indistinguishable from "ECU absent". That is why the
test plan (§10) starts with a known-alive ECU and an ELM327 cross-check.

### 6.2 Path B — ELM327 v1.4+ / STN11xx (fallback, PC-side ISO-TP)

EdiabasLib drives a *genuine* ELM327 ≥ v1.4 in raw-CAN "protocol B" and runs the
§5 state machine itself. **[CONFIRMED]** (`EdElmInterface.cs:56-85`).

One-time init (each command answered `OK`):

```
ATD  ATE0  ATSH6F1  ATCF600  ATCM700  ATPBC001  ATSPB
ATAT0  ATSTFF  ATAL  ATH1  ATS0  ATL0  [ATCSM0]  [ATCTM5]
```

`ATPBC001` = protocol-B options 0xC0 (11-bit, 500 kbit divisor 01); `ATSPB` =
select protocol B; `ATCF600/ATCM700` = accept 0x600-0x6FF; `ATH1 ATS0 ATL0` =
headers on, no spaces/linefeeds. Frames are then sent as hex lines
(`12 03 22 F1 90` style after `ATSH6F1`) and received as `612 F1 03 62 …` lines;
the §5.3/5.4 algorithms run on top. ELM327's *own* ISO-TP cannot do BMW extended
addressing reliably below v1.4 — that is why EdiabasLib manages SF/FF/CF/FC
manually. If the ELM supports it, the "full transport" variant instead uses
`ATFCSH6F1 / ATFCSD <tgt>300000 / ATCEA<tgt> / ATFCSM1` to delegate flow control
(`EdElmInterface.cs:505-538`). **[CONFIRMED]**

This path needs a different physical adapter than the user's KDCAN cable but has
zero unknowns; implement it if (and only if) Path A fails on hardware.

### 6.3 Path C — Deep-OBD custom-firmware adapter framing (documented, skip)

Only exists after reflashing an adapter with EdiabasLib's firmware. For
completeness, the serial packet that carries one CAN-bound message
(`CreateCanTelegram`, `EdCustomAdapterCommon.cs:396-499`, telegram type 0x03):

```
[0x00] [0x03] [PROT] [BAUD] [FLAGS] [BS] [ST] [TXIDH] [TXIDL] [RXIDH] [RXIDL] [LENH] [LENL] [DATA…] [CS=sum8]
 PROT:  0x00 BMW  0x01 TP2.0  0x02 ISO-TP
 BAUD:  0x01 = 500 kbit   0x09 = 100 kbit
 FLAGS: 0x01 NO_ECHO  0x02 CAN_ERROR  0x04 CONNECT_CHECK  0x08 DISCONNECT
```

In BMW D-CAN mode the custom adapter does NOT use this — it takes raw BMW-FAST
telegrams at 115200 exactly like Path A and echoes them
(`EdCustomAdapterCommon.cs:966-1016`), reinforcing that "BMW-FAST @115200,
conversion in firmware" is *the* BMW adapter convention. **[CONFIRMED]**

---

## 7. Timing & error handling (transport level)

EDIABAS CommParameter is an array of 32-bit values; indices below are array
indices as EdiabasLib consumes them for concept 0x0110
(`EdInterfaceObd.cs:828-845`, block must have ≥ 30 elements). **[CONFIRMED]**

| Name | CommParameter index | Meaning | Sane default |
|------|--------------------:|---------|--------------|
| `timeout_std` | [7] | first-byte wait for a response telegram | 2000 ms |
| `regen_time` | [8] | min idle between last response and next request | 20 ms |
| `timeout_nr78` | [9] | first-byte wait while a 0x7F..0x78 is pending | 5000 ms |
| `retry_nr78` | [10] | max consecutive 0x78 per ECU before abort | 5 |
| `timeout_tel_end` | — (hardcoded) | inter-byte / telegram-end watchdog | **10 ms** (line 839) |
| baud | — (hardcoded) | serial baud | 115200 |

NR21/NR23 request-time/retry parameters exist for other concepts
(`ParRequestTimeNr21/23`, `ParRetryNr21/23`); for 0x0110 they stay at their
defaults — implement the §4 handling with configurable values, defaulting to
"return the 0x7F telegram to the VM" when unset (EDIABAS gives the job the
negative response and lets SGBD logic decide). **[CONFIRMED]** in structure,
default values **[LIKELY]**.

Open item **[UNVERIFIED]**: whether our `.prg` parser sees the D-CAN
CommParameter block as u16-packed (like the 18-byte DS2 block `CommConfig::parse`
handles) or u32-packed. Dump the `xsetpar` block of one 0x0110 SGBD (e.g.
`MSD80.prg`) before extending `CommConfig::parse`; the concept word at [0] and
the ≥30-element count make the width unambiguous in a hex dump.

Error mapping (all drain RX before returning, like DS2):

| Condition | Error |
|-----------|-------|
| no first byte within timeout | Timeout ("ECU silent") |
| echo mismatch | Protocol("D-CAN: echo mismatch — adapter not in D-CAN mode?") |
| `(b0 & 0xC0) != 0x80` | Protocol("bad BMW-FAST header") |
| sum checksum wrong | Checksum { expected, got } |
| NR78 retries exhausted | Timeout after `retry_nr78 × timeout_nr78` |

---

## 8. Mapping to our code

### `configure(&CommConfig)`
- Store cfg; require `cfg.concept == 0x0110`.
- Timing fields: `timeout_std_ms`, `regen_time_ms` map directly; add
  `timeout_nr78 / retry_nr78` fields (new — CommConfig extension or
  DCanTransport fields with defaults from §7).
- **Do not** honor `cfg.interbyte_ms` (must be 0 here) or `len_offset/len_add`
  (meaningless — BMW-FAST length is structural).
- Serial params: 115200 / 8N1 / no parity. **Gap**: the `Driver` trait has no
  baud/parity setter (`SerialDriver` fixes them at `open_parity`). Options:
  (a) `Session` opens the port at 115200-no-parity when
  `cfg.protocol.is_can()` — works because concept is known from the .prg before
  the port opens; (b) add `set_line(baud, parity)` to `Driver`. Do (a) first,
  (b) when live baud switching (KWP fast-init at 10400 → 115200) is needed anyway.
- `driver.set_timeout(cfg.timeout_std_ms)`.

### `init_connection()`
- No wire handshake exists (no 5-baud, no fast-init, no wake): like DS2,
  `EcuConnected` is immediate (`EdInterfaceObd.cs` sets no start-comm telegram
  for 0x0110). Set DTR=true, RTS=false, `flush_rx()`, return Ok.
- ECU presence is proven only by the first real job — same GUI rule as DS2
  (run INITIALISIERUNG / IDENT after connect).

### `exchange(frame)`
- Exactly §4: regen wait → flush → append sum8 → write (no inter-byte delay) →
  echo drain+verify (flag, default true) → BMW-FAST length-driven receive with
  NR78/NR21/23 loop → return telegram minus checksum.
- `trace!` TX/RX like DS2 — indispensable for the §10 bring-up.

### `disconnect()`
- Nothing on the wire. Optionally DTR=false.

### Driver primitives used
`write`, `read_exact` (header/echo — exact counts are known), `read_some`
(only for drain-on-error), `set_timeout` (switch between `timeout_std`,
`timeout_nr78`, `timeout_tel_end` per §4), `set_dtr`, `set_rts`, `flush_rx`.
`set_break` is never used (no K-line init on CAN).

---

## 9. What ISO-TP code do we write now?

For Path A: **none** — but implement §5 as a pure, driver-independent module
(`isotp.rs`: `encode_frames(addr, payload) -> Vec<CanFrame>` +
`Reassembler::push(CanFrame) -> Option<Vec<u8>>` + FC logic) when Path B or a
SocketCAN/J2534 driver lands. The §5 pseudocode is complete enough to implement
and unit-test against the worked examples:

```
payload 22 F1 90  → 0x6F1: 12 03 22 F1 90 00 00 00                       (SF)
payload 2E F1 90 + 16 bytes (19 total) →
  0x6F1: 12 10 13 2E F1 90 xx xx                                          (FF)
  0x612: F1 30 00 00 00 00 00 00                                          (FC CTS)
  0x6F1: 12 21 xx xx xx xx xx xx                                          (CF SN=1)
  0x6F1: 12 22 xx xx xx xx xx xx                                          (CF SN=2)
  0x6F1: 12 23 xx xx 00 00 00 00                                          (CF SN=3)
```

---

## 10. Live-hardware verification plan (D-CAN car + our KDCAN clone)

Goal: falsify/confirm the **[UNVERIFIED]** items with minimal code — extend the
`raw` CLI command with `--dcan` (115200, no parity, sum8 checksum, BMW-FAST
receive) before touching the VM path.

1. **Known-alive probe** — UDS session to a mandatory ECU, engine off, ign on:
   `./ediabas-prg raw --dcan --port COMx "83 12 F1 22 F1 90"`  (DDE VIN) and
   `"83 40 F1 22 F1 90"` (CAS). Expect `9x F1 <addr> 62 F1 90 …`.
2. **Echo behaviour**: run once with echo drain on; if the first 7 RX bytes are
   not the TX frame, rerun with echo off → sets the `echo` default for D-CAN.
3. If silence: repeat with DTR low, then with DTR pulsed-during-TX (§2 note) —
   three DTR strategies × echo on/off is the whole search space.
4. Still silent → clone likely lacks the D-CAN bridge: verify the car side with
   any ELM327 (`ATSPB` sequence from §6.2, send `12 03 22 F1 90`); if the ELM
   gets `62 F1 90 …`, the car is fine and the cable is the problem → implement
   Path B / try the vendor "DCAN utility" on the cable.
5. **Long-message check** (exercises the cable's ISO-TP RX): `19 02 0C`
   (ReadDTCInformation) or `22 F1 88` — any multi-frame response; verify the
   BMW-FAST long-form length parsing (§3.2) against it.
6. Record CommParameter dump from a real 0x0110 SGBD (§7 open item).

---

## 11. Source & confidence summary

| Area | Confidence | Source |
|------|-----------|--------|
| 0x0110 = BMW-FAST telegrams @115200 8N1 over the serial link, no PC-side CAN framing | CONFIRMED | `EdInterfaceObd.cs:828-845` |
| BMW-FAST telegram forms, length algo, sum8 checksum | CONFIRMED | `EdInterfaceBase.cs:881-941` |
| exchange loop, echo verify, NR78/21/23 handling, 10 ms tel-end | CONFIRMED | `EdInterfaceObd.cs:4110-4264` |
| CAN IDs 0x6F1 / 0x600\|addr, extended addressing, SF≤6/FF=5/CF=6, FC fields, filter 600/700 | CONFIRMED | `EdElmInterface.cs:56-94, 440-929` |
| ISO-TP PCI encodings, STmin semantics | CONFIRMED | ISO 15765-2 (matches EdElmInterface byte-for-byte) |
| DTR/RTS states, DTR-pulse variant | CONFIRMED (EdiabasLib) / UNVERIFIED (clone cares?) | `EdInterfaceObd.cs:478-479, 836, 843, 3315-3379`; `EdFtdiInterfaceAndroid.cs:406-467` |
| Clone cable auto-bridges UART↔ISO-TP in D-CAN mode | LIKELY mechanism, UNVERIFIED for our unit | inference from `EdCustomAdapterCommon.cs:966-1016` + community reports; **biggest open risk** |
| ELM327 fallback init + manual ISO-TP | CONFIRMED | `EdElmInterface.cs:56-85, 505-929` |
| Custom-adapter CAN packet (Path C) | CONFIRMED (framing) | `EdCustomAdapterCommon.cs:30-47, 396-499` |
| D-CAN CommParameter width (u16 vs u32) in our .prg pipeline | UNVERIFIED | dump needed (§7) |

References:
- EdiabasLib — https://github.com/uholeschak/ediabaslib (files under
  `EdiabasLib/EdiabasLib/`, line numbers per master @ 2026-07-03)
- ISO 15765-2:2016 — Road vehicles, diagnostic communication over CAN, transport layer
- ELM327 datasheet v1.4b (AT command semantics for §6.2)
