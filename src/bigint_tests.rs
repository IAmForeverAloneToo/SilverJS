//! BigInt (J4.3b): the primitive, its operators, conversions and
//! constructor, as the reference browser gives them (the numbers probe,
//! Chrome 158).

use crate::agent::{Agent, AgentOptions, Outcome};

fn run(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    agent.scope(|scope| match scope.run_script_named(src, "u.js") {
        Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
        Ok(Outcome::Threw(e)) => format!("throw {}", scope.to_text(e).unwrap_or_default()),
        Ok(Outcome::Terminated) => "terminated".into(),
        Err(e) => format!("compile error: {e}"),
    })
}

#[test]
fn bigints_compare_convert_and_key_collections_by_value() {
    assert_eq!(
        run(
            "var m = new Map([[1n, 'a']]); var s = new Set([2n ** 70n]); \
             var x = 5n; x++; x += 1n; var y = -x; \
             [m.get(1n), s.has(2n ** 70n), s.has(2n ** 71n), x, y, ~y, 10n > '9', '10' < 9n, 1n == '0x1', \
              2n ** 53n + 1n > 2 ** 53, 2n ** 53n + 1n == 2 ** 53, 0n == '', 1n < 'x', 1n >= 'x', \
              typeof Object(1n), Object(1n) == 1n, Object.prototype.toString.call(1n), \
              (-(2n ** 64n)).toString(36), BigInt.asUintN(64, -1n), BigInt.asIntN(3, 4n), \
              -7n >> 1n, -7n << -1n, 7n >> 100n, -7n >> 100n, 0n ** 0n, (-2n) ** 3n, (-1n) ** 2n, 1n < 1].join()"
        ),
        "a,true,false,7,-7,6,true,false,true,true,false,true,false,false,object,true,[object BigInt],\
         -3w5e11264sgsg,18446744073709551615,-4,-4,-4,0,-1,1,-8,1,false"
    );
}

#[test]
fn large_bigints_multiply_and_divide_exactly() {
    assert_eq!(
        run("var a = 3n ** 2000n, b = 7n ** 1500n, p = a * b; \
             [p / b === a, p % b, (p + 12345n) % a, String(a).length, a.toString(16).length].join()"),
        "true,0,12345,955,793"
    );
}

#[test]
fn bigints_survive_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    let got = agent.scope(|scope| {
        match scope.run_script(
            "var keep = []; for (var i = 0n; i < 30n; i++) keep.push(i ** 20n + 1n); \
             var m = new Map(); keep.forEach(function (k, j) { m.set(k, j); }); \
             [keep.reduce(function (s, k) { return s + k; }, 0n) % 1000003n, m.get(29n ** 20n + 1n)].join()",
        ) {
            Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
            _ => "failed".into(),
        }
    });
    assert_eq!(got, "16536,29");
}

#[test]
fn bigint_elements_survive_collection() {
    // Every element read makes a BigInt: the kept ones of filter, the
    // pair a comparator is given, reduce's accumulator and map's results
    // must stay rooted while the next is made (reduce's first element,
    // with no initial value, among them), and an element's value while
    // its descriptor object is made.
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    let got = agent.scope(|scope| {
        match scope.run_script(
            "var a = new BigInt64Array(64); for (var i = 0; i < 64; i++) a[i] = BigInt(i) * 3n - 90n; \
             var f = a.filter(function (x) { return x % 2n === 0n; }); \
             var s = a.toSorted(function (x, y) { var d = y - x; return d > 0n ? 1 : d < 0n ? -1 : 0; }); \
             var r = a.reduce(function (acc, x) { return acc + x * x; }, 0n); \
             var r2 = a.reduce(function (acc, x) { return acc + x; }); \
             var m = a.map(function (x) { return -x; }); \
             var u = new BigUint64Array(a.buffer); \
             var d = Object.getOwnPropertyDescriptor(a, '5').value; \
             [f.length, f[0], s[0], s[63], r, r2, d, m[0], u[0], Atomics.add(a, 0, 1n), a.join().length].join()",
        ) {
            Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
            _ => "failed".into(),
        }
    });
    assert_eq!(
        got,
        "32,-90,99,-90,197856,288,-75,90,18446744073709551526,-90,214"
    );
}

