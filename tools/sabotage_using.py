"""SilverJS's sabotage of explicit resource management (J4.4b): `using`
and `await using`, `DisposableStack`, `AsyncDisposableStack`,
`SuppressedError` and `%AsyncIteratorPrototype%[Symbol.asyncDispose]`.
One break at a time, `cargo test`, restored byte for byte. Prints which
tests each break fails, then every test no break failed (which must be
none).

    python -I tools/sabotage_using.py <repository root>

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
D = 'src/builtins/disposable.rs'
G = 'src/compiler/codegen/stmt.rs'
cases = [
    # The resources, last first.
    (D, '        let at = n - 3;', '        let at = S_RESOURCES;'),
    # An await using of null: its await, and the one before a sync resource.
    (D, '        if hint & ASYNC == 0 && needs_await && !has_awaited {',
     '        if false && needs_await && !has_awaited {'),
    (D, '            self.set_dslot(cap, S_NEEDS_AWAIT, Value::TRUE);\n            return Ok(Step::Next);',
     '            return Ok(Step::Next);'),
    (D, '        if v.is_nullish() {\n            if is_async {', '        if false {\n            if is_async {'),
    (D, '        if v.as_object().is_none() {\n            return Err(self.type_error(NOT_OBJECT));',
     '        if false {\n            return Err(self.type_error(NOT_OBJECT));'),
    # The kept throw, and a SuppressedError over it.
    (D, '        let kept = if self.dslot(cap, S_THREW) == Value::TRUE {', '        let kept = if false {'),
    (D, '            for (name, v) in [("error", error), ("suppressed", suppressed)] {',
     '            for (name, v) in [("error", suppressed), ("suppressed", error)] {'),
    # The stacks.
    (D, '            self.call_value(m, Value::UNDEFINED, &[v])', '            self.call_value(m, v, &[])'),
    (D, '        self.set_dslot(s, S_STATE, Value::int(DISPOSED));\n        Ok(n)', '        Ok(n)'),
    (D, '    agent.set_dslot(s, S_STATE, Value::int(DISPOSED));\n    agent.dispose_all(s)?;',
     '    agent.dispose_all(s)?;'),
    (D, '                        return self.reject_promise(promise, e);',
     '                        let _ = e;\n                        return self.resolve_promise(promise, Value::UNDEFINED);'),
    (D, '    agent.dispose_throw(cap, e)?;\n    agent.dispose_async_run(cap, promise)?;',
     '    let _ = e;\n    agent.dispose_async_run(cap, promise)?;'),
    # The codegen.
    (G, '        self.reg_op(Op::Ldar, value);\n        self.reg_op(Op::DisposeThrow, cap);\n        self.bind(steps);',
     '        self.bind(steps);'),
    (G, '        self.reg_op(Op::DisposeEnd, cap);', '        let _ = cap;'),
    (G, '            self.reg_op(Op::Ldar, awaited);\n            self.await_acc(Span::new(0, 0));',
     '            self.reg_op(Op::Ldar, awaited);'),
    (G, '                    if let Some(v) = kept {\n                        g.reg_op(Op::Ldar, v);\n                    }',
     '                    let _ = kept;'),
    (G, '            self.emit(Op::DisposeAdd, &[i64::from(cap), hint]);\n            self.store_name(&name.name, node.span, true)?;',
     '            let _ = (cap, hint);\n            self.store_name(&name.name, node.span, true)?;'),
    # The rest.
    ('src/builtins/generator.rs', '        agent.perform_then(wrapped, unwrap?, Value::UNDEFINED, Some(cap));',
     '        let _ = (unwrap?, cap);\n        agent.resolve_promise(promise, Value::UNDEFINED)?;'),
    ('src/builtins/error.rs', '        for (k, name) in [(0, "error"), (1, "suppressed")] {',
     '        for (k, name) in [(1, "error"), (0, "suppressed")] {'),
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


FILTERS = ('using_tests::',)
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
