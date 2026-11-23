//! Generators' built-ins (ECMA-262 sections 27.3 to 27.5):
//! `%GeneratorFunction%`, its prototype, and `%GeneratorPrototype%` with
//! `next`, `return` and `throw`, in the reference browser's order (the
//! generator probe, Chrome 157). A call of `next`, `return` or `throw`
//! from the interpreter resumes the generator on the caller's own loop
//! ([`crate::interp`]'s generator module); called otherwise, the natives
//! here resume it and run it to its next stop.

use super::{CONFIGURABLE, Call, Entry, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::interp::Res;
use crate::interp::generator::Resume;
use crate::value::Value;

/// `%GeneratorPrototype%`: given `%GeneratorFunction.prototype%`.
pub(crate) const GENERATOR_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Given(0), CONFIGURABLE),
    method("next", 1, next),
    method("return", 1, return_),
    method("throw", 1, throw),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("Generator"),
        CONFIGURABLE,
    ),
];

/// `%GeneratorFunction.prototype%`: given `%GeneratorPrototype%` and
/// `%GeneratorFunction%`.
pub(crate) const GENERATOR_FUNCTION_PROTO: &[Entry] = &[
    entry(Name::Str("prototype"), What::Given(0), CONFIGURABLE),
    entry(Name::Str("constructor"), What::Given(1), CONFIGURABLE),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("GeneratorFunction"),
        CONFIGURABLE,
    ),
];

/// `%AsyncFunction.prototype%`: given `%AsyncFunction%`.
pub(crate) const ASYNC_FUNCTION_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Given(0), CONFIGURABLE),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("AsyncFunction"),
        CONFIGURABLE,
    ),
];

/// How native `index` of [`GENERATOR_PROTO`] resumes a generator.
pub(crate) fn resume_of(index: u32) -> Option<Resume> {
    match index {
        1 => Some(Resume::Next),
        2 => Some(Resume::Return),
        3 => Some(Resume::Throw),
        _ => None,
    }
}

fn run(agent: &mut Agent, c: Call, how: Resume) -> Res<Value> {
    let sent = agent.arg(c, 0);
    match agent.resume_generator(c.this, how, sent, true, c.callee)? {
        Some(v) => Ok(v),
        None => agent.run_frames(),
    }
}

fn next(agent: &mut Agent, c: Call) -> Res<Value> {
    run(agent, c, Resume::Next)
}

fn return_(agent: &mut Agent, c: Call) -> Res<Value> {
    run(agent, c, Resume::Return)
}

fn throw(agent: &mut Agent, c: Call) -> Res<Value> {
    run(agent, c, Resume::Throw)
}
