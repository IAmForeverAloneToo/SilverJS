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

fn shared_agent() -> Agent {
    Agent::new(AgentOptions {
        shared_memory: true,
        ..AgentOptions::default()
    })
}

#[test]
fn data_views_read_and_write_in_either_byte_order() {
    assert_eq!(
        run(
            "var b = new ArrayBuffer(12); var d = new DataView(b, 2); var u = new Uint8Array(b); \
             d.setInt16(0, -2); d.setInt16(2, -2, true); d.setUint32(4, 0x01020304); \
             var e = new DataView(new ArrayBuffer(8)); e.setFloat64(0, Math.PI); \
             [u.join(), d.getUint16(0), d.getUint16(0, true), d.getInt32(4, true), d.byteLength, \
              e.getFloat64(0) === Math.PI, e.getFloat64(0, true) === Math.PI, e.getUint8(0), \
              ArrayBuffer.isView(d), new DataView(b, 12).byteLength, \
              (function () { var r = new ArrayBuffer(4, { maxByteLength: 8 }); var t = new DataView(r); r.transfer(); \
                try { return t.byteLength; } catch (x) { return x.name; } })()].join('|')"
        ),
        "0,0,255,254,254,255,1,2,3,4,0,0|65534|65279|67305985|10|true|false|64|true|0|TypeError"
    );
}

#[test]
fn atomics_wrap_as_their_element_type_does() {
    assert_eq!(
        run(
            "var i8 = new Int8Array([127, -1]); var u16 = new Uint16Array(1); var i32 = new Int32Array([6]); \
             [Atomics.add(i8, 0, 1), i8[0], Atomics.sub(u16, 0, 1), u16[0], Atomics.xor(i32, 0, 3), \
              Atomics.and(i32, 0, -1), Atomics.or(i32, 0, 8), i32[0], \
              Atomics.compareExchange(i8, 1, 255, 5), i8[1], Atomics.compareExchange(i8, 1, 4, 9), i8[1], \
              Atomics.exchange(u16, 0, -2), u16[0], Atomics.store(i8, 0, Infinity), i8[0], \
              (function () { var r = new ArrayBuffer(8, { maxByteLength: 8 }); var t = new Int32Array(r, 0, 2); \
                try { return Atomics.store(t, 0, { valueOf: function () { r.resize(4); return 1; } }); } catch (x) { return x.name; } })()].join()"
        ),
        "127,-128,0,65535,6,5,5,13,-1,5,5,5,65535,65534,Infinity,0,TypeError"
    );
}

#[test]
fn shared_buffers_grow_and_notify_wakes_waiters_in_order() {
    let mut agent = shared_agent();
    agent.set_stress(true);
    assert_eq!(
        run_in(
            &mut agent,
            "var log = []; var s = new SharedArrayBuffer(2, { maxByteLength: 8 }); var v = new Uint16Array(s); \
             s.grow(8); var a = new Int32Array(s); \
             Atomics.waitAsync(a, 0, 0).value.then(function (r) { log.push('first ' + r); }); \
             Atomics.waitAsync(a, 0, 0).value.then(function (r) { log.push('second ' + r); }); \
             Atomics.waitAsync(a, 1, 0).value.then(function (r) { log.push('other ' + r); }); \
             [v.length, s.byteLength, s.growable].join()"
        ),
        "4,8,true"
    );
    // The waiters' promises are held by the agent alone while they wait.
    assert_eq!(
        run_in(
            &mut agent,
            "log.push(Atomics.notify(a, 0, 1), Atomics.notify(a, 1), Atomics.notify(a, 0), Atomics.notify(a, 0)); \
             log.join()"
        ),
        "1,1,1,0"
    );
    assert_eq!(
        run_in(&mut agent, "log.join()"),
        "1,1,1,0,first ok,other ok,second ok"
    );
}