#[test]
fn bigint_typed_arrays_keep_their_content_type() {
    // A species of the other content type is refused (section
    // 23.2.4.1); a BigInt element's descriptor and [[Get]] give BigInts;
    // a BigUint64Array finds only what is in its range.
    assert_eq!(
        run(
            "class S extends BigInt64Array { static get [Symbol.species]() { return Int8Array; } } \
             var e; try { new S(2).slice(0); } catch (x) { e = x.constructor.name; } \
             var u = BigUint64Array.of(2n ** 64n - 1n, 0n); \
             [e, Object.getOwnPropertyDescriptor(BigInt64Array.of(5n), '0').value, \
              u.includes(-1n), u.indexOf(2n ** 64n - 1n), u.lastIndexOf(0n), \
              Reflect.get(BigInt64Array.of(-7n), 0)].join()"
        ),
        "TypeError,5,false,0,1,-7"
    );
}

#[test]
fn the_interrupt_hook_stops_long_bigint_work() {
    use std::cell::Cell;
    use std::rc::Rc;
    // Each script does a few instructions and one long operation: only
    // the operation's own calls of the hook can stop it.
    for src in [
        "10n ** 10000000n",
        "(1n << 30000000n).toString()",
        "BigInt('7'.repeat(3000000))",
        "var x = 3n ** 600000n; x * x",
        "var x = 3n ** 600000n; x / 7n ** 300000n",
    ] {
        let mut agent = Agent::new(AgentOptions::default());
        let calls = Rc::new(Cell::new(0u32));
        let hook = Rc::clone(&calls);
        agent.set_interrupt(Some(Box::new(move || {
            hook.set(hook.get() + 1);
            hook.get() > 3
        })));
        let ended = agent.scope(|scope| match scope.run_script(src) {
            Ok(Outcome::Terminated) => "terminated".to_owned(),
            Ok(Outcome::Value(_)) => "finished".to_owned(),
            Ok(Outcome::Threw(e)) => format!("threw {}", scope.to_text(e).unwrap_or_default()),
            Err(e) => format!("did not compile: {e}"),
        });
        assert_eq!(ended, "terminated", "{src}");
        assert!(
            calls.get() > 3,
            "{src}: the hook was called {}",
            calls.get()
        );
    }
}

