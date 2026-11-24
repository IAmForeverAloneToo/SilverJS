"""SilverJS's sabotage of BigInt (J4.3b). One
break at a time, `cargo test`, restored byte for byte. Prints which tests
each break fails, then every test no break failed (which must be none).

    python -I tools/sabotage_bigint.py <repository root>

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
B = 'src/bigint.rs'
N = 'src/builtins/bigint.rs'
O = 'src/interp/ops.rs'
H = 'src/heap.rs'
TB = 'src/table.rs'
BM = 'src/builtins/mod.rs'
cases = [
    # The size cap, as the reference browser checks it.
    (B, '            if self.digits().max(o.digits()) >= MAX_DIGITS {', '            if self.digits().max(o.digits()) > MAX_DIGITS {'),
    # Signs: floor shifts, remainders, powers.
    (B, '        if self.neg && low_bits_set(&self.mag, k) {', '        if self.neg && false {'),
    (B, '        Ok((BigInt::of(self.neg != o.neg, q), BigInt::of(self.neg, r)))', '        Ok((BigInt::of(self.neg != o.neg, q), BigInt::of(o.neg, r)))'),
    (B, '            let odd = e.mag.first().is_some_and(|&x| x & 1 == 1);', '            let odd = true;'),
    # Karatsuba's recombination.
    (B, '    add_shifted(&mut out, &z2, 2 * m);', '    add_shifted(&mut out, &z2, 2 * m + 1);'),
    # asIntN and asUintN.
    (B, '        if m.bit_len() == bits {', '        if m.bit_len() > bits {'),
    (B, '        if self.bit_len() < bits {', '        if self.bit_len() <= bits {'),
    # Numbers: comparison and the one rounding.
    (B, '        Some(if x > whole {', '        Some(if x < whole {'),
    (B, '            let sticky = u64::from(low_bits_set(&self.mag, shift));', '            let sticky = 0;'),
    # Parsing.
    (B, '            [0x2d, rest @ ..] => (true, rest),', '            [0x2d, rest @ ..] => (false, rest),'),
    (B, '                0x78 => Some(16),', '                0x78 => Some(10),'),
    (B, '        0x09 | 0x0a | 0x0b | 0x0c | 0x0d | 0x20 | 0xa0 | 0x1680 | 0x2000', '        0x09 | 0x0a | 0x0b | 0x0c | 0x0d | 0xa0 | 0x1680 | 0x2000'),
    # The agent's side: an interrupt stops the script; comparisons with
    # strings that are no BigInt; ToBigInt; toString's radix.
    (N, '            Refused::Interrupted => Abrupt::Terminate,', '            Refused::Interrupted => self.range_error("interrupted"),'),
    (N, '            (Some(None), _) | (_, Some(None)) => None,', '            (Some(None), _) | (_, Some(None)) => Some(Ordering::Less),'),
    (N, '                Op::TestLt => o == Ordering::Less,', '                Op::TestLt => o != Ordering::Greater,'),
    (N, '            return Ok(BigInt::from_u64(u64::from(b)));', '            return Ok(BigInt::from_u64(0));'),
    (N, '    if !(2.0..=36.0).contains(&radix) {', '    if !(2.0..=37.0).contains(&radix) {'),
    # The operators' and conversions' BigInt arms.
    (O, '            return !b.is_zero();', '            return true;'),
    (O, '            return Ok(x.compare_number(n) == Some(std::cmp::Ordering::Equal));', '            return Ok(false);'),
    (O, '            return Ok(n.is_some_and(|n| self.bigint_of(big) == Some(&n)));', '            return Ok(false);'),
    (BM, '            ObjectKind::BigInt => matches!(p.as_reference(), Some((RefKind::BigInt, _))),\n', ''),
    # Collections key BigInts by value; the collector keeps them.
    (TB, '        return x == y;\n    }\n    a == b', '        return false;\n    }\n    a == b'),
    (TB, '    if let Some(b) = bigint_of(p, v) {', '    if let Some(b) = None::<&crate::bigint::BigInt> {'),
    (H, '                    self.bigints.keep(i);', '                    let _ = i;'),
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


FILTERS = ('bigint',)
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