#[test]
fn base64_and_hex_decode_as_far_as_the_array_goes() {
    assert_eq!(
        run(
            "var a = new Uint8Array(4); var r = a.setFromBase64('AQIDBAUG'); \
             var c = new Uint8Array(6); var e; try { c.setFromBase64('AQIDBA*'); } catch (x) { e = x.name; } \
             var h = new Uint8Array(3); var hr = h.setFromHex('0aFf'); \
             var bytes = new Uint8Array([0, 1, 2, 253, 254, 255]); var url = bytes.toBase64({ alphabet: 'base64url' }); \
             [r.read, r.written, a.join(), e, c.join(), hr.read, hr.written, h.join(), bytes.toBase64(), url, \
              Uint8Array.fromBase64(url, { alphabet: 'base64url' }).join(), bytes.toHex(), \
              Uint8Array.fromBase64('AQ=\\n=').join(), Uint8Array.fromBase64('AQI', { lastChunkHandling: 'stop-before-partial' }).length, \
              new Uint8Array(0).setFromBase64('AQID').read, \
              Uint8Array.fromBase64('-w', { alphabet: 'base64url' }).join()].join('|')"
        ),
        "4|3|1,2,3,0|SyntaxError|1,2,3,0,0,0|4|2|10,255,0|AAEC/f7/|AAEC_f7_|0,1,2,253,254,255|000102fdfeff|1|0|0|251"
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

/// The binary data probe's `DataView`, `Atomics` and base64 and hex
/// (J4.2b's), on a page without shared memory.
const PROBE_VIEWS: &str = r#"function keys(o) {
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
if (typeof JSON === 'undefined') var JSON = { stringify: function (o) { return '{' + Object.keys(o).filter(function (k) { return o[k] !== undefined; }).map(function (k) { return '"' + k + '":' + String(o[k]); }).join(',') + '}'; } };
function show(v) { return Object.is(v, -0) ? '-0' : String(v); }
function list(ta) { return Array.prototype.map.call(ta, show).join(); }
var out = { keys: {}, facts: [], errors: [] }; var f = out.facts;
out.keys['Uint8Array'] = keys(Uint8Array);
out.keys['Uint8Array.prototype'] = keys(Uint8Array.prototype);
out.keys['DataView'] = keys(DataView);
out.keys['DataView.prototype'] = keys(DataView.prototype);
out.keys['Atomics'] = keys(Atomics);
out.keys.SharedArrayBuffer = typeof SharedArrayBuffer;
f.push('dv ' + (function () { var d = new DataView(new ArrayBuffer(8)); d.setUint16(0, 0x1234); d.setFloat32(4, 1.5, true); return [d.getUint8(0), d.getUint16(0, true), d.getFloat32(4, true), d.getInt8(1), d.byteLength, d.getFloat16 ? d.getFloat16(4, true) : 'none'].join(); })());
f.push('b64 ' + new Uint8Array([0xfb, 0xff, 0x01]).toBase64() + ' ' + new Uint8Array([0xfb, 0xff, 0x01]).toBase64({ alphabet: 'base64url', omitPadding: true }) + ' ' + new Uint8Array([0xab, 0x0c]).toHex() + ' ' + list(Uint8Array.fromBase64('+/8B')) + ' ' + list(Uint8Array.fromHex('ab0C')) + ' ' + list(Uint8Array.fromBase64('ab c', { lastChunkHandling: 'loose' })));
f.push('setFrom ' + JSON.stringify(new Uint8Array(2).setFromBase64('AQID')) + ' ' + JSON.stringify(new Uint8Array(4).setFromHex('0102')));
f.push('atomics ' + (function () { var a = new Int32Array(2); return [Atomics.add(a, 0, 5), Atomics.load(a, 0), Atomics.compareExchange(a, 0, 5, 7), a[0], Atomics.isLockFree(4), Atomics.notify(a, 0, 1)].join(); })());
function err(label, fn) {
  try { var v = fn(); out.errors.push(label + ': no throw ' + String(v)); }
  catch (e) { out.errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }
}
var det = new ArrayBuffer(8); var detv = new Uint8Array(det); det.transfer();
err('DataView()', function () { DataView(new ArrayBuffer(1)); });
err('new DataView(1)', function () { new DataView(1); });
err('DataView offset', function () { new DataView(new ArrayBuffer(4), 5); });
err('DataView length', function () { new DataView(new ArrayBuffer(4), 1, 4); });
err('DataView detached', function () { new DataView(det); });
err('getInt8 range', function () { new DataView(new ArrayBuffer(4)).getInt8(4); });
err('getInt32 range', function () { new DataView(new ArrayBuffer(4)).getInt32(1); });
err('getInt8 negative', function () { new DataView(new ArrayBuffer(4)).getInt8(-1); });
err('getInt8 receiver', function () { DataView.prototype.getInt8.call({}, 0); });
err('dv detached get', function () { var b = new ArrayBuffer(4); var d = new DataView(b); b.transfer(); d.getInt8(0); });
err('dv byteLength detached', function () { var b = new ArrayBuffer(4); var d = new DataView(b); b.transfer(); return d.byteLength; });
err('fromBase64 bad', function () { Uint8Array.fromBase64('*'); });
err('fromBase64 partial', function () { Uint8Array.fromBase64('AQ'); });
err('fromBase64 strict', function () { Uint8Array.fromBase64('AR==', { lastChunkHandling: 'strict' }); });
err('fromBase64 option', function () { Uint8Array.fromBase64('AQ==', { alphabet: 'x' }); });
err('fromBase64 not string', function () { Uint8Array.fromBase64(1); });
err('fromHex odd', function () { Uint8Array.fromHex('abc'); });
err('fromHex bad', function () { Uint8Array.fromHex('zz'); });
err('toBase64 receiver', function () { Uint8Array.prototype.toBase64.call(new Int8Array(1)); });
err('toHex detached', function () { var b = new ArrayBuffer(1); var a = new Uint8Array(b); b.transfer(); a.toHex(); });
err('Atomics non-integer', function () { Atomics.add(new Float64Array(1), 0, 1); });
err('Atomics index', function () { Atomics.load(new Int32Array(1), 5); });
err('Atomics wait', function () { Atomics.wait(new Int32Array(1), 0, 0, 0); });
err('Atomics not ta', function () { Atomics.load([1], 0); });
var lines = [];
Object.keys(out.keys).forEach(function (k) { var v = out.keys[k]; lines.push(k + ': ' + (typeof v === 'string' ? v : v.join(', '))); });
lines.concat(out.facts, out.errors).join('\n')"#;

/// The shared memory probe: a cross-origin isolated page's
/// `SharedArrayBuffer` and `Atomics`, and the messages the first probe
/// did not reach.
const PROBE_SHARED: &str = r#"if (typeof JSON === 'undefined') var JSON = { stringify: function (o) { return '{' + Object.keys(o).filter(function (k) { return o[k] !== undefined; }).map(function (k) { return '"' + k + '":' + String(o[k]); }).join(',') + '}'; } };

// SilverJS's J4.2b probe: SharedArrayBuffer and Atomics as Chrome shows
// them on a cross-origin isolated page (served with
// Cross-Origin-Opener-Policy: same-origin and
// Cross-Origin-Embedder-Policy: require-corp), plus DataView's and
// base64's messages the first probe did not reach.
function keys(o) {
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
var out = { isolated: true, keys: {}, facts: [], errors: [] };
out.order = Object.getOwnPropertyNames(globalThis).filter(function (k) { return ['Math', 'Intl', 'ArrayBuffer', 'SharedArrayBuffer', 'Atomics', 'Uint8Array', 'Float16Array', 'DataView', 'Map'].indexOf(k) >= 0; }).join();
function fact(label, fn) {
  try { out.facts.push(label + ' ' + String(fn())); }
  catch (e) { out.facts.push(label + ' threw ' + e.constructor.name + ': ' + e.message); }
}
function err(label, fn) {
  try { var v = fn(); out.errors.push(label + ': no throw ' + String(v)); }
  catch (e) { out.errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }
}
var S = typeof SharedArrayBuffer === 'function' ? SharedArrayBuffer : null;
if (S) {
  out.keys.SharedArrayBuffer = keys(S);
  out.keys['SharedArrayBuffer.prototype'] = keys(S.prototype);
  var gsab = new S(4, { maxByteLength: 8 });
  fact('sab', function () { var s = new S(4); return [s.byteLength, s.growable, s.maxByteLength, Object.prototype.toString.call(s), S.length, S.name, ArrayBuffer.isView(new Int8Array(s))]; });
  fact('sab growable', function () { return [gsab.byteLength, gsab.growable, gsab.maxByteLength, (gsab.grow(6), gsab.byteLength), new Uint8Array(gsab).length]; });
  fact('sab slice', function () { var s = new S(4); new Uint8Array(s).set([1, 2, 3, 4]); var t = s.slice(1, 3); return [t instanceof S, t.byteLength, new Uint8Array(t).join()]; });
  fact('sab species', function () { return Object.getOwnPropertyDescriptor(S, Symbol.species).get.name; });
  fact('sab transfer', function () { return typeof S.prototype.transfer + ' ' + typeof S.prototype.resize + ' ' + typeof S.prototype.detached; });
  fact('dv over sab', function () { var d = new DataView(new S(4)); d.setInt16(0, -2); return [d.getInt16(0), d.buffer instanceof S]; });
  fact('ta over sab', function () { var a = new Int32Array(new S(8)); a[1] = 7; return [a.length, a[1], a.buffer instanceof S]; });
  fact('atomics shared', function () { var a = new Int32Array(new S(8)); return [Atomics.store(a, 0, -0), Object.is(Atomics.store(a, 0, -0), 0), Atomics.store(a, 0, 3.7), Atomics.add(a, 0, 2), Atomics.sub(a, 0, 1), Atomics.and(a, 0, 6), Atomics.or(a, 0, 1), Atomics.xor(a, 0, 3), Atomics.exchange(a, 0, 9), Atomics.load(a, 0), Atomics.notify(a, 0), Atomics.notify(a, 0, 0)]; });
  fact('atomics wrap', function () { var a = new Uint8Array(new S(2)); return [Atomics.add(a, 0, 300), a[0], Atomics.sub(a, 0, 50), a[0], Atomics.store(a, 1, 1e10), a[1], Atomics.compareExchange(a, 1, 1e10, 5), a[1]]; });
  fact('waitAsync not-equal', function () { var r = Atomics.waitAsync(new Int32Array(new S(4)), 0, 1); return [r.async, r.value, Object.keys(r).join('/')]; });
  fact('waitAsync timed-out', function () { var r = Atomics.waitAsync(new Int32Array(new S(4)), 0, 0, 0); return [r.async, r.value]; });
  fact('waitAsync promise', function () { var a = new Int32Array(new S(4)); var r = Atomics.waitAsync(a, 0, 0); var n = Atomics.notify(a, 0); return [r.async, r.value instanceof Promise, n, Object.getPrototypeOf(r) === Object.prototype]; });
  fact('waitAsync timeout NaN', function () { var r = Atomics.waitAsync(new Int32Array(new S(4)), 0, 0, NaN); return [r.async, typeof r.value]; });
  err('sab()', function () { S(1); });
  err('new sab(-1)', function () { new S(-1); });
  err('sab max', function () { new S(8, { maxByteLength: 4 }); });
  err('sab grow shrink', function () { new S(4, { maxByteLength: 8 }).grow(2); });
  err('sab grow past max', function () { new S(4, { maxByteLength: 8 }).grow(9); });
  err('sab grow fixed', function () { new S(4).grow(4); });
  err('sab byteLength on ab', function () { Object.getOwnPropertyDescriptor(S.prototype, 'byteLength').get.call(new ArrayBuffer(1)); });
  err('ab byteLength on sab', function () { Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, 'byteLength').get.call(new S(1)); });
  err('ab slice on sab', function () { ArrayBuffer.prototype.slice.call(new S(1)); });
  err('sab slice species', function () { var s = new S(4); s.constructor = {}; s.constructor[Symbol.species] = function () { return s; }; s.slice(); });
  err('sab slice species ab', function () { var s = new S(4); s.constructor = {}; s.constructor[Symbol.species] = function (n) { return new ArrayBuffer(n); }; s.slice(); });
  err('sab slice short', function () { var s = new S(4); s.constructor = {}; s.constructor[Symbol.species] = function () { return new S(1); }; s.slice(); });
  err('wait main thread', function () { Atomics.wait(new Int32Array(new S(4)), 0, 0, 0); });
  err('wait not-equal main', function () { return Atomics.wait(new Int32Array(new S(4)), 0, 1, 0); });
  err('wait Int8', function () { Atomics.wait(new Int8Array(new S(4)), 0, 0, 0); });
  err('wait Uint32', function () { Atomics.wait(new Uint32Array(new S(4)), 0, 0, 0); });
  err('waitAsync Int8', function () { Atomics.waitAsync(new Int8Array(new S(4)), 0, 0); });
  err('waitAsync index', function () { Atomics.waitAsync(new Int32Array(new S(4)), 2, 0); });
  err('notify Int8', function () { Atomics.notify(new Int8Array(new S(4)), 0); });
  err('notify float', function () { Atomics.notify(new Float64Array(new S(8)), 0); });
  err('notify count', function () { return Atomics.notify(new Int32Array(new S(4)), 0, -5); });
  err('dv sab detached?', function () { var d = new DataView(new S(4)); return d.byteLength; });
}
fact('isLockFree', function () { return [0, 1, 2, 3, 4, 5, 8, 16].map(function (n) { return Atomics.isLockFree(n); }); });
fact('pause', function () { return [Atomics.pause(), Atomics.pause(5), Atomics.pause(-0), Atomics.pause.length]; });
fact('atomics plain', function () { var a = new Uint16Array(2); return [Atomics.store(a, 0, 70000), a[0], Atomics.exchange(a, 0, -1), a[0], Atomics.compareExchange(a, 0, -1, 2), a[0], Atomics.notify(a, 0), Atomics.notify(new Int32Array(1), 0, 5)]; });
fact('atomics tag', function () { return [Object.prototype.toString.call(Atomics), typeof Atomics]; });
fact('waitAsync plain', function () { try { Atomics.waitAsync(new Int32Array(1), 0, 0); return 'no throw'; } catch (e) { return e.message; } });
fact('dv tag', function () { return [DataView.name, DataView.length, Object.prototype.toString.call(new DataView(new ArrayBuffer(1)))]; });
fact('dv float16', function () { var d = new DataView(new ArrayBuffer(8)); d.setFloat16(0, 1e5); d.setFloat16(2, 65519, true); d.setFloat16(4, 5.960464477539063e-8); return [d.getFloat16(0), d.getFloat16(2, true), d.getFloat16(4), d.getUint16(0), d.getUint16(4)]; });
fact('dv tracking', function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); var d = new DataView(b, 1); var e = new DataView(b, 1, 2); b.resize(8); var r = [d.byteLength, e.byteLength]; b.resize(2); r.push(d.byteLength); try { e.byteLength; } catch (x) { r.push(x.message); } b.resize(0); try { d.byteOffset; } catch (x) { r.push(x.message); } return r; });
fact('dv conversions', function () { var d = new DataView(new ArrayBuffer(8)); d.setUint8(0, 257); d.setInt8(1, 200); d.setUint32(4, -1); return [d.getUint8(0), d.getInt8(1), d.getUint32(4), d.getInt32(4), d.getUint16(undefined), d.setUint8(0, 1), d.getUint8('0x0')]; });
fact('dv set order', function () { var b = new ArrayBuffer(4); var d = new DataView(b); var log = []; try { d.setUint8({ valueOf: function () { log.push('i'); return 9; } }, { valueOf: function () { log.push('v'); return 1; } }); } catch (e) { log.push(e.constructor.name); } return log.join(); });
fact('dv set detached in value', function () { var b = new ArrayBuffer(4); var d = new DataView(b); try { d.setUint8(0, { valueOf: function () { b.transfer(); return 1; } }); return 'no throw'; } catch (e) { return e.constructor.name + ': ' + e.message; } });
fact('b64 options', function () { var a = new Uint8Array([1, 2, 3, 4]); return [a.toBase64(), a.toBase64({ omitPadding: true }), a.toBase64({ alphabet: 'base64url' }), a.toBase64({ omitPadding: 0 }), Uint8Array.fromBase64(' A Q I D ').join(), Uint8Array.fromBase64('AQID', { lastChunkHandling: 'stop-before-partial' }).join(), Uint8Array.fromBase64('AQIDBA', { lastChunkHandling: 'stop-before-partial' }).join(), Uint8Array.fromBase64('AR', { lastChunkHandling: 'loose' }).join()]; });
fact('setFromBase64', function () { var a = new Uint8Array(4); var r = a.setFromBase64('AQIDBAUG'); var b = new Uint8Array(2); var s = b.setFromBase64('AQ==AQ', { lastChunkHandling: 'loose' }); return [JSON.stringify(r), a.join(), JSON.stringify(s), b.join()]; });
fact('setFromHex', function () { var a = new Uint8Array(1); var r = a.setFromHex('0102'); return [JSON.stringify(r), a.join()]; });
fact('b64 keys order', function () { return Object.keys(new Uint8Array(2).setFromBase64('AQ==')).join(); });
err('setInt8 range', function () { new DataView(new ArrayBuffer(4)).setInt8(4, 0); });
err('setFloat64 range', function () { new DataView(new ArrayBuffer(4)).setFloat64(0, 0); });
err('dv getIndex huge', function () { new DataView(new ArrayBuffer(4)).getInt8(2 ** 53); });
err('dv detached set', function () { var b = new ArrayBuffer(4); var d = new DataView(b); b.transfer(); d.setInt8(0, 0); });
err('dv byteOffset detached', function () { var b = new ArrayBuffer(4); var d = new DataView(b); b.transfer(); return d.byteOffset; });
err('dv buffer receiver', function () { Object.getOwnPropertyDescriptor(DataView.prototype, 'buffer').get.call({}); });
err('dv byteLength receiver', function () { Object.getOwnPropertyDescriptor(DataView.prototype, 'byteLength').get.call(new Int8Array(1)); });
err('dv byteOffset receiver', function () { Object.getOwnPropertyDescriptor(DataView.prototype, 'byteOffset').get.call(1); });
err('setFloat16 receiver', function () { DataView.prototype.setFloat16.call({}, 0, 0); });
err('DataView ta', function () { new DataView(new Int8Array(1)); });
err('DataView offset neg', function () { new DataView(new ArrayBuffer(4), -1); });
err('DataView length neg', function () { new DataView(new ArrayBuffer(4), 0, -1); });
err('DataView resizable past', function () { new DataView(new ArrayBuffer(4, { maxByteLength: 8 }), 5); });
err('DataView detached in proto', function () { var b = new ArrayBuffer(4); var nt = function () {}.bind(); Object.defineProperty(nt, 'prototype', { get: function () { b.transfer(); return DataView.prototype; } }); Reflect.construct(DataView, [b], nt); });
err('DataView shrunk in proto', function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); var nt = function () {}.bind(); Object.defineProperty(nt, 'prototype', { get: function () { b.resize(1); return DataView.prototype; } }); Reflect.construct(DataView, [b, 2], nt); });
err('DataView shrunk len in proto', function () { var b = new ArrayBuffer(4, { maxByteLength: 8 }); var nt = function () {}.bind(); Object.defineProperty(nt, 'prototype', { get: function () { b.resize(2); return DataView.prototype; } }); Reflect.construct(DataView, [b, 0, 3], nt); });
err('toBase64 alphabet', function () { new Uint8Array(1).toBase64({ alphabet: 'x' }); });
err('toBase64 options type', function () { new Uint8Array(1).toBase64(1); });
err('fromBase64 handling', function () { Uint8Array.fromBase64('AQ==', { lastChunkHandling: 'x' }); });
err('fromBase64 partial strict', function () { Uint8Array.fromBase64('AQ', { lastChunkHandling: 'strict' }); });
err('fromBase64 extra pad', function () { Uint8Array.fromBase64('AQ===' ); });
err('fromBase64 pad middle', function () { Uint8Array.fromBase64('AQ==AQID'); });
err('fromBase64 single', function () { Uint8Array.fromBase64('A'); });
err('fromBase64 url char', function () { Uint8Array.fromBase64('-_8B'); });
err('fromBase64 std char in url', function () { Uint8Array.fromBase64('+/8B', { alphabet: 'base64url' }); });
err('setFromBase64 receiver', function () { Uint8Array.prototype.setFromBase64.call(new Int8Array(1), 'AQ=='); });
err('setFromBase64 not string', function () { new Uint8Array(1).setFromBase64(1); });
err('setFromBase64 detached', function () { var b = new ArrayBuffer(1); var a = new Uint8Array(b); b.transfer(); a.setFromBase64('AQ=='); });
err('setFromBase64 bad', function () { new Uint8Array(4).setFromBase64('AQ*D'); });
err('setFromHex receiver', function () { Uint8Array.prototype.setFromHex.call([], '01'); });
err('setFromHex bad', function () { new Uint8Array(4).setFromHex('01zz'); });
err('setFromHex odd', function () { new Uint8Array(4).setFromHex('012'); });
err('setFromHex not string', function () { new Uint8Array(4).setFromHex(1); });
err('fromHex not string', function () { Uint8Array.fromHex(1); });
err('toHex receiver', function () { Uint8Array.prototype.toHex.call(new Uint16Array(1)); });
err('toBase64 detached', function () { var b = new ArrayBuffer(1); var a = new Uint8Array(b); b.transfer(); a.toBase64(); });
err('toBase64 detached in option', function () { var b = new ArrayBuffer(1); var a = new Uint8Array(b); a.toBase64({ get alphabet() { b.transfer(); return 'base64'; } }); });
err('Atomics number', function () { Atomics.load(1, 0); });
err('Atomics string', function () { Atomics.load('ab', 0); });
err('Atomics object', function () { Atomics.load({}, 0); });
err('Atomics dv', function () { Atomics.load(new DataView(new ArrayBuffer(4)), 0); });
err('Atomics subclass', function () { class I extends Float32Array {} Atomics.load(new I(1), 0); });
err('Atomics store float',function () { Atomics.store(new Float32Array(1), 0, 1); });
err('Atomics clamped', function () { Atomics.load(new Uint8ClampedArray(1), 0); });
err('Atomics index neg', function () { Atomics.load(new Int32Array(1), -1); });
err('Atomics index huge', function () { Atomics.load(new Int32Array(1), 2 ** 53); });
err('Atomics detached', function () { var b = new ArrayBuffer(4); var a = new Int32Array(b); b.transfer(); Atomics.load(a, 0); });
err('Atomics detached in value', function () { var b = new ArrayBuffer(4); var a = new Int32Array(b); return Atomics.store(a, 0, { valueOf: function () { b.transfer(); return 1; } }); });
err('Atomics shrunk in value', function () { var b = new ArrayBuffer(8, { maxByteLength: 8 }); var a = new Int32Array(b); return Atomics.add(a, 1, { valueOf: function () { b.resize(4); return 1; } }); });
err('Atomics.wait index', function () { Atomics.wait(new Int32Array(1), 5, 0, 0); });
err('Atomics.notify index', function () { Atomics.notify(new Int32Array(1), 5); });
err('Atomics.pause float', function () { Atomics.pause(1.5); });
err('Atomics.pause string', function () { Atomics.pause('1'); });
err('Atomics.pause NaN', function () { Atomics.pause(NaN); });
err('Atomics.pause Infinity', function () { Atomics.pause(Infinity); });
err('Atomics.pause big', function () { return Atomics.pause(2 ** 60); });

