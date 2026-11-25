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

/// `%AsyncGeneratorPrototype%`: given `%AsyncGeneratorFunction.prototype%`
/// (the async generator probe, Chrome 157).
pub(crate) const ASYNC_GENERATOR_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Given(0), CONFIGURABLE),
    method("next", 1, async_next),
    method("return", 1, async_return),
    method("throw", 1, async_throw),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("AsyncGenerator"),
        CONFIGURABLE,
    ),
];

/// `%AsyncGeneratorFunction.prototype%`: given `%AsyncGeneratorPrototype%`
/// and `%AsyncGeneratorFunction%`.
pub(crate) const ASYNC_GENERATOR_FUNCTION_PROTO: &[Entry] = &[
    entry(Name::Str("prototype"), What::Given(0), CONFIGURABLE),
    entry(Name::Str("constructor"), What::Given(1), CONFIGURABLE),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("AsyncGeneratorFunction"),
        CONFIGURABLE,
    ),
];

/// `%AsyncIteratorPrototype%`.
pub(crate) const ASYNC_ITERATOR_PROTO: &[Entry] = &[
    entry(
        Name::Sym(Wk::AsyncIterator),
        What::Method(0, async_iterator_self),
        super::HIDDEN,
    ),
    entry(
        Name::Sym(Wk::AsyncDispose),
        What::Method(0, async_dispose),
        super::HIDDEN,
    ),
];

fn async_iterator_self(_: &mut Agent, c: Call) -> Res<Value> {
    Ok(c.this)
}

/// `%AsyncIteratorPrototype%[@@asyncDispose]` (explicit resource
/// management, section 27.1.3.1): `this.return()` awaited, its value
/// dropped.
fn async_dispose(agent: &mut Agent, c: Call) -> Res<Value> {
    let promise = agent.new_promise()?;
    agent.vm.temps.push(promise);
    let r = (|| {
        let key = agent.name_key(Name::Str("return"));
        let ret = match agent.get_value(c.this, key) {
            Ok(r) => r,
            Err(crate::interp::Abrupt::Throw) => {
                let e = std::mem::replace(&mut agent.vm.exception, Value::UNDEFINED);
                return agent.reject_promise(promise, e);
            }
            Err(e) => return Err(e),
        };
        if ret.is_nullish() {
            return agent.resolve_promise(promise, Value::UNDEFINED);
        }
        if !agent.is_callable(ret) {
            let (r, o) = (agent.display(ret), agent.display(c.this));
            let e = agent.make_error(
                crate::interp::ErrorType::TypeError,
                &format!("'{r}' returned for property 'return' of object '{o}' is not a function"),
            )?;
            return agent.reject_promise(promise, e);
        }
        // `return()`'s result awaited, the promise then fulfilled with
        // undefined (or rejected as it is).
        // Called with no argument, as the reference browser and Test262
        // have it.
        let wrapped = agent.call_value(ret, c.this, &[]).and_then(|r| {
            let ctor = agent.realm().intrinsics.promise;
            agent.promise_resolve(ctor, r)
        });
        let wrapped = match wrapped {
            Ok(w) => w,
            Err(crate::interp::Abrupt::Throw) => {
                let e = std::mem::replace(&mut agent.vm.exception, Value::UNDEFINED);
                return agent.reject_promise(promise, e);
            }
            Err(e) => return Err(e),
        };
        agent.vm.temps.push(wrapped);
        let unwrap = agent.undefined_thunk();
        agent.vm.temps.pop();
        let cap = super::promise::Capability {
            promise,
            resolve: Value::UNDEFINED,
            reject: Value::UNDEFINED,
        };
        agent.perform_then(wrapped, unwrap?, Value::UNDEFINED, Some(cap));
        Ok(())
    })();
    agent.vm.temps.pop();
    r.map(|()| promise)
}

/// `%AsyncGeneratorPrototype%.next`, `return` and `throw` (sections
/// 27.6.1.2 to 27.6.1.4): a promise, rejected at once for what is no
/// async generator.
fn async_run(agent: &mut Agent, c: Call, how: Resume) -> Res<Value> {
    let sent = agent.arg(c, 0);
    if !agent.is_async_generator(c.this) {
        let name = match how {
            Resume::Next => "next",
            Resume::Throw => "throw",
            Resume::Return => "return",
        };
        let text = agent.display(c.this);
        let e = agent.make_error(
            crate::interp::ErrorType::TypeError,
            &crate::interp::messages::async_generator_incompatible(name, &text),
        )?;
        agent.vm.temps.push(e);
        let p = agent.new_promise();
        agent.vm.temps.pop();
        let p = p?;
        agent.reject_promise(p, e)?;
        return Ok(p);
    }
    agent.async_gen_enqueue(c.this, how, sent)
}

fn async_next(agent: &mut Agent, c: Call) -> Res<Value> {
    async_run(agent, c, Resume::Next)
}

fn async_return(agent: &mut Agent, c: Call) -> Res<Value> {
    async_run(agent, c, Resume::Return)
}

fn async_throw(agent: &mut Agent, c: Call) -> Res<Value> {
    async_run(agent, c, Resume::Throw)
}

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
