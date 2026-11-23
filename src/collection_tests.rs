//! The keyed collections and weak references (J4.1): `Map` and `Set`
//! in insertion order with SameValueZero keys, iterators that keep their
//! place across deletion and compaction, the set methods, and the weak
//! collections' entries living exactly as long as their keys.

use crate::agent::{Agent, AgentOptions, Outcome};

fn run_in(agent: &mut Agent, src: &str) -> String {
    agent.scope(|scope| match scope.run_script_named(src, "u.js") {
        Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
        Ok(Outcome::Threw(e)) => format!("throw {}", scope.to_text(e).unwrap_or_default()),
        Ok(Outcome::Terminated) => "terminated".into(),
        Err(e) => format!("compile error: {e}"),
    })
}

fn run(src: &str) -> String {
    run_in(&mut Agent::new(AgentOptions::default()), src)
}

#[test]
fn maps_and_sets_keep_insertion_order_and_same_value_zero_keys() {
    assert_eq!(
        run(
            "var m = new Map([[1, 'a'], [-0, 'z'], ['k', 'v'], [NaN, 'n']]); m.set('k', 'w'); \
             var s = new Set([3, 1, 3, 2]); s.delete(1); s.add(1); \
             var r = new Set(); for (var i = 0; i < 2000; i++) r.add(i); for (var i = 1999; i >= 0; i--) r.delete(i); \
             [r.size,m.size, m.get(-0), m.get('k'), m.get(NaN), Object.is([...m.keys()][1], 0), [...m.values()].join(''), \
              [...s].join(''), s.has(2), m.delete('nope'), String(m), String(s.values())].join()"
        ),
        "0,4,z,w,n,true,azwn,321,true,false,[object Map],[object Set Iterator]"
    );
}

#[test]
fn iteration_is_live_across_deletion_clearing_and_compaction() {
    assert_eq!(
        run("var out = []; var s = new Set([1, 2, 3]); var seen = []; \
             s.forEach(function (v) { seen.push(v); if (v === 1) { s.delete(2); s.add(4); } }); out.push(seen.join('')); \
             var m = new Map([[1, 1], [2, 2], [3, 3]]); var k = m.keys(); k.next(); m.delete(1); m.delete(2); \
             for (var i = 4; i < 40; i++) m.set(i, i); for (var i = 4; i < 39; i++) m.delete(i); \
             out.push(k.next().value, k.next().value, k.next().done); \
             var c = new Map([[1, 1]]); var e = c.entries(); c.clear(); c.set(2, 2); out.push(e.next().value.join('')); \
             var s2 = new Set([1]); var d = s2.values(); d.next(); d.next(); s2.add(9); out.push(d.next().done); \
             var big = new Map(); for (var i = 0; i < 5000; i++) big.set('k' + i, i); var sum = 0; \
             for (var [, v] of big) { sum += v; if (v < 4990) big.delete('k' + (v + 1)); } \
             out.push(sum, big.size); out.join()"),
        "134,3,39,true,22,true,6272475,2505"
    );
}

#[test]
fn the_set_methods_and_group_by() {
    assert_eq!(
        run("function a(s) { return [...s].join(''); } \
             var like = { size: 2, has: function (v) { return v === 2 || v === 9; }, keys: function () { return [2, 9][Symbol.iterator](); } }; \
             var g = Map.groupBy([1, 2, 3, 4, 5], function (x) { return x % 2 ? 'odd' : 'even'; }); \
             [a(new Set([1, 2, 3]).union(new Set([3, 4]))), a(new Set([1, 2, 3]).intersection(new Set([3, 2]))), \
              a(new Set([1, 2, 3]).intersection(like)), a(new Set([1, 2, 3]).difference(new Set([2]))), \
              a(new Set([1, 2]).symmetricDifference(new Set([2, 3]))), new Set([2]).isSubsetOf(like), \
              new Set([1, 2]).isSupersetOf(new Set([1])), new Set([1]).isDisjointFrom(like), \
              [...g.keys()].join(''), g.get('odd').join('')].join()"),
        "1234,32,2,13,13,true,true,true,oddeven,135"
    );
}

