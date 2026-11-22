//! Any bytes, run as a script twice: once as usual and once collecting
//! every few allocations, the first byte choosing how few (every one,
//! every 2nd, 8th or 64th: most runs fast, a quarter exhaustive), the
//! rest the script. A collection is never observable to script, so the
//! two runs must come to the same text; a value the collector freed
//! while it was still in use shows as a difference, or as a stale
//! handle's panic. The script may make realms (`createRealm()`) and
//! collect (`gc()`).

#![no_main]

use std::cell::Cell;
use std::rc::Rc;

use libfuzzer_sys::fuzz_target;
use silverjs::agent::{Agent, AgentOptions, Outcome, PropertyDescriptor, Scope};
use silverjs::{JsResult, Thrown};

/// The interrupt hook's calls a run may take (each 10,000 steps): a
/// collection at every allocation is slow.
const BUDGET: u32 = 8;

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

/// The strides the first byte chooses from.
const STRIDES: [u32; 4] = [1, 2, 8, 64];

/// What the script came to, as text: its value or thrown value made a
/// string (which may run script), or how it ended otherwise; collecting
/// every `stride`-th allocation (0: only as the heap grows).
fn run(source: &str, stride: u32) -> String {
    let mut agent = Agent::new(AgentOptions {
        heap_limit: 16 << 20,
    });
    agent.set_stress_every(stride);
    agent.set_random_seed(0x5eed);
    let calls = Rc::new(Cell::new(0u32));
    let hook = Rc::clone(&calls);
    agent.set_interrupt(Some(Box::new(move || {
        hook.set(hook.get() + 1);
        hook.get() > BUDGET
    })));
    agent.scope(|scope| {
        if install(scope).is_err() {
            return "no host".to_owned();
        }
        let text = |scope: &mut Scope<'_>, v| match scope.to_text(v) {
            Ok(t) => t,
            Err(Thrown::Value(_)) => "(ToString threw)".to_owned(),
            Err(Thrown::Terminated) => "(ToString terminated)".to_owned(),
        };
        match scope.run_script(source) {
            Ok(Outcome::Value(v)) => format!("value {}", text(scope, v)),
            Ok(Outcome::Threw(v)) => format!("threw {}", text(scope, v)),
            Ok(Outcome::Terminated) => "terminated".to_owned(),
            Err(e) => format!("did not compile: {e}"),
        }
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&first, rest)) = data.split_first() else {
        return;
    };
    let stride = STRIDES[usize::from(first) % STRIDES.len()];
    let source = String::from_utf8_lossy(rest);
    let plain = run(&source, 0);
    let stressed = run(&source, stride);
    assert_eq!(plain, stressed, "a collection changed the outcome");
});
