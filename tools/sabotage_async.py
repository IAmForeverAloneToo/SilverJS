"""SilverJS's sabotage of promises, the job queue and async functions
(J3.4a). One break at a time, `cargo test`, restored byte for byte.
Prints which tests each break fails, then every test no break failed
(which must be none).

    python -I tools/sabotage_async.py <repository root>

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
P = 'src/builtins/promise.rs'
G = 'src/interp/generator.rs'
M = 'src/interp/mod.rs'
A = 'src/interp/asyncfn.rs'
S = 'src/builtins/stack.rs'
E = 'src/compiler/codegen/expr.rs'
C = 'src/compiler/codegen/mod.rs'
I = 'src/interp/mod.rs'
PA = 'src/parser/mod.rs'
cases = [
    # Resolving.
    (P, '        for g in [f, other] {', '        for g in [f] {'),
    (P, '        if resolution == promise {', '        if false {'),
    (P, '        if resolution.as_object().is_none() {\n            return self.settle(promise, FULFILLED, resolution);',
        '        if resolution.as_object().is_none() || true {\n            return self.settle(promise, FULFILLED, resolution);'),
    (P, '        if !self.is_callable(then) {', '        if true {'),
    (P, '            let handler = if state == FULFILLED { at(0) } else { at(1) };', '            let handler = at(0);'),
    # Reactions and jobs.
    (P, '        if state == PENDING {\n            self.heap.with_internal_slots(o, |s| {', '        if false {\n            self.heap.with_internal_slots(o, |s| {'),
    (P, '                if self.is_async_driver(handler) {', '                if false && self.is_async_driver(handler) {'),
    (P, '                if handler.is_undefined() {\n                    return self.settle_capability(cap, argument, rejected);',
        '                if handler.is_undefined() {\n                    return self.settle_capability(cap, argument, false);'),
    (P, '                    Ok(v) => self.settle_capability(cap, v, false),', '                    Ok(v) => self.settle_capability(cap, v, true),'),
    (P, '            if ctor == c {', '            if false {'),
    # The constructor and statics.
    (P, '        return Err(agent.type_error(messages::PROMISE_WITHOUT_NEW));', '        return Ok(Value::UNDEFINED);'),
    (P, '    method("withResolvers", 0, with_resolvers),', '    method("withResolvers", 1, with_resolvers),'),
    (P, '        internal_id(if thrower_ { THROWER } else { VALUE_THUNK }),', '        internal_id(VALUE_THUNK),'),
    # The combinators.
    (P, '                left == 0', '                left <= 1'),
    (P, '                let left = s.get(1).and_then(|v| v.as_int()).unwrap_or(1) + 1;', '                let left = s.get(1).and_then(|v| v.as_int()).unwrap_or(1);'),
    (P, '    if slots.get(2).copied() == Some(Value::TRUE) {', '    if false {'),
    (P, '        if !cap.resolve.is_undefined() {\n            return Ok(cap);', '        if true {\n            return Ok(cap);'),
    # Async functions.
    (G, '        let promise = self.promise_resolve(c, v)?;\n        self.perform_then(promise, d, d, None);',
        '        let promise = self.promise_resolve(c, v)?;\n        self.perform_then(promise, d, Value::UNDEFINED, None);'),
    (G, '        self.set_driver_state(d, COMPLETED);\n        self.resolve_promise(promise, value)?;', '        self.set_driver_state(d, COMPLETED);\n        self.reject_promise(promise, value)?;'),
    (G, '        let how = if rejected {\n            Resume::Throw', '        let how = if false {\n            Resume::Throw'),
    (M, '            if abrupt == Abrupt::Throw && self.is_async_driver(frame.generator) {', '            if false && self.is_async_driver(frame.generator) {'),
    (M, '                            self.vm.acc = if self.is_async_driver(g) {', '                            self.vm.acc = if false {'),
    (A, '        let r = self.run_jobs();', '        let r = Ok::<(), Abrupt>(());'),
    (C, '            self.op(Op::AsyncStart);\n', ''),
    (E, '                let next = self.new_label();\n                self.jump_with(Op::ResumeYield, &[i64::from(m)], next);\n                self.bind(next);',
        '                let next = self.new_label();\n                self.bind(next);'),
    # Async stack traces and messages.
    (S, '        let mut driver = frames.first().map_or(Value::UNDEFINED, |f| f.generator);', '        let mut driver = Value::UNDEFINED;'),
    (S, '            format!("async {text}")', '            text'),
    (P, '            .find(|h| self.is_async_driver(*h))', '            .find(|h| self.is_async_driver(*h) && false)'),
    (I, '        if literal {', '        if false {'),
    (PA, '        if after_await && !self.cx.await_ && tok.kind != TokenKind::Eof {', '        if false {'),
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


FILTERS = ('async_tests',)
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
