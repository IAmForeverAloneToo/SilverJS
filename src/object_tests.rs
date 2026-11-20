//! Objects' tests: the ordinary internal methods and the Array exotic
//! object's `length`, as ECMA-262 sections 10.1 and 10.4.2 say; strings
//! and symbols as keys and values.

use crate::agent::{
    Agent, AgentOptions, GetResult, Local, Property, PropertyDescriptor, Scope, SetResult,
};
use crate::string::PropertyKey;

fn agent() -> Agent {
    Agent::new(AgentOptions::default())
}

fn value<'a>(scope: &mut Scope<'a>, object: Local<'_>, key: PropertyKey) -> Local<'a> {
    match scope.get(object, key) {
        Some(GetResult::Value(v)) => v,
        _ => panic!("not a data property"),
    }
}

fn number(scope: &mut Scope<'_>, object: Local<'_>, key: PropertyKey) -> Option<f64> {
    let v = value(scope, object, key);
    scope.as_number(v)
}

fn set(scope: &mut Scope<'_>, object: Local<'_>, key: PropertyKey, v: Local<'_>) -> bool {
    matches!(scope.set(object, key, v), Some(SetResult::Done(true)))
}

#[test]
fn data_properties_get_set_and_respect_writable() {
    let mut agent = agent();
    agent.scope(|scope| {
        let o = scope.object(None).unwrap();
        let x = scope.key("x");
        let one = scope.number(1.0);
        assert!(set(scope, o, x, one));
        assert_eq!(number(scope, o, x), Some(1.0));
        let Some(Property::Data {
            writable,
            enumerable,
            configurable,
            ..
        }) = scope.get_own_property(o, x)
        else {
            panic!("a data property");
        };
        assert!(writable && enumerable && configurable);
        // Made non-writable, it refuses assignment and keeps its value.
        assert!(scope.define_property(
            o,
            x,
            PropertyDescriptor {
                writable: Some(false),
                ..PropertyDescriptor::default()
            }
        ));
        let two = scope.number(2.0);
        assert!(!set(scope, o, x, two));
        assert_eq!(number(scope, o, x), Some(1.0));
        // A missing property reads as undefined.
        let y = scope.key("y");
        let missing = value(scope, o, y);
        let undefined = scope.undefined();
        assert!(scope.same(missing, undefined));
    });
}

#[test]
fn accessors_are_handed_back_to_be_called() {
    let mut agent = agent();
    agent.scope(|scope| {
        let o = scope.object(None).unwrap();
        let getter = scope.object(None).unwrap();
        let k = scope.key("k");
        assert!(scope.define_property(
            o,
            k,
            PropertyDescriptor {
                get: Some(getter),
                configurable: Some(true),
                ..PropertyDescriptor::default()
            }
        ));
        let Some(GetResult::Getter(g)) = scope.get(o, k) else {
            panic!("a getter");
        };
        assert!(scope.same(g, getter));
        // No setter: assignment fails.
        let v = scope.number(1.0);
        assert!(matches!(scope.set(o, k, v), Some(SetResult::Done(false))));
        // A setter added later keeps the getter.
        let setter = scope.object(None).unwrap();
        assert!(scope.define_property(
            o,
            k,
            PropertyDescriptor {
                set: Some(setter),
                ..PropertyDescriptor::default()
            }
        ));
        let Some(SetResult::Setter(s)) = scope.set(o, k, v) else {
            panic!("a setter");
        };
        assert!(scope.same(s, setter));
        let Some(GetResult::Getter(g)) = scope.get(o, k) else {
            panic!("the getter kept");
        };
        assert!(scope.same(g, getter));
        // Turned into a data property, it is one.
        let three = scope.number(3.0);
        assert!(scope.define_property(o, k, PropertyDescriptor::data(three)));
        assert_eq!(number(scope, o, k), Some(3.0));
    });
}

