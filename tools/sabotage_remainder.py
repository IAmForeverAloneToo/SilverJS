"""SilverJS's sabotage of the built-ins' remainders (J4.4c):
`Array.fromAsync`, `Object.groupBy`, `Promise.try`, the immutable
`Object.prototype`, IsArray of a revoked proxy, a proxy's traps from the
array methods and from for-in. One break at a time, `cargo test`,
restored byte for byte. Prints which tests each break fails, then every
test no break failed (which must be none).

    python -I tools/sabotage_remainder.py <repository root>

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
F = 'src/builtins/from_async.rs'
O = 'src/builtins/object.rs'
M = 'src/builtins/mod.rs'
X = 'src/interp/ops.rs'
cases = [
    # Array.fromAsync: its closes, the reference browser's.
    (F, '            (NEXT | MAPPED, true) => self.fa_close(run, v),',
     '            (NEXT | MAPPED, true) => self.fa_reject(run, v),'),
    (F, '            Err(Abrupt::Throw) => {\n                let e = self.caught();\n                self.fa_close(run, e)\n            }\n            Err(a) => Err(a),\n        }\n    }\n\n    /// An awaited iterator result',
     '            Err(Abrupt::Throw) => {\n                let e = self.caught();\n                self.fa_reject(run, e)\n            }\n            Err(a) => Err(a),\n        }\n    }\n\n    /// An awaited iterator result'),
    (F, '            Ok(m) => self.fa_await(run, m, MAPPED),\n            Err(Abrupt::Throw) => {\n                let e = self.caught();\n                self.fa_close(run, e)',
     '            Ok(m) => self.fa_await(run, m, MAPPED),\n            Err(Abrupt::Throw) => {\n                let e = self.caught();\n                self.fa_reject(run, e)'),
    (F, '            (CLOSING, _) => {\n                let e = self.fslot(run, S_ERROR);',
     '            (CLOSING, _) => {\n                let e = v;'),
    # Its steps.
    (F, '        if k >= len {', '        if k > len {'),
    (F, '                self.set_fslot(run, S_K, Value::number(k + 1.0));\n                self.fa_step(run)',
     '                self.fa_step(run)'),
    (F, '            self.async_from_sync_iterator(it, next)?', '            (it, next)'),
    (F, '        let a = if self.is_constructor_value(c.this) {\n            self.construct_value(c.this, &[], c.this)?',
     '        let a = if false {\n            self.construct_value(c.this, &[], c.this)?'),
    # Object.groupBy.
    (O, '                if group.is_undefined() {', '                if true {'),
    # The immutable Object.prototype.
    ('src/builtins/proxy.rs', '            if self.is_immutable_prototype(o) {', '            if false {'),
    (O, '        if agent.is_immutable_prototype(i) {\n            return Err(agent.type_error(messages::IMMUTABLE_PROTOTYPE));\n        }\n        if agent.is_proxy(i) {',
     '        if agent.is_proxy(i) {'),
    # Promise.try's promise as it is.
    ('src/builtins/promise.rs', '            let r = agent.promise_resolve(ctor, v);',
     '            let r = agent\n                .new_capability(ctor)\n                .and_then(|cap| agent.settle_capability(cap, v, false).map(|()| cap.promise));'),
    # IsArray, and a proxy's traps from the array methods.
    (M, '                    if target.is_null() {', '                    if false && target.is_null() {'),
    (M, '        if o.as_object().is_some_and(|i| self.is_proxy(i)) {\n            return self.define_or_throw(o, key, Descriptor::data(v));',
     '        if false {\n            return self.define_or_throw(o, key, Descriptor::data(v));'),
    (M, '            Some(i) => self.o_delete(i, key)?,', '            Some(i) => self.heap.delete(i, key),'),
    # for-in over a proxy.
    (X, '                    if proxy {\n                        names.push((key, Value::reference(RefKind::Object, o)));',
     '                    if false {\n                        names.push((key, Value::reference(RefKind::Object, o)));'),
    (X, '                    Some(d) => enumerable(&d),', '                    Some(_) => true,'),
    (X, '                    None => match self.o_get_prototype_of(p)?.as_object() {\n                        Some(up) => self.o_has_property(up, pk)?,\n                        None => false,\n                    },',
     '                    None => false,'),
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


FILTERS = ('remainder_tests::',)
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
