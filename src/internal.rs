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

#[derive(Debug, Clone)]
pub(crate) enum InternalCell {
    Env(Env),
    ForIn(ForIn),
}

impl Trace for InternalCell {
    fn trace(&self, out: &mut Vec<Value>) {
        match self {
            InternalCell::Env(e) => {
                out.push(e.parent);
                out.push(e.object);
                out.extend_from_slice(&e.slots);
            }
            InternalCell::ForIn(f) => {
                out.push(f.object);
                out.extend_from_slice(&f.keys);
            }
        }
    }

    fn extra_bytes(&self) -> usize {
        match self {
            InternalCell::Env(e) => e.slots.capacity() * 8,
            InternalCell::ForIn(f) => f.keys.capacity() * 8,
        }
    }
}
