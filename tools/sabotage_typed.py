"""SilverJS's sabotage of binary data (J4.2). One
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
N = 'src/builtins/binary.rs'
AT = 'src/builtins/atomics.rs'
B64 = 'src/builtins/base64.rs'
V = 'src/interp/mod.rs'
G = 'src/builtins/global.rs'
cases = [
    # The elements' conversions.
    (D, '        n.round_ties_even() as u8', '        n.round() as u8'),
    (D, '        let m = (a * f64::powi(2.0, 24)).round_ties_even() as u16;', '        let m = (a * f64::powi(2.0, 24)).round() as u16;'),
    (D, '    if e16 >= 31 {', '    if e16 > 31 {'),
    (D, '        0 => frac * f64::powi(2.0, -24),', '        0 => frac * f64::powi(2.0, -25),'),
    (D, '            Elem::Int8 => f64::from(at(0) as i8),', '            Elem::Int8 => f64::from(at(0)),'),
    # A view's length: in bounds, tracking, detached.
    (D, '        if buf.detached {\n            return None;\n        }\n        let size = buf.bytes.len();\n        match v.length', '        let size = buf.bytes.len();\n        match v.length'),
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
    # J4.2b: DataView.
    (D, '        if buf.detached {\n            return None;\n        }\n        let size = buf.bytes.len();\n        let end = match v.length {', '        let size = buf.bytes.len();\n        let end = match v.length {'),
    (D, '        (v.offset <= size && end <= size).then(|| end - v.offset)', '        Some(end.saturating_sub(v.offset))'),
    (D, '                ObjectKind::ArrayBuffer | ObjectKind::SharedArrayBuffer', '                ObjectKind::ArrayBuffer'),
    (N, '        if !little {\n            bytes.reverse();\n        }\n        Ok(Value::number(elem.read(&bytes)))', '        let _ = little;\n        Ok(Value::number(elem.read(&bytes)))'),
    (N, '        if !little {\n            bytes.reverse();\n        }\n        if let Some(v) = self.heap.data_view(o) {', '        let _ = little;\n        if let Some(v) = self.heap.data_view(o) {'),
    (N, '        if i.checked_add(elem.size()).is_none_or(|end| end > size) {', '        if i.checked_add(elem.size()).is_none_or(|end| end > size + 1) {'),
    (N, '    let proto = agent.prototype_from(c.new_target, |r| r.intrinsics.data_view_prototype)?;\n    if agent.is_detached(b) {\n        return Err(agent.detached_error(M));\n    }', '    let proto = agent.prototype_from(c.new_target, |r| r.intrinsics.data_view_prototype)?;'),
    (N, '    if !length_arg.is_undefined() && length.is_some_and(|n| offset + n > size) {', '    if false && !length_arg.is_undefined() && length.is_some_and(|n| offset + n > size) {'),
    # SharedArrayBuffer.
    (N, '    let Some(new_len) = new_len.filter(|&n| n <= max && n >= old) else {', '    let Some(new_len) = new_len.filter(|&n| n <= max) else {'),
    (N, '    if n == b {', '    if false && n == b {'),
    (G, '        if self.shared_memory {', '        if true {'),
    # Atomics.
    (AT, '            elem == Elem::Int32\n', '            true\n'),
    (AT, '                Elem::Int8 | Elem::Uint8 | Elem::Int16', '                Elem::Uint8 | Elem::Int16'),
    (AT, '        let Some(i) = self.index_value(self.arg(c, 1))?.filter(|&i| i < len) else {', '        let Some(i) = self.index_value(self.arg(c, 1))? else {'),
    (AT, '        let fits = self.heap.view_length(o).is_some()\n            && v.is_some_and', '        let fits = v.is_some_and'),
    (AT, '                    .is_some_and(|b| at + v.elem.size() <= b.bytes.len())', '                    .is_some_and(|b| at <= b.bytes.len())'),
    (AT, '        let new = op(to_int32(old) as u32, to_int32(v) as u32);', '        let new = op(to_int32(v) as u32, to_int32(old) as u32);'),
    (AT, '    if bytes == want {', '    if bytes != want {'),
    (AT, '    Ok(Value::bool([1.0, 2.0, 4.0, 8.0].contains(&n)))', '    Ok(Value::bool([1.0, 2.0, 4.0].contains(&n)))'),
    (AT, '    if f64::from(v) != w {', '    if false {'),
    (AT, '    if t == 0.0 {', '    if false {'),
    (AT, ' && w.buffer == buffer && w.at == at {', ' && w.buffer == buffer {'),
    (AT, '            .is_some_and(|n| n.is_finite() && n.trunc() == n)', '            .is_some_and(|n| n.trunc() == n)'),
    (V, '            out.extend([w.buffer, w.promise]);', '            let _ = w;'),
    # Base64 and hex.
    (B64, '    matches!(u, 0x09 | 0x0a | 0x0c | 0x0d | 0x20)', '    matches!(u, 0x09 | 0x0c | 0x0d | 0x20)'),
    (B64, '    if strict && whole.get(keep).is_some_and(|&b| b != 0) {', '    if false {'),
    (B64, '                0x2b | 0x2f => return fail(out, BAD_CHARACTER),\n', ''),
    (B64, '                0x2d => 0x2b,\n', ''),
    (B64, '        if (remaining == 1 && chunk.len() == 2) || (remaining == 2 && chunk.len() == 3) {', '        if false {'),
    (B64, "        if c.len() > 2 {\n            out.push(at(n));\n        } else if !omit_padding {", "        if c.len() > 2 {\n            out.push(at(n));\n        } else if false {"),
    (B64, '        if self.heap.view(o).is_some_and(|v| v.elem != Elem::Uint8) {', '        if self.heap.view(o).is_some_and(|v| v.elem == Elem::Float64) {'),
    (B64, '        match text.and_then(|t| allowed.iter().position(|a| *a == t)) {', '        match text.and_then(|t| allowed.iter().position(|a| *a == t).or(Some(0))) {'),
    (B64, '    fn set_from(&mut self, o: u32, d: Decoded) -> Res<Value> {\n        self.heap.store_view_bytes(o, 0, &d.bytes);\n        if let Some(e) = d.error {\n            return Err(self.syntax_error(e));\n        }', '    fn set_from(&mut self, o: u32, d: Decoded) -> Res<Value> {\n        if let Some(e) = d.error {\n            return Err(self.syntax_error(e));\n        }\n        self.heap.store_view_bytes(o, 0, &d.bytes);'),
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
