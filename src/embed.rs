//! The embedding API's running half (docs/design.md section 1.10):
//! natives written in Rust, and the operations that may run script
//! (`[[Get]]` calling a getter, calls, conversions), each giving back
//! what was thrown. SilverJS's own Test262 runner is built on this and
//! nothing private.

use std::rc::Rc;

use crate::agent::{Agent, Local, Scope};
use crate::interp::{Abrupt, ErrorType, Res};
use crate::string::PropertyKey;
use crate::value::Value;

/// How an operation that ran script ended abruptly.
#[derive(Clone, Copy)]
pub enum Thrown<'s> {
    /// It threw this value.
    Value(Local<'s>),
    /// The interrupt hook stopped it.
    Terminated,
}

/// What an operation that may run script gives back.
pub type JsResult<'s, T> = Result<T, Thrown<'s>>;

/// The error constructors an embedder may throw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Error,
    TypeError,
    RangeError,
    ReferenceError,
    SyntaxError,
    EvalError,
    UriError,
}

impl ErrorKind {
    fn to_type(self) -> ErrorType {
        match self {
            ErrorKind::Error => ErrorType::Error,
            ErrorKind::TypeError => ErrorType::TypeError,
            ErrorKind::RangeError => ErrorType::RangeError,
            ErrorKind::ReferenceError => ErrorType::ReferenceError,
            ErrorKind::SyntaxError => ErrorType::SyntaxError,
            ErrorKind::EvalError => ErrorType::EvalError,
            ErrorKind::UriError => ErrorType::UriError,
        }
    }
}

/// A realm of an agent's, by its index there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealmId(u32);

/// A native written by the embedder: given the scope of its call, its
/// `this` and its arguments; it returns a value or what it throws.
pub type HostFn =
    dyn for<'s> Fn(&mut Scope<'s>, Local<'s>, &[Local<'s>]) -> JsResult<'s, Local<'s>>;

/// The embedder's natives, which the agent keeps: a native's id past
/// [`HOST_BASE`] names one.
pub(crate) struct Host {
    pub name: String,
    pub f: Rc<HostFn>,
}

/// The first id of the embedder's natives (the built-ins' are below).
pub(crate) const HOST_BASE: u32 = 0x4000_0000;

/// Calls the embedder's native `index`.
pub(crate) fn call_host(agent: &mut Agent, index: u32, this: Value, args: &[Value]) -> Res<Value> {
    let Some(f) = agent.vm.hosts.get(index as usize).map(|h| Rc::clone(&h.f)) else {
        return Err(agent.throw_error(ErrorType::TypeError, "Illegal invocation"));
    };
    let base = agent.handles_len();
    let result = {
        let mut scope = Scope::over(agent);
        let this = scope.local_of(this);
        let args: Vec<Local<'_>> = args.iter().map(|&v| scope.local_of(v)).collect();
        match f(&mut scope, this, &args) {
            Ok(v) => Ok(scope.value_of(v)),
            Err(Thrown::Value(e)) => {
                let e = scope.value_of(e);
                Err((e, Abrupt::Throw))
            }
            Err(Thrown::Terminated) => Err((Value::UNDEFINED, Abrupt::Terminate)),
        }
    };
    agent.truncate_handles(base);
    match result {
        Ok(v) => Ok(v),
        Err((e, Abrupt::Throw)) => {
            agent.vm.exception = e;
            Err(Abrupt::Throw)
        }
        Err((_, a)) => Err(a),
    }
}

