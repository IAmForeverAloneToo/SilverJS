"""SilverJS's compiler sabotage: one break at a time, `cargo test`, restored
byte for byte. Prints which tests each break fails, then every test no
break failed (which must be none).

    python -I tools/sabotage_compiler.py <repository root>

With QUIET_SH set to the path of a quiet-machine script, each build first
runs `bash $QUIET_SH wait`, so that it waits while another job needs the
machine quiet.
"""
import os
import re
import shutil
import subprocess
import sys

root = sys.argv[1].replace('\\', '/').rstrip('/') + '/'
S = 'src/compiler/scope.rs'
B = 'src/compiler/bytecode.rs'
M = 'src/compiler/codegen/mod.rs'
E = 'src/compiler/codegen/expr.rs'
T = 'src/compiler/codegen/stmt.rs'
U = 'src/compiler/mod.rs'
N = 'src/number.rs'
cases = [
    # Scope analysis.
    (S, "self.a.scope_mut(c).bindings[i as usize].captured = true;", "let _ = i;"),
    (S, "scope.dynamic |= !strict;", "let _ = strict;"),
    (S, "if b.kind == BindingKind::Param {\n                    b.captured = true;",
        "if false {\n                    b.captured = true;"),
    (S, "Loc::Register(p)\n", "Loc::Register(p + 1)\n"),
    (S, "s.has_env = !s.slots.is_empty() || s.has_eval || s.kind == ScopeKind::With;",
        "s.has_env = !s.slots.is_empty() || s.has_eval;"),
    (S, "depth += 1;", "depth += 0;"),
    (S, "ScopeKind::FuncName { .. } => depth.saturating_sub(1),", "ScopeKind::FuncName { .. } => depth,"),
    (S, '"arguments" => Some(BindingKind::Arguments),', '"arguments" => None,'),
    (S, "(ScopeKind::Script | ScopeKind::Eval | ScopeKind::Function { .. }, _)\n                | (_, None) => return s,",
        "(ScopeKind::Script | ScopeKind::Eval, _)\n                | (_, None) => return s,"),
    (S, '&& &*i.name == "eval"', '&& &*i.name == "evil"'),
    (S, "self.a.functions[fi].mapped_arguments = true;", ""),
    (S, "BindingKind::Let | BindingKind::Const | BindingKind::Class\n", "BindingKind::Let | BindingKind::Class\n"),
    # Code generation.
    (M, "!(r.same_function && !r.in_switch && at >= r.tdz_end)", "!(r.same_function && at >= r.tdz_end)"),
    (M, "self.reg_op(Op::Ldar, reg);\n                self.tdz_acc(&r, name, span);",
        "self.reg_op(Op::Ldar, reg);"),
    (M, "Some(BindingKind::Const) if !init && r.place != Place::Global =>",
        "Some(BindingKind::Const) if false =>"),
    (M, "(BindingKind::Const, _) => GlobalDecl::Const(name),", "(BindingKind::Const, _) => GlobalDecl::Let(name),"),
    (M, "                self.op(Op::LdaUninitialized);\n                self.reg_op(Op::Star, r);\n            }\n        }",
        "                self.reg_op(Op::Star, r);\n            }\n        }"),
    (M, "let info = self.scope_info(fs);\n            self.emit(Op::CreateFunctionContext, &[info]);",
        "let info = self.scope_info(fs);\n            let _ = info;"),
    (M, "                        self.reg_op(Op::Ldar, p);\n", ""),
    (M, "(None, Some(n)) => n.into(),", "(None, Some(_)) => Box::default(),"),
    (M, "let offset = i64::from(*target) - i64::from(*start);", "let offset = i64::from(*target);"),
    (M, "self.handlers.push(h);", "let _ = h;"),
    (E, "Expr::Call { callee, .. } => self.unparen(*callee) == link,", "Expr::Call { .. } => false,"),
    (E, 'matches!(&callee_node.kind, Expr::Ident(i) if &*i.name == "eval");', "false;"),
    (E, "BinaryOp::NotEq => (Op::TestEq, true),", "BinaryOp::NotEq => (Op::TestEq, false),"),
    (E, "                        LogicalOp::Coalesce => Op::JumpIfNotNullish,\n                    },\n                    end,",
        "                        LogicalOp::Coalesce => Op::JumpIfNullish,\n                    },\n                    end,"),
    (E, "if !dynamic && matches!(elems.last(), Some(None)) {", "if false {"),
    (E, "Expr::Member { prop, optional, .. } => {\n                    if *optional && let Some(t) = target {\n                        self.jump(Op::JumpIfNullish, t);\n                    }",
        "Expr::Member { prop, .. } => {"),
    (E, "self.op(Op::ToNumeric);\n                    let old = self.alloc();", "let old = self.alloc();"),
    (T, "let per_iteration = save.save.is_some();", "let per_iteration = false;"),
    (T, "if target == Completion::Return {\n                        self.reg_op(Op::Star, value);\n                    }", ""),
    (T, "if let Some(r) = pop {\n                    self.reg_op(Op::PopContext, r);\n                }\n                let label",
        "let _ = pop;\n                let label"),
    (T, "Some(Control::Context { save }) => pop = Some(*save),", "Some(Control::Context { .. }) => {}"),
    (T, "self.reg_op(Op::StoreContext, r);\n            Some(r)", "let _ = r;\n            None"),
    # Encoding.
    (B, "(_, 1) => u8::try_from(value).is_ok(),", "(_, 1) => u16::try_from(value).is_ok(),"),
    (B, "(Operand::Imm, 1) => i8::try_from(value).is_ok(),", "(Operand::Imm, 1) => u8::try_from(value).is_ok(),"),
    (B, "*slot = if matches!(kind, Operand::Imm | Operand::Jump) {", "*slot = if matches!(kind, Operand::Imm) {"),
    (B, "if matches!(first, Op::Wide | Op::ExtraWide) {\n            return None;", "if false {\n            return None;"),
    # The validator.
    (U, "if !matches!(last, Some(Op::Return | Op::Throw | Op::Jump | Op::JumpLoop)) {",
        "if !matches!(last, Some(_)) {"),
    (U, "&& (*i == 0 || *i as usize >= functions)", "&& false"),
    # Numbers' text.
    (N, "if k <= n && n <= 21 {", "if k <= n && n < 21 {"),
    (N, "} else if -6 < n && n <= 0 {", "} else if -7 < n && n <= 0 {"),
]
env = dict(os.environ, CARGO_BUILD_JOBS='4')
quiet = os.environ.get('QUIET_SH')


