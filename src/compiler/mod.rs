//! The compiler (design note, section 1.2): a parsed program into a
//! [`CompiledUnit`] of register bytecode, which holds no heap reference
//! (constants, scope descriptors, source positions), so that it can be
//! made off the thread that runs it and kept apart from any agent.
//!
//! Two passes over J1's tree:
//!
//! - [`scope`] resolves every name, iteratively: each binding becomes a
//!   register of its function's frame, a slot of an environment when a
//!   closure captures it or a direct `eval` or `with` can reach it, or a
//!   global looked up by name;
//! - [`codegen`] emits each function's bytecode. Functions are compiled
//!   from a queue, never by recursion; within one, expressions recurse
//!   only where the parser recursed (brackets, functions, statements),
//!   and the left-deep and right-deep chains the parser builds in loops
//!   (operators, member accesses and calls, unary prefixes) are walked
//!   with a stack of the compiler's own. Every level checks the same
//!   stack budget the parser had, from the compiler's entry (the lead,
//!   2026-11-20), so that the compiler is never the one to refuse a
//!   source first.
//!
//! What J2 does not run yet (classes, generators, `async`, destructuring,
//! spread, iterators, tagged templates, modules) is refused with
//! [`CompileError::Unsupported`], which later steps of the plan shrink to
//! nothing.

mod bytecode;
mod codegen;
mod scope;

use std::fmt::Write as _;

pub use bytecode::{Instr, Op, Operand, decode};

use crate::SyntaxError;
use crate::ast::Program;
use crate::parser::{DEFAULT_STACK_BUDGET, Goal, parse_with_stack};

/// Why a source did not compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    /// It does not parse, or it nests deeper than the stack budget.
    Syntax(SyntaxError),
    /// It uses what this version of the engine does not run yet.
    Unsupported {
        feature: &'static str,
        start: u32,
        end: u32,
    },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Syntax(e) => e.fmt(f),
            CompileError::Unsupported { feature, .. } => write!(f, "not supported yet: {feature}"),
        }
    }
}

impl std::error::Error for CompileError {}

impl From<SyntaxError> for CompileError {
    fn from(e: SyntaxError) -> CompileError {
        CompileError::Syntax(e)
    }
}

/// A constant of a function's pool.
#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Number(f64),
    /// A string, or a name (of a binding or a property), in UTF-16.
    String(Box<[u16]>),
    BigInt {
        radix: u32,
        digits: Box<str>,
    },
}

/// What an environment of a scope holds: each slot's name (for lookups
/// by name, from `eval` and `with`) and whether it starts uninitialized
/// (a `let`, `const` or class binding) rather than undefined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeInfo {
    pub names: Vec<Box<[u16]>>,
    pub lexical: Vec<bool>,
    /// Per slot: 0 assignable, 1 a `const` (an assignment throws), 2 a
    /// named function expression's own name (strict code's assignment
    /// throws, sloppy code's does nothing).
    pub constant: Vec<u8>,
}

/// A try statement's handler: an exception thrown by an instruction in
/// `start..end` jumps to `target` with the exception in the accumulator,
/// the environment restored from `context` (a register), or to the
/// function's own when `None`. Inner handlers come first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handler {
    pub start: u32,
    pub end: u32,
    pub target: u32,
    pub context: Option<u32>,
}

/// From `pc` on, the code being run is the source's `start..end` (a
/// statement, or the expression an instruction that may throw
/// evaluates: a call's callee, a property access).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub pc: u32,
    pub start: u32,
    pub end: u32,
    /// Where a stack frame stopped here points, as the reference
    /// browser's does: a call's callee name (the property's, for
    /// `o.m()`), a property access's name, a construction's `new`.
    pub at: u32,
}

/// What a function is, for calls and `new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    /// A script's top-level code.
    Script,
    /// A function declaration or expression: callable and constructible.
    Normal,
    Arrow,
    /// An object literal's method, getter or setter: not constructible.
    Method,
    /// A generator function or method: its call makes a generator.
    Generator,
    /// A class's constructor: constructible only, never called.
    ClassConstructor,
    /// A derived class's constructor: its `this` made by `super()`.
    DerivedConstructor,
}

