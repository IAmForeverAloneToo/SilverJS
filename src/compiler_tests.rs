//! The compiler: encoding, scope placement, code shapes, what is refused
//! for now, and the depth rule (the compiler never refuses a source the
//! parser accepted with the same stack budget).

use crate::compiler::{
    CompileError, CompiledUnit, Constant, FunctionKind, GlobalDecl, Op, Operand, compile,
    compile_with_stats, decode,
};
use crate::parser::Goal;

fn unit(src: &str) -> CompiledUnit {
    let u = compile(src, Goal::Script).unwrap_or_else(|e| panic!("{src}: {e:?}"));
    u.validate()
        .unwrap_or_else(|e| panic!("{src}: {e}\n{}", u.disassemble()));
    u
}

fn dis(src: &str, f: usize) -> String {
    unit(src).functions[f].disassemble()
}

fn ops(src: &str, f: usize) -> Vec<Op> {
    let u = unit(src);
    let code = &u.functions[f].code;
    let mut out = Vec::new();
    let mut pc = 0;
    while let Some(i) = decode(code, pc) {
        out.push(i.op);
        pc += i.len;
    }
    out
}

/// Runs `f` on a thread with a large stack, as the deep tests need in
/// debug builds.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn every_instruction_round_trips_at_every_width() {
    let mut b = 0u8;
    while let Some(op) = Op::from_byte(b) {
        b += 1;
        if matches!(op, Op::Wide | Op::ExtraWide) {
            continue;
        }
        for scale in [1i64, 300, 70_000, -5, -200, -40_000] {
            let operands: Vec<i64> = op
                .operands()
                .iter()
                .enumerate()
                .map(|(i, k)| match k {
                    Operand::Imm | Operand::Jump => scale - i as i64,
                    _ => scale.abs() + i as i64,
                })
                .collect();
            let mut code = Vec::new();
            crate::compiler::bytecode_encode_for_tests(&mut code, op, &operands);
            let instr = decode(&code, 0).unwrap();
            assert_eq!(instr.op, op);
            assert_eq!(instr.len as usize, code.len(), "{op:?}");
            assert_eq!(
                &instr.operands[..operands.len()],
                &operands[..],
                "{op:?} at {scale}"
            );
            let wide = code[0] == Op::Wide as u8 || code[0] == Op::ExtraWide as u8;
            let needs = operands.iter().zip(op.operands()).any(|(v, k)| match k {
                Operand::Jump => false,
                Operand::Imm => i8::try_from(*v).is_err(),
                _ => u8::try_from(*v).is_err(),
            });
            assert_eq!(wide, needs, "{op:?} at {scale}");
        }
    }
    // Two prefixes in a row are not an instruction.
    assert!(decode(&[Op::Wide as u8, Op::Wide as u8, 0], 0).is_none());
    assert!(decode(&[255], 0).is_none());
}

#[test]
fn a_small_script_compiles_as_written() {
    assert_eq!(
        dis("var x = 1 + 2; x", 0),
        "    0 LdaSmi 1\n    2 Star r1\n    4 LdaSmi 2\n    6 Add r1\n    8 StaGlobal [0] \"x\", ic0\n   11 LdaGlobal [0] \"x\", ic1\n   14 Star r0\n   16 Ldar r0\n   18 Return\n"
    );
    let u = unit("var x; let y; const z = 1; function f() {}");
    assert_eq!(
        u.globals,
        [
            GlobalDecl::Var("x".encode_utf16().collect()),
            GlobalDecl::Let("y".encode_utf16().collect()),
            GlobalDecl::Const("z".encode_utf16().collect()),
            GlobalDecl::Function("f".encode_utf16().collect(), 1),
        ]
    );
    // A top-level `let` is initialized as a global lexical binding.
    assert!(ops("let y = 2", 0).contains(&Op::InitGlobalLexical));
}

