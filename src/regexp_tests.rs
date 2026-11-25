//! RegExp (J4.5a): the constructor, the prototype's methods and the String
//! methods that use them, with their results and messages as the reference
//! browser gives them (the J4.5 probes, Chrome 158).

use crate::agent::{Agent, AgentOptions, Outcome};

fn run(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    agent.scope(|scope| match scope.run_script_named(src, "r.js") {
        Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
        Ok(Outcome::Threw(e)) => format!("throw {}", scope.to_text(e).unwrap_or_default()),
        Ok(Outcome::Terminated) => "terminated".into(),
        Err(e) => format!("compile error: {e}"),
    })
}

/// What each of `cases` throws (`name: message`), or `no throw`.
fn errors(cases: &[&str]) -> Vec<String> {
    cases
        .iter()
        .map(|c| {
            run(&format!(
                "(function () {{ try {{ {c}; return 'no throw'; }} \
                 catch (e) {{ return e.constructor.name + ': ' + e.message; }} }})()"
            ))
        })
        .collect()
}

#[test]
fn exec_gives_the_match_array() {
    assert_eq!(
        run(r#"var r = /(?<x>a)(b)?/d.exec('zab');
            JSON.stringify(Reflect.ownKeys(r)) + ' ' + JSON.stringify(r) + ' ' + r.index + ' ' +
            r.input + ' ' + JSON.stringify(r.groups) + ' ' + Object.getPrototypeOf(r.groups) + ' ' +
            JSON.stringify(r.indices) + ' ' + JSON.stringify(r.indices.groups)"#),
        r#"["0","1","2","length","index","input","groups","indices"] ["ab","a","b"] 1 zab {"x":"a"} null [[1,3],[1,2],[2,3]] {"x":[1,2]}"#
    );
    assert_eq!(
        run("var r = /a/.exec('a'); JSON.stringify(Reflect.ownKeys(r)) + ' ' + r.groups"),
        r#"["0","length","index","input","groups"] undefined"#
    );
}

#[test]
fn flags_source_and_to_string() {
    assert_eq!(
        run("[/a/dgimsuy.flags, /a/v.flags, new RegExp('a', 'gimsuyd').flags].join(' ')"),
        "dgimsuy v dgimsuy"
    );
    assert_eq!(
        run(
            r#"[String(/a\/b/), String(new RegExp('a/b')), String(new RegExp('')),
            new RegExp('\n').source, new RegExp('/').source, String(new RegExp('[/]')),
            RegExp.prototype.source, RegExp.prototype.flags, String(RegExp.prototype),
            RegExp.prototype.global].join(' | ')"#
        ),
        r"/a\/b/ | /a\/b/ | /(?:)/ | \n | \/ | /[/]/ | (?:) |  | /(?:)/ | "
    );
    assert_eq!(
        run(
            "[typeof RegExp[Symbol.species], RegExp[Symbol.species] === RegExp, \
             RegExp.prototype[Symbol.split].name, RegExp.prototype[Symbol.matchAll].name].join(' ')"
        ),
        "function true [Symbol.split] [Symbol.matchAll]"
    );
}

