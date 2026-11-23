"""SilverJS's sabotage of the collections and weak references (J4.1). One
break at a time, `cargo test`, restored byte for byte. Prints which tests
each break fails, then every test no break failed (which must be none).

    python -I tools/sabotage_collections.py <repository root>

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
T = 'src/table.rs'
H = 'src/heap.rs'
C = 'src/builtins/collection.rs'
A = 'src/agent.rs'
J = 'src/interp/asyncfn.rs'
B = 'src/builtins/mod.rs'
cases = [
    # The table: keys by SameValueZero, holes, chains, compaction.
    (T, '        } else if n == 0.0 {\n            0', '        } else if false {\n            0'),
    (T, '        return x == y || (x.is_nan() && y.is_nan());', '        return x == y;'),
    (T, '            (Some(x), Some(y)) => x.same_units(y),', '            (Some(x), Some(y)) => std::ptr::eq(x, y),'),
    (T, '            if let Some(b) = self.buckets.get_mut(h) {\n                *b = after;\n            }', '            if let Some(b) = self.buckets.get_mut(h) {\n                *b = NONE;\n            }'),
    (T, '        self.live -= 1;\n    }', '    }'),
    (T, '        self.entries.fill((Value::HOLE, Value::HOLE));', '        let _ = &self.entries;'),
    (T, '            .find(|(_, e)| e.0 != Value::HOLE)', '            .find(|_| true)'),
    (T, '        self.rehash(size);\n        before', '        let _ = size;\n        before'),
    (T, '                cell.set_internal(1, Value::number((at - shift) as f64));', '                cell.set_internal(1, Value::number(at as f64 + 0.0 * shift as f64));'),
    (T, '            table.iterators.push(o);', '            let _ = o;'),
    # The collector: ephemerons, weak tables, registries.
    (H, '                        if k != Value::HOLE && self.kept(k) && !self.kept(v) {', '                        if false && k != Value::HOLE && self.kept(k) && !self.kept(v) {'),
    (H, '                            InternalCell::Table(t) if t.strength != Strength::Strong => {', '                            InternalCell::Table(t) if false && t.strength != Strength::Strong => {'),
    (H, '                        t.cleared.push(held);', '                        let _ = held;'),
    (A, '            self.vm\n                .jobs\n                .push_back(crate::builtins::promise::Job::Cleanup { table });', '            let _ = table;'),
    (J, '        self.vm.kept_alive.clear();', '        let _ = &self.vm.kept_alive;'),
    # The built-ins.
    (C, '    if key.as_number() == Some(0.0) {', '    if false {'),
    (C, '            Some((RefKind::Symbol, _)) => !self.vm.registry.values().any(|&s| s == v),', '            Some((RefKind::Symbol, _)) => true,'),
    (C, '                cell.set_internal(0, Value::UNDEFINED);', '                cell.set_internal(0, t);'),
    (C, '                self.call_value(callback, this_arg, &[v, k, c.this])?;', '                self.call_value(callback, this_arg, &[k, v, c.this])?;'),
    (C, '        if item.as_object().is_none() {\n            let text = self.display(item);', '        if false {\n            let text = self.display(item);'),
    (C, '        if size < 0.0 {', '        if false {'),
    (C, '                if agent.heap.table_find(t, v).is_some() {\n                    agent.heap.table_delete(rt, v);\n                } else {', '                if agent.heap.table_find(t, v).is_none() {\n                    agent.heap.table_delete(rt, v);\n                } else {'),
    (C, '    if crate::object::same_value(&agent.heap, target, held) {', '    if false {'),
    (C, '        Some((_, target, _)) => {', '        Some((_, _target, _)) => {\n            let target = Value::UNDEFINED;'),
    (C, '    agent.heap.table_set(t, k, v);\n    Ok(v)\n}\n\n/// `Map.prototype.getOrInsertComputed`', '    let _ = (t, k);\n    Ok(v)\n}\n\n/// `Map.prototype.getOrInsertComputed`'),
    # The reference browser's names for objects in messages.
    (B, '                return format!("#<{name}>");', '                return format!("#<Object{}>", name.len() * 0);'),
    (B, '            Some(ObjectKind::Array) => "Array",', '            Some(ObjectKind::Array) => "Object",'),
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


FILTERS = ('collection_tests',)
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