#[test]
fn bindings_live_in_registers_unless_captured() {
    let src = "function f(a, b) { var c = a; var d = 1; return function () { return b + c; }; }";
    let u = unit(src);
    let f = &u.functions[1];
    let text = f.disassemble();
    // `b` and `c` are captured: f's environment holds them; `a` and `d`
    // stay in registers.
    assert!(
        text.starts_with("    0 CreateFunctionContext scope0\n    2 Ldar r1\n    4 StaSlot 0, 0\n"),
        "{text}"
    );
    assert!(
        text.contains("Ldar r0\n") && text.contains("StaSlot 0, 1\n"),
        "{text}"
    );
    assert_eq!(u.scopes[0].names.len(), 2);
    let inner = u.functions[2].disassemble();
    assert!(
        inner.contains("LdaSlot 0, 0") && inner.contains("LdaSlot 0, 1"),
        "{inner}"
    );
    // Nothing captured: no environment at all.
    assert!(!ops("function g(a) { var b = a; return b; }", 1).contains(&Op::CreateFunctionContext));
}

#[test]
fn a_block_gets_an_environment_only_when_captured() {
    let plain = ops("function f() { { let x = 1; x; } }", 1);
    assert!(!plain.contains(&Op::PushContext));
    let captured = ops("function f() { { let x = 1; return () => x; } }", 1);
    assert!(captured.contains(&Op::PushContext) && captured.contains(&Op::PopContext));
    // An inner function reads it one environment out.
    let u = unit("function f() { let x = 1; { let y = 2; return () => x + y; } }");
    let arrow = u.functions[2].disassemble();
    assert!(
        arrow.contains("LdaSlot 1, 0") && arrow.contains("LdaSlot 0, 0"),
        "{arrow}"
    );
}

#[test]
fn loops_copy_a_captured_let_each_iteration() {
    let captured = ops(
        "function f() { for (let i = 0; i < 3; i++) g(() => i); }",
        1,
    );
    assert_eq!(
        captured.iter().filter(|o| **o == Op::CopyContext).count(),
        2
    );
    assert!(captured.contains(&Op::JumpLoop));
    let plain = ops("function f() { for (let i = 0; i < 3; i++) g(i); }", 1);
    assert!(!plain.contains(&Op::CopyContext));
}

#[test]
fn the_tdz_is_checked_only_where_it_can_be_hit() {
    // Read after the declaration, in the same function: no check.
    let after = ops("function f() { let x = 1; return x; }", 1);
    assert!(!after.contains(&Op::CheckTdz));
    // Read by a closure, or before, or in a switch: checked.
    let u = unit("function f() { g(); let x = 1; function g() { return x; } }");
    assert!(u.functions[2].disassemble().contains("CheckTdz [0] \"x\""));
    assert!(ops("function f() { x; let x; }", 1).contains(&Op::CheckTdz));
    assert!(ops("function f() { x; const x = 1; }", 1).contains(&Op::CheckTdz));
    // A captured const starts uninitialized in its environment.
    let u = unit("function f() { const c = 1; return () => c; }");
    assert_eq!(u.scopes[0].lexical, [true]);
    assert!(
        ops(
            "function f(a) { switch (a) { case 0: let y = 1; case 1: return y; } }",
            1
        )
        .contains(&Op::CheckTdz)
    );
    // An assignment before the declaration checks before it stores.
    assert!(ops("function f() { x = 2; let x; }", 1).contains(&Op::CheckTdzRegister));
}

#[test]
fn constants_and_function_names_refuse_assignment() {
    assert!(ops("function f() { const c = 1; c = 2; }", 1).contains(&Op::ThrowConstAssign));
    assert!(ops("function f() { const c = 1; c += 2; }", 1).contains(&Op::ThrowConstAssign));
    // A named function expression's own name: refused in strict code,
    // ignored in sloppy.
    assert!(ops("(function g() { 'use strict'; g = 1; })", 1).contains(&Op::ThrowConstAssign));
    let sloppy = ops("(function g() { g = 1; return g; })", 1);
    assert!(!sloppy.contains(&Op::ThrowConstAssign) && sloppy.contains(&Op::LdaClosure));
    // A script's top-level const is a global; the interpreter refuses.
    assert!(ops("const c = 1; c = 2;", 0).contains(&Op::StaGlobal));
}

