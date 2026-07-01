# Bosch EDC15C — Communication / Diagnostic Protocol Reference

Extracted and translated from the official Bosch **Funktionsbeschreibung** (function
description) for the **EDC15C** diesel ECU (`Y 445 S00 003-CC0 / EDC15C B079.CC0`,
release 05.01.01). Source chapter **10 "Externe Kommunikation (K-Leitung)"** plus the
fault-memory/MIL sections. All page citations refer to the printed page numbers shown in
the document footers (e.g. `10-60`) and, where useful, the extractor's `PAGE N` markers.

> **Scope / applicability.** This is a **Bosch diesel** ECU implemented for **BMW**
> (labels `xcc…`, EWS immobilizer, BMW part numbers). The very ECU family described here is
> the **DDE4.0 / DDE4.1** — the project's target. The protocol is BMW's **"Keyword
> Protocol 2000"** in two flavours:
> 1. a **DS2-compatible mode** (runs at **9600 Bd**, so it can share a bus with older DS2
>    ECUs) and
> 2. a **Standard KWP2000 mode** (runs at **10400 Bd**, with 5-baud or fast init).
>
> This is *KWP/K-Line*, not the classic BMW DS2 byte protocol. See
> **[§12 Differences vs. classic BMW DS2](#12-differences-vs-classic-bmw-ds2)** — in
> particular the checksum here is documented as an **additive sum**, whereas the project's
> current `ds2.rs` assumes an **XOR** checksum.

---

## 1. Physical layer (K-Leitung)

- The **K-Line** is a **digital single-wire interface** (*digitale Eindrahtschnittstelle*)
  at **battery voltage level** (`UBatt`). Transmission is **asynchronous**, similar to the
  V.24 standard. ECU (SG) and tester (TG) **never transmit simultaneously** (half-duplex).
  (p. 10-69 / PAGE 435)
- **Mode state machine** `xcm_mode` (Data Link Layer state, p. 10-1 / PAGE 367):

  | `xcm_mode` | Meaning |
  |-----------|---------|
  | `10` | Protocol-checking mode (DS2-compatible) |
  | `11` | Static KWP2000\* (DS2-compatible mode, **9600 Bd**) |
  | `12` | Standard KWP2000 mode (**10400 Bd**) |

- Automatic mode switching can be disabled via label `cowVAR_KWP`
  (`0` = auto KWP2000/2000\* detection, `1` = KWP2000\* only). (p. 10-2 / PAGE 368)

### Byte framing

| Context | Framing |
|--------|---------|
| **Block transfer**, Standard KWP2000 (p. 10-63 / PAGE 429) | 1 start bit, **8 data bits LSB-first, 1 stop bit** (no parity) |
| **5-baud init address word** (p. 10-61 / PAGE 427) | 1 start bit (logic 0), **7 data bits LSB-first**, 1 **parity** bit, 1 stop bit (logic 1) |
| **Sync pattern `55h`** (p. 10-62 / PAGE 428) | 8 data bits, **no parity** |
| **Keywords** (p. 10-62 / PAGE 428) | **7 data bits, odd parity** |
| **McMess** (p. 10-69 / PAGE 435) | **9 data bits** + start + stop bit |

- **Baud rates:** DS2-compatible mode **9600 Bd (9,6 kBaud)**; after any 5-baud/fast init
  the link is fixed at **10400 Baud** (p. 10-61 / PAGE 427). McMess supports **10,4 kBaud**
  (PC-compatible) or **38,4 kBaud** (fast EDC mode) (p. 10-71 / PAGE 437).
- Data flow follows **ISO 9141** (fig. XCKW02 "Datenablauf nach ISO 9141", p. 10-62).
- The `StartDiagnosticSession` service can switch baud via a *BaudrateIdentifier*:
  `01 = 9,6 kBaud`, `02 = 19,2 kBaud`, `03 = 38,4 kBaud`, `85 = 125 kBaud` (p. 10-6 / PAGE 372).

---

## 2. Initialization / wake-up

### 2.1 5-baud initialization (*Initialisierung mit 5 Baud*) — p. 10-60…10-63

The tester (**TG**) transmits a 7-bit **address word at 5 baud** on the K-Line
(*Reizung*, "stimulation"):

- **Functional addresses** (address a whole system):
  - `33h` — OBD-II emissions-relevant system (SAE J1979) — **even parity**
  - `7Dh` — Fahrberechtigungssystem (immobilizer / EWS) — even parity
- **Physical addresses** (address one ECU): `xxh` per Table 10-1 — **odd parity**
  (e.g. `12h` for DDE4.0)

The ECU aborts the *Reizung* if the start/data bits are disturbed, the address is unknown,
or no valid stop bit is seen; it then re-arms for stimulation after time `T0`
(p. 10-62 / PAGE 428). Subsequent communication is fixed at **10400 Baud**.

**Connection setup (Kommunikationsaufbau) after successful stimulation** (fig. XCKW02):

1. ECU → TG: **sync pattern `55h`** (8 data bits, no parity)
2. ECU → TG: **Keyword 1 = `6Bh`**, **Keyword 2 = `8Fh`** (7 data bits, odd parity)
   (per "KWP 2000 Spec./II, table 2.5.1.2.1.2")
3. TG → ECU: **logical inversion of Keyword 2**
4. ECU → TG: **logical inversion of the init address**

```
Init(5bd)   55h        KW1  KW2         ~KW2            ~InitAddr
  TG───►    ◄──SG   ◄──SG──SG    ──TG──►          ◄──SG
            sync    keywords     inv KW2 (TG)     inv addr (SG)
   T0   T1        T2   T3     T4              T4
```
(fig. 10-4 XCKW02, p. 10-62)

### 2.2 Fast init / Wake-up pattern (*Schneller Einstieg*) — p. 10-64…10-65

To shorten setup the TG can send a **Wake-up-Pattern**:

- Pull K-Line low for `TiniL` (24–26 ms), pattern duration `TWuP` (49–51 ms) (p. 10-66).
- Then TG sends a **StartCommunication request** with **Mode byte `81h`**.
- ECU replies with **Mode byte `C1h`** plus the two **keywords** (`KW1`,`KW2`).
- The subsequent communication flow is identical to 5-baud init **using physical address
  `12h`** (p. 10-65 / PAGE 431).

```
Wake-up   Req: [Typ][Tgt][Src][M=81][CS]                 ← from TG
Resp:     [Typ][Tgt][Src][Lä][M=C1][KW1][KW2][CS]        ← from SG
```
(fig. 10-6 XCKW04, p. 10-65)

---

## 3. Telegram / block format (*Blockaufbau*, fig. 10-5 XCKW03, p. 10-64)

A block has three parts:

```
┌── Kopfteil (header) ──┬──────── Informationsteil ────────┬─ Prüfteil ─┐
  Format  Target  Source   [Länge]  Mode/SID  Data1..DataN     CS
   (Typ)   (Tgt)   (Src)   (opt.)     (M)                     (checksum)
```

- **Format byte (Typ)** — top two bits `A1 A0` select the addressing type; the lower
  6 bits are a **length field `L` (1…63)**:

  | `A1 A0` | Meaning |
  |--------|---------|
  | `0 0` | not allowed (header without address info) |
  | `0 1` | emissions-relevant system (SAE J1979) |
  | `1 0` | header — **physical** addressing |
  | `1 1` | header — **functional** addressing |

  If `L = 0`, a **separate Length byte** follows the Source byte and encodes length
  **1…255**. Otherwise `L` (1…63) is the info-part length. Max info part = 256 bytes
  (length + 255 data bytes). (p. 10-63/10-64)

- **Target / Source** — for physical/functional addressing: Target = receiver address,
  Source = sender address. For the emissions system (init `33h`) the first header byte
  carries **no length**; length is implied by Mode/PID, max **11 bytes** incl. header and
  1-byte checksum (Typ `68h`→SG / `48h`←SG, Target `6Ah`/`6Bh`, Source `Fxh`/ECU-addr)
  (p. 10-63 / PAGE 429).

- **Mode byte** = KWP2000 **Service Identifier (SID)**; optional PID follows.

- **Checksum (CS)** — "Prüfsumme in Hex-Code, wobei **CS = LOW-Byte der Prüfsumme**"
  (p. 10-63). McMess states this explicitly: **`CS = Byte1 + Byte2 + Byte3 + …`**, i.e. the
  **low byte of the arithmetic sum** of all preceding bytes (p. 10-69 / PAGE 435). ⚠️ See §12.

### 3.1 Concrete header used by the BMW DS2-compatible services

Every DS2-compatible service in §10.1.2 uses a **fixed** header layout (p. 10-6 onward):

| Field | Request | Positive response |
|------|---------|-------------------|
| Format Byte | `B8` | `B8` |
| Target Byte | ECU addr (Tab 10-1, e.g. `12`) | `F1` (tester) |
| Source Byte | `F1` (tester) | ECU addr (Tab 10-1) |
| Length Byte | info-part length (mode+data) | info-part length |
| Service ID / Mode | request SID | `SID + 0x40` |
| … data … | | |
| Checksum | `CS` | `CS` |

- **`TesterPresent`** uses **Format `80`** instead of `B8` (p. 10-67 / PAGE 433).
- Note: the DS2-compatible services carry an **explicit Length byte** even though Format is
  the constant `B8`, so in this BMW usage `B8` behaves as a fixed magic header value rather
  than the generic length-encoded format byte of §3. `80` (physical addressing, `L=0` →
  length byte follows) is consistent with the generic scheme.

---

## 4. Addressing (Table 10-1, p. 10-5 / PAGE 371)

| Party | Address |
|------|---------|
| Tester (TG) source | `F1h` |
| **DDE4.0** (physical) | `12h` |
| **DDE4.1 Master** (physical) | `12h` |
| **DDE4.1 Slave** (physical) | `13h` |
| Functional — OBD-II emissions (SAE J1979) | `33h` |
| Functional — immobilizer (Fahrberechtigung) | `7Dh` |
| McMess target | `08h` |

---

## 5. Timing parameters (*Zeitdefinitionen*, p. 10-66 / PAGE 432)

Values in parentheses apply to the emissions system (init `33h`, 5 baud).

| Symbol | Range | Meaning |
|-------|-------|---------|
| `T0` | > 300 ms | Logic "1" idle time before initialization |
| `TiniL` | 24–26 ms | Logic "0" time at fast-init |
| `TWuP` | 49–51 ms | Wake-up-pattern duration (fast init) |
| `T1` | 60–300 ms | End of init → start of sync pattern |
| `T2` | 5–20 ms | End of sync → first keyword |
| `T3` | 0–20 ms | End of KW1 → start of KW2 |
| `T4` | 25–50 ms | KW2 → inverted-KW2, and inverted-KW2 → inverted init address |
| `T5` | > 300 ms | Tester restart after detected init error |
| `P1` | 0–20 ms | Inter-byte time, **ECU → tester** blocks |
| `P2` | 0–50 ms (25 ms) | End of tester block → start of ECU block (request→response) |
| `P3` | 55 ms – 5 s | End of last ECU block → start of new tester block (**keepalive window**) |
| `P4` | 5–20 ms | Inter-byte time, **tester → ECU** blocks |
| `P5` | 5–20 ms | Inter-byte time for the "Diagnose-Start" (Mode 81) request on fast init |
| `P6` | 5 ms (25 ms) – `P2max` | Time between successive ECU→TG blocks |

- **Error handling:** an init timeout (esp. `T4`) makes the ECU re-arm for the stimulation
  address within `T5min`. On a **bad checksum** the ECU waits for a fresh, correct request.
  On a **malformed request structure** it returns an **acknowledge/negative response with
  code `12h`** ("unverständliche Anforderung"). Exceeding `P3max` **ends communication**.
  (p. 10-67 / PAGE 433)

---

## 6. TesterPresent / keepalive (p. 10-67 / PAGE 433)

To keep an idle connection alive (no data), the tester must send acknowledge `3E`
**within `P3`**; the ECU answers `7E`.

| | Format | Target | Source | Len | SID | CS |
|-|-------|--------|--------|-----|-----|----|
| Request | `80` | Tab 10-1 | `F1` | `01` | `3E` | `??` |
| Positive resp | `80` | `F1` | Tab 10-1 | `01` | `7E` | `??` |
| Negative resp | `80` | `F1` | Tab 10-1 | `03` | `7F 3E RC` | `??` |

---

## 7. Implemented services (DS2-compatible mode) — Table pp. 10-4/10-5 (PAGES 370–371)

`SID` = Service Identifier (Mode byte). Positive response SID = `SID + 0x40`
(e.g. `10→50`, `1A→5A`, `21→61`, `2C→6C`, `30→70`, `31→71`). All services are supported by
DDE4.0 and DDE4.1.

| Page | Description | KWP2000 service | SID | Sub |
|-----:|-------------|-----------------|-----|-----|
| 10-6  | Change diagnostic session | StartDiagnosticSession | `10` | mode/baud (`85`=flash) |
| 10-7  | Clear fault memory | ClearDiagnosticInformation | `14` | group HB/LB |
| 10-8  | Read fault memory (long) | ReadStatusOfDTC | `17` | DTC HB/LB |
| 10-10 | Read fault memory (short) | ReadDTCByStatus | `18` | `00 00 00` |
| 10-11 | Read ECU identification | ReadEcuIdentification | `1A` | `80` |
| 10-13 | Read system-specific addresses | ReadDataByLocalIdentifier | `21` | `01` |
| 10-15 | EWS receive status | ReadDataByLocalIdentifier | `21` | `06` |
| 10-16 | Read measured values (static) | ReadDataByLocalIdentifier | `21` | `20`–`2F` (block 0–15) |
| 10-18 | Read inspection stamp | ReadDataByCommonIdentifier | `22` | `1000` |
| 10-19 | Read shadow fault memory | ReadDataByCommonIdentifier | `22` | `2000` |
| 10-20 | Read central code | ReadDataByCommonIdentifier | `22` | `4000` |
| 10-31 | Read memory by address | ReadMemoryByAddress | `23` | — |
| 10-23 | Security access — request SEED | SecurityAccess#2 | `27` | `02` |
| 10-24 | Security access — get SEED | SecurityAccess#1 | `27` | `01` |
| 10-25 | Security access — send KEY | SecurityAccess#2 | `27` | `04` |
| 10-25 | Read measured values (dynamic) | DynamicallyDefinedLocalIdentifier | `2C` | `10` + PIDs |
| 10-27 | Write inspection stamp | WriteDataByCommonIdentifier | `2E` | — |
| 10-28 | Write central code | WriteDataByCommonIdentifier | `2E` | `4000` |
| 10-30 | Read/set/program adjustment values | InputOutputControlByLocalIdentifier | `30` | `A1`–`AD` |
| 10-33 | KLI / MFL_FGR info read/clear | InputOutputControlByLocalIdentifier | `30` | `A9` |
| 10-41 | **Actuator drive (Stellglied)** | InputOutputControlByLocalIdentifier | `30` | `C1`–`CC` |
| 10-43 | Actuator release | InputOutputControlByLocalIdentifier | `30` | `C1`–`CC` + `00` |
| 10-44 | Read programming status | InputOutputControlByLocalIdentifier | `30` | `EB 01` |
| 10-45 | Diagnostic routine start | StartRoutineByLocalIdentifier | `31` | `40` |
| 10-49 | Read coding checksum | StartRoutineByLocalIdentifier | `31` | `01` |
| 10-50 | Erase memory | StartRoutineByLocalIdentifier | `31` | `02` |
| 10-51 | EWS start-value init | StartRoutineByLocalIdentifier | `31` | `83` |
| 10-52 | Diagnostic routine stop | StopRoutineByLocalIdentifier | `32` | `40` |
| 10-53 | Request download | RequestDownload | `34` | — |
| 10-54 | Transfer data | TransferData | `36` | — |
| 10-55 | Request transfer exit | RequestTransferExit | `37` | — |
| 10-56 | Stop communication | StopCommunication | `82` | — |
| 10-57 | Access timing parameter | AccessTimingParameter | `83` | — |
| 10-59 | Read DS2 ECU identification | ReadDS2EcuIdentification | `A2` | — (pos. resp `E2`) |
| 10-67 | **TesterPresent** (KWP2000 mode only) | TesterPresent | `3E` | — (pos. resp `7E`) |

### 7.1 Selected request/response structures (templates as printed; `??` = checksum not given numerically)

**ReadEcuIdentification (`1A 80`)** — p. 10-11/10-12. Positive response `5A`, Length `2C`,
returns BMW part number (7 ASCII), hardware number, coding/diagnostic/bus index, mfg
week/year, Rover HW number, software index, change index, individual ECU number.
```
Req:  B8 <TGT> F1 02 1A 80 <CS>
Resp: B8 F1 <SRC> 2C 5A 80 <44 ASCII/data bytes...> <CS>
```

**ReadDataByLocalIdentifier — measured values static (`21`, block `20`–`2F`)** — p. 10-15.
Each of the 16 blocks holds up to 10 freely-configurable measured values (labels
`xcwD20_E1 … xcwD2F_E10`). If an undefined message number is hit, the response is truncated
at that point and closed with the checksum.
```
Req:  B8 <TGT> F1 02 21 2<n> <CS>      (n = 0..F → block 0..15)
Resp: B8 F1 <SRC> <len> 61 2<n> <values...> <CS>
```

**DynamicallyDefinedLocalIdentifier — measured values dynamic (`2C 10`)** — p. 10-25/10-26.
Up to **10 PIDs** (2 bytes each) are defined and stored; a subsequent `2C 10` request with
no PIDs re-reads them. Positive response `6C`.
```
Req:  B8 <TGT> F1 <len> 2C 10 <PID1_H PID1_L ... PID10_H PID10_L> <CS>
Resp: B8 F1 <SRC> <len> 6C 10 <val1_H val1_L ...> <CS>
```

**ClearDiagnosticInformation (`14`)** — p. 10-7. Group HB/LB `00 00` clears the entire
(powertrain) fault memory; a specific DTC clears just that fault. Positive response `54`.
```
Req:  B8 <TGT> F1 03 14 <GHB> <GLB> <CS>
Resp: B8 F1 <SRC> 03 54 <GHB> <GLB> <CS>
```

**ReadDiagnosticTroubleCodesByStatus — short (`18 00 00 00`)** — p. 10-10. Positive
response `58`, Length `02+errors`, then `numberOfDTC`, followed by triplets
`DTC_HB, DTC_LB, statusOfDTC` per fault.
```
Req:  B8 <TGT> F1 04 18 00 00 00 <CS>
Resp: B8 F1 <SRC> <02+n*3> 58 <count> [DTC_H DTC_L STATUS]... <CS>
```

**ReadStatusOfDiagnosticTroubleCodes — long (`17`)** — p. 10-7/10-8. Reads one DTC with
frequency/logistics counters and up to three environment-condition (Umweltbedingung) sets
(first / second / last occurrence, each with UB1–UB4 and a KM-stand). Positive response `57`.

**Actuator drive — InputOutputControlByLocalIdentifier (`30`, LID `C1`–`CC`)** — p. 10-41/10-42.
The `inputOutputControlParameter = 07` (`IOCP_STA`) takes a **duty-cycle byte
(Tastverhältnis)**; 5 %…95 % limits (values `00`–`05` → 5 %, `5F`–`64` → 95 %). Multiple
actuators can be driven at once; positive response `70`. Example LIDs (R6 variant): `C2`
EGR steller (0–100 %, 1 LSB = 1 %), `C3` electric fuel pump (on/off), `C6` boost-pressure
steller, `CB` auxiliary heater, etc. The rail-pressure control valve is intentionally not
drivable.
```
Drive:   B8 <TGT> F1 04 30 <LID> 07 <duty> <CS>   → resp 70 <LID> 07 <duty>
Release: B8 <TGT> F1 03 30 <LID> 00       <CS>   → resp 70 <LID> 00
```

**StartRoutineByLocalIdentifier (`31 40`) — injector/cylinder diagnostics** — p. 10-45/10-46.
Modes selected by routine option: `00` = cylinder-selective quantity corrections
(`xcoInFeMo=10h`, needs rpm > 0); `01` = cylinder-selective speeds with running-smoothness
controller (LRR) disabled (`11h`, rpm > 0); rpm = 0 → **compression test** (`12h`, injection
blocked so the engine cranks without starting). Values are exposed on measurement channels
`dzmzMk1..8` / `dzmzN1..8` and read back via a measured-value service.

**ReadDS2EcuIdentification (`A2`)** — p. 10-60. Positive response **`E2`**, Length `2B`;
same identification fields as `1A 80`, DS2-protocol formatting. Negative-response codes:
`11` = service not present, `12` = unplausible request.

---

## 8. Negative / error responses

General negative-response frame:

```
[Format] [Target=F1] [Source=ECU] [Len=03] 7F <original SID> <ResponseCode> [CS]
```
(e.g. p. 10-6, 10-8, 10-44)

**Response codes (ResponseCode) seen in the document** (full list per "KWP 2000 Spec./III,
Table 4.4"):

| Code | Meaning |
|------|---------|
| `10` | (e.g. EWS) start-value init not yet performed / ECU not reset this drive cycle |
| `11` | Service not present |
| `12` | Unplausible request / wrong LocalIdentifier or parameter / "unverständliche Anforderung" |
| `21` | busy-repeatRequest |
| `22` | Cannot be executed (e.g. rpm not zero) |
| `31` | Wrong baud rate |

`StartDiagnosticSession` negative codes: `21` busy, `22` conditions not correct, `31` wrong
baud (p. 10-6). Programming-status data byte codes (`30 EB 01`): `01` OK, `09`/`0A`/`0B`
reference errors (BRIF/ZIF), `0C` program incomplete, `0D`/`0E` data reference errors, `0F`
data incomplete (p. 10-45).

---

## 9. Fault codes — SAE J2012 structure (p. 10-67 / PAGE 434)

DTCs are **2 bytes**. The **first nibble** encodes the category, the remaining three nibbles
are **BCD-coded**:

| First 2 bits | Letter | System |
|-------------|--------|--------|
| `00` | **P** | Powertrain (Motor/Antriebsstrang) |
| `01` | **C** | Chassis (Fahrgestell) |
| `10` | **B** | Body (Karosserie) |
| `11` | **U** | reserved |

Next 2 bits = group 0–3. Remaining 3 nibbles = fault number 0–9 each (BCD).

---

## 10. McMess (fast measurement protocol) — p. 10-68…10-73

Optional K-Line protocol for fast RAM read-out, entered via Keyword Protocol 2000.
Frames use **9 data bits**, checksum = **sum of preceding bytes**.

Handshake (p. 10-69/10-70):
```
TG→SG StartComm:   Format=81  Target=08  Source=xx  Mode=81  CS      (1 data byte)
SG→TG Keywords:    Format=83  Target=xx  Source=08  Mode=C1  KW1=C4 KW2=46  CS
TG→SG select McM:  Format=82  Target=08  Source=xx  Mode=A0  Param(A5=10.4k / A6=38.4k) CS
SG→TG McM ident:   Format=83  Target=xx  Source=08  E0  <param mirror>  Ident=08  CS
```
- Format-byte low bits = number of data bytes in the block (`81`→1, `82`→2, `83`→3).
- EDC communication identifier = `08`; word-handshake fast measuring uses 2 ms inter-byte
  time; high baud 38,4 kBaud, PC baud 10,4 kBaud. Byte order in measure mode = Low, High.
- Timeouts: measure-mode `xcwMcM_ToM` up to 132 s; block-mode `xcwMcM_ToB` up to 656 s.
- Supports ignition-synchronous measuring with a trigger byte `1tt` (p. 10-71/10-72).

---

## 11. Blink-code diagnostics (historical context)

The document does **not** describe a dedicated K-Line "blink-code read-out" procedure.
"Blinkcode" here refers to the **MIL (malfunction indicator lamp) blink attribute** per
fault path: label `fbwS...BCO` ("Blinkkode") is an applicable attribute, and for
emissions-relevant faults with the blink attribute the MIL is set to blink
(`fbmSMIL.Bit1 = 1`), toggled by `fbwT_DBLNK` / `fbwT_MBLNK` on/off times
(pp. 8-32…8-39 / PAGES 326–333). Fault de-bounce (Entprellung) types: time-quanta or
driving-cycle for entry and healing (p. 8-33). No blink-code request telegram exists — all
tester interaction is via the KWP services above.

---

## 12. Differences vs. classic BMW DS2

The project's `CLAUDE.md` targets **"BMW DS2, concept 0x0006, 4-byte header, XOR checksum"**
and its known-good DDE4.0 telegram is `B8 12 F1 04 2C 10 0F 10` with **CHK = 7C**.

| Aspect | This document (EDC15C / DDE4.0 KWP) | Project's current DS2 assumption |
|--------|-------------------------------------|----------------------------------|
| Header | `Format=B8, Target, Source=F1, Length` (4-byte, matches!) | 4-byte `[ADDR][LEN][..][CHK]`, len@1 |
| Format byte | `B8` (physical), `80` for TesterPresent | fixed part of frame |
| **Checksum** | **Additive sum, low byte** (`CS = Σ bytes`) — explicit for KWP2000 & McMess (p. 10-63, 10-69) | **XOR** of all bytes |
| Baud | 9600 (DS2-compat) or 10400 (KWP2000) | 9600 |
| Parity | init word: even (functional) / odd (physical); **block bytes: none** (8N1) | project uses **Even** for block bytes |
| Init | DS2-compat mode = no 5-baud; KWP2000 mode = 5-baud or fast init | DS2 needs no 5-baud init |

> ⚠️ **Key reconciliation point.** For the concrete DDE4.0 telegram
> `B8 12 F1 04 2C 10 0F 10`, an **XOR** of all bytes yields `7C` (matches the project's
> observed checksum), whereas the **additive sum** low byte yields `1A`. So the ECU the
> project talks to behaves per **classic DS2 (XOR)**, *not* per the additive checksum this
> Bosch document specifies for its KWP2000 / McMess modes. Two plausible explanations:
> 1. The project is using the **classic BMW DS2** byte protocol (XOR), which coexists on the
>    same bus as this ECU's "DS2-compatible mode" but is a different framing than the
>    KWP2000 blocks documented here; or
> 2. The additive-checksum text applies specifically to the Standard-KWP2000 (10400 Bd) and
>    McMess paths, while the 9600-Bd DS2-compatible service framing uses a different (XOR)
>    checksum that the document leaves as `??`.
>
> The document does **not** explicitly state the checksum algorithm inside the DS2-compatible
> service tables (all show `CS = ??`), so **treat the checksum as the one empirically
> verified against the real ECU (XOR → 7C)** and keep the additive-sum rule documented above
> for the Standard-KWP2000 / McMess modes.

---

## 13. Source page map

| Topic | Printed pages | Extractor PAGE markers |
|------|--------------|------------------------|
| Fault memory / MIL blink attributes | 8-32…8-45 | 326–339 |
| Chapter 10 intro, DLL state machine, `xcm_mode` | 10-1…10-3 | 367–369 |
| Service overview table + addresses (Tab 10-1) | 10-4…10-5 | 370–371 |
| Individual DS2-compatible services | 10-6…10-59 | 372–426 |
| ReadDS2EcuIdentification | 10-60…10-61 | 426–427 |
| 5-baud init, sync/keywords, block format | 10-60…10-64 | 426–430 |
| Fast init (wake-up) | 10-64…10-65 | 430–431 |
| Timing definitions | 10-66 | 432 |
| TesterPresent, error handling, SAE J2012 DTC | 10-67…10-68 | 433–434 |
| McMess | 10-68…10-73 | 434–439 |

*Uncertain / not fully specified in the source:* numeric checksum values (all `??`), and the
exact checksum algorithm used by the 9600-Bd DS2-compatible services (see §12).