#[test]
fn get_or_insert_keeps_what_is_there() {
    assert_eq!(
        run(
            "var m = new Map([[0, 'z']]); var w = new WeakMap(); var k = {}; \
             [m.getOrInsert(-0, 1), m.getOrInsert('a', 1), m.getOrInsert('a', 2), \
              m.getOrInsertComputed('b', function (x) { m.set('b', 'inner'); return x + '!'; }), m.get('b'), m.size, \
              w.getOrInsert(k, 5), w.getOrInsertComputed(k, function () { return 6; }), w.get(k)].join()"
        ),
        "z,1,1,b!,b!,3,5,5,5"
    );
}

#[test]
fn weak_keys_must_be_held_weakly() {
    assert_eq!(
        run(
            "function err(f) { try { return 'no throw ' + f(); } catch (e) { return e.constructor.name; } } \
             var sym = Symbol('s'); var wm = new WeakMap([[sym, 1]]); \
             [wm.get(sym), wm.get(1), wm.has(Symbol.for('x')), err(function () { return new WeakMap().set(1, 1); }), \
              err(function () { return new WeakMap().set(Symbol.for('r'), 1); }), \
              err(function () { return new WeakSet().add('a'); }), err(function () { return new WeakRef(1); }), \
              err(function () { var o = {}; return new FinalizationRegistry(function () {}).register(o, o); }), \
              err(function () { return Map.prototype.get.call(new Set(), 1); }), \
              new FinalizationRegistry(function () {}).unregister({})].join()"
        ),
        "1,,false,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,false"
    );
}

#[test]
fn weak_entries_live_exactly_as_long_as_their_keys() {
    let mut agent = Agent::new(AgentOptions::default());
    run_in(
        &mut agent,
        "var wm = new WeakMap(); var k1 = {}; var k2 = {}; var v = { name: 'v' }; \
         wm.set(k1, k2); wm.set(k2, v); var rv = new WeakRef(v); var rk2 = new WeakRef(k2); \
         var lone = new WeakRef({}); v = null; k2 = null; var log = []; \
         var fr = new FinalizationRegistry(function (h) { log.push(h); }); var token = {}; \
         (function () { fr.register({}, 'a'); fr.register({}, 'b', token); fr.register(k1, 'c'); })(); \
         fr.unregister(token);",
    );
    agent.collect();
    // Through the ephemeron chain k1 -> k2 -> v, all live; the lone
    // target died; 'a' was cleaned up, 'b' unregistered first.
    assert_eq!(
        run_in(
            &mut agent,
            "[rv.deref() && rv.deref().name, rk2.deref() !== undefined, lone.deref()].join()"
        ),
        "v,true,"
    );
    assert_eq!(run_in(&mut agent, "k1 = null; log.join()"), "a");
    agent.collect();
    run_in(&mut agent, "0");
    assert_eq!(
        run_in(&mut agent, "[rv.deref(), rk2.deref(), log.join('')].join()"),
        ",,ac"
    );
}

#[test]
fn collections_survive_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    assert_eq!(
        run_in(
            &mut agent,
            "var m = new Map(); var s = new Set(); var w = new WeakMap(); var keep = []; \
             for (var i = 0; i < 60; i++) { var k = { i: i }; keep.push(k); m.set('k' + i, [i]); s.add('s' + i); w.set(k, { v: i }); \
               if (i % 3 === 0) { m.delete('k' + (i - 1)); s.delete('s' + (i - 2)); } } \
             var t = 0; m.forEach(function (v) { t += v[0]; }); var n = 0; keep.forEach(function (k) { n += w.get(k).v; }); \
             [m.size, s.size, t, n, [...s].slice(0, 3).join('')].join()"
        ),
        "41,41,1219,1770,s0s2s3"
    );
}

