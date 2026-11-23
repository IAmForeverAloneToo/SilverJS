//! Any bytes, run as a script: the engine compiles it or gives back a
//! SyntaxError, and runs it to a value, a throw or the end of its step
//! budget, never panicking and never overflowing the stack. The script
//! may make realms (`createRealm()`, a new realm's global object, which
//! has the same two) and collect (`gc()`) where it likes.

#![no_main]

use std::cell::Cell;
use std::rc::Rc;

use libfuzzer_sys::fuzz_target;
use silverjs::agent::{Agent, AgentOptions, Outcome, PropertyDescriptor, Scope};
use silverjs::{JsResult, Thrown};

/// The interrupt hook's calls a run may take (each 10,000 steps).
const BUDGET: u32 = 200;
/// The heap's limit, and what may be past it after a collection.
const HEAP_LIMIT: usize = 64 << 20;
const SLACK: usize = 64 << 10;

/// `createRealm` and `gc` on the current realm's global object.
fn install<'s>(scope: &mut Scope<'s>) -> JsResult<'s, ()> {
    let Ok(global) = scope.global_object() else {
        return Err(Thrown::Terminated);
    };
    let create = scope.native_function("createRealm", 0, |scope, _, _| {
        let realm = scope.new_realm()?;
        scope.in_realm(realm, |scope| {
            install(scope)?;
            scope.global_object().map_err(|_| Thrown::Terminated)
        })
    })?;
    let gc = scope.native_function("gc", 0, |scope, _, _| {
        scope.collect();
        Ok(scope.undefined())
    })?;
    for (name, f) in [("createRealm", create), ("gc", gc)] {
        let key = scope.key(name);
        scope.define_property(global, key, PropertyDescriptor::data(f));
    }
    Ok(())
}

fuzz_target!(|data: &[u8]| {
    let source = String::from_utf8_lossy(data);
    let mut agent = Agent::new(AgentOptions {
        heap_limit: HEAP_LIMIT,
        ..AgentOptions::default()
    });
    let calls = Rc::new(Cell::new(0u32));
    let hook = Rc::clone(&calls);
    agent.set_interrupt(Some(Box::new(move || {
        hook.set(hook.get() + 1);
        hook.get() > BUDGET
    })));
    agent.scope(|scope| {
        if install(scope).is_err() {
            return;
        }
        // The value or the thrown value made text, which may run script
        // too (a `toString`, a `stack` formatted).
        match scope.run_script(&source) {
            Ok(Outcome::Value(v) | Outcome::Threw(v)) => {
                let _ = scope.to_text(v);
            }
            Ok(Outcome::Terminated) | Err(_) => {}
        }
        // The heap stays under its limit (but for the small error that
        // reports reaching it).
        scope.collect();
        let used = scope.heap_stats().bytes;
        assert!(used <= HEAP_LIMIT + SLACK, "{used} bytes in use");
    });
});
