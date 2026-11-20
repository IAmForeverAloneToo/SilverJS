//! The interpreter (docs/design.md section 1.2): register bytecode run
//! on the agent's own stack.
//!
//! - A call pushes a [`Frame`] and its registers onto the agent's
//!   register file and goes on in the same loop: script calling script
//!   never recurses in Rust. Depth is a count the engine chooses
//!   ([`MAX_FRAMES`], then a `RangeError`), not the thread's stack.
//! - Native code calling script (a getter, `valueOf`, a native's
//!   callback) runs a nested loop, bounded by a re-entry count
//!   ([`MAX_REENTRY`]).
//! - An exception unwinds frame by frame through each function's handler
//!   table, the environment restored from the register the handler
//!   names; termination unwinds past every handler.
//! - The step budget is charged at backward jumps and calls; every
//!   [`STEPS_PER_CHECK`] steps the embedder's interrupt hook is asked, and
//!   its `true` terminates the run.
//!
//! Everything a frame holds is in the agent's roots: the registers, each
//! frame's closure, receiver, new target and environments, the
//! accumulator, the exception being thrown, the realm, and the installed
//! units' strings.

mod messages;
mod ops;
mod realm;

use std::rc::Rc;

pub(crate) use messages::ErrorType;
pub(crate) use realm::Realm;

use crate::agent::Agent;
use crate::compiler::{CompiledUnit, Constant, FunctionKind, GlobalDecl, Op};
use crate::heap::{InternalCell, ObjectCell};
use crate::internal::Env;
use crate::object::{Callable, Descriptor, ObjectKind};
use crate::string::PropertyKey;
use crate::value::{RefKind, Value};

/// The most frames the register file holds: past it, a `RangeError`
/// ("Maximum call stack size exceeded"), as the reference browser
/// throws at about this depth.
pub(crate) const MAX_FRAMES: usize = 10_000;
/// The most nested runs native code may start (a getter calling a
/// getter...), each of which uses the Rust stack.
pub(crate) const MAX_REENTRY: u32 = 256;
/// The steps between two calls of the interrupt hook.
pub(crate) const STEPS_PER_CHECK: u32 = 10_000;

/// How a run ended other than normally. A thrown value is in
/// `vm.exception`, rooted while it unwinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Abrupt {
    Throw,
    /// The interrupt hook stopped the run: no `catch` or `finally` runs.
    Terminate,
}

pub(crate) type Res<T> = Result<T, Abrupt>;

/// A compiled unit installed in the agent: its string constants made
/// strings of the heap, and the property keys they name, once.
pub(crate) struct Installed {
    pub code: Rc<CompiledUnit>,
    pub strings: Vec<Vec<Value>>,
    pub keys: Vec<Vec<Option<PropertyKey>>>,
}

/// A function's activation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frame {
    pub unit: u32,
    pub func: u32,
    /// The instruction under way while the frame waits on a call (where
    /// a handler is looked for).
    pub pc: u32,
    /// Where it goes on after that call returns.
    pub resume: u32,
    /// The register file's index of its first argument; its registers
    /// begin at `base`.
    pub args: u32,
    pub base: u32,
    pub closure: Value,
    pub this: Value,
    pub new_target: Value,
    /// The environment its code runs in now, and its function's own (a
    /// handler with no register names that).
    pub context: Value,
    pub fn_context: Value,
    /// Made by `new`: a non-object return value gives `this`.
    pub construct: bool,
    /// Pushed by Rust (a native's call): its return ends a nested run.
    pub entry: bool,
}

/// The interpreter's state, which the agent owns.
pub(crate) struct Vm {
    pub units: Vec<Installed>,
    pub stack: Vec<Value>,
    pub frames: Vec<Frame>,
    pub acc: Value,
    pub exception: Value,
    pub realm: Option<Realm>,
    pub steps: u32,
    pub interrupt: Option<Box<dyn FnMut() -> bool>>,
    pub reentry: u32,
    /// Values Rust holds across an allocation (a new object being
    /// filled), rooted while it does.
    pub temps: Vec<Value>,
}

impl Default for Vm {
    fn default() -> Vm {
        Vm {
            units: Vec::new(),
            stack: Vec::new(),
            frames: Vec::new(),
            acc: Value::UNDEFINED,
            exception: Value::UNDEFINED,
            realm: None,
            steps: STEPS_PER_CHECK,
            interrupt: None,
            reentry: 0,
            temps: Vec::new(),
        }
    }
}

impl Vm {
    /// Every value the interpreter holds.
    pub(crate) fn roots(&self) -> Vec<Value> {
        let mut out = Vec::with_capacity(self.stack.len() + self.frames.len() * 5 + 64);
        out.extend_from_slice(&self.stack);
        for f in &self.frames {
            out.extend([f.closure, f.this, f.new_target, f.context, f.fn_context]);
        }
        out.extend([self.acc, self.exception]);
        out.extend_from_slice(&self.temps);
        if let Some(realm) = &self.realm {
            realm.trace(&mut out);
        }
        for u in &self.units {
            for s in &u.strings {
                out.extend_from_slice(s);
            }
        }
        out
    }
}

/// How a script ended, for the embedder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Completion {
    Normal(Value),
    Threw(Value),
    Terminated,
}

/// The operands of the instruction under way, read in order.
struct Reader<'c> {
    code: &'c [u8],
    pc: usize,
    scale: usize,
}

impl Reader<'_> {
    #[inline]
    fn byte(&self, at: usize) -> u32 {
        u32::from(self.code.get(at).copied().unwrap_or(0))
    }

    #[inline]
    fn u(&mut self) -> usize {
        let at = self.pc;
        let v = match self.scale {
            1 => self.byte(at),
            2 => self.byte(at) | self.byte(at + 1) << 8,
            _ => {
                self.byte(at)
                    | self.byte(at + 1) << 8
                    | self.byte(at + 2) << 16
                    | self.byte(at + 3) << 24
            }
        };
        self.pc += self.scale;
        v as usize
    }

    #[inline]
    fn i(&mut self) -> i32 {
        let at = self.pc;
        let v = match self.scale {
            1 => i32::from(self.byte(at) as u8 as i8),
            2 => i32::from((self.byte(at) | self.byte(at + 1) << 8) as u16 as i16),
            _ => {
                (self.byte(at)
                    | self.byte(at + 1) << 8
                    | self.byte(at + 2) << 16
                    | self.byte(at + 3) << 24) as i32
            }
        };
        self.pc += self.scale;
        v
    }

    #[inline]
    fn j(&mut self) -> i32 {
        let at = self.pc;
        let v = (self.byte(at)
            | self.byte(at + 1) << 8
            | self.byte(at + 2) << 16
            | self.byte(at + 3) << 24) as i32;
        self.pc += 4;
        v
    }
}

