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

/// The numbers probe's BigInt parts (J4.3b's).
const PROBE: &str = "function keys(o) {\n  return Reflect.ownKeys(o).map(function (k) {\n    var d = Object.getOwnPropertyDescriptor(o, k);\n    var kind = 'get' in d ? 'accessor' : typeof d.value;\n    var flags = (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-');\n    var f = typeof d.value === 'function' ? d.value : d.get;\n    var extra = typeof f === 'function' ? ' len=' + f.length + ' name=' + f.name : '';\n    if (d.set) extra += ' set=' + d.set.name;\n    return String(k) + ' [' + kind + ' ' + flags + extra + ']';\n  });\n}\nvar out = { keys: {}, facts: [], errors: [] };\nfunction fact(label, fn) {\n  try { out.facts.push(label + ' ' + String(fn())); }\n  catch (e) { out.facts.push(label + ' threw ' + e.constructor.name + ': ' + e.message); }\n}\nfunction err(label, fn) {\n  try { var v = fn(); out.errors.push(label + ': no throw ' + String(v)); }\n  catch (e) { out.errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }\n}\nout.keys.BigInt = keys(BigInt); out.keys['BigInt.prototype'] = keys(BigInt.prototype);\nerr('stringify bigint', function () { JSON.stringify({ a: 1n }); });\nfact('bigint basics', function () { return [typeof 1n, 1n + 2n, 2n ** 64n, -(2n ** 63n), 7n / 2n, -7n / 2n, -7n % 2n, 5n & -3n, 5n | -3n, 5n ^ -3n, ~5n, 1n << 70n, -9n >> 1n, 1n << -1n, 0n == -0n, 1n == 1, 1n === 1, 2n > 1, '10' > 9n, 1n < 1.5, 0n ? 'y' : 'n', String(10n ** 30n)].join(); });\nfact('bigint conversions', function () { return [BigInt(10), BigInt('0x1f'), BigInt(' 12 '), BigInt(''), BigInt(true), BigInt('-12'), BigInt('0b101'), BigInt(Number.MAX_SAFE_INTEGER) + 2n, BigInt.asIntN(8, 255n), BigInt.asUintN(8, -1n), BigInt.asIntN(64, 2n ** 63n), BigInt.asUintN(0, 5n), (255n).toString(16), (-255n).toString(2), (123n).toLocaleString(), Object(1n) + 1n, typeof Object(1n), Number(2n ** 64n), parseInt('12n'), 1n.valueOf()].join(); });\nfact('bigint mixed', function () { return [1n == '1', 1n == true, 2n == 2.5, 1n < '2', [1n, 2n].indexOf(1n), [1n].includes(1n), new Set([1n, 1n]).size, Object.is(0n, -0n), JSON.stringify(Object.getOwnPropertyNames(BigInt.prototype)), 10n.toString(36)].join(); });\nerr('BigInt(1.5)', function () { BigInt(1.5); });\nerr('BigInt(NaN)', function () { BigInt(NaN); });\nerr('BigInt(x)', function () { BigInt('x'); });\nerr('BigInt(1n.5)', function () { BigInt('1.5'); });\nerr('BigInt(undefined)', function () { BigInt(undefined); });\nerr('BigInt(null)', function () { BigInt(null); });\nerr('BigInt(Symbol)', function () { BigInt(Symbol('s')); });\nerr('new BigInt', function () { new BigInt(1); });\nerr('mix add', function () { 1n + 1; });\nerr('mix compare ok', function () { return 1n < 2; });\nerr('unary plus', function () { +1n; });\nerr('Number ok', function () { return Number(1n); });\nerr('Math.max', function () { Math.max(1n); });\nerr('ushr', function () { 1n >>> 0n; });\nerr('div zero', function () { 1n / 0n; });\nerr('mod zero', function () { 1n % 0n; });\nerr('negative exponent', function () { 2n ** -1n; });\nerr('too big', function () { 2n ** (2n ** 32n); });\nerr('shift too big', function () { 1n << (2n ** 40n); });\nerr('toString radix', function () { 1n.toString(37); });\nerr('asIntN bits', function () { BigInt.asIntN(-1, 1n); });\nerr('asIntN value', function () { BigInt.asIntN(8, 1); });\nerr('valueOf receiver', function () { BigInt.prototype.valueOf.call(1); });\nerr('toString receiver', function () { BigInt.prototype.toString.call({}); });\nerr('isNaN bigint', function () { isNaN(1n); });\nerr('bigint in template', function () { return `${1n}`; });\nerr('bigint symbol desc', function () { return Symbol(1n).description; });\nerr('bigint key', function () { var o = {}; o[1n] = 2; return Object.keys(o).join(); });\nerr('sumPrecise bigint', function () { Math.sumPrecise([1n]); });\n[out.keys.BigInt.join(', '), out.keys['BigInt.prototype'].join(', ')].concat(out.facts, out.errors).join('\\n')";

