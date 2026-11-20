//! The heap's and the handles' tests: a collection keeps exactly what
//! is reachable from the roots, long-lived handles check their cell and
//! their agent, and the heap limit holds.

use crate::agent::{Agent, AgentOptions, GetResult, Local, OutOfMemory, Scope, SetResult};
use crate::string::PropertyKey;

/// Puts `value` at index `i` of `object`.
fn put(scope: &mut Scope<'_>, object: Local<'_>, i: u32, value: Local<'_>) {
    assert!(matches!(
        scope.set(object, PropertyKey::Index(i), value),
        Some(SetResult::Done(true))
    ));
}

/// The value at index `i` of `object`.
fn at<'a>(scope: &mut Scope<'a>, object: Local<'_>, i: u32) -> Local<'a> {
    match scope.get(object, PropertyKey::Index(i)) {
        Some(GetResult::Value(v)) => v,
        _ => panic!("no element {i}"),
    }
}

fn agent() -> Agent {
    Agent::new(AgentOptions::default())
}

#[test]
fn locals_keep_their_values_and_the_rest_is_collected() {
    let mut agent = agent();
    agent.scope(|scope| {
        let kept = scope.object(None).unwrap();
        let text = scope.string("kept").unwrap();
        put(scope, kept, 0, text);
        let weak_kept = scope.weak(kept);
        let dropped = scope.nested(|inner| {
            let garbage = inner.object(None).unwrap();
            inner.weak(garbage)
        });
        scope.collect();
        assert!(scope.get_weak(&dropped).is_none());
        let again = scope.get_weak(&weak_kept).unwrap();
        assert!(scope.same(again, kept));
        let slot = at(scope, kept, 0);
        assert_eq!(scope.string_text(slot).as_deref(), Some("kept"));
        scope.release_weak(dropped);
        scope.release_weak(weak_kept);
    });
}

#[test]
fn cycles_die_together() {
    let mut agent = agent();
    agent.scope(|scope| {
        let (wa, wb) = scope.nested(|inner| {
            let a = inner.object(None).unwrap();
            let b = inner.object(Some(a)).unwrap();
            put(inner, a, 0, b);
            (inner.weak(a), inner.weak(b))
        });
        scope.collect();
        assert!(scope.get_weak(&wa).is_none());
        assert!(scope.get_weak(&wb).is_none());
    });
}

#[test]
fn a_long_chain_is_marked_without_recursion() {
    let mut agent = agent();
    agent.scope(|scope| {
        let head = scope.object(None).unwrap();
        let tail_weak = scope.nested(|inner| {
            let mut last = head;
            // Under gc-stress every link collects: a shorter chain.
            let links = if cfg!(feature = "gc-stress") {
                2_000
            } else {
                200_000
            };
            for _ in 0..links {
                let next = inner.object(None).unwrap();
                put(inner, last, 0, next);
                last = next;
            }
            inner.weak(last)
        });
        scope.collect();
        // The tail is reached through every link from the head.
        assert!(scope.get_weak(&tail_weak).is_some());
    });
}

#[test]
fn globals_outlive_scopes_until_released() {
    let mut agent = agent();
    let global = agent.scope(|scope| {
        let o = scope.object(None).unwrap();
        let n = scope.number(42.5);
        put(scope, o, 0, n);
        scope.global(o)
    });
    agent.collect();
    let weak = agent.scope(|scope| {
        let o = scope.get_global(&global);
        let n = at(scope, o, 0);
        assert_eq!(scope.as_number(n), Some(42.5));
        scope.weak(o)
    });
    agent.scope(|scope| scope.release(global));
    agent.collect();
    agent.scope(|scope| assert!(scope.get_weak(&weak).is_none()));
}

#[test]
fn a_reused_cell_is_not_the_one_a_weak_handle_named() {
    let mut agent = agent();
    agent.scope(|scope| {
        let weak = scope.nested(|inner| {
            let o = inner.object(None).unwrap();
            inner.weak(o)
        });
        scope.collect();
        // The freed slot is reused by the next object made.
        let fresh = scope.object(None).unwrap();
        let _ = fresh;
        assert!(scope.get_weak(&weak).is_none());
    });
}

#[test]
fn a_cell_made_after_two_collections_lives() {
    // The first collection leaves garbage unswept; the second sweeps it
    // onto the free list before marking; the next cell comes from that
    // list, at an index the new sweep has not reached.
    let mut agent = agent();
    agent.scope(|scope| {
        scope.nested(|inner| {
            for _ in 0..10 {
                inner.object(None).unwrap();
            }
        });
        scope.collect();
        scope.collect();
        let o = scope.object(None).unwrap();
        let n = scope.number(7.0);
        put(scope, o, 0, n);
        let back = at(scope, o, 0);
        assert_eq!(scope.as_number(back), Some(7.0));
    });
}

#[test]
#[should_panic(expected = "used in agent")]
fn a_handle_of_another_agent_panics() {
    let mut first = agent();
    let mut second = agent();
    let global = first.scope(|scope| {
        let o = scope.object(None).unwrap();
        scope.global(o)
    });
    second.scope(|scope| {
        scope.get_global(&global);
    });
}

#[test]
#[should_panic(expected = "released handle")]
fn a_released_handle_panics() {
    let mut agent = agent();
    agent.scope(|scope| {
        let o = scope.object(None).unwrap();
        let global = scope.global(o);
        let copy = crate::agent::Global::clone_for_test(&global);
        scope.release(global);
        scope.get_global(&copy);
    });
}

#[test]
fn the_heap_limit_holds_and_garbage_makes_room() {
    let mut agent = Agent::new(AgentOptions {
        heap_limit: 2 << 20,
    });
    agent.scope(|scope| {
        // Garbage only: collections keep making room.
        for _ in 0..2000 {
            scope.nested(|inner| {
                inner.string(&"x".repeat(4096)).unwrap();
            });
        }
        // All of it kept: the limit is reached.
        let holder = scope.object(None).unwrap();
        let mut result = Ok(());
        for i in 0..2000 {
            match scope.string(&"x".repeat(4096)) {
                Ok(s) => {
                    put(scope, holder, i, s);
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        assert_eq!(result, Err(OutOfMemory));
        assert!(scope.heap_stats().bytes <= 2 << 20);
    });
}

#[test]
fn under_stress_every_allocation_collects_and_nothing_rooted_is_lost() {
    let mut agent = agent();
    agent.set_stress(true);
    agent.scope(|scope| {
        let root = scope.object(None).unwrap();
        for i in 0..300 {
            let s = scope.string(&format!("s{i}")).unwrap();
            let o = scope.object(Some(root)).unwrap();
            put(scope, o, 0, s);
            put(scope, root, i, o);
        }
        for i in 0..300 {
            let o = at(scope, root, i);
            let s = at(scope, o, 0);
            assert_eq!(scope.string_text(s), Some(format!("s{i}")));
            let proto = scope.proto(o).unwrap();
            assert!(scope.same(proto, root));
        }
        assert!(scope.heap_stats().collections >= 600);
    });
}

#[test]
fn escape_moves_one_local_out() {
    let mut agent = agent();
    agent.scope(|scope| {
        let kept = scope.escape(|inner| {
            let _garbage = inner.object(None).unwrap();
            inner.string("out").unwrap()
        });
        scope.collect();
        assert_eq!(scope.string_text(kept).as_deref(), Some("out"));
    });
}