#[test]
fn last_index_moves_as_in_chrome() {
    // A global or sticky regexp starts at lastIndex; in u mode a lastIndex
    // inside a surrogate pair steps back to the pair's lead.
    assert_eq!(
        run(r"var r = /a/g, s = 'aXa', o = [];
            o.push(r.test(s), r.lastIndex, r.test(s), r.lastIndex, r.test(s), r.lastIndex);
            var y = /a/y; o.push(y.test('ba'), y.lastIndex);
            var u = /\udc00/u; u.lastIndex = 1; o.push(u.exec('\ud800\udc00'));
            var uy = /./gu; uy.lastIndex = 1;
            o.push(JSON.stringify(uy.exec('\ud800\udc00x')), uy.lastIndex);
            o.join()"),
        "true,1,true,3,false,0,false,0,,[\"\u{10000}\"],2"
    );
}

#[test]
fn replace_split_and_match() {
    assert_eq!(
        run(
            r#"['abc'.replace(/b/, '[$&|$`|$\'|$$]'), 'abc'.replace(/(?<n>b)/, '<$<n>>'),
            'aaa'.replace(/a/g, function (m, i) { return i; }), 'x-y'.replace(/(-)/, '$1$2$01$10'),
            'abc'.replaceAll(/b/g, 'B'), 'a1b2'.replace(/\d/g, '#')].join(' ')"#
        ),
        "a[b|a|c|$]c a<b>c 012 x-$2--0y aBc a#b#"
    );
    // A global regexp's empty match moves lastIndex on by one.
    assert_eq!(
        run(
            "['abc'.replace(/x*/g, '-'), JSON.stringify('abc'.match(/x*/g)), \
             Array.from('ab'.matchAll(/x*/g), function (m) { return m.index; }).join()].join(' ')"
        ),
        r#"-a-b-c- ["","","",""] 0,1,2"#
    );
    assert_eq!(
        run(
            r#"[JSON.stringify('a1b2c'.split(/\d/)), JSON.stringify('a1b2c'.split(/(\d)/)),
            JSON.stringify('abc'.split(/(?:)/)), JSON.stringify(''.split(/a/)),
            JSON.stringify('ab'.split(/a*?/)),
            JSON.stringify('A<B>bold</B>and<CODE>coded</CODE>'.split(/<(\/)?([^<>]+)>/))].join(' ')"#
        ),
        r#"["a","b","c"] ["a","1","b","2","c"] ["a","b","c"] [""] ["a","b"] ["A",null,"B","bold","/","B","and",null,"CODE","coded","/","CODE",""]"#
    );
    assert_eq!(
        run(
            r"[JSON.stringify('a1b2'.match(/\d/g)), JSON.stringify('ab'.match(/x/g)),
            'a1b2'.search(/\d/),
            JSON.stringify(Array.from('a1b2'.matchAll(/\d/g), function (m) { return m[0] + m.index; }))].join(' ')"
        ),
        r#"["1","2"] null 1 ["11","23"]"#
    );
}

#[test]
fn the_constructor_and_regexp_likes() {
    assert_eq!(
        run("var r = /a/g, o = []; \
             o.push(RegExp(r) === r, new RegExp(r) === r, new RegExp(r, 'i').flags, RegExp(r, 'm').source); \
             var like = { [Symbol.match]: true, source: 'x', flags: 'g', constructor: RegExp }; \
             o.push(RegExp(like) === like, new RegExp(like).source); o.join()"),
        "true,false,i,a,true,x"
    );
}