fn target(start: usize, offset: i32) -> usize {
    usize::try_from(start as i64 + i64::from(offset)).unwrap_or(usize::MAX)
}

impl Agent {
    /// Sets the hook asked every [`STEPS_PER_CHECK`] steps of a run: its
    /// `true` terminates the run (no `catch` or `finally` runs).
    pub fn set_interrupt(&mut self, hook: Option<Box<dyn FnMut() -> bool>>) {
        self.vm.interrupt = hook;
    }

    pub(crate) fn realm(&self) -> &Realm {
        // Made before any code runs (`ensure_realm`).
        match &self.vm.realm {
            Some(r) => r,
            None => unreachable_realm(),
        }
    }

    /// The installed unit `u`'s code.
    fn unit_code(&self, u: u32) -> Option<Rc<CompiledUnit>> {
        self.vm.units.get(u as usize).map(|i| Rc::clone(&i.code))
    }

    /// Installs a compiled unit: its strings made, its names interned.
    pub(crate) fn install(&mut self, unit: CompiledUnit) -> Res<u32> {
        let index = u32::try_from(self.vm.units.len()).unwrap_or(u32::MAX);
        let code = Rc::new(unit);
        self.vm.units.push(Installed {
            code: Rc::clone(&code),
            strings: Vec::new(),
            keys: Vec::new(),
        });
        for f in &code.functions {
            let mut strings = Vec::with_capacity(f.constants.len());
            let mut keys = Vec::with_capacity(f.constants.len());
            // Rooted as it fills: an allocation may collect.
            if let Some(u) = self.vm.units.last_mut() {
                u.strings.push(Vec::new());
            }
            for c in &f.constants {
                match c {
                    Constant::String(units) => {
                        let s = self.new_string_units(units)?;
                        strings.push(s);
                        if let Some(u) = self.vm.units.last_mut()
                            && let Some(last) = u.strings.last_mut()
                        {
                            last.push(s);
                        }
                        keys.push(Some(self.heap.atoms.key(units)));
                    }
                    _ => {
                        strings.push(Value::UNDEFINED);
                        if let Some(u) = self.vm.units.last_mut()
                            && let Some(last) = u.strings.last_mut()
                        {
                            last.push(Value::UNDEFINED);
                        }
                        keys.push(None);
                    }
                }
            }
            if let Some(u) = self.vm.units.last_mut() {
                u.keys.push(keys);
            }
        }
        Ok(index)
    }

    /// Runs a compiled script in the agent's realm: its global
    /// declarations, then its code.
    pub(crate) fn run_compiled(&mut self, unit: CompiledUnit) -> Completion {
        let result = self.ensure_realm().and_then(|()| {
            let index = self.install(unit)?;
            self.global_declarations(index)?;
            self.push_script_frame(index)?;
            self.run_frames()
        });
        match result {
            Ok(v) => Completion::Normal(v),
            Err(Abrupt::Throw) => Completion::Threw(self.vm.exception),
            Err(Abrupt::Terminate) => Completion::Terminated,
        }
    }

    fn push_script_frame(&mut self, unit: u32) -> Res<()> {
        let code = self.unit_code(unit).ok_or(Abrupt::Throw)?;
        let regs = code.functions.first().map_or(0, |f| f.register_count);
        let global = self.realm().global;
        let args = u32::try_from(self.vm.stack.len()).unwrap_or(u32::MAX);
        self.vm
            .stack
            .resize(self.vm.stack.len() + regs as usize, Value::UNDEFINED);
        self.vm.frames.push(Frame {
            unit,
            func: 0,
            pc: 0,
            resume: 0,
            args,
            base: args,
            closure: Value::UNDEFINED,
            this: global,
            new_target: Value::UNDEFINED,
            context: Value::UNDEFINED,
            fn_context: Value::UNDEFINED,
            construct: false,
            entry: true,
        });
        Ok(())
    }

