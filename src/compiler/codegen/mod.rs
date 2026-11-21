//! Bytecode generation, a function at a time.
//!
//! The functions are compiled in the analysis's order, in which a
//! function comes after the one that makes it, so that a name inferred
//! where a closure is made (`var f = function () {}`) is known when the
//! function's turn comes: a loop, never a recursion over functions.
//! Within a function, expressions and statements recurse only where the
//! parser did, each level checking the stack budget (see the module
//! above).

mod expr;
mod stmt;

use std::collections::HashMap;

use super::bytecode::{Op, encode, patch_jump};
use super::scope::{Analysis, BindingKind, Loc, Place, Resolved, ScopeId, ScopeKey, ScopeKind};
use super::{
    CompileError, CompiledUnit, Constant, FunctionCode, FunctionKind, GlobalDecl, Handler,
    Position, ScopeInfo,
};
use crate::SyntaxError;
use crate::ast::{Ast, Body, FuncId, FuncKind, Pat, Program, Span};
use crate::stack::StackGuard;

type R<T = ()> = Result<T, CompileError>;

pub(super) fn units(s: &str) -> Box<[u16]> {
    s.encode_utf16().collect()
}

pub(super) fn unsupported(feature: &'static str, span: Span) -> CompileError {
    CompileError::Unsupported {
        feature,
        start: span.start,
        end: span.end,
    }
}

/// What the whole unit shares while its functions are compiled.
struct Shared<'a> {
    program: &'a Program,
    a: &'a Analysis,
    stack: StackGuard,
    peak_stack: usize,
    scopes: Vec<ScopeInfo>,
    scope_index: HashMap<ScopeId, u32>,
    /// Each function's name and kind, as its maker found them.
    names: Vec<Option<(Box<[u16]>, FunctionKind)>>,
}

impl Shared<'_> {
    fn scope_info(&mut self, s: ScopeId) -> u32 {
        if let Some(i) = self.scope_index.get(&s) {
            return *i;
        }
        let scope = self.a.scope(s);
        let info = ScopeInfo {
            names: scope.slots.iter().map(|(n, _, _)| units(n)).collect(),
            lexical: scope.slots.iter().map(|(_, l, _)| *l).collect(),
            constant: scope.slots.iter().map(|(_, _, c)| *c).collect(),
        };
        let i = u32::try_from(self.scopes.len()).unwrap_or(u32::MAX);
        self.scopes.push(info);
        self.scope_index.insert(s, i);
        i
    }
}

/// Compiles every function of an analyzed script.
pub(super) fn generate(
    program: &Program,
    a: &Analysis,
    source: &str,
    stack_budget: usize,
) -> R<CompiledUnit> {
    Ok(generate_with_stats(program, a, source, stack_budget)?.0)
}

/// As [`generate`], and the most stack the compiler used past its entry.
pub(super) fn generate_with_stats(
    program: &Program,
    a: &Analysis,
    source: &str,
    stack_budget: usize,
) -> R<(CompiledUnit, usize)> {
    let mut shared = Shared {
        program,
        a,
        stack: StackGuard::new(stack_budget),
        peak_stack: 0,
        scopes: Vec::new(),
        scope_index: HashMap::new(),
        names: vec![None; a.functions.len()],
    };
    let mut functions = Vec::with_capacity(a.functions.len());
    for fi in 0..a.functions.len() {
        let index = u32::try_from(fi).unwrap_or(u32::MAX);
        functions.push(Gen::new(&mut shared, index).function()?);
    }
    let globals = globals(program, a);
    let peak = shared.peak_stack;
    Ok((
        CompiledUnit {
            functions,
            scopes: shared.scopes,
            globals,
            source: source.into(),
            eval: a.scopes.first().is_some_and(|s| s.kind == ScopeKind::Eval),
        },
        peak,
    ))
}

