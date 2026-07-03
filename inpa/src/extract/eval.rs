//! A tiny stack evaluator that recovers each call's argument list.
//!
//! The bytecode delimits every call's arguments with a `callframe` (0x0f) marker, so we
//! replay the instruction stream on a value stack and, at each call, split off everything
//! pushed since the matching frame. Values we cannot compute statically (globals, locals,
//! non-string arithmetic) become [`Val::Unknown`] — which is exactly right, since the
//! measurement/activation recipes only depend on the plain constant operands.

use crate::opcode::{self, mode, op, CALL_PROC};
use crate::record::Const;

/// A statically-recovered operand value. `Bool`/`Global`/`Local` carry their operand to keep
/// the stack model faithful and Debug-legible even though the recipes only read the rest.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(super) enum Val {
    Str(String),
    Num(f64),
    Bool(bool),
    Global(u16),
    Local(u16),
    ScreenRef(u16),
    MenuRef(u16),
    Unknown,
}

impl Val {
    pub(super) fn as_str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }
    pub(super) fn as_num(&self) -> Option<f64> {
        match self {
            Val::Num(n) => Some(*n),
            _ => None,
        }
    }
}

/// What a [`CallSite`] targets.
#[derive(Debug, Clone, Copy)]
pub(super) enum Target {
    Proc(u16),
    Builtin(u16),
    Import,
}

/// One recovered call: its target and (best-effort) argument list, in source order.
#[derive(Debug, Clone)]
pub(super) struct CallSite {
    pub target: Target,
    pub args: Vec<Val>,
}

/// Replay a code body and return its call sites in order.
pub(super) fn call_sites(code: &[u8], consts: &[Const]) -> Vec<CallSite> {
    let mut stack: Vec<Val> = Vec::new();
    let mut frames: Vec<usize> = Vec::new();
    let mut out: Vec<CallSite> = Vec::new();

    for ins in opcode::decode(code) {
        match ins.op {
            op::CALLFRAME => frames.push(stack.len()),
            op::PUSH => stack.push(push_val(ins.mode, ins.opnd, consts)),
            op::PUSH_DEREF => stack.push(Val::Unknown),
            op::PUSHREF => stack.push(match ins.mode {
                mode::SCREEN => Val::ScreenRef(ins.opnd),
                mode::MENU => Val::MenuRef(ins.opnd),
                mode::GLOBAL => Val::Global(ins.opnd),
                mode::LOCAL => Val::Local(ins.opnd),
                _ => Val::Unknown,
            }),
            op::ALU => apply_alu(&mut stack, ins.mode),
            op::POP => {
                let n = ins.opnd as usize;
                let len = stack.len();
                stack.truncate(len.saturating_sub(n));
            }
            // store / store-ref leave the value on the stack → no net effect here.
            op::STORE | op::STORE_REF => {}
            op::CALL => {
                let target = match ins.mode {
                    CALL_PROC => Target::Proc(ins.opnd),
                    _ => Target::Builtin(ins.opnd),
                };
                out.push(pop_call(&mut stack, &mut frames, target));
            }
            op::IMPORT => {
                out.push(pop_call(&mut stack, &mut frames, Target::Import));
            }
            // decl / jmp / brf / ret and callframe-less flow: no arg-stack effect.
            _ => {}
        }
    }
    out
}

fn pop_call(stack: &mut Vec<Val>, frames: &mut Vec<usize>, target: Target) -> CallSite {
    let start = frames.pop().unwrap_or(0).min(stack.len());
    let args = stack.split_off(start);
    stack.push(Val::Unknown); // call result (harmless for void statement calls)
    CallSite { target, args }
}

fn push_val(m: u8, opnd: u16, consts: &[Const]) -> Val {
    // A plain `push CONST`/`push GLOBAL` both index the constant/data pool: CONST is an
    // inline literal, GLOBAL reads an initialised global whose value lives in the same pool
    // (INPA compiles some string literals — e.g. the `Info` page values — as GLOBAL reads).
    // Resolve either from the pool; fall back to an opaque Global only when nothing is there
    // (a real, uninitialised variable). `pushref GLOBAL` (an address) stays opaque elsewhere.
    let from_pool = |opnd: u16| match consts.get(opnd as usize) {
        Some(Const::Str(s)) => Some(Val::Str(s.clone())),
        Some(Const::Int(i)) => Some(Val::Num(*i as f64)),
        Some(Const::Long(i)) => Some(Val::Num(*i as f64)),
        Some(Const::Byte(b)) => Some(Val::Num(*b as f64)),
        Some(Const::Real(r)) => Some(Val::Num(*r)),
        Some(Const::Bool(b)) => Some(Val::Bool(*b)),
        None => None,
    };
    match m {
        mode::CONST => from_pool(opnd).unwrap_or(Val::Unknown),
        mode::GLOBAL => from_pool(opnd).unwrap_or(Val::Global(opnd)),
        mode::LOCAL => Val::Local(opnd),
        _ => Val::Unknown,
    }
}

fn apply_alu(stack: &mut Vec<Val>, sub: u8) {
    match sub {
        0x60 => {
            // add / string concat
            let b = stack.pop();
            let a = stack.pop();
            match (a, b) {
                (Some(Val::Str(x)), Some(Val::Str(y))) => stack.push(Val::Str(x + &y)),
                _ => stack.push(Val::Unknown),
            }
        }
        0x6C | 0x6D => {
            // unary not / neg
            stack.pop();
            stack.push(Val::Unknown);
        }
        _ => {
            // binary comparison / arithmetic we don't need to resolve
            stack.pop();
            stack.pop();
            stack.push(Val::Unknown);
        }
    }
}
