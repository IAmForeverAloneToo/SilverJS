//! `Error.prototype.stack` as the reference browser has it (design note,
//! section 1.11; Chrome 157's observed output, docs/chrome-surface.md):
//! an own accessor of each error, its getter and setter shared, the
//! frames captured when the error is made (at most
//! `Error.stackTraceLimit`), the text made when `stack` is first read
//! (`Error.prepareStackTrace`'s, if a function) and kept.
//!
//! A frame reads `at name (url:line:column)`: a method's name after its
//! receiver's type (`Object.m`), a construction's after `new`, a
//! native's location `<anonymous>`, code from a string's
//! `eval at caller (location), <anonymous>:line:column`; a script's top
//! level, and a function without a name, show the location alone.

use super::{Call, Entry, method};
use crate::agent::Agent;
use crate::internal::{InternalCell, StackData, StackFrame};
use crate::interp::{Origin, Res};
use crate::object::{Callable, Descriptor, Got, OwnProperty};
use crate::string::PropertyKey;
use crate::value::{RefKind, Value};

/// The accessor pair every error's `stack` shares (named `""`, as the
/// reference browser's are).
pub(crate) const ACCESSORS: &[Entry] = &[method("", 0, stack_get), method("", 1, stack_set)];

/// How far a nested `eval`'s origin is followed when shown.
const MAX_EVAL_DEPTH: usize = 64;

impl Agent {
    /// The key a captured stack is kept under: a symbol no script holds,
    /// which own-key listings leave out.
    fn stack_key(&self) -> PropertyKey {
        match self.vm.stack_symbol.as_reference() {
            Some((RefKind::Symbol, s)) => PropertyKey::Symbol(s),
            _ => PropertyKey::Index(u32::MAX),
        }
    }

    /// `Error.stackTraceLimit` as a count, read without calling anything;
    /// `None` when it is not a number (no stack is captured).
    fn stack_trace_limit(&self) -> Option<usize> {
        let error = self.realm().intrinsics.errors.first().copied()?;
        let key = self.realm().stack_trace_limit_key;
        match self.heap.get(error.as_object()?, key) {
            Got::Value(v) => {
                let n = v.as_number()?;
                if n.is_nan() {
                    return Some(0);
                }
                Some(if n <= 0.0 { 0 } else { n.min(1e6) as usize })
            }
            Got::Getter(_) => None,
        }
    }

    /// The frames now on the stack, the topmost first: the functions'
    /// and the natives' between them. The first native (an error's
    /// constructor, `captureStackTrace`) is left out when `skip_native`,
    /// and every frame down to and including `until`'s when it is a
    /// function.
    fn capture_frames(&self, limit: usize, skip_native: bool, until: Value) -> Vec<StackFrame> {
        let mut out = Vec::new();
        let natives = &self.vm.natives;
        let frames = &self.vm.frames;
        let (mut ni, mut fi) = (natives.len(), frames.len());
        let mut skip_native = skip_native;
        let mut skipping = !until.is_undefined();
        let top = frames.len();
        while out.len() < limit {
            let entry = if let Some(n) = ni.checked_sub(1).and_then(|i| natives.get(i))
                && n.below as usize >= fi
            {
                ni -= 1;
                StackFrame {
                    unit: 0,
                    func: 0,
                    pc: 0,
                    receiver: n.this,
                    callee: n.callee,
                    native: true,
                    construct: n.construct,
                    accessor: false,
                    is_async: false,
                }
            } else if let Some(f) = fi.checked_sub(1).and_then(|i| frames.get(i)) {
                // The running frame's place is the instruction under way;
                // a frame waiting on a call, its call's.
                let running = fi == top && natives.last().is_none_or(|n| (n.below as usize) < top);
                fi -= 1;
                StackFrame {
                    unit: f.unit,
                    func: f.func,
                    pc: if running { self.vm.pc } else { f.pc },
                    receiver: f.this,
                    callee: f.closure,
                    native: false,
                    construct: f.construct,
                    accessor: f.accessor,
                    is_async: false,
                }
            } else {
                break;
            };
            if entry.native && skip_native {
                skip_native = false;
                continue;
            }
            if entry.native && !self.native_shown(entry.callee) {
                continue;
            }
            skip_native = false;
            if skipping {
                if entry.callee == until {
                    skipping = false;
                }
                continue;
            }
            out.push(entry);
        }
        // Below the frames, the async functions waiting on the one at the
        // bottom, each at its `await`.
        let mut driver = frames.first().map_or(Value::UNDEFINED, |f| f.generator);
        while out.len() < limit && !skipping {
            let Some(next) = self.async_awaiter(driver) else {
                break;
            };
            let Some(frame) = self.suspended_frame(next) else {
                break;
            };
            out.push(frame);
            driver = next;
        }
        out
    }

