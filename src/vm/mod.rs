// BEST/2 VM — instruction format: [OPCODE][MODE][arg0_bytes...][arg1_bytes...]
// MODE nibbles: upper=arg0 type, lower=arg1 type.
// Operand sizes: 0=None, 1=RegS(1b), 2=RegAB(1b), 3=RegI(1b), 4=RegL(1b),
//                5=Imm8(1b), 6=Imm16(2b), 7=Imm32(4b), 8=ImmStr(NN_lo NN_hi + N bytes)
//
// Register map: B0-BF=0x00-0x0f  I0-I7=0x10-0x17  L0-L3=0x18-0x1b  S0-S23=0x1c-0x33
// ByteRegs B0-B3 → bytes 0-3 of L0;  B4-B7 → bytes 0-3 of L1
//
// Full opcode table (EdiabasLib OcList, 184 entries 0x00-0xB7):
//   00 move   01 clear  02 comp   03 subb   04 adds   05 mult   06 divs   07 and
//   08 or     09 xor    0a not    0b jump   0c jtsr   0d ret    0e jc     0f jae
//   10 jz     11 jnz    12 jv     13 jnv    14 jmi    15 jpl    16 clrc   17 setc
//   18 asr    19 lsl    1a lsr    1b asl    1c nop    1d eoj    1e push   1f pop
//   20 scmp   21 scat   22 scut   23 slen   24 spaste 25 serase 26 xconnect 27 xhangup
//   28 xsetpar 29 xawlen 2a xsend 2b xsendf 2c xrequf 2d xstopf 2e xkeyb 2f xstate
//   30 xboot  31 xreset 32 xtype  33 xvers  34 ergb   35 ergw   36 ergd   37 ergi
//   38 ergr   39 ergs   3a a2flt  3b fadd   3c fsub   3d fmul   3e fdiv   3f ergy
//   40 enewset 41 etag  42 xreps  43 gettmr 44 settmr 45 sett   46 clrt   47 jt
//   48 jnt    49 addc   4a subc   4b break  4c clrv   4d eerr   4e popf   4f pushf
//   50 atsp   51 swap   52 setspc 53 srevrs 54 stoken 55 parb   56 parw   57 parl
//   58 pars   59 fclose 5a jg     5b jge    5c jl     5d jle    5e ja     5f jbe
//   60 fopen  61 fread  62 freadln 63 fseek 64 fseekln 65 ftell 66 ftellln 67 a2fix
//   68 fix2flt 69 parr  6a test   6b wait   6c date   6d time   6e xbatt  6f tosp
//   70 xdownl 71 xgetport 72 xignit 73 xloopt 74 xprog 75 xraw 76 xsetport 77 xsireset
//   78 xstoptr 79 fix2hex 7a fix2dez 7b tabset 7c tabseek 7d tabget 7e strcat 7f pary
//   80 parn   81 ergc   82 ergl   83 tabline 84 xsendr 85 xrecv 86 xinfo  87 flt2a
//   88 setflt 89 cfgig  8a cfgsg  8b cfgis  8c a2y    8d xparraw 8e hex2y 8f strcmp
//   90 strlen 91 y2bcd  92 y2hex  93 shmset 94 shmget 95 ergsysi 96 flt2fix 97 iupdate
//   98 irange 99 iincpos 9a tabseeku 9b flt2y4 9c flt2y8 9d y42flt 9e y82flt 9f plink
//   a0 pcall  a1 fcomp  a2 plinkv a3 ppush  a4 ppop   a5 ppushflt a6 ppopflt a7 ppushy
//   a8 ppopy  a9 pjtsr  aa tabsetex ab ufix2dez ac generr ad ticks ae waitex af xopen
//   b0 xclose b1 xcloseex b2 xswitch b3 xsendex b4 xrecvex b5 ssize b6 tabcols b7 tabrows

use std::collections::HashMap;
use crate::config::CommConfig;
use crate::transport::Transport;
use crate::prg::Table;
use crate::trace::{trace, vtrace};

mod decode;
mod value;

pub use value::Value;
use value::{fmt_flt, parse_f64};
use decode::{
    blat, cstr, hex, nib_width, nibble_size, parse_hex_str, read_imm_u32,
    read_long_at, read_str_at, reg_val_from_map, skip_instr, unlat,
};

pub type ResultSet = HashMap<String, Value>;

const REG_L0: u8 = 0x18;
const REG_L1: u8 = 0x19;
const REG_S1: u8 = 0x1d;

#[derive(Default, Clone)]
struct Flags {
    carry: bool,
    zero: bool,
    minus: bool,
    overflow: bool,
}

pub struct Vm {
    regs: HashMap<u8, Value>,
    /// EDIABAS value stack — byte-granular (Stack<byte>). push/pop/atsp operate
    /// on `width` bytes little-endian, where width is the operand's data length.
    stack: Vec<u8>,
    transport: Box<dyn Transport>,
    tables: HashMap<String, Table>,
    active_table: Option<String>,
    found_row: Option<usize>,
    flags: Flags,
    call_stack: Vec<usize>,
    comm_cfg: CommConfig,
    /// Set once `xawlen` (CommAnswerLen) has defined the answer-length format. After
    /// that a later `xsetpar` must NOT re-derive len_offset/len_add from its concept
    /// (some clusters, e.g. IKI, place xsetpar after xawlen and would otherwise reset
    /// the old-format LEN@1 back to the 4-byte LEN@3 and mis-parse every response).
    awlen_set: bool,
    /// Job argument buffer (EDIABAS ArgString), read via par*/pary opcodes.
    args: Vec<u8>,
}

impl Vm {
    pub fn new(transport: Box<dyn Transport>, tables: HashMap<String, Table>) -> Self {
        Self {
            regs: HashMap::new(),
            stack: Vec::new(),
            transport,
            tables,
            active_table: None,
            found_row: None,
            flags: Flags::default(),
            call_stack: Vec::new(),
            comm_cfg: CommConfig::default(),
            awlen_set: false,
            args: Vec::new(),
        }
    }

    /// Set the job argument buffer (EDIABAS ArgString) before running a job.
    pub fn set_args(&mut self, args: Vec<u8>) {
        self.args = args;
    }

    /// Return the ';'-separated argument segments (EDIABAS multi-arg convention).
    fn arg_segments(&self) -> Vec<&[u8]> {
        if self.args.is_empty() { return Vec::new(); }
        self.args.split(|&b| b == b';').collect()
    }

    /// n-th argument (1-based) parsed as an integer, else 0.
    fn arg_int(&self, n: usize) -> i32 {
        let segs = self.arg_segments();
        if n == 0 || n > segs.len() { return 0; }
        let s = String::from_utf8_lossy(segs[n - 1]);
        let t = s.trim();
        i32::from_str_radix(t.trim_start_matches("0x"), 16)
            .or_else(|_| t.parse::<i32>())
            .unwrap_or(0)
    }

