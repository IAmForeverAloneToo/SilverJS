//! JSON (J4.3a): `parse` with its reviver and source text, `stringify`
//! with replacers, gaps and cycles, `rawJSON`, and `Math.sumPrecise`, as
//! the reference browser gives them (the numbers probe, Chrome 158).

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
fn deep_texts_and_objects_need_no_native_stack() {
    // Under gc-stress every allocation collects: a shallower text.
    let n: usize = if cfg!(feature = "gc-stress") {
        1_000
    } else {
        100_000
    };
    let want = format!(
        "{},{},true,{},[{{}},[{{}}],{{\"x\":{{}}}}]",
        n - 1,
        2 * n,
        6 * n + 2
    );
    assert_eq!(
        run(&format!(
            "var n = {n}; var s = '['.repeat(n) + ']'.repeat(n); var v = JSON.parse(s); var d = 0; \
             while (Array.isArray(v) && v.length) {{ v = v[0]; d++; }} \
             var o = {{}}; for (var i = 0; i < n; i++) o = {{ o: o }}; \
             [d, JSON.stringify(JSON.parse(s)).length, Array.isArray(JSON.parse(s, function (k, v) {{ return v; }})), \
              JSON.stringify(o).length, JSON.stringify((function () {{ var x = {{}}; return [x, [x], {{ x: x }}]; }})())].join()"
        )),
        want
    );
}

#[test]
fn values_survive_collection_while_parsed_revived_and_serialized() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    let got = agent.scope(|scope| {
        // A string the reviver replaces by an equal one keeps its source:
        // the parse record holds the first, collected or not.
        let src = "var seen = []; JSON.parse('[\"aaa\", \"bbb\", \"ccc\"]', function (k, v, c) { \
                     if (k === '0') { this[1] = ['b', 'b', 'b'].join(''); this[2] = ['c', 'c', 'c'].join(''); } \
                     for (var i = 0; i < 50; i++) [i, String(i)]; \
                     if (k !== '') seen.push(c.source); return v; }); \
                   var o = JSON.parse('{\"a\":[1,\"two\",{\"b\":\"three\"}],\"c\":\"four\"}', \
                   function (k, v) { return typeof v === 'string' ? v + '!' : v; }); \
                   seen.join() + '\\n' + JSON.stringify(o, function (k, v) { return k === 'c' ? [v, v] : v; }, 1)";
        match scope.run_script(src) {
            Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
            _ => "failed".into(),
        }
    });
    assert_eq!(
        got,
        "\"aaa\",\"bbb\",\"ccc\"\n{\n \"a\": [\n  1,\n  \"two!\",\n  {\n   \"b\": \"three!\"\n  }\n ],\n \"c\": [\n  \"four!\",\n  \"four!\"\n ]\n}"
    );
}

#[test]
fn the_reviver_sees_source_only_for_values_unchanged() {
    assert_eq!(
        run(
            "var log = []; JSON.parse('[1, 2, {\"x\": \"y\"}]', function (k, v, c) { \
               if (k === '0') { this[1] = 3; this[2] = { x: 'y' }; } \
               log.push(k + '=' + c.source); return v; }); \
             [log.join(' '), JSON.stringify({ a: { toJSON: function (k) { return k + '!'; } }, b: [{ toJSON: function (k) { return k; } }] }), \
              JSON.stringify('\\ud800\\udc00\\udc00').length, JSON.stringify(JSON.rawJSON('-0')), \
              JSON.stringify([undefined, function () {}, Symbol()]), JSON.stringify({ a: undefined, b: Symbol() })].join('|')"
        ),
        "0=1 1=undefined x=undefined 2=undefined =undefined|{\"a\":\"a!\",\"b\":[\"0\"]}|10|-0|[null,null,null]|{}"
    );
}