#[test]
fn bigints_read_as_the_reference_browser_gives_them() {
    assert_eq!(
        run(PROBE),
        "length [number --c], name [string --c], prototype [object ---], asUintN [function w-c len=2 name=asUintN], asIntN [function w-c len=2 name=asIntN]\nconstructor [function w-c len=1 name=BigInt], toLocaleString [function w-c len=0 name=toLocaleString], toString [function w-c len=0 name=toString], valueOf [function w-c len=0 name=valueOf], Symbol(Symbol.toStringTag) [string --c]\nbigint basics bigint,3,18446744073709551616,-9223372036854775808,3,-3,-1,5,-3,-8,-6,1180591620717411303424,-5,0,true,true,false,true,true,true,n,1000000000000000000000000000000\nbigint conversions 10,31,12,0,1,-12,5,9007199254740993,-1,255,-9223372036854775808,0,ff,-11111111,123,2,object,18446744073709552000,12,1\nbigint mixed true,true,false,true,0,true,1,true,[\"constructor\",\"toLocaleString\",\"toString\",\"valueOf\"],a\nstringify bigint: TypeError: Do not know how to serialize a BigInt\nBigInt(1.5): RangeError: The number 1.5 cannot be converted to a BigInt because it is not an integer\nBigInt(NaN): RangeError: The number NaN cannot be converted to a BigInt because it is not an integer\nBigInt(x): SyntaxError: Cannot convert x to a BigInt\nBigInt(1n.5): SyntaxError: Cannot convert 1.5 to a BigInt\nBigInt(undefined): TypeError: Cannot convert undefined to a BigInt\nBigInt(null): TypeError: Cannot convert null to a BigInt\nBigInt(Symbol): TypeError: Cannot convert Symbol(s) to a BigInt\nnew BigInt: TypeError: BigInt is not a constructor\nmix add: TypeError: Cannot mix BigInt and other types, use explicit conversions\nmix compare ok: no throw true\nunary plus: TypeError: Cannot convert a BigInt value to a number\nNumber ok: no throw 1\nMath.max: TypeError: Cannot convert a BigInt value to a number\nushr: TypeError: BigInts have no unsigned right shift, use >> instead\ndiv zero: RangeError: Division by zero\nmod zero: RangeError: Division by zero\nnegative exponent: RangeError: Exponent must be positive\ntoo big: RangeError: Maximum BigInt size exceeded\nshift too big: RangeError: Maximum BigInt size exceeded\ntoString radix: RangeError: toString() radix argument must be between 2 and 36\nasIntN bits: RangeError: Invalid value -1: not (convertible to) a safe integer\nasIntN value: TypeError: Cannot convert 1 to a BigInt\nvalueOf receiver: TypeError: BigInt.prototype.valueOf requires that 'this' be a BigInt\ntoString receiver: TypeError: BigInt.prototype.toString requires that 'this' be a BigInt\nisNaN bigint: TypeError: Cannot convert a BigInt value to a number\nbigint in template: no throw 1\nbigint symbol desc: no throw 1\nbigint key: no throw 1\nsumPrecise bigint: TypeError: Type of 'Iterator value' must be 'number', found 'bigint'"
    );
}