#[test]
fn a_primitive_pattern_is_never_asked_for_its_method() {
    // Chrome 158 (the j45 prim probe) never looks a symbol method up on a
    // primitive argument: Number.prototype's getter is not called.
    assert_eq!(
        run(r"var o = [], called = [];
            [Symbol.match, Symbol.matchAll, Symbol.replace, Symbol.search, Symbol.split].forEach(function (s) {
              Object.defineProperty(Number.prototype, s, { configurable: true,
                get: function () { called.push(String(s)); return undefined; } });
            });
            o.push(JSON.stringify('a1b1c'.match(1)), Array.from('a1b1c'.matchAll(1), String).join('/'),
              'a1b1c'.replace(1, 'x'), 'a1b1c'.replaceAll(1, 'x'), 'a1b1c'.search(1),
              JSON.stringify('a1b1c'.split(1)), called.length);
            o.join(' ')"),
        r#"["1"] 1/1 axb1c axbxc 1 ["a","b","c"] 0"#
    );
}

#[test]
fn messages_are_chromes() {
    assert_eq!(
        errors(&[
            "new RegExp('(')",
            "new RegExp('a', 'gg')",
            "new RegExp('a', 'uv')",
            "RegExp.prototype.exec.call({}, 'a')",
            "Object.getOwnPropertyDescriptor(RegExp.prototype, 'flags').get.call(1)",
            "Object.getOwnPropertyDescriptor(RegExp.prototype, 'global').get.call({})",
            "'a'.matchAll(/a/)",
            "'a'.replaceAll(/a/, 'b')",
            "RegExp.prototype.toString.call(1)",
            "RegExp.prototype.test.call(1, 'a')",
            "var r = /a/g; Object.defineProperty(r, 'lastIndex', { writable: false }); r.exec('b')",
            "Object.getPrototypeOf('a'.matchAll(/a/g)).next.call({})",
            r"eval('/(?<a>.)\\k<b>/')",
            "var r = /a/; r.exec = function () { return 1; }; r.test('a')",
            "RegExp.prototype.test.call({ exec: function () { return 'x'; } }, 'a')",
        ]),
        [
            "SyntaxError: Invalid regular expression: /(/: Unterminated group",
            "SyntaxError: Invalid flags supplied to RegExp constructor 'gg'",
            "SyntaxError: Invalid flags supplied to RegExp constructor 'uv'",
            "TypeError: Method RegExp.prototype.exec called on incompatible receiver #<Object>",
            "TypeError: RegExp.prototype.flags getter called on non-object 1",
            "TypeError: RegExp.prototype.global getter called on non-RegExp object",
            "TypeError: String.prototype.matchAll called with a non-global RegExp argument",
            "TypeError: String.prototype.replaceAll called with a non-global RegExp argument",
            "TypeError: Method RegExp.prototype.toString called on incompatible receiver 1",
            "TypeError: Method RegExp.prototype.test called on incompatible receiver 1",
            "TypeError: Cannot assign to read only property 'lastIndex' of object '[object RegExp]'",
            "TypeError: Method %RegExpStringIterator%.prototype.next called on incompatible receiver #<Object>",
            "SyntaxError: Invalid regular expression: /(?<a>.)\\k<b>/: Invalid named capture referenced",
            "TypeError: RegExp exec method returned something other than an Object or null",
            "TypeError: RegExp exec method returned something other than an Object or null",
        ]
    );
}

#[test]
fn case_folding_and_lookbehind() {
    // Without u, ignoreCase maps by toUppercase but never from a non-ASCII
    // character to an ASCII one; with u it is simple case folding, and \w
    // takes in U+017F and U+212A. Lookbehind captures run right to left.
    assert_eq!(
        run(
            r"[/\u017f/i.test('s'), /\u017f/iu.test('s'), /\w/i.test('\u212a'), /\w/iu.test('\u212a'),
            /[a-z]/i.test('K'), /\u00e5/i.test('\u00c5'), /\W/iu.test('\u017f')].join()"
        ),
        "false,true,false,true,true,true,false"
    );
    assert_eq!(
        run(r"[JSON.stringify(/(?<=(\d+)(\d+))$/.exec('1053')),
            '$10.53'.match(/(?<=\$)\d+(\.\d*)?/)[0], '$10.53'.match(/(?<=\$\d+\.)\d+/)[0],
            JSON.stringify('abcdef'.match(/(?<=(c))def/)), /(?<!a)b/.test('ab')].join(' ')"),
        r#"["","1","053"] 10.53 53 ["def","c"] false"#
    );
}

#[test]
fn a_catastrophic_pattern_stops_soon_after_the_interrupt() {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};
    // Backreferences turn memoization off, so these would run for ages:
    // a watchdog's interrupt must stop each within 100 ms, as it stops a
    // long loop. The interrupt comes once the hook has been asked three
    // times (30,000 steps, so the match is under way, however long the
    // agent took to start). The agent runs on its own thread so that a
    // broken bound fails the test instead of hanging it.
    for src in [
        r"/^(a+)+\1$/.test('a'.repeat(40) + '!')",
        r"/^((a+)+)\2\1$/i.exec('a'.repeat(40) + '!')",
    ] {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let asked = Arc::new(AtomicU32::new(0));
        let count = Arc::clone(&asked);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut agent = Agent::new(AgentOptions::default());
            agent.set_interrupt(Some(Box::new(move || {
                count.fetch_add(1, Ordering::Relaxed);
                flag.load(Ordering::Relaxed)
            })));
            let ended = agent.scope(|scope| match scope.run_script(src) {
                Ok(Outcome::Terminated) => "terminated".to_owned(),
                Ok(Outcome::Value(v)) => {
                    format!("finished {}", scope.to_text(v).unwrap_or_default())
                }
                Ok(Outcome::Threw(e)) => format!("threw {}", scope.to_text(e).unwrap_or_default()),
                Err(e) => format!("did not compile: {e}"),
            });
            let _ = tx.send((ended, Instant::now()));
        });
        let begun = Instant::now();
        while asked.load(Ordering::Relaxed) < 3 {
            assert!(
                begun.elapsed() < Duration::from_secs(120),
                "{src}: the match never began"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let at = Instant::now();
        stop.store(true, Ordering::Relaxed);
        let (ended, done) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the interrupt did not stop it");
        assert_eq!(ended, "terminated", "{src}");
        let took = done.saturating_duration_since(at);
        assert!(
            took < Duration::from_millis(100),
            "{src}: stopped {took:?} after the interrupt"
        );
    }
}

