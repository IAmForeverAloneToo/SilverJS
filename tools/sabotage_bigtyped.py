"""SilverJS's sabotage of the BigInt typed arrays (J4.3c):
BigInt64Array and BigUint64Array, DataView's BigInt accessors and Atomics
on BigInt elements. One break at a time, `cargo test`, restored byte for
byte. Prints which tests each break fails, then every test no break
failed (which must be none).

    python -I tools/sabotage_bigtyped.py <repository root>

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
BU = 'src/buffer.rs'
BI = 'src/bigint.rs'
TY = 'src/builtins/typed.rs'
OB = 'src/object.rs'
PX = 'src/builtins/proxy.rs'
DV = 'src/builtins/binary.rs'
AT = 'src/builtins/atomics.rs'
OBJ = 'src/builtins/object.rs'
cases = [
    # The elements' bytes and their reading.
    (BU, '            Elem::Float64 | Elem::BigUint64 | Elem::BigInt64 => 8,',
     '            Elem::Float64 => 8,\n            Elem::BigUint64 | Elem::BigInt64 => 4,'),
    (BU, '            Elt::Big(bits) if self.is_bigint() => return put(&bits.to_le_bytes()),',
     '            Elt::Big(bits) if self.is_bigint() => return put(&bits.to_be_bytes()),'),
    (BI, '        if self.neg { m.wrapping_neg() } else { m }', '        m'),
    (BI, '        if self.neg && !self.is_zero() || self.mag.len() > 2 {',
     '        if self.mag.len() > 2 {'),
    (BU, '        if self.view(o).is_some_and(|w| w.elem.is_bigint()) {', '        if false {'),
    # The heap leaves a BigInt element's value to the agent.
    (OB, '                    Some((i, crate::buffer::Elt::Big(_))) => Got::Element(o, i),',
     '                    Some((_, crate::buffer::Elt::Big(_))) => Got::Value(Value::UNDEFINED),'),
    (PX, '                *value = self.element(o, i)?;', '                *value = Value::UNDEFINED;'),
    # Conversions, signedness, content types.
    (TY, '            Ok(Elt::Big(self.bigint_from(v)?.low_u64()))', '            Ok(Elt::Num(self.number(v)?))'),
    (TY, '            Elt::Big(bits) if elem == Elem::BigInt64 => {',
     '            Elt::Big(bits) if elem == Elem::BigUint64 => {'),
    (TY, '        if self.elem_of(a).is_bigint() == self.elem_of(b).is_bigint() {', '        if true {'),
    (TY, '        self.same_content(o, exemplar, CONTENT_MISMATCH)?;\n', ''),
    (TY, '                (Elt::Big(x), Elt::Big(y)) if elem == Elem::BigInt64 => x as i64 > y as i64,',
     '                (Elt::Big(x), Elt::Big(y)) if elem == Elem::BigInt64 => x > y,'),
    (TY, '        b.to_i64().map(|i| Elt::Big(i as u64))', '        b.to_u64().map(Elt::Big)'),
    (TY, '                Elt::Big(bits) if elem == Elem::BigInt64 => (bits as i64).to_string(),',
     '                Elt::Big(bits) if elem == Elem::BigInt64 => bits.to_string(),'),
    (TY, '        let Some(bytes) = len.checked_mul(elem.size()).filter(|&b| b < MAX_BYTES) else {',
     '        let Some(bytes) = len.checked_mul(elem.size()).filter(|&b| b <= MAX_BYTES) else {'),
    # The BigInts made while others are held stay rooted.
    (TY, '                agent.vm.temps.push(v);\n                kept.push(v);', '                kept.push(v);'),
    (TY, '            agent.vm.temps.push(acc);\n            let v = agent.element(o, k)?;',
     '            let v = agent.element(o, k)?;\n            agent.vm.temps.push(acc);'),
    (TY, '            self.vm.temps.push(a);\n            let b = self.elt_value(elem, y)?;',
     '            let b = self.elt_value(elem, y)?;\n            self.vm.temps.push(a);'),
    (OBJ, '            OwnProperty::Data { value, .. } => self.vm.temps.push(value),',
     '            OwnProperty::Data { .. } => {}'),
    # DataView.
    (DV, '    get_big_uint64, set_big_uint64 = BigUint64, "BigUint64";',
     '    get_big_uint64, set_big_uint64 = BigInt64, "BigUint64";'),
    (DV, '    method("getBigInt64", 1, get_big_int64),\n', ''),
    # Atomics.
    (AT, '                    | Elem::BigInt64\n                    | Elem::BigUint64\n', ''),
    (AT, '            matches!(elem, Elem::Int32 | Elem::BigInt64)', '            elem == Elem::Int32'),
    (AT, '        let new = if elem.is_bigint() {', '        let new = if false {'),
    (AT, '            return Ok((Elt::Big(b.low_u64()), v));',
     '            return Ok((Elt::Big(b.low_u64()), Value::UNDEFINED));'),
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


FILTERS = ('bigint_tests::bigint_', 'typed_tests::views_atomics')
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