/// One function's code.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionCode {
    /// Its name, declared or inferred (`var f = function () {}`); empty
    /// for none.
    pub name: Box<[u16]>,
    pub kind: FunctionKind,
    pub strict: bool,
    /// `length`: the parameters before the first default or rest.
    pub length: u32,
    /// The parameters' registers, `0..param_count`, which a call fills
    /// with its arguments.
    pub param_count: u32,
    /// All of the frame's registers: parameters, bindings, temporaries.
    pub register_count: u32,
    pub code: Vec<u8>,
    pub constants: Vec<Constant>,
    pub handlers: Vec<Handler>,
    pub positions: Vec<Position>,
    /// Inline-cache slots the code's instructions name.
    pub ic_slots: u32,
    /// The source text, `start..end`: `Function.prototype.toString`'s.
    pub span: (u32, u32),
    /// For a mapped arguments object: per parameter, one more than the
    /// slot of the function's environment it is mapped to, or 0 (a
    /// repeated name maps its last parameter only).
    pub mapped: Vec<u32>,
}

/// A top-level declaration of a script, which the interpreter makes
/// before running it (ECMA-262 GlobalDeclarationInstantiation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalDecl {
    Var(Box<[u16]>),
    /// A function declaration: its name and its function in the unit.
    Function(Box<[u16]>, u32),
    Let(Box<[u16]>),
    Const(Box<[u16]>),
}

/// A compiled script.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledUnit {
    /// Function 0 is the script's top-level code.
    pub functions: Vec<FunctionCode>,
    pub scopes: Vec<ScopeInfo>,
    /// A script's top-level declarations; eval code's, those a sloppy
    /// eval makes in its caller's variable environment.
    pub globals: Vec<GlobalDecl>,
    pub source: Box<str>,
    /// Eval code ([`compile_eval`]).
    pub eval: bool,
}

/// Parses and compiles `source` within [`DEFAULT_STACK_BUDGET`] of the
/// caller's stack.
pub fn compile(source: &str, goal: Goal) -> Result<CompiledUnit, CompileError> {
    compile_with_stack(source, goal, DEFAULT_STACK_BUDGET)
}

/// Compiles eval code: strict if its caller is (or it says so), with
/// `new.target` allowed in a function's; a name it does not declare is
/// looked up by name at run time, and a sloppy one's `var`s and
/// functions are made in its caller's variable environment
/// ([`CompiledUnit::eval`]).
pub fn compile_eval(
    source: &str,
    strict: bool,
    new_target: bool,
) -> Result<CompiledUnit, CompileError> {
    let program = crate::parser::parse_eval(source, strict, new_target, DEFAULT_STACK_BUDGET)?;
    let analysis = scope::analyze_eval(&program);
    codegen::generate(&program, &analysis, source, DEFAULT_STACK_BUDGET)
}

/// Parses with `stack_budget`, then compiles with the same budget, from
/// the compiler's own entry.
pub fn compile_with_stack(
    source: &str,
    goal: Goal,
    stack_budget: usize,
) -> Result<CompiledUnit, CompileError> {
    let program = parse_with_stack(source, goal, stack_budget)?;
    compile_program(&program, source, stack_budget)
}

/// Parses and compiles as [`compile_with_stack`] does, and says the most
/// stack each pass used past its entry: the parse's and the compiler's.
#[cfg(test)]
pub(crate) fn compile_with_stats(
    source: &str,
    goal: Goal,
    stack_budget: usize,
) -> Result<(CompiledUnit, crate::parser::ParseStats, usize), CompileError> {
    let (program, stats) = crate::parser::parse_with_stats(source, goal, stack_budget);
    let program = program?;
    let analysis = scope::analyze(&program);
    let (unit, peak) = codegen::generate_with_stats(&program, &analysis, source, stack_budget)?;
    Ok((unit, stats, peak))
}

/// Encodes one instruction, for the encoding tests.
#[cfg(test)]
pub(crate) fn bytecode_encode_for_tests(code: &mut Vec<u8>, op: Op, operands: &[i64]) {
    bytecode::encode(code, op, operands);
}

/// Compiles a program parsed from `source`.
pub fn compile_program(
    program: &Program,
    source: &str,
    stack_budget: usize,
) -> Result<CompiledUnit, CompileError> {
    if program.module {
        return Err(CompileError::Unsupported {
            feature: "modules",
            start: program.span.start,
            end: program.span.end,
        });
    }
    let analysis = scope::analyze(program);
    codegen::generate(program, &analysis, source, stack_budget)
}