# Every case's pattern must stand exactly once in its file: a case that
# no longer matches its code would look like a check and prove nothing.
# Checked before any build; any such case fails the pass.
stale = []
for path, a, _ in cases:
    with open(root + path, 'rb') as f:
        text = f.read().decode('utf-8').replace('\r\n', '\n')
    if text.count(a) != 1:
        stale.append(f'{path}: found {text.count(a)} times: {a.strip()[:70]!r}')
if stale:
    print('STALE CASES: each pattern must match its code exactly once', flush=True)
    for line in stale:
        print('  ' + line, flush=True)
    sys.exit(2)


def wait_quiet():
    if quiet:
        # Resolved on PATH: on Windows a bare 'bash' would be looked up in
        # the system directory first.
        bash = shutil.which('bash') or 'bash'
        subprocess.run([bash, quiet, 'wait'], check=True)


all_tests = set()
failed_ever = set()
wait_quiet()
r = subprocess.run(['cargo', 'test', '-q', '--lib', '--', '--list'], cwd=root,
                   capture_output=True, text=True, env=env)
for line in r.stdout.splitlines():
    # The compiler's tests, and the number formatting it folds constants
    # with (number.rs's other tests are the built-ins' pass's).
    name = line[:-len(': test')]
    if line.endswith(': test') and (name.startswith('compiler_tests::')
                                    or name == 'number::tests::numbers_print_as_the_spec_says'):
        all_tests.add(line[:-len(': test')])
print(f'{len(all_tests)} tests', flush=True)
for path, a, b in cases:
    full = root + path
    with open(full, 'rb') as f:
        src = f.read().decode('utf-8')
    if '\r\n' in src:
        a, b = a.replace('\n', '\r\n'), b.replace('\n', '\r\n')
    if src.count(a) != 1:
        sys.exit(f'STALE CASE ({src.count(a)}) {path}: {a.strip()[:70]}')
    shutil.copy(full, full + '.bak')
    with open(full, 'wb') as f:
        f.write(src.replace(a, b).encode('utf-8'))
    try:
        wait_quiet()
        r = subprocess.run(['cargo', 'test', '-q', '--lib', '--', 'compiler_tests', 'number'],
                           cwd=root, capture_output=True, text=True, encoding='utf-8',
                           errors='replace', timeout=900, env=env)
        out = r.stdout + r.stderr
    finally:
        shutil.move(full + '.bak', full)
        os.utime(full)
    if 'test result' not in out and 'could not compile' not in out and r.returncode != 0:
        # The test process died (a stack overflow, an abort): caught.
        print(f'CAUGHT (the tests crashed) {a.strip()[:55]!r}', flush=True)
        continue
    if 'test result' not in out:
        print('DID NOT RUN ' + a.strip()[:60] + ' :: ' + out[-400:].replace('\n', ' '), flush=True)
        continue
    failed = set(re.findall(r'^test (\S+) \.\.\. FAILED', out, re.M))
    failed |= set(re.findall(r'^    (\S+)$', out, re.M)) & all_tests
    failed_ever |= failed
    tag = 'CAUGHT' if r.returncode != 0 else 'MISSED'
    print(f'{tag} {a.strip()[:55]!r} -> {sorted(failed)}', flush=True)
print('never failed:', sorted(all_tests - failed_ever), flush=True)