/// The bigtyped probe and the numbers probe's typed parts (J4.3c's).
const TYPED_PROBE: &str = "function keys(o) {\n  return Reflect.ownKeys(o).map(function (k) {\n    var d = Object.getOwnPropertyDescriptor(o, k);\n    var kind = 'get' in d ? 'accessor' : typeof d.value;\n    var flags = (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-');\n    var f = typeof d.value === 'function' ? d.value : d.get;\n    var extra = typeof f === 'function' ? ' len=' + f.length + ' name=' + f.name : '';\n    return String(k) + ' [' + kind + ' ' + flags + extra + ']';\n  });\n}\nvar out = { keys: {}, facts: [], errors: [] };\nfunction fact(label, fn) {\n  try { out.facts.push(label + ' ' + String(fn())); }\n  catch (e) { out.facts.push(label + ' threw ' + e.constructor.name + ': ' + e.message); }\n}\nfunction err(label, fn) {\n  try { out.errors.push(label + ' no error: ' + String(fn())); }\n  catch (e) { out.errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }\n}\nout.keys.BigUint64Array = keys(BigUint64Array);\nout.keys['BigUint64Array.prototype'] = keys(BigUint64Array.prototype);\nout.keys.DataViewBig = keys(DataView.prototype).filter(function (k) { return String(k).indexOf('Big') >= 0; });\nfact('wrap', function () { var a = new BigInt64Array(2); a[0] = 2n ** 64n + 5n; a[1] = -(2n ** 63n) - 1n; var u = new BigUint64Array(1); u[0] = -1n; return [a.join(), u.join(), a[5], typeof a[0]].join(); });\nfact('convert', function () { var a = new BigInt64Array(3); a[0] = true; a[1] = '0x10'; a[2] = { valueOf: function () { return 7n; } }; return a.join(); });\nfact('sort', function () { var u = BigUint64Array.of(2n ** 64n - 1n, 0n, 5n, 2n ** 63n); var s = BigInt64Array.of(-1n, 3n, -(2n ** 63n), 0n); return [u.sort().join(), s.sort().join(), s.toSorted(function (x, y) { return x < y ? 1 : -1; }).join()].join(' | '); });\nfact('search', function () { var a = BigInt64Array.of(0n, 1n, -1n); return [a.includes(0n), a.includes(0), a.indexOf(1n), a.indexOf(1), a.lastIndexOf(-1n), a.at(-1), a.findLast(function (x) { return x > 0n; })].join(); });\nfact('methods', function () { var a = BigInt64Array.of(1n, 2n, 3n); return [a.map(function (x) { return x * 2n; }).join(), a.filter(function (x) { return x & 1n; }).join(), a.reduce(function (s, x) { return s + x; }), a.slice(1).join(), a.subarray(1).join(), a.fill(9n, 2).join(), a.with(0, 4n).join(), a.toReversed().join(), Array.from(a.entries()).join(';'), a.toString(), a.toLocaleString()].join(' | '); });\nfact('copy', function () { var a = BigInt64Array.of(-1n, 2n); var b = new BigInt64Array(a); var u = new BigUint64Array(a); var c = new BigInt64Array(4); c.set(u, 1); return [b.join(), u.join(), c.join(), BigUint64Array.from([1n, 2n]).join(), BigInt64Array.from({ length: 2, 0: 5n, 1: '6' }).join()].join(' | '); });\nfact('dataview', function () { var d = new DataView(new ArrayBuffer(16)); d.setBigUint64(0, 2n ** 64n - 2n); d.setBigInt64(8, -3n, true); return [d.getBigInt64(0), d.getBigUint64(0, true), d.getBigInt64(8, true), d.getBigUint64(8), d.getUint8(0), d.getUint8(8), d.setBigInt64(0, 1n), DataView.prototype.getBigInt64.length, DataView.prototype.setBigUint64.length].join(); });\nfact('atomics', function () { var a = new BigInt64Array(new ArrayBuffer(16)); var u = new BigUint64Array(a.buffer); return [Atomics.store(a, 0, 5n), Atomics.add(a, 0, 2n), Atomics.sub(a, 0, 10n), a[0], Atomics.and(u, 0, 0xffn), Atomics.or(a, 1, 6n), Atomics.xor(a, 1, 3n), Atomics.exchange(a, 1, -1n), Atomics.compareExchange(u, 1, 2n ** 64n - 1n, 9n), Atomics.compareExchange(a, 1, -1n, 8n), Atomics.load(a, 1), Atomics.store(u, 0, -1n), u[0], Atomics.isLockFree(8)].join(); });\nfact('json', function () { try { return JSON.stringify(BigInt64Array.of(1n)); } catch (e) { return e.message; } });\nerr('ctor from Int8Array', function () { new BigInt64Array(new Int8Array(1)); });\nerr('ctor Int8Array from BigInt64Array', function () { new Int8Array(new BigInt64Array(1)); });\nerr('ctor from number array', function () { new BigUint64Array([1n, 2]); });\nerr('ctor from string', function () { new BigInt64Array(['x']); });\nerr('ctor from symbol', function () { new BigInt64Array([Symbol('s')]); });\nerr('from mapped number', function () { BigInt64Array.from([1n], function () { return 1; }); });\nerr('fill number', function () { new BigInt64Array(1).fill(1); });\nerr('fill undefined', function () { new BigInt64Array(1).fill(); });\nerr('set from array', function () { new BigInt64Array(1).set([1]); });\nerr('set number from bigint array', function () { new Float64Array(1).set(new BigUint64Array(1)); });\nerr('with number', function () { new BigInt64Array(1).with(0, 1); });\nerr('map returns number', function () { BigInt64Array.of(1n).map(function () { return 1; }); });\nerr('define number', function () { Object.defineProperty(new BigInt64Array(1), '0', { value: 1 }); });\nerr('setBigUint64 string', function () { new DataView(new ArrayBuffer(8)).setBigUint64(0, 'x'); });\nerr('getBigInt64 range', function () { new DataView(new ArrayBuffer(4)).getBigInt64(0); });\nerr('setBigInt64 range', function () { new DataView(new ArrayBuffer(4)).setBigInt64(0, 1n); });\nerr('getBigInt64 receiver', function () { DataView.prototype.getBigInt64.call({}, 0); });\nerr('atomics number value', function () { Atomics.store(new BigInt64Array(1), 0, 1); });\nerr('atomics bigint on Int32Array', function () { Atomics.store(new Int32Array(1), 0, 1n); });\nerr('atomics wait bigint non-shared', function () { Atomics.wait(new BigInt64Array(1), 0, 0n); });\nerr('atomics wait number on bigint', function () { Atomics.wait(new BigInt64Array(new SharedArrayBuffer(8)), 0, 0); });\nerr('atomics on BigUint64Array wait', function () { Atomics.wait(new BigUint64Array(new SharedArrayBuffer(8)), 0, 0n); });\nerr('sort comparator number result', function () { return BigInt64Array.of(2n, 1n).sort(function (a, b) { return a - b; }).join(); });\nerr('unary plus element', function () { return +new BigInt64Array(1)[0]; });\nerr('length too large', function () { new BigInt64Array(2 ** 32); });\nerr('offset misaligned', function () { new BigInt64Array(new ArrayBuffer(16), 4); });\nerr('length misaligned', function () { new BigUint64Array(new ArrayBuffer(12)); });\nfact('bigint typed', function () { var a = new BigInt64Array([1n, -1n]); var u = new BigUint64Array(a.buffer); var d = new DataView(a.buffer); return [a.join(), u.join(), d.getBigInt64(8, true), d.getBigUint64(0), (d.setBigInt64(0, -2n), a[0]), BigInt64Array.BYTES_PER_ELEMENT, Atomics.add(a, 0, 3n), a[0]].join(); });\nerr('typed set number', function () { new BigInt64Array(1)[0] = 1; });\nerr('typed from numbers', function () { new BigInt64Array([1]); });\nerr('typed set bigint into number array', function () { new Int8Array(1)[0] = 1n; });\nerr('typed set mixed', function () { new Int8Array(1).set(new BigInt64Array(1)); });\nerr('dv setBigInt64 number', function () { new DataView(new ArrayBuffer(8)).setBigInt64(0, 1); });\n[out.keys['BigUint64Array'].join(', '), out.keys['BigUint64Array.prototype'].join(', '), out.keys['DataViewBig'].join(', ')].concat(out.facts, out.errors).join('\\n')";

