//! `eval` (ECMA-262 section 19.2.1): direct, in its caller's scope, and
//! indirect, in the global one. Eval code is compiled as its own unit
//! ([`crate::compiler::compile_eval`]) and run as a frame with its
//! caller's environment, `this` and `new.target`; a sloppy one's `var`s
//! and functions are first made in its caller's variable environment
//! (EvalDeclarationInstantiation, section 19.2.1.3), a function's kept
//! in an object of extra bindings on that environment.

use super::messages::{self, ErrorType};
use super::{Abrupt, Origin, Res};
use crate::agent::Agent;
use crate::compiler::{CompileError, FunctionKind, GlobalDecl};
use crate::heap::{InternalCell, ObjectCell};
use crate::object::{Descriptor, ObjectKind, OwnProperty};
use crate::string::PropertyKey;
use crate::value::{RefKind, Value};

/// An environment's slot names, which of them are lexical, and its
/// parent.
type EnvNames = (Vec<Box<[u16]>>, Vec<bool>, Value);

impl Agent {
    /// Compiles eval code, a SyntaxError thrown for what does not parse.
    fn compile_eval_code(
        &mut self,
        source: Value,
        strict: bool,
        cx: &crate::parser::EvalContext,
    ) -> Res<crate::compiler::CompiledUnit> {
        let text = String::from_utf16_lossy(&self.string_units(source).unwrap_or_default());
        match crate::compiler::compile_eval(&text, strict, cx) {
            Ok(u) => Ok(u),
            Err(CompileError::Syntax(e)) => {
                Err(self.throw_error(ErrorType::SyntaxError, e.message))
            }
            Err(e) => Err(self.throw_error(ErrorType::SyntaxError, e.to_string())),
        }
    }

    /// The names a scope descriptor gives environment `e`'s slots,
    /// whether each is lexical, and the environment's parent.
    fn env_names(&self, e: u32) -> Option<EnvNames> {
        let InternalCell::Env(env) = self.heap.internals.get(e)? else {
            return None;
        };
        let (unit, scope) = env.scope?;
        let info = self
            .vm
            .units
            .get(unit as usize)?
            .code
            .scopes
            .get(scope as usize)?;
        Some((info.names.clone(), info.lexical.clone(), env.parent))
    }

    /// EvalDeclarationInstantiation's checks and bindings for a sloppy
    /// eval's `decls`, with `context` its caller's environment and
    /// `var_env` its variable environment (undefined: the global one).
    fn eval_declarations(
        &mut self,
        decls: &[GlobalDecl],
        context: Value,
        var_env: Value,
    ) -> Res<()> {
        let names: Vec<(&[u16], bool)> = decls
            .iter()
            .filter_map(|d| match d {
                GlobalDecl::Var(n) => Some((&n[..], false)),
                GlobalDecl::Function(n, _) => Some((&n[..], true)),
                _ => None,
            })
            .collect();
        // A `var` may not be hoisted across a lexical binding of its name.
        for &(name, _) in &names {
            let mut at = context;
            while let Some((RefKind::Internal, e)) = at.as_reference() {
                let Some((slots, lexical, parent)) = self.env_names(e) else {
                    // A `with`'s object environment: past it.
                    at = match self.heap.internals.get(e) {
                        Some(InternalCell::Env(env)) => env.parent,
                        _ => Value::UNDEFINED,
                    };
                    continue;
                };
                let clash = slots.iter().zip(&lexical).any(|(n, &l)| l && **n == *name);
                if clash {
                    let text = String::from_utf16_lossy(name);
                    return Err(
                        self.throw_error(ErrorType::SyntaxError, messages::already_declared(&text))
                    );
                }
                if at == var_env {
                    break;
                }
                at = parent;
            }
            if var_env.is_undefined() {
                let key = self.heap.atoms.key(name);
                if self.realm().lexical.contains_key(&key) {
                    let text = String::from_utf16_lossy(name);
                    return Err(
                        self.throw_error(ErrorType::SyntaxError, messages::already_declared(&text))
                    );
                }
            }
        }
        for (name, function) in names {
            let key = self.heap.atoms.key(name);
            if var_env.is_undefined() {
                self.eval_global_binding(key, function)?;
            } else {
                self.eval_function_binding(var_env, name, key)?;
            }
        }
        Ok(())
    }

