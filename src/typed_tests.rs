//! Binary data (J4.2a): `ArrayBuffer`, resizable and transferable, and
//! the typed arrays over it, their elements converted as the spec has
//! them, read and written through the integer-indexed exotic object, and
//! out of bounds once their buffer shrinks or is detached.

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
fn views_share_their_buffer_and_convert_what_is_written() {
    assert_eq!(
        run(
            "var b = new ArrayBuffer(8); var u8 = new Uint8Array(b); var i16 = new Int16Array(b, 2, 2); \
             var f32 = new Float32Array(b, 4); u8[2] = 0xff; u8[3] = 0x7f; i16[1] = -2; f32[0] = 1.5;              u8[0] = { valueOf: function () { return 258; } }; \
             [i16[0], u8[4], u8[5], u8[6], u8[7], i16[1], Object.is(Float64Array.of(-0)[0], -0), \
              isNaN(new Float64Array([NaN])[0]), new Uint8Array([' 7 ', null, true, '0x10']), u8[0],               Float16Array.of(1e5, -65520, 65519)].join()"
        ),
        "32767,0,0,192,63,0,true,true,7,0,1,16,2,Infinity,-Infinity,65504"
    );
}

#[test]
fn typed_arrays_iterate_and_list_their_indices() {
    assert_eq!(
        run("var ks = []; for (var k in new Int8Array(2)) ks.push(k); \
             [[...new Int8Array([1, 2, 3])].join(), Array.from(new Uint16Array([1, 2]).entries()).join(';'), \
              ks.join(), Object.getOwnPropertyNames(new Int8Array(2)).join(), \
              Array.prototype.slice.call(new Int8Array([4, 5])).join()].join('|')"),
        "1,2,3|0,1;1,2|0,1|0,1|4,5"
    );
}

#[test]
fn resizing_and_transfer_move_the_bytes() {
    assert_eq!(
        run(
            "var b = new ArrayBuffer(4, { maxByteLength: 8 }); var a = new Uint8Array(b); a.set([1, 2, 3, 4]); \
             b.resize(6); var n = a.length; var c = b.transfer(); var d = new Uint8Array(c); \
             [n, a.length, b.byteLength, c.byteLength, c.resizable, c.maxByteLength, d.join(), \
              new Uint8Array(c.transferToFixedLength(2)).join(), c.detached].join() +              (function () { var e = new ArrayBuffer(8, { maxByteLength: 16 }); var t = new Uint8Array(e);                e.transfer(); try { t.fill(0); return ' filled'; } catch (x) { return ' ' + x.name; } })()"
        ),
        "6,0,0,6,true,8,1,2,3,4,0,0,1,2,true TypeError"
    );
}

#[test]
fn buffers_survive_collection_and_count_against_the_heap() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    assert_eq!(
        run_in(
            &mut agent,
            "var keep = []; for (var i = 0; i < 50; i++) { var a = new Float64Array(16); a[i % 16] = i; keep.push(a.subarray(1)); } \
             var t = 0; keep.forEach(function (s) { t += s.reduce(function (x, y) { return x + y; }, 0); }); t"
        ),
        "1129"
    );
    // A buffer past the heap's limit is refused as the reference browser
    // refuses one it cannot allocate.
    assert_eq!(
        run("try { new ArrayBuffer(2 ** 30); } catch (e) { e.name + ': ' + e.message }"),
        "RangeError: Array buffer allocation failed"
    );
}