#[test]
fn bigint_typed_arrays_read_as_the_reference_browser_gives_them() {
    assert_eq!(
        run(TYPED_PROBE),
        "length [number --c], name [string --c], prototype [object ---], BYTES_PER_ELEMENT [number ---]\nconstructor [function w-c len=3 name=BigUint64Array], BYTES_PER_ELEMENT [number ---]\ngetBigInt64 [function w-c len=1 name=getBigInt64], setBigInt64 [function w-c len=2 name=setBigInt64], getBigUint64 [function w-c len=1 name=getBigUint64], setBigUint64 [function w-c len=2 name=setBigUint64]\nwrap 5,9223372036854775807,18446744073709551615,,bigint\nconvert 1,16,7\nsort 0,5,9223372036854775808,18446744073709551615 | -9223372036854775808,-1,0,3 | 3,0,-1,-9223372036854775808\nsearch true,false,1,-1,2,-1,1\nmethods 2,4,6 | 1,3 | 6 | 2,3 | 2,3 | 1,2,9 | 4,2,9 | 9,2,1 | 0,1;1,2;2,9 | 1,2,9 | 1,2,9\ncopy -1,2 | 18446744073709551615,2 | 0,-1,2,0 | 1,2 | 5,6\ndataview -2,18374686479671623679,-3,18302628885633695743,255,253,,1,2\natomics 5,5,7,-3,18446744073709551613,0,6,5,18446744073709551615,9,9,-1,18446744073709551615,true\njson Do not know how to serialize a BigInt\nbigint typed 1,-1,1,18446744073709551615,-1,72057594037927936,-72057594037927937,8,-72057594037927937,-72057594037927934\nctor from Int8Array: TypeError: Cannot mix BigInt and other types, use explicit conversions\nctor Int8Array from BigInt64Array: TypeError: Cannot mix BigInt and other types, use explicit conversions\nctor from number array: TypeError: Cannot convert 2 to a BigInt\nctor from string: SyntaxError: Cannot convert x to a BigInt\nctor from symbol: TypeError: Cannot convert Symbol(s) to a BigInt\nfrom mapped number: TypeError: Cannot convert 1 to a BigInt\nfill number: TypeError: Cannot convert 1 to a BigInt\nfill undefined: TypeError: Cannot convert undefined to a BigInt\nset from array: TypeError: Cannot convert 1 to a BigInt\nset number from bigint array: TypeError: Cannot mix BigInt and other types, use explicit conversions\nwith number: TypeError: Cannot convert 1 to a BigInt\nmap returns number: TypeError: Cannot convert 1 to a BigInt\ndefine number: TypeError: Cannot convert 1 to a BigInt\nsetBigUint64 string: SyntaxError: Cannot convert x to a BigInt\ngetBigInt64 range: RangeError: Offset is outside the bounds of the DataView\nsetBigInt64 range: RangeError: Offset is outside the bounds of the DataView\ngetBigInt64 receiver: TypeError: Method DataView.prototype.getBigInt64 called on incompatible receiver #<Object>\natomics number value: TypeError: Cannot convert 1 to a BigInt\natomics bigint on Int32Array: TypeError: Cannot convert a BigInt value to a number\natomics wait bigint non-shared: TypeError: [object BigInt64Array] is not a shared typed array.\natomics wait number on bigint: ReferenceError: SharedArrayBuffer is not defined\natomics on BigUint64Array wait: ReferenceError: SharedArrayBuffer is not defined\nsort comparator number result: TypeError: Cannot convert a BigInt value to a number\nunary plus element: TypeError: Cannot convert a BigInt value to a number\nlength too large: RangeError: Invalid typed array length: 4294967296\noffset misaligned: RangeError: start offset of BigInt64Array should be a multiple of 8\nlength misaligned: RangeError: byte length of BigUint64Array should be a multiple of 8\ntyped set number: TypeError: Cannot convert 1 to a BigInt\ntyped from numbers: TypeError: Cannot convert 1 to a BigInt\ntyped set bigint into number array: TypeError: Cannot convert a BigInt value to a number\ntyped set mixed: TypeError: Cannot mix BigInt and other types, use explicit conversions\ndv setBigInt64 number: TypeError: Cannot convert 1 to a BigInt"
    );
}