    /// Gives `o` a stack: the frames captured now, kept under the hidden
    /// key, and the shared `stack` accessor (not enumerable,
    /// configurable). `message` is the text the heading will show.
    pub(crate) fn capture_stack(
        &mut self,
        o: Value,
        message: Value,
        skip_native: bool,
        until: Value,
    ) -> Res<()> {
        let Some(i) = o.as_object() else {
            return Ok(());
        };
        let (frames, formatted) = match self.stack_trace_limit() {
            Some(limit) => (self.capture_frames(limit, skip_native, until), Value::HOLE),
            None => (Vec::new(), Value::UNDEFINED),
        };
        self.vm.temps.extend([o, message]);
        let cell = self.alloc_internal(InternalCell::Stack(StackData {
            frames,
            message,
            formatted,
        }));
        self.vm.temps.truncate(self.vm.temps.len() - 2);
        let cell = cell?;
        let key = self.stack_key();
        self.heap.define_own_property(
            i,
            key,
            Descriptor {
                value: Some(cell),
                writable: Some(true),
                enumerable: Some(false),
                configurable: Some(true),
                ..Descriptor::default()
            },
        );
        let (get, set) = (
            self.realm().intrinsics.stack_get,
            self.realm().intrinsics.stack_set,
        );
        let stack = self
            .heap
            .atoms
            .key(&"stack".encode_utf16().collect::<Vec<u16>>());
        self.define_or_throw(
            o,
            stack,
            Descriptor {
                get: Some(get),
                set: Some(set),
                enumerable: Some(false),
                configurable: Some(true),
                ..Descriptor::default()
            },
        )
    }

    /// The stack data `o` holds, if any.
    fn stack_cell(&self, o: Value) -> Option<u32> {
        let key = self.stack_key();
        match self.heap.get_own_property(o.as_object()?, key)? {
            OwnProperty::Data { value, .. } => match value.as_reference()? {
                (RefKind::Internal, c) => Some(c),
                _ => None,
            },
            OwnProperty::Accessor { .. } => None,
        }
    }

    fn stack_data(&self, cell: u32) -> Option<&StackData> {
        match self.heap.internals.get(cell)? {
            InternalCell::Stack(s) => Some(s),
            _ => None,
        }
    }

    /// The UTF-8 offsets unit `u`'s lines begin at.
    fn line_starts(&self, u: u32) -> Vec<u32> {
        let Some(installed) = self.vm.units.get(u as usize) else {
            return vec![0];
        };
        installed
            .lines
            .get_or_init(|| {
                let src = installed.code.source.as_bytes();
                let mut starts = vec![0u32];
                let mut i = 0;
                while i < src.len() {
                    let b = src.get(i).copied().unwrap_or(0);
                    let next = match b {
                        b'\n' => Some(i + 1),
                        b'\r' if src.get(i + 1) == Some(&b'\n') => {
                            i += 1;
                            Some(i + 1)
                        }
                        b'\r' => Some(i + 1),
                        // U+2028 and U+2029: E2 80 A8 / A9.
                        0xe2 if src.get(i + 1) == Some(&0x80)
                            && matches!(src.get(i + 2), Some(0xa8 | 0xa9)) =>
                        {
                            i += 2;
                            Some(i + 1)
                        }
                        _ => None,
                    };
                    if let Some(n) = next {
                        starts.push(u32::try_from(n).unwrap_or(u32::MAX));
                    }
                    i += 1;
                }
                starts
            })
            .clone()
    }

    /// The line and column (both from 1, the column in UTF-16 units) of
    /// where function `f` of unit `u` stands at `pc`.
    fn line_column(&self, u: u32, f: u32, pc: u32) -> (usize, usize) {
        let Some(installed) = self.vm.units.get(u as usize) else {
            return (1, 1);
        };
        let offset = installed
            .code
            .functions
            .get(f as usize)
            .and_then(|code| code.positions.iter().rev().find(|p| p.pc <= pc))
            .map_or(0, |p| p.at as usize);
        let starts = self.line_starts(u);
        let line = starts.partition_point(|&s| s as usize <= offset).max(1);
        let start = starts.get(line - 1).copied().unwrap_or(0) as usize;
        let column = installed
            .code
            .source
            .get(start..offset)
            .map_or(0, |t| t.encode_utf16().count());
        (line, column + 1)
    }