#[test]
fn eval_and_with_make_names_dynamic() {
    let u = unit("function h() { var v = 1; eval('v'); return w; }");
    let text = u.functions[1].disassemble();
    assert!(
        text.contains("CallEval") && text.contains("LdaDynamic [") && text.contains("\"w\""),
        "{text}"
    );
    // `v` is in the environment, where eval's code can find it by name.
    assert!(
        u.scopes
            .iter()
            .any(|s| s.names.iter().any(|n| String::from_utf16_lossy(n) == "v"))
    );
    // Strict eval cannot add a var: `w` stays a global.
    let strict =
        unit("function h() { 'use strict'; eval('1'); return w; }").functions[1].disassemble();
    assert!(
        strict.contains("LdaGlobal [") && !strict.contains("LdaDynamic"),
        "{strict}"
    );
    let with = unit("function k(o) { with (o) { return x; } }").functions[1].disassemble();
    assert!(
        with.contains("PushWithContext") && with.contains("LdaDynamic"),
        "{with}"
    );
    // Only `eval(...)` itself is direct.
    assert!(!ops("(0, eval)('1')", 0).contains(&Op::CallEval));
}

#[test]
fn calls_keep_their_receiver() {
    let method = ops("o.m(1, 2)", 0);
    assert!(method.contains(&Op::Call) && !method.contains(&Op::CallUndefined));
    assert!(ops("o[k](1)", 0).contains(&Op::Call));
    assert!(ops("(o.m)()", 0).contains(&Op::Call));
    assert!(ops("f(1)", 0).contains(&Op::CallUndefined));
    assert!(ops("new F(1)", 0).contains(&Op::New));
    // Optional chains short-circuit to undefined.
    let text = dis("o?.a.b; o.m?.(); delete o?.a", 0);
    assert!(text.matches("JumpIfNullish").count() == 3, "{text}");
    assert!(
        text.contains("DeleteProperty") && text.contains("LdaTrue"),
        "{text}"
    );
}

#[test]
fn try_finally_runs_on_every_way_out() {
    let src = "function f(a) { for (;;) { try { if (a) return 1; if (a > 1) break; continue; } catch (e) { g(e); } finally { h(); } } }";
    let u = unit(src);
    let f = &u.functions[1];
    assert_eq!(f.handlers.len(), 2, "{}", f.disassemble());
    // The catch's handler is inner, so first.
    assert!(f.handlers[0].start >= f.handlers[1].start && f.handlers[0].end <= f.handlers[1].end);
    let text = f.disassemble();
    assert!(
        text.contains("Throw\n") && text.contains("Return\n"),
        "{text}"
    );
    // A try in a block with an environment restores it on a throw.
    let u = unit("function f() { { let x; g(() => x); try { h(); } catch (e) {} } }");
    let h = u.functions[1].handlers[0];
    assert!(h.context.is_some());
}

#[test]
fn literals_and_inferred_names() {
    let u = unit(
        "var f = function () {}; let g = () => 1; var o = { m() {}, ['k']: function () {}, get p() { return 1; } };",
    );
    let names: Vec<String> = u
        .functions
        .iter()
        .map(|f| String::from_utf16_lossy(&f.name))
        .collect();
    assert_eq!(names, ["", "f", "g", "m", "", "get p"]);
    assert_eq!(u.functions[3].kind, FunctionKind::Method);
    assert_eq!(u.functions[2].kind, FunctionKind::Arrow);
    let text = u.functions[0].disassemble();
    assert!(
        text.contains("SetFunctionName") && text.contains("DefineGetter"),
        "{text}"
    );
    let array = ops("[1, , 3, ,]", 0);
    assert!(array.contains(&Op::SetArrayLength));
    assert!(ops("({ __proto__: null })", 0).contains(&Op::SetProtoLiteral));
    assert!(ops("({ ...o })", 0).contains(&Op::CopyDataProperties));
    assert!(ops("`a${b}c`", 0).contains(&Op::ToString));
    let u = unit("var n = { 1.5: 1, 0x10: 2 }; 1e300; -0; 2147483648");
    let strings: Vec<String> = u.functions[0]
        .constants
        .iter()
        .filter_map(|c| match c {
            Constant::String(s) => Some(String::from_utf16_lossy(s)),
            _ => None,
        })
        .collect();
    assert!(
        strings.contains(&"1.5".into()) && strings.contains(&"16".into()),
        "{strings:?}"
    );
    assert!(
        u.functions[0]
            .constants
            .contains(&Constant::Number(2147483648.0))
    );
}