    /// ECMA-262 GlobalDeclarationInstantiation (section 16.1.7): the
    /// script's lexical names checked against what is there, then its
    /// functions made and its `var`s and lexical bindings created.
    fn global_declarations(&mut self, unit: u32) -> Res<()> {
        let code = self.unit_code(unit).ok_or(Abrupt::Throw)?;
        let global = self.realm().global;
        let g = global.as_object().unwrap_or(0);
        for decl in &code.globals {
            if let GlobalDecl::Let(name) | GlobalDecl::Const(name) = decl {
                let key = self.heap.atoms.key(name);
                let clash = self.realm().lexical.contains_key(&key)
                    || self
                        .heap
                        .get_own_property(g, key)
                        .is_some_and(|p| !ops::configurable(&p));
                if clash {
                    let text = String::from_utf16_lossy(name);
                    return Err(
                        self.throw_error(ErrorType::SyntaxError, messages::already_declared(&text))
                    );
                }
            }
            if let GlobalDecl::Var(name) | GlobalDecl::Function(name, _) = decl {
                let key = self.heap.atoms.key(name);
                if self.realm().lexical.contains_key(&key) {
                    let text = String::from_utf16_lossy(name);
                    return Err(
                        self.throw_error(ErrorType::SyntaxError, messages::already_declared(&text))
                    );
                }
            }
        }
        for decl in &code.globals {
            match decl {
                GlobalDecl::Var(name) => {
                    let key = self.heap.atoms.key(name);
                    if self.heap.get_own_property(g, key).is_none() {
                        self.heap.define_own_property(
                            g,
                            key,
                            Descriptor {
                                value: Some(Value::UNDEFINED),
                                writable: Some(true),
                                enumerable: Some(true),
                                configurable: Some(false),
                                ..Descriptor::default()
                            },
                        );
                    }
                }
                GlobalDecl::Function(name, func) => {
                    let key = self.heap.atoms.key(name);
                    let f = self.make_closure(unit, *func, Value::UNDEFINED)?;
                    let existing = self.heap.get_own_property(g, key);
                    let desc = if existing.is_none_or(|p| ops::configurable(&p)) {
                        Descriptor {
                            value: Some(f),
                            writable: Some(true),
                            enumerable: Some(true),
                            configurable: Some(false),
                            ..Descriptor::default()
                        }
                    } else {
                        Descriptor {
                            value: Some(f),
                            ..Descriptor::default()
                        }
                    };
                    self.heap.define_own_property(g, key, desc);
                }
                GlobalDecl::Let(name) | GlobalDecl::Const(name) => {
                    let key = self.heap.atoms.key(name);
                    let constant = matches!(decl, GlobalDecl::Const(_));
                    if let Some(realm) = self.vm.realm.as_mut() {
                        realm.lexical.insert(
                            key,
                            realm::Lexical {
                                value: Value::UNINITIALIZED,
                                constant,
                            },
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Calls `callee` with `this` and `args` from Rust (a native, an
    /// accessor, a conversion): a nested run, bounded by the re-entry
    /// count.
    pub(crate) fn call_value(&mut self, callee: Value, this: Value, args: &[Value]) -> Res<Value> {
        let Some(callable) = self.callable(callee) else {
            let text = self.display(callee);
            return Err(self.throw_error(ErrorType::TypeError, messages::not_a_function(&text)));
        };
        if self.vm.reentry >= MAX_REENTRY {
            return Err(self.throw_error(ErrorType::RangeError, messages::STACK_OVERFLOW));
        }
        // The arguments on the register file, where the callee reads
        // them and the collector sees them.
        let start = self.vm.stack.len();
        self.vm.stack.extend_from_slice(args);
        let result = match callable {
            Callable::Native { id } => {
                self.vm.reentry += 1;
                let r = realm::call_native(self, id, this, start, args.len());
                self.vm.reentry -= 1;
                r
            }
            Callable::Code { unit, func, env } => {
                match self.push_frame(
                    callee,
                    unit,
                    func,
                    env,
                    this,
                    start,
                    args.len(),
                    Value::UNDEFINED,
                    false,
                    true,
                ) {
                    Ok(()) => {
                        self.vm.reentry += 1;
                        let r = self.run_frames();
                        self.vm.reentry -= 1;
                        r
                    }
                    Err(e) => Err(e),
                }
            }
        };
        self.vm.stack.truncate(start);
        result
    }

    /// What calling `v` runs, if it is a function.
    pub(crate) fn callable(&self, v: Value) -> Option<Callable> {
        self.heap.objects.get(v.as_object()?)?.callable()
    }

    /// Pushes a frame for function `func` of unit `unit`, its arguments
    /// the `argc` values from `args` on the register file.
    #[allow(clippy::too_many_arguments)]
    fn push_frame(
        &mut self,
        closure: Value,
        unit: u32,
        func: u32,
        env: Value,
        this: Value,
        args: usize,
        argc: usize,
        new_target: Value,
        construct: bool,
        entry: bool,
    ) -> Res<()> {
        if self.vm.frames.len() >= MAX_FRAMES {
            return Err(self.throw_error(ErrorType::RangeError, messages::STACK_OVERFLOW));
        }
        let Some(code) = self.vm.units.get(unit as usize).map(|u| Rc::clone(&u.code)) else {
            return Err(Abrupt::Throw);
        };
        let Some(f) = code.functions.get(func as usize) else {
            return Err(Abrupt::Throw);
        };
        let base = self.vm.stack.len();
        self.vm
            .stack
            .resize(base + f.register_count as usize, Value::UNDEFINED);
        let params = (f.param_count as usize).min(argc);
        for i in 0..params {
            let v = self
                .vm
                .stack
                .get(args + i)
                .copied()
                .unwrap_or(Value::UNDEFINED);
            if let Some(slot) = self.vm.stack.get_mut(base + i) {
                *slot = v;
            }
        }
        let this = match f.kind {
            FunctionKind::Arrow => Value::UNDEFINED,
            _ if !f.strict && this.is_nullish() => self.realm().global,
            _ => this,
        };
        let args = u32::try_from(args).unwrap_or(u32::MAX);
        self.vm.frames.push(Frame {
            unit,
            func,
            pc: 0,
            resume: 0,
            args,
            base: u32::try_from(base).unwrap_or(u32::MAX),
            closure,
            this,
            new_target,
            context: env,
            fn_context: env,
            construct,
            entry,
        });
        self.tick()
    }

    /// Charges a step; asks the interrupt hook every so many.
    #[inline]
    fn tick(&mut self) -> Res<()> {
        self.vm.steps -= 1;
        if self.vm.steps == 0 {
            self.vm.steps = STEPS_PER_CHECK;
            if let Some(hook) = self.vm.interrupt.as_mut()
                && hook()
            {
                return Err(Abrupt::Terminate);
            }
        }
        Ok(())
    }

    /// Unwinds from the top frame, which threw at its `pc`: to the first
    /// handler found, or, past the run's entry frame, back to Rust.
    fn unwind(&mut self, abrupt: Abrupt) -> Res<()> {
        loop {
            let Some(frame) = self.vm.frames.last().copied() else {
                return Err(abrupt);
            };
            if abrupt == Abrupt::Throw
                && let Some(code) = self.unit_code(frame.unit)
                && let Some(f) = code.functions.get(frame.func as usize)
                && let Some(h) = f
                    .handlers
                    .iter()
                    .find(|h| h.start <= frame.pc && frame.pc < h.end)
            {
                let context = match h.context {
                    Some(r) => self.reg(frame.base, r as usize),
                    None => frame.fn_context,
                };
                if let Some(top) = self.vm.frames.last_mut() {
                    top.context = context;
                    top.resume = h.target;
                    top.pc = h.target;
                }
                self.vm.acc = self.vm.exception;
                return Ok(());
            }
            self.vm.frames.pop();
            self.vm.stack.truncate(frame.args as usize);
            if frame.entry {
                return Err(abrupt);
            }
        }
    }

    #[inline]
    fn reg(&self, base: u32, r: usize) -> Value {
        self.vm
            .stack
            .get(base as usize + r)
            .copied()
            .unwrap_or(Value::UNDEFINED)
    }

    #[inline]
    fn set_reg(&mut self, base: u32, r: usize, v: Value) {
        if let Some(slot) = self.vm.stack.get_mut(base as usize + r) {
            *slot = v;
        }
    }

    /// The source text the instruction at `pc` evaluates (a callee, for
    /// "o.f is not a function").
    fn text_at(&self, unit: u32, func: u32, pc: u32) -> String {
        let Some(code) = self.vm.units.get(unit as usize).map(|u| &u.code) else {
            return String::new();
        };
        let Some(f) = code.functions.get(func as usize) else {
            return String::new();
        };
        let at = f.positions.iter().rev().find(|p| p.pc <= pc);
        at.and_then(|p| code.source.get(p.start as usize..p.end as usize))
            .unwrap_or("")
            .to_owned()
    }

    /// The constant `k` of a function, as a value.
    fn constant(&mut self, unit: u32, func: u32, k: usize) -> Res<Value> {
        let u = self.vm.units.get(unit as usize);
        if let Some(s) = u
            .and_then(|u| u.strings.get(func as usize))
            .and_then(|s| s.get(k))
            .copied()
            .filter(|v| !v.is_undefined())
        {
            return Ok(s);
        }
        let c = u
            .and_then(|u| u.code.functions.get(func as usize))
            .and_then(|f| f.constants.get(k))
            .cloned();
        match c {
            Some(Constant::Number(n)) => Ok(Value::number(n)),
            Some(Constant::BigInt { .. }) => {
                Err(self.throw_error(ErrorType::SyntaxError, messages::not_supported("BigInt")))
            }
            _ => Ok(Value::UNDEFINED),
        }
    }

    /// The property key the constant `k` names.
    #[inline]
    fn key_constant(&self, unit: u32, func: u32, k: usize) -> PropertyKey {
        self.vm
            .units
            .get(unit as usize)
            .and_then(|u| u.keys.get(func as usize))
            .and_then(|k2| k2.get(k))
            .copied()
            .flatten()
            .unwrap_or(PropertyKey::Index(0))
    }

    /// The environment `depth` hops out from `from`.
    fn env_at(&self, from: Value, depth: usize) -> Option<u32> {
        let mut at = from;
        for _ in 0..depth {
            at = match self.heap.internals.get(at.as_reference()?.1)? {
                InternalCell::Env(e) => e.parent,
                InternalCell::ForIn(_) => return None,
            };
        }
        match at.as_reference()? {
            (RefKind::Internal, i) => Some(i),
            _ => None,
        }
    }

    fn slot(&self, env: Option<u32>, slot: usize) -> Value {
        match env.and_then(|e| self.heap.internals.get(e)) {
            Some(InternalCell::Env(e)) => e.slots.get(slot).copied().unwrap_or(Value::UNDEFINED),
            _ => Value::UNDEFINED,
        }
    }

    fn set_slot(&mut self, env: Option<u32>, slot: usize, v: Value) {
        if let Some(InternalCell::Env(e)) = env.and_then(|e| self.heap.internals.get_mut(e))
            && let Some(s) = e.slots.get_mut(slot)
        {
            *s = v;
        }
    }

    /// A new environment of scope `scope` of unit `unit` inside `parent`.
    fn new_env(&mut self, unit: u32, scope: usize, parent: Value) -> Res<Value> {
        let lexical: Vec<bool> = self
            .vm
            .units
            .get(unit as usize)
            .and_then(|u| u.code.scopes.get(scope))
            .map(|s| s.lexical.clone())
            .unwrap_or_default();
        let slots = lexical
            .iter()
            .map(|&l| {
                if l {
                    Value::UNINITIALIZED
                } else {
                    Value::UNDEFINED
                }
            })
            .collect();
        self.alloc_internal(InternalCell::Env(Env {
            parent,
            slots,
            scope: Some((unit, u32::try_from(scope).unwrap_or(u32::MAX))),
            object: Value::UNDEFINED,
        }))
    }

    /// Runs the top frame, and the frames it calls, until the run's entry
    /// frame returns: its value, or how it ended.
    pub(crate) fn run_frames(&mut self) -> Res<Value> {
        'frames: loop {
            // Defined inside the loop, whose label they name.
            macro_rules! fail {
                ($e:expr) => {{
                    let e = $e;
                    match self.unwind(e) {
                        Ok(()) => continue 'frames,
                        Err(e) => return Err(e),
                    }
                }};
            }
            let Some(frame) = self.vm.frames.last().copied() else {
                return Ok(self.vm.acc);
            };
            let Some(unit) = self.unit_code(frame.unit) else {
                return Err(Abrupt::Throw);
            };
            let Some(func) = unit.functions.get(frame.func as usize) else {
                return Err(Abrupt::Throw);
            };
            let strict = func.strict;
            let base = frame.base;
            let (u, fi) = (frame.unit, frame.func);
            let mut r = Reader {
                code: &func.code,
                pc: frame.resume as usize,
                scale: 1,
            };
            macro_rules! tri {
                ($e:expr, $start:expr) => {
                    match $e {
                        Ok(v) => v,
                        Err(e) => {
                            if let Some(top) = self.vm.frames.last_mut() {
                                top.pc = $start as u32;
                            }
                            fail!(e)
                        }
                    }
                };
            }
            loop {
                let start = r.pc;
                let mut byte = r.byte(r.pc);
                r.scale = 1;
                if byte == Op::Wide as u32 {
                    r.scale = 2;
                    r.pc += 1;
                    byte = r.byte(r.pc);
                } else if byte == Op::ExtraWide as u32 {
                    r.scale = 4;
                    r.pc += 1;
                    byte = r.byte(r.pc);
                }
                r.pc += 1;
                let Some(op) = Op::from_byte(byte as u8) else {
                    return Err(Abrupt::Throw);
                };
                match op {
                    Op::Wide | Op::ExtraWide => return Err(Abrupt::Throw),
                    Op::LdaUndefined => self.vm.acc = Value::UNDEFINED,
                    Op::LdaNull => self.vm.acc = Value::NULL,
                    Op::LdaTrue => self.vm.acc = Value::TRUE,
                    Op::LdaFalse => self.vm.acc = Value::FALSE,
                    Op::LdaUninitialized => self.vm.acc = Value::UNINITIALIZED,
                    Op::LdaSmi => self.vm.acc = Value::int(r.i()),
                    Op::LdaConst => {
                        let k = r.u();
                        self.vm.acc = tri!(self.constant(u, fi, k), start);
                    }
                    Op::Ldar => {
                        let x = r.u();
                        self.vm.acc = self.reg(base, x);
                    }
                    Op::Star => {
                        let x = r.u();
                        let v = self.vm.acc;
                        self.set_reg(base, x, v);
                    }
                    Op::Mov => {
                        let (a, b) = (r.u(), r.u());
                        let v = self.reg(base, a);
                        self.set_reg(base, b, v);
                    }
                    Op::LdaThis => self.vm.acc = self.top().this,
                    Op::LdaNewTarget => self.vm.acc = self.top().new_target,
                    Op::LdaClosure => self.vm.acc = self.top().closure,
                    Op::LdaGlobalThis => self.vm.acc = self.realm().global,
                    Op::CheckTdz => {
                        let k = r.u();
                        if self.vm.acc == Value::UNINITIALIZED {
                            let name = self.name_text(u, fi, k);
                            tri!(
                                Err::<(), Abrupt>(self.throw_error(
                                    ErrorType::ReferenceError,
                                    messages::before_initialization(&name)
                                )),
                                start
                            );
                        }
                    }
                    Op::CheckTdzRegister => {
                        let (x, k) = (r.u(), r.u());
                        if self.reg(base, x) == Value::UNINITIALIZED {
                            let name = self.name_text(u, fi, k);
                            tri!(
                                Err::<(), Abrupt>(self.throw_error(
                                    ErrorType::ReferenceError,
                                    messages::before_initialization(&name)
                                )),
                                start
                            );
                        }
                    }
                    Op::CheckTdzSlot => {
                        let (d, s, k) = (r.u(), r.u(), r.u());
                        let env = self.env_at(self.top().context, d);
                        if self.slot(env, s) == Value::UNINITIALIZED {
                            let name = self.name_text(u, fi, k);
                            tri!(
                                Err::<(), Abrupt>(self.throw_error(
                                    ErrorType::ReferenceError,
                                    messages::before_initialization(&name)
                                )),
                                start
                            );
                        }
                    }
                    Op::ThrowConstAssign => {
                        let _ = r.u();
                        tri!(
                            Err::<(), Abrupt>(
                                self.throw_error(ErrorType::TypeError, messages::CONST_ASSIGNMENT)
                            ),
                            start
                        );
                    }
                    Op::ThrowInvalidLhs => {
                        tri!(
                            Err::<(), Abrupt>(self.throw_error(
                                ErrorType::ReferenceError,
                                messages::INVALID_ASSIGNMENT
                            )),
                            start
                        );
                    }
                    Op::LdaSlot => {
                        let (d, s) = (r.u(), r.u());
                        let env = self.env_at(self.top().context, d);
                        self.vm.acc = self.slot(env, s);
                    }
                    Op::StaSlot => {
                        let (d, s) = (r.u(), r.u());
                        let env = self.env_at(self.top().context, d);
                        let v = self.vm.acc;
                        self.set_slot(env, s, v);
                    }
                    Op::LdaGlobal | Op::LdaGlobalTypeof => {
                        let (k, _ic) = (r.u(), r.u());
                        let key = self.key_constant(u, fi, k);
                        self.vm.acc = tri!(self.global_get(key, op == Op::LdaGlobalTypeof), start);
                    }
                    Op::StaGlobal => {
                        let (k, _ic) = (r.u(), r.u());
                        let key = self.key_constant(u, fi, k);
                        let v = self.vm.acc;
                        tri!(self.global_set(key, v, strict), start);
                    }
                    Op::InitGlobalLexical => {
                        let k = r.u();
                        let key = self.key_constant(u, fi, k);
                        let v = self.vm.acc;
                        if let Some(realm) = self.vm.realm.as_mut()
                            && let Some(b) = realm.lexical.get_mut(&key)
                        {
                            b.value = v;
                        }
                    }
                    Op::LdaDynamic | Op::LdaDynamicTypeof => {
                        let k = r.u();
                        let key = self.key_constant(u, fi, k);
                        let context = self.top().context;
                        self.vm.acc = tri!(
                            self.dynamic_get(context, key, op == Op::LdaDynamicTypeof),
                            start
                        );
                    }
                    Op::StaDynamic => {
                        let k = r.u();
                        let key = self.key_constant(u, fi, k);
                        let context = self.top().context;
                        let v = self.vm.acc;
                        tri!(self.dynamic_set(context, key, v, strict), start);
                    }
                    Op::DeleteName => {
                        let k = r.u();
                        let key = self.key_constant(u, fi, k);
                        let context = self.top().context;
                        self.vm.acc = Value::bool(self.dynamic_delete(context, key));
                    }
                    Op::PushContext => {
                        let (x, scope) = (r.u(), r.u());
                        let outer = self.top().context;
                        self.set_reg(base, x, outer);
                        let env = tri!(self.new_env(u, scope, outer), start);
                        if let Some(top) = self.vm.frames.last_mut() {
                            top.context = env;
                        }
                    }
                    Op::PopContext => {
                        let x = r.u();
                        let env = self.reg(base, x);
                        if let Some(top) = self.vm.frames.last_mut() {
                            top.context = env;
                        }
                    }
                    Op::StoreContext => {
                        let x = r.u();
                        let env = self.top().context;
                        self.set_reg(base, x, env);
                    }
                    Op::CreateFunctionContext => {
                        let scope = r.u();
                        let outer = self.top().context;
                        let env = tri!(self.new_env(u, scope, outer), start);
                        if let Some(top) = self.vm.frames.last_mut() {
                            top.context = env;
                            top.fn_context = env;
                        }
                    }
                    Op::CopyContext => {
                        let current = self.top().context;
                        let copy = match current
                            .as_reference()
                            .and_then(|(_, i)| self.heap.internals.get(i))
                        {
                            Some(InternalCell::Env(e)) => Some(e.clone()),
                            _ => None,
                        };
                        if let Some(e) = copy {
                            let env = tri!(self.alloc_internal(InternalCell::Env(e)), start);
                            if let Some(top) = self.vm.frames.last_mut() {
                                top.context = env;
                            }
                        }
                    }
                    Op::PushWithContext => {
                        let x = r.u();
                        let object = self.vm.acc;
                        if object.is_nullish() {
                            tri!(
                                Err::<(), Abrupt>(self.throw_error(
                                    ErrorType::TypeError,
                                    messages::NULLISH_TO_OBJECT
                                )),
                                start
                            );
                        }
                        let outer = self.top().context;
                        self.set_reg(base, x, outer);
                        let env = tri!(
                            self.alloc_internal(InternalCell::Env(Env {
                                parent: outer,
                                slots: Vec::new(),
                                scope: None,
                                object,
                            })),
                            start
                        );
                        if let Some(top) = self.vm.frames.last_mut() {
                            top.context = env;
                        }
                    }
                    Op::CreateClosure => {
                        let f = r.u();
                        let env = self.top().context;
                        self.vm.acc = tri!(self.make_closure(u, f as u32, env), start);
                    }
                    Op::CreateArguments => {
                        let _mapped = r.u();
                        let frame = *self.top();
                        self.vm.acc = tri!(
                            self.make_arguments(
                                frame.args as usize,
                                (frame.base - frame.args) as usize
                            ),
                            start
                        );
                    }
                    Op::CreateRestParameter => {
                        let from = r.u();
                        let frame = *self.top();
                        let argc = (frame.base - frame.args) as usize;
                        let from = from.min(argc);
                        self.vm.acc = tri!(
                            self.make_array_from_stack(frame.args as usize + from, argc - from),
                            start
                        );
                    }
                    Op::CreateObject => {
                        let proto = self.realm().object_prototype;
                        self.vm.acc = tri!(
                            self.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary)),
                            start
                        );
                    }
                    Op::CreateArray => {
                        let proto = self.realm().array_prototype;
                        self.vm.acc = tri!(
                            self.alloc_object(ObjectCell::new(proto, ObjectKind::Array)),
                            start
                        );
                    }
                    Op::CreateRegExp => {
                        let _ = (r.u(), r.u());
                        tri!(
                            Err::<(), Abrupt>(self.throw_error(
                                ErrorType::SyntaxError,
                                messages::not_supported("RegExp")
                            )),
                            start
                        );
                    }
                    Op::GetNamed => {
                        let (o, k, _ic) = (r.u(), r.u(), r.u());
                        let key = self.key_constant(u, fi, k);
                        let target = self.reg(base, o);
                        self.vm.acc = tri!(self.get_value(target, key), start);
                    }
                    Op::GetKeyed => {
                        let (o, _ic) = (r.u(), r.u());
                        let target = self.reg(base, o);
                        let k = self.vm.acc;
                        let v = tri!(self.get_keyed(target, k), start);
                        self.vm.acc = v;
                    }
                    Op::SetNamed => {
                        let (o, k, _ic) = (r.u(), r.u(), r.u());
                        let key = self.key_constant(u, fi, k);
                        let target = self.reg(base, o);
                        let v = self.vm.acc;
                        tri!(self.set_value(target, key, v, strict), start);
                    }
                    Op::SetKeyed => {
                        let (o, k, _ic) = (r.u(), r.u(), r.u());
                        let target = self.reg(base, o);
                        let kv = self.reg(base, k);
                        let v = self.vm.acc;
                        tri!(self.set_keyed(target, kv, v, strict), start);
                    }
                    Op::DefineField => {
                        let (o, k) = (r.u(), r.u());
                        let key = self.key_constant(u, fi, k);
                        let target = self.reg(base, o);
                        let v = self.vm.acc;
                        tri!(self.define_value(target, key, v), start);
                    }
                    Op::DefineKeyed => {
                        let (o, k) = (r.u(), r.u());
                        let target = self.reg(base, o);
                        let kv = self.reg(base, k);
                        let key = tri!(self.property_key(kv), start);
                        let v = self.vm.acc;
                        tri!(self.define_value(target, key, v), start);
                    }
                    Op::DefineIndex => {
                        let (o, i) = (r.u(), r.u());
                        let target = self.reg(base, o);
                        let v = self.vm.acc;
                        tri!(
                            self.define_value(target, PropertyKey::Index(i as u32), v),
                            start
                        );
                    }
                    Op::SetArrayLength => {
                        let (o, n) = (r.u(), r.u());
                        if let Some(a) = self.reg(base, o).as_object() {
                            let key = self.heap.length_key;
                            self.heap.define_own_property(
                                a,
                                key,
                                Descriptor {
                                    value: Some(Value::number(n as f64)),
                                    ..Descriptor::default()
                                },
                            );
                        }
                    }
                    Op::DefineGetter | Op::DefineSetter => {
                        let (o, k) = (r.u(), r.u());
                        let target = self.reg(base, o);
                        let kv = self.reg(base, k);
                        let key = tri!(self.property_key(kv), start);
                        let f = self.vm.acc;
                        if let Some(t) = target.as_object() {
                            let desc = if op == Op::DefineGetter {
                                Descriptor {
                                    get: Some(f),
                                    enumerable: Some(true),
                                    configurable: Some(true),
                                    ..Descriptor::default()
                                }
                            } else {
                                Descriptor {
                                    set: Some(f),
                                    enumerable: Some(true),
                                    configurable: Some(true),
                                    ..Descriptor::default()
                                }
                            };
                            self.heap.define_own_property(t, key, desc);
                        }
                    }
                    Op::SetProtoLiteral => {
                        let o = r.u();
                        let proto = self.vm.acc;
                        if (proto.as_object().is_some() || proto.is_null())
                            && let Some(t) = self.reg(base, o).as_object()
                        {
                            self.heap.set_prototype_of(t, proto);
                        }
                    }
                    Op::CopyDataProperties => {
                        let o = r.u();
                        let target = self.reg(base, o);
                        let source = self.vm.acc;
                        tri!(self.copy_data_properties(target, source), start);
                    }
                    Op::SetFunctionName => {
                        let (k, prefix) = (r.u(), r.u());
                        let kv = self.reg(base, k);
                        let f = self.vm.acc;
                        tri!(self.set_function_name(f, kv, prefix), start);
                    }
                    Op::ToPropertyKey => {
                        let v = self.vm.acc;
                        self.vm.acc = tri!(self.property_key_value(v), start);
                    }
                    Op::DeleteProperty => {
                        let o = r.u();
                        let target = self.reg(base, o);
                        let k = self.vm.acc;
                        self.vm.acc = tri!(self.delete_property(target, k, strict), start);
                    }
                    Op::Add => {
                        let x = r.u();
                        let a = self.reg(base, x);
                        let b = self.vm.acc;
                        self.vm.acc = match (a.as_int(), b.as_int()) {
                            (Some(i), Some(j)) => match i.checked_add(j) {
                                Some(s) => Value::int(s),
                                None => Value::double(f64::from(i) + f64::from(j)),
                            },
                            _ => tri!(self.add(a, b), start),
                        };
                    }
                    Op::Sub
                    | Op::Mul
                    | Op::Div
                    | Op::Mod
                    | Op::Exp
                    | Op::BitOr
                    | Op::BitXor
                    | Op::BitAnd
                    | Op::Shl
                    | Op::Shr
                    | Op::UShr => {
                        let x = r.u();
                        let a = self.reg(base, x);
                        let b = self.vm.acc;
                        self.vm.acc = match (op, a.as_int(), b.as_int()) {
                            (Op::Sub, Some(i), Some(j)) => i.checked_sub(j).map_or_else(
                                || Value::double(f64::from(i) - f64::from(j)),
                                Value::int,
                            ),
                            (Op::BitOr, Some(i), Some(j)) => Value::int(i | j),
                            (Op::BitAnd, Some(i), Some(j)) => Value::int(i & j),
                            (Op::BitXor, Some(i), Some(j)) => Value::int(i ^ j),
                            _ => tri!(self.numeric_binary(op, a, b), start),
                        };
                    }
                    Op::Inc | Op::Dec => {
                        let v = self.vm.acc;
                        let delta = if op == Op::Inc { 1 } else { -1 };
                        self.vm.acc = match v.as_int().and_then(|i| i.checked_add(delta)) {
                            Some(n) => Value::int(n),
                            None => {
                                let n = tri!(self.number(v), start);
                                Value::number(n + f64::from(delta))
                            }
                        };
                    }
                    Op::Negate => {
                        let v = self.vm.acc;
                        let n = tri!(self.number(v), start);
                        self.vm.acc = Value::number(-n);
                    }
                    Op::BitNot => {
                        let v = self.vm.acc;
                        let n = tri!(self.number(v), start);
                        self.vm.acc = Value::int(!ops::to_int32(n));
                    }
                    Op::ToNumber | Op::ToNumeric => {
                        let v = self.vm.acc;
                        if !v.is_number() {
                            let n = tri!(self.number(v), start);
                            self.vm.acc = Value::number(n);
                        }
                    }
                    Op::Not => {
                        let v = self.vm.acc;
                        self.vm.acc = Value::bool(!self.to_boolean(v));
                    }
                    Op::TypeOf => {
                        let v = self.vm.acc;
                        self.vm.acc = self.type_of(v);
                    }
                    Op::ToString => {
                        let v = self.vm.acc;
                        self.vm.acc = tri!(self.string_value(v), start);
                    }
                    Op::TestEq => {
                        let x = r.u();
                        let a = self.reg(base, x);
                        let b = self.vm.acc;
                        self.vm.acc = Value::bool(tri!(self.loose_equals(a, b), start));
                    }
                    Op::TestStrictEq => {
                        let x = r.u();
                        let a = self.reg(base, x);
                        let b = self.vm.acc;
                        self.vm.acc = Value::bool(self.strict_equals(a, b));
                    }
                    Op::TestLt | Op::TestGt | Op::TestLtEq | Op::TestGtEq => {
                        let x = r.u();
                        let a = self.reg(base, x);
                        let b = self.vm.acc;
                        let result = match (a.as_int(), b.as_int()) {
                            (Some(i), Some(j)) => match op {
                                Op::TestLt => i < j,
                                Op::TestGt => i > j,
                                Op::TestLtEq => i <= j,
                                _ => i >= j,
                            },
                            _ => tri!(self.relational(op, a, b), start),
                        };
                        self.vm.acc = Value::bool(result);
                    }
                    Op::TestIn => {
                        let x = r.u();
                        let key = self.reg(base, x);
                        let object = self.vm.acc;
                        self.vm.acc = Value::bool(tri!(self.has_in(key, object), start));
                    }
                    Op::TestInstanceof => {
                        let x = r.u();
                        let value = self.reg(base, x);
                        let target = self.vm.acc;
                        self.vm.acc = Value::bool(tri!(self.instance_of(value, target), start));
                    }
                    Op::Jump => {
                        let off = r.j();
                        r.pc = target(start, off);
                    }
                    Op::JumpLoop => {
                        let off = r.j();
                        r.pc = target(start, off);
                        tri!(self.tick(), start);
                    }
                    Op::JumpIfTrue | Op::JumpIfFalse => {
                        let off = r.j();
                        let v = self.vm.acc;
                        let truthy = match v {
                            Value::TRUE => true,
                            Value::FALSE => false,
                            _ => self.to_boolean(v),
                        };
                        if truthy == (op == Op::JumpIfTrue) {
                            r.pc = target(start, off);
                        }
                    }
                    Op::JumpIfNullish | Op::JumpIfNotNullish => {
                        let off = r.j();
                        if self.vm.acc.is_nullish() == (op == Op::JumpIfNullish) {
                            r.pc = target(start, off);
                        }
                    }
                    Op::JumpIfUndefined | Op::JumpIfNotUndefined => {
                        let off = r.j();
                        if self.vm.acc.is_undefined() == (op == Op::JumpIfUndefined) {
                            r.pc = target(start, off);
                        }
                    }
                    Op::Call | Op::CallUndefined | Op::CallEval | Op::New => {
                        let callee_reg = r.u();
                        let recv = if op == Op::Call {
                            self.reg(base, r.u())
                        } else {
                            Value::UNDEFINED
                        };
                        let (first, argc) = (r.u(), r.u());
                        if op != Op::CallEval {
                            let _ic = r.u();
                        }
                        let callee = self.reg(base, callee_reg);
                        let args = base as usize + first;
                        // Its pc while the callee runs, and where it goes
                        // on after.
                        if let Some(top) = self.vm.frames.last_mut() {
                            top.pc = start as u32;
                            top.resume = r.pc as u32;
                        }
                        let construct = op == Op::New;
                        let Some(callable) = self.callable(callee) else {
                            let text = self.text_at(u, fi, start as u32);
                            let message = if construct {
                                messages::not_a_constructor(&text)
                            } else {
                                messages::not_a_function(&text)
                            };
                            fail!(self.throw_error(ErrorType::TypeError, message))
                        };
                        match callable {
                            Callable::Code { unit, func, env } => {
                                let (this, new_target) = if construct {
                                    if !self.is_constructor(callable) {
                                        let text = self.text_at(u, fi, start as u32);
                                        fail!(self.throw_error(
                                            ErrorType::TypeError,
                                            messages::not_a_constructor(&text)
                                        ))
                                    }
                                    let this = match self.construct_this(callee) {
                                        Ok(t) => t,
                                        Err(e) => fail!(e),
                                    };
                                    (this, callee)
                                } else {
                                    (recv, Value::UNDEFINED)
                                };
                                // A copy of the arguments above the caller's
                                // registers: the callee's own.
                                let copy = self.vm.stack.len();
                                for i in 0..argc {
                                    let v = self
                                        .vm
                                        .stack
                                        .get(args + i)
                                        .copied()
                                        .unwrap_or(Value::UNDEFINED);
                                    self.vm.stack.push(v);
                                }
                                if let Err(e) = self.push_frame(
                                    callee, unit, func, env, this, copy, argc, new_target,
                                    construct, false,
                                ) {
                                    self.vm.stack.truncate(copy);
                                    fail!(e)
                                }
                                continue 'frames;
                            }
                            Callable::Native { id } => {
                                if construct && !self.is_constructor(callable) {
                                    let text = self.text_at(u, fi, start as u32);
                                    fail!(self.throw_error(
                                        ErrorType::TypeError,
                                        messages::not_a_constructor(&text)
                                    ))
                                }
                                if let Err(e) = self.tick() {
                                    fail!(e)
                                }
                                match realm::call_native(self, id, recv, args, argc) {
                                    Ok(v) => self.vm.acc = v,
                                    Err(e) => fail!(e),
                                }
                            }
                        }
                    }
                    Op::Return => {
                        let mut value = self.vm.acc;
                        let Some(frame) = self.vm.frames.pop() else {
                            return Ok(value);
                        };
                        self.vm.stack.truncate(frame.args as usize);
                        if frame.construct && value.as_object().is_none() {
                            value = frame.this;
                        }
                        self.vm.acc = value;
                        if frame.entry {
                            return Ok(value);
                        }
                        continue 'frames;
                    }
                    Op::Throw => {
                        self.vm.exception = self.vm.acc;
                        if let Some(top) = self.vm.frames.last_mut() {
                            top.pc = start as u32;
                        }
                        fail!(Abrupt::Throw)
                    }
                    Op::Debugger => {}
                    Op::ForInEnumerate => {
                        let v = self.vm.acc;
                        self.vm.acc = tri!(self.for_in_start(v), start);
                    }
                    Op::ForInNext => {
                        let x = r.u();
                        let it = self.reg(base, x);
                        self.vm.acc = self.for_in_next(it);
                    }
                }
            }
        }
    }

    fn top(&self) -> &Frame {
        match self.vm.frames.last() {
            Some(f) => f,
            None => no_frame(),
        }
    }

    fn name_text(&self, unit: u32, func: u32, k: usize) -> String {
        match self
            .vm
            .units
            .get(unit as usize)
            .and_then(|u| u.code.functions.get(func as usize))
            .and_then(|f| f.constants.get(k))
        {
            Some(Constant::String(s)) => String::from_utf16_lossy(s),
            _ => String::new(),
        }
    }

    /// Whether a callable may be called with `new`.
    fn is_constructor(&self, c: Callable) -> bool {
        match c {
            Callable::Code { unit, func, .. } => self
                .vm
                .units
                .get(unit as usize)
                .and_then(|u| u.code.functions.get(func as usize))
                .is_some_and(|f| f.kind == FunctionKind::Normal),
            Callable::Native { id } => realm::native_constructs(id),
        }
    }

    /// The object `new F` makes: its prototype `F.prototype` if an
    /// object, else the realm's `Object.prototype`.
    fn construct_this(&mut self, callee: Value) -> Res<Value> {
        let key = self.realm().prototype_key;
        let proto = self.get_value(callee, key)?;
        let proto = if proto.as_object().is_some() {
            proto
        } else {
            self.realm().object_prototype
        };
        self.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary))
    }
}

/// The realm is made before anything runs; reaching here is a bug of
/// the engine's own.
#[cold]
fn unreachable_realm() -> ! {
    panic!("the interpreter ran before its realm was made")
}

/// A frame is pushed before any instruction runs; reaching here is a
/// bug of the engine's own.
#[cold]
fn no_frame() -> ! {
    panic!("an instruction ran with no frame")
}