#[test]
fn buffers_changed_inside_conversions_and_callbacks_give_the_spec_outcomes() {
    // Each line: a buffer detached, shrunk or grown from inside a value's
    // conversion or a callback, while a view's method is under way.
    assert_eq!(
        run(
            "function out(f) { try { var r = f(); return String(r); } catch (e) { return e.name; } } \
             var r = []; \
             var b = new ArrayBuffer(4); var a = new Uint8Array(b); \
             r.push(out(function () { return a.fill({ valueOf: function () { b.transfer(); return 1; } }); })); \
             var b2 = new ArrayBuffer(4); var a2 = new Uint8Array(b2); \
             a2[0] = { valueOf: function () { b2.transfer(); return 9; } }; r.push(a2[0], a2.length); \
             var b3 = new ArrayBuffer(4, { maxByteLength: 8 }); var a3 = new Uint8Array(b3); \
             r.push(out(function () { return a3.set({ length: 3, get 0() { b3.resize(1); return 5; }, 1: 6, 2: 7 }, 0); }), a3.join()); \
             var b4 = new ArrayBuffer(4); var a4 = new Uint8Array(b4); \
             r.push(out(function () { return a4.slice({ valueOf: function () { b4.transfer(); return 0; } }); })); \
             var b5 = new ArrayBuffer(4, { maxByteLength: 8 }); var a5 = new Uint8Array(b5); \
             r.push(a5.includes(undefined, { valueOf: function () { b5.resize(1); return 0; } })); \
             var b6 = new ArrayBuffer(4, { maxByteLength: 8 }); var a6 = new Uint8Array(b6); a6.set([4, 3, 2, 1]); \
             r.push(out(function () { return a6.sort(function (x, y) { b6.resize(2); return x - y; }).join(); })); \
             var b7 = new ArrayBuffer(4, { maxByteLength: 8 }); var a7 = new Uint8Array(b7); a7.set([1, 2, 3, 4]); \
             r.push(out(function () { return a7.copyWithin({ valueOf: function () { b7.resize(3); return 0; } }, 1).join(); })); \
             var b8 = new ArrayBuffer(4); var a8 = new Uint8Array(b8); var seen = []; \
             r.push(out(function () { for (var x of a8) { seen.push(x); b8.transfer(); } }), seen.length); \
             var b9 = new ArrayBuffer(4, { maxByteLength: 8 }); var a9 = new Uint8Array(b9, 0, 4); \
             r.push(out(function () { return a9.map(function (x, i) { if (i === 1) b9.resize(2); return 7; }).join(); })); \
             r.join('|')"
        ),
        "TypeError||0|undefined|5|TypeError|true|1,2|2,3,3|TypeError|1|7,7,7,7"
    );
}

#[test]
fn numeric_keys_are_the_elements_and_no_others() {
    assert_eq!(
        run(
            "var t = new Int8Array(1); t['-0'] = 5; t['1.5'] = 1; t['01'] = 2; t[0] = { valueOf: function () { return 7; } }; \
             var b = new ArrayBuffer(4); var v = new Uint8Array(b, 0, 2); v[2] = 9; \
             Object.prototype[7] = 'p'; var past = t[7]; delete Object.prototype[7]; \
             var p = Object.create(t); p[10] = 1; \
             Object.defineProperty(t, '0', { value: { valueOf: function () { return 3; } } }); \
             var r = new ArrayBuffer(1, { maxByteLength: 2 }); var pe; \
             try { Object.preventExtensions(new Uint8Array(r)); pe = 'none'; } catch (e) { pe = e.name; } \
             var f = new Float32Array([1.5]); f.constructor = {}; f.constructor[Symbol.species] = Int8Array; \
             [Object.keys(t).join(), t[0], new Uint8Array(b)[2], past, Object.keys(p).length, \
              Reflect.defineProperty(t, '0', { value: 1, writable: false }), pe, \
              new Int8Array([1, 2, 3, 4, 5]).copyWithin(1, 0).join(''), f.slice()[0]].join()"
        ),
        "0,01,3,0,,0,false,TypeError,11234,1"
    );
}

/// The binary data probe's keys, as the reference browser gave them
/// (J4.2a's groups).
const PROBE_KEYS: &str = r#"function keys(o) {
  return Reflect.ownKeys(o).map(function (k) {
    var d = Object.getOwnPropertyDescriptor(o, k);
    var kind = 'get' in d ? 'accessor' : typeof d.value;
    var flags = (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-');
    var f = typeof d.value === 'function' ? d.value : d.get;
    var extra = typeof f === 'function' ? ' len=' + f.length + ' name=' + f.name : '';
    if (d.set) extra += ' set=' + d.set.name;
    return String(k) + ' [' + kind + ' ' + flags + extra + ']';
  });
}
var TA = Object.getPrototypeOf(Int8Array);
['ArrayBuffer: ' + keys(ArrayBuffer).join(', '),
'ArrayBuffer.prototype: ' + keys(ArrayBuffer.prototype).join(', '),
'TypedArray: ' + keys(TA).join(', '),
'TypedArray.prototype: ' + keys(TA.prototype).join(', '),
'Int8Array: ' + keys(Int8Array).join(', '),
'Int8Array.prototype: ' + keys(Int8Array.prototype).join(', '),
'Float16Array: ' + keys(Float16Array).join(', '),
'instance: ' + keys(new Int8Array([5, 6])).join(', ')].join('\n')"#;