/// The numbers probe's BigInt parts (J4.3b's).
const PROBE: &str = "function keys(o) {\n  return Reflect.ownKeys(o).map(function (k) {\n    var d = Object.getOwnPropertyDescriptor(o, k);\n    var kind = 'get' in d ? 'accessor' : typeof d.value;\n    var flags = (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-');\n    var f = typeof d.value === 'function' ? d.value : d.get;\n    var extra = typeof f === 'function' ? ' len=' + f.length + ' name=' + f.name : '';\n    if (d.set) extra += ' set=' + d.set.name;\n    return String(k) + ' [' + kind + ' ' + flags + extra + ']';\n  });\n}\nvar out = { keys: {}, facts: [], errors: [] };\nfunction fact(label, fn) {\n  try { out.facts.push(label + ' ' + String(fn())); }\n  catch (e) { out.facts.push(label + ' threw ' + e.constructor.name + ': ' + e.message); }\n}\nfunction err(label, fn) {\n  try { var v = fn(); out.errors.push(label + ': no throw ' + String(v)); }\n  catch (e) { out.errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }\n}\nout.keys.BigInt = keys(BigInt); out.keys['BigInt.prototype'] = keys(BigInt.prototype);\nerr('stringify bigint', function () { JSON.stringify({ a: 1n }); });\nfact('bigint basics', function () { return [typeof 1n, 1n + 2n, 2n ** 64n, -(2n ** 63n), 7n / 2n, -7n / 2n, -7n % 2n, 5n & -3n, 5n | -3n, 5n ^ -3n, ~5n, 1n << 70n, -9n >> 1n, 1n << -1n, 0n == -0n, 1n == 1, 1n === 1, 2n > 1, '10' > 9n, 1n < 1.5, 0n ? 'y' : 'n', String(10n ** 30n)].join(); });\nfact('bigint conversions', function () { return [BigInt(10), BigInt('0x1f'), BigInt(' 12 '), BigInt(''), BigInt(true), BigInt('-12'), BigInt('0b101'), BigInt(Number.MAX_SAFE_INTEGER) + 2n, BigInt.asIntN(8, 255n), BigInt.asUintN(8, -1n), BigInt.asIntN(64, 2n ** 63n), BigInt.asUintN(0, 5n), (255n).toString(16), (-255n).toString(2), (123n).toLocaleString(), Object(1n) + 1n, typeof Object(1n), Number(2n ** 64n), parseInt('12n'), 1n.valueOf()].join(); });\nfact('bigint mixed', function () { return [1n == '1', 1n == true, 2n == 2.5, 1n < '2', [1n, 2n].indexOf(1n), [1n].includes(1n), new Set([1n, 1n]).size, Object.is(0n, -0n), JSON.stringify(Object.getOwnPropertyNames(BigInt.prototype)), 10n.toString(36)].join(); });\nerr('BigInt(1.5)', function () { BigInt(1.5); });\nerr('BigInt(NaN)', function () { BigInt(NaN); });\nerr('BigInt(x)', function () { BigInt('x'); });\nerr('BigInt(1n.5)', function () { BigInt('1.5'); });\nerr('BigInt(undefined)', function () { BigInt(undefined); });\nerr('BigInt(null)', function () { BigInt(null); });\nerr('BigInt(Symbol)', function () { BigInt(Symbol('s')); });\nerr('new BigInt', function () { new BigInt(1); });\nerr('mix add', function () { 1n + 1; });\nerr('mix compare ok', function () { return 1n < 2; });\nerr('unary plus', function () { +1n; });\nerr('Number ok', function () { return Number(1n); });\nerr('Math.max', function () { Math.max(1n); });\nerr('ushr', function () { 1n >>> 0n; });\nerr('div zero', function () { 1n / 0n; });\nerr('mod zero', function () { 1n % 0n; });\nerr('negative exponent', function () { 2n ** -1n; });\nerr('too big', function () { 2n ** (2n ** 32n); });\nerr('shift too big', function () { 1n << (2n ** 40n); });\nerr('toString radix', function () { 1n.toString(37); });\nerr('asIntN bits', function () { BigInt.asIntN(-1, 1n); });\nerr('asIntN value', function () { BigInt.asIntN(8, 1); });\nerr('valueOf receiver', function () { BigInt.prototype.valueOf.call(1); });\nerr('toString receiver', function () { BigInt.prototype.toString.call({}); });\nerr('isNaN bigint', function () { isNaN(1n); });\nerr('bigint in template', function () { return `${1n}`; });\nerr('bigint symbol desc', function () { return Symbol(1n).description; });\nerr('bigint key', function () { var o = {}; o[1n] = 2; return Object.keys(o).join(); });\nerr('sumPrecise bigint', function () { Math.sumPrecise([1n]); });\n[out.keys.BigInt.join(', '), out.keys['BigInt.prototype'].join(', ')].concat(out.facts, out.errors).join('\\n')";

