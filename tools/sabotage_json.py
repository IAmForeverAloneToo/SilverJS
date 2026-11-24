"""SilverJS's sabotage of JSON and Math.sumPrecise (J4.3a). One
break at a time, `cargo test`, restored byte for byte. Prints which tests
each break fails, then every test no break failed (which must be none).

    python -I tools/sabotage_json.py <repository root>

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
J = 'src/builtins/json.rs'
M = 'src/builtins/math.rs'
cases = [
    # Parsing and its messages.
    (J, '    matches!(u, 0x20 | 0x09 | 0x0a | 0x0d)', '    matches!(u, 0x09 | 0x0a | 0x0d)'),
    (J, '        let within = if m.ends_with("JSON") { "" } else { "in JSON " };', '        let within = "in JSON ";'),
    (J, '            } else if at < len - CONTEXT {', '            } else if at < len {'),
    (J, '            if ["undefined", "NaN", "Infinity", "[object Object]"].contains(&whole.as_str()) {', '            if ["undefined", "Infinity", "[object Object]"].contains(&whole.as_str()) {'),
    (J, '            u if u < 0x20 => {', '            u if u < 0x01 => {'),
    (J, '            return refuse(Fault::Says("Unterminated fractional number"), i);', '            return refuse(Fault::Token, i);'),
    # The reviver's walk and its source text.
    (J, '                .is_some_and(|n| same_value(&self.heap, n.value, value))', '                .is_some()'),
    (J, '            self.o_delete(o, name).map(|_| ())', '            Ok(())'),
    # Serializing.
    (J, '        } else if (0xdc00..0xe000).contains(&u) {', '        } else if false {'),
    (J, '                    Some(ObjectKind::RawJson) => {', '                    Some(ObjectKind::Error) => {'),
    (J, '                if n.is_finite() {', '                if !n.is_nan() {'),
    (J, '        const FIRST: usize = 2;', '        const FIRST: usize = 3;'),
    (J, '                if any && !st.gap.is_empty() {', '                if !st.gap.is_empty() {'),
    (J, '                Serial::Nothing if !array => {}', '                Serial::Nothing => {}'),
    (J, '            if !list.contains(&item) {', '            if true {'),
    (J, '            let n = n.trunc().clamp(0.0, 10.0);', '            let n = n.trunc().clamp(0.0, 20.0);'),
    (J, '            return Ok(u.into_iter().take(10).collect());', '            return Ok(u);'),
    # rawJSON.
    (J, '    let wrong = if is_space(first) || first == 0x7b || first == 0x5b {', '    let wrong = if is_space(first) {'),
    (J, '        agent.freeze_value(o)?;\n', ''),
    # Math.sumPrecise.
    (M, '                *l += if negative { -piece } else { piece };', '                *l += piece;'),
    (M, '            if half && (sticky || mantissa & 1 == 1) {', '            if half && mantissa & 1 == 1 {'),
    (M, '            if field >= 2047 {', '            if field > 2047 {'),
    (M, '            State::MinusZero => -0.0,', '            State::MinusZero => 0.0,'),
    (M, '            } else if !(n == 0.0 && n.is_sign_negative())', '            } else if true'),
    # A string parsed is rooted until its container holds it; an object
    # serialized leaves the cycle check once it is closed.
    (J, '                    let v = self.new_string_units(&units)?;\n                    self.vm.temps.push(v);\n', '                    let v = self.new_string_units(&units)?;\n'),
    (J, '                    st.open.remove(&o);\n', ''),
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


FILTERS = ('json_tests',)
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