    /// Where function `f` of unit `u` stands at `pc`: `url:line:column`,
    /// or for code from a string `eval at caller (its location),
    /// <anonymous>:line:column`.
    fn location(&self, u: u32, f: u32, pc: u32, depth: usize) -> String {
        let (line, column) = self.line_column(u, f, pc);
        match self.vm.units.get(u as usize).map(|i| &i.origin) {
            Some(Origin::Eval {
                caller,
                caller_name,
                ..
            }) => {
                let at = match caller {
                    Some((cu, cf, cpc)) if depth < MAX_EVAL_DEPTH => {
                        self.location(*cu, *cf, *cpc, depth + 1)
                    }
                    _ => "<anonymous>".to_owned(),
                };
                format!("eval at {caller_name} ({at}), <anonymous>:{line}:{column}")
            }
            Some(Origin::Script(url)) if !url.is_empty() => format!("{url}:{line}:{column}"),
            _ => format!("<anonymous>:{line}:{column}"),
        }
    }

    /// A function's own `name`, if a string (a getter's `get g`), read
    /// without calling anything.
    fn own_name(&self, f: Value) -> Option<String> {
        let key = self.realm().name_key;
        match self.heap.get_own_property(f.as_object()?, key)? {
            OwnProperty::Data { value, .. } => self
                .string_units(value)
                .map(|u| String::from_utf16_lossy(&u)),
            OwnProperty::Accessor { .. } => None,
        }
    }

    /// The type a method frame names its receiver by: a function by its
    /// name, an object by its constructor's; none for a primitive, for
    /// the global object and for nothing.
    fn receiver_type(&self, receiver: Value) -> Option<String> {
        let o = receiver.as_object()?;
        if receiver == self.realm().global {
            return None;
        }
        if self.callable(receiver).is_some() {
            return self.own_name(receiver).filter(|n| !n.is_empty());
        }
        // A generator is named after its function (`thrower.next`).
        if let Some(f) = self.generator_function(receiver) {
            return self.own_name(f).filter(|n| !n.is_empty());
        }
        let key = self.realm().constructor_key;
        let mut at = Some(o);
        while let Some(i) = at {
            if let Some(OwnProperty::Data { value, .. }) = self.heap.get_own_property(i, key) {
                if self.callable(value).is_some() {
                    return self.own_name(value).filter(|n| !n.is_empty());
                }
                return None;
            }
            at = self.heap.get_prototype_of(i).as_object();
        }
        Some("Object".to_owned())
    }

    /// The name a frame shows for its function: a native's, a
    /// function's own; `eval` for code from a string; none for a script.
    fn frame_name(&self, f: &StackFrame) -> Option<String> {
        if f.native {
            return match self.callable(f.callee) {
                Some(Callable::Native { id, .. }) => Some(match self.host_name(id) {
                    Some(n) => n.to_owned(),
                    None => super::native_name(id),
                }),
                _ => None,
            };
        }
        let origin = self.vm.units.get(f.unit as usize).map(|i| &i.origin);
        let from_string = matches!(origin, Some(Origin::Eval { .. }));
        let function_ctor = matches!(origin, Some(Origin::Eval { function: true, .. }));
        if f.func == 0 {
            return from_string.then(|| "eval".to_owned());
        }
        if function_ctor && f.func == 1 {
            return Some("eval".to_owned());
        }
        self.own_name(f.callee).filter(|n| !n.is_empty())
    }

    /// Whether a frame shows its function's name with no receiver: a
    /// class's initializer (`<static_initializer>`) and a private method
    /// or accessor (`#m`, `get #g`), as Chrome 157 shows them.
    fn bare_frame(&self, f: &StackFrame, name: Option<&str>) -> bool {
        if f.native {
            return false;
        }
        let kind = self
            .vm
            .units
            .get(f.unit as usize)
            .and_then(|u| u.code.functions.get(f.func as usize))
            .map(|c| c.kind);
        kind == Some(crate::compiler::FunctionKind::Initializer)
            || name.is_some_and(|n| {
                n.starts_with('#') || n.starts_with("get #") || n.starts_with("set #")
            })
    }