#[test]
fn wide_operands_are_used_past_a_byte() {
    let names: Vec<String> = (0..300).map(|i| format!("o.p{i}")).collect();
    let src = format!("function f(o) {{ {}; }}", names.join(";"));
    let u = unit(&src);
    let code = &u.functions[1].code;
    let mut pc = 0;
    let mut prefixed = 0;
    while let Some(i) = decode(code, pc) {
        if code[pc as usize] == Op::Wide as u8 {
            prefixed += 1;
            assert_eq!(i.op, Op::GetNamed);
        }
        pc += i.len;
    }
    // Names past the 256th take a two-byte operand.
    assert_eq!(prefixed, 300 - 256);
    let args: Vec<String> = (0..300).map(|i| i.to_string()).collect();
    let u = unit(&format!("f({})", args.join(",")));
    assert!(u.functions[0].register_count > 300);
}

#[test]
fn what_is_not_compiled_yet_is_refused() {
    assert!(matches!(
        compile("export {}", Goal::Module),
        Err(CompileError::Unsupported {
            feature: "modules",
            ..
        })
    ));
    // And a parse error is a parse error.
    assert!(matches!(
        compile("var 1", Goal::Script),
        Err(CompileError::Syntax(_))
    ));
}

#[test]
fn a_medley_compiles_into_valid_units() {
    let src = r#"
        'use strict';
        var a = 1, b = 'two', c = null, d = undefined;
        function outer(x, y = x + 1, ...rest) {
            var self = this, args = arguments;
            label: for (var i = 0; i < x; i++) {
                for (var k in rest) { if (k) continue label; else break; }
                switch (i) { case 0: break; case 1: { let z = i; } default: i++; }
                do { x--; } while (x > 0 && !(x === 3) || (x ?? 2));
                while (false) {}
            }
            try { throw new Error('e'); } catch { } finally { y = typeof q; }
            var o = { a, b: [x, y], c() { return this; }, set d(v) {}, [x + 'k']: 1 };
            o.a += 1; o['b'] ||= 2; o.c?.(); ++o.a; o.a--; x **= 2; delete o.a;
            return () => self && args && new.target && `${x}-${y}`;
        }
        outer.call(null, 3);
        void 0, typeof outer, -a, +b, ~a, !c, a in o, a instanceof Object;
        /re/g; 10n; 0.5;
    "#;
    let sloppy = unit(src.replace("'use strict';", "").as_str());
    assert!(sloppy.functions.len() >= 4);
    let strict = unit(src);
    assert!(strict.functions.iter().all(|f| f.strict));
}

#[test]
fn long_chains_compile_without_recursing() {
    // The parser builds these in loops; so must the compiler walk them.
    let chains = [
        format!("x{}", "+x".repeat(200_000)),
        format!("x{}", "**x".repeat(50_000)),
        format!("x{}", "||x".repeat(100_000)),
        format!("x{}", ".a".repeat(100_000)),
        format!("f{}", "()".repeat(50_000)),
        format!("x{}", "?.a".repeat(50_000)),
        format!("x{}", ",x".repeat(100_000)),
        format!("{}x", "!".repeat(900)),
    ];
    for src in chains {
        let u = compile(&src, Goal::Script).unwrap_or_else(|e| panic!("{}: {e:?}", &src[..20]));
        u.validate().unwrap();
    }
}

/// A source nested `depth` levels of `shape`, cycling through its
/// openings: function expressions called at once and arrows, the way
/// minified bundles nest (every function compiled on its own); then
/// expressions alone and statements alone, nested in one function.
fn nested(depth: usize) -> String {
    nested_shape(depth, 0)
}

const SHAPES: [(&[&str], &[&str]); 3] = [
    (
        &[
            "(function(){return ",
            "{a:",
            "[",
            "f(",
            "c?",
            "(a=>",
            "(function(){if(a){return ",
            "(",
        ],
        &["})()", "}", "]", ")", ":0", ")", "}})()", ")"],
    ),
    (
        &[
            "{a:", "[", "f(", "c?", "(", "o[", "x+(", "-(", "g(1,", "new F(",
        ],
        &["}", "]", ")", ":0", ")", "]", ")", ")", ")", ")"],
    ),
    (
        &[
            "if(a){",
            "{",
            "while(b){",
            "for(;;){",
            "try{",
            "switch(c){case 1:",
            "if(b);else{",
            "do{",
        ],
        &["}", "}", "}", "}", "}catch(e){}", "}", "}", "}while(d)"],
    ),
];