var lines = ['order ' + out.order];
Object.keys(out.keys).forEach(function (k) { lines.push(k + ': ' + out.keys[k].join(', ')); });
lines.concat(out.facts, out.errors).join('\n')"#;

#[test]
fn views_atomics_and_base64_read_as_the_reference_browser_gives_them() {
    assert_eq!(
        run(PROBE_VIEWS),
        r#"Uint8Array: length [number --c], name [string --c], prototype [object ---], BYTES_PER_ELEMENT [number ---], fromBase64 [function w-c len=1 name=fromBase64], fromHex [function w-c len=1 name=fromHex]
Uint8Array.prototype: constructor [function w-c len=3 name=Uint8Array], BYTES_PER_ELEMENT [number ---], toBase64 [function w-c len=0 name=toBase64], setFromBase64 [function w-c len=1 name=setFromBase64], toHex [function w-c len=0 name=toHex], setFromHex [function w-c len=1 name=setFromHex]
DataView: length [number --c], name [string --c], prototype [object ---]
DataView.prototype: constructor [function w-c len=1 name=DataView], buffer [accessor --c len=0 name=get buffer], byteLength [accessor --c len=0 name=get byteLength], byteOffset [accessor --c len=0 name=get byteOffset], getInt8 [function w-c len=1 name=getInt8], setInt8 [function w-c len=2 name=setInt8], getUint8 [function w-c len=1 name=getUint8], setUint8 [function w-c len=2 name=setUint8], getInt16 [function w-c len=1 name=getInt16], setInt16 [function w-c len=2 name=setInt16], getUint16 [function w-c len=1 name=getUint16], setUint16 [function w-c len=2 name=setUint16], getInt32 [function w-c len=1 name=getInt32], setInt32 [function w-c len=2 name=setInt32], getUint32 [function w-c len=1 name=getUint32], setUint32 [function w-c len=2 name=setUint32], getFloat16 [function w-c len=1 name=getFloat16], setFloat16 [function w-c len=2 name=setFloat16], getFloat32 [function w-c len=1 name=getFloat32], setFloat32 [function w-c len=2 name=setFloat32], getFloat64 [function w-c len=1 name=getFloat64], setFloat64 [function w-c len=2 name=setFloat64], Symbol(Symbol.toStringTag) [string --c]
Atomics: load [function w-c len=2 name=load], store [function w-c len=3 name=store], add [function w-c len=3 name=add], sub [function w-c len=3 name=sub], and [function w-c len=3 name=and], or [function w-c len=3 name=or], xor [function w-c len=3 name=xor], exchange [function w-c len=3 name=exchange], compareExchange [function w-c len=4 name=compareExchange], isLockFree [function w-c len=1 name=isLockFree], wait [function w-c len=4 name=wait], waitAsync [function w-c len=4 name=waitAsync], notify [function w-c len=3 name=notify], pause [function w-c len=0 name=pause], Symbol(Symbol.toStringTag) [string --c]
SharedArrayBuffer: undefined
dv 18,13330,1.5,52,8,0
b64 +/8B -_8B ab0c 251,255,1 171,12 105,183
setFrom {"read":0,"written":0} {"read":4,"written":2}
atomics 0,5,5,7,true,0
DataView(): TypeError: Constructor DataView requires 'new'
new DataView(1): TypeError: First argument to DataView constructor must be an ArrayBuffer
DataView offset: RangeError: Start offset 5 is outside the bounds of the buffer
DataView length: RangeError: Invalid DataView length 4
DataView detached: TypeError: Cannot perform DataView constructor on a detached ArrayBuffer
getInt8 range: RangeError: Offset is outside the bounds of the DataView
getInt32 range: RangeError: Offset is outside the bounds of the DataView
getInt8 negative: RangeError: Offset is outside the bounds of the DataView
getInt8 receiver: TypeError: Method DataView.prototype.getInt8 called on incompatible receiver #<Object>
dv detached get: TypeError: Cannot perform DataView.prototype.getInt8 on a detached or out-of-bounds ArrayBuffer
dv byteLength detached: TypeError: Cannot perform get DataView.prototype.byteLength on a detached or out-of-bounds ArrayBuffer
fromBase64 bad: SyntaxError: Found a character that cannot be part of a valid base64 string.
fromBase64 partial: no throw undefined
fromBase64 strict: SyntaxError: The base64 input terminates with non-zero padding bits.
fromBase64 option: TypeError: invalid option x
fromBase64 not string: TypeError: input argument must be a string
fromHex odd: SyntaxError: Input string must contain hex characters in even length
fromHex bad: SyntaxError: Input string must contain hex characters in even length
toBase64 receiver: TypeError: Method Uint8Array.prototype.toBase64 called on incompatible receiver undefined
toHex detached: TypeError: Cannot perform Uint8Array.prototype.toHex on a detached or out-of-bounds ArrayBuffer
Atomics non-integer: TypeError: [object Float64Array] is not an integer typed array.
Atomics index: RangeError: Invalid atomic access index
Atomics wait: TypeError: [object Int32Array] is not a shared typed array.
Atomics not ta: TypeError: [object Array] is not an integer typed array."#
    );
    let mut shared = Agent::new(AgentOptions {
        shared_memory: true,
        ..AgentOptions::default()
    });
    assert_eq!(
        run_in(&mut shared, PROBE_SHARED),
        r#"order Math,ArrayBuffer,Atomics,Uint8Array,Float16Array,DataView,Map,SharedArrayBuffer
SharedArrayBuffer: length [number --c], name [string --c], prototype [object ---], Symbol(Symbol.species) [accessor --c len=0 name=get [Symbol.species]]
SharedArrayBuffer.prototype: constructor [function w-c len=1 name=SharedArrayBuffer], byteLength [accessor --c len=0 name=get byteLength], slice [function w-c len=2 name=slice], maxByteLength [accessor --c len=0 name=get maxByteLength], growable [accessor --c len=0 name=get growable], grow [function w-c len=1 name=grow], Symbol(Symbol.toStringTag) [string --c]
sab 4,false,4,[object SharedArrayBuffer],1,SharedArrayBuffer,true
sab growable 4,true,8,6,6
sab slice true,2,2,3
sab species get [Symbol.species]
sab transfer undefined undefined undefined
dv over sab -2,true
ta over sab 2,7,true
atomics shared 0,true,3,3,5,4,4,5,6,9,0,0
atomics wrap 0,44,44,250,10000000000,0,0,5
waitAsync not-equal false,not-equal,async/value
waitAsync timed-out false,timed-out
waitAsync promise true,true,1,true
waitAsync timeout NaN true,object
isLockFree false,true,true,false,true,false,true,false
pause ,,,0
atomics plain threw TypeError: [object Uint16Array] is not an int32 or BigInt64 typed array.
atomics tag [object Atomics],object
waitAsync plain [object Int32Array] is not a shared typed array.
dv tag DataView,1,[object DataView]
dv float16 Infinity,65504,5.960464477539063e-8,31744,1
dv tracking 7,2,1,Cannot perform get DataView.prototype.byteLength on a detached or out-of-bounds ArrayBuffer,Cannot perform get DataView.prototype.byteOffset on a detached or out-of-bounds ArrayBuffer
dv conversions 1,-56,4294967295,-1,456,,1
dv set order i,v,RangeError
dv set detached in value TypeError: Cannot perform DataView.prototype.setUint8 on a detached or out-of-bounds ArrayBuffer
b64 options AQIDBA==,AQIDBA,AQIDBA==,AQIDBA==,1,2,3,1,2,3,1,2,3,1
setFromBase64 threw SyntaxError: Found a character that cannot be part of a valid base64 string.
setFromHex {"read":2,"written":1},1
b64 keys order read,written
sab(): TypeError: Constructor SharedArrayBuffer requires 'new'
new sab(-1): RangeError: Invalid array buffer length
sab max: RangeError: Invalid array buffer max length
sab grow shrink: RangeError: SharedArrayBuffer.prototype.grow: Invalid length parameter
sab grow past max: RangeError: SharedArrayBuffer.prototype.grow: Invalid length parameter
sab grow fixed: TypeError: Method SharedArrayBuffer.prototype.grow called on incompatible receiver #<SharedArrayBuffer>
sab byteLength on ab: TypeError: Method get SharedArrayBuffer.prototype.byteLength called on incompatible receiver #<ArrayBuffer>
ab byteLength on sab: TypeError: Method get ArrayBuffer.prototype.byteLength called on incompatible receiver #<SharedArrayBuffer>
ab slice on sab: TypeError: Method ArrayBuffer.prototype.slice called on incompatible receiver #<SharedArrayBuffer>
sab slice species: TypeError: SharedArrayBuffer subclass returned this from species constructor
sab slice species ab: TypeError: Method SharedArrayBuffer.prototype.slice called on incompatible receiver #<ArrayBuffer>
sab slice short: TypeError: Derived SharedArrayBuffer constructor created a buffer which was too small
wait main thread: TypeError: Atomics.wait cannot be called in this context
wait not-equal main: TypeError: Atomics.wait cannot be called in this context
wait Int8: TypeError: [object Int8Array] is not an int32 or BigInt64 typed array.
wait Uint32: TypeError: [object Uint32Array] is not an int32 or BigInt64 typed array.
waitAsync Int8: TypeError: [object Int8Array] is not an int32 or BigInt64 typed array.
waitAsync index: RangeError: Invalid atomic access index
notify Int8: TypeError: [object Int8Array] is not an int32 or BigInt64 typed array.
notify float: TypeError: [object Float64Array] is not an int32 or BigInt64 typed array.
notify count: no throw 0
dv sab detached?: no throw 4
setInt8 range: RangeError: Offset is outside the bounds of the DataView
setFloat64 range: RangeError: Offset is outside the bounds of the DataView
dv getIndex huge: RangeError: Offset is outside the bounds of the DataView
dv detached set: TypeError: Cannot perform DataView.prototype.setInt8 on a detached or out-of-bounds ArrayBuffer
dv byteOffset detached: TypeError: Cannot perform get DataView.prototype.byteOffset on a detached or out-of-bounds ArrayBuffer
dv buffer receiver: TypeError: Method get DataView.prototype.buffer called on incompatible receiver #<Object>
dv byteLength receiver: TypeError: Method get DataView.prototype.byteLength called on incompatible receiver [object Int8Array]
dv byteOffset receiver: TypeError: Method get DataView.prototype.byteOffset called on incompatible receiver 1
setFloat16 receiver: TypeError: Method DataView.prototype.setFloat16 called on incompatible receiver #<Object>
DataView ta: TypeError: First argument to DataView constructor must be an ArrayBuffer
DataView offset neg: RangeError: Start offset -1 is outside the bounds of the buffer
DataView length neg: RangeError: Invalid DataView length -1
DataView resizable past: RangeError: Start offset 5 is outside the bounds of the buffer
DataView detached in proto: TypeError: Cannot perform DataView constructor on a detached ArrayBuffer
DataView shrunk in proto: RangeError: Start offset 2 is outside the bounds of the buffer
DataView shrunk len in proto: RangeError: Invalid DataView length undefined
toBase64 alphabet: TypeError: invalid option x
toBase64 options type: TypeError: invalid_argument
fromBase64 handling: TypeError: invalid option x
fromBase64 partial strict: SyntaxError: The base64 input terminates with a single character, excluding padding (=).
fromBase64 extra pad: SyntaxError: Found a character that cannot be part of a valid base64 string.
fromBase64 pad middle: SyntaxError: Found a character that cannot be part of a valid base64 string.
fromBase64 single: SyntaxError: The base64 input terminates with a single character, excluding padding (=).
fromBase64 url char: SyntaxError: Found a character that cannot be part of a valid base64 string.
fromBase64 std char in url: SyntaxError: Found a character that cannot be part of a valid base64 string.
setFromBase64 receiver: TypeError: Method Uint8Array.prototype.setFromBase64 called on incompatible receiver undefined
setFromBase64 not string: TypeError: input argument must be a string
setFromBase64 detached: TypeError: Cannot perform Uint8Array.prototype.setFromBase64 on a detached or out-of-bounds ArrayBuffer
setFromBase64 bad: SyntaxError: Found a character that cannot be part of a valid base64 string.
setFromHex receiver: TypeError: this is not a typed array.
setFromHex bad: SyntaxError: Input string must contain hex characters in even length
setFromHex odd: SyntaxError: Input string must contain hex characters in even length
setFromHex not string: TypeError: input argument must be a string
fromHex not string: TypeError: input argument must be a string
toHex receiver: TypeError: Method Uint8Array.prototype.toHex called on incompatible receiver undefined
toBase64 detached: TypeError: Cannot perform Uint8Array.prototype.toBase64 on a detached or out-of-bounds ArrayBuffer
toBase64 detached in option: TypeError: Cannot perform Uint8Array.prototype.toBase64 on a detached or out-of-bounds ArrayBuffer
Atomics number: TypeError: 1 is not an integer typed array.
Atomics string: TypeError: ab is not an integer typed array.
Atomics object: TypeError: #<Object> is not an integer typed array.
Atomics dv: TypeError: #<DataView> is not an integer typed array.
Atomics subclass: TypeError: [object Float32Array] is not an integer typed array.
Atomics store float: TypeError: [object Float32Array] is not an integer typed array.
Atomics clamped: TypeError: [object Uint8ClampedArray] is not an integer typed array.
Atomics index neg: RangeError: Invalid atomic access index
Atomics index huge: RangeError: Invalid atomic access index
Atomics detached: TypeError: Cannot perform Atomics.load on a detached or out-of-bounds ArrayBuffer
Atomics detached in value: TypeError: Cannot perform Atomics.store on a detached or out-of-bounds ArrayBuffer
Atomics shrunk in value: TypeError: Cannot perform Atomics.add on a detached or out-of-bounds ArrayBuffer
Atomics.wait index: TypeError: [object Int32Array] is not a shared typed array.
Atomics.notify index: RangeError: Invalid atomic access index
Atomics.pause float: TypeError: Atomics.pause argument must be undefined or an integer
Atomics.pause string: TypeError: Atomics.pause argument must be undefined or an integer
Atomics.pause NaN: TypeError: Atomics.pause argument must be undefined or an integer
Atomics.pause Infinity: TypeError: Atomics.pause argument must be undefined or an integer
Atomics.pause big: no throw undefined"#
    );
}