    /// A sloppy eval's `var` or function on the global object: made
    /// configurable (deletable), as eval's bindings are.
    fn eval_global_binding(&mut self, key: PropertyKey, function: bool) -> Res<()> {
        let g = self.realm().global.as_object().unwrap_or(0);
        let existing = self.heap.get_own_property(g, key);
        let extensible = self.heap.is_extensible(g);
        let refuse = |agent: &mut Agent| {
            let text = agent.key_display(key);
            agent.throw_error(
                ErrorType::TypeError,
                format!("Cannot redefine property: {text}"),
            )
        };
        match existing {
            None if !extensible => return Err(refuse(self)),
            None => {
                self.heap.define_own_property(
                    g,
                    key,
                    Descriptor {
                        value: Some(Value::UNDEFINED),
                        writable: Some(true),
                        enumerable: Some(true),
                        configurable: Some(true),
                        ..Descriptor::default()
                    },
                );
            }
            Some(p) if function => {
                let configurable = matches!(
                    p,
                    OwnProperty::Data {
                        configurable: true,
                        ..
                    } | OwnProperty::Accessor {
                        configurable: true,
                        ..
                    }
                );
                let plain = matches!(
                    p,
                    OwnProperty::Data {
                        writable: true,
                        enumerable: true,
                        ..
                    }
                );
                if configurable {
                    self.heap.define_own_property(
                        g,
                        key,
                        Descriptor {
                            value: Some(Value::UNDEFINED),
                            writable: Some(true),
                            enumerable: Some(true),
                            configurable: Some(true),
                            ..Descriptor::default()
                        },
                    );
                } else if !plain {
                    return Err(refuse(self));
                }
            }
            Some(_) => {}
        }
        Ok(())
    }

    /// A sloppy eval's `var` or function in a function's environment: a
    /// slot of its name if it has one, else a property of the
    /// environment's object of extra bindings.
    fn eval_function_binding(&mut self, var_env: Value, name: &[u16], key: PropertyKey) -> Res<()> {
        let Some((RefKind::Internal, e)) = var_env.as_reference() else {
            return Ok(());
        };
        if let Some((slots, _, _)) = self.env_names(e)
            && slots.iter().any(|n| **n == *name)
        {
            return Ok(());
        }
        let vars = match self.heap.internals.get(e) {
            Some(InternalCell::Env(env)) => env.vars,
            _ => return Ok(()),
        };
        let vars = if vars.as_object().is_some() {
            vars
        } else {
            let o = self.alloc_object(ObjectCell::new(Value::NULL, ObjectKind::Ordinary))?;
            self.heap.with_internal(e, |c| {
                if let InternalCell::Env(env) = c {
                    env.vars = o;
                }
            });
            o
        };
        if let Some(o) = vars.as_object()
            && self.heap.get_own_property(o, key).is_none()
        {
            self.heap.define_own_property(
                o,
                key,
                Descriptor {
                    value: Some(Value::UNDEFINED),
                    writable: Some(true),
                    enumerable: Some(true),
                    configurable: Some(true),
                    ..Descriptor::default()
                },
            );
        }
        Ok(())
    }

    /// The private names (`#x`, given without the `#`) of the classes
    /// around `context`.
    fn private_names_around(&self, context: Value) -> Vec<String> {
        let mut names = Vec::new();
        let mut at = context;
        while let Some((RefKind::Internal, e)) = at.as_reference() {
            let Some(InternalCell::Env(env)) = self.heap.internals.get(e) else {
                break;
            };
            if let Some((slots, _, _)) = self.env_names(e) {
                for n in slots.iter() {
                    if n.first() == Some(&u16::from(b'#')) {
                        names.push(String::from_utf16_lossy(n.get(1..).unwrap_or(&[])));
                    }
                }
            }
            at = env.parent;
        }
        names
    }

