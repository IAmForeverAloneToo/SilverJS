"""SilverJS's sabotage of the stack surface and direct eval (J2.5b): one
break at a time, `cargo test`, restored byte for byte. Prints which
tests each break fails, then every test no break failed (which must be
none).

    python -I tools/sabotage_stack.py <repository root>

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
ST = 'src/builtins/stack.rs'
EV = 'src/interp/eval.rs'
EX = 'src/compiler/codegen/expr.rs'
CG = 'src/compiler/codegen/mod.rs'
SC = 'src/compiler/scope.rs'
M = 'src/interp/mod.rs'
P = 'src/interp/ops.rs'
O = 'src/object.rs'
cases = [
    # Where frames point.
    (EX, '            } => i.span.start,\n            Expr::Member {\n                prop: MemberProp::Computed(e),',
        '            } => node.span.start,\n            Expr::Member {\n                prop: MemberProp::Computed(e),'),
    (EX, 'self.pos_at(self.ast.expr(*callee).span, span.start);', 'self.pos(self.ast.expr(*callee).span);'),
    (EX, 'self.pos_at(span, i.span.start);', 'self.pos(span);'),
    (EX, '            _ => node.span.end,\n', '            _ => node.span.start,\n'),
    (ST, 'pc: if running { self.vm.pc } else { f.pc },', 'pc: f.pc,'),
    (ST, '.map_or(0, |t| t.encode_utf16().count());\n        (line, column + 1)', '.map_or(0, str::len);\n        (line, column + 1)'),
    (ST, "                        b'\\n' => Some(i + 1),\n", ''),
    # Which frames are shown.
    (ST, '            if entry.native && skip_native {\n                skip_native = false;\n                continue;\n            }\n', ''),
    (ST, '            if entry.native && !self.native_shown(entry.callee) {\n                continue;\n            }\n', ''),
    (ST, 'Some(if n <= 0.0 { 0 } else { n.min(1e6) as usize })', 'Some(if n <= 0.0 { 0 } else { n.min(1e6) as usize + 1 })'),
    (ST, '            Got::Getter(_) => None,\n        }\n    }', '            Got::Getter(_) => Some(10),\n        }\n    }'),
    (ST, '                if entry.callee == until {', '                if false {'),
    # What a frame says.
    (ST, 'Some(format!("new {}", name.as_deref().unwrap_or("<anonymous>")))', 'name.clone()'),
    (ST, '} else if f.accessor || (!f.native && f.func == 0) {', '} else if !f.native && f.func == 0 {'),
    (ST, '        if receiver == self.realm().global {\n            return None;\n        }\n', ''),
    (ST, 'let n = name.or_else(|| self.method_name_of(f.receiver, f.callee));', 'let n = name;'),
    (ST, 'format!("eval at {caller_name} ({at}), <anonymous>:{line}:{column}")',
        'format!("eval at <anonymous> ({at}), <anonymous>:{line}:{column}")'),
    (ST, '        if function_ctor && f.func == 1 {\n            return Some("eval".to_owned());\n        }\n', ''),
    # The heading, the accessor and the data.
    (ST, '        let name = if name.is_undefined() {\n            "Error".encode_utf16().collect()',
        '        let name: Vec<u16> = if true {\n            "Error".encode_utf16().collect()'),
    (ST, '    if let Some(InternalCell::Stack(d)) = agent.heap.internals.get_mut(cell) {\n        d.formatted = text;\n    }\n', ''),
    (ST, '        && let Some(InternalCell::Stack(d)) = agent.heap.internals.get_mut(cell)\n    {\n        d.formatted = v;\n    }',
        '        && let Some(InternalCell::Stack(_)) = agent.heap.internals.get_mut(cell)\n    {\n        let _ = v;\n    }'),
    (O, '            keys.retain(|k| *k != PropertyKey::Symbol(h));', '            let _ = h;'),
    (ST, '        self.callable(callee)?;\n        let mut at = receiver.as_object();', '        let mut at = receiver.as_object();'),
    (ST, "        Some(Origin::Script(url)) if !url.is_empty() => agent.new_string(&url),\n        _ => Ok(Value::UNDEFINED),",
        "        _ => Ok(Value::UNDEFINED),"),
    # Direct eval.
    (M, 'if op == Op::CallEval && argc > 0 && callee == self.realm().intrinsics.eval', 'if false'),
    (EV, '                if clash {', '                if false {'),
    (EV, '            None => {\n                self.heap.define_own_property(\n                    g,\n                    key,\n                    Descriptor {\n                        value: Some(Value::UNDEFINED),\n                        writable: Some(true),\n                        enumerable: Some(true),\n                        configurable: Some(true),',
        '            None => {\n                self.heap.define_own_property(\n                    g,\n                    key,\n                    Descriptor {\n                        value: Some(Value::UNDEFINED),\n                        writable: Some(true),\n                        enumerable: Some(true),\n                        configurable: Some(false),'),
    (P, '            if let Some(o) = e.vars.as_object()\n                && self.heap.has_property(o, key)', '            if let Some(o) = e.vars.as_object().filter(|_| false)\n                && self.heap.has_property(o, key)'),
    (M, '                            if !sloppy_eval {\n                                top.fn_context = env;\n                            }', '                            top.fn_context = env;\n                            let _ = sloppy_eval;'),
    (EV, '                self.slot_named(frame.context, "new.target").is_some()', '                true'),
    (EX, 'let direct_eval = !*optional\n', 'let direct_eval = true\n'),
    (SC, '                || (scope.kind == ScopeKind::Eval && special)', ''),
    (P, '                    2 => Ok(()),\n', '                    2 => {\n                        self.set_slot(Some(env), slot, value);\n                        Ok(())\n                    }\n'),
    (P, '                if self.slot(Some(env), slot) == Value::UNINITIALIZED {\n                    let name = self.key_display(key);',
        '                if false {\n                    let name = self.key_display(key);'),
    (CG, '                (BindingKind::This, Loc::Slot(k)) => {\n                    self.op(Op::LdaThis);\n                    self.emit(Op::StaSlot, &[0, i64::from(k)]);\n                }\n                (BindingKind::NewTarget, Loc::Slot(k)) => {\n                    self.op(Op::LdaNewTarget);\n                    self.emit(Op::StaSlot, &[0, i64::from(k)]);\n                }\n                (kind, Loc::Register(r)) if kind.lexical() => {\n                    self.op(Op::LdaUninitialized);\n                    self.reg_op(Op::Star, r);\n                }\n                _ => {}\n            }\n        }\n        for b in &scope.bindings {\n            if let Some(g) = b.func {\n                self.closure(g, None)?;\n                if b.loc == Loc::Dynamic {',
        '                (BindingKind::This, Loc::Slot(_)) => {}\n                (BindingKind::NewTarget, Loc::Slot(k)) => {\n                    self.op(Op::LdaNewTarget);\n                    self.emit(Op::StaSlot, &[0, i64::from(k)]);\n                }\n                (kind, Loc::Register(r)) if kind.lexical() => {\n                    self.op(Op::LdaUninitialized);\n                    self.reg_op(Op::Star, r);\n                }\n                _ => {}\n            }\n        }\n        for b in &scope.bindings {\n            if let Some(g) = b.func {\n                self.closure(g, None)?;\n                if b.loc == Loc::Dynamic {'),
]

env = dict(os.environ)
quiet = env.get('QUIET_SH')


def wait_quiet():
    if quiet:
        bash = shutil.which('bash') or 'bash'
        subprocess.run([bash, quiet, 'wait'], capture_output=True)


SUITES = ('stack_tests',)
all_tests = set()
failed_ever = set()
wait_quiet()
r = subprocess.run(['cargo', 'test', '-q', '--lib', '--', '--list'], cwd=root,
                   capture_output=True, text=True, env=env)
for line in r.stdout.splitlines():
    if line.endswith(': test') and line.split('::')[0] in SUITES:
        all_tests.add(line[:-len(': test')])
print(f'{len(all_tests)} tests', flush=True)
for path, a, b in cases:
    full = root + path
    with open(full, 'rb') as f:
        src = f.read().decode('utf-8')
    if '\r\n' in src:
        a, b = a.replace('\n', '\r\n'), b.replace('\n', '\r\n')
    if src.count(a) != 1:
        print(f'NOT FOUND ({src.count(a)}) {path}: {a.strip()[:70]}', flush=True)
        continue
    shutil.copy(full, full + '.bak')
    with open(full, 'wb') as f:
        f.write(src.replace(a, b).encode('utf-8'))
    try:
        wait_quiet()
        r = subprocess.run(['cargo', 'test', '-q', '--lib', '--', 'stack_tests'],
                           cwd=root, capture_output=True, text=True, encoding='utf-8',
                           errors='replace', timeout=900, env=env)
        out = r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        out = 'test result: TIMEOUT'
    finally:
        shutil.move(full + '.bak', full)
        os.utime(full)
    if 'test result' not in out:
        print('DID NOT RUN ' + a.strip()[:60] + ' :: ' + out[-400:].replace('\n', ' '), flush=True)
        continue
    failed = set(re.findall(r'^test (\S+) \.\.\. FAILED', out, re.M))
    failed |= set(re.findall(r'^    (\S+)$', out, re.M)) & all_tests
    failed_ever |= failed
    tag = 'CAUGHT' if (r.returncode != 0 or 'TIMEOUT' in out) else 'MISSED'
    print(f'{tag} {a.strip()[:55]!r} -> {sorted(failed)}', flush=True)
print('never failed:', sorted(all_tests - failed_ever), flush=True)