/// The probe's behaviours (J4.2a's).
const PROBE_FACTS: &str = r#"if (typeof JSON === 'undefined') var JSON = { stringify: function (o) { return '{' + Object.keys(o).filter(function (k) { return o[k] !== undefined; }).map(function (k) { return '"' + k + '":' + String(o[k]); }).join(',') + '}'; } };
var TA = Object.getPrototypeOf(Int8Array);
var f = [];
function show(v) {
  if (typeof v === 'number' && v === 0 && 1 / v < 0) return '-0';
  return String(v);
}
function list(ta) { return Array.prototype.map.call(ta, show).join(); }
f.push('name ' + TA.name + ' ' + TA.length, String(new Uint8Array([1, 2])),
  Object.prototype.toString.call(new Int8Array(1)),
  Object.prototype.toString.call(new ArrayBuffer(1)),
  TA.prototype[Symbol.iterator] === TA.prototype.values,
  TA.prototype.toString === Array.prototype.toString,
  Object.getOwnPropertyDescriptor(TA.prototype, Symbol.toStringTag).get.call(3));
f.push('clamped ' + list(new Uint8ClampedArray([1.5, 2.5, -1, 300, NaN, 0.5, 254.5, Infinity])));
f.push('int8 ' + list(new Int8Array([200, -129, 1.9, -1.9, NaN, Infinity, -0])));
f.push('uint32 ' + list(new Uint32Array([-1, 4294967296.5, 2.5])));
f.push('f16 ' + list(new Float16Array([65520, 65519, 1 / 3, 1e-8, -0, 5.960464477539063e-8, 2.9802322387695312e-8, 2.98023223876953125e-8 * 1.0000001])));
f.push('f32 ' + list(new Float32Array([1 / 3, 3.4028235677973366e38, 1e-46])));
var ta = new Int8Array([1, 2, 3]);
f.push('keys ' + Object.keys(ta).join(), JSON.stringify(ta), ta['-0'], ta['1.5'], '1.5' in ta, '-0' in ta, '2' in ta, '3' in ta,
  ta[-1], Reflect.set(ta, '5', 1), Reflect.set(ta, '1.5', 1), Reflect.defineProperty(ta, '5', { value: 1 }),
  Reflect.defineProperty(ta, '0', { value: 9 }), ta[0], delete ta[0], delete ta[5], Reflect.deleteProperty(ta, '0'));