/// The J4.5b probe (Chrome 158): property escapes, `v`-mode sets, their
/// messages and their case folding, and a bad regexp literal's message
/// through `Function` and `eval`.
const PROPERTY_PROBE: &str = r#"// SilverJS's J4.5b probe cases: property escapes and v-mode sets, their
// messages and their case folding. Each is [label, source of an
// expression]; the page and SilverJS's shell run the same list.
var cases = [
  ['bad name', "new RegExp('\\\\p{Foo}', 'u')"],
  ['bad name in class', "new RegExp('[\\\\p{Foo}]', 'u')"],
  ['bad value', "new RegExp('\\\\p{Script=Foo}', 'u')"],
  ['bad name v', "new RegExp('\\\\p{Foo}', 'v')"],
  ['bad name in class v', "new RegExp('[\\\\p{Foo}]', 'v')"],
  ['string property with u', "new RegExp('\\\\p{RGI_Emoji}', 'u')"],
  ['negated string property', "new RegExp('\\\\P{RGI_Emoji}', 'v')"],
  ['negated class with strings', "new RegExp('[^\\\\p{RGI_Emoji}]', 'v')"],
  ['gc=L alone and loose', "[/\\p{L}/u.test('a'), /\\p{gc=L}/u.test('1'), /\\p{General_Category=Letter}/u.test('b')].join()"],
  ['loose matching is not done', "new RegExp('\\\\p{letter}', 'u')"],
  ['Script vs Extensions', "[/\\p{sc=Deva}/u.test('\\u0964'), /\\p{scx=Deva}/u.test('\\u0964'), /\\p{scx=Zyyy}/u.test('\\u0964')].join()"],
  ['Unknown', "[/\\p{sc=Zzzz}/u.test('\\u0378'), /\\p{scx=Unknown}/u.test('a')].join()"],
  ['P without i', "[/\\P{Lu}/u.test('A'), /\\P{Lu}/u.test('a'), /[\\P{L}]/u.test('1'), /[^\\P{L}]/u.test('a')].join()"],
  ['P Lu iu',"[/\\P{Lu}/iu.test('a'), /\\P{Lu}/iu.test('A'), /\\p{Lu}/iu.test('a'), /[^\\p{Lu}]/iu.test('a')].join()"],
  ['P Lu iv', "[/\\P{Lu}/iv.test('a'), /\\P{Lu}/iv.test('A'), /\\p{Lu}/iv.test('a'), /[^\\p{Lu}]/iv.test('a')].join()"],
  ['P Ll iv', "[/\\P{Ll}/iv.test('a'), /\\P{Ll}/iv.test('A'), /[\\P{Ll}]/iv.test('A')].join()"],
  ['emoji longest', "JSON.stringify('\\u{1F468}\\u200D\\u{1F469}\\u200D\\u{1F467}x'.match(/^\\p{RGI_Emoji}/v)[0].length)"],
  ['keycap and flag', "[/^\\p{Emoji_Keycap_Sequence}$/v.test('#\\uFE0F\\u20E3'), /^\\p{RGI_Emoji_Flag_Sequence}$/v.test('\\u{1F1EB}\\u{1F1F7}')].join()"],
  ['set ops', "[/[\\p{L}--[a-z]]/v.test('a'), /[\\p{L}&&\\p{sc=Grek}]/v.test('\\u03B1'), /[[a-z]--\\q{a|bc}]/v.test('a'), /^[\\q{abc|d}]$/v.test('abc')].join()"],
  ['set ops i', "[/[\\p{Lu}--[A-Z]]/iv.test('a'), /[\\p{Lu}&&[a-z]]/iv.test('A'), /[[a-z]--[A]]/iv.test('a')].join()"],
  ['Any Assigned ASCII', "[/^\\p{Any}$/u.test('\\u{10FFFF}'), /\\p{Assigned}/u.test('\\u0378'), /\\p{ASCII}/u.test('\\x7F')].join()"],
  ['17.0 script', "[/\\p{sc=Sidetic}/u.test('\\u{10940}'), /\\p{Script=Sidt}/u.test('\\u{10940}')].join()"],
  ['source and flags', "String(new RegExp('\\\\p{L}', 'v'))"],
  ['v class syntax characters', "['[{]', '[a&&&b]', '[a--]', '[(]', '[a-]', '[&&a]', '[a!!]'].map(function (p) { try { new RegExp(p, 'v'); return p + ' ok'; } catch (e) { return p + ' ' + e.message; } }).join(' | ')"],
  ['literal error in Function', "(function () { try { new Function('return /a{2,1}/'); } catch (e) { return e.message; } })()"],
  ['literal error in eval', "(function () { try { eval('/(/'); } catch (e) { return e.message; } })()"]
];
function run() {
  return cases.map(function (c) {
    try { return c[0] + ': ' + String(new Function('return ' + c[1])()); }
    catch (e) { return c[0] + ': ' + e.constructor.name + ': ' + e.message; }
  });
}
run().join('\n')"#;