    /// The name `receiver` holds `callee` under, along its prototype
    /// chain (data properties only), if it does.
    fn method_name_of(&self, receiver: Value, callee: Value) -> Option<String> {
        self.callable(callee)?;
        let mut at = receiver.as_object();
        while let Some(o) = at {
            for key in self.heap.own_keys(o) {
                if let (PropertyKey::Atom(a), Some(OwnProperty::Data { value, .. })) =
                    (key, self.heap.get_own_property(o, key))
                    && value == callee
                {
                    return Some(String::from_utf16_lossy(self.heap.atoms.units(a)));
                }
            }
            at = self.heap.get_prototype_of(o).as_object();
        }
        None
    }

    /// Whether a native's frame is shown: `Function.prototype.call` and
    /// `apply` are not, as the reference browser shows them.
    fn native_shown(&self, callee: Value) -> bool {
        let Some(Callable::Native { id, .. }) = self.callable(callee) else {
            return true;
        };
        let table = super::table::FUNCTION_PROTO;
        ![super::id_of(table, "call"), super::id_of(table, "apply")].contains(&Some(id))
    }

    /// One frame's text, after `at `.
    fn frame_text(&self, f: &StackFrame) -> String {
        let text = self.frame_text_of(f);
        if f.is_async {
            format!("async {text}")
        } else {
            text
        }
    }

    fn frame_text_of(&self, f: &StackFrame) -> String {
        let name = self.frame_name(f);
        let shown = if f.construct {
            Some(format!("new {}", name.as_deref().unwrap_or("<anonymous>")))
        } else if f.accessor || (!f.native && f.func == 0) || self.bare_frame(f, name.as_deref()) {
            name
        } else {
            match self.receiver_type(f.receiver) {
                // A method: `Type.name`, the name the receiver holds it
                // under standing in for none.
                Some(t) => {
                    let n = name.or_else(|| self.method_name_of(f.receiver, f.callee));
                    Some(format!("{t}.{}", n.as_deref().unwrap_or("<anonymous>")))
                }
                None => name,
            }
        };
        let location = if f.native {
            "<anonymous>".to_owned()
        } else {
            self.location(f.unit, f.func, f.pc, 0)
        };
        match shown {
            Some(n) => format!("{n} ({location})"),
            None => location,
        }
    }

    /// The text of `o`'s stack: its heading (`name: message`, the name
    /// read now, the message as it was), then a line a frame.
    fn format_stack(&mut self, o: Value, cell: u32) -> Res<Value> {
        let Some(data) = self.stack_data(cell).cloned() else {
            return Ok(Value::UNDEFINED);
        };
        let error = self
            .realm()
            .intrinsics
            .errors
            .first()
            .copied()
            .unwrap_or(Value::UNDEFINED);
        let key = self
            .heap
            .atoms
            .key(&"prepareStackTrace".encode_utf16().collect::<Vec<u16>>());
        let prepare = self.get_value(error, key)?;
        if self.is_callable(prepare) {
            let sites = self.call_sites(&data.frames)?;
            self.vm.temps.push(sites);
            let r = self.call_value(prepare, error, &[o, sites]);
            self.vm.temps.pop();
            return r;
        }
        let name_key = self.realm().name_key;
        let name = self.get_value(o, name_key)?;
        let name = if name.is_undefined() {
            "Error".encode_utf16().collect()
        } else {
            self.text_units(name)?
        };
        let message = self.string_units(data.message).unwrap_or_default();
        let mut text = if name.is_empty() {
            message
        } else if message.is_empty() {
            name
        } else {
            let mut t = name;
            t.extend(": ".encode_utf16());
            t.extend(message);
            t
        };
        for f in &data.frames {
            text.extend("\n    at ".encode_utf16());
            text.extend(self.frame_text(f).encode_utf16());
        }
        self.new_string_units(&text)
    }
}

/// The getter of every error's `stack`: the receiver's stack, made at
/// its first read and kept; undefined for an object without one.
fn stack_get(agent: &mut Agent, c: Call) -> Res<Value> {
    let Some(cell) = agent.stack_cell(c.this) else {
        return Ok(Value::UNDEFINED);
    };
    let formatted = agent
        .stack_data(cell)
        .map_or(Value::UNDEFINED, |d| d.formatted);
    if formatted != Value::HOLE {
        return Ok(formatted);
    }
    let text = agent.format_stack(c.this, cell)?;
    if let Some(InternalCell::Stack(d)) = agent.heap.internals.get_mut(cell) {
        d.formatted = text;
    }
    Ok(text)
}