var proto = Object.create(ta); proto[1] = 7; f.push('proto set ' + ta[1] + ' ' + Object.keys(proto).join());
ta.foo = 1; f.push('expando ' + Object.keys(ta).join() + ' ' + Reflect.ownKeys(ta).join());
var calls = []; var vo = { valueOf: function () { calls.push('v'); return 4; } };
ta['1.5'] = vo; ta[10] = vo; f.push('valueOf ' + calls.join());
var rab = new ArrayBuffer(4, { maxByteLength: 16 });
var track = new Uint8Array(rab); var fixed = new Uint8Array(rab, 0, 4); var off = new Uint8Array(rab, 2);
f.push('rab ' + rab.resizable + ' ' + rab.maxByteLength + ' ' + track.length + ' ' + fixed.length);
rab.resize(8); f.push('grown ' + track.length + ' ' + fixed.length + ' ' + off.length + ' ' + off.byteLength);
rab.resize(1); f.push('shrunk ' + track.length + ' ' + fixed.length + ' ' + fixed.byteLength + ' ' + fixed.byteOffset + ' ' + off.length + ' ' + off.byteOffset + ' ' + Object.keys(fixed).length);
var buf = new ArrayBuffer(8); var v8 = new Uint8Array(buf); var t2 = buf.transfer(4);
f.push('detached ' + buf.detached + ' ' + buf.byteLength + ' ' + v8.length + ' ' + v8.byteOffset + ' ' + v8[0] + ' ' + t2.byteLength + ' ' + t2.resizable);
f.push('slice ' + new ArrayBuffer(8).slice(-3).byteLength + ' ' + new ArrayBuffer(8, { maxByteLength: 9 }).transfer().resizable + ' ' + new ArrayBuffer(8, { maxByteLength: 9 }).transferToFixedLength().resizable);
f.push('sort ' + list(new Float64Array([3, NaN, -0, 0, -Infinity, 1]).sort()));
f.push('toLocale ' + new Float64Array([1234.5, 2]).toLocaleString() + ' ' + new Int8Array([1, 2]).join('-'));
f.push('subarray ' + list(new Int8Array([1, 2, 3, 4]).subarray(1, -1)) + ' ' + new Int8Array(new ArrayBuffer(8), 2).subarray(1).byteOffset);
f.push('from ' + list(Int16Array.from({ length: 2, 0: '7' })) + ' ' + list(Int16Array.of(1, '2', 3)));
f.push('map ' + list(new Int8Array([1, 2]).map(function (x) { return x * 200; })));
f.push('set ' + (function () { var a = new Int8Array(4); a.set([1, 2], 1); a.set(new Uint8Array([255]), 3); return list(a); })());
f.push('overlap ' + (function () { var a = new Uint8Array([1, 2, 3, 4, 5]); a.set(a.subarray(0, 3), 2); return list(a); })());
f.push('copyWithin ' + list(new Int8Array([1, 2, 3, 4, 5]).copyWithin(0, 3)));
f.push('with ' + list(new Int8Array([1, 2]).with(-1, 9)) + ' ' + list(new Int8Array([3, 1, 2]).toSorted()) + ' ' + list(new Int8Array([1, 2]).toReversed()));
f.push('includes ' + new Float32Array([NaN]).includes(NaN) + ' ' + new Float32Array([NaN]).indexOf(NaN) + ' ' + new Int8Array(2).includes(undefined));
f.push('species ' + (function () { var C = function () {}; var a = new Int8Array(2); a.constructor = {}; a.constructor[Symbol.species] = Uint8Array; return Object.prototype.toString.call(a.slice()); })());
f.push('iter ' + (function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); var a = new Uint8Array(b); var it = a.values(); it.next(); b.resize(1); return JSON.stringify(it.next()); })());
f.push('freeze ' + Object.isFrozen(Object.freeze(new Int8Array(0))) + ' ' + Object.isSealed(new Int8Array(1)));
f.map(function (v) { return String(v); }).join('\n')"#;