#[test]
fn the_prototype_chain_reads_and_assignment_shadows() {
    let mut agent = agent();
    agent.scope(|scope| {
        let proto = scope.object(None).unwrap();
        let child = scope.object(Some(proto)).unwrap();
        let a = scope.key("a");
        let b = scope.key("b");
        let one = scope.number(1.0);
        assert!(set(scope, proto, a, one));
        assert_eq!(number(scope, child, a), Some(1.0));
        assert!(scope.has_property(child, a));
        assert!(scope.get_own_property(child, a).is_none());
        // Assigning through the child makes an own property of the child.
        let two = scope.number(2.0);
        assert!(set(scope, child, a, two));
        assert_eq!(number(scope, child, a), Some(2.0));
        assert_eq!(number(scope, proto, a), Some(1.0));
        // A non-writable property up the chain refuses assignment.
        let frozen = scope.number(5.0);
        assert!(scope.define_property(
            proto,
            b,
            PropertyDescriptor {
                value: Some(frozen),
                ..PropertyDescriptor::default()
            }
        ));
        assert!(!set(scope, child, b, two));
        assert!(scope.get_own_property(child, b).is_none());
    });
}

#[test]
fn non_configurable_properties_hold_their_attributes() {
    let mut agent = agent();
    agent.scope(|scope| {
        let o = scope.object(None).unwrap();
        let k = scope.key("k");
        let nan = scope.number(f64::NAN);
        let fixed = PropertyDescriptor {
            value: Some(nan),
            ..PropertyDescriptor::default()
        };
        assert!(scope.define_property(o, k, fixed));
        // The same value again (NaN is SameValue to itself): allowed.
        let nan2 = scope.number(f64::NAN);
        assert!(scope.define_property(
            o,
            k,
            PropertyDescriptor {
                value: Some(nan2),
                ..PropertyDescriptor::default()
            }
        ));
        // Any change: refused.
        let one = scope.number(1.0);
        for desc in [
            PropertyDescriptor {
                value: Some(one),
                ..PropertyDescriptor::default()
            },
            PropertyDescriptor {
                writable: Some(true),
                ..PropertyDescriptor::default()
            },
            PropertyDescriptor {
                enumerable: Some(true),
                ..PropertyDescriptor::default()
            },
            PropertyDescriptor {
                configurable: Some(true),
                ..PropertyDescriptor::default()
            },
            PropertyDescriptor {
                get: Some(one),
                ..PropertyDescriptor::default()
            },
        ] {
            assert!(!scope.define_property(o, k, desc));
        }
        assert!(!scope.delete(o, k));
        // +0 and -0 are not SameValue.
        let z = scope.key("z");
        let zero = scope.number(0.0);
        assert!(scope.define_property(
            o,
            z,
            PropertyDescriptor {
                value: Some(zero),
                ..PropertyDescriptor::default()
            }
        ));
        let minus = scope.number(-0.0);
        assert!(!scope.define_property(
            o,
            z,
            PropertyDescriptor {
                value: Some(minus),
                ..PropertyDescriptor::default()
            }
        ));
        // A writable non-configurable property may be made read-only, not
        // back.
        let w = scope.key("w");
        assert!(scope.define_property(
            o,
            w,
            PropertyDescriptor {
                value: Some(one),
                writable: Some(true),
                ..PropertyDescriptor::default()
            }
        ));
        assert!(scope.define_property(
            o,
            w,
            PropertyDescriptor {
                writable: Some(false),
                ..PropertyDescriptor::default()
            }
        ));
        assert!(!scope.define_property(
            o,
            w,
            PropertyDescriptor {
                writable: Some(true),
                ..PropertyDescriptor::default()
            }
        ));
    });
}

#[test]
fn keys_come_indices_first_then_names_then_symbols_in_order() {
    let mut agent = agent();
    agent.scope(|scope| {
        let o = scope.object(None).unwrap();
        let v = scope.number(0.0);
        let sym = scope.symbol(Some("s")).unwrap();
        let sym_key = scope.key_of(sym).unwrap();
        for name in ["b", "2", "a", "0"] {
            let k = scope.key(name);
            if name == "a" {
                assert!(set(scope, o, sym_key, v));
            }
            assert!(set(scope, o, k, v));
        }
        let keys = scope.own_keys(o);
        let texts: Vec<Option<String>> = keys.iter().map(|&k| scope.key_text(k)).collect();
        assert_eq!(
            texts,
            vec![
                Some("0".to_owned()),
                Some("2".to_owned()),
                Some("b".to_owned()),
                Some("a".to_owned()),
                None
            ]
        );
        // Deleting keeps the others' order.
        let b = scope.key("b");
        assert!(scope.delete(o, b));
        let a = scope.key("a");
        assert_eq!(
            scope.own_keys(o),
            vec![PropertyKey::Index(0), PropertyKey::Index(2), a, sym_key]
        );
    });
}

