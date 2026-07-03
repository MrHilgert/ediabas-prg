# PROTO-KWP2000 — BMW-FAST (0x010F) and KWP2000-BMW (0x010C/0x010D) over K-line

Implementation-ready byte-level spec for `src/transport/kwp2000.rs`.
Grounded in the EdiabasLib reference implementation (see [Sources](#sources)) and
ISO 14230 (KWP2000). Everything here maps onto our existing `Driver` / `Transport`
primitives — no new physical primitives are needed.

**Covers three EDIABAS concepts, all on single-wire K-line via FTDI/KDCAN:**

| Concept | EDIABAS name | Baud (typ.) | Parity | Checksum | Init handshake | Keep-alive |
|---------|--------------|-------------|--------|----------|----------------|------------|
| 0x010F  | BMW-FAST     | 10400 (from CommParameter[1]) | **None** | **SUM mod 256** | **none** — ECU always listens | none at transport level |
| 0x010C  | KWP2000-BMW  | from CommParameter[1] (typ. 10400) | **None** | **SUM mod 256** | fast-init BREAK pulse + StartCommunication telegram from CommParameter | TesterPresent telegram from CommParameter, cadence from CommParameter |
| 0x010D  | KWP2000\*    | from CommParameter[1] | **Even** | **XOR** | **none** | none |

> **⚠ Checksum correction.** BMW K-line is *not* uniformly XOR. DS2 (0x0001/5/6) and
> KWP2000\* (0x010D) use XOR-of-all-bytes; **BMW-FAST (0x010F) and KWP2000-BMW (0x010C)
> use the ISO 14230 checksum: arithmetic sum of all bytes modulo 256.**
> Verified in EdiabasLib: `CalcChecksumBmwFast` (`sum += data[i]`) vs `CalcChecksumXor`
> (`sum ^= data[i]`). Using XOR here is the classic silent-failure bug — the ECU simply
> never answers. (D-CAN, concept 0x0110, reuses the exact same SUM framing at 115200,
> so this spec is directly reusable for the future D-CAN transport.)

All three concepts use 8 data bits, 1 stop bit.

---

## 0. The architectural rule (division of labour)

The SGBD bytecode (our VM) builds the **complete request telegram including the
header** — `[FMT][TGT][SRC](LEN)[SID][DATA…]` — and calls
`transport.exchange(frame)` with the frame **without the trailing checksum**.
EdiabasLib works identically: `TransKwp2000` computes `sendLength =
TelLengthBmwFast(sendData)` and writes the checksum at that index
(`EdInterfaceObd.cs:4119-4122`).

The transport therefore owns exactly four things:

1. **Checksum** — append 1 byte to the outgoing frame; verify + strip on the response.
2. **Init handshake** — fast-init wakeup + StartCommunication (0x010C only).
3. **Response framing** — read the reply using the BMW header length rules, return
   the **full telegram minus the trailing checksum** (same as `ds2.rs::receive()`:
   EDIABAS result byte-positions index into the full frame, header included, and jobs
   validate header bytes).
4. **Keep-alive** — TesterPresent cadence (0x010C only) + transparent NR 0x78
   (responsePending) handling inside `exchange()`.

The transport must **never** construct or modify request header bytes.
(One EdiabasLib nuance we deliberately diverge from: it hands the response back
*including* the trailing checksum (`receiveLength = TelLengthBmwFast(..) + 1`,
`EdInterfaceObd.cs:4262`). Our VM's `xrecv` semantics were established with
`ds2.rs` returning *minus* CHK and all DS2 jobs work that way — keep it consistent.
If a KWP job ever indexes the checksum position, revisit.)

---

## 1. Frame layout — BMW-FAST framing (0x010F and 0x010C)

Identical framing for request and response. Three length forms, selected by the
FMT byte (source: `EdInterfaceBase.cs::TelLengthBmwFast`, line 881):

### 1.1 FMT byte

```
bit:   7 6 | 5 4 3 2 1 0
       A1A0|   L5..L0
```

- `A1A0` — address mode (ISO 14230-2). BMW uses `10` = physical addressing with
  target/source bytes present. **Response validity check: `(FMT & 0xC0) == 0x80`**,
  i.e. only `0x80..0xBF` accepted as a response header; anything else → protocol
  error, drain RX, fail the exchange (`EdInterfaceObd.cs:4172`).
- `L5..L0` — payload length (SID + data bytes), 1..63. `0` ⇒ a separate length
  byte/word follows the 3-byte header.

### 1.2 The three forms (lengths *exclude* the trailing CHK)

```
Short   (1..63 bytes payload):   [FMT=0x80|N][TGT][SRC][payload ×N]              len = N + 3
Long    (1..255 bytes payload):  [FMT=0x80][TGT][SRC][LEN≠0][payload ×LEN]       len = LEN + 4
Extra   (256..65535, BMW ext.):  [FMT=0x80][TGT][SRC][0x00][LENH][LENL][payload] len = LEN16 + 6
```

- `TGT` = target address (ECU, e.g. 0x12 = DDE), `SRC` = source (tester = 0xF1).
- Long form: `LEN` at byte [3], 8-bit.
- Extra-long form is a BMW extension (not ISO 14230, which caps at 255): triggered
  when `FMT & 0x3F == 0` **and** byte[3] == 0; then bytes [4],[5] are a
  **big-endian** u16 payload length. On receive this means: after the initial
  4-byte header read, if `(hdr[0] & 0x3F) == 0 && hdr[3] == 0`, read **2 more**
  header bytes before computing the tail length (`EdInterfaceObd.cs:4180-4188`).
- CHK = **sum of all preceding bytes mod 256**, appended after the payload.

### 1.3 Worked examples (verify your checksum code against these)

Request, short form — readEcuIdentification (SID 0x1A 0x80) to DDE (0x12):

```
82 12 F1 1A 80 | 1F
FMT=0x82 → 0x80|2 = 2 payload bytes; TGT=12 SRC=F1; payload=1A 80
CHK = (82+12+F1+1A+80) mod 256 = 0x21F & 0xFF = 1F
```

Positive response, short form (SID+0x40 = 0x5A, 2 ident bytes 41 42):

```
84 F1 12 5A 80 41 42 | E4        (TGT/SRC swapped: ECU → tester)
CHK = (84+F1+12+5A+80+41+42) mod 256 = 0x2E4 & 0xFF = E4
```

Negative response (see §4):

```
83 F1 12 7F 1A 78 | 97           payload = 7F <req-SID> <NRC>
CHK = (83+F1+12+7F+1A+78) mod 256 = 0x297 & 0xFF = 97
```

Request, long form (FMT low bits 0, LEN at [3]):

```
80 12 F1 04 2C 10 0F 10 | E2
len-without-chk = LEN+4 = 8; CHK = (80+12+F1+04+2C+10+0F+10) mod 256 = E2
```

TesterPresent (0x3E) + typical positive response (0x7E) — see §6:

```
TX: 81 12 F1 3E | C2
RX: 81 F1 12 7E | 02
```

---

## 2. Frame layout — KWP2000\* (0x010D)

Different transceiver in EdiabasLib (`TransKwp2000S`, `EdInterfaceObd.cs:4762`):

```
[FMT][TGT][SRC][LEN][payload ×LEN] | CHK(XOR)
```

- Header is **always 4 bytes**; length is **always** at byte [3]
  (`TelLengthKwp2000S = data[3] + 4`, line 4861). FMT low bits are **ignored** for
  length; there is no extra-long form and no `(FMT & 0xC0)` validity gate in the
  reference implementation.
- **CHK = XOR of all preceding bytes** (`CalcChecksumXor`).
- **Parity = Even**, baud from CommParameter[1].
- No init handshake, no TesterPresent. NR 0x78 handling identical to §4.
- In our `CommConfig` terms this is exactly `len_offset = 3`, `len_add = 5` — the
  same read path as the DS2 "BMW 4-byte header" case, plus the NR78 loop.

Example: `B8 12 F1 02 21 0B | 73` (XOR: B8^12^F1^02^21^0B = 73).

> Note: our `Protocol::from_concept` maps 0x010C **and** 0x010D to
> `Protocol::Kwp2000Bmw`, so `Kwp2000Transport` must branch on `cfg.concept`
> (kept in `CommConfig::concept` for exactly this purpose): 0x010D ⇒ XOR + Even +
> LEN@[3] + no init; 0x010C ⇒ SUM + None + §1 framing + §3 init; 0x010F ⇒ SUM +
> None + §1 framing, no init.

---

## 3. Fast-init wakeup + StartCommunication (0x010C only)

Source: `SendWakeFastInit` (`EdInterfaceObd.cs:3506`) + `TransKwp2000Bmw`
(`EdInterfaceObd.cs:4685`). Matches ISO 14230-2 fast initialization:
T_iniL = 25 ± 1 ms low, T_WuP = 50 ± 1 ms from falling edge to first bit of the
StartCommunication request.

### 3.1 Sequence (exact, in our Driver primitives)

```
// Pre-condition: bus idle. EdiabasLib waits until (now − last_comm) ≥ TimeoutStd
// (CommParameter[2]) before initiating. ISO 14230 calls this W5 ≥ 300 ms.
// Enforce: idle ≥ max(300 ms, cfg.timeout_std_ms since last traffic).

driver.flush_rx();
driver.set_break(true);          // K-line LOW           t = 0
sleep(25 ms);                    //                      (±1 ms — see §3.3)
driver.set_break(false);         // K-line HIGH          t = 25 ms
sleep(25 ms);                    //                      t = 50 ms
driver.flush_rx();               // discard break artifact (FTDI often reports a
                                 // 0x00 framing byte for the BREAK). EdiabasLib
                                 // purges RX inside SendData just before TX.
send StartCommTel + SUM-checksum // normal TX path (echo drain applies, §5.2)
read response                    // normal §5 receive path, timeout = TimeoutStd
EcuConnected = true
```

### 3.2 The StartCommunication telegram

It is **not hardcoded** — it comes from the SGBD via CommParameter elements
[22..22+len), length in element [21] (max 11 bytes), *without* checksum (the
transport appends SUM as usual). Typical content:

```
TX: 81 <ECU> F1 81 | CHK          e.g. 81 12 F1 81 | 05
RX: 83 F1 <ECU> C1 <KB1> <KB2> | CHK   e.g. 83 F1 12 C1 EA 8F | C0
```

`0x81` = StartCommunication, positive response `0xC1` + two key bytes (commonly
`EA 8F` / `E9 8F` for ISO 14230-4-style ECUs — informational; we don't act on them).
If `StartCommTelLen == 0`, skip both the BREAK pulse and the telegram — the ECU
connects without stimulation (EdiabasLib only wakes when `ParStartCommTelLen > 0`).

**Reconnect rule (lazy, exactly as EdiabasLib):** the connected flag drops to
`false` on *any* exchange error; the next `exchange()` re-runs §3.1 before sending
the real request. `init_connection()` runs it eagerly once.

### 3.3 Timing precision (Windows caveat)

The 25 ms half-pulses have ±1 ms tolerance in ISO 14230. `std::thread::sleep` on
Windows has ~15.6 ms default granularity. EdiabasLib busy-waits on `Stopwatch`
for exactly this reason. Implement as: coarse `sleep(20 ms)` + spin on
`Instant::elapsed()` to 25 ms (or raise timer resolution). **Needs live-hardware
confirmation** that a KDCAN clone's `SET_BREAK` latency over USB keeps the pulse
within tolerance; USB FS frames add up to ~1 ms jitter per transition, which is
borderline but is exactly what EdiabasLib ships with over serial.

### 3.4 BMW-FAST (0x010F): explicitly NO init

`TransBmwFast` is a one-line wrapper around the raw transceiver — no wake pulse,
no StartCommunication, no connected-state gate (`EdInterfaceObd.cs:3768-3771`).
BMW-FAST ECUs listen permanently at 10400 baud. `init_connection()` = `Ok(())`,
like DS2. Same for 0x010D.

---

## 4. Positive vs negative responses, and the NR78 loop

A response telegram's payload starts at:
- byte [3] (short form), byte [4] (long form), byte [6] (extra-long) — for §1 framing;
- byte [4] always — for 0x010D.

**Negative response** = payload is exactly 3 bytes: `7F <request-SID> <NRC>`.
(EdiabasLib's actual test: `dataLen == 3 && payload[0] == 0x7F` — and for the
0x78 branch `payload[2] == 0x78`; it does not verify the echoed SID. Mirror that.)

Common NRCs (ISO 14230-3):

| NRC  | Meaning | Transport behaviour |
|------|---------|---------------------|
| 0x10 | generalReject | return telegram to VM |
| 0x11 | serviceNotSupported | return to VM |
| 0x12 | subFunctionNotSupported | return to VM |
| 0x21 | busyRepeatRequest | **return to VM** — for BMW concepts EdiabasLib's NR21 auto-retry is disabled (retry params = 0; only the VAG/EDIC path enables it). The SGBD job does its own retry. |
| 0x22 | conditionsNotCorrectOrRequestSequenceError | return to VM |
| 0x23 | routineNotComplete | return to VM (same note as 0x21) |
| 0x31 | requestOutOfRange | return to VM |
| 0x33 | securityAccessDenied | return to VM |
| 0x35 | invalidKey | return to VM |
| 0x78 | requestCorrectlyReceived-**ResponsePending** | **do NOT return — keep waiting** (below) |

### 4.1 NR 0x78 (responsePending) — the one the transport must absorb

Algorithm (from `TransKwp2000` loop + `Nr78DictAdd`, `EdInterfaceObd.cs:4160-4260`,
3719):

```
pending78: map<src_addr, retry_count>      // src = response byte [2]
clear pending78 before each send
loop:
    first_byte_timeout = pending78.is_empty() ? TimeoutStd : TimeoutNr78
    read one complete, checksum-valid telegram (§5)
    if payload == [7F, _, 78]:
        count = pending78[src]++ (insert as 0 if new)
        if count > RetryNr78: remove entry        // exceeded
        if pending78 is empty: return this telegram   // deliver final 7F..78 to VM
        continue loop                             // wait again with TimeoutNr78
    else:
        pending78.remove(src); return telegram    // final answer
```

Key points: the ECU sends `7F xx 78` to say "working on it — extend your timeout";
each occurrence re-arms the wait with `TimeoutNr78` (typ. 2000-3000 ms, from
CommParameter). When `RetryNr78` is exceeded, the *last 7F..78 telegram itself*
is returned to the VM with success (not an error) — the job decides what to do.
The per-source map matters only for functional addressing; with one ECU a single
counter is equivalent, but keep the src key — it's cheap and correct.

---

## 5. exchange() — full TX/RX algorithm

### 5.1 Regeneration gap (P3)

Before every send: wait until `regen_time_ms` (ParRegenTime, CommParameter) has
elapsed since the **end of the previous response** (`LastResponseTick` in
EdiabasLib). Identical to what `ds2.rs::exchange()` already does.

### 5.2 Transmit

```
1. flush_rx()                                    // clean slate (EdiabasLib purges in SendData)
2. chk = sum-mod-256 (0x010F/0x010C) or xor (0x010D) over frame
3. write frame ‖ [chk]
   - if interbyte_ms > 0 (0x010C CommParameter[5]; always 0 for 0x010F):
     byte-by-byte with interbyte_ms spacing, same loop as ds2.rs send_raw (P4 timer)
4. echo drain (KDCAN mirrors TX on RX): read_exact(frame.len()+1) with
   set_timeout(100) — EdiabasLib EchoTimeout = 100 ms — and compare to what was
   sent; mismatch ⇒ drain RX, Error (EdInterfaceObd.cs:4137-4156)
```

Sanity check (recommended, warn-only): `tel_length(frame) == frame.len()` using
the §1/§2 length rules — catches a VM/SGBD framing bug before it hits the wire.

### 5.3 Receive (§1 framing — 0x010F / 0x010C)

```
1. set_timeout(first_byte_timeout)               // TimeoutStd, or TimeoutNr78 (§4.1)
   read_exact(4)                                 // header
2. if (hdr[0] & 0xC0) != 0x80 → drain RX, Error::Protocol
3. if (hdr[0] & 0x3F) == 0 && hdr[3] == 0:
       set_timeout(timeout_tel_end); read_exact(2)   // LENH LENL (extra-long)
4. total_without_chk = tel_length(hdr)           // §1.2 rules
   remaining = total_without_chk − header_bytes_read + 1   // + CHK
5. set_timeout(timeout_tel_end)                  // P1 inter-byte, typ. 20 ms
   read_exact(remaining)                         // EdiabasLib uses TelEnd for both
                                                 // first-byte and inter-byte of the tail
6. verify checksum (SUM); mismatch → drain RX, Error::Checksum
7. NR78 logic (§4.1); on final: record LastResponseTick,
   return full frame minus trailing CHK
```

For 0x010D replace step 2-4 with: header = 4 bytes, `total_without_chk = hdr[3] + 4`,
checksum = XOR.

### 5.4 Timer map (ISO 14230-2 ↔ EDIABAS ↔ our code)

| ISO timer | Meaning | EDIABAS parameter | Our field / primitive |
|-----------|---------|-------------------|------------------------|
| P1 | ECU inter-byte gap within response | ParTimeoutTelEnd (typ. 20 ms) | `timeout_tel_ms` → `set_timeout` for tail/inter-byte reads |
| P2 | request end → response start | ParTimeoutStd (typ. 600-2000 ms; generous vs ISO's 25-50 ms) | `timeout_std_ms` → `set_timeout` for first header byte |
| P2\* extended | while 7F..78 pending | ParTimeoutNr78 (typ. 2000-3000 ms) | new `timeout_nr78_ms` |
| P3 | response end → next request | ParRegenTime | `regen_time_ms` sleep before TX |
| P4 | tester inter-byte within request | ParInterbyteTime | `interbyte_ms` TX spacing |

---

## 6. TesterPresent keep-alive (0x010C only)

Source: `IdleKwp2000Bmw` (`EdInterfaceObd.cs:4725`).

- Telegram comes from CommParameter elements [10..10+len), length in [9]
  (max 11 bytes), without checksum. Typical: `81 <ECU> F1 3E` → response
  `81 F1 <ECU> 7E`.
- Cadence: if `(now − last_comm) ≥ TesterPresentTime` (CommParameter[8], ms) and
  `TesterPresentTelLen > 0`, send it through the normal §5 exchange path (full
  NR78 handling, echo drain, checksum). Any error ⇒ `EcuConnected = false`
  (next exchange re-inits per §3.2). `last_comm` refreshes on **every** exchange,
  so keep-alive only fires when genuinely idle.
- 0x010F and 0x010D have **no** transport-level keep-alive in EdiabasLib
  (no idle function registered).

**Plumbing in our architecture** (EdiabasLib runs this on a comm thread; our
`Transport` trait has no idle hook). Two options, spec'd in order of preference:

1. **Lazy (minimum viable, recommended first):** inside `exchange()`, if connected
   and `elapsed ≥ tester_present_time_ms`, send TesterPresent before the real
   request; combined with the §3.2 auto-reconnect this is self-healing even when
   the session lapsed. GUI live-polling keeps the session alive anyway.
2. **Active:** add an optional `Transport::idle()` (default no-op) and have the
   `Session` worker thread call it on a ~500 ms tick. Needed only if jobs are run
   sporadically and re-init per request proves too slow on real hardware.

---

## 7. CommParameter layouts (input to `CommConfig::parse`)

The `xsetpar` ImmStr block is an array of little-endian u16 elements (element
index below = u16 index, byte offset = 2×index). Telegram bytes are stored **one
byte per element**. Layouts from the EdiabasLib concept dispatch
(`EdInterfaceObd.cs:695-800`):

**0x010F BMW-FAST** (≥ 7 elements; element [7] = optional parameter checksum):

| idx | value |
|-----|-------|
| 0 | concept = 0x010F |
| 1 | baud (10400) |
| 2 | TimeoutStd (P2) |
| 3 | RegenTime (P3) |
| 4 | TimeoutTelEnd (P1) |
| 5 | RetryNr78 |
| 6 | TimeoutNr78 |

**0x010C KWP2000-BMW** (≥ 33 elements; [33] = optional parameter checksum):

| idx | value |
|-----|-------|
| 0 | concept = 0x010C |
| 1 | baud |
| 2 | TimeoutStd |
| 3 | RegenTime |
| 4 | TimeoutTelEnd |
| 5 | InterbyteTime (P4) |
| 6 | RetryNr78 |
| 7 | TimeoutNr78 |
| 8 | TesterPresentTime (ms) |
| 9 | TesterPresentTelLen (0..11) |
| 10-20 | TesterPresent telegram bytes (no CHK) |
| 21 | StartCommTelLen (0..11) |
| 22-32 | StartCommunication telegram bytes (no CHK) |

**0x010D KWP2000\*** (≥ 7 elements; [21] = optional parameter checksum):
indices 0-7 identical to 0x010C ([5] InterbyteTime, [6] RetryNr78,
[7] TimeoutNr78); no TesterPresent/StartComm fields.

> ⚠ Our current `CommConfig::parse` applies the **DS2 layout** (timeouts at
> elements 5/6/7, interbyte at 8) to every concept — for the KWP concepts the
> indices shift (timeouts at 2/3/4). `parse()` must branch on the concept.
> **Verify against the corpus** that KWP SGBDs really emit u16 elements in
> `xsetpar` (grep a 0x010C .prg for its xsetpar block and check
> `len ≥ 68` bytes and plausible values); EdiabasLib receives them API-side as
> `UInt32[]`, so a u32-element encoding in some SGBDs is conceivable.

### 7.1 New CommConfig fields required

```
retry_nr78: u32,                 timeout_nr78_ms: u64,
tester_present_time_ms: u64,     tester_present_tel: Vec<u8>,   // no CHK
start_comm_tel: Vec<u8>,         // no CHK; empty = no init stimulation
```

Defaults when absent: `retry_nr78 = 3`, `timeout_nr78_ms = 3000`, telegrams empty.
Also: parity must become per-concept (`None` for 0x010F/0x010C, `Even` for 0x010D)
— the current blanket "non-DS2 ⇒ None" rule is wrong for 0x010D.

---

## 8. Mapping to our code

| Piece | Where | Notes |
|-------|-------|-------|
| checksum select (SUM vs XOR), framing select (§1 vs §2) | `Kwp2000Transport` internal, branch on `cfg.concept` | store as an enum field set in `configure()` |
| baud/parity per concept | `configure()` → driver (re)open handled by Session/driver as for DS2 | 10400 is a non-standard rate: FT232 handles it natively; **verify** the `serialport` crate + KDCAN clone accept it on Windows |
| `timeout_std_ms`, `timeout_tel_ms`, `regen_time_ms`, `interbyte_ms` | `configure()` from `CommConfig` | same semantics as ds2.rs |
| fast-init pulse + StartComm (§3) | `init_connection()`; also lazily from `exchange()` when `!connected` (0x010C) | uses only `set_break`, `flush_rx`, normal TX/RX |
| no-op init | `init_connection()` for 0x010F / 0x010D | like DS2 |
| append CHK, echo drain, framed receive, checksum verify, NR78 loop, regen gap | `exchange()` | returns full frame minus CHK |
| TesterPresent | lazy hook at top of `exchange()` (§6 option 1) | 0x010C only |
| `disconnect()` | set `connected = false`; nothing on the wire | EdiabasLib registers no finish function for 0x010C (StopCommunication 0x82 exists in ISO but EDIABAS doesn't send it here) |
| DTR/RTS | **not needed for KDCAN** | EdiabasLib toggles DTR around TX only for echo-less ADS-style cables (`ParSendSetDtr = !HasAdapterEcho`); with echo adapters DTR is just held asserted. Our DS2 works without touching DTR on KDCAN — same here. Keep `echo = true`. |

Suggested internal state: `connected: bool`, `last_comm: Instant`,
`last_response: Instant`, `pending78: HashMap<u8, u32>` (clear before each send).

---

## 9. Needs live-hardware confirmation

1. BREAK pulse timing through KDCAN/FTDI VCP on Windows — is 25 ms ± jitter
   accepted by real 0x010C ECUs (§3.3)?
2. 10400 baud on the clone adapter (custom divisor path in the FTDI driver).
3. Whether KWP xsetpar blocks in our .prg corpus are u16- or u32-element encoded (§7).
4. Whether any K-line SGBD job depends on seeing the trailing checksum byte in
   `xrecv` data (§0 divergence from EdiabasLib).
5. Echo byte-exactness during fast-init: some adapters mirror the BREAK as a 0x00
   byte *after* `flush_rx()` due to USB buffering — if StartComm echo compare
   fails with a leading 0x00, drop leading zeros received within 1 ms of TX start.

---

## Sources

- **EdiabasLib** (authoritative reference), github.com/uholeschak/ediabaslib, master @ 2026-07:
  - `EdiabasLib/EdiabasLib/EdInterfaceObd.cs` — concept dispatch (lines ~695-800:
    0x010C at 717, 0x010D at 774, 0x010F at 801), `TransBmwFast` (3768),
    `TransKwp2000(bool)` raw transceiver (4110-4265: header check 4172, 2-byte
    length 4180, NR78 4246-4260), `TransKwp2000Bmw` (4685: lazy connect + fast
    init + StartComm), `IdleKwp2000Bmw` (4725: TesterPresent), `TransKwp2000S`
    (4762: 0x010D), `TelLengthKwp2000S` (4861), `CalcChecksumXor` (4984),
    `SendWakeFastInit` (3506: 25/25 ms BREAK), `Nr78DictAdd` (3719),
    `EchoTimeout = 100` (164), `SendData`/`ReceiveData` timeout semantics (3241/3381).
  - `EdiabasLib/EdiabasLib/EdInterfaceBase.cs` — `TelLengthBmwFast` (881),
    `DataLengthBmwFast` (907), `CalcChecksumBmwFast` (933).
- **ISO 14230-2** (KWP2000 data link layer): fast init T_iniL = 25 ± 1 ms,
  T_WuP = 50 ± 1 ms, W5 ≥ 300 ms; P1-P4 timers; FMT address-mode bits.
- **ISO 14230-3** (application layer): service IDs, 0x7F negative response
  format, NRC table, TesterPresent 0x3E, StartCommunication 0x81.
- Our working reference implementation: `src/transport/ds2.rs` (echo drain,
  regen gap, framed receive returning frame-minus-CHK).