/// The probe's errors (J4.2a's).
const PROBE_ERRORS: &str = r#"var TA = Object.getPrototypeOf(Int8Array);
var errors = [];
function err(label, fn) {
  try { var v = fn(); errors.push(label + ': no throw ' + String(v)); }
  catch (e) { errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }
}
var det = new ArrayBuffer(8); var detv = new Uint8Array(det); det.transfer();
err('ArrayBuffer()', function () { ArrayBuffer(1); });
err('new ArrayBuffer(-1)', function () { new ArrayBuffer(-1); });
err('new ArrayBuffer(2**53)', function () { new ArrayBuffer(2 ** 53); });
err('new ArrayBuffer(1e12)', function () { new ArrayBuffer(1e12); });
err('maxByteLength < length', function () { new ArrayBuffer(8, { maxByteLength: 4 }); });
err('maxByteLength huge', function () { new ArrayBuffer(8, { maxByteLength: 2 ** 53 }); });
err('resize fixed', function () { new ArrayBuffer(8).resize(4); });
err('resize past max', function () { new ArrayBuffer(8, { maxByteLength: 9 }).resize(10); });
err('resize detached', function () { var b = new ArrayBuffer(8, { maxByteLength: 9 }); b.transfer(); b.resize(1); });
err('byteLength receiver', function () { Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, 'byteLength').get.call({}); });
err('slice receiver', function () { ArrayBuffer.prototype.slice.call(new Int8Array(1)); });
err('slice detached', function () { det.slice(); });
err('transfer detached', function () { det.transfer(); });
err('slice species not buffer', function () { var b = new ArrayBuffer(8); b.constructor = {}; b.constructor[Symbol.species] = function () { return {}; }; b.slice(); });
err('slice species same', function () { var b = new ArrayBuffer(8); b.constructor = {}; b.constructor[Symbol.species] = function () { return b; }; b.slice(); });
err('slice species small', function () { var b = new ArrayBuffer(8); b.constructor = {}; b.constructor[Symbol.species] = function () { return new ArrayBuffer(1); }; b.slice(); });
err('Int8Array()', function () { Int8Array(1); });
err('new TypedArray()', function () { new TA(); });
err('TypedArray()', function () { TA(); });
err('new Int8Array(-1)', function () { new Int8Array(-1); });
err('new Int8Array(2**53)', function () { new Int8Array(2 ** 53); });
err('new Float64Array(2**31)', function () { new Float64Array(2 ** 31); });
err('Int16 odd offset', function () { new Int16Array(new ArrayBuffer(4), 1); });
err('Int16 odd length', function () { new Int16Array(new ArrayBuffer(3)); });
err('offset past end', function () { new Int8Array(new ArrayBuffer(4), 8); });
err('length past end', function () { new Int8Array(new ArrayBuffer(4), 1, 4); });
err('Int32 offset past', function () { new Int32Array(new ArrayBuffer(8), 4, 2); });
err('negative offset', function () { new Int8Array(new ArrayBuffer(4), -1); });
err('detached buffer ctor', function () { new Int8Array(det); });
err('detached ta ctor', function () { new Int8Array(detv); });
err('fill receiver', function () { Int8Array.prototype.fill.call([]); });
err('length receiver', function () { Object.getOwnPropertyDescriptor(TA.prototype, 'length').get.call([]); });
err('buffer receiver', function () { Object.getOwnPropertyDescriptor(TA.prototype, 'buffer').get.call({}); });
err('fill detached', function () { detv.fill(0); });
err('map detached', function () { detv.map(function () {}); });
err('values detached', function () { detv.values(); });
err('next detached', function () { var b = new ArrayBuffer(4); var it = new Uint8Array(b).keys(); b.transfer(); it.next(); });
err('join out of bounds', function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); var a = new Uint8Array(b, 2, 2); b.resize(1); a.join(); });
err('set offset', function () { new Int8Array(2).set([1, 2, 3], 1); });
err('set negative', function () { new Int8Array(2).set([1], -1); });
err('set detached source', function () { new Int8Array(2).set(detv); });
err('from not ctor', function () { Int8Array.from.call(Object, []); });
err('from mapper', function () { Int8Array.from([1], 1); });
err('of not ctor', function () { Int8Array.of.call(function () { return {}; }, 1); });
err('species not ta', function () { var a = new Int8Array(2); a.constructor = {}; a.constructor[Symbol.species] = function () { return {}; }; a.slice(); });
err('species short', function () { var a = new Int8Array(2); a.constructor = {}; a.constructor[Symbol.species] = function () { return new Int8Array(1); }; a.map(function (x) { return x; }); });
err('define accessor', function () { Object.defineProperty(new Int8Array(2), '0', { get: function () {} }); });
err('define past end', function () { Object.defineProperty(new Int8Array(2), '5', { value: 1 }); });
err('define not configurable', function () { Object.defineProperty(new Int8Array(2), '0', { value: 1, configurable: false }); });
err('strict set past end', function () { 'use strict'; var a = new Int8Array(2); a[5] = 1; return a[5]; });
err('freeze', function () { Object.freeze(new Int8Array(1)); });
err('seal', function () { Object.seal(new Int8Array(1)); });
err('symbol value', function () { new Int8Array(1)[0] = Symbol(); });
err('sort comparator', function () { new Int8Array(2).sort(1); });
err('with index', function () { new Int8Array(2).with(5, 1); });
err('incompatible display', function () { Map.prototype.get.call(new Int8Array(2)); });
err('incompatible buffer display', function () { Map.prototype.get.call(new ArrayBuffer(2)); });
errors.join('\n')"#;

