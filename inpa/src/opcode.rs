//! Instruction decoder — every INPA instruction is a fixed 4 bytes:
//! `op(1) mode(1) operand(u16 LE)`. There are 14 opcodes.

/// Opcode bytes.
pub mod op {
    pub const PUSH: u8 = 0x01; // push value        (mode = operand class)
    pub const PUSHREF: u8 = 0x02; // push reference (out-arg / object arg)
    pub const PUSH_DEREF: u8 = 0x03; // push value through a reference slot
    pub const POP: u8 = 0x05; // discard n stack values (flag survives for BRF)
    pub const STORE: u8 = 0x06; // store into slot (leaves value on stack)
    pub const STORE_REF: u8 = 0x07; // store through reference slot (out/inout writeback)
    pub const DECL: u8 = 0x08; // declare local; `mode` = type (0x50..0x55)
    pub const ALU: u8 = 0x09; // ALU op; `mode` = sub-op (see `alu`)
    pub const JMP: u8 = 0x0A; // jump to instruction index (operand)
    pub const BRF: u8 = 0x0B; // branch-if-false to instruction index (operand)
    pub const CALL: u8 = 0x0C; // mode 0x80 = call proc #operand, 0x81 = call builtin #operand
    pub const IMPORT: u8 = 0x0D; // call DLL import; operand = const-pool signature index
    pub const RET: u8 = 0x0E; // return
    pub const CALLFRAME: u8 = 0x0F; // begin-call marker (delimits an arg list)
}

/// Operand classes carried in the `mode` byte of value/ref ops.
pub mod mode {
    pub const GLOBAL: u8 = 0x00;
    pub const CONST: u8 = 0x01;
    pub const LOCAL: u8 = 0x02;
    pub const SCREEN: u8 = 0x40; // object table: SCREEN
    pub const MENU: u8 = 0x41; // object table: MENU
    pub const STATE: u8 = 0x42; // object table: STATE
    pub const SM: u8 = 0x43; // object table: STATEMACHINE
    pub const BUILTIN: u8 = 0x81; // reference to a builtin (callback arg); only seen with PUSHREF
}

/// `mode` selector for [`op::CALL`].
pub const CALL_PROC: u8 = 0x80;
pub const CALL_BUILTIN: u8 = 0x81;

/// ALU sub-op names (the `mode` byte of [`op::ALU`]).
pub fn alu_name(sub: u8) -> Option<&'static str> {
    Some(match sub {
        0x60 => "add",
        0x61 => "sub",
        0x62 => "mul",
        0x63 => "div",
        0x64 => "lt",
        0x65 => "gt",
        0x66 => "le",
        0x67 => "ge",
        0x68 => "eq",
        0x69 => "ne",
        0x6A => "and",
        0x6B => "or",
        0x6C => "not",
        0x6D => "neg",
        0x6E => "bor",
        0x6F => "band",
        _ => return None,
    })
}

/// A decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instr {
    pub op: u8,
    pub mode: u8,
    pub opnd: u16,
}

impl Instr {
    /// True when the (op, mode) pair is one of the known encodings.
    pub fn is_known(&self) -> bool {
        match self.op {
            op::PUSH | op::PUSHREF | op::PUSH_DEREF | op::STORE | op::STORE_REF => matches!(
                self.mode,
                mode::GLOBAL
                    | mode::CONST
                    | mode::LOCAL
                    | mode::SCREEN
                    | mode::MENU
                    | mode::STATE
                    | mode::SM
                    | mode::BUILTIN
            ),
            op::CALL => matches!(self.mode, CALL_PROC | CALL_BUILTIN),
            op::POP | op::DECL | op::ALU | op::JMP | op::BRF | op::IMPORT | op::RET | op::CALLFRAME => true,
            _ => false,
        }
    }

    /// A `push const #i` / `push local #i` etc. — helper to spot value pushes.
    pub fn is_push(&self) -> bool {
        self.op == op::PUSH
    }

    /// If this is `call.builtin #id`, return the builtin id.
    pub fn builtin(&self) -> Option<u16> {
        (self.op == op::CALL && self.mode == CALL_BUILTIN).then_some(self.opnd)
    }

    /// If this is `call.proc #idx`, return the proc index.
    pub fn proc_call(&self) -> Option<u16> {
        (self.op == op::CALL && self.mode == CALL_PROC).then_some(self.opnd)
    }
}

/// Decode a code body (`count * 4` bytes) into instructions. Trailing partials are ignored.
pub fn decode(code: &[u8]) -> Vec<Instr> {
    code.chunks_exact(4)
        .map(|b| Instr {
            op: b[0],
            mode: b[1],
            opnd: (b[2] as u16) | ((b[3] as u16) << 8),
        })
        .collect()
}
