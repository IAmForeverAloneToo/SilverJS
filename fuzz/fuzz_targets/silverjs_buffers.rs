//! Buffers changed under their views' feet: the bytes become a script
//! that makes `ArrayBuffer`s (resizable and fixed), `SharedArrayBuffer`s
//! (growable and fixed), typed arrays and `DataView`s over them, then
//! reads, writes and calls the views' methods (the BigInt arrays' with
//! BigInts), `Atomics` and the base64
//! and hex decoders while values' `valueOf`, getters, comparators,
//! callbacks and iteration resize, transfer (detach) or grow the
//! buffers. Each step is caught, and the script checks that whatever was
//! thrown is a `TypeError`, a `RangeError` or (a decoder's) a
//! `SyntaxError`, that a detached view reads as empty, that every index
//! below a view's length reads a number (a BigInt array's a BigInt) and
//! the one at it undefined,
//! that a data view reads its last byte and throws a RangeError past
//! it, and that no view reaches past its buffer. The run must never panic, and
//! the script must come out "ok" (or be stopped by its step budget). The
//! first byte sets how often the collector runs, as `silverjs_gc` does.

#![no_main]

use std::cell::Cell;
use std::fmt::Write;
use std::rc::Rc;

use libfuzzer_sys::fuzz_target;
use silverjs::agent::{Agent, AgentOptions, Outcome};

/// The interrupt hook's calls a run may take (each 10,000 steps).
const BUDGET: u32 = 8;

const STRIDES: [u32; 4] = [0, 1, 8, 64];

/// The script's own functions: the buffers and views, the actions that
/// change a buffer, hostile values that act when converted, and the
/// checks.
const PRELUDE: &str = r#"
var C = [Uint8Array, Int8Array, Uint16Array, Int16Array, Uint32Array, Int32Array,
  BigUint64Array, BigInt64Array, Uint8ClampedArray, Float32Array, Float64Array, Float16Array];
var bufs = [new ArrayBuffer(8, { maxByteLength: 16 }), new ArrayBuffer(8), new ArrayBuffer(12, { maxByteLength: 12 })];
var views = [new Uint8Array(bufs[0]), new Int16Array(bufs[0], 2, 2), new Float64Array(bufs[1]),
  new Uint32Array(bufs[2], 4), new Float16Array(bufs[0], 2), new BigInt64Array(bufs[1])];
var shared = [new SharedArrayBuffer(8, { maxByteLength: 16 }), new SharedArrayBuffer(4)];
var dvs = [new DataView(bufs[0]), new DataView(bufs[1], 2, 4), new DataView(shared[0], 1)];
var bad = [];
function kind(e) {
  if (!(e instanceof TypeError || e instanceof RangeError || e instanceof SyntaxError)) bad.push('threw ' + String(e));
}
function act(k) {
  var i = k % bufs.length, b = bufs[i];
  try {
    switch ((k >> 2) % 5) {
      case 0: if (b.resizable) b.resize(k % 17); break;
      case 1: bufs[i] = b.transfer(); break;
      case 2: if (b.resizable) b.resize(0); break;
      case 3: if (b.resizable) b.resize(16); break;
      case 4: if (k & 1) shared[k % 2].grow(k % 17); else bufs.push(new ArrayBuffer(k % 9, { maxByteLength: 16 })); break;
    }
  } catch (e) { kind(e); }
}
function H(k, v) { return { valueOf: function () { act(k); return v; }, toString: function () { act(k); return ',' } }; }
function HB(k, v) { return { valueOf: function () { act(k); return v; } }; }
function big(v) { return v instanceof BigInt64Array || v instanceof BigUint64Array; }
function V(a) { return views[a % (views.length || 1)]; }
function B(a) { return a & 64 ? shared[a % shared.length] : bufs[a % bufs.length]; }
function D(a) { return dvs[a % dvs.length]; }
function mkd(a, b) {
  var buf = B(a), d = b & 1 ? new DataView(buf, b % 5) : new DataView(buf, b % 3, b % 4);
  dvs.push(d);
  return d.byteLength;
}
function T(a) { return C[a % C.length]; }
function mk(a, b, c) {
  var t = T(a), buf = B(b), off = (c % 3) * t.BYTES_PER_ELEMENT;
  var v = c & 4 ? new t(buf, off) : new t(buf, off, c % 3);
  views.push(v);
  return v.length;
}
function like(k) {
  return { length: 3, get 0() { act(k); return 1; }, get 1() { return 2; }, get 2() { act(k + 1); return 3; } };
}
function check() {
  views.forEach(function (v) {
    var n = v.length;
    if (v.buffer.detached && (n !== 0 || v.byteLength !== 0 || v.byteOffset !== 0)) bad.push('detached view has elements');
    var want = big(v) ? 'bigint' : 'number';
    for (var i = 0; i < n; i++) if (typeof v[i] !== want) bad.push('no ' + want + ' at ' + i + ' of ' + n);
    if (v[n] !== undefined || v[-1] !== undefined || v['-0'] !== undefined) bad.push('past the end');
    if (n > 0 && n * v.BYTES_PER_ELEMENT + v.byteOffset > v.buffer.byteLength) bad.push('outside its buffer');
    if (Object.keys(v).length !== n) bad.push('keys');
  });
  dvs.forEach(function (d) {
    var n;
    try { n = d.byteLength; } catch (e) { if (!(e instanceof TypeError)) bad.push('data view threw ' + e); return; }
    if (d.byteOffset + n > d.buffer.byteLength) bad.push('data view outside its buffer');
    if (n > 0 && typeof d.getUint8(n - 1) !== 'number') bad.push('data view last byte');
    try { d.getUint8(n); bad.push('data view read past the end'); } catch (e) { if (!(e instanceof RangeError)) bad.push('data view past the end threw ' + e); }
  });
}
function op(f) {
  try { f(); } catch (e) { kind(e); }
}
"#;

