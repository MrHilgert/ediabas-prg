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
use crate::config::{CommConfig, Protocol};
use crate::transport::Transport;
use crate::prg::Table;

#[derive(Debug, Clone)]
pub enum Value {
    Long(i32),
    Str(String),
    Data(Vec<u8>),
}

impl Value {
    pub fn as_long(&self) -> i32 {
        match self {
            Value::Long(v) => *v,
            Value::Str(s) => s.trim().parse().unwrap_or(0),
            Value::Data(_) => 0,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Value::Str(s) => s,
            _ => "",
        }
    }

    pub fn as_data(&self) -> &[u8] {
        match self {
            Value::Data(d) => d,
            Value::Str(s) => s.as_bytes(),
            Value::Long(_) => &[],
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Long(v) => write!(f, "{v}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Data(d) => write!(f, "{}", hex(d)),
        }
    }
}

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
    stack: Vec<Value>,
    transport: Box<dyn Transport>,
    tables: HashMap<String, Table>,
    active_table: Option<String>,
    found_row: Option<usize>,
    flags: Flags,
    call_stack: Vec<usize>,
    comm_cfg: CommConfig,
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
        }
    }

    pub fn run_job(&mut self, code: &[u8]) -> Result<Vec<ResultSet>, String> {
        self.stack.clear();
        self.call_stack.clear();
        let mut ip = 0usize;
        let mut current: ResultSet = HashMap::new();
        let mut sets: Vec<ResultSet> = Vec::new();

        while ip < code.len() {
            if code[ip] == 0xf7 { break; }

            let slice = &code[ip..];
            match slice {

                // ── eoj ──────────────────────────────────────────────────────────
                [0x1d, 0x00, ..] => break,

                [0x1d, 0x10, 0x1d, ..] => {
                    let s = self.reg_str(REG_S1);
                    current.insert("JOB_STATUS".into(), Value::Str(s));
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

                // move RegS, RegS: 00 11 R1 R2
                [0x00, 0x11, r1, r2, ..] => {
                    let v = self.regs.get(r2).cloned().unwrap_or(Value::Str(String::new()));
                    self.regs.insert(*r1, v);
                    ip += 4;
                }

                // move RegS, ImmStr: 00 18 R NN_lo NN_hi [NN]
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
                    let idx_val = self.regs.get(idx).map(Value::as_long).unwrap_or(0) as usize;
                    let byte_val = {
                        let data = self.reg_bytes(src);
                        data.get(idx_val).copied().unwrap_or(0)
                    };
                    self.set_byte_reg(*dst, byte_val);
                    ip += 5;
                }

                // move [DST_REG + IDX_REG], SRC_BYTE: 00 a2 DST IDX SRC (5 bytes)
                // MODE=0xa2: upper=0xa(IdxReg dst), lower=0x2(RegAB src)
                [0x00, 0xa2, dst_reg, idx_reg, src_b, ..] => {
                    let idx = self.regs.get(idx_reg).map(Value::as_long).unwrap_or(0) as usize;
                    let byte_val = self.get_byte_reg(*src_b);
                    let mut data = match self.regs.get(dst_reg) {
                        Some(Value::Data(d)) => d.clone(),
                        Some(Value::Str(s))  => s.as_bytes().to_vec(),
                        _                    => Vec::new(),
                    };
                    if data.len() <= idx { data.resize(idx + 1, 0); }
                    data[idx] = byte_val;
                    self.regs.insert(*dst_reg, Value::Data(data));
                    ip += 5;
                }

                // move [DST_REG + IMM_OFF], SRC_BYTE: 00 92 DST IMM SRC (5 bytes)
                // MODE=0x92: upper=0x9(IdxImm dst), lower=0x2(RegAB src)
                [0x00, 0x92, dst_reg, imm_off, src_b, ..] => {
                    let idx = *imm_off as usize;
                    let byte_val = self.get_byte_reg(*src_b);
                    let mut data = match self.regs.get(dst_reg) {
                        Some(Value::Data(d)) => d.clone(),
                        Some(Value::Str(s))  => s.as_bytes().to_vec(),
                        _                    => Vec::new(),
                    };
                    if data.len() <= idx { data.resize(idx + 1, 0); }
                    data[idx] = byte_val;
                    self.regs.insert(*dst_reg, Value::Data(data));
                    ip += 5;
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

                // move ImmStr into second reg (a2flt encoding): 3a 18 R NN_lo NN_hi [NN]
                // Note: opcode 0x3a = a2flt but in practice this form loads an ImmStr
                [0x3a, 0x18, reg, nn_lo, nn_hi, ..] => {
                    let n = *nn_lo as usize | ((*nn_hi as usize) << 8);
                    if ip + 5 + n > code.len() {
                        return Err(format!("MOVE3A truncated at ip={ip:#x}"));
                    }
                    let s = cstr(&code[ip + 5..ip + 5 + n]);
                    self.regs.insert(*reg, Value::Str(s));
                    ip += 5 + n;
                }

                // move RegAB, Imm8: 00 25 R V
                [0x00, 0x25, reg, v, ..] => {
                    self.set_byte_reg(*reg, *v);
                    ip += 4;
                }

                // move RegI, Imm16: 00 36 R V_lo V_hi
                [0x00, 0x36, reg, v_lo, v_hi, ..] => {
                    let v = *v_lo as i32 | ((*v_hi as i32) << 8);
                    self.regs.insert(*reg, Value::Long(v));
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
                    let idx = self.regs.get(idx_reg).map(Value::as_long).unwrap_or(0) as usize;
                    let src_byte = self.get_byte_reg(*src_b);
                    let mut data = match self.regs.get(base) {
                        Some(Value::Data(d)) => d.clone(),
                        Some(Value::Str(s))  => s.as_bytes().to_vec(),
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

                // push #Imm32
                [0x1e, 0x70, v0, v1, v2, v3, ..] => {
                    let v = i32::from_le_bytes([*v0, *v1, *v2, *v3]);
                    self.stack.push(Value::Long(v));
                    ip += 6;
                }

                // push LReg
                [0x1e, 0x40, xx, ..] => {
                    let v = self.regs.get(xx).cloned().unwrap_or(Value::Long(0));
                    self.stack.push(v);
                    ip += 3;
                }

                // push RegS
                [0x1e, 0x10, xx, ..] => {
                    let v = self.regs.get(xx).cloned().unwrap_or(Value::Str(String::new()));
                    self.stack.push(v);
                    ip += 3;
                }

                // push ImmStr
                [0x1e, 0x80, nn_lo, nn_hi, ..] => {
                    let n = *nn_lo as usize | ((*nn_hi as usize) << 8);
                    let s = if ip + 4 + n <= code.len() { cstr(&code[ip+4..ip+4+n]) } else { String::new() };
                    self.stack.push(Value::Str(s));
                    ip += 4 + n;
                }

                // pop → LReg
                [0x1f, 0x40, xx, ..] => {
                    let v = self.stack.pop().unwrap_or(Value::Long(0));
                    self.regs.insert(*xx, v);
                    ip += 3;
                }

                // pop → RegS
                [0x1f, 0x10, xx, ..] => {
                    let v = self.stack.pop().unwrap_or(Value::Str(String::new()));
                    self.regs.insert(*xx, v);
                    ip += 3;
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

                // scmp — compare strings: sets L0 = strcmp(R1,R2); 20 11 R1 R2
                [0x20, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let (s1, p1) = read_str_at(&self.regs, hi, code, pos);
                    pos = p1;
                    let (s2, p2) = read_str_at(&self.regs, lo, code, pos);
                    let result = s1.cmp(&s2) as i32;
                    self.regs.insert(REG_L0, Value::Long(result));
                    self.flags.zero = result == 0;
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
                        let cut: String = s.chars().take(n).collect();
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
                    let len = s.len() as i32;
                    self.regs.insert(REG_L0, Value::Long(len));
                    ip = p2;
                }

                // spaste — paste S2 into S1 at position L0: 24 11 R1 R2
                [0x24, ..] if ip + 1 < code.len() => {
                    let mode = code[ip + 1];
                    let hi = mode >> 4;
                    let lo = mode & 0xf;
                    let mut pos = ip + 2;
                    let r1 = if (1..=4).contains(&hi) && pos < code.len() {
                        let r = code[pos]; pos += 1; r
                    } else { 0xff };
                    let (s2, p2) = read_str_at(&self.regs, lo, code, pos);
                    if r1 != 0xff {
                        let mut s1 = self.reg_str(r1);
                        let insert_pos = self.regs.get(&REG_L0)
                            .map(Value::as_long).unwrap_or(0).max(0) as usize;
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
                    if dst != 0xff { self.regs.insert(dst, Value::Long(v)); }
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

                // a2flt (3a) — only the non-ImmStr forms (ImmStr already handled above)
                [0x3a, ..] if ip + 1 < code.len() => {
                    ip = skip_instr(code, ip);
                }

                // fix2flt (68) — integer → float (no-op, store as-is)
                [0x68, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // flt2a (87) — float → string (treat float reg as string)
                [0x87, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // flt2fix (96) — float → integer
                [0x96, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

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
                        let v = self.regs.get(&reg).map(Value::as_long).unwrap_or(0) & 0xffff;
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
                        let v = self.regs.get(&reg).map(Value::as_long).unwrap_or(0);
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
                        let v = self.regs.get(&reg).map(Value::as_long).unwrap_or(0);
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
                        let v = self.regs.get(&reg).map(Value::as_long).unwrap_or(0);
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

                // ── result sets ───────────────────────────────────────────────

                // enewset (40) — commit current result set, start new
                [0x40, 0x00, ..] => {
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
                        eprintln!("TABGET [{col_name}] → {val:?}");
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
                    eprintln!("XSEND telegram ({} bytes): {}", data.len(),
                        data.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "));
                    let response = self.transport.exchange(&data)
                        .map_err(|e| format!("xsend failed: {e}"))?;
                    eprintln!("XSEND response ({} bytes): {}", response.len(),
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
                    eprintln!("[xrequf@{ip:#x}] raw bytes: {}",
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

                // xconnect (26): port already open; CommConfig set by xsetpar/xawlen. Skip.
                [0x26, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }
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
                        self.comm_cfg = parse_comm_params(&code[ip + 4..ip + 4 + n]);
                        eprintln!("[xsetpar] concept={:?} baud={} len_offset={}",
                            self.comm_cfg.protocol, self.comm_cfg.baud, self.comm_cfg.len_offset);
                        if let Err(e) = self.transport.configure(&self.comm_cfg) {
                            eprintln!("[xsetpar@{ip:#x}] configure failed: {e}");
                        }
                    }
                    ip += 4 + n;
                }

                // xawlen (29 80): ImmStr with 4-byte CommAnswerLen
                [0x29, 0x80, nn_lo, nn_hi, ..] => {
                    let n = (*nn_lo as usize) | ((*nn_hi as usize) << 8);
                    if n >= 3 && ip + 4 + n <= code.len() {
                        let p = &code[ip + 4..ip + 4 + n];
                        self.comm_cfg.len_offset = (-(p[0] as i8)) as usize;
                        self.comm_cfg.len_add    = p[2] as usize;
                        eprintln!("[xawlen] len_offset={} len_add={}",
                            self.comm_cfg.len_offset, self.comm_cfg.len_add);
                        if let Err(e) = self.transport.configure(&self.comm_cfg) {
                            eprintln!("[xawlen@{ip:#x}] configure failed: {e}");
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

                // ── atsp (50) — set protocol parameters ──────────────────────
                [0x50, 0x47, _, _, _, _, _, ..] => { ip += 7; }
                [0x50, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); }

                // ── float arithmetic (no-op) ──────────────────────────────────
                [0x3b, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // fadd
                [0x3c, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // fsub
                [0x3d, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // fmul
                [0x3e, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // fdiv
                [0xa1, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // fcomp
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
                [0x55, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // parb
                [0x56, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // parw
                [0x57, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // parl
                [0x58, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // pars
                [0x69, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // parr
                [0x7f, ..] if ip + 1 < code.len() => { ip = skip_instr(code, ip); } // pary
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
                    eprintln!(
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

    /// Decode two ALU operands from the MODE byte, handling RegAB (nibble=2) correctly.
    /// Returns (hi_nibble, dst_reg, dst_val, src_val, next_ip). dst_reg=0xff if no dst.
    fn alu2_typed(&self, code: &[u8], ip: usize) -> (u8, u8, i32, i32, usize) {
        if ip + 1 >= code.len() { return (0, 0xff, 0, 0, ip + 1); }
        let mode = code[ip + 1];
        let hi = mode >> 4;
        let lo = mode & 0xf;
        let mut pos = ip + 2;

        let dst = if (1..=4).contains(&hi) && pos < code.len() {
            let r = code[pos]; pos += 1; r
        } else { 0xff };

        let dst_val = if dst != 0xff {
            match hi {
                2 => self.get_byte_reg(dst) as i32,
                _ => self.regs.get(&dst).map(Value::as_long).unwrap_or(0),
            }
        } else { 0 };

        let (src_val, next) = self.read_typed_operand(lo, code, pos);
        (hi, dst, dst_val, src_val, next)
    }

    /// Read a single operand value by nibble type, with correct RegAB (nibble=2) handling.
    fn read_typed_operand(&self, nibble: u8, code: &[u8], pos: usize) -> (i32, usize) {
        match nibble {
            0 => (0, pos),
            1 | 3 | 4 => {
                if pos < code.len() {
                    (self.regs.get(&code[pos]).map(Value::as_long).unwrap_or(0), pos + 1)
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
            9 | 0xa => (0, pos + 2),
            0xb     => (0, pos + 3),
            _       => (0, pos + 3),
        }
    }

    /// Write ALU result back to destination, using correct RegAB handling (nibble=2).
    fn alu2_write(&mut self, hi: u8, dst: u8, result: i32) {
        if dst == 0xff { return; }
        match hi {
            2 => self.set_byte_reg(dst, result as u8),
            _ => { self.regs.insert(dst, Value::Long(result)); }
        }
    }

    fn reg_str(&self, reg: u8) -> String {
        match self.regs.get(&reg) {
            Some(Value::Str(s)) => s.clone(),
            Some(Value::Data(d)) => hex(d),
            Some(Value::Long(v)) => v.to_string(),
            None => String::new(),
        }
    }

    fn reg_bytes(&self, reg: &u8) -> Vec<u8> {
        match self.regs.get(reg) {
            Some(Value::Data(d)) => d.clone(),
            Some(Value::Str(s)) => s.as_bytes().to_vec(),
            Some(Value::Long(v)) => v.to_le_bytes().to_vec(),
            None => Vec::new(),
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

// ── free functions ────────────────────────────────────────────────────────────


/// Read a value from the operand at `pos` based on MODE nibble.
fn read_long_at(regs: &HashMap<u8, Value>, nibble: u8, code: &[u8], pos: usize) -> (i32, usize) {
    match nibble {
        0 => (0, pos),
        1..=4 => {
            if pos < code.len() {
                let r = code[pos];
                (regs.get(&r).map(Value::as_long).unwrap_or(0), pos + 1)
            } else { (0, pos + 1) }
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
        // Extended indexed modes — read base reg (and idx_reg if applicable) but value is 0
        9 | 0xa => (0, pos + 2),
        0xb     => (0, pos + 3),
        _       => (0, pos + 3),
    }
}

/// Read a string operand from the given nibble.
fn read_str_at(regs: &HashMap<u8, Value>, nibble: u8, code: &[u8], pos: usize) -> (String, usize) {
    match nibble {
        0 => (String::new(), pos),
        1..=4 => {
            if pos < code.len() {
                let r = code[pos];
                let s = match regs.get(&r) {
                    Some(Value::Str(s)) => s.clone(),
                    Some(Value::Long(v)) => v.to_string(),
                    Some(Value::Data(d)) => hex(d),
                    None => String::new(),
                };
                (s, pos + 1)
            } else { (String::new(), pos + 1) }
        }
        5 => if pos < code.len() { ((code[pos] as char).to_string(), pos + 1) } else { (String::new(), pos + 1) },
        6 => if pos + 1 < code.len() { (String::new(), pos + 2) } else { (String::new(), pos + 2) },
        7 => if pos + 3 < code.len() { (String::new(), pos + 4) } else { (String::new(), pos + 4) },
        8 => {
            if pos + 1 < code.len() {
                let n = code[pos] as usize | ((code[pos+1] as usize) << 8);
                let end = (pos + 2 + n).min(code.len());
                let s = cstr(&code[pos+2..end]);
                (s, pos + 2 + n)
            } else { (String::new(), pos) }
        }
        9 | 0xa => (String::new(), pos + 2),
        0xb     => (String::new(), pos + 3),
        _       => (String::new(), pos + 3),
    }
}

/// Parse a 18-byte EDIABAS CommParameter block into CommConfig.
/// Layout: [concept u16][baud u16][ecu_type u16][0 u16][0 u16][timeout_std u16]
///         [regen u16][tel_end u16][hdr_len u16]  (all little-endian)
fn parse_comm_params(p: &[u8]) -> CommConfig {
    let u16le = |off: usize| -> u16 {
        if off + 1 < p.len() { u16::from_le_bytes([p[off], p[off + 1]]) } else { 0 }
    };
    let concept        = u16le(0);
    let baud           = u16le(2) as u32;
    let timeout_std_ms = u16le(10) as u64;
    let regen_time_ms  = u16le(12) as u64;
    let timeout_tel_ms = u16le(14) as u64;
    let hdr_len        = u16le(16) as usize;

    let protocol = match concept {
        0x0001 | 0x0005 | 0x0006 => Protocol::Ds2,
        0x0002 | 0x0003          => Protocol::Kwp1281,
        0x010F                   => Protocol::BmwFast,
        0x0110                   => Protocol::DCan,
        _                        => Protocol::Ds2,
    };

    let mut cfg = CommConfig::default();
    cfg.protocol       = protocol;
    cfg.baud           = baud;
    cfg.len_offset     = if hdr_len > 0 { hdr_len - 1 } else { 1 };
    cfg.len_add        = 0; // overridden by subsequent xawlen
    cfg.timeout_std_ms = if timeout_std_ms > 0 { timeout_std_ms } else { 2000 };
    cfg.regen_time_ms  = if regen_time_ms  > 0 { regen_time_ms  } else { 20 };
    cfg.timeout_tel_ms = if timeout_tel_ms > 0 { timeout_tel_ms } else { 50 };
    cfg
}

/// Compute how many extra bytes a MODE nibble occupies, given the current position.
/// Basic types 0-8 are standard.  Extended indexed types 9-f:
///   9 = IdxImm  : base_reg(1) + imm8(1)  = 2 bytes
///   a = IdxReg  : base_reg(1) + idx_reg(1) = 2 bytes  (observed: 00 a2 ...)
///   b = IdxRegImm: base_reg(1)+idx_reg(1)+imm8(1) = 3 bytes
///   c-f = indexed-with-length variants; assume 3 bytes as safe minimum
fn nibble_size(nibble: u8, code: &[u8], pos: usize) -> usize {
    match nibble {
        0 => 0,
        1|2|3|4|5 => 1,
        6 => 2,
        7 => 4,
        8 => {
            if pos + 1 < code.len() {
                let n = code[pos] as usize | ((code[pos+1] as usize) << 8);
                2 + n
            } else { 2 }
        }
        9 | 0xa => 2,     // IdxImm / IdxReg
        0xb => 3,         // IdxRegImm
        _ => 3,           // IdxImmLen* / IdxRegLen* — conservative
    }
}

/// Skip an instruction using the MODE byte to determine length.
fn skip_instr(code: &[u8], ip: usize) -> usize {
    if ip + 1 >= code.len() { return ip + 1; }
    let mode = code[ip + 1];
    let hi = mode >> 4;
    let lo = mode & 0xf;
    let mut pos = ip + 2;
    pos += nibble_size(hi, code, pos);
    pos += nibble_size(lo, code, pos);
    pos
}

/// Parse a hex string like "B812F104" into a byte vec.
fn parse_hex_str(s: &str) -> Vec<u8> {
    let s = s.trim();
    (0..s.len().saturating_sub(0))
        .step_by(2)
        .filter(|&i| i + 1 < s.len())
        .filter_map(|i| u8::from_str_radix(&s[i..i+2], 16).ok())
        .collect()
}

fn cstr(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}
