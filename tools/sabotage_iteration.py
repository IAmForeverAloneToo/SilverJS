"""SilverJS's sabotage of iteration (J3.1): the iterator protocol's
instructions, for-of, spread and destructuring, and the iterator
built-ins. One break at a time, `cargo test`, restored byte for byte.
Prints which tests each break fails, then every test no break failed
(which must be none).

    python -I tools/sabotage_iteration.py <repository root>

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
M = 'src/interp/mod.rs'
I = 'src/interp/iter.rs'
O = 'src/interp/ops.rs'
B = 'src/builtins/iterator.rs'
J = 'src/builtins/object.rs'
P = 'src/compiler/codegen/pattern.rs'
S = 'src/compiler/codegen/stmt.rs'
E = 'src/compiler/codegen/expr.rs'
cases = [
    # The instructions.
    (I, '        if self.reg(base, d) == Value::TRUE {\n            return Ok(false);\n        }\n', ''),
    (I, '        // closed (section 7.4.8).\n        self.set_reg(base, d, Value::TRUE);\n', '        // closed (section 7.4.8).\n'),
    (I, '                self.set_reg(base, d, Value::FALSE);\n', ''),
    (I, '            None => {\n                self.vm.acc = acc;\n                Ok(false)', '            None => {\n                Ok(false)'),
    (I, '        if self.reg(base, d) == Value::TRUE {\n            return Ok(());\n        }\n        self.set_reg(base, d, Value::TRUE);\n',
        '        let _ = d;\n'),
    (I, '        self.vm.temps.pop();\n        self.vm.acc = acc;\n        closed', '        self.vm.temps.pop();\n        closed'),
    (I, 'if flags & 1 != 0 && n > 0 && callee == self.realm().intrinsics.eval {', 'if false {'),
    (M, '            self.vm.stack.extend(internal.iter().skip(2).copied());\n            self.vm.stack.extend(tail);', '            self.vm.stack.extend(tail);'),
    # The protocol.
    (I, '        if method.is_undefined() {\n            return Err(self.throw_error(ErrorType::TypeError, message()));',
        '        if false {\n            return Err(self.throw_error(ErrorType::TypeError, message()));'),
    (I, '        if iterator.as_object().is_none() {', '        if false {'),
    (I, '        let result = self.call_value(next, iterator, &[])?;\n        if result.as_object().is_none() {', '        let result = self.call_value(next, iterator, &[])?;\n        if false {'),
    (I, '        let result = self.call_value(method, iterator, &[])?;\n        if result.as_object().is_none() {', '        let result = self.call_value(method, iterator, &[])?;\n        if false {'),
    (I, '            if self.to_boolean(done) {', '            if !self.to_boolean(done) && false {'),
    (I, '        self.put(array, key, Value::number(f64::from(len) + 1.0))', '        self.put(array, key, Value::number(f64::from(len)))'),
    # The rest of an object pattern.
    (O, '            if excluded.contains(&key) {\n                continue;\n            }\n            if let Some(p)', '            if let Some(p)'),
    # The built-ins.
    (B, '    if index >= len {\n        agent.set_internal(c.this, 0, Value::UNDEFINED);', '    if index >= len {'),
    (B, '    let end = if pair { pos + 2 } else { pos + 1 };', '    let end = pos + 1;'),
    (B, '        self.vm.temps.push(value);\n        let made = self.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary));\n        self.vm.temps.pop();',
        '        let made = self.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary));'),
    (B, '    let Some(slots) = agent.internal_of(c.this, ObjectKind::ArrayIterator) else {', '    let Some(slots) = agent.internal_of(c.this, agent.heap.objects.get(c.this.as_object().unwrap_or(0)).map_or(ObjectKind::ArrayIterator, |c| c.kind())) else {'),
    (O, '        self.heap.define_own_property(o, key, hidden(values));\n', ''),
    (J, '            if let Err(e) = added {\n                agent.iterator_close_quiet(iterator)?;', '            if let Err(e) = added {'),
    # Code generation: closing.
    (S, '                    let (it, done) = (*it, *done);\n                    self.emit(Op::IteratorClose, &[i64::from(it), i64::from(done)]);\n                }\n                if let Some(r) = pop {',
        '                    let _ = (it, done);\n                }\n                if let Some(r) = pop {'),
    (S, '                    // A return\'s value stays in the accumulator.\n                    self.emit(Op::IteratorClose, &[i64::from(it), i64::from(done)]);',
        '                    let _ = (it, done);'),
    (S, '        self.emit(Op::IteratorCloseQuiet, &[i64::from(it), i64::from(done)]);\n        self.op(Op::Throw);\n        self.bind(end);',
        '        self.op(Op::Throw);\n        self.bind(end);'),
    (P, '        // Done with it before it was: closed.\n        self.emit(Op::IteratorClose, &[i64::from(it), i64::from(done)]);\n', ''),
    (P, '        self.emit(Op::IteratorCloseQuiet, &[i64::from(it), i64::from(done)]);\n        self.op(Op::Throw);\n        self.bind(past);',
        '        self.op(Op::Throw);\n        self.bind(past);'),
    # Code generation: patterns.
    (P, '        self.jump(Op::JumpIfNotUndefined, skip);\n        let name = match', '        self.jump(Op::JumpIfNotNullish, skip);\n        let name = match'),
    (P, '        let Pat::Expr(e) = &self.ast.pat(pat).kind else {\n            return Ok(Target::Later);\n        };',
        '        let Pat::Expr(_) = &self.ast.pat(pat).kind else {\n            return Ok(Target::Later);\n        };\n        if true {\n            return Ok(Target::Later);\n        }\n        let Pat::Expr(e) = &self.ast.pat(pat).kind else {\n            return Ok(Target::Later);\n        };'),
    (P, '                    if let Some(k) = keys {\n                        let c = self.string(&key);\n                        self.emit(Op::LdaConst, &[c]);\n                        self.reg_op(Op::ArrayPush, k);\n                    }\n', ''),
    (P, '        self.emit(Op::RequireObjectCoercible, &[n, plain]);\n', '        let _ = (n, plain);\n'),
    (P, '                    self.op(Op::LdaUndefined);\n                    self.iterator_next(it, next, done, got);', '                    self.iterator_next(it, next, done, got);'),
    (S, '                let mode = if lexical { Bind::Init } else { Bind::Var };', '                let mode = Bind::Var;'),
    # Code generation: spread, completions.
    (E, '                    if !dynamic && i > 0 {', '                    if false {'),
    (E, '                None if dynamic => self.reg_op(Op::ArrayPushHole, a),\n', ''),
    (S, '        if let Some(c) = self.completion\n            && matches!(', '        if let Some(c) = self.completion.filter(|_| false)\n            && matches!('),
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


FILTERS = ('iteration_tests',)
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
