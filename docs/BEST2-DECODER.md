# BEST/2 instruction decoder — authoritative spec

Foundation for a desync-proof VM. Verified empirically against **1748 real ECU `.prg`
files** (101 598 jobs): **99.92 % decode cleanly to `eoj` (0x1d)** with the rules below.
The remaining 0.08 % are embedded data / jump-tables that linear scan walks into but
real (jump-following) execution never does — not decoder errors.

## Instruction layout

Every instruction is regular:

```
[opcode] [mode] [operand0 data] [operand1 data]
```

- `mode` high nibble = operand0 addressing mode, low nibble = operand1 addressing mode.
- Operand data length is fully determined by its nibble (see table). This means one
  correct length function gives the correct length for **every** opcode → the
  instruction stream can never desync.

## Addressing modes (EdiabasLib `OpAddrMode` enum — authoritative)

| Nibble | Name          | Operand data bytes                         |
|--------|---------------|--------------------------------------------|
| 0x0    | None          | 0                                          |
| 0x1    | RegS          | 1 (reg id)                                 |
| 0x2    | RegAb         | 1                                          |
| 0x3    | RegI          | 1                                          |
| 0x4    | RegL          | 1                                          |
| 0x5    | Imm8          | 1                                          |
| 0x6    | Imm16         | 2                                          |
| 0x7    | Imm32         | 4                                          |
| 0x8    | ImmStr        | 2 (u16 LE length) + length data bytes      |
| 0x9    | IdxImm        | 2  (base reg + imm8 offset)                |
| 0xA    | IdxReg        | 2  (base reg + reg offset)                 |
| 0xB    | IdxRegImm     | 3  (base reg + reg offset + imm8)          |
| 0xC    | IdxImmLenImm  | 3  (base reg + imm8 offset + imm8 length)  |
| 0xD    | IdxImmLenReg  | 3  (base reg + imm8 offset + reg length)   |
| 0xE    | IdxRegLenImm  | 3  (base reg + reg offset + imm8 length)   |
| 0xF    | IdxRegLenReg  | 3  (base reg + reg offset + reg length)    |

Length vector by nibble: `[0,1,1,1,1,1,2,4, ImmStr, 2,2,3, 3,3,3,3]`
where `ImmStr = 2 + u16le(next two bytes)`.

## Special opcodes (do NOT follow the generic 2-operand scheme)

| Opcode | Name     | Length | Note                                             |
|--------|----------|--------|--------------------------------------------------|
| 0x40   | enewset  | **2**  | 2nd byte is not a mode byte. Commit current result set, start a new one. Verified: fixing this alone recovers 1141 corpus jobs. |

> If any further special opcodes surface, add them here. Everything else decodes with
> the generic rule above.

## Registers (from CLAUDE.md, consistent with corpus)

```
B0-BF = 0x00-0x0F (byte)   I0-I7 = 0x10-0x17 (16-bit)
L0-L3 = 0x18-0x1B (32-bit) S0-S23 = 0x1C-0x33 (string/data)
```

## Reference

EdiabasLib (uholeschak) `EdiabasNet.cs` — `OpAddrMode` enum. Empirical validation:
`scratchpad/analyze.py` over `ecu/*.prg`.