#[test]
fn many_properties_are_found_through_the_index() {
    let mut agent = agent();
    agent.scope(|scope| {
        let o = scope.object(None).unwrap();
        for i in 0..100 {
            let k = scope.key(&format!("p{i}"));
            let v = scope.number(f64::from(i));
            assert!(set(scope, o, k, v));
        }
        // Read back before any deletion rebuilds the index.
        for i in 0..100 {
            let k = scope.key(&format!("p{i}"));
            assert_eq!(number(scope, o, k), Some(f64::from(i)), "p{i}");
        }
        for i in (0..100).step_by(3) {
            let k = scope.key(&format!("p{i}"));
            assert!(scope.delete(o, k));
        }
        for i in 0..100 {
            let k = scope.key(&format!("p{i}"));
            let expected = (i % 3 != 0).then_some(f64::from(i));
            let found = match scope.get(o, k) {
                Some(GetResult::Value(v)) => scope.as_number(v),
                _ => None,
            };
            assert_eq!(found, expected, "p{i}");
        }
        assert_eq!(scope.own_keys(o).len(), 66);
    });
}

#[test]
fn array_length_follows_its_elements_and_cuts_them() {
    let mut agent = agent();
    agent.scope(|scope| {
        let a = scope.array(None).unwrap();
        let length = scope.key("length");
        assert_eq!(number(scope, a, length), Some(0.0));
        let v = scope.number(1.0);
        assert!(set(scope, a, PropertyKey::Index(4), v));
        assert_eq!(number(scope, a, length), Some(5.0));
        let Some(Property::Data {
            enumerable,
            configurable,
            ..
        }) = scope.get_own_property(a, length)
        else {
            panic!("length is a data property");
        };
        assert!(!enumerable && !configurable);
        // A shorter length deletes what is past it.
        let two = scope.number(2.0);
        assert!(set(scope, a, length, two));
        assert!(scope.get_own_property(a, PropertyKey::Index(4)).is_none());
        // A non-configurable element stops the cut just past itself.
        assert!(scope.define_property(
            a,
            PropertyKey::Index(1),
            PropertyDescriptor {
                value: Some(v),
                ..PropertyDescriptor::default()
            }
        ));
        let zero = scope.number(0.0);
        assert!(!set(scope, a, length, zero));
        assert_eq!(number(scope, a, length), Some(2.0));
        // Frozen length: no element may grow it.
        let b = scope.array(None).unwrap();
        assert!(scope.define_property(
            b,
            length,
            PropertyDescriptor {
                writable: Some(false),
                ..PropertyDescriptor::default()
            }
        ));
        assert!(!set(scope, b, PropertyKey::Index(0), v));
        // An invalid length is refused, on an array nothing else stops.
        let c = scope.array(None).unwrap();
        assert!(set(scope, c, PropertyKey::Index(3), v));
        let half = scope.number(1.5);
        assert!(!set(scope, c, length, half));
        assert_eq!(number(scope, c, length), Some(4.0));
    });
}

#[test]
fn a_huge_length_is_cut_back_at_once() {
    let mut agent = agent();
    agent.scope(|scope| {
        let a = scope.array(None).unwrap();
        let length = scope.key("length");
        let v = scope.number(7.0);
        assert!(set(scope, a, PropertyKey::Index(3), v));
        let huge = scope.number(4_294_967_295.0);
        assert!(set(scope, a, length, huge));
        let start = std::time::Instant::now();
        let zero = scope.number(0.0);
        assert!(set(scope, a, length, zero));
        assert!(
            start.elapsed().as_secs() < 2,
            "the cut stepped index by index"
        );
        assert!(scope.own_keys(a).iter().all(|k| *k == length));
    });
}