#[test]
fn sums_round_once_to_even_and_non_finite_numbers_serialize_as_null() {
    assert_eq!(
        run(
            "[Math.sumPrecise([1, 2 ** -53]), Math.sumPrecise([1, 2 ** -53, 2 ** -105]), \
              Math.sumPrecise([1 + 2 ** -52, 2 ** -53]), Math.sumPrecise([5e-324, 5e-324, -5e-324]), \
              Math.sumPrecise([-1e308, -1e308]), JSON.stringify([Infinity, -Infinity, NaN, -0])].join('|')"
        ),
        "1|1.0000000000000002|1.0000000000000004|5e-324|-Infinity|[null,null,null,0]"
    );
}

/// The numbers probe's JSON and `Math.sumPrecise` parts.
const PROBE: &str = "function keys(o) {\n  return Reflect.ownKeys(o).map(function (k) {\n    var d = Object.getOwnPropertyDescriptor(o, k);\n    var kind = 'get' in d ? 'accessor' : typeof d.value;\n    var flags = (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-');\n    var f = typeof d.value === 'function' ? d.value : d.get;\n    var extra = typeof f === 'function' ? ' len=' + f.length + ' name=' + f.name : '';\n    if (d.set) extra += ' set=' + d.set.name;\n    return String(k) + ' [' + kind + ' ' + flags + extra + ']';\n  });\n}\nvar out = { keys: {}, facts: [], errors: [] };\nfunction fact(label, fn) {\n  try { out.facts.push(label + ' ' + String(fn())); }\n  catch (e) { out.facts.push(label + ' threw ' + e.constructor.name + ': ' + e.message); }\n}\nfunction err(label, fn) {\n  try { var v = fn(); out.errors.push(label + ': no throw ' + String(v)); }\n  catch (e) { out.errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }\n}\nout.keys.JSON = keys(JSON);\nfact('stringify indent', function () { return [JSON.stringify({ a: [1, { b: 2 }], c: {} , d: [] }, null, 2), JSON.stringify([1], null, '--------------x'), JSON.stringify({ a: 1 }, null, 20), JSON.stringify({ a: 1 }, null, new Number(3)), JSON.stringify({ a: 1 }, null, new String('ab'))].join(' | '); });\nfact('stringify replacer', function () { return [JSON.stringify({ a: 1, b: 2, 3: 4 }, ['b', 3, 'a', 'b', new String('a')]), JSON.stringify({ a: 1, b: [2] }, function (k, v) { return typeof v === 'number' ? v * 10 : v; })].join(' | '); });\nfact('rawJSON', function () { var r = JSON.rawJSON('1e1000'); return [JSON.stringify({ a: r }), JSON.isRawJSON(r), JSON.isRawJSON({}), Object.getPrototypeOf(r), Object.isFrozen(r), Object.keys(r).join(), r.rawJSON, JSON.stringify([JSON.rawJSON('\"x\"'), JSON.rawJSON('null')])].join(' | '); });\nfact('parse', function () { return [JSON.stringify(JSON.parse(' {\"a\":[1,2,{\"b\":null}],\"__proto__\":1,\"c\":-0.5e1} ')), Object.keys(JSON.parse('{\"b\":1,\"a\":2,\"1\":3}')).join(), JSON.parse('\"\\\\u00e9\\\\ud83d\\\\ude00\"'), Object.is(JSON.parse('-0'), -0), JSON.parse('1E400'), JSON.parse('{\"__proto__\":[]}').__proto__.length].join(' | '); });\nfact('parse reviver', function () { var log = []; JSON.parse('{\"a\":[1,2],\"b\":{\"c\":3}}', function (k, v, ctx) { log.push(k + ':' + (ctx && 'source' in ctx ? ctx.source : '-')); return v; }); return log.join(','); });\nfact('parse reviver delete', function () { return JSON.stringify(JSON.parse('{\"a\":1,\"b\":2,\"c\":[1,2]}', function (k, v) { return k === 'b' || k === '0' ? undefined : v; })); });\nfact('parse reviver source', function () { var s = []; JSON.parse('[1.0, -0, 1e2, \"x\", true, null, {\"a\": 10}]', function (k, v, ctx) { s.push(k + '=' + JSON.stringify(ctx)); return v; }); return s.join(' '); });\nerr('parse empty', function () { JSON.parse(''); });\nerr('parse end', function () { JSON.parse('{\"a\":'); });\nerr('parse token', function () { JSON.parse('{a:1}'); });\nerr('parse trailing comma', function () { JSON.parse('[1,]'); });\nerr('parse trailing', function () { JSON.parse('1 2'); });\nerr('parse undefined', function () { JSON.parse(undefined); });\nerr('parse long', function () { JSON.parse('{\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\": x}'); });\nerr('parse single quote', function () { JSON.parse(\"'a'\"); });\nerr('parse bad escape', function () { JSON.parse('\"\\\\x\"'); });\nerr('parse control', function () { JSON.parse('\"a\\u0001\"'); });\nerr('parse number', function () { JSON.parse('01'); });\nerr('parse minus', function () { JSON.parse('-'); });\nerr('parse dot', function () { JSON.parse('1.'); });\nerr('parse unterminated string', function () { JSON.parse('\"abc'); });\nerr('parse bad unicode', function () { JSON.parse('\"\\\\u12\"'); });\nerr('parse nan', function () { JSON.parse('NaN'); });\nerr('parse object', function () { JSON.parse({}); });\nerr('stringify cycle', function () { var o = { a: { b: {} } }; o.a.b.c = o; JSON.stringify(o); });\nerr('stringify cycle array', function () { var a = [1]; a.push([a]); JSON.stringify(a); });\nerr('stringify cycle class', function () { class K { constructor() { this.self = this; } } JSON.stringify(new K()); });\nerr('stringify cycle long', function () { var o = { p1: { p2: { p3: { p4: {} } } } }; o.p1.p2.p3.p4.back = o; JSON.stringify(o); });\nerr('rawJSON object', function () { JSON.rawJSON('{}'); });\nerr('rawJSON space', function () { JSON.rawJSON(' 1'); });\nerr('rawJSON bad', function () { JSON.rawJSON('x'); });\nerr('rawJSON empty', function () { JSON.rawJSON(''); });\nfact('sumPrecise', function () { return [Math.sumPrecise([1, 2, 3]), Math.sumPrecise([1e20, 0.1, -1e20]), Math.sumPrecise([]), Object.is(Math.sumPrecise([-0, -0]), -0), Math.sumPrecise([0.1, 0.2]), Math.sumPrecise([Infinity, 1]), Math.sumPrecise([Infinity, -Infinity]), Math.sumPrecise([NaN]), Math.sumPrecise([1e308, 1e308]), Math.sumPrecise([1e308, 1e308, -1e308])].join(); });\nerr('sumPrecise string', function () { Math.sumPrecise(['1']); });\nerr('sumPrecise not iterable', function () { Math.sumPrecise(1); });\n[out.keys.JSON.join(', ')].concat(out.facts, out.errors).join('\\n')";

