"""SilverJS's sabotage of the iterator helpers (J4.4a): `Iterator`, its
prototype's methods, `from`, `concat`, `zip` and `zipKeyed`. One break at
a time, `cargo test`, restored byte for byte. Prints which tests each
break fails, then every test no break failed (which must be none).

    python -I tools/sabotage_iter.py <repository root>

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
I = 'src/builtins/iter_helpers.rs'
cases = [
    # The helpers' steps.
    (I, '                let n = self.count(h);\n                self.call_closing(arg, it, &[v, n]).map(Some)',
     '                let n = Value::int(0);\n                self.call_closing(arg, it, &[v, n]).map(Some)'),
    (I, '                if self.to_boolean(keep) {', '                if !self.to_boolean(keep) {'),
    (I, '                if left == 0.0 {\n                    self.iterator_close(it)?;',
     '                if left == 0.0 {'),
    (I, '                while left > 0.0 {', '                while left > 1.0 {'),
    # The generator states.
    (I, '        Some(RUNNING) => return Err(agent.type_error(messages::GENERATOR_RUNNING)),\n        Some(DONE) => return agent.iter_result(Value::UNDEFINED, true),\n        _ => {}\n    }\n    agent.set_hslot(h, S_STATE, Value::int(RUNNING));',
     '        Some(DONE) => return agent.iter_result(Value::UNDEFINED, true),\n        _ => {}\n    }\n    agent.set_hslot(h, S_STATE, Value::int(RUNNING));'),
    (I, '                let it = agent.hslot(h, S_ITER);\n                agent.iterator_close(it)?;',
     '                let it = agent.hslot(h, S_ITER);'),
    (I, '    agent.set_hslot(h, S_STATE, Value::int(RUNNING));\n    let closed = agent.helper_close(h);',
     '    agent.set_hslot(h, S_STATE, Value::int(DONE));\n    let closed = agent.helper_close(h);'),
    # The methods.
    (I, '        |a, _, r| a.to_boolean(r).then_some(Value::TRUE),', '        |a, _, r| a.to_boolean(r).then_some(Value::FALSE),'),
    (I, '        if !v.is_nullish() {', '        if true {'),
    (I, '        if left > 0.0 {\n            left -= 1.0;\n            continue;', '        if left > 1.0 {\n            left -= 1.0;\n            continue;'),
    (I, '            Some(n) if n < 0.0 || (n.is_finite() && n > super::typed::MAX_SAFE) => {',
     '            Some(n) if n.is_finite() && n > super::typed::MAX_SAFE => {'),
    (I, '        if agent.heap.same_value_zero(v, search) {', '        if v == search {'),
    (I, '        if n.is_finite() && n > super::typed::MAX_SAFE {', '        if false {'),
    (I, '    if has? {\n        return Ok(iterator);', '    if false && has? {\n        return Ok(iterator);'),
    (I, '            if item.as_object().is_none() {\n                return Err(agent.type_error("Iterator.concat called on non-object"));',
     '            if false {\n                return Err(agent.type_error("Iterator.concat called on non-object"));'),
    (I, '                                    if k != 0 {', '                                    if false {'),
    (I, '                "longest" => LONGEST,', '                "longest" => SHORTEST,'),
    (I, '        agent.vm.temps.push(padding);\n        agent.new_zip(iters, nexts, mode, padding, keys)',
     '        agent.new_zip(iters, nexts, mode, padding, keys)'),
    (I, '    if this == home {', '    if false {'),
    (I, '    if !m.is_undefined() {\n        agent.call_value(m, c.this, &[])?;\n    }\n    Ok(Value::UNDEFINED)',
     '    let _ = m;\n    Ok(Value::UNDEFINED)'),
    (I, '    if c.new_target == agent.realm().intrinsics.iterator {', '    if false {'),
    (I, '                let inner = self.flattenable(mapped, false, "Iterator.prototype.flatMap");',
     '                let inner = self.flattenable(mapped, true, "Iterator.prototype.flatMap");'),
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


FILTERS = ('iter_tests::',)
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
