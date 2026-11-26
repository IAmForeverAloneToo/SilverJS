"""SilverJS's sabotage of the lone surrogates in source made from a
JavaScript string (J4.6): their stand-ins' window, its choice, and the
places that write them back. One break at a time, `cargo test` of the
tests they guard, restored byte for byte. Prints which tests each break
fails, then every one of those tests no break failed (which must be
none).

    python -I tools/sabotage_surrogates.py <repository root>

With QUIET_SH set to the path of a quiet-machine script, each build and
test run takes its lock (`bash $QUIET_SH run`) at low priority
(QUIET_LOW=1), one case at a time.
"""
import os
import re
import shutil
import subprocess
import sys

root = sys.argv[1].replace(chr(92), '/').rstrip('/') + '/'
U = 'src/source_units.rs'
# At most five cases a step, on the checks that guard correctness (the
# lead, 2026-11-25).
cases = [
    # A string literal writes its stand-ins back.
    ('src/lexer.rs', '                    crate::source_units::push_char(&mut value, c, self.stand_ins);',
     '                    crate::source_units::push_char(&mut value, c, None);'),
    # So does a regexp literal's body.
    ('src/compiler/codegen/expr.rs', '                    body,\n                    self.sh.program.stand_ins,',
     '                    body,\n                    None,'),
    # eval's text keeps its lone surrogates.
    ('src/interp/eval.rs', '        let (text, stand_ins) = crate::source_units::text_of(&units);',
     '        let (text, stand_ins) = (String::from_utf16_lossy(&units), None);'),
    # The window is one the text leaves free; none without a lone surrogate.
    (U, '        .find(|&b| !in_window(b));', '        .find(|&b| in_window(b) || b > 0);'),
    (U, '    if decoded.iter().all(Result::is_ok) {', '    if false {'),
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


env['QUIET_LOW'] = '1'


def locked(label, cmd, limit=None):
    """`cmd` under the quiet lock of its own, at low priority, and a time
    limit inside the lock: waiting for it is not the run's time."""
    if not quiet:
        return cmd
    inner = ['timeout', str(limit), *cmd] if limit else cmd
    return [shutil.which('bash') or 'bash', quiet, 'run', label, '--', *inner]


# The tests the cases guard, which every case must reach between them.
FILTERS = ('language_tests::lone_surrogates_survive_eval_and_function',
           'source_units::tests::a_text_without_lone_surrogates_is_itself',
           'source_units::tests::lone_surrogates_take_a_window_the_text_leaves_free')
all_tests = set()
failed_ever = set()
r = subprocess.run(locked('silverjs sabotage surrogates list', ['cargo', 'test', '-q', '--lib', '--', '--list']), cwd=root,
                   capture_output=True, text=True, env=env)
for line in r.stdout.splitlines():
    name = line[:-len(': test')]
    if line.endswith(': test') and name in FILTERS:
        all_tests.add(name)
print(f'{len(all_tests)} tests', flush=True)
for n, (path, a, b) in enumerate(cases, 1):
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
        test = ['cargo', 'test', '-q', '--lib', '--', '--exact', *FILTERS]
        r = subprocess.run(locked(f'silverjs sabotage surrogates {n}', test, 900),
                           cwd=root, capture_output=True, text=True, encoding='utf-8',
                           errors='replace', timeout=None if quiet else 900, env=env)
        out = r.stdout + r.stderr
        if quiet and r.returncode == 124:
            out += '\ntest result: TIMEOUT'
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
    if not failed:
        print('    no test named; the output ends: ' + out[-300:].replace('\n', ' '), flush=True)
print('never failed:', sorted(all_tests - failed_ever), flush=True)