#[test]
fn property_escapes_and_sets_read_as_the_reference_browser_gives_them() {
    let want = [
        r#"bad name: SyntaxError: Invalid regular expression: /\p{Foo}/u: Invalid property name"#,
        r#"bad name in class: SyntaxError: Invalid regular expression: /[\p{Foo}]/u: Invalid property name in character class"#,
        r#"bad value: SyntaxError: Invalid regular expression: /\p{Script=Foo}/u: Invalid property name"#,
        r#"bad name v: SyntaxError: Invalid regular expression: /\p{Foo}/v: Invalid property name"#,
        r#"bad name in class v: SyntaxError: Invalid regular expression: /[\p{Foo}]/v: Invalid property name in character class"#,
        r#"string property with u: SyntaxError: Invalid regular expression: /\p{RGI_Emoji}/u: Invalid property name"#,
        r#"negated string property: SyntaxError: Invalid regular expression: /\P{RGI_Emoji}/v: Invalid property name"#,
        r#"negated class with strings: SyntaxError: Invalid regular expression: /[^\p{RGI_Emoji}]/v: Negated character class may contain strings"#,
        r#"gc=L alone and loose: true,false,true"#,
        r#"loose matching is not done: SyntaxError: Invalid regular expression: /\p{letter}/u: Invalid property name"#,
        r#"Script vs Extensions: false,true,false"#,
        r#"Unknown: true,false"#,
        r#"P without i: false,true,true,true"#,
        r#"P Lu iu: true,true,true,false"#,
        r#"P Lu iv: false,false,true,false"#,
        r#"P Ll iv: false,false,false"#,
        r#"emoji longest: 8"#,
        r#"keycap and flag: true,true"#,
        r#"set ops: false,true,false,true"#,
        r#"set ops i: false,true,false"#,
        r#"Any Assigned ASCII: true,false,true"#,
        r#"17.0 script: true,true"#,
        r#"source and flags: /\p{L}/v"#,
        r#"v class syntax characters: [{] Invalid regular expression: /[{]/v: Invalid character in character class | [a&&&b] Invalid regular expression: /[a&&&b]/v: Invalid character in character class | [a--] Invalid regular expression: /[a--]/v: Invalid character in character class | [(] Invalid regular expression: /[(]/v: Invalid character in character class | [a-] Invalid regular expression: /[a-]/v: Invalid character in character class | [&&a] Invalid regular expression: /[&&a]/v: Invalid set operation in character class | [a!!] Invalid regular expression: /[a!!]/v: Invalid set operation in character class"#,
        r#"literal error in Function: Invalid regular expression: /a{2,1}/: numbers out of order in {} quantifier"#,
        r#"literal error in eval: Invalid regular expression: /(/: Unterminated group"#,
    ];
    let got = run(PROPERTY_PROBE);
    let got: Vec<&str> = got.split('\n').collect();
    assert_eq!(got, want);
}
