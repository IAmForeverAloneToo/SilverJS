//! The interpreter's own cells (docs/design.md section 1.4): the
//! environments closures capture, and for-in's iterators. Script never
//! sees them; values name them with the internal tag
//! ([`crate::value::RefKind::Internal`]).

use crate::heap::Trace;
use crate::value::Value;

/// An environment (ECMA-262 section 9.1): the slots of one scope's
/// captured bindings, or, for `with`, an object whose properties are the
/// bindings.
#[derive(Debug, Clone)]
pub(crate) struct Env {
    /// The environment around it, or undefined: the global one.
    pub parent: Value,
    pub slots: Vec<Value>,
    /// The compiled unit and scope descriptor naming its slots, for
    /// lookups by name (`eval`, `with`).
    pub scope: Option<(u32, u32)>,
    /// A `with` statement's object; undefined for a declarative one.
    pub object: Value,
    /// The `var`s and functions a sloppy direct eval made in this
    /// (function) environment, as an object's properties; undefined for
    /// none.
    pub vars: Value,
}

/// A for-in's walk (section 14.7.5.10): the keys found at its start,
/// each checked again as it comes, since one deleted meanwhile is
/// skipped.
#[derive(Debug, Clone)]
pub(crate) struct ForIn {
    pub object: Value,
    pub keys: Vec<Value>,
    pub next: usize,
}

/// A frame of a captured stack: a function's activation (its unit,
/// function and pc) or a native's, with its receiver and callee.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StackFrame {
    pub unit: u32,
    pub func: u32,
    pub pc: u32,
    pub receiver: Value,
    pub callee: Value,
    pub native: bool,
    pub construct: bool,
    pub accessor: bool,
    /// An async function waiting at an `await` for the frames above it
    /// (`at async outer`).
    pub is_async: bool,
}

/// A stack captured for an error (or `Error.captureStackTrace`'s
/// object): its frames, the message as it was, and the text made of them
/// when `stack` is first read (a hole until then).
#[derive(Debug, Clone)]
pub(crate) struct StackData {
    pub frames: Vec<StackFrame>,
    pub message: Value,
    pub formatted: Value,
}

#[derive(Debug, Clone)]
pub(crate) enum InternalCell {
    Env(Env),
    ForIn(ForIn),
    Stack(StackData),
    /// A `Map`'s, `Set`'s or weak collection's entries.
    Table(Box<crate::table::Table>),
}

impl Trace for InternalCell {
    fn trace(&self, out: &mut Vec<Value>) {
        match self {
            InternalCell::Env(e) => {
                out.push(e.parent);
                out.push(e.object);
                out.push(e.vars);
                out.extend_from_slice(&e.slots);
            }
            InternalCell::ForIn(f) => {
                out.push(f.object);
                out.extend_from_slice(&f.keys);
            }
            InternalCell::Stack(s) => {
                out.push(s.message);
                out.push(s.formatted);
                for f in &s.frames {
                    out.push(f.receiver);
                    out.push(f.callee);
                }
            }
            InternalCell::Table(t) => t.trace(out),
        }
    }

    fn extra_bytes(&self) -> usize {
        match self {
            InternalCell::Env(e) => e.slots.capacity() * 8,
            InternalCell::ForIn(f) => f.keys.capacity() * 8,
            InternalCell::Stack(s) => s.frames.capacity() * std::mem::size_of::<StackFrame>(),
            InternalCell::Table(t) => std::mem::size_of::<crate::table::Table>() + t.extra_bytes(),
        }
    }
}