/// The collections probe's keys, as the reference browser gave them.
const PROBE_KEYS: &str = r#"function keys(o) {
  return Reflect.ownKeys(o).map(function (k) {
    var d = Object.getOwnPropertyDescriptor(o, k);
    var kind = 'get' in d ? 'accessor' : typeof d.value;
    var flags = (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-');
    var f = typeof d.value === 'function' ? d.value : d.get;
    var extra = typeof f === 'function' ? ' len=' + f.length + ' name=' + f.name : '';
    return String(k) + ' [' + kind + ' ' + flags + extra + ']';
  });
}
var out = { keys: {} };
['Map', 'Set', 'WeakMap', 'WeakSet', 'WeakRef', 'FinalizationRegistry'].forEach(function (n) {
  out.keys[n] = keys(globalThis[n]);
  out.keys[n + '.prototype'] = keys(globalThis[n].prototype);
});
out.keys.MapIteratorPrototype = keys(Object.getPrototypeOf(new Map().keys()));
out.keys.SetIteratorPrototype = keys(Object.getPrototypeOf(new Set().keys()));
Object.keys(out.keys).map(function (k) { return k + ': ' + out.keys[k].join(', '); }).join('\n')"#;

/// The collections probe's errors (but one, J4.4's).
const PROBE_ERRORS: &str = r#"var errors = [];
function err(label, fn) {
  try { var v = fn(); errors.push(label + ': no throw ' + String(v)); }
  catch (e) { errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }
}
err('Map()', function () { Map(); });
err('Set()', function () { Set(); });
err('WeakMap()', function () { WeakMap(); });
err('WeakSet()', function () { WeakSet(); });
err('WeakRef()', function () { WeakRef({}); });
err('FinalizationRegistry()', function () { FinalizationRegistry(function () {}); });
err('new Map(1)', function () { new Map(1); });
err('new Map([1])', function () { new Map([1]); });
err('new Set(1)', function () { new Set(1); });
err('Map set not callable', function () { var M = class extends Map { constructor(i) { super(i); } }; M.prototype.set = 1; new M([[1, 2]]); });
err('Set add not callable', function () { var S = class extends Set {}; S.prototype.add = 1; new S([1]); });
err('Map get receiver', function () { Map.prototype.get.call({}, 1); });
err('Map set receiver', function () { Map.prototype.set.call(new Set(), 1, 2); });
err('Map size receiver', function () { Object.getOwnPropertyDescriptor(Map.prototype, 'size').get.call({}); });
err('Set has receiver', function () { Set.prototype.has.call(new Map(), 1); });
err('Set size receiver', function () { Object.getOwnPropertyDescriptor(Set.prototype, 'size').get.call([]); });
err('Map forEach not callable', function () { new Map().forEach(1); });
err('Set forEach not callable', function () { new Set().forEach({}); });
err('Map iterator next receiver', function () { new Map().keys().next.call({}); });
err('WeakMap set key', function () { new WeakMap().set(1, 1); });
err('WeakMap set registered symbol', function () { new WeakMap().set(Symbol.for('x'), 1); });
err('WeakMap get receiver', function () { WeakMap.prototype.get.call(new Map(), {}); });
err('WeakMap get primitive', function () { return new WeakMap().get(1); });
err('WeakSet add value', function () { new WeakSet().add('a'); });
err('WeakSet has receiver', function () { WeakSet.prototype.has.call({}, {}); });
err('new WeakMap([1])', function () { new WeakMap([1]); });
err('new WeakMap([[1, 2]])', function () { new WeakMap([[1, 2]]); });
err('new WeakRef(1)', function () { new WeakRef(1); });
err('WeakRef deref receiver', function () { WeakRef.prototype.deref.call({}); });
err('new FinalizationRegistry(1)', function () { new FinalizationRegistry(1); });
err('register target', function () { new FinalizationRegistry(function () {}).register(1, 2); });
err('register same', function () { var o = {}; new FinalizationRegistry(function () {}).register(o, o); });
err('register token', function () { new FinalizationRegistry(function () {}).register({}, 1, 2); });
err('unregister token', function () { new FinalizationRegistry(function () {}).unregister(1); });
err('register receiver', function () { FinalizationRegistry.prototype.register.call({}, {}, 1); });
err('union not object', function () { new Set().union(1); });
err('union size NaN', function () { new Set().union({ size: 'x', has: function () {}, keys: function () {} }); });
err('union size negative', function () { new Set().union({ size: -1, has: function () {}, keys: function () {} }); });
err('union has not callable', function () { new Set().union({ size: 1, has: 1, keys: function () {} }); });
err('union keys not callable', function () { new Set().union({ size: 1, has: function () {}, keys: 1 }); });
err('union keys not object', function () { new Set().union({ size: 1, has: function () {}, keys: function () { return 1; } }); });
err('isSubsetOf receiver', function () { Set.prototype.isSubsetOf.call({}, new Set()); });
err('Map.groupBy callback', function () { Map.groupBy([1], 1); });
err('Map.groupBy items', function () { Map.groupBy(1, function () {}); });
errors.join('\n')"#;