#[test]
fn json_and_sum_precise_read_as_the_reference_browser_gives_them() {
    assert_eq!(
        run(PROBE),
        "parse [function w-c len=2 name=parse], stringify [function w-c len=3 name=stringify], rawJSON [function w-c len=1 name=rawJSON], isRawJSON [function w-c len=1 name=isRawJSON], Symbol(Symbol.toStringTag) [string --c]\nstringify indent {\n  \"a\": [\n    1,\n    {\n      \"b\": 2\n    }\n  ],\n  \"c\": {},\n  \"d\": []\n} | [\n----------1\n] | {\n          \"a\": 1\n} | {\n   \"a\": 1\n} | {\nab\"a\": 1\n}\nstringify replacer {\"b\":2,\"3\":4,\"a\":1} | {\"a\":10,\"b\":[20]}\nrawJSON {\"a\":1e1000} | true | false |  | true | rawJSON | 1e1000 | [\"x\",null]\nparse {\"a\":[1,2,{\"b\":null}],\"__proto__\":1,\"c\":-5} | 1,b,a | \u{e9}\u{1f600} | true | Infinity | 0\nparse reviver 0:1,1:2,a:-,c:3,b:-,:-\nparse reviver delete {\"a\":1,\"c\":[null,2]}\nparse reviver source 0={\"source\":\"1.0\"} 1={\"source\":\"-0\"} 2={\"source\":\"1e2\"} 3={\"source\":\"\\\"x\\\"\"} 4={\"source\":\"true\"} 5={\"source\":\"null\"} a={\"source\":\"10\"} 6={} ={}\nsumPrecise 6,0.1,0,true,0.30000000000000004,Infinity,NaN,NaN,Infinity,1e+308\nparse empty: SyntaxError: Unexpected end of JSON input\nparse end: SyntaxError: Unexpected end of JSON input\nparse token: SyntaxError: Expected property name or '}' in JSON at position 1 (line 1 column 2)\nparse trailing comma: SyntaxError: Unexpected token ']', \"[1,]\" is not valid JSON\nparse trailing: SyntaxError: Unexpected non-whitespace character after JSON at position 2 (line 1 column 3)\nparse undefined: SyntaxError: \"undefined\" is not valid JSON\nparse long: SyntaxError: Unexpected token 'x', ...\"aaaaaaa\": x}\" is not valid JSON\nparse single quote: SyntaxError: Unexpected token ''', \"'a'\" is not valid JSON\nparse bad escape: SyntaxError: Bad escaped character in JSON at position 2 (line 1 column 3)\nparse control: SyntaxError: Bad control character in string literal in JSON at position 2 (line 1 column 3)\nparse number: SyntaxError: Unexpected number in JSON at position 1 (line 1 column 2)\nparse minus: SyntaxError: No number after minus sign in JSON at position 1 (line 1 column 2)\nparse dot: SyntaxError: Unterminated fractional number in JSON at position 2 (line 1 column 3)\nparse unterminated string: SyntaxError: Unterminated string in JSON at position 4 (line 1 column 5)\nparse bad unicode: SyntaxError: Bad Unicode escape in JSON at position 5 (line 1 column 6)\nparse nan: SyntaxError: \"NaN\" is not valid JSON\nparse object: SyntaxError: \"[object Object]\" is not valid JSON\nstringify cycle: TypeError: Converting circular structure to JSON\n    --> starting at object with constructor 'Object'\n    |     property 'a' -> object with constructor 'Object'\n    |     property 'b' -> object with constructor 'Object'\n    --- property 'c' closes the circle\nstringify cycle array: TypeError: Converting circular structure to JSON\n    --> starting at object with constructor 'Array'\n    |     index 1 -> object with constructor 'Array'\n    --- index 0 closes the circle\nstringify cycle class: TypeError: Converting circular structure to JSON\n    --> starting at object with constructor 'K'\n    --- property 'self' closes the circle\nstringify cycle long: TypeError: Converting circular structure to JSON\n    --> starting at object with constructor 'Object'\n    |     property 'p1' -> object with constructor 'Object'\n    |     property 'p2' -> object with constructor 'Object'\n    |     ...\n    |     property 'p4' -> object with constructor 'Object'\n    --- property 'back' closes the circle\nrawJSON object: SyntaxError: Unexpected token '{', \"{}\" is not valid JSON\nrawJSON space: SyntaxError: Unexpected token ' ', \" 1\" is not valid JSON\nrawJSON bad: SyntaxError: Unexpected token 'x', \"x\" is not valid JSON\nrawJSON empty: SyntaxError: Invalid value for JSON.rawJSON\nsumPrecise string: TypeError: Type of 'Iterator value' must be 'number', found 'string'\nsumPrecise not iterable: TypeError: [object Number] is not iterable"
    );
}