/// The script's top-level declarations.
fn globals(program: &Program, a: &Analysis) -> Vec<GlobalDecl> {
    let _ = program;
    let Some(top) = a.key(ScopeKey::Program) else {
        return Vec::new();
    };
    let eval = a.scope(top).kind == ScopeKind::Eval;
    a.scope(top)
        .bindings
        .iter()
        .filter(|b| !eval || b.loc == Loc::Dynamic)
        .map(|b| {
            let name = units(&b.name);
            match (b.kind, b.func) {
                (_, Some(f)) => {
                    GlobalDecl::Function(name, a.func_index.get(&f).copied().unwrap_or(0))
                }
                (BindingKind::Let | BindingKind::Class, _) => GlobalDecl::Let(name),
                (BindingKind::Const, _) => GlobalDecl::Const(name),
                _ => GlobalDecl::Var(name),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Label(u32);

/// A scope entered: the register its outer environment is saved in, if
/// it pushed one, and the scope to go back to.
#[derive(Debug, Clone, Copy)]
pub(super) struct Scoped {
    pub save: Option<u32>,
    pub outer: ScopeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BreakKind {
    Loop,
    Switch,
    Block,
}

/// Where a `break`, `continue` or `return` goes, past what it leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Completion {
    Break(usize),
    Continue(usize),
    Return,
}

/// What a jump out of the code being compiled must pass through.
#[derive(Debug, Clone)]
pub(super) enum Control {
    Breakable {
        labels: Vec<Box<str>>,
        kind: BreakKind,
        break_to: Label,
        continue_to: Option<Label>,
    },
    /// An environment pushed, the one before it saved in `save`.
    Context { save: u32 },
    /// A `finally` block, run with `token` saying how it was entered: 0
    /// falling through, 1 by an exception (in `value`), 2 and on by the
    /// `completions` in order (a return's value in `value`).
    Finally {
        token: u32,
        value: u32,
        entry: Label,
        completions: Vec<Completion>,
    },
}

pub(super) struct Gen<'s, 'a> {
    sh: &'s mut Shared<'a>,
    pub(super) ast: &'a Ast,
    pub(super) a: &'a Analysis,
    index: u32,
    pub(super) strict: bool,
    code: Vec<u8>,
    constants: Vec<Constant>,
    strings: HashMap<Box<[u16]>, u32>,
    numbers: HashMap<u64, u32>,
    handlers: Vec<Handler>,
    positions: Vec<Position>,
    ic: u32,
    labels: Vec<Option<u32>>,
    patches: Vec<(usize, u32, Label)>,
    next_reg: u32,
    max_reg: u32,
    pub(super) scope: ScopeId,
    pub(super) control: Vec<Control>,
    /// The script's completion value, in a register (scripts only).
    pub(super) completion: Option<u32>,
    /// Labels waiting for the loop they name.
    pub(super) pending_labels: Vec<Box<str>>,
}

impl<'s, 'a> Gen<'s, 'a> {
    fn new(sh: &'s mut Shared<'a>, index: u32) -> Gen<'s, 'a> {
        let a = sh.a;
        let info = a.function(index);
        Gen {
            ast: &sh.program.ast,
            a,
            index,
            strict: info.strict,
            code: Vec::new(),
            constants: Vec::new(),
            strings: HashMap::new(),
            numbers: HashMap::new(),
            handlers: Vec::new(),
            positions: Vec::new(),
            ic: 0,
            labels: Vec::new(),
            patches: Vec::new(),
            next_reg: info.registers,
            max_reg: info.registers,
            scope: info.scope,
            control: Vec::new(),
            completion: None,
            pending_labels: Vec::new(),
            sh,
        }
    }

    // The stack.

    /// Refuses the source where the stack budget is spent ("Maximum call
    /// stack size exceeded", as the parser words it).
    pub(super) fn check(&mut self, span: Span) -> R {
        let used = self.sh.stack.used();
        self.sh.peak_stack = self.sh.peak_stack.max(used);
        if self.sh.stack.exceeded() {
            return Err(CompileError::Syntax(SyntaxError::new(
                "Maximum call stack size exceeded",
                span.start as usize,
                span.end as usize,
            )));
        }
        Ok(())
    }

    // Emitting.

    pub(super) fn pc(&self) -> u32 {
        u32::try_from(self.code.len()).unwrap_or(u32::MAX)
    }

    pub(super) fn emit(&mut self, op: Op, operands: &[i64]) {
        encode(&mut self.code, op, operands);
    }

    pub(super) fn op(&mut self, op: Op) {
        self.emit(op, &[]);
    }

    pub(super) fn reg_op(&mut self, op: Op, r: u32) {
        self.emit(op, &[i64::from(r)]);
    }

    pub(super) fn new_label(&mut self) -> Label {
        let l = Label(u32::try_from(self.labels.len()).unwrap_or(u32::MAX));
        self.labels.push(None);
        l
    }

    pub(super) fn bind(&mut self, l: Label) {
        let pc = self.pc();
        if let Some(slot) = self.labels.get_mut(l.0 as usize) {
            *slot = Some(pc);
        }
    }

    pub(super) fn jump(&mut self, op: Op, l: Label) {
        let start = self.pc();
        let target = self.labels.get(l.0 as usize).copied().flatten();
        let offset = target.map_or(0, |t| i64::from(t) - i64::from(start));
        if let Some(at) = encode(&mut self.code, op, &[offset])
            && target.is_none()
        {
            self.patches.push((at, start, l));
        }
    }

    pub(super) fn ic(&mut self) -> i64 {
        self.ic += 1;
        i64::from(self.ic - 1)
    }

    /// Records the source being run from here on.
    pub(super) fn pos(&mut self, span: Span) {
        self.pos_at(span, span.start);
    }

    /// The position of the next instruction: `span` for the text an error
    /// quotes, `at` for a stack frame's column.
    pub(super) fn pos_at(&mut self, span: Span, at: u32) {
        let pc = self.pc();
        let p = Position {
            pc,
            start: span.start,
            end: span.end,
            at,
        };
        match self.positions.last_mut() {
            Some(last) if last.pc == pc => *last = p,
            Some(last) if last.start == p.start && last.end == p.end && last.at == at => {}
            _ => self.positions.push(p),
        }
    }

    pub(super) fn handler(&mut self, h: Handler) {
        self.handlers.push(h);
    }

    // Registers: temporaries are taken and given back last first.

    pub(super) fn alloc(&mut self) -> u32 {
        let r = self.next_reg;
        self.next_reg += 1;
        self.max_reg = self.max_reg.max(self.next_reg);
        r
    }

    /// `n` consecutive registers; the first.
    pub(super) fn alloc_n(&mut self, n: u32) -> u32 {
        let r = self.next_reg;
        self.next_reg += n;
        self.max_reg = self.max_reg.max(self.next_reg);
        r
    }

    pub(super) fn free(&mut self, r: u32) {
        debug_assert_eq!(r + 1, self.next_reg, "registers are given back last first");
        self.next_reg = r;
    }

    pub(super) fn free_n(&mut self, first: u32, n: u32) {
        debug_assert_eq!(
            first + n,
            self.next_reg,
            "registers are given back last first"
        );
        self.next_reg = first;
    }

    // Constants.

    pub(super) fn string(&mut self, s: &[u16]) -> i64 {
        if let Some(i) = self.strings.get(s) {
            return i64::from(*i);
        }
        let i = u32::try_from(self.constants.len()).unwrap_or(u32::MAX);
        self.constants.push(Constant::String(s.into()));
        self.strings.insert(s.into(), i);
        i64::from(i)
    }

    pub(super) fn name(&mut self, s: &str) -> i64 {
        self.string(&units(s))
    }

    pub(super) fn number(&mut self, n: f64) -> i64 {
        let bits = n.to_bits();
        if let Some(i) = self.numbers.get(&bits) {
            return i64::from(*i);
        }
        let i = u32::try_from(self.constants.len()).unwrap_or(u32::MAX);
        self.constants.push(Constant::Number(n));
        self.numbers.insert(bits, i);
        i64::from(i)
    }

    pub(super) fn constant(&mut self, c: Constant) -> i64 {
        let i = u32::try_from(self.constants.len()).unwrap_or(u32::MAX);
        self.constants.push(c);
        i64::from(i)
    }

    pub(super) fn scope_info(&mut self, s: ScopeId) -> i64 {
        i64::from(self.sh.scope_info(s))
    }

    /// Records the name and kind a function is made with, for its turn.
    pub(super) fn name_function(&mut self, index: u32, name: Box<[u16]>, kind: FunctionKind) {
        if let Some(slot) = self.sh.names.get_mut(index as usize) {
            *slot = Some((name, kind));
        }
    }

    // Names.

    pub(super) fn resolve(&self, name: &str) -> Resolved {
        self.a.resolve(self.scope, name)
    }

    /// Whether a read of a binding at `at` must check its TDZ.
    pub(super) fn needs_tdz(r: &Resolved, at: u32) -> bool {
        r.kind.is_some_and(BindingKind::lexical)
            && !(r.same_function && !r.in_switch && at >= r.tdz_end)
    }

    /// Stores the accumulator into a binding of the current scope at its
    /// entry (depth 0).
    pub(super) fn store_loc(&mut self, loc: Loc) {
        match loc {
            Loc::Register(r) => self.reg_op(Op::Star, r),
            Loc::Slot(k) => self.emit(Op::StaSlot, &[0, i64::from(k)]),
            Loc::Global | Loc::Intrinsic | Loc::Unassigned | Loc::Dynamic => {}
        }
    }

    /// Enters a block's scope: its environment, if it has one, its
    /// lexical bindings in registers set uninitialized, and its function
    /// declarations made. A node without a scope of its own enters
    /// nothing.
    pub(super) fn enter_scope(&mut self, key: ScopeKey) -> R<Scoped> {
        let outer = self.scope;
        let Some(s) = self.a.key(key) else {
            return Ok(Scoped { save: None, outer });
        };
        self.scope = s;
        let save = if self.a.scope(s).has_env {
            let r = self.alloc();
            let info = self.scope_info(s);
            self.emit(Op::PushContext, &[i64::from(r), info]);
            self.control.push(Control::Context { save: r });
            Some(r)
        } else {
            None
        };
        self.init_bindings(s)?;
        Ok(Scoped { save, outer })
    }

    pub(super) fn leave_scope(&mut self, scoped: Scoped) {
        if let Some(r) = scoped.save {
            self.reg_op(Op::PopContext, r);
            self.control.pop();
            self.free(r);
        }
        self.scope = scoped.outer;
    }

    /// A scope's lexical register bindings uninitialized, and its
    /// function declarations made.
    fn init_bindings(&mut self, s: ScopeId) -> R {
        let scope = self.a.scope(s);
        for b in &scope.bindings {
            if b.kind.lexical()
                && let Loc::Register(r) = b.loc
            {
                self.op(Op::LdaUninitialized);
                self.reg_op(Op::Star, r);
            }
        }
        for b in &scope.bindings {
            if let Some(f) = b.func {
                self.closure(f, None)?;
                self.store_loc(b.loc);
            }
        }
        Ok(())
    }

    // A function.

    fn function(mut self) -> R<FunctionCode> {
        let info = self.a.function(self.index).clone();
        let mut mapped = Vec::new();
        let (span, kind, name, length) = match info.func {
            None => {
                let span = self.sh.program.span;
                self.script()?;
                (
                    (span.start, span.end),
                    FunctionKind::Script,
                    Box::default(),
                    0,
                )
            }
            Some(fid) => {
                let f = self.ast.func(fid);
                let (name, kind) = match self.sh.names.get(self.index as usize).cloned().flatten() {
                    Some(nk) => nk,
                    None => (
                        f.name.as_ref().map(|n| units(&n.name)).unwrap_or_default(),
                        FunctionKind::Normal,
                    ),
                };
                let length = f.params.iter().take_while(|p| p.default.is_none()).count();
                if info.mapped_arguments {
                    let mut map = vec![0u32; info.param_count as usize];
                    for b in &self.a.scope(self.scope).bindings {
                        if let (BindingKind::Param, Loc::Slot(k), Some(p)) =
                            (b.kind, b.loc, b.param)
                            && let Some(m) = map.get_mut(p as usize)
                        {
                            *m = k + 1;
                        }
                    }
                    mapped = map;
                }
                self.body(fid)?;
                (
                    (f.span.start, f.span.end),
                    kind,
                    name,
                    u32::try_from(length).unwrap_or(u32::MAX),
                )
            }
        };
        let mut code = std::mem::take(&mut self.code);
        for (at, start, l) in &self.patches {
            if let Some(Some(target)) = self.labels.get(l.0 as usize) {
                let offset = i64::from(*target) - i64::from(*start);
                patch_jump(&mut code, *at, i32::try_from(offset).unwrap_or(0));
            }
        }
        Ok(FunctionCode {
            name,
            kind,
            strict: self.strict,
            length,
            param_count: info.param_count,
            register_count: self.max_reg,
            code,
            constants: std::mem::take(&mut self.constants),
            handlers: std::mem::take(&mut self.handlers),
            positions: std::mem::take(&mut self.positions),
            ic_slots: self.ic,
            span,
            mapped,
        })
    }

    fn script(&mut self) -> R {
        let program = self.sh.program;
        let c = self.alloc();
        self.completion = Some(c);
        let top = self.scope;
        if self.a.scope(top).kind == ScopeKind::Eval {
            self.eval_prologue(top)?;
        }
        self.stmts(&program.body)?;
        self.reg_op(Op::Ldar, c);
        self.op(Op::Return);
        Ok(())
    }

    /// Eval code's entry: its environment, its lexical bindings in
    /// registers uninitialized, its functions made (a sloppy one's into
    /// its caller's variable environment, by name).
    fn eval_prologue(&mut self, top: ScopeId) -> R {
        let scope = self.a.scope(top);
        if scope.has_env {
            let info = self.scope_info(top);
            self.emit(Op::CreateFunctionContext, &[info]);
        }
        for b in &scope.bindings {
            match (b.kind, b.loc) {
                (BindingKind::This, Loc::Slot(k)) => {
                    self.op(Op::LdaThis);
                    self.emit(Op::StaSlot, &[0, i64::from(k)]);
                }
                (BindingKind::NewTarget, Loc::Slot(k)) => {
                    self.op(Op::LdaNewTarget);
                    self.emit(Op::StaSlot, &[0, i64::from(k)]);
                }
                (kind, Loc::Register(r)) if kind.lexical() => {
                    self.op(Op::LdaUninitialized);
                    self.reg_op(Op::Star, r);
                }
                _ => {}
            }
        }
        for b in &scope.bindings {
            if let Some(g) = b.func {
                self.closure(g, None)?;
                if b.loc == Loc::Dynamic {
                    let n = self.name(&b.name);
                    self.emit(Op::StaDynamic, &[n]);
                } else {
                    self.store_loc(b.loc);
                }
            }
        }
        Ok(())
    }

    fn body(&mut self, fid: FuncId) -> R {
        let f = self.ast.func(fid);
        if f.is_generator {
            return Err(unsupported("generators", f.span));
        }
        if f.is_async {
            return Err(unsupported("async functions", f.span));
        }
        if matches!(
            f.kind,
            FuncKind::Constructor { .. } | FuncKind::ClassField | FuncKind::StaticBlock
        ) {
            return Err(unsupported("classes", f.span));
        }
        let fs = self.scope;
        let scope = self.a.scope(fs);
        if scope.has_env {
            let info = self.scope_info(fs);
            self.emit(Op::CreateFunctionContext, &[info]);
        }
        // Its own name, for a named function expression.
        if let Some(ns) = self.a.key(ScopeKey::FuncName(fid))
            && let Some(b) = self.a.scope(ns).bindings.first()
        {
            self.op(Op::LdaClosure);
            self.store_loc(b.loc);
        }
        let mapped = self.a.function(self.index).mapped_arguments;
        let param_count = self.a.function(self.index).param_count;
        for b in &scope.bindings {
            match (b.kind, b.loc) {
                (BindingKind::Param, Loc::Slot(k)) => {
                    if let Some(p) = b.param {
                        self.reg_op(Op::Ldar, p);
                        self.emit(Op::StaSlot, &[0, i64::from(k)]);
                    }
                }
                (BindingKind::This, Loc::Slot(k)) => {
                    self.op(Op::LdaThis);
                    self.emit(Op::StaSlot, &[0, i64::from(k)]);
                }
                (BindingKind::NewTarget, Loc::Slot(k)) => {
                    self.op(Op::LdaNewTarget);
                    self.emit(Op::StaSlot, &[0, i64::from(k)]);
                }
                (BindingKind::Arguments, loc) => {
                    self.emit(Op::CreateArguments, &[i64::from(mapped)]);
                    self.store_loc(loc);
                }
                (kind, Loc::Register(r)) if kind.lexical() => {
                    self.op(Op::LdaUninitialized);
                    self.reg_op(Op::Star, r);
                }
                _ => {}
            }
        }
        if let Some(rest) = f.rest {
            let node = self.ast.pat(rest);
            let Pat::Ident(name) = &node.kind else {
                return Err(unsupported("destructuring", node.span));
            };
            self.emit(Op::CreateRestParameter, &[i64::from(param_count)]);
            self.store_name(&name.name, node.span, true)?;
        }
        for p in &f.params {
            let node = self.ast.pat(p.target);
            let Pat::Ident(name) = &node.kind else {
                return Err(unsupported("destructuring", node.span));
            };
            if let Some(d) = p.default {
                let skip = self.new_label();
                self.load_name(&name.name, node.span, false)?;
                self.jump(Op::JumpIfNotUndefined, skip);
                let inferred = units(&name.name);
                self.expr_named(d, Some(&inferred))?;
                self.store_name(&name.name, node.span, true)?;
                self.bind(skip);
            }
        }
        for b in &scope.bindings {
            if let Some(g) = b.func {
                self.closure(g, None)?;
                self.store_loc(b.loc);
            }
        }
        match &f.body {
            Body::Block(stmts) => {
                self.stmts(stmts)?;
                self.op(Op::LdaUndefined);
            }
            Body::Expr(e) => {
                self.pos(self.ast.expr(*e).span);
                self.expr(*e)?;
            }
        }
        self.op(Op::Return);
        Ok(())
    }

    /// Makes a closure of `fid` into the accumulator; `inferred` names
    /// an anonymous one.
    pub(super) fn closure(&mut self, fid: FuncId, inferred: Option<&[u16]>) -> R {
        let f = self.ast.func(fid);
        let index = self.a.func_index.get(&fid).copied().unwrap_or(0);
        if f.is_generator {
            return Err(unsupported("generators", f.span));
        }
        if f.is_async {
            return Err(unsupported("async functions", f.span));
        }
        let kind = match f.kind {
            FuncKind::Normal => FunctionKind::Normal,
            FuncKind::Arrow => FunctionKind::Arrow,
            FuncKind::Method | FuncKind::Getter | FuncKind::Setter => FunctionKind::Method,
            FuncKind::Constructor { .. } | FuncKind::ClassField | FuncKind::StaticBlock => {
                return Err(unsupported("classes", f.span));
            }
        };
        let name = match (&f.name, inferred) {
            (Some(n), _) => units(&n.name),
            (None, Some(n)) => n.into(),
            (None, None) => Box::default(),
        };
        self.name_function(index, name, kind);
        self.emit(Op::CreateClosure, &[i64::from(index)]);
        Ok(())
    }

    // Loads and stores of names.

    pub(super) fn load_name(&mut self, name: &str, span: Span, typeof_: bool) -> R {
        let r = self.resolve(name);
        match r.place {
            Place::Register(reg) => {
                self.reg_op(Op::Ldar, reg);
                self.tdz_acc(&r, name, span);
            }
            Place::Slot { depth, slot } => {
                self.emit(Op::LdaSlot, &[i64::from(depth), i64::from(slot)]);
                self.tdz_acc(&r, name, span);
            }
            Place::Global => {
                let n = self.name(name);
                let ic = self.ic();
                self.pos(span);
                let op = if typeof_ {
                    Op::LdaGlobalTypeof
                } else {
                    Op::LdaGlobal
                };
                self.emit(op, &[n, ic]);
            }
            Place::Dynamic => {
                let n = self.name(name);
                self.pos(span);
                let op = if typeof_ {
                    Op::LdaDynamicTypeof
                } else {
                    Op::LdaDynamic
                };
                self.emit(op, &[n]);
            }
            Place::Intrinsic => {
                self.op(if name == "this" {
                    Op::LdaThis
                } else {
                    Op::LdaNewTarget
                });
            }
            Place::GlobalThis => self.op(Op::LdaGlobalThis),
        }
        Ok(())
    }

    fn tdz_acc(&mut self, r: &Resolved, name: &str, span: Span) {
        if Self::needs_tdz(r, span.start) {
            let n = self.name(name);
            self.pos(span);
            self.emit(Op::CheckTdz, &[n]);
        }
    }

    /// Stores the accumulator into a name; `init` for a declaration's
    /// initialization (no TDZ check, and a constant may be set).
    pub(super) fn store_name(&mut self, name: &str, span: Span, init: bool) -> R {
        let r = self.resolve(name);
        let check = !init && Self::needs_tdz(&r, span.start);
        let n = self.name(name);
        if check {
            self.pos(span);
            match r.place {
                Place::Register(reg) => self.emit(Op::CheckTdzRegister, &[i64::from(reg), n]),
                Place::Slot { depth, slot } => {
                    self.emit(Op::CheckTdzSlot, &[i64::from(depth), i64::from(slot), n]);
                }
                _ => {}
            }
        }
        match r.kind {
            Some(BindingKind::Const) if !init && r.place != Place::Global => {
                self.pos(span);
                self.emit(Op::ThrowConstAssign, &[n]);
                return Ok(());
            }
            Some(BindingKind::FuncSelf) if !init => {
                // Immutable: an error in strict code, ignored otherwise.
                if self.strict {
                    self.pos(span);
                    self.emit(Op::ThrowConstAssign, &[n]);
                }
                return Ok(());
            }
            _ => {}
        }
        match r.place {
            Place::Register(reg) => self.reg_op(Op::Star, reg),
            Place::Slot { depth, slot } => {
                self.emit(Op::StaSlot, &[i64::from(depth), i64::from(slot)]);
            }
            Place::Global => {
                if init && r.kind.is_some_and(BindingKind::lexical) {
                    self.emit(Op::InitGlobalLexical, &[n]);
                } else {
                    let ic = self.ic();
                    self.pos(span);
                    self.emit(Op::StaGlobal, &[n, ic]);
                }
            }
            Place::Dynamic => {
                self.pos(span);
                self.emit(Op::StaDynamic, &[n]);
            }
            Place::Intrinsic | Place::GlobalThis => {}
        }
        Ok(())
    }
}