/// The step `code` with bytes `a` and `k`, as a statement.
fn step(out: &mut String, code: u8, a: u8, k: u8) {
    let i = a % 20;
    let body = match code % 50 {
        0 => format!("V({a})[{i}]"),
        1 => format!("V({a})[{i}] = {k}"),
        2 => format!("V({a})[{i}] = H({k}, 3)"),
        3 => format!("mk({a}, {k}, {})", a ^ k),
        4 => format!("act({k})"),
        5 => format!("V({a}).fill(H({k}, 7), H({}, 1))", k.wrapping_add(4)),
        6 => format!("V({a}).set(V({k}), H({k}, 0))"),
        7 => format!("V({a}).set(like({k}), 0)"),
        8 => format!("V({a}).copyWithin(H({k}, 0), 1)"),
        9 => format!("V({a}).slice(H({k}, 1))"),
        10 => format!("V({a}).subarray(H({k}, 0))"),
        11 => format!("V({a}).map(function (x, j) {{ act({k}); return x + j; }})"),
        12 => format!("V({a}).sort(function (x, y) {{ act({k}); return x - y; }})"),
        13 => format!("V({a}).includes(undefined, H({k}, 0))"),
        14 => format!("V({a}).lastIndexOf(0, H({k}, 5))"),
        15 => format!("V({a}).join(H({k}, 0))"),
        16 => format!("V({a}).with(H({k}, 0), H({}, 9))", k.wrapping_add(8)),
        17 => format!("V({a}).at(H({k}, 1))"),
        18 => format!("V({a}).reduce(function (s, x) {{ act({k}); return s + x; }}, 0)"),
        19 => format!("V({a}).filter(function (x) {{ act({k}); return true; }})"),
        20 => format!("V({a}).toSorted(function (x, y) {{ act({k}); return y - x; }})"),
        21 => format!("new (T({k}))(V({a}))"),
        22 => format!("T({k}).from(V({a}), function (x) {{ act({k}); return x; }})"),
        23 => format!("T({a}).from(like({k}))"),
        24 => format!(
            "Object.defineProperty(V({a}), '{}', {{ value: H({k}, 3) }})",
            i % 4
        ),
        25 => format!("Reflect.set(V({a}), '{}', H({k}, 2))", i % 4),
        26 => format!("for (var x of V({a})) act({k})"),
        27 => format!("B({a}).slice(H({k}, 1))"),
        28 => format!("V({a}).forEach(function () {{ act({k}); }})"),
        29 => format!("V({a}).entries().next(act({k}))"),
        30 => format!("V({a}).indexOf(H({k}, 0))"),
        31 => format!("mkd({a}, {k})"),
        32 => format!(
            "D({a}).setFloat64(H({k}, 0), H({}, 1.5), {})",
            k.wrapping_add(1),
            k & 1
        ),
        33 => format!("D({a}).getInt32(H({k}, 1), true)"),
        34 => format!("D({a}).setUint16({i}, {k})"),
        35 => format!(
            "Atomics.add(V({a}), H({k}, 0), H({}, 1))",
            k.wrapping_add(2)
        ),
        36 => format!("Atomics.compareExchange(V({a}), {i}, H({k}, 0), 7)"),
        37 => format!("Atomics.store(V({a}), 0, H({k}, 3)); Atomics.load(V({a}), 0)"),
        38 => format!(
            "var w = new Int32Array(shared[{}]); Atomics.waitAsync(w, 0, 0); Atomics.notify(w, 0, H({k}, 1))",
            a % 2
        ),
        39 => format!(
            "V({a}).setFromBase64('AQIDBAUGBwgJ', {{ get alphabet() {{ act({k}); return 'base64'; }} }})"
        ),
        40 => format!("V({a}).setFromHex('0102030405060708')"),
        41 => format!("V({a}).toBase64({{ get omitPadding() {{ act({k}); return true; }} }})"),
        42 => format!("Uint8Array.fromBase64(new Uint8Array(B({a})).toBase64()).length"),
        43 => format!("V({a})[{i}] = HB({k}, -{k}n)"),
        44 => format!(
            "D({a}).setBigInt64(H({k}, 0), HB({}, -{k}n), {})",
            k.wrapping_add(1),
            k & 1
        ),
        45 => format!("D({a}).getBigUint64(H({k}, 0), true)"),
        46 => format!("V({a}).fill(HB({k}, 5n), H({}, 1))", k.wrapping_add(4)),
        47 => format!(
            "Atomics.add(V({a}), H({k}, 0), HB({}, 1n))",
            k.wrapping_add(2)
        ),
        48 => format!(
            "V({a}).sort(function (x, y) {{ act({k}); return x < y ? -1 : x > y ? 1 : 0; }})"
        ),
        _ => "check()".to_owned(),
    };
    let _ = writeln!(out, "op(function () {{ {body}; }});");
}