    /// The value of the binding `name` found from `context`, if a slot
    /// holds it.
    fn slot_named(&self, context: Value, name: &str) -> Option<Value> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut at = context;
        while let Some((RefKind::Internal, e)) = at.as_reference() {
            let InternalCell::Env(env) = self.heap.internals.get(e)? else {
                return None;
            };
            if let Some((slots, _, _)) = self.env_names(e)
                && let Some(i) = slots.iter().position(|n| **n == *units)
            {
                return env.slots.get(i).copied();
            }
            at = env.parent;
        }
        None
    }

    /// Starts a direct eval of `source` from the top frame, stopped at
    /// `pc`: its frame pushed, for the caller's loop to run.
    pub(crate) fn begin_direct_eval(&mut self, source: Value, pc: u32) -> Res<()> {
        let frame = *self.top_frame().ok_or(Abrupt::Throw)?;
        let code = self
            .vm
            .units
            .get(frame.unit as usize)
            .map(|u| std::rc::Rc::clone(&u.code));
        let Some(caller) = code
            .as_ref()
            .and_then(|c| c.functions.get(frame.func as usize))
        else {
            return Err(Abrupt::Throw);
        };
        let (strict, kind) = (caller.strict, caller.kind);
        let in_eval = code.as_ref().is_some_and(|c| c.eval);
        // `new.target` is allowed in a function's code: an arrow's, or
        // eval code's, if a function encloses it (whose binding it reads).
        let new_target = match kind {
            FunctionKind::Normal
            | FunctionKind::Method
            | FunctionKind::Generator
            | FunctionKind::ClassConstructor
            | FunctionKind::DerivedConstructor
            | FunctionKind::Initializer
            | FunctionKind::Async
            | FunctionKind::AsyncGenerator => true,
            _ if in_eval || kind == FunctionKind::Arrow => {
                self.slot_named(frame.context, "new.target").is_some()
            }
            _ => false,
        };
        let cx = crate::parser::EvalContext {
            new_target,
            // A method's code (or an arrow's in it): its home object.
            super_prop: self
                .slot_named(frame.context, "%home")
                .is_some_and(|h| h.as_object().is_some()),
            class_field: self.slot_named(frame.context, "%field").is_some(),
            privates: self.private_names_around(frame.context),
        };
        let unit = self.compile_eval_code(source, strict, &cx)?;
        let sloppy = unit.functions.first().is_some_and(|f| !f.strict);
        let decls = if sloppy {
            unit.globals.clone()
        } else {
            Vec::new()
        };
        let caller_name: std::rc::Rc<str> = if frame.func == 0 {
            if in_eval { "eval" } else { "<anonymous>" }.into()
        } else {
            self.function_name(frame.closure)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "<anonymous>".to_owned())
                .into()
        };
        let index = self.install(
            unit,
            Origin::Eval {
                caller: Some((frame.unit, frame.func, pc)),
                caller_name,
                function: false,
            },
        )?;
        // The caller's variable environment: its function's, or the
        // global one for a script's top level.
        let var_env = frame.fn_context;
        self.eval_declarations(&decls, frame.context, var_env)?;
        // Its `this` and `new.target`: the caller's (an arrow's, its
        // enclosing function's, kept in the environment).
        let (this, new_target) = if kind == FunctionKind::Arrow {
            let this = self
                .slot_named(frame.context, "this")
                .unwrap_or(self.realm().global);
            let nt = self
                .slot_named(frame.context, "new.target")
                .unwrap_or(Value::UNDEFINED);
            (this, nt)
        } else {
            (frame.this, frame.new_target)
        };
        let args = self.vm.stack.len();
        self.push_frame(
            Value::UNDEFINED,
            index,
            0,
            frame.context,
            this,
            args,
            0,
            new_target,
            false,
            false,
        )?;
        if let Some(top) = self.vm.frames.last_mut() {
            top.this = this;
            if sloppy {
                top.fn_context = var_env;
            }
        }
        Ok(())
    }

    /// Indirect eval (section 19.2.1.1 with direct false): sloppy unless
    /// it says otherwise, in the global scope, run to its value.
    pub(crate) fn indirect_eval(&mut self, source: Value) -> Res<Value> {
        let unit = self.compile_eval_code(source, false, &crate::parser::EvalContext::default())?;
        let sloppy = unit.functions.first().is_some_and(|f| !f.strict);
        let decls = if sloppy {
            unit.globals.clone()
        } else {
            Vec::new()
        };
        let origin = Origin::Eval {
            caller: self.caller_position(),
            caller_name: "<anonymous>".into(),
            function: false,
        };
        let index = self.install(unit, origin)?;
        self.eval_declarations(&decls, Value::UNDEFINED, Value::UNDEFINED)?;
        if self.vm.reentry >= super::MAX_REENTRY {
            return Err(self.throw_error(ErrorType::RangeError, messages::STACK_OVERFLOW));
        }
        let global = self.realm().global;
        let args = self.vm.stack.len();
        self.push_frame(
            Value::UNDEFINED,
            index,
            0,
            Value::UNDEFINED,
            global,
            args,
            0,
            Value::UNDEFINED,
            false,
            true,
        )?;
        self.vm.reentry += 1;
        let r = self.run_frames();
        self.vm.reentry -= 1;
        r
    }

    /// A function's own `name`, if a string.
    pub(crate) fn function_name(&self, f: Value) -> Option<String> {
        let key = self.realm().name_key;
        match self.heap.get_own_property(f.as_object()?, key)? {
            OwnProperty::Data { value, .. } => self
                .string_units(value)
                .map(|u| String::from_utf16_lossy(&u)),
            OwnProperty::Accessor { .. } => None,
        }
    }

    fn top_frame(&self) -> Option<&super::Frame> {
        self.vm.frames.last()
    }
}