#[test]
fn bigints_read_as_the_reference_browser_gives_them() {
    assert_eq!(
        run(PROBE),
        "length [number --c], name [string --c], prototype [object ---], asUintN [function w-c len=2 name=asUintN], asIntN [function w-c len=2 name=asIntN]\nconstructor [function w-c len=1 name=BigInt], toLocaleString [function w-c len=0 name=toLocaleString], toString [function w-c len=0 name=toString], valueOf [function w-c len=0 name=valueOf], Symbol(Symbol.toStringTag) [string --c]\nbigint basics bigint,3,18446744073709551616,-9223372036854775808,3,-3,-1,5,-3,-8,-6,1180591620717411303424,-5,0,true,true,false,true,true,true,n,1000000000000000000000000000000\nbigint conversions 10,31,12,0,1,-12,5,9007199254740993,-1,255,-9223372036854775808,0,ff,-11111111,123,2,object,18446744073709552000,12,1\nbigint mixed true,true,false,true,0,true,1,true,[\"constructor\",\"toLocaleString\",\"toString\",\"valueOf\"],a\nstringify bigint: TypeError: Do not know how to serialize a BigInt\nBigInt(1.5): RangeError: The number 1.5 cannot be converted to a BigInt because it is not an integer\nBigInt(NaN): RangeError: The number NaN cannot be converted to a BigInt because it is not an integer\nBigInt(x): SyntaxError: Cannot convert x to a BigInt\nBigInt(1n.5): SyntaxError: Cannot convert 1.5 to a BigInt\nBigInt(undefined): TypeError: Cannot convert undefined to a BigInt\nBigInt(null): TypeError: Cannot convert null to a BigInt\nBigInt(Symbol): TypeError: Cannot convert Symbol(s) to a BigInt\nnew BigInt: TypeError: BigInt is not a constructor\nmix add: TypeError: Cannot mix BigInt and other types, use explicit conversions\nmix compare ok: no throw true\nunary plus: TypeError: Cannot convert a BigInt value to a number\nNumber ok: no throw 1\nMath.max: TypeError: Cannot convert a BigInt value to a number\nushr: TypeError: BigInts have no unsigned right shift, use >> instead\ndiv zero: RangeError: Division by zero\nmod zero: RangeError: Division by zero\nnegative exponent: RangeError: Exponent must be positive\ntoo big: RangeError: Maximum BigInt size exceeded\nshift too big: RangeError: Maximum BigInt size exceeded\ntoString radix: RangeError: toString() radix argument must be between 2 and 36\nasIntN bits: RangeError: Invalid value -1: not (convertible to) a safe integer\nasIntN value: TypeError: Cannot convert 1 to a BigInt\nvalueOf receiver: TypeError: BigInt.prototype.valueOf requires that 'this' be a BigInt\ntoString receiver: TypeError: BigInt.prototype.toString requires that 'this' be a BigInt\nisNaN bigint: TypeError: Cannot convert a BigInt value to a number\nbigint in template: no throw 1\nbigint symbol desc: no throw 1\nbigint key: no throw 1\nsumPrecise bigint: TypeError: Type of 'Iterator value' must be 'number', found 'bigint'"
    );
}