/// The script the bytes make.
fn script(bytes: &[u8]) -> String {
    let mut out = String::from(PRELUDE);
    for (n, s) in bytes.chunks(3).take(200).enumerate() {
        let (code, a, k) = (
            s[0],
            s.get(1).copied().unwrap_or(0),
            s.get(2).copied().unwrap_or(0),
        );
        step(&mut out, code, a, k);
        if n % 8 == 7 {
            out.push_str("check();\n");
        }
    }
    out.push_str("check();\nbad.length ? 'bad ' + bad.join('; ') : 'ok'\n");
    out
}

fn run(source: &str, stride: u32) -> String {
    let mut agent = Agent::new(AgentOptions {
        heap_limit: 16 << 20,
        shared_memory: true,
    });
    agent.set_stress_every(stride);
    let calls = Rc::new(Cell::new(0u32));
    let hook = Rc::clone(&calls);
    agent.set_interrupt(Some(Box::new(move || {
        hook.set(hook.get() + 1);
        hook.get() > BUDGET
    })));
    agent.scope(|scope| match scope.run_script(source) {
        Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
        Ok(Outcome::Threw(v)) => format!("threw {}", scope.to_text(v).unwrap_or_default()),
        Ok(Outcome::Terminated) => "terminated".to_owned(),
        Err(e) => format!("did not compile: {e}"),
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&first, rest)) = data.split_first() else {
        return;
    };
    let stride = STRIDES[usize::from(first) % STRIDES.len()];
    let source = script(rest);
    let outcome = run(&source, stride);
    assert!(
        outcome == "ok" || outcome == "terminated",
        "{outcome}\n--- script ---\n{source}"
    );
});