/// The setter: the value is the receiver's stack from now on (the
/// accessor stays).
fn stack_set(agent: &mut Agent, c: Call) -> Res<Value> {
    let v = agent.arg(c, 0);
    if let Some(cell) = agent.stack_cell(c.this)
        && let Some(InternalCell::Stack(d)) = agent.heap.internals.get_mut(cell)
    {
        d.formatted = v;
    }
    Ok(Value::UNDEFINED)
}

/// `CallSite.prototype`, in the reference browser's order: what
/// `Error.prepareStackTrace` is given a frame as.
pub(crate) const CALL_SITE: &[Entry] = &[
    super::CONSTRUCTOR,
    method("getColumnNumber", 0, site_column),
    method("getEnclosingColumnNumber", 0, site_enclosing_column),
    method("getEnclosingLineNumber", 0, site_enclosing_line),
    method("getEvalOrigin", 0, site_eval_origin),
    method("getFileName", 0, site_file_name),
    method("getFunction", 0, site_function),
    method("getFunctionName", 0, site_function_name),
    method("getLineNumber", 0, site_line),
    method("getMethodName", 0, site_method_name),
    method("getPosition", 0, site_position),
    method("getPromiseIndex", 0, site_null),
    method("getScriptNameOrSourceURL", 0, site_file_name),
    method("getScriptHash", 0, site_script_hash),
    method("getThis", 0, site_this),
    method("getTypeName", 0, site_type_name),
    method("isAsync", 0, site_false),
    method("isConstructor", 0, site_is_constructor),
    method("isEval", 0, site_is_eval),
    method("isNative", 0, site_false),
    method("isPromiseAll", 0, site_false),
    method("isToplevel", 0, site_is_toplevel),
    method("toString", 0, site_to_string),
];

/// `CallSite` itself, which script cannot construct.
pub(crate) const CALL_SITE_CTOR: &[Entry] = &[method("CallSite", 0, call_site_ctor)];

fn call_site_ctor(agent: &mut Agent, _: Call) -> Res<Value> {
    Err(agent.type_error("Illegal constructor"))
}