#[test]
fn keys_and_errors_read_as_the_reference_browser_gives_them() {
    assert_eq!(
        run(PROBE_KEYS),
        r#"Map: length [number --c], name [string --c], prototype [object ---], groupBy [function w-c len=2 name=groupBy], Symbol(Symbol.species) [accessor --c len=0 name=get [Symbol.species]]
Map.prototype: constructor [function w-c len=0 name=Map], get [function w-c len=1 name=get], set [function w-c len=2 name=set], has [function w-c len=1 name=has], delete [function w-c len=1 name=delete], clear [function w-c len=0 name=clear], entries [function w-c len=0 name=entries], forEach [function w-c len=1 name=forEach], keys [function w-c len=0 name=keys], size [accessor --c len=0 name=get size], values [function w-c len=0 name=values], getOrInsert [function w-c len=2 name=getOrInsert], getOrInsertComputed [function w-c len=2 name=getOrInsertComputed], Symbol(Symbol.toStringTag) [string --c], Symbol(Symbol.iterator) [function w-c len=0 name=entries]
Set: length [number --c], name [string --c], prototype [object ---], Symbol(Symbol.species) [accessor --c len=0 name=get [Symbol.species]]
Set.prototype: constructor [function w-c len=0 name=Set], has [function w-c len=1 name=has], add [function w-c len=1 name=add], delete [function w-c len=1 name=delete], difference [function w-c len=1 name=difference], clear [function w-c len=0 name=clear], entries [function w-c len=0 name=entries], forEach [function w-c len=1 name=forEach], intersection [function w-c len=1 name=intersection], isSubsetOf [function w-c len=1 name=isSubsetOf], isSupersetOf [function w-c len=1 name=isSupersetOf], isDisjointFrom [function w-c len=1 name=isDisjointFrom], size [accessor --c len=0 name=get size], symmetricDifference [function w-c len=1 name=symmetricDifference], union [function w-c len=1 name=union], values [function w-c len=0 name=values], keys [function w-c len=0 name=values], Symbol(Symbol.toStringTag) [string --c], Symbol(Symbol.iterator) [function w-c len=0 name=values]
WeakMap: length [number --c], name [string --c], prototype [object ---]
WeakMap.prototype: constructor [function w-c len=0 name=WeakMap], delete [function w-c len=1 name=delete], get [function w-c len=1 name=get], set [function w-c len=2 name=set], has [function w-c len=1 name=has], getOrInsert [function w-c len=2 name=getOrInsert], getOrInsertComputed [function w-c len=2 name=getOrInsertComputed], Symbol(Symbol.toStringTag) [string --c]
WeakSet: length [number --c], name [string --c], prototype [object ---]
WeakSet.prototype: constructor [function w-c len=0 name=WeakSet], delete [function w-c len=1 name=delete], has [function w-c len=1 name=has], add [function w-c len=1 name=add], Symbol(Symbol.toStringTag) [string --c]
WeakRef: length [number --c], name [string --c], prototype [object ---]
WeakRef.prototype: constructor [function w-c len=1 name=WeakRef], deref [function w-c len=0 name=deref], Symbol(Symbol.toStringTag) [string --c]
FinalizationRegistry: length [number --c], name [string --c], prototype [object ---]
FinalizationRegistry.prototype: constructor [function w-c len=1 name=FinalizationRegistry], register [function w-c len=2 name=register], unregister [function w-c len=1 name=unregister], Symbol(Symbol.toStringTag) [string --c]
MapIteratorPrototype: next [function w-c len=0 name=next], Symbol(Symbol.toStringTag) [string --c]
SetIteratorPrototype: next [function w-c len=0 name=next], Symbol(Symbol.toStringTag) [string --c]"#
    );
    assert_eq!(
        run(PROBE_ERRORS),
        r#"Map(): TypeError: Constructor Map requires 'new'
Set(): TypeError: Constructor Set requires 'new'
WeakMap(): TypeError: Constructor WeakMap requires 'new'
WeakSet(): TypeError: Constructor WeakSet requires 'new'
WeakRef(): TypeError: Constructor WeakRef requires 'new'
FinalizationRegistry(): TypeError: Constructor FinalizationRegistry requires 'new'
new Map(1): TypeError: number 1 is not iterable (cannot read property Symbol(Symbol.iterator))
new Map([1]): TypeError: Iterator value 1 is not an entry object
new Set(1): TypeError: number 1 is not iterable (cannot read property Symbol(Symbol.iterator))
Map set not callable: TypeError: '1' returned for property 'set' of object '#<M>' is not a function
Set add not callable: TypeError: '1' returned for property 'add' of object '#<S>' is not a function
Map get receiver: TypeError: Method Map.prototype.get called on incompatible receiver #<Object>
Map set receiver: TypeError: Method Map.prototype.set called on incompatible receiver #<Set>
Map size receiver: TypeError: Method get Map.prototype.size called on incompatible receiver #<Object>
Set has receiver: TypeError: Method Set.prototype.has called on incompatible receiver #<Map>
Set size receiver: TypeError: Method get Set.prototype.size called on incompatible receiver [object Array]
Map forEach not callable: TypeError: number 1 is not a function
Set forEach not callable: TypeError: object is not a function
Map iterator next receiver: TypeError: Method Map Iterator.prototype.next called on incompatible receiver #<Object>
WeakMap set key: TypeError: Invalid value used as weak map key
WeakMap set registered symbol: TypeError: Invalid value used as weak map key
WeakMap get receiver: TypeError: Method WeakMap.prototype.get called on incompatible receiver #<Map>
WeakMap get primitive: no throw undefined
WeakSet add value: TypeError: Invalid value used in weak set
WeakSet has receiver: TypeError: Method WeakSet.prototype.has called on incompatible receiver #<Object>
new WeakMap([1]): TypeError: Iterator value 1 is not an entry object
new WeakMap([[1, 2]]): TypeError: Invalid value used as weak map key
new WeakRef(1): TypeError: WeakRef: invalid target
WeakRef deref receiver: TypeError: Method WeakRef.prototype.deref called on incompatible receiver #<Object>
new FinalizationRegistry(1): TypeError: FinalizationRegistry: cleanup must be callable
register target: TypeError: FinalizationRegistry.prototype.register: invalid target
register same: TypeError: FinalizationRegistry.prototype.register: target and holdings must not be same
register token: TypeError: Invalid unregisterToken ('2')
unregister token: TypeError: Invalid unregisterToken ('1')
register receiver: TypeError: Method FinalizationRegistry.prototype.register called on incompatible receiver #<Object>
union not object: TypeError: Set.prototype.union argument must be an object
union size NaN: TypeError: The .size property is NaN
union size negative: RangeError: '-1' is an invalid size
union has not callable: TypeError: string "has" is not a function
union keys not callable: TypeError: string "keys" is not a function
union keys not object: TypeError: Result of the keys method is not an object
isSubsetOf receiver: TypeError: Method Set.prototype.isSubsetOf called on incompatible receiver #<Object>
Map.groupBy callback: TypeError: 1 is not a function
Map.groupBy items: TypeError: number 1 is not iterable (cannot read property Symbol(Symbol.iterator))"#
    );
}
