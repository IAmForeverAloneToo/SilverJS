"""SilverJS's sabotage of the buffers and typed arrays (J4.2a). One
break at a time, `cargo test`, restored byte for byte. Prints which tests
each break fails, then every test no break failed (which must be none).

    python -I tools/sabotage_typed.py <repository root>

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
D = 'src/buffer.rs'
O = 'src/object.rs'
T = 'src/builtins/typed.rs'
P = 'src/builtins/proxy.rs'
I = 'src/builtins/iterator.rs'
A = 'src/agent.rs'
cases = [
    # The elements' conversions.
    (D, '        n.round_ties_even() as u8', '        n.round() as u8'),
    (D, '        let m = (a * f64::powi(2.0, 24)).round_ties_even() as u16;', '        let m = (a * f64::powi(2.0, 24)).round() as u16;'),
    (D, '    if e16 >= 31 {', '    if e16 > 31 {'),
    (D, '        0 => frac * f64::powi(2.0, -24),', '        0 => frac * f64::powi(2.0, -25),'),
    (D, '            Elem::Int8 => f64::from(at(0) as i8),', '            Elem::Int8 => f64::from(at(0)),'),
    # A view's length: in bounds, tracking, detached.
    (D, '        if buf.detached {\n            return None;\n        }\n        let size', '        let size'),
    (D, '            Some(n) => (v.offset + n * v.elem.size() <= size).then_some(n),', '            Some(n) => Some(n),'),
    (D, '            None => (v.offset <= size).then(|| (size - v.offset) / v.elem.size()),', '            None => Some(size.saturating_sub(v.offset) / v.elem.size()),'),
    (D, '        if i >= len {\n            return;\n        }\n        let Some(v) = self.view(o) else {', '        let _ = len;\n        let Some(v) = self.view(o) else {'),
    # Canonical numeric keys.
    (D, "                if units == [u16::from(b'-'), u16::from(b'0')] {\n                    return Some(None);", "                if units == [u16::from(b'-'), u16::from(b'0')] {\n                    return None;"),
    (D, '                if !back.encode_utf16().eq(units.iter().copied()) {\n                    return None;\n                }', '                let _ = back;'),
    # The integer-indexed exotic object.
    (O, '                let v = i.and_then(|i| self.view_get(o, i));\n                return Got::Value(v.map_or(Value::UNDEFINED, Value::number));', '                if let Some(v) = i.and_then(|i| self.view_get(o, i)) {\n                    return Got::Value(Value::number(v));\n                }'),
    (O, '                if !self.view_valid(o, i) {\n                    return SetOutcome::Done(true);\n                }', '                let _ = i;'),
    (O, '            || desc.writable == Some(false)\n', '\n'),
    (O, '            return !self.view_valid(o, i);', '            return true || !self.view_valid(o, i);'),
    (O, '            all.extend(keys);\n            keys = all;', '            all.clear();\n            all.extend(keys);\n            keys = all;'),
    (O, '        if !self.fixed_length(o) {\n            return false;\n        }', '        let _ = self.fixed_length(o);'),
    (T, '            self.heap.view_set(t, i, n);', '            let _ = (t, i, n);'),
    (P, '                    self.heap.view_set(o, i, n);', '                    let _ = (o, i, n);'),
    (P, '                return Ok(self.heap.view_valid(at, i));', '                return Ok(true || self.heap.view_valid(at, i));'),
    (I, '    let view = array.as_object().filter(|&o| agent.heap.view(o).is_some());', '    let view: Option<u32> = None;'),
    # The methods.
    (D, '        if keep && from < to && to < from + count {', '        if false && keep && from < to && to < from + count {'),
    (T, '            && s.elem == d.elem', '            && s.elem != d.elem'),
    (T, '                Some(m) if m <= MAX_BYTES && len <= m => max = Some(m),', '                Some(m) => max = Some(m),'),
    (T, '        buf.bytes.resize(new_len, 0);', '        buf.bytes.truncate(new_len);'),
    (T, '        if self.reserve(len + 128).is_err() || bytes', '        if bytes'),
    (T, '            buf.detached = true;', '            let _ = &buf.detached;'),
    # A request over half the live heap does not collect at every
    # allocation.
    (A, '            self.next_collection = self.next_collection.max((after as f64 * GROWTH) as usize);', '            let _ = after;'),
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


FILTERS = ('typed_tests', 'interp_tests::building_a_string_copies_linearly')
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