#[test]
fn keys_behaviours_and_errors_read_as_the_reference_browser_gives_them() {
    assert_eq!(
        run(PROBE_KEYS),
        r#"ArrayBuffer: length [number --c], name [string --c], prototype [object ---], isView [function w-c len=1 name=isView], Symbol(Symbol.species) [accessor --c len=0 name=get [Symbol.species]]
ArrayBuffer.prototype: constructor [function w-c len=1 name=ArrayBuffer], byteLength [accessor --c len=0 name=get byteLength], slice [function w-c len=2 name=slice], maxByteLength [accessor --c len=0 name=get maxByteLength], resizable [accessor --c len=0 name=get resizable], resize [function w-c len=1 name=resize], transfer [function w-c len=0 name=transfer], transferToFixedLength [function w-c len=0 name=transferToFixedLength], detached [accessor --c len=0 name=get detached], Symbol(Symbol.toStringTag) [string --c]
TypedArray: length [number --c], name [string --c], prototype [object ---], of [function w-c len=0 name=of], from [function w-c len=1 name=from], Symbol(Symbol.species) [accessor --c len=0 name=get [Symbol.species]]
TypedArray.prototype: constructor [function w-c len=0 name=TypedArray], buffer [accessor --c len=0 name=get buffer], byteLength [accessor --c len=0 name=get byteLength], byteOffset [accessor --c len=0 name=get byteOffset], length [accessor --c len=0 name=get length], entries [function w-c len=0 name=entries], keys [function w-c len=0 name=keys], values [function w-c len=0 name=values], at [function w-c len=1 name=at], copyWithin [function w-c len=2 name=copyWithin], every [function w-c len=1 name=every], fill [function w-c len=1 name=fill], filter [function w-c len=1 name=filter], find [function w-c len=1 name=find], findIndex [function w-c len=1 name=findIndex], findLast [function w-c len=1 name=findLast], findLastIndex [function w-c len=1 name=findLastIndex], forEach [function w-c len=1 name=forEach], includes [function w-c len=1 name=includes], indexOf [function w-c len=1 name=indexOf], join [function w-c len=1 name=join], lastIndexOf [function w-c len=1 name=lastIndexOf], map [function w-c len=1 name=map], reverse [function w-c len=0 name=reverse], reduce [function w-c len=1 name=reduce], reduceRight [function w-c len=1 name=reduceRight], set [function w-c len=1 name=set], slice [function w-c len=2 name=slice], some [function w-c len=1 name=some], sort [function w-c len=1 name=sort], subarray [function w-c len=2 name=subarray], toReversed [function w-c len=0 name=toReversed], toSorted [function w-c len=1 name=toSorted], with [function w-c len=2 name=with], toLocaleString [function w-c len=0 name=toLocaleString], toString [function w-c len=0 name=toString], Symbol(Symbol.toStringTag) [accessor --c len=0 name=get [Symbol.toStringTag]], Symbol(Symbol.iterator) [function w-c len=0 name=values]
Int8Array: length [number --c], name [string --c], prototype [object ---], BYTES_PER_ELEMENT [number ---]
Int8Array.prototype: constructor [function w-c len=3 name=Int8Array], BYTES_PER_ELEMENT [number ---]
Float16Array: length [number --c], name [string --c], prototype [object ---], BYTES_PER_ELEMENT [number ---]
instance: 0 [number wec], 1 [number wec]"#
    );
    assert_eq!(
        run(PROBE_FACTS),
        r#"name TypedArray 0
1,2
[object Int8Array]
[object ArrayBuffer]
true
true
undefined
clamped 2,2,0,255,0,0,254,255
int8 -56,127,1,-1,0,0,0
uint32 4294967295,0,2
f16 Infinity,65504,0.333251953125,0,-0,5.960464477539063e-8,0,5.960464477539063e-8
f32 0.3333333432674408,Infinity,0
keys 0,1,2
{"0":1,"1":2,"2":3}
undefined
undefined
false
false
true
false
undefined
true
true
false
true
9
false
true
false
proto set 2 1
expando 0,1,2,foo 0,1,2,foo
valueOf v,v
rab true 16 4 4
grown 8 4 6 6
shrunk 1 0 0 0 0 0 0
detached true 0 0 0 undefined 4 false
slice 3 true false
sort -Infinity,-0,0,1,3,NaN
toLocale 1,234.5,2 1-2
subarray 2,3 3
from 7,0 1,2,3
map -56,-112
set 0,1,2,-1
overlap 1,2,1,2,3
copyWithin 4,5,3,4,5
with 1,9 1,2,3 2,1
includes true -1 false
species [object Uint8Array]
iter {"done":true}
freeze true false"#
    );
    assert_eq!(
        run(PROBE_ERRORS),
        r#"ArrayBuffer(): TypeError: Constructor ArrayBuffer requires 'new'
new ArrayBuffer(-1): RangeError: Invalid array buffer length
new ArrayBuffer(2**53): RangeError: Invalid array buffer length
new ArrayBuffer(1e12): RangeError: Invalid array buffer length
maxByteLength < length: RangeError: Invalid array buffer max length
maxByteLength huge: RangeError: Invalid array buffer max length
resize fixed: TypeError: Method ArrayBuffer.prototype.resize called on incompatible receiver #<ArrayBuffer>
resize past max: RangeError: ArrayBuffer.prototype.resize: Invalid length parameter
resize detached: TypeError: Cannot perform ArrayBuffer.prototype.resize on a detached ArrayBuffer
byteLength receiver: TypeError: Method get ArrayBuffer.prototype.byteLength called on incompatible receiver #<Object>
slice receiver: TypeError: Method ArrayBuffer.prototype.slice called on incompatible receiver [object Int8Array]
slice detached: TypeError: Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer
transfer detached: TypeError: Cannot perform ArrayBuffer.prototype.transfer on a detached ArrayBuffer
slice species not buffer: TypeError: Method ArrayBuffer.prototype.slice called on incompatible receiver #<Object>
slice species same: TypeError: ArrayBuffer subclass returned this from species constructor
slice species small: TypeError: Derived ArrayBuffer constructor created a buffer which was too small
Int8Array(): TypeError: Constructor Int8Array requires 'new'
new TypedArray(): TypeError: Abstract class TypedArray not directly constructable
TypedArray(): TypeError: Abstract class TypedArray not directly constructable
new Int8Array(-1): RangeError: Invalid typed array length: -1
new Int8Array(2**53): RangeError: Invalid typed array length: 9007199254740992
new Float64Array(2**31): RangeError: Array buffer allocation failed
Int16 odd offset: RangeError: start offset of Int16Array should be a multiple of 2
Int16 odd length: RangeError: byte length of Int16Array should be a multiple of 2
offset past end: RangeError: Start offset 8 is outside the bounds of the buffer
length past end: RangeError: Invalid typed array length: 4
Int32 offset past: RangeError: Invalid typed array length: 2
negative offset: RangeError: Start offset -1 is outside the bounds of the buffer
detached buffer ctor: TypeError: Cannot perform Construct on a detached ArrayBuffer
detached ta ctor: TypeError: Cannot perform Construct on a detached or out-of-bounds ArrayBuffer
fill receiver: TypeError: this is not a typed array.
length receiver: TypeError: Method get TypedArray.prototype.length called on incompatible receiver [object Array]
buffer receiver: TypeError: Method get %TypedArray%.prototype.buffer called on incompatible receiver #<Object>
fill detached: TypeError: Cannot perform %TypedArray%.prototype.fill on a detached or out-of-bounds ArrayBuffer
map detached: TypeError: Cannot perform %TypedArray%.prototype.map on a detached or out-of-bounds ArrayBuffer
values detached: TypeError: Cannot perform %TypedArray%.prototype.values on a detached or out-of-bounds ArrayBuffer
next detached: TypeError: Cannot perform Array Iterator.prototype.next on a detached or out-of-bounds ArrayBuffer
join out of bounds: TypeError: Cannot perform %TypedArray%.prototype.join on a detached or out-of-bounds ArrayBuffer
set offset: RangeError: offset is out of bounds
set negative: RangeError: offset is out of bounds
set detached source: TypeError: Cannot perform %TypedArray%.prototype.set on a detached or out-of-bounds ArrayBuffer
from not ctor: TypeError: Method %TypedArray%.from called on incompatible receiver [object Number]
from mapper: TypeError: number 1 is not a function
of not ctor: TypeError: Method %TypedArray%.of called on incompatible receiver #<Object>
species not ta: TypeError: Method %TypedArray%.prototype.slice called on incompatible receiver #<Object>
species short: TypeError: Derived TypedArray constructor created an array which was too small
define accessor: TypeError: Cannot redefine property: 0
define past end: TypeError: Invalid typed array index
define not configurable: TypeError: Cannot redefine property: 0
strict set past end: no throw undefined
freeze: TypeError: Cannot freeze array buffer views with elements
seal: TypeError: Cannot seal array buffer views with elements
symbol value: TypeError: Cannot convert a Symbol value to a number
sort comparator: TypeError: The comparison function must be either a function or undefined: 1
with index: RangeError: Invalid typed array index
incompatible display: TypeError: Method Map.prototype.get called on incompatible receiver [object Int8Array]
incompatible buffer display: TypeError: Method Map.prototype.get called on incompatible receiver #<ArrayBuffer>"#
    );
}