impl Agent {
    /// The CallSite objects of `frames`, as an array.
    fn call_sites(&mut self, frames: &[StackFrame]) -> Res<Value> {
        let out = self.array_of(&[])?;
        self.vm.temps.push(out);
        let proto = self.realm().intrinsics.call_site_prototype;
        let mut result = Ok(out);
        for (i, f) in frames.iter().enumerate() {
            let flags =
                i32::from(f.native) | i32::from(f.construct) << 1 | i32::from(f.accessor) << 2;
            let site = self.alloc_object(crate::heap::ObjectCell::with_internal(
                proto,
                crate::object::ObjectKind::Ordinary,
                vec![
                    f.receiver,
                    f.callee,
                    Value::int(f.unit as i32),
                    Value::int(f.func as i32),
                    Value::int(f.pc as i32),
                    Value::int(flags),
                ],
            ));
            match site {
                Ok(s) => {
                    self.create_data_property(out, PropertyKey::Index(i as u32), s);
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.vm.temps.pop();
        result
    }

    /// The frame a CallSite stands for.
    fn site(&mut self, v: Value) -> Res<StackFrame> {
        let proto = self.realm().intrinsics.call_site_prototype;
        let internal = v
            .as_object()
            .filter(|&o| self.heap.get_prototype_of(o) == proto)
            .and_then(|o| self.heap.objects.get(o))
            .map(|c| c.internal().to_vec())
            .filter(|i| i.len() == 6);
        let Some(i) = internal else {
            return Err(self.type_error("CallSite method called on non-CallSite object"));
        };
        let int = |k: usize| i.get(k).and_then(|v| v.as_int()).unwrap_or(0);
        let flags = int(5);
        Ok(StackFrame {
            receiver: i.first().copied().unwrap_or(Value::UNDEFINED),
            callee: i.get(1).copied().unwrap_or(Value::UNDEFINED),
            unit: int(2) as u32,
            func: int(3) as u32,
            pc: int(4) as u32,
            native: flags & 1 != 0,
            construct: flags & 2 != 0,
            accessor: flags & 4 != 0,
            is_async: false,
        })
    }

    /// Whether frame `f` runs strict code (its `this` and function are
    /// not shown then).
    fn site_strict(&self, f: &StackFrame) -> bool {
        f.native
            || self
                .vm
                .units
                .get(f.unit as usize)
                .and_then(|u| u.code.functions.get(f.func as usize))
                .is_some_and(|c| c.strict)
    }

    fn site_origin(&self, f: &StackFrame) -> Option<Origin> {
        if f.native {
            return None;
        }
        self.vm.units.get(f.unit as usize).map(|u| u.origin.clone())
    }
}

fn site_column(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    if f.native {
        return Ok(Value::NULL);
    }
    Ok(Value::number(
        agent.line_column(f.unit, f.func, f.pc).1 as f64,
    ))
}

fn site_line(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    if f.native {
        return Ok(Value::NULL);
    }
    Ok(Value::number(
        agent.line_column(f.unit, f.func, f.pc).0 as f64,
    ))
}

/// Where the frame's function begins.
fn enclosing(agent: &Agent, f: &StackFrame) -> Option<(usize, usize)> {
    if f.native {
        return None;
    }
    let installed = agent.vm.units.get(f.unit as usize)?;
    let start = installed.code.functions.get(f.func as usize)?.span.0 as usize;
    let starts = agent.line_starts(f.unit);
    let line = starts.partition_point(|&s| s as usize <= start).max(1);
    let begin = starts.get(line - 1).copied().unwrap_or(0) as usize;
    let column = installed
        .code
        .source
        .get(begin..start)
        .map_or(0, |t| t.encode_utf16().count());
    Some((line, column + 1))
}

fn site_enclosing_column(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    Ok(enclosing(agent, &f).map_or(Value::NULL, |(_, col)| Value::number(col as f64)))
}

fn site_enclosing_line(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    Ok(enclosing(agent, &f).map_or(Value::NULL, |(line, _)| Value::number(line as f64)))
}

fn site_eval_origin(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    if !matches!(agent.site_origin(&f), Some(Origin::Eval { .. })) {
        return Ok(Value::UNDEFINED);
    }
    let text = agent.location(f.unit, f.func, f.pc, 0);
    let origin = text.split(", <anonymous>").next().unwrap_or("").to_owned();
    agent.new_string(&origin)
}

fn site_file_name(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    match agent.site_origin(&f) {
        Some(Origin::Script(url)) if !url.is_empty() => agent.new_string(&url),
        _ => Ok(Value::UNDEFINED),
    }
}

fn site_function(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    Ok(if agent.site_strict(&f) {
        Value::UNDEFINED
    } else {
        f.callee
    })
}

fn site_this(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    Ok(if agent.site_strict(&f) {
        Value::UNDEFINED
    } else {
        f.receiver
    })
}

fn site_function_name(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    match agent.frame_name(&f) {
        Some(n) => agent.new_string(&n),
        None => Ok(Value::NULL),
    }
}

/// The name the receiver holds the function under, if any.
fn site_method_name(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    if f.construct {
        return Ok(Value::NULL);
    }
    match agent.method_name_of(f.receiver, f.callee) {
        Some(n) => agent.new_string(&n),
        None => Ok(Value::NULL),
    }
}

fn site_position(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    if f.native {
        return Ok(Value::int(0));
    }
    let Some(installed) = agent.vm.units.get(f.unit as usize) else {
        return Ok(Value::int(0));
    };
    let offset = installed
        .code
        .functions
        .get(f.func as usize)
        .and_then(|code| code.positions.iter().rev().find(|p| p.pc <= f.pc))
        .map_or(0, |p| p.at as usize);
    let n = installed
        .code
        .source
        .get(..offset)
        .map_or(0, |t| t.encode_utf16().count());
    Ok(Value::number(n as f64))
}

fn site_null(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.site(c.this)?;
    Ok(Value::NULL)
}

fn site_false(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.site(c.this)?;
    Ok(Value::FALSE)
}

fn site_script_hash(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.site(c.this)?;
    Ok(agent.realm().strings.empty)
}

fn site_type_name(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    if f.construct {
        return Ok(Value::NULL);
    }
    match agent.receiver_type(f.receiver) {
        Some(t) => agent.new_string(&t),
        None => Ok(Value::NULL),
    }
}

fn site_is_constructor(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    Ok(Value::bool(f.construct))
}

fn site_is_eval(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    Ok(Value::bool(matches!(
        agent.site_origin(&f),
        Some(Origin::Eval { .. })
    )))
}

/// A frame of the global object, or of nothing, is top-level.
fn site_is_toplevel(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    if f.construct {
        return Ok(Value::FALSE);
    }
    Ok(Value::bool(
        f.receiver.is_nullish() || f.receiver == agent.realm().global,
    ))
}

fn site_to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.site(c.this)?;
    let text = agent.frame_text(&f);
    agent.new_string(&text)
}