fn utf16_text(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

impl FunctionCode {
    /// The code as text, an instruction a line, for tests and debugging:
    ///
    /// ```text
    ///     0 LdaSmi 1
    ///     2 Star r0
    ///     4 GetNamed r0, [1] "x", ic0
    /// ```
    pub fn disassemble(&self) -> String {
        let mut out = String::new();
        let mut pc = 0u32;
        while (pc as usize) < self.code.len() {
            let Some(instr) = decode(&self.code, pc) else {
                let _ = writeln!(out, "{pc:>5} <bad byte>");
                break;
            };
            let _ = write!(out, "{pc:>5} {}", instr.op.name());
            let kinds = instr.op.operands();
            for (i, (value, kind)) in instr.operands.iter().zip(kinds).enumerate() {
                out.push_str(if i == 0 { " " } else { ", " });
                self.operand(&mut out, instr, i, *value, *kind);
            }
            out.push('\n');
            pc += instr.len;
        }
        for h in &self.handlers {
            let _ = write!(out, "handler {}..{} -> {}", h.start, h.end, h.target);
            if let Some(r) = h.context {
                let _ = write!(out, " (context r{r})");
            }
            out.push('\n');
        }
        out
    }

    fn operand(&self, out: &mut String, instr: Instr, i: usize, value: i64, kind: Operand) {
        match kind {
            Operand::Reg => {
                let _ = write!(out, "r{value}");
            }
            Operand::Imm => {
                let _ = write!(out, "{value}");
            }
            Operand::Jump => {
                let _ = write!(out, "-> {}", i64::from(instr.pc) + value);
            }
            Operand::Idx => match idx_kind(instr.op, i) {
                IdxKind::Const | IdxKind::Name => {
                    let _ = write!(out, "[{value}]");
                    match usize::try_from(value)
                        .ok()
                        .and_then(|v| self.constants.get(v))
                    {
                        Some(Constant::String(s)) => {
                            let _ = write!(out, " {:?}", utf16_text(s));
                        }
                        Some(Constant::Number(n)) => {
                            let _ = write!(out, " {n}");
                        }
                        Some(Constant::BigInt { radix, digits }) => {
                            let _ = write!(out, " {digits}n/{radix}");
                        }
                        None => out.push_str(" ?"),
                    }
                }
                IdxKind::Ic => {
                    let _ = write!(out, "ic{value}");
                }
                IdxKind::Scope => {
                    let _ = write!(out, "scope{value}");
                }
                IdxKind::Func => {
                    let _ = write!(out, "fn{value}");
                }
                IdxKind::Plain => {
                    let _ = write!(out, "{value}");
                }
            },
        }
    }
}

/// What an index operand names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdxKind {
    /// A constant of any kind.
    Const,
    /// A string constant used as a name.
    Name,
    Ic,
    Scope,
    Func,
    /// A count, a depth, a slot, an element index, a flag.
    Plain,
}

fn idx_kind(op: Op, i: usize) -> IdxKind {
    match (op, i) {
        (Op::LdaConst, 0) => IdxKind::Const,
        (
            Op::CheckTdz
            | Op::ThrowConstAssign
            | Op::LdaGlobal
            | Op::LdaGlobalTypeof
            | Op::StaGlobal
            | Op::InitGlobalLexical
            | Op::LdaDynamic
            | Op::LdaDynamicTypeof
            | Op::StaDynamic
            | Op::DeleteName
            | Op::CreateRegExp,
            0,
        )
        | (
            Op::CheckTdzRegister | Op::DefineField | Op::GetNamed | Op::SetNamed | Op::CreateRegExp,
            1,
        )
        | (Op::CheckTdzSlot, 2)
        | (Op::RequireObjectCoercible, 0) => IdxKind::Name,
        (Op::LdaGlobal | Op::LdaGlobalTypeof | Op::StaGlobal | Op::GetKeyed, 1)
        | (Op::GetNamed | Op::SetNamed | Op::SetKeyed, 2)
        | (Op::CallUndefined | Op::New, 3)
        | (Op::Call, 4) => IdxKind::Ic,
        (Op::PushContext, 1) | (Op::CreateFunctionContext, 0) => IdxKind::Scope,
        (Op::CreateClosure, 0) => IdxKind::Func,
        _ => IdxKind::Plain,
    }
}