#[test]
fn elements_go_holey_then_sparse_and_keep_their_order() {
    let mut agent = agent();
    agent.scope(|scope| {
        let a = scope.array(None).unwrap();
        let v = scope.number(1.0);
        for i in [0u32, 3, 1_000_000, 2] {
            assert!(set(scope, a, PropertyKey::Index(i), v));
        }
        assert_eq!(
            scope.own_keys(a)[..4],
            [
                PropertyKey::Index(0),
                PropertyKey::Index(2),
                PropertyKey::Index(3),
                PropertyKey::Index(1_000_000)
            ]
        );
        let length = scope.key("length");
        assert_eq!(number(scope, a, length), Some(1_000_001.0));
        // A hole reads through to the prototype, and is no own property.
        assert!(scope.get_own_property(a, PropertyKey::Index(1)).is_none());
        // An element with attributes of its own.
        assert!(scope.define_property(
            a,
            PropertyKey::Index(5),
            PropertyDescriptor {
                value: Some(v),
                enumerable: Some(false),
                writable: Some(true),
                configurable: Some(true),
                ..PropertyDescriptor::default()
            }
        ));
        let Some(Property::Data { enumerable, .. }) =
            scope.get_own_property(a, PropertyKey::Index(5))
        else {
            panic!("element 5");
        };
        assert!(!enumerable);
        assert!(scope.delete(a, PropertyKey::Index(3)));
        assert!(scope.get_own_property(a, PropertyKey::Index(3)).is_none());
    });
}

#[test]
fn extensibility_and_prototype_cycles() {
    let mut agent = agent();
    agent.scope(|scope| {
        let a = scope.object(None).unwrap();
        let b = scope.object(Some(a)).unwrap();
        // b's chain holds a: a may not take b as its prototype.
        assert!(!scope.set_proto(a, Some(b)));
        assert!(scope.set_proto(b, None));
        assert!(scope.set_proto(a, Some(b)));
        assert!(scope.prevent_extensions(a));
        assert!(!scope.is_extensible(a));
        let k = scope.key("k");
        let v = scope.number(1.0);
        assert!(!set(scope, a, k, v));
        assert!(!scope.set_proto(a, None));
        // The same prototype again is no change, and allowed.
        assert!(scope.set_proto(a, Some(b)));
    });
}

#[test]
fn symbol_keys_keep_their_symbols_alive() {
    let mut agent = agent();
    agent.scope(|scope| {
        let o = scope.object(None).unwrap();
        let (key, weak) = scope.nested(|inner| {
            let s = inner.symbol(Some("held")).unwrap();
            let key = inner.key_of(s).unwrap();
            let v = inner.number(9.0);
            assert!(set(inner, o, key, v));
            (key, inner.weak(s))
        });
        scope.collect();
        // Read in a scope of its own: a local here would keep it alive.
        assert!(scope.nested(|inner| inner.get_weak(&weak).is_some()));
        assert_eq!(number(scope, o, key), Some(9.0));
        assert!(scope.delete(o, key));
        scope.collect();
        assert!(scope.get_weak(&weak).is_none());
    });
}

#[test]
fn strings_are_compared_by_their_units() {
    let mut agent = agent();
    agent.scope(|scope| {
        let a = scope.string("ab").unwrap();
        let b = scope.string("a").unwrap();
        let c = scope.string("b").unwrap();
        let ab = scope.concat(b, c).unwrap().unwrap();
        assert!(!scope.same(a, ab));
        assert!(scope.same_value(a, ab));
        let wide = scope.string("\u{3b1}").unwrap();
        let mixed = scope.concat(a, wide).unwrap().unwrap();
        assert_eq!(scope.string_text(mixed).as_deref(), Some("ab\u{3b1}"));
        let lone = scope.string_from_units(&[0xD800]).unwrap();
        assert!(scope.string_text(lone).is_some());
        // Keys from equal strings are one key.
        let k1 = scope.key_of(a).unwrap();
        let k2 = scope.key_of(ab).unwrap();
        assert_eq!(k1, k2);
        let n = scope.string("12").unwrap();
        assert_eq!(scope.key_of(n), Some(PropertyKey::Index(12)));
    });
}

#[test]
fn a_plain_object_of_two_properties_is_small() {
    // QuickJS-ng takes about 121 bytes for `{a, b}` (the micro-bench's
    // measure); this holds SilverJS's own count under 128.
    let mut agent = agent();
    agent.scope(|scope| {
        let holder = scope.array(None).unwrap();
        let a = scope.key("a");
        let b = scope.key("b");
        let before = scope.heap_stats().bytes;
        const N: u32 = 10_000;
        for i in 0..N {
            let o = scope.object(None).unwrap();
            let v = scope.number(f64::from(i));
            assert!(set(scope, o, a, v));
            assert!(set(scope, o, b, v));
            assert!(set(scope, holder, PropertyKey::Index(i), o));
        }
        let per = (scope.heap_stats().bytes - before) as f64 / f64::from(N);
        assert!(per < 128.0, "{per} bytes per object");
    });
}