    pub fn run_job(&mut self, code: &[u8]) -> Result<Vec<ResultSet>, String> {
        // Each job starts with a clean register/stack state (EDIABAS runs every
        // apiJob fresh). Transport/protocol config lives in the transport, not
        // here, so it survives across jobs in a Session.
        self.regs.clear();
        self.stack.clear();
        self.call_stack.clear();
        let mut ip = 0usize;
        let mut current: ResultSet = HashMap::new();
        let mut sets: Vec<ResultSet> = Vec::new();

        let trace = crate::trace::verbose();
        let mut steps: u64 = 0;
        while ip < code.len() {
            if code[ip] == 0xf7 { break; }

            steps += 1;
            if steps > 20_000_000 {
                trace!("vm: instruction limit exceeded at ip={ip:#06x} — aborting (possible infinite loop)");
                break;
            }

            if trace {
                let end = code.len().min(ip + 6);
                let l0 = self.regs.get(&0x18).map(Value::as_long).unwrap_or(0);
                let l1 = self.regs.get(&0x19).map(Value::as_long).unwrap_or(0);
                let l2 = self.regs.get(&0x1a).map(Value::as_long).unwrap_or(0);
                vtrace!("[trace] ip={ip:#06x} op={:#04x} : {:<17} L0={l0:#x} L1={l1:#x} L2={l2:#x} sp={}",
                          code[ip], hex(&code[ip..end]), self.stack.len());
            }

            let slice = &code[ip..];
            match slice {

                // ── eoj ── end of job (opcode 0x1d always ends). An arg0 operand
                // (mode != None) sets JOB_STATUS to that string (EDIABAS OpEoj).
                [0x1d, 0x00, ..] => break,
                [0x1d, mode, ..] => {
                    match mode >> 4 {
                        1 => {  // RegS: JOB_STATUS = string register
                            let reg = code.get(ip + 2).copied().unwrap_or(0);
                            let s = self.reg_str(reg);
                            current.insert("JOB_STATUS".into(), Value::Str(s));
                        }
                        8 => {  // ImmStr literal
                            let n = code.get(ip + 2).copied().unwrap_or(0) as usize
                                  | ((code.get(ip + 3).copied().unwrap_or(0) as usize) << 8);
                            let s = if ip + 4 + n <= code.len() { cstr(&code[ip + 4..ip + 4 + n]) }
                                    else { String::new() };
                            current.insert("JOB_STATUS".into(), Value::Str(s));
                        }
                        _ => {}
                    }
                    break;
                }

                // ── nop ──────────────────────────────────────────────────────────
                [0x1c, 0x00, ..] => { ip += 2; }

                // ── move ─────────────────────────────────────────────────────────

                // reg-to-reg copy: 01 10 DST 00 11 DST SRC (7 bytes) — before clear
                [0x01, 0x10, dst, 0x00, 0x11, dst2, src, ..] if dst == dst2 => {
                    let v = self.regs.get(src).cloned().unwrap_or(Value::Long(0));
                    self.regs.insert(*dst, v);
                    ip += 7;
                }

                // clear RegS/RegL: 01 10 R (3 bytes)
                [0x01, 0x10, reg, ..] => {
                    let zero = if *reg >= 0x18 && *reg <= 0x1b {
                        Value::Long(0)
                    } else {
                        Value::Str(String::new())
                    };
                    self.regs.insert(*reg, zero);
                    ip += 3;
                }

                // clear LReg: 01 40 XX (3 bytes)
                [0x01, 0x40, xx, ..] => {
                    self.regs.insert(*xx, Value::Long(0));
                    ip += 3;
                }

                // clear RegI: 01 30 XX (3 bytes) — mode 0x30 hi=3(RegI). I overlaps L.
                [0x01, 0x30, reg, ..] => {
                    self.write_reg_val(*reg, 0);
                    ip += 3;
                }

                // move with no operands (mode 0x00) — no-op, 2 bytes. Appears in real
                // bytecode (e.g. FS_LESEN); harmless placeholder, decodes cleanly.
                [0x00, 0x00, ..] => { ip += 2; }

                // move RegS, RegS: 00 11 R1 R2
                [0x00, 0x11, r1, r2, ..] => {
                    let v = self.regs.get(r2).cloned().unwrap_or(Value::Str(String::new()));
                    self.regs.insert(*r1, v);
                    ip += 4;
                }

                // move RegS, ImmStr: 00 18 R NN_lo NN_hi [NN]
                // A literal may be text ("ERROR_ARGUMENT") or a binary telegram prefix
                // (B8 12 F1 03 2C 10). Storing binary as a Rust String corrupts non-UTF8
                // bytes, so keep binary literals as Data (raw, byte-preserving).
                [0x00, 0x18, reg, nn_lo, nn_hi, ..] => {
                    let n = *nn_lo as usize | ((*nn_hi as usize) << 8);
                    if ip + 5 + n > code.len() {
                        return Err(format!("MOVE_STR truncated at ip={ip:#x}"));
                    }
                    let s = cstr(&code[ip + 5..ip + 5 + n]);
                    self.regs.insert(*reg, Value::Str(s));
                    ip += 5 + n;
                }

                // move RegAB, SReg[IReg]: 00 2a DST SRC IDX (5 bytes)
                [0x00, 0x2a, dst, src, idx, ..] => {
                    let idx_val = reg_val_from_map(&self.regs, *idx).max(0) as usize;
                    let data = self.reg_bytes(src);
                    let byte_val = data.get(idx_val).copied().unwrap_or(0);
                    if crate::trace::verbose() {
                        vtrace!("MOVB0 dst={:#04x} src=S{} idx={idx_val} byte={byte_val:#04x} data={:02X?}",
                                  dst, src.wrapping_sub(0x1c), data);
                    }
                    self.set_byte_reg(*dst, byte_val);
                    ip += 5;
                }

                // move [DST_REG + IDX_REG], SRC_BYTE: 00 a2 DST IDX SRC (5 bytes)
                // MODE=0xa2: upper=0xa(IdxReg dst), lower=0x2(RegAB src)
                [0x00, 0xa2, dst_reg, idx_reg, src_b, ..] => {
                    let idx = reg_val_from_map(&self.regs, *idx_reg).max(0) as usize;
                    let byte_val = self.get_byte_reg(*src_b);
                    let mut data = match self.regs.get(dst_reg) {
                        Some(Value::Data(d)) => d.clone(),
                        Some(Value::Str(s))  => unlat(s),
                        _                    => Vec::new(),
                    };
                    if data.len() <= idx { data.resize(idx + 1, 0); }
                    data[idx] = byte_val;
                    self.regs.insert(*dst_reg, Value::Data(data));
                    ip += 5;
                }

                // move [DST_REG + IDX_REG], SRC_STRING: 00 a1 DST IDX SRC (5 bytes)
                // MODE=0xa1: upper=0xa(IdxReg dst), lower=0x1(RegS src).
                // Copies the source string/data bytes into the dst buffer at byte offset idx
                // (used to splice converted selectors into a telegram being built).
                [0x00, 0xa1, dst_reg, idx_reg, src_s, ..] => {
                    let idx = reg_val_from_map(&self.regs, *idx_reg).max(0) as usize;
                    let src_bytes = self.reg_bytes(src_s);
                    let mut data = match self.regs.get(dst_reg) {
                        Some(Value::Data(d)) => d.clone(),
                        Some(Value::Str(s))  => unlat(s),
                        _                    => Vec::new(),
                    };
                    let end = idx + src_bytes.len();
                    if data.len() < end { data.resize(end, 0); }
                    data[idx..end].copy_from_slice(&src_bytes);
                    self.regs.insert(*dst_reg, Value::Data(data));
                    ip += 5;
                }

                // move [DST_REG + IMM], SRC_BYTE: 00 92 DST IMM_lo IMM_hi SRC (6 bytes)
                // MODE=0x92: upper=0x9(IdxImm dst = reg + u16 offset), lower=0x2(RegAB src)
                [0x00, 0x92, dst_reg, imm_lo, imm_hi, src_b, ..] => {
                    let idx = (*imm_lo as usize) | ((*imm_hi as usize) << 8);
                    let byte_val = self.get_byte_reg(*src_b);
                    let mut data = match self.regs.get(dst_reg) {
                        Some(Value::Data(d)) => d.clone(),
                        Some(Value::Str(s))  => unlat(s),
                        _                    => Vec::new(),
                    };
                    if data.len() <= idx { data.resize(idx + 1, 0); }
                    data[idx] = byte_val;
                    self.regs.insert(*dst_reg, Value::Data(data));
                    ip += 6;
                }

                // move RegAB, RegAB: 00 22 B1 B2
                [0x00, 0x22, dst, src, ..] => {
                    let v = self.get_byte_reg(*src);
                    self.set_byte_reg(*dst, v);
                    ip += 4;
                }

                // move LReg, #Imm32: 00 47 XX V V V V
                [0x00, 0x47, xx, v0, v1, v2, v3, ..] => {
                    let v = i32::from_le_bytes([*v0, *v1, *v2, *v3]);
                    self.regs.insert(*xx, Value::Long(v));
                    ip += 7;
                }

                // move RegAB, Imm8: 00 25 R V
                [0x00, 0x25, reg, v, ..] => {
                    self.set_byte_reg(*reg, *v);
                    ip += 4;
                }

                // move RegI, Imm16: 00 36 R V_lo V_hi  (I overlaps L — write via helper)
                [0x00, 0x36, reg, v_lo, v_hi, ..] => {
                    let v = *v_lo as i32 | ((*v_hi as i32) << 8);
                    self.write_reg_val(*reg, v);
                    ip += 5;
                }

                // move RegL, RegL: 00 44 R1 R2
                [0x00, 0x44, r1, r2, ..] => {
                    let v = self.regs.get(r2).map(Value::as_long).unwrap_or(0);
                    self.regs.insert(*r1, Value::Long(v));
                    ip += 4;
                }

                // move RegL, Imm8: 00 45 R V
                [0x00, 0x45, reg, v, ..] => {
                    self.regs.insert(*reg, Value::Long(*v as i32));
                    ip += 4;
                }

                // move <numreg>, S[idx] — read dest-width bytes little-endian from an
                // S-register at an immediate (IdxImm) or register (IdxReg) index.
                // EDIABAS OpMove with indexed byte[] source; len = dest register width.
                [0x00, mode, ..]
                    if matches!(mode >> 4, 2 | 3 | 4)
                       && matches!(mode & 0xf, 0x9 | 0xa)
                       && ip + 4 < code.len() =>
                {
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let width = nib_width(hi);
                    let dst = code[ip + 2];
                    let base = code[ip + 3];
                    let (offset, next) = if lo == 0x9 {
                        let lo_b = code.get(ip + 4).copied().unwrap_or(0);
                        let hi_b = code.get(ip + 5).copied().unwrap_or(0);
                        (u16::from_le_bytes([lo_b, hi_b]) as usize, ip + 6)
                    } else {
                        let ir = code.get(ip + 4).copied().unwrap_or(0);
                        (reg_val_from_map(&self.regs, ir).max(0) as usize, ip + 5)
                    };
                    let value = self.read_sreg_value(base, offset, width);
                    if crate::trace::verbose() {
                        let src = self.reg_bytes(&base);
                        vtrace!("MOVEIDX dst_reg={:#04x} base=S{} off={offset} w={width} val={value:#x} src={:02X?}",
                                  dst, base.wrapping_sub(0x1c), src);
                    }
                    self.write_num_reg(hi, dst, value);
                    self.flags.carry = false;
                    self.flags.overflow = false;
                    self.set_flags_wl(value, width);
                    ip = next;
                }

                // move RegS, S[idx:len] — copy a byte slice from a source S-register
                // (indexed-with-length modes) into a destination S-register.
                // EDIABAS OpMove byte[] ← byte[]. Used to extract value bytes.
                [0x00, mode, ..]
                    if (mode >> 4) == 1
                       && matches!(mode & 0xf, 0xc | 0xd | 0xe | 0xf)
                       && ip + 3 < code.len() =>
                {
                    let lo = mode & 0xf;
                    let dst = code[ip + 2];
                    if let Some((base, idx, len, next)) = self.read_slice_operand(lo, code, ip + 3) {
                        let src = self.reg_bytes(&base);
                        let slice: Vec<u8> = (0..len).map(|i| src.get(idx + i).copied().unwrap_or(0)).collect();
                        if crate::trace::verbose() {
                            vtrace!("SLICE dst=S{} base=S{} idx={idx} len={len} src_len={} slice={:02X?}",
                                      dst.wrapping_sub(0x1c), base.wrapping_sub(0x1c), src.len(), slice);
                        }
                        self.regs.insert(dst, Value::Data(slice));
                        self.flags.carry = false;
                        self.flags.zero = false;
                        self.flags.minus = false;
                        self.flags.overflow = false;
                        ip = next;
                    } else {
                        ip = skip_instr(code, ip);
                    }
                }

                // move generic (fallthrough for other 00 XX encodings)

                // ── comp (02): compare dst-src, set flags; never writes any reg ──
                [0x02, ..] if ip + 1 < code.len() => {
                    let (_, _, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let result = dst_val.wrapping_sub(src_val);
                    self.flags.zero     = result == 0;
                    self.flags.minus    = result < 0;
                    self.flags.carry    = (dst_val as u32) < (src_val as u32);
                    self.flags.overflow = ((dst_val ^ src_val) & (dst_val ^ result)) < 0;
                    ip = next;
                }

                // ── subb (03): dst -= src; set flags ─────────────────────────
                [0x03, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let result = dst_val.wrapping_sub(src_val);
                    self.flags.zero     = result == 0;
                    self.flags.minus    = result < 0;
                    self.flags.carry    = (dst_val as u32) < (src_val as u32);
                    self.flags.overflow = ((dst_val ^ src_val) & (dst_val ^ result)) < 0;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // adds [BaseReg+IdxReg], ByteReg: 04 a2 base idx src (MODE=0xa2)
                [0x04, 0xa2, base, idx_reg, src_b, ..] => {
                    let idx = reg_val_from_map(&self.regs, *idx_reg).max(0) as usize;
                    let src_byte = self.get_byte_reg(*src_b);
                    let mut data = match self.regs.get(base) {
                        Some(Value::Data(d)) => d.clone(),
                        Some(Value::Str(s))  => unlat(s),
                        _                    => Vec::new(),
                    };
                    if data.len() <= idx { data.resize(idx + 1, 0); }
                    data[idx] = data[idx].wrapping_add(src_byte);
                    self.regs.insert(*base, Value::Data(data));
                    ip += 5;
                }

                // ── adds (04): dst += src; set flags ─────────────────────────
                [0x04, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let (result, carry) = (dst_val as u32).overflowing_add(src_val as u32);
                    let result = result as i32;
                    self.flags.zero     = result == 0;
                    self.flags.minus    = result < 0;
                    self.flags.carry    = carry;
                    self.flags.overflow = ((!(dst_val ^ src_val)) & (dst_val ^ result)) < 0;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // ── mult (05): dst *= src ─────────────────────────────────────
                [0x05, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let result = dst_val.wrapping_mul(src_val);
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // ── divs (06): dst /= src ─────────────────────────────────────
                [0x06, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let result = if src_val != 0 { dst_val.wrapping_div(src_val) } else { 0 };
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // ── and (07): dst &= src ──────────────────────────────────────
                [0x07, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let result = dst_val & src_val;
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.flags.carry = false; self.flags.overflow = false;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // ── or (08): dst |= src ───────────────────────────────────────
                [0x08, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let result = dst_val | src_val;
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.flags.carry = false; self.flags.overflow = false;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // ── xor (09): dst ^= src ──────────────────────────────────────
                [0x09, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let result = dst_val ^ src_val;
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.flags.carry = false; self.flags.overflow = false;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // ── not (0a): dst = ~dst ──────────────────────────────────────
                [0x0a, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let pos = ip + 2;
                    if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos];
                        let v = self.regs.get(&r).map(Value::as_long).unwrap_or(0);
                        let result = !v;
                        self.regs.insert(r, Value::Long(result));
                        self.flags.zero = result == 0; self.flags.minus = result < 0;
                        ip = pos + 1;
                    } else {
                        ip = skip_instr(code, ip);
                    }
                }

                // ── jumps ─────────────────────────────────────────────────────

                // jmp unconditional: 0b 70 OFF OFF OFF OFF
                [0x0b, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    ip = self.jump_target(ip, 6, off, code.len())?;
                }

                // jtsr (call subroutine): 0c 70 OFF OFF OFF OFF
                [0x0c, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    self.call_stack.push(ip + 6);
                    ip = self.jump_target(ip, 6, off, code.len())?;
                }

                // ret (return): 0d 00
                [0x0d, 0x00, ..] => {
                    if let Some(ret_ip) = self.call_stack.pop() {
                        ip = ret_ip;
                    } else {
                        break; // ret with empty call stack = end job
                    }
                }

                // jc — jump if carry: 0e 70 OFF...
                [0x0e, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    if self.flags.carry { ip = self.jump_target(ip, 6, off, code.len())?; }
                    else { ip += 6; }
                }

                // jae — jump if above or equal (no carry): 0f 70 OFF...
                [0x0f, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    if !self.flags.carry { ip = self.jump_target(ip, 6, off, code.len())?; }
                    else { ip += 6; }
                }

                // jz: 10 70 OFF...
                [0x10, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    if self.flags.zero { ip = self.jump_target(ip, 6, off, code.len())?; }
                    else { ip += 6; }
                }

                // jnz: 11 70 OFF...
                [0x11, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    let taken = !self.flags.zero;
                    ip = if taken { self.jump_target(ip, 6, off, code.len())? } else { ip + 6 };
                }

                // jv — jump if overflow: 12 70 OFF...
                [0x12, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    if self.flags.overflow { ip = self.jump_target(ip, 6, off, code.len())?; }
                    else { ip += 6; }
                }

                // jnv — jump if no overflow: 13 70 OFF...
                [0x13, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    if !self.flags.overflow { ip = self.jump_target(ip, 6, off, code.len())?; }
                    else { ip += 6; }
                }

                // jmi — jump if minus flag set: 14 70 OFF...
                [0x14, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    if self.flags.minus { ip = self.jump_target(ip, 6, off, code.len())?; }
                    else { ip += 6; }
                }

                // jpl — jump if plus (not minus): 15 70 OFF...
                [0x15, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    if !self.flags.minus { ip = self.jump_target(ip, 6, off, code.len())?; }
                    else { ip += 6; }
                }

                // clrc: 16 00
                [0x16, 0x00, ..] => { self.flags.carry = false; ip += 2; }

                // setc: 17 00
                [0x17, 0x00, ..] => { self.flags.carry = true; ip += 2; }

                // clrv: 4c 00
                [0x4c, 0x00, ..] => { self.flags.overflow = false; ip += 2; }

                // asr — arithmetic shift right: 18 XX R shift
                [0x18, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let shift = (src_val & 31) as u32;
                    let result = dst_val >> shift;
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // lsl — logical shift left: 19 XX R shift
                [0x19, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let shift = (src_val & 31) as u32;
                    let result = if hi == 2 {
                        // byte register: shift within u8 range
                        ((dst_val as u8) << (shift as u8)) as i32
                    } else { dst_val << shift };
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // lsr — logical shift right: 1a XX R shift
                [0x1a, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let shift = (src_val & 31) as u32;
                    let result = ((dst_val as u32) >> shift) as i32;
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // asl — arithmetic shift left (same as lsl): 1b XX R shift
                [0x1b, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let shift = (src_val & 31) as u32;
                    let result = dst_val << shift;
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // addc — add with carry: 49
                [0x49, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let c = self.flags.carry as i32;
                    let result = dst_val.wrapping_add(src_val).wrapping_add(c);
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // subc — subtract with carry: 4a
                [0x4a, ..] if ip + 1 < code.len() => {
                    let (hi, dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let c = self.flags.carry as i32;
                    let result = dst_val.wrapping_sub(src_val).wrapping_sub(c);
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.alu2_write(hi, dst, result);
                    ip = next;
                }

                // test — AND but only set flags (6a)
                [0x6a, ..] if ip + 1 < code.len() => {
                    let (_, _dst, dst_val, src_val, next) = self.alu2_typed(code, ip);
                    let result = dst_val & src_val;
                    self.flags.zero = result == 0; self.flags.minus = result < 0;
                    self.flags.carry = false; self.flags.overflow = false;
                    ip = next;
                }

                // ── signed/unsigned conditional jumps ─────────────────────────

                // jg — jump if greater (signed, M==V && !Z): 5a 70 OFF...
                [0x5a, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    let cond = !self.flags.zero && (self.flags.minus == self.flags.overflow);
                    if cond { ip = self.jump_target(ip, 6, off, code.len())?; } else { ip += 6; }
                }

                // jge — jump if greater or equal (signed): 5b 70 OFF...
                [0x5b, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    let cond = self.flags.minus == self.flags.overflow;
                    if cond { ip = self.jump_target(ip, 6, off, code.len())?; } else { ip += 6; }
                }

                // jl — jump if less (signed): 5c 70 OFF...
                [0x5c, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    let cond = self.flags.minus != self.flags.overflow;
                    if cond { ip = self.jump_target(ip, 6, off, code.len())?; } else { ip += 6; }
                }

                // jle — jump if less or equal (signed): 5d 70 OFF...
                [0x5d, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    let cond = self.flags.zero || (self.flags.minus != self.flags.overflow);
                    if cond { ip = self.jump_target(ip, 6, off, code.len())?; } else { ip += 6; }
                }

                // ja — jump if above (unsigned, !C && !Z): 5e 70 OFF...
                [0x5e, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    let cond = !self.flags.carry && !self.flags.zero;
                    if cond { ip = self.jump_target(ip, 6, off, code.len())?; } else { ip += 6; }
                }

                // jbe — jump if below or equal (unsigned, C || Z): 5f 70 OFF...
                [0x5f, 0x70, o0, o1, o2, o3, ..] => {
                    let off = i32::from_le_bytes([*o0, *o1, *o2, *o3]);
                    let cond = self.flags.carry || self.flags.zero;
                    if cond { ip = self.jump_target(ip, 6, off, code.len())?; } else { ip += 6; }
                }

                // jt/jnt — jump if timer set/not-set (no timer; treat as not-taken / taken)
                [0x47, 0x70, _, _, _, _, ..] => { ip += 6; }   // jt
                [0x48, 0x70, _, _, _, _, ..] => {               // jnt
                    let off = i32::from_le_bytes([code[ip+2], code[ip+3], code[ip+4], code[ip+5]]);
                    ip = self.jump_target(ip, 6, off, code.len())?;
                }

                // etag — end-tag (conditional jump, treat as no-op skip): 41 70 OFF...
                [0x41, 0x70, _, _, _, _, ..] => { ip += 6; }

                // ── push / pop ────────────────────────────────────────────────

                // push arg0 — width-aware byte push (EDIABAS OpPush).
                // Pushes `width` bytes little-endian; width = operand data length.
                [0x1e, ..] if ip + 1 < code.len() => {
                    let hi = code[ip + 1] >> 4;
                    let (val, next) = self.read_typed_operand(hi, code, ip + 2);
                    let width = nib_width(hi);
                    if width > 0 { self.stack_push_bytes(val as i64, width); }
                    ip = next;
                }

                // pop → reg — width-aware byte pop (EDIABAS OpPop).
                // Pops `width` bytes (width = destination register width), sets flags.
                [0x1f, ..] if ip + 2 < code.len() => {
                    let hi = code[ip + 1] >> 4;
                    let width = nib_width(hi);
                    if width > 0 {
                        let reg = code[ip + 2];
                        let value = self.stack_pop_bytes(width);
                        self.write_num_reg(hi, reg, value);
                        self.flags.overflow = false;
                        self.set_flags_wl(value, width);
                        ip += 3;
                    } else {
                        ip = skip_instr(code, ip);
                    }
                }

                // popf / pushf — flags stack (no-op, 2 bytes)
                [0x4e, 0x00, ..] => { ip += 2; }
                [0x4f, 0x00, ..] => { ip += 2; }

                // swap — swap top two stack items
                [0x51, 0x00, ..] => {
                    let len = self.stack.len();
                    if len >= 2 { self.stack.swap(len - 1, len - 2); }
                    ip += 2;
                }

                // ── string operations ─────────────────────────────────────────

                // scmp — compare strings, FLAGS ONLY (like `comp`): 20 11 R1 R2.
                // Must NOT write a register — the result value variant is `strcmp` (0x8f).
                // Writing L0 here clobbers a read offset held in I0/L0 across the compare
                // (broke STATUS_* jobs: the DATA_TYPE switch reset the response byte index).
                [0x20, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let (s1, p1) = read_str_at(&self.regs, hi, code, pos);
                    pos = p1;
                    let (s2, p2) = read_str_at(&self.regs, lo, code, pos);
                    let result = s1.cmp(&s2) as i32;
                    self.flags.zero  = result == 0;
                    self.flags.minus = result < 0;
                    self.flags.carry = result < 0;
                    ip = p2;
                }

                // scat — append R2 to R1: 21 11 R1 R2
                [0x21, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let r1 = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (s2, p2) = read_str_at(&self.regs, lo, code, pos);
                    if r1 != 0xff {
                        let s1 = self.reg_str(r1);
                        self.regs.insert(r1, Value::Str(s1 + &s2));
                    }
                    ip = p2;
                }

                // scut — cut string R1 to first N chars (N from R2/imm): 22 XX R1 R2
                [0x22, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let r1 = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (n_val, p2) = read_long_at(&self.regs, lo, code, pos);
                    if r1 != 0xff {
                        let s = self.reg_str(r1);
                        let n = n_val.max(0) as usize;
                        // EDIABAS OpScut removes `n` bytes from the END; the stored
                        // byte array is null-terminated so its length is chars+1.
                        // Keep the first (chars + 1 - n) characters.
                        let total = s.chars().count();
                        let keep = (total + 1).saturating_sub(n).min(total);
                        let cut: String = s.chars().take(keep).collect();
                        self.regs.insert(r1, Value::Str(cut));
                    }
                    ip = p2;
                }

                // slen — string length → L0: 23 XX RS  (MODE upper=RegS/RegL dst, lower=src)
                [0x23, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    // dst register (for result length) — skip it
                    if (1..=4).contains(&hi) && pos < code.len() { pos += 1; }
                    let (s, p2) = read_str_at(&self.regs, lo, code, pos);
                    // Latin-1 strings store each logical byte as one char; use the
                    // char count, not the UTF-8 byte length (`s.len()`), which would
                    // over-count bytes ≥ 0x80 that encode as 2 UTF-8 bytes.
                    let len = s.chars().count() as i32;
                    self.regs.insert(REG_L0, Value::Long(len));
                    ip = p2;
                }

                // spaste — paste src string into dst at a position: 24 XX DST SRC.
                // DST may be a plain RegS (position from L0) or an indexed operand
                // S[idx] / S[reg] (position from the index). Consume operand bytes
                // by mode so the instruction stream never desyncs.
                [0x24, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let g = |p: usize| code.get(p).copied().unwrap_or(0);
                    let (r1, index): (u8, Option<usize>) = match hi {
                        1..=4 => { let r = g(pos); pos += 1; (r, None) }
                        0x9 => { // IdxImm: base + u16 idx
                            let r = g(pos);
                            let idx = u16::from_le_bytes([g(pos + 1), g(pos + 2)]) as usize;
                            pos += 3; (r, Some(idx))
                        }
                        0xa => { // IdxReg: base + reg idx
                            let r = g(pos);
                            let idx = reg_val_from_map(&self.regs, g(pos + 1)).max(0) as usize;
                            pos += 2; (r, Some(idx))
                        }
                        _ => { pos += nibble_size(hi, code, pos); (0xff, None) }
                    };
                    let (s2, p2) = read_str_at(&self.regs, lo, code, pos);
                    if r1 != 0xff {
                        let mut s1 = self.reg_str(r1);
                        let insert_pos = index.unwrap_or_else(|| {
                            self.regs.get(&REG_L0).map(Value::as_long).unwrap_or(0).max(0) as usize
                        });
                        if insert_pos >= s1.len() { s1.push_str(&s2); }
                        else { s1.insert_str(insert_pos, &s2); }
                        self.regs.insert(r1, Value::Str(s1));
                    }
                    ip = p2;
                }

                // serase — erase N chars from S1 at position: 25 XX R1 R2
                [0x25, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let r1 = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (n_val, p2) = read_long_at(&self.regs, lo, code, pos);
                    if r1 != 0xff {
                        let mut s = self.reg_str(r1);
                        let erase_pos = self.regs.get(&REG_L0)
                            .map(Value::as_long).unwrap_or(0).max(0) as usize;
                        let erase_len = n_val.max(0) as usize;
                        if erase_pos < s.len() {
                            let end = (erase_pos + erase_len).min(s.len());
                            s.drain(erase_pos..end);
                        }
                        self.regs.insert(r1, Value::Str(s));
                    }
                    ip = p2;
                }

                // strcat — same as scat but opcode 7e
                [0x7e, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let r1 = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (s2, p2) = read_str_at(&self.regs, lo, code, pos);
                    if r1 != 0xff {
                        let s1 = self.reg_str(r1);
                        self.regs.insert(r1, Value::Str(s1 + &s2));
                    }
                    ip = p2;
                }

                // strcmp (8f) — compare strings → L0 (same as scmp but no flags)
                [0x8f, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let (s1, p1) = read_str_at(&self.regs, hi, code, pos); pos = p1;
                    let (s2, p2) = read_str_at(&self.regs, lo, code, pos);
                    let result = s1.cmp(&s2) as i32;
                    self.regs.insert(REG_L0, Value::Long(result));
                    ip = p2;
                }

                // strlen (90) — string length → L0
                [0x90, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    if (1..=4).contains(&hi) && pos < code.len() { pos += 1; }
                    let (s, p2) = read_str_at(&self.regs, lo, code, pos);
                    self.regs.insert(REG_L0, Value::Long(s.len() as i32));
                    ip = p2;
                }

                // stoken (54) — extract token from string
                [0x54, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let r1 = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (delim, p2) = read_str_at(&self.regs, lo, code, pos);
                    if r1 != 0xff {
                        let s = self.reg_str(r1);
                        let delim_char = delim.chars().next().unwrap_or(' ');
                        if let Some(idx) = s.find(delim_char) {
                            self.regs.insert(r1, Value::Str(s[..idx].to_string()));
                        }
                    }
                    ip = p2;
                }

                // srevrs (53) — reverse string
                [0x53, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let pos = ip + 2;
                    if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos];
                        let s: String = self.reg_str(r).chars().rev().collect();
                        self.regs.insert(r, Value::Str(s));
                        ip = pos + 1;
                    } else { ip = skip_instr(code, ip); }
                }

                // setspc (52) — set space character (no-op, affects print formatting)
                [0x52, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // ssize (b5) — string size → L0
                [0xb5, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    if (1..=4).contains(&hi) && pos < code.len() { pos += 1; }
                    let (s, p2) = read_str_at(&self.regs, lo, code, pos);
                    self.regs.insert(REG_L0, Value::Long(s.len() as i32));
                    ip = p2;
                }

                // ── conversions ───────────────────────────────────────────────

                // a2fix (67) — ASCII string → integer register: handles "0x1F" and decimal
                [0x67, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let dst = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (s, p2) = read_str_at(&self.regs, lo, code, pos);
                    let s = s.trim();
                    let v: i32 = if s.starts_with("0x") || s.starts_with("0X") {
                        i32::from_str_radix(&s[2..], 16).unwrap_or(0)
                    } else {
                        s.parse().unwrap_or(0)
                    };
                    // Honour the B/I→L register overlap for numeric dst (e.g. tabget
                    // into I1); write_reg_val stores S/L regs directly as before.
                    if dst != 0xff { self.write_reg_val(dst, v); }
                    ip = p2;
                }

                // fix2hex (79) — integer → hex string: 79 14 R_S R_L
                [0x79, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let dst = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (v, p2) = read_long_at(&self.regs, lo, code, pos);
                    if dst != 0xff { self.regs.insert(dst, Value::Str(format!("{v:X}"))); }
                    ip = p2;
                }

                // fix2dez (7a) — integer → decimal string: 7a 14 R_S R_L
                [0x7a, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let dst = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (v, p2) = read_long_at(&self.regs, lo, code, pos);
                    if dst != 0xff { self.regs.insert(dst, Value::Str(format!("{v}"))); }
                    ip = p2;
                }

                // ufix2dez (ab) — unsigned integer → decimal string
                [0xab, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let dst = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (v, p2) = read_long_at(&self.regs, lo, code, pos);
                    if dst != 0xff { self.regs.insert(dst, Value::Str(format!("{}", v as u32))); }
                    ip = p2;
                }

                // a2flt (3a) — ASCII/reg → float; fix2flt (68) — int → float.
                // Both: dst_float_reg = src_as_f64 (read_flt_src handles Str/int/ImmStr).
                [0x3a, ..] | [0x68, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let (hi, lo) = (mode >> 4, mode & 0xf);
                    let pos0 = ip + 2;
                    let l0 = nibble_size(hi, code, pos0);
                    let dst = code[pos0];
                    let (val, l1) = self.read_flt_src(code, pos0 + l0, lo);
                    self.regs.insert(dst, Value::Float(val));
                    self.flags.zero = val == 0.0;
                    self.flags.minus = val < 0.0;
                    ip += 2 + l0 + l1;
                }

                // flt2a (87) — float → ASCII string
                [0x87, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let (hi, lo) = (mode >> 4, mode & 0xf);
                    let pos0 = ip + 2;
                    let l0 = nibble_size(hi, code, pos0);
                    let dst = code[pos0];
                    let (val, l1) = self.read_flt_src(code, pos0 + l0, lo);
                    self.regs.insert(dst, Value::Str(fmt_flt(val)));
                    ip += 2 + l0 + l1;
                }

                // flt2fix (96) — float → integer (rounded), into an int/long reg
                [0x96, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let (hi, lo) = (mode >> 4, mode & 0xf);
                    let pos0 = ip + 2;
                    let l0 = nibble_size(hi, code, pos0);
                    let dst = code[pos0];
                    let (val, l1) = self.read_flt_src(code, pos0 + l0, lo);
                    let iv = val.round() as i32;
                    self.regs.insert(dst, Value::Long(iv));
                    self.flags.zero = iv == 0;
                    self.flags.minus = iv < 0;
                    ip += 2 + l0 + l1;
                }

                // ── binary ↔ hex/string conversions ──────────────────────────

                // hex2y (8e) — hex string → binary byte array: 8e XX DST SRC
                [0x8e, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let dst = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (s, p2) = read_str_at(&self.regs, lo, code, pos);
                    let bytes = parse_hex_str(&s);
                    if dst != 0xff { self.regs.insert(dst, Value::Data(bytes)); }
                    ip = p2;
                }

                // a2y (8c) — ASCII (decimal) → binary byte array
                [0x8c, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let dst = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (s, p2) = read_str_at(&self.regs, lo, code, pos);
                    let bytes = parse_hex_str(&s);
                    if dst != 0xff { self.regs.insert(dst, Value::Data(bytes)); }
                    ip = p2;
                }

                // y2hex (92) — binary → hex string: 92 XX DST SRC
                [0x92, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4; let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let dst = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (s, p2) = read_str_at(&self.regs, lo, code, pos);
                    let hex_s = s.bytes().map(|b| format!("{b:02X}")).collect::<String>();
                    if dst != 0xff { self.regs.insert(dst, Value::Str(hex_s)); }
                    ip = p2;
                }

                // y2bcd (91) — binary → BCD (no-op for now)
                [0x91, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // ── result emission (erg*) ────────────────────────────────────

                // ergb (34) — emit byte result: 34 82 NN 00 NAME\0 R_AB
                [0x34, 0x82, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 4 + n + 1;
                    if ip + total <= code.len() {
                        let name = cstr(&code[ip + 4..ip + 4 + n]);
                        let reg = code[ip + 4 + n];
                        let v = self.get_byte_reg(reg) as i32;
                        current.insert(name, Value::Long(v));
                    }
                    ip += 4 + n + 1;
                }

                // ergw (35) — emit word result: 35 83 NN 00 NAME\0 R_I
                [0x35, 0x83, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 4 + n + 1;
                    if ip + total <= code.len() {
                        let name = cstr(&code[ip + 4..ip + 4 + n]);
                        let reg = code[ip + 4 + n];
                        // Honour the B/I → L register overlap: I/B regs have no own key.
                        let v = reg_val_from_map(&self.regs, reg) & 0xffff;
                        current.insert(name, Value::Long(v));
                    }
                    ip += 4 + n + 1;
                }

                // ergd (36) — emit dword result: 36 84 NN 00 NAME\0 R_L
                [0x36, 0x84, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 4 + n + 1;
                    if ip + total <= code.len() {
                        let name = cstr(&code[ip + 4..ip + 4 + n]);
                        let reg = code[ip + 4 + n];
                        let v = reg_val_from_map(&self.regs, reg);
                        current.insert(name, Value::Long(v));
                    }
                    ip += 4 + n + 1;
                }

                // ergi (37) — emit integer result: 37 83 NN 00 NAME\0 R
                [0x37, 0x83, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 4 + n + 1;
                    if ip + total <= code.len() {
                        let name = cstr(&code[ip + 4..ip + 4 + n]);
                        let reg = code[ip + 4 + n];
                        let v = reg_val_from_map(&self.regs, reg);
                        current.insert(name, Value::Long(v));
                    }
                    ip += total;
                }

                // ergr (38) — emit real/float result (treat as string)
                [0x38, 0x81, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 4 + n + 1;
                    if ip + total <= code.len() {
                        let name = cstr(&code[ip + 4..ip + 4 + n]);
                        let src_reg = code[ip + 4 + n];
                        current.insert(name, Value::Str(self.reg_str(src_reg)));
                    }
                    ip += total;
                }

                // ergs (39) — emit string result: 39 81 NN 00 NAME\0 R
                [0x39, 0x81, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 4 + n + 1;
                    if ip + total <= code.len() {
                        let name = cstr(&code[ip + 4..ip + 4 + n]);
                        let src_reg = code[ip + 4 + n];
                        current.insert(name, Value::Str(self.reg_str(src_reg)));
                    }
                    ip += total;
                }

                // ergy (3f) — emit y (data/byte-array) result
                [0x3f, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    if mode == 0x81 && ip + 4 < code.len() {
                        let n = code[ip + 2] as usize;
                        let total = 4 + n + 1;
                        if ip + total <= code.len() {
                            let name = cstr(&code[ip + 4..ip + 4 + n]);
                            let reg = code[ip + 4 + n];
                            let data = self.reg_bytes(&reg);
                            current.insert(name, Value::Data(data));
                        }
                        ip += total;
                    } else { ip = skip_instr(code, ip); }
                }

                // ergc (81) — emit character result (as string)
                [0x81, 0x81, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 4 + n + 1;
                    if ip + total <= code.len() {
                        let name = cstr(&code[ip + 4..ip + 4 + n]);
                        let reg = code[ip + 4 + n];
                        current.insert(name, Value::Str(self.reg_str(reg)));
                    }
                    ip += total;
                }

                // ergl (82) — emit long result: 82 84 NN 00 NAME\0 R
                [0x82, 0x84, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 4 + n + 1;
                    if ip + total <= code.len() {
                        let name = cstr(&code[ip + 4..ip + 4 + n]);
                        let reg = code[ip + 4 + n];
                        let v = reg_val_from_map(&self.regs, reg);
                        current.insert(name, Value::Long(v));
                    }
                    ip += total;
                }

                // ergsysi (95) — emit system info string (no-op; emit empty)
                [0x95, 0x81, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 4 + n + 1;
                    if ip + total <= code.len() {
                        let name = cstr(&code[ip + 4..ip + 4 + n]);
                        current.insert(name, Value::Str(String::new()));
                    }
                    ip += total;
                }

                // erg* with a DYNAMIC (register) result name — mode hi=1 (RegS name).
                // EDIABAS OpErg*: arg0 = result name string, arg1 = value. Covers
                // e.g. `ergr S6, S10` / `ergs S7, S5` used by MW_SELECT_LESEN_NORM.
                [op @ (0x34 | 0x35 | 0x36 | 0x37 | 0x38 | 0x39 | 0x81 | 0x82), mode, ..]
                    if (mode >> 4) == 1 && ip + 2 < code.len() =>
                {
                    let lo = mode & 0xf;
                    let name = self.reg_str(code[ip + 2]);
                    let vpos = ip + 3;
                    let (value, next) = match op {
                        0x39 | 0x81 | 0x38 => { // ergs / ergc / ergr → string value
                            let (s, n) = read_str_at(&self.regs, lo, code, vpos);
                            (Value::Str(s), n)
                        }
                        _ => { // ergb / ergw / ergd / ergi / ergl → integer value
                            let (v, n) = read_long_at(&self.regs, lo, code, vpos);
                            (Value::Long(v), n)
                        }
                    };
                    if !name.is_empty() { current.insert(name, value); }
                    ip = next;
                }

                // ── result sets ───────────────────────────────────────────────

                // enewset (40) — commit current result set, start new.
                // SPECIAL ENCODING: always 2 bytes; the 2nd byte is NOT a mode byte
                // (verified across corpus — see BEST2-DECODER.md). Any 0x40 form commits.
                [0x40, ..] if ip + 1 < code.len() => {
                    sets.push(std::mem::take(&mut current));
                    ip += 2;
                }

                // ── table operations ──────────────────────────────────────────

                // tabset (7b): 7b 80 NN 00 NAME\0
                [0x7b, 0x80, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    if ip + 4 + n <= code.len() {
                        let tname = cstr(&code[ip + 4..ip + 4 + n]);
                        self.active_table = Some(tname.to_uppercase());
                        self.found_row = None;
                    }
                    ip += 4 + n;
                }

                // tabseek (7c): 7c 81 NN 00 NAME\0 R
                [0x7c, 0x81, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 5 + n;
                    if ip + total <= code.len() {
                        let col_name = cstr(&code[ip + 4..ip + 4 + n]);
                        let val_reg  = code[ip + 4 + n];
                        let search_val = self.reg_str(val_reg);
                        let found = self.active_table.as_ref()
                            .and_then(|tname| self.tables.get(tname.as_str()))
                            .and_then(|t| {
                                let ci = t.col_index(&col_name)?;
                                t.find_row(ci, &search_val)
                            });
                        if crate::trace::enabled() {
                            let sample: Vec<String> = self.active_table.as_ref()
                                .and_then(|tn| self.tables.get(tn.as_str()))
                                .map(|t| {
                                    let ci = t.col_index(&col_name);
                                    t.rows.iter().take(6)
                                        .map(|r| ci.and_then(|c| r.get(c)).cloned().unwrap_or_default())
                                        .collect()
                                }).unwrap_or_default();
                            trace!("TABSEEK tab={:?} col={col_name} key={search_val:?} found={found:?} adr_sample={sample:?}",
                                      self.active_table);
                        }
                        self.found_row = found;
                        self.regs.insert(REG_L0, Value::Long(if found.is_some() { 1 } else { 0 }));
                    }
                    ip += 5 + n;
                }

                // tabseeku (9a) — same as tabseek (unsigned, no-op distinction)
                [0x9a, 0x81, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    let total = 5 + n;
                    if ip + total <= code.len() {
                        let col_name = cstr(&code[ip + 4..ip + 4 + n]);
                        let val_reg  = code[ip + 4 + n];
                        let search_val = self.reg_str(val_reg);
                        let found = self.active_table.as_ref()
                            .and_then(|tname| self.tables.get(tname.as_str()))
                            .and_then(|t| {
                                let ci = t.col_index(&col_name)?;
                                t.find_row(ci, &search_val)
                            });
                        self.found_row = found;
                        self.regs.insert(REG_L0, Value::Long(if found.is_some() { 1 } else { 0 }));
                    }
                    ip += 5 + n;
                }

                // tabget (7d): 7d 18 DST NN 00 NAME\0
                [0x7d, 0x18, dst, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    if ip + 5 + n <= code.len() {
                        let col_name = cstr(&code[ip + 5..ip + 5 + n]);
                        let val = self.active_table.as_ref()
                            .and_then(|tname| self.tables.get(tname.as_str()))
                            .and_then(|t| {
                                let ci = t.col_index(&col_name)?;
                                let ri = self.found_row?;
                                Some(t.cell(ri, ci).to_string())
                            })
                            .unwrap_or_default();
                        if crate::trace::enabled() {
                            trace!("TABGET [{col_name}] → {val:?}");
                        }
                        self.regs.insert(*dst, Value::Str(val));
                    }
                    ip += 5 + n;
                }

                // tabline (83) — get current row number → L0
                [0x83, 0x00, ..] => {
                    let row = self.found_row.unwrap_or(0) as i32;
                    self.regs.insert(REG_L0, Value::Long(row));
                    ip += 2;
                }

                // tabcols (b6) — number of columns → L0
                [0xb6, 0x00, ..] => {
                    let n = self.active_table.as_ref()
                        .and_then(|tname| self.tables.get(tname.as_str()))
                        .map(|t| t.columns.len())
                        .unwrap_or(0) as i32;
                    self.regs.insert(REG_L0, Value::Long(n));
                    ip += 2;
                }

                // tabrows (b7) — number of rows → L0
                [0xb7, 0x00, ..] => {
                    let n = self.active_table.as_ref()
                        .and_then(|tname| self.tables.get(tname.as_str()))
                        .map(|t| t.rows.len())
                        .unwrap_or(0) as i32;
                    self.regs.insert(REG_L0, Value::Long(n));
                    ip += 2;
                }

                // tabsetex (aa) — extended tabset (same as tabset but with extra args)
                [0xaa, 0x80, nn, 0x00, ..] => {
                    let n = *nn as usize;
                    if ip + 4 + n <= code.len() {
                        let tname = cstr(&code[ip + 4..ip + 4 + n]);
                        self.active_table = Some(tname.to_uppercase());
                        self.found_row = None;
                    }
                    ip += 4 + n;
                }

                // ── transport ─────────────────────────────────────────────────

                // xsend: 2a 18 DST NN_lo NN_hi [NN]
                [0x2a, 0x18, dst_reg, nn_lo, nn_hi, ..] => {
                    let n = *nn_lo as usize | ((*nn_hi as usize) << 8);
                    if ip + 5 + n > code.len() {
                        return Err(format!("XSEND truncated at ip={ip:#x}"));
                    }
                    let telegram = &code[ip + 5..ip + 5 + n];
                    let response = self.transport.exchange(telegram)
                        .map_err(|e| format!("xsend failed: {e}"))?;
                    self.regs.insert(*dst_reg, Value::Data(response));
                    ip += 5 + n;
                }

                // xsend with register source: 2a 11 DST SRC (send string reg as binary)
                [0x2a, 0x11, dst, src, ..] => {
                    let data = self.reg_bytes(src);
                    trace!("XSEND telegram ({} bytes): {}", data.len(),
                        data.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "));
                    let response = self.transport.exchange(&data)
                        .map_err(|e| format!("xsend failed: {e}"))?;
                    trace!("XSEND response ({} bytes): {}", response.len(),
                        response.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "));
                    self.regs.insert(*dst, Value::Data(response));
                    ip += 4;
                }

                // xsend with data register: 2a 14 DST SRC
                [0x2a, 0x14, dst, src, ..] => {
                    let data = self.reg_bytes(src);
                    let response = self.transport.exchange(&data)
                        .map_err(|e| format!("xsend failed: {e}"))?;
                    self.regs.insert(*dst, Value::Data(response));
                    ip += 4;
                }

                // xsendf (2b) / xrequf (2c) / xraw (75) — raw send variants
                [0x2b, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x2c, ..] if ip + 1 < code.len() => {
                    let next = skip_instr(code, ip);
                    trace!("[xrequf@{ip:#x}] raw bytes: {}",
                        code[ip..next].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "));
                    ip = next;
                }
                [0x75, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // xrecv init (2e): 2e 10 1e
                [0x2e, 0x10, 0x1e, ..] => { ip += 3; }
                [0x2e, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // xstopf (2d) — stop communication (2 bytes)
                [0x2d, 0x00, ..] => { ip += 2; }

                // xkeyb (2e) / xstate (2f) — no-op
                [0x2f, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // xconnect (26): port already open and CommConfig applied by the
                // preceding xsetpar/xawlen. This is where the transport performs its
                // physical init handshake (5-baud for KWP1281, fast-init for 0x010C).
                // No-op for DS2/BMW-FAST/D-CAN. A failure here means the ECU is absent.
                [0x26, ..] if ip + 1 < code.len() => {
                    self.transport.init_connection()
                        .map_err(|e| format!("xconnect failed: {e}"))?;
                    ip = skip_instr(code, ip);
                }
                [0x27, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xhangup
                [0x30, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xboot
                [0x31, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xreset
                [0x32, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xtype
                [0x33, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xvers

                // xreps (42): 42 70 OFF...
                [0x42, 0x70, _, _, _, _, ..] => { ip += 6; }

                // xsetpar (28 80): ImmStr with 18-byte CommParameter
                [0x28, 0x80, nn_lo, nn_hi, ..] => {
                    let n = (*nn_lo as usize) | ((*nn_hi as usize) << 8);
                    if n >= 18 && ip + 4 + n <= code.len() {
                        if let Some(cfg) = CommConfig::parse(&code[ip + 4..ip + 4 + n]) {
                            if self.awlen_set {
                                // xawlen already fixed the answer-length format — keep it.
                                // xsetpar only refreshes protocol/baud/timeouts/interbyte.
                                let (lo, la) = (self.comm_cfg.len_offset, self.comm_cfg.len_add);
                                self.comm_cfg = cfg;
                                self.comm_cfg.len_offset = lo;
                                self.comm_cfg.len_add = la;
                            } else {
                                self.comm_cfg = cfg;
                            }
                        }
                        trace!("[xsetpar] concept={:?} baud={} len_offset={} interbyte_ms={} (from .prg)",
                            self.comm_cfg.protocol, self.comm_cfg.baud, self.comm_cfg.len_offset,
                            self.comm_cfg.interbyte_ms);
                        if let Err(e) = self.transport.configure(&self.comm_cfg) {
                            trace!("[xsetpar@{ip:#x}] configure failed: {e}");
                        }
                    }
                    ip += 4 + n;
                }

                // xawlen (29 80): ImmStr with 4-byte CommAnswerLen
                [0x29, 0x80, nn_lo, nn_hi, ..] => {
                    let n = (*nn_lo as usize) | ((*nn_hi as usize) << 8);
                    if n >= 3 && ip + 4 + n <= code.len() {
                        let p = &code[ip + 4..ip + 4 + n];
                        // CommAnswerLen[0]: NEGATIVE (as i8) = dynamic — the LEN byte
                        // sits at offset -p[0] (0xFF→1, 0xFE→2); p[2] is the addend.
                        // NON-negative = not an offset spec we model (e.g. a fixed-length
                        // hint); fall back to old-style DS2 (LEN@1, total=LEN) instead of
                        // computing a garbage usize that would crash `receive()`.
                        let s = p[0] as i8;
                        let (lo, la) = if s < 0 {
                            (((-s) as usize).clamp(1, 8), p[2] as usize)
                        } else {
                            (1, 0)
                        };
                        self.comm_cfg.len_offset = lo;
                        self.comm_cfg.len_add    = la;
                        self.awlen_set = true; // authoritative from now on; xsetpar won't reset it
                        trace!("[xawlen] raw={:02X?} -> len_offset={} len_add={}",
                            p, self.comm_cfg.len_offset, self.comm_cfg.len_add);
                        if let Err(e) = self.transport.configure(&self.comm_cfg) {
                            trace!("[xawlen@{ip:#x}] configure failed: {e}");
                        }
                    }
                    ip += 4 + n;
                }

                // xbatt/xgetport/xsetport/xignit/xloopt/xsireset/xstoptr/xprog (no-op)
                [0x6e, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x71, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x72, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x73, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x74, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x76, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x77, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x78, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // xsendr (84) / xrecv (85) / xinfo (86) — advanced transport no-ops
                [0x84, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x85, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x86, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0xb3, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xsendex
                [0xb4, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xrecvex

                // ── timer / misc ──────────────────────────────────────────────

                // gettmr (43), settmr (44): 43/44 XX R
                [0x43, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x44, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x45, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // sett
                [0x46, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // clrt

                // wait (6b) — delay (ignore in VM)
                [0x6b, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // waitex (ae) — extended wait
                [0xae, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // ticks (ad) — get tick count → L0 (return 0)
                [0xad, ..] if ip + 1 < code.len() => {
                    self.regs.insert(REG_L0, Value::Long(0));
                    ip = skip_instr(code, ip);
                }

                // date (6c), time (6d) — return empty strings
                [0x6c, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x6d, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // break (4b) — stop current job
                [0x4b, 0x00, ..] => { break; }

                // eerr (4d) — emit error (no-op)
                [0x4d, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // generr (ac) — generate error (no-op)
                [0xac, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // atsp reg, #pos — peek `width` bytes from the value stack without
                // popping (EDIABAS OpAtsp): index = pos - width from top, MSB-first.
                [0x50, ..] if ip + 2 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let width = nib_width(hi);
                    if width > 0 {
                        let reg = code[ip + 2];
                        let (pos_val, next) = self.read_typed_operand(lo, code, ip + 3);
                        let value = self.stack_atsp(pos_val as usize, width);
                        if crate::trace::verbose() {
                            vtrace!("ATSP reg={:#04x} pos={pos_val} w={width} val={value:#x} stacklen={}",
                                      reg, self.stack.len());
                        }
                        self.write_num_reg(hi, reg, value);
                        self.set_flags_wl(value, width);
                        ip = next;
                    } else {
                        ip = skip_instr(code, ip);
                    }
                }

                // ── float arithmetic (no-op) ──────────────────────────────────
                // float arithmetic: dst = dst OP src, sets zero/minus flags on the result.
                // Setting flags is essential — measurement jobs branch on jz/jnz after these.
                [op @ 0x3b..=0x3e, ..] if ip + 1 < code.len() => {
                    let opcode = *op;
                    let mode = code[ip + 1];
                    let (hi, lo) = (mode >> 4, mode & 0xf);
                    let pos0 = ip + 2;
                    let l0 = nibble_size(hi, code, pos0);
                    let dst = code[pos0];
                    let (src, l1) = self.read_flt_src(code, pos0 + l0, lo);
                    let a = self.reg_flt(dst);
                    let r = match opcode {
                        0x3b => a + src,
                        0x3c => a - src,
                        0x3d => a * src,
                        _    => if src != 0.0 { a / src } else { 0.0 }, // fdiv
                    };
                    self.regs.insert(dst, Value::Float(r));
                    self.flags.zero = r == 0.0;
                    self.flags.minus = r < 0.0;
                    ip += 2 + l0 + l1;
                }

                // fcomp (a1) — compare two floats, set flags only (no write)
                [0xa1, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let (hi, lo) = (mode >> 4, mode & 0xf);
                    let pos0 = ip + 2;
                    let l0 = nibble_size(hi, code, pos0);
                    let a = self.reg_flt(code[pos0]);
                    let (b, l1) = self.read_flt_src(code, pos0 + l0, lo);
                    let d = a - b;
                    self.flags.zero = d == 0.0;
                    self.flags.minus = d < 0.0;
                    self.flags.carry = a < b;
                    ip += 2 + l0 + l1;
                }

                [0x88, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // setflt
                [0x9b, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // flt2y4
                [0x9c, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // flt2y8
                [0x9d, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // y42flt
                [0x9e, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // y82flt

                // ── file operations (no-op) ───────────────────────────────────
                [0x60, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // fopen
                [0x61, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // fread
                [0x62, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // freadln
                [0x63, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // fseek
                [0x64, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // fseekln
                [0x65, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // ftell
                [0x66, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // ftellln
                [0x59, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // fclose

                // ── parameter ops (par*) ──────────────────────────────────────
                // pary (7f) — whole argument buffer → S-reg (as string).
                // Sets zero flag when there are no args (jobs jz right after).
                [0x7f, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let (hi, lo) = (mode >> 4, mode & 0xf);
                    let dst = code[ip + 2];
                    let s = blat(&self.args);
                    // EDIABAS OpPary: Zero = true (no arg); Zero = false when arg present.
                    self.flags.zero = self.args.is_empty();
                    self.regs.insert(dst, Value::Str(s));
                    ip += 2 + nibble_size(hi, code, ip + 2) + nibble_size(lo, code, ip + 2);
                }

                // parn (80) — number of arguments → int/long reg
                [0x80, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let (hi, lo) = (mode >> 4, mode & 0xf);
                    let dst = code[ip + 2];
                    let n = self.arg_segments().len() as i32;
                    self.flags.zero = n == 0;
                    self.regs.insert(dst, Value::Long(n));
                    ip += 2 + nibble_size(hi, code, ip + 2) + nibble_size(lo, code, ip + 2);
                }

                // pars (58) / parr (69) — n-th argument (1-based) → S-reg (string / raw)
                [0x58, ..] | [0x69, ..] if ip + 1 < code.len() => {
                    let opcode = code[ip];
                    let mode = code[ip + 1];
                    let (hi, lo) = (mode >> 4, mode & 0xf);
                    let dst = code[ip + 2];
                    let l0 = nibble_size(hi, code, ip + 2);
                    let n = read_imm_u32(code, ip + 2 + l0, lo) as usize;
                    let seg = self.arg_segments().get(n.wrapping_sub(1)).map(|s| s.to_vec());
                    // zero flag = requested argument is absent/empty (jobs jnz on presence)
                    self.flags.zero = seg.as_ref().map_or(true, |s| s.is_empty());
                    let seg = seg.unwrap_or_default();
                    let v = if opcode == 0x69 {
                        Value::Data(seg)
                    } else {
                        Value::Str(String::from_utf8_lossy(&seg).into_owned())
                    };
                    self.regs.insert(dst, v);
                    ip += 2 + l0 + nibble_size(lo, code, ip + 2 + l0);
                }

                // parb (55) / parw (56) / parl (57) — n-th argument (1-based) as integer
                [0x55, ..] | [0x56, ..] | [0x57, ..] if ip + 1 < code.len() => {
                    let opcode = code[ip];
                    let mode = code[ip + 1];
                    let (hi, lo) = (mode >> 4, mode & 0xf);
                    let dst = code[ip + 2];
                    let l0 = nibble_size(hi, code, ip + 2);
                    let n = read_imm_u32(code, ip + 2 + l0, lo) as usize;
                    let seg_count = self.arg_segments().len();
                    self.flags.zero = n == 0 || n > seg_count;
                    let val = self.arg_int(n);
                    if opcode == 0x55 && hi == 2 {
                        self.set_byte_reg(dst, val as u8);
                    } else {
                        self.regs.insert(dst, Value::Long(val));
                    }
                    ip += 2 + l0 + nibble_size(lo, code, ip + 2 + l0);
                }
                [0x80, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // parn

                // ── shared memory (no-op) ─────────────────────────────────────
                [0x93, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // shmset
                [0x94, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // shmget

                // ── config (no-op) ────────────────────────────────────────────
                [0x89, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // cfgig
                [0x8a, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // cfgsg
                [0x8b, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // cfgis
                [0x8d, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xparraw

                // ── i-update ops (no-op) ──────────────────────────────────────
                [0x97, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // iupdate
                [0x98, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // irange
                [0x99, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // iincpos

                // ── plink / pcall / ppush / ppop (no-op) ─────────────────────
                [0x9f, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // plink
                [0xa0, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // pcall
                [0xa2, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // plinkv
                [0xa3, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // ppush
                [0xa4, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // ppop
                [0xa5, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // ppushflt
                [0xa6, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // ppopflt
                [0xa7, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // ppushy
                [0xa8, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // ppopy
                [0xa9, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // pjtsr

                // ── xopen / xclose / xswitch (no-op) ─────────────────────────
                [0xaf, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xopen
                [0xb0, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xclose
                [0xb1, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xcloseex
                [0xb2, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xswitch

                // tosp (6f) — no-op
                [0x6f, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
                [0x70, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // xdownl

                // ── unknown: use MODE to skip correctly ───────────────────────
                _ => {
                    trace!(
                        "vm: unknown opcode {:#04x} at ip={ip:#06x} (context: {})",
                        code[ip],
                        hex(&code[ip..code.len().min(ip + 8)])
                    );
                    ip = skip_instr(code, ip);
                }
            }
        }

        sets.push(current);
        Ok(sets)
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    fn get_byte_reg(&self, sub_reg: u8) -> u8 {
        let long_reg = REG_L0 + sub_reg / 4;
        let byte_pos = (sub_reg % 4) as u32;
        let v = self.regs.get(&long_reg).map(Value::as_long).unwrap_or(0);
        ((v >> (byte_pos * 8)) & 0xff) as u8
    }

    fn set_byte_reg(&mut self, sub_reg: u8, val: u8) {
        let long_reg = REG_L0 + sub_reg / 4;
        let byte_pos = (sub_reg % 4) as u32;
        let v = self.regs.get(&long_reg).map(Value::as_long).unwrap_or(0);
        let mask = !(0xffi32 << (byte_pos * 8));
        let new_val = (v & mask) | ((val as i32) << (byte_pos * 8));
        self.regs.insert(long_reg, Value::Long(new_val));
    }

    /// Word registers I0-I7 (0x10-0x17) OVERLAY the L registers, like B regs:
    /// I0=low word of L0, I1=high word of L0, I2=low word of L1, … (EDIABAS flat
    /// register file). Writing I updates the containing L register.
    fn set_word_reg(&mut self, i_reg: u8, val: u16) {
        let idx = i_reg - 0x10;
        let long_reg = REG_L0 + idx / 2;
        let wp = (idx % 2) as u32;
        let v = self.regs.get(&long_reg).map(Value::as_long).unwrap_or(0);
        let mask = !(0xffffi32 << (wp * 16));
        self.regs.insert(long_reg, Value::Long((v & mask) | (((val as i32) & 0xffff) << (wp * 16))));
    }

    /// Write a numeric value into any B/I/L register, honouring the B/I → L overlap.
    fn write_reg_val(&mut self, reg: u8, val: i32) {
        match reg {
            0x00..=0x0f => self.set_byte_reg(reg, val as u8),
            0x10..=0x17 => self.set_word_reg(reg, val as u16),
            _ => { self.regs.insert(reg, Value::Long(val)); }
        }
    }

    /// Push `width` bytes of `val` (little-endian) onto the byte stack.
    fn stack_push_bytes(&mut self, val: i64, width: usize) {
        let mut v = val as u64;
        for _ in 0..width {
            self.stack.push((v & 0xff) as u8);
            v >>= 8;
        }
    }

    /// Pop `width` bytes off the byte stack, MSB-first (matches EDIABAS OpPop).
    fn stack_pop_bytes(&mut self, width: usize) -> i64 {
        let mut value: u64 = 0;
        for _ in 0..width {
            let b = self.stack.pop().unwrap_or(0);
            value = (value << 8) | b as u64;
        }
        value as i64
    }

    /// Peek `width` bytes at stack position `pos` without popping (EDIABAS OpAtsp).
    /// index = pos - width from the top of stack, read MSB-first.
    fn stack_atsp(&self, pos: usize, width: usize) -> i64 {
        let len = self.stack.len();
        if width == 0 || len < width { return 0; }
        let index = pos.saturating_sub(width);
        let mut value: u64 = 0;
        for i in 0..width {
            let si = index + i;
            let b = if si < len { self.stack[len - 1 - si] } else { 0 };
            value = (value << 8) | b as u64;
        }
        value as i64
    }

    /// Write a numeric value into `reg`. The register id selects the B/I/L overlap.
    fn write_num_reg(&mut self, _nib: u8, reg: u8, value: i64) {
        self.write_reg_val(reg, value as i32);
    }

    /// Decode an indexed-with-length source operand (modes IdxImmLenImm/IdxImmLenReg/
    /// IdxRegLenImm/IdxRegLenReg) → (base_reg, index, length, next_pos). Byte layouts
    /// per EdiabasLib GetOpArg.
    fn read_slice_operand(&self, lo: u8, code: &[u8], pos: usize) -> Option<(u8, usize, usize, usize)> {
        let g = |p: usize| code.get(p).copied().unwrap_or(0);
        let reg_val = |r: u8| reg_val_from_map(&self.regs, r).max(0) as usize;
        match lo {
            0xc => { // IdxImmLenImm: reg + u16 idx + u16 len   (5 bytes)
                let base = g(pos);
                let idx = u16::from_le_bytes([g(pos + 1), g(pos + 2)]) as usize;
                let len = u16::from_le_bytes([g(pos + 3), g(pos + 4)]) as usize;
                Some((base, idx, len, pos + 5))
            }
            0xd => { // IdxImmLenReg: reg + u16 idx + reg len    (4 bytes)
                let base = g(pos);
                let idx = u16::from_le_bytes([g(pos + 1), g(pos + 2)]) as usize;
                Some((base, idx, reg_val(g(pos + 3)), pos + 4))
            }
            0xe => { // IdxRegLenImm: reg + reg idx + u16 len    (4 bytes)
                let base = g(pos);
                let idx = reg_val(g(pos + 1));
                let len = u16::from_le_bytes([g(pos + 2), g(pos + 3)]) as usize;
                Some((base, idx, len, pos + 4))
            }
            0xf => { // IdxRegLenReg: reg + reg idx + reg len    (3 bytes)
                let base = g(pos);
                Some((base, reg_val(g(pos + 1)), reg_val(g(pos + 2)), pos + 3))
            }
            _ => None,
        }
    }

    /// Read `width` bytes little-endian from S-register `sreg`'s data at byte `offset`
    /// (EDIABAS Operand.GetValueData over an indexed byte[] source).
    fn read_sreg_value(&self, sreg: u8, offset: usize, width: usize) -> i64 {
        let data = self.reg_bytes(&sreg);
        let mut value: u64 = 0;
        for i in (0..width).rev() {
            let b = data.get(offset + i).copied().unwrap_or(0);
            value = (value << 8) | b as u64;
        }
        value as i64
    }

    /// EDIABAS Flags.UpdateFlags: set zero + sign(minus) by operand width.
    fn set_flags_wl(&mut self, value: i64, width: usize) {
        let (mask, sign): (u64, u64) = match width {
            1 => (0xff, 0x80),
            2 => (0xffff, 0x8000),
            _ => (0xffff_ffff, 0x8000_0000),
        };
        let v = value as u64;
        self.flags.zero = (v & mask) == 0;
        self.flags.minus = (v & sign) != 0;
    }

    /// Decode two ALU operands from the MODE byte, handling RegAB (nibble=2) correctly.
    /// Returns (hi_nibble, dst_reg, dst_val, src_val, next_ip). dst_reg=0xff if no dst.
    fn alu2_typed(&self, code: &[u8], ip: usize) -> (u8, u8, i32, i32, usize) {
        if ip + 1 >= code.len() { return (0, 0xff, 0, 0, ip + 1); }
        let mode = code[ip + 1];
        let hi = mode >> 4;
        let lo = mode & 0xf;
        let mut pos = ip + 2;

        let (dst, dst_val) = if (1..=4).contains(&hi) && pos < code.len() {
            let r = code[pos]; pos += 1;
            (r, reg_val_from_map(&self.regs, r))
        } else if hi >= 9 {
            // Indexed dst operand (e.g. `comp S1[#0], #0`): read its value and
            // advance past the operand bytes so the stream doesn't desync. dst=0xff
            // marks it non-writable-back (fine for comp/test; alu writes are skipped).
            let (v, next) = self.read_indexed_val(hi, code, pos);
            pos = next;
            (0xff, v)
        } else {
            (0xff, 0)
        };

        let (src_val, next) = self.read_typed_operand(lo, code, pos);
        (hi, dst, dst_val, src_val, next)
    }

    /// Read the value of an indexed operand (modes 9/a/c/d/e/f) from an S-register,
    /// returning (value, next_pos). IdxImm/IdxReg read a single byte; the *Len* modes
    /// read up to 4 bytes little-endian.
    fn read_indexed_val(&self, nib: u8, code: &[u8], pos: usize) -> (i32, usize) {
        let g = |p: usize| code.get(p).copied().unwrap_or(0);
        let reg_val = |r: u8| reg_val_from_map(&self.regs, r).max(0) as usize;
        match nib {
            0x9 => {
                let base = g(pos);
                let idx = u16::from_le_bytes([g(pos + 1), g(pos + 2)]) as usize;
                (self.read_sreg_value(base, idx, 1) as i32, pos + 3)
            }
            0xa => {
                let base = g(pos);
                let idx = reg_val(g(pos + 1));
                (self.read_sreg_value(base, idx, 1) as i32, pos + 2)
            }
            0xc | 0xd | 0xe | 0xf => {
                if let Some((base, idx, len, next)) = self.read_slice_operand(nib, code, pos) {
                    (self.read_sreg_value(base, idx, len.clamp(1, 4)) as i32, next)
                } else {
                    (0, pos + nibble_size(nib, code, pos))
                }
            }
            _ => (0, pos + nibble_size(nib, code, pos)),
        }
    }

    /// Read a single operand value by nibble type, with correct RegAB (nibble=2) handling.
    fn read_typed_operand(&self, nibble: u8, code: &[u8], pos: usize) -> (i32, usize) {
        match nibble {
            0 => (0, pos),
            1 | 3 | 4 => {
                if pos < code.len() {
                    (reg_val_from_map(&self.regs, code[pos]), pos + 1)
                } else { (0, pos + 1) }
            }
            2 => {
                if pos < code.len() { (self.get_byte_reg(code[pos]) as i32, pos + 1) }
                else { (0, pos + 1) }
            }
            5 => if pos < code.len() { (code[pos] as i32, pos + 1) } else { (0, pos + 1) },
            6 => if pos + 1 < code.len() {
                (u16::from_le_bytes([code[pos], code[pos+1]]) as i32, pos + 2)
            } else { (0, pos + 2) },
            7 => if pos + 3 < code.len() {
                (i32::from_le_bytes([code[pos], code[pos+1], code[pos+2], code[pos+3]]), pos + 4)
            } else { (0, pos + 4) },
            8 => {
                if pos + 1 < code.len() {
                    let n = code[pos] as usize | ((code[pos+1] as usize) << 8);
                    (0, pos + 2 + n)
                } else { (0, pos) }
            }
            0x9 => (0, pos + 3),   // IdxImm
            0xa => (0, pos + 2),   // IdxReg
            0xb => (0, pos + 4),   // IdxRegImm
            0xc => (0, pos + 5),   // IdxImmLenImm
            0xd => (0, pos + 4),   // IdxImmLenReg
            0xe => (0, pos + 4),   // IdxRegLenImm
            _   => (0, pos + 3),   // IdxRegLenReg
        }
    }

    /// Write ALU result back to destination, using correct RegAB handling (nibble=2).
    fn alu2_write(&mut self, _hi: u8, dst: u8, result: i32) {
        if dst == 0xff { return; }
        self.write_reg_val(dst, result);
    }

    fn reg_str(&self, reg: u8) -> String {
        match self.regs.get(&reg) {
            Some(Value::Str(s)) => s.clone(),
            Some(Value::Data(d)) => blat(d),
            Some(Value::Long(v)) => v.to_string(),
            Some(Value::Float(f)) => fmt_flt(*f),
            None => String::new(),
        }
    }

    fn reg_bytes(&self, reg: &u8) -> Vec<u8> {
        match self.regs.get(reg) {
            Some(Value::Data(d)) => d.clone(),
            Some(Value::Str(s)) => unlat(s),
            Some(Value::Long(v)) => v.to_le_bytes().to_vec(),
            Some(Value::Float(f)) => f.to_le_bytes().to_vec(),
            None => Vec::new(),
        }
    }

    /// Read a register's value as f64 (float registers live in S-regs).
    fn reg_flt(&self, id: u8) -> f64 {
        match self.regs.get(&id) {
            Some(Value::Float(f)) => *f,
            Some(Value::Long(v)) => *v as f64,
            Some(Value::Str(s)) => parse_f64(s),
            Some(Value::Data(d)) if d.len() >= 8 => {
                f64::from_le_bytes(d[..8].try_into().unwrap())
            }
            _ => 0.0,
        }
    }

    /// Read the second operand of a float instruction as f64, honouring its
    /// addressing-mode nibble. Returns (value, operand_byte_len).
    fn read_flt_src(&self, code: &[u8], pos: usize, nib: u8) -> (f64, usize) {
        match nib {
            1 => (self.reg_flt(code[pos]), 1),                       // RegS
            2 => (self.get_byte_reg(code[pos]) as f64, 1),          // RegAb
            3 | 4 => (                                               // RegI / RegL
                self.regs.get(&code[pos]).map(Value::as_long).unwrap_or(0) as f64, 1),
            5 => (code[pos] as f64, 1),                              // Imm8
            6 => (u16::from_le_bytes([code[pos], code[pos + 1]]) as f64, 2),
            7 => (i32::from_le_bytes(
                    [code[pos], code[pos + 1], code[pos + 2], code[pos + 3]]) as f64, 4),
            8 => {                                                   // ImmStr → parse
                let n = code[pos] as usize | ((code[pos + 1] as usize) << 8);
                let s = cstr(&code[pos + 2..(pos + 2 + n).min(code.len())]);
                (parse_f64(&s), 2 + n)
            }
            _ => (0.0, nibble_size(nib, code, pos)),                 // indexed fallback
        }
    }

    fn jump_target(&self, ip: usize, insn_size: usize, off: i32, code_len: usize)
        -> Result<usize, String>
    {
        let base = (ip + insn_size) as i64;
        let target = base + off as i64;
        if target < 0 || target as usize > code_len {
            return Err(format!("jump out of bounds at ip={ip:#x}: offset={off}"));
        }
        Ok(target as usize)
    }
}


