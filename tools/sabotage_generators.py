"""SilverJS's sabotage of generators (J3.2): frames suspended and
resumed, next, return and throw, yield*, and the generator built-ins.
One break at a time, `cargo test`, restored byte for byte. Prints which
tests each break fails, then every test no break failed (which must be
none).

    python -I tools/sabotage_generators.py <repository root>

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
G = 'src/interp/generator.rs'
M = 'src/interp/mod.rs'
O = 'src/interp/ops.rs'
C = 'src/compiler/codegen/mod.rs'
E = 'src/compiler/codegen/expr.rs'
S = 'src/builtins/stack.rs'
GL = 'src/builtins/global.rs'
F = 'src/builtins/function.rs'
cases = [
    # Resuming.
    (G, '        if state == EXECUTING {', '        if false {'),
    (G, 'if state == COMPLETED || (state == SUSPENDED_START && how != Resume::Next) {', 'if state == COMPLETED {'),
    (G, '                Resume::Return => self.iter_result(sent, true).map(Some),', '                Resume::Return => self.iter_result(Value::UNDEFINED, true).map(Some),'),
    (G, '                *slot = Value::int(how as i32);', '                *slot = Value::int(0);'),
    (G, '            self.vm.acc = sent;\n', ''),
    (G, '            cell.set_internal(STATE, Value::int(EXECUTING));', '            cell.set_internal(STATE, Value::int(SUSPENDED_YIELD));'),
    (G, '        let result = self.iter_result(value, true)?;\n        self.set_generator_state(g, COMPLETED);',
        '        let result = self.iter_result(value, true)?;'),
    (G, '        self.vm.stack.truncate(frame.args as usize);\n        self.pop_resume_frame();', '        self.vm.stack.truncate(frame.args as usize);'),
    (G, '            self.iter_result(v, false)?', '            v'),
    # yield*.
    (G, '                    self.iterator_close(iterator)?;\n', ''),
    (G, '                    self.set_reg(base, outcome, Value::int(2));\n', ''),
    (G, '            Ok(if how == 2 { 2 } else { 1 })', '            Ok(1)'),
    (G, '        if self.reg(base, outcome) != Value::int(3) {\n            return Ok(());\n        }\n        let result = self.vm.acc;\n        if result.as_object().is_none() {',
        '        if self.reg(base, outcome) != Value::int(3) {\n            return Ok(());\n        }\n        let result = self.vm.acc;\n        if false {'),
    # The loop.
    (M, '                        if how == Value::int(generator::Resume::Throw as i32) {', '                        if false {'),
    (M, '                        } else if how == Value::int(generator::Resume::Return as i32) {', '                        } else if false {'),
    (M, '                            let (g, v) = (top.generator, self.vm.acc);\n                            self.vm.acc = tri!(self.generator_return(g, v), start);',
        '                            let _ = top;'),
    (M, '            if frame.generator != Value::UNDEFINED {\n                self.set_generator_state(frame.generator, generator::COMPLETED);',
        '            if false {\n                self.set_generator_state(frame.generator, generator::COMPLETED);'),
    (M, '                if let Some(how) = crate::builtins::generator_resume(id) {', '                if let Some(how) = crate::builtins::generator_resume(id).filter(|_| false) {'),
    (M, '                            if let Some(top) = self.vm.frames.last_mut() {\n                                top.caller_realm = outer;',
        '                            if let Some(top) = self.vm.frames.last_mut().filter(|_| false) {\n                                top.caller_realm = outer;'),
    # Compiling.
    (C, '        if f.is_generator {\n            self.op(Op::GeneratorStart);\n        }\n', ''),
    (C, '        let kind = if f.is_generator {\n            FunctionKind::Generator\n        } else {\n            kind\n        };',
        '        let kind = if false {\n            FunctionKind::Generator\n        } else {\n            kind\n        };'),
    (C, '                        if f.is_generator {\n                            FunctionKind::Generator', '                        if false {\n                            FunctionKind::Generator'),
    (E, '            self.bind(ret);\n            self.jump_out(super::Completion::Return);\n            self.bind(past);', '            self.bind(ret);\n            self.bind(past);'),
    (E, '        self.emit(Op::DelegateCheck, &[i64::from(mode), i64::from(outcome)]);\n', ''),
    # Making them.
    (O, '        let generator = kind == FunctionKind::Generator;\n        let proto = if generator {', '        let generator = kind == FunctionKind::Generator;\n        let proto = if false {'),
    (O, '        if generator && result.is_ok() {', '        if false && result.is_ok() {'),
    (S, '        if let Some(f) = self.generator_function(receiver) {', '        if let Some(f) = self.generator_function(receiver).filter(|_| false) {'),
    (GL, '        if let Some(o) = gf.as_object() {\n            self.heap.set_prototype_of(o, function);\n        }', '        let _ = (gf, function);'),
    (F, '    dynamic_function(agent, c, "function*", |r| {', '    dynamic_function(agent, c, "function", |r| {'),
]

env = dict(os.environ)
quiet = env.get('QUIET_SH')


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
        bash = shutil.which('bash') or 'bash'
        subprocess.run([bash, quiet, 'wait'], capture_output=True)


FILTERS = ('generator_tests', 'realm_tests::a_generator', 'builtins_tests::the_generator')
all_tests = set()
failed_ever = set()
wait_quiet()
r = subprocess.run(['cargo', 'test', '-q', '--lib', '--', '--list'], cwd=root,
                   capture_output=True, text=True, env=env)
for line in r.stdout.splitlines():
    name = line[:-len(': test')]
    if line.endswith(': test') and any(name.startswith(f) for f in FILTERS):
        all_tests.add(name)
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
        r = subprocess.run(['cargo', 'test', '-q', '--lib', '--', *FILTERS],
                           cwd=root, capture_output=True, text=True, encoding='utf-8',
                           errors='replace', timeout=900, env=env)
        out = r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        out = 'test result: TIMEOUT'
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
    tag = 'CAUGHT' if (r.returncode != 0 or 'TIMEOUT' in out) else 'MISSED'
    print(f'{tag} {a.strip()[:55]!r} -> {sorted(failed)}', flush=True)
print('never failed:', sorted(all_tests - failed_ever), flush=True)