impl<'a> Scope<'a> {
    /// What an abrupt `Res` comes to, for the embedder: the exception
    /// taken out of the agent into a local.
    fn thrown(&mut self, a: Abrupt) -> Thrown<'a> {
        match a {
            Abrupt::Throw => {
                let e = self.agent_mut().vm.exception;
                self.agent_mut().vm.exception = Value::UNDEFINED;
                Thrown::Value(self.local_of(e))
            }
            Abrupt::Terminate => Thrown::Terminated,
        }
    }

    fn lift<T>(&mut self, r: Res<T>) -> JsResult<'a, T> {
        r.map_err(|a| self.thrown(a))
    }

    /// The realm made if it was not, as every running operation needs.
    fn realm_ready(&mut self) -> JsResult<'a, ()> {
        let r = self.agent_mut().ensure_realm();
        self.lift(r)
    }

    /// A function running `f`, with `name` and `length`.
    pub fn native_function<F>(&mut self, name: &str, length: u32, f: F) -> JsResult<'a, Local<'a>>
    where
        F: for<'s> Fn(&mut Scope<'s>, Local<'s>, &[Local<'s>]) -> JsResult<'s, Local<'s>> + 'static,
    {
        self.realm_ready()?;
        let agent = self.agent_mut();
        let index = u32::try_from(agent.vm.hosts.len()).unwrap_or(u32::MAX);
        agent.vm.hosts.push(Host {
            name: name.to_owned(),
            f: Rc::new(f),
        });
        let made = agent.native_function(HOST_BASE + index, length, name);
        let v = self.lift(made)?;
        Ok(self.local_of(v))
    }

    /// A new error of `kind` with `message`, to throw.
    pub fn new_error(&mut self, kind: ErrorKind, message: &str) -> JsResult<'a, Local<'a>> {
        self.realm_ready()?;
        let made = self.agent_mut().make_error(kind.to_type(), message);
        let v = self.lift(made)?;
        Ok(self.local_of(v))
    }

    /// `[[Get]]` (GetV for a primitive), a getter called.
    pub fn get_property(&mut self, target: Local<'_>, key: PropertyKey) -> JsResult<'a, Local<'a>> {
        self.realm_ready()?;
        let t = self.value_of(target);
        let r = self.agent_mut().get_value(t, key);
        let v = self.lift(r)?;
        Ok(self.local_of(v))
    }

    /// `[[Set]]` as strict code does it, a setter called.
    pub fn set_property(
        &mut self,
        target: Local<'_>,
        key: PropertyKey,
        value: Local<'_>,
    ) -> JsResult<'a, ()> {
        self.realm_ready()?;
        let (t, v) = (self.value_of(target), self.value_of(value));
        let r = self.agent_mut().set_value(t, key, v, true);
        self.lift(r)
    }

    /// Calls `f` with `this` and `args`.
    pub fn call(
        &mut self,
        f: Local<'_>,
        this: Local<'_>,
        args: &[Local<'_>],
    ) -> JsResult<'a, Local<'a>> {
        self.realm_ready()?;
        let f = self.value_of(f);
        let this = self.value_of(this);
        let args: Vec<Value> = args.iter().map(|&a| self.value_of(a)).collect();
        let r = self.agent_mut().call_value(f, this, &args);
        let v = self.lift(r)?;
        Ok(self.local_of(v))
    }

    /// ToString, as Rust text (lone surrogates replaced).
    pub fn to_text(&mut self, v: Local<'_>) -> JsResult<'a, String> {
        self.realm_ready()?;
        let v = self.value_of(v);
        let r = self.agent_mut().text_units(v);
        let units = self.lift(r)?;
        Ok(String::from_utf16_lossy(&units))
    }

    /// Whether `v` is a function.
    pub fn is_function(&self, v: Local<'_>) -> bool {
        let v = self.value_of(v);
        self.agent_ref().callable(v).is_some()
    }

    /// A new realm of the agent's (ECMA-262 section 9.3): its own global
    /// object and built-ins. The current realm stays current.
    pub fn new_realm(&mut self) -> JsResult<'a, RealmId> {
        self.realm_ready()?;
        let r = self.agent_mut().create_realm();
        self.lift(r).map(RealmId)
    }

    /// The running code's realm (the first made if none was).
    pub fn current_realm(&mut self) -> JsResult<'a, RealmId> {
        self.realm_ready()?;
        Ok(RealmId(self.agent_ref().vm.current))
    }

    /// Runs `f` with `realm` current: the scripts it runs, the objects
    /// and natives it makes, are that realm's. A realm of another agent
    /// (no such index here) leaves the current one current.
    pub fn in_realm<R>(&mut self, realm: RealmId, f: impl FnOnce(&mut Scope<'a>) -> R) -> R {
        let known = (realm.0 as usize) < self.agent_ref().vm.realms.len();
        if !known {
            return f(self);
        }
        let caller = self.agent_mut().enter_realm(realm.0);
        let out = f(self);
        self.agent_mut().vm.current = caller;
        out
    }

    /// Runs `source` as a script in the realm from inside a native (an
    /// embedder's `evalScript`): its completion value, or what it threw.
    pub fn eval_script(&mut self, source: &str) -> JsResult<'a, Local<'a>> {
        self.realm_ready()?;
        let origin = crate::interp::Origin::Script("".into());
        let r = self.agent_mut().eval_script(source, origin);
        let v = self.lift(r)?;
        Ok(self.local_of(v))
    }
}

impl Agent {
    /// The name of the embedder's native `id`, if it is one.
    pub(crate) fn host_name(&self, id: u32) -> Option<&str> {
        let index = id.checked_sub(HOST_BASE)?;
        self.vm.hosts.get(index as usize).map(|h| h.name.as_str())
    }
}

/// Used by the built-ins' dispatch: whether `id` is the embedder's.
pub(crate) fn is_host(id: u32) -> bool {
    id >= HOST_BASE
}