impl CompiledUnit {
    /// Every function's disassembly, each under a header line.
    pub fn disassemble(&self) -> String {
        let mut out = String::new();
        for (i, f) in self.functions.iter().enumerate() {
            let _ = writeln!(
                out,
                "function {i} {:?} ({:?}{}, params {}, registers {}, ics {})",
                utf16_text(&f.name),
                f.kind,
                if f.strict { ", strict" } else { "" },
                f.param_count,
                f.register_count,
                f.ic_slots
            );
            out.push_str(&f.disassemble());
        }
        out
    }
}

impl CompiledUnit {
    /// Checks the unit's structure: every instruction decodes, every jump
    /// and handler lands on an instruction, every register, constant,
    /// name, inline-cache slot, scope and function an operand names
    /// exists, and no function's code runs off its end. What the
    /// compiler writes always passes; a unit read from bytes must pass
    /// before it is run.
    pub fn validate(&self) -> Result<(), String> {
        let functions = self.functions.len();
        for g in &self.globals {
            if let GlobalDecl::Function(_, i) = g
                && (*i == 0 || *i as usize >= functions)
            {
                return Err(format!("a global function names function {i}"));
            }
        }
        for (fi, f) in self.functions.iter().enumerate() {
            let fail = |what: String| Err(format!("function {fi}: {what}"));
            let mut starts = std::collections::HashSet::new();
            let mut jumps = Vec::new();
            let mut pc = 0u32;
            let mut last = None;
            while (pc as usize) < f.code.len() {
                let Some(instr) = decode(&f.code, pc) else {
                    return fail(format!("no instruction at {pc}"));
                };
                starts.insert(pc);
                // A call's arguments are a range of registers, empty for
                // none: its first and its count.
                let args = match instr.op {
                    Op::Call => Some((2, 3)),
                    Op::CallUndefined | Op::New | Op::CallEval => Some((1, 2)),
                    _ => None,
                };
                if let Some((first, count)) = args {
                    let end = instr.operands.get(first).copied().unwrap_or(0)
                        + instr.operands.get(count).copied().unwrap_or(0);
                    let empty = instr.operands.get(count) == Some(&0);
                    if !empty && end > i64::from(f.register_count) {
                        return fail(format!(
                            "{} at {pc}: arguments past the frame",
                            instr.op.name()
                        ));
                    }
                }
                for (i, (value, kind)) in instr.operands.iter().zip(instr.op.operands()).enumerate()
                {
                    let bad = match kind {
                        Operand::Reg if args.is_some_and(|(first, _)| first == i) => false,
                        Operand::Reg => *value >= i64::from(f.register_count),
                        Operand::Jump => {
                            jumps.push((pc, instr.target()));
                            false
                        }
                        Operand::Imm => false,
                        Operand::Idx => match idx_kind(instr.op, i) {
                            IdxKind::Const => {
                                usize::try_from(*value).map_or(true, |v| v >= f.constants.len())
                            }
                            IdxKind::Name => !matches!(
                                usize::try_from(*value)
                                    .ok()
                                    .and_then(|v| f.constants.get(v)),
                                Some(Constant::String(_))
                            ),
                            IdxKind::Ic => *value >= i64::from(f.ic_slots),
                            IdxKind::Scope => {
                                usize::try_from(*value).map_or(true, |v| v >= self.scopes.len())
                            }
                            IdxKind::Func => {
                                usize::try_from(*value).map_or(true, |v| v >= functions || v == 0)
                            }
                            IdxKind::Plain => false,
                        },
                    };
                    if bad {
                        return fail(format!(
                            "{} at {pc}: operand {i} is {value}",
                            instr.op.name()
                        ));
                    }
                }
                last = Some(instr.op);
                pc += instr.len;
            }
            if !matches!(last, Some(Op::Return | Op::Throw | Op::Jump | Op::JumpLoop)) {
                return fail("the code runs off its end".into());
            }
            for (from, to) in jumps {
                if !to.is_some_and(|t| starts.contains(&t)) {
                    return fail(format!("the jump at {from} lands off an instruction"));
                }
            }
            for h in &f.handlers {
                let bad_context = h.context.is_some_and(|r| r >= f.register_count);
                if h.start > h.end || !starts.contains(&h.target) || bad_context {
                    return fail(format!("a bad handler {h:?}"));
                }
            }
            if f.positions
                .windows(2)
                .any(|w| w.first().map(|p| p.pc) >= w.get(1).map(|p| p.pc))
            {
                return fail("positions out of order".into());
            }
            if f.param_count > f.register_count {
                return fail("more parameters than registers".into());
            }
        }
        Ok(())
    }
}