fn nested_shape(depth: usize, shape: usize) -> String {
    let (opens, closes) = SHAPES[shape];
    let mut src = String::from(if shape == 2 { "" } else { "var r=" });
    for i in 0..depth {
        src.push_str(opens[i % opens.len()]);
    }
    src.push_str(if shape == 2 { "x;" } else { "1" });
    for i in (0..depth).rev() {
        src.push_str(closes[i % closes.len()]);
    }
    src.push(';');
    src
}

#[test]
fn the_compiler_never_refuses_what_the_parser_accepts() {
    on_big_stack(|| {
        // Deeper and deeper, to past the parser's limit: at every depth
        // the parser accepts, the compiler takes no more stack than the
        // parser did, so the budget the parse needed fits the compile.
        for shape in 0..SHAPES.len() {
            let mut deepest = 0;
            let mut n = 1;
            loop {
                let src = nested_shape(n, shape);
                let (u, stats, compile_peak) =
                    match compile_with_stats(&src, Goal::Script, 128 << 20) {
                        Ok(r) => r,
                        Err(CompileError::Syntax(e)) => {
                            assert_eq!(e.message, "Maximum call stack size exceeded", "{n}");
                            break;
                        }
                        Err(e) => panic!("{n}: {e:?}"),
                    };
                u.validate().unwrap();
                assert!(
                    compile_peak <= stats.peak_stack,
                    "shape {shape}, nesting {}: the compiler took {compile_peak} bytes of stack, the parser {}",
                    stats.max_depth,
                    stats.peak_stack
                );
                let budget = stats.peak_stack + 4096;
                compile_with_stats(&src, Goal::Script, budget).unwrap_or_else(|e| {
                    panic!("nesting {}, budget {budget}: {e:?}", stats.max_depth)
                });
                deepest = deepest.max(stats.max_depth);
                n += 5;
            }
            // The deepest real bundle met nests 275 levels; this went past.
            assert!(deepest >= 275, "shape {shape}: {deepest}");
        }
        // Past the budget it is the parser that refuses.
        let src = nested(100);
        match compile_with_stats(&src, Goal::Script, 16 << 10) {
            Err(CompileError::Syntax(e)) => {
                assert_eq!(e.message, "Maximum call stack size exceeded");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
    });
}

#[test]
fn sloppy_arguments_map_to_the_parameters() {
    let u = unit("function f(a) { arguments; return a; }");
    let text = u.functions[1].disassemble();
    // The parameter moves into the environment, the object mapped to it.
    assert!(
        text.contains("CreateArguments 1") && text.contains("LdaSlot 0, 0"),
        "{text}"
    );
    // Strict, or with a default: unmapped, and the parameter stays.
    let strict =
        unit("function f(a) { 'use strict'; arguments; return a; }").functions[1].disassemble();
    assert!(
        strict.contains("CreateArguments 0") && strict.contains("Ldar r0"),
        "{strict}"
    );
    assert!(ops("function f(a = 1) { return arguments; }", 1).contains(&Op::CreateArguments));
    // An arrow's arguments are its function's.
    let u = unit("function f() { return () => arguments; }");
    assert!(u.functions[1].disassemble().contains("CreateArguments"));
    assert!(u.functions[2].disassemble().contains("LdaSlot 0, 0"));
}

#[test]
fn environment_depth_counts_with_and_function_names() {
    // `this` is never looked up in a with's object, but the object's
    // environment is one hop on the way out.
    let u = unit("function k(o) { with (o) { return () => this; } }");
    let arrow = u.functions[2].disassemble();
    assert!(arrow.contains("LdaSlot 1, 0"), "{arrow}");
    // A function's own name lives in its environment: an arrow inside
    // reads it there, not one further out.
    let u = unit("(function g() { return () => g; })");
    let arrow = u.functions[2].disassemble();
    assert!(arrow.starts_with("    0 LdaSlot 0, 0\n"), "{arrow}");
}

#[test]
fn operators_compile_as_their_meaning() {
    let ne = ops("a != b", 0);
    assert!(ne.contains(&Op::TestEq) && ne.contains(&Op::Not));
    let sne = ops("a !== b", 0);
    assert!(sne.contains(&Op::TestStrictEq) && sne.contains(&Op::Not));
    assert!(ops("a ?? b", 0).contains(&Op::JumpIfNotNullish));
    assert!(ops("a && b", 0).contains(&Op::JumpIfFalse));
    assert!(ops("a || b", 0).contains(&Op::JumpIfTrue));
    assert!(ops("a ||= b", 0).contains(&Op::JumpIfTrue));
    let post = ops("function f(x) { return x++; }", 1);
    assert!(post.contains(&Op::ToNumeric) && post.contains(&Op::Inc));
    assert!(!ops("function f(x) { return ++x; }", 1).contains(&Op::ToNumeric));
    assert!(ops("typeof undeclared", 0).contains(&Op::LdaGlobalTypeof));
    assert!(ops("delete x", 0).contains(&Op::DeleteName));
    assert!(ops("function f(x) { return delete x; }", 1).contains(&Op::LdaFalse));
}

#[test]
fn blocks_reset_their_tdz_and_jumps_leave_their_environments() {
    // Each entry of a loop's body block makes its `let` uninitialized.
    assert!(ops("function f() { for (;;) { g(x); let x; } }", 1).contains(&Op::LdaUninitialized));
    // A break out of a block with an environment pops it on the way.
    let u = unit("function f() { for (;;) { let x; g(() => x); if (x) break; } }");
    let pops = u.functions[1].disassemble().matches("PopContext").count();
    assert_eq!(pops, 2, "{}", u.functions[1].disassemble());
}

#[test]
fn a_return_through_finally_keeps_its_value() {
    assert_eq!(
        dis("function f(a) { try { return a; } finally { g(); } }", 1),
        concat!(
            // return a: the value kept, token 2, into the finally block.
            "    0 Ldar r0
    2 Star r2
    4 LdaSmi 2
    6 Star r1
    8 Jump -> 28
",
            // Falling out of the try block: token 0.
            "   13 LdaSmi 0
   15 Star r1
   17 Jump -> 28
",
            // The handler: the exception kept, token 1.
            "   22 Star r2
   24 LdaSmi 1
   26 Star r1
",
            // The finally block, then on as it was entered.
            "   28 LdaGlobal [0] \"g\", ic0
   31 Star r3
   33 CallUndefined r3, r4, 0, ic1
",
            "   38 LdaSmi 2
   40 TestStrictEq r1
   42 JumpIfFalse -> 50
   47 Ldar r2
   49 Return
",
            "   50 LdaSmi 1
   52 TestStrictEq r1
   54 JumpIfFalse -> 62
   59 Ldar r2
   61 Throw
",
            "   62 LdaUndefined
   63 Return
",
            "handler 0..13 -> 22
",
        )
    );
}

#[test]
fn the_validator_refuses_broken_units() {
    let good = unit("function f(a) { if (a) return g(a); return 1; }");
    let f = 1;
    let broken = |change: &dyn Fn(&mut CompiledUnit)| {
        let mut u = good.clone();
        change(&mut u);
        u.validate().is_err()
    };
    // Running off the end.
    assert!(broken(&|u| {
        u.functions[f].code.pop();
    }));
    assert!(broken(&|u| {
        let n = u.functions[f].code.len();
        u.functions[f].code.truncate(n - 1);
        u.functions[f].code.push(Op::LdaUndefined as u8);
    }));
    // A register, a constant, an inline cache or a function past the end.
    assert!(broken(&|u| u.functions[f].register_count = 0));
    assert!(broken(&|u| u.functions[f].constants.clear()));
    assert!(broken(&|u| u.functions[f].ic_slots = 0));
    // A global function declaration naming a function not there.
    assert!(broken(&|u| u.functions.truncate(1)));
    assert!(broken(&|u| {
        let code = &mut u.functions[0].code;
        code.clear();
        code.extend([Op::CreateClosure as u8, 9, Op::Return as u8]);
    }));
    // A jump into the middle of an instruction, and a bad handler.
    assert!(broken(&|u| {
        let code = &mut u.functions[f].code;
        let at = code
            .iter()
            .position(|b| *b == Op::JumpIfFalse as u8)
            .unwrap();
        code[at + 1] = code[at + 1].wrapping_add(1);
    }));
    assert!(broken(&|u| {
        u.functions[f].handlers.push(crate::compiler::Handler {
            start: 0,
            end: 1,
            target: 1,
            context: None,
        });
    }));
    assert!(good.validate().is_ok());
}
