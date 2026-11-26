"""SilverJS's sabotage of default parameters' TDZ and an array pattern's
iterator close on a return (J4.6b). One break at a time, `cargo test` of
the test they guard, restored byte for byte. Prints which tests each
break fails, then every one of those tests no break failed (which must
be none).

    python -I tools/sabotage_params.py <repository root>

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
M = 'src/compiler/codegen/mod.rs'
# At most five cases a step, on the checks that guard correctness (the
# lead, 2026-11-25).
cases = [
    # A parameter of a list that is not simple has a TDZ.
    (M, '            || ((r.kind.is_some_and(BindingKind::lexical) || Self::param_tdz(r))',
     '            || (r.kind.is_some_and(BindingKind::lexical)'),
    # Read from its own function before its element ends, it throws.
    (M, '        Self::param_tdz(r) && r.same_function && at < r.tdz_end',
     '        false && r.same_function && at < r.tdz_end'),
    # A captured one gets its argument when its element is bound.
    (M, '                (BindingKind::Param, Loc::Slot(k)) if !b.param_tdz() => {',
     '                (BindingKind::Param, Loc::Slot(k)) => {'),
    # The analysis gives the list's parameters their TDZ.
    ('src/compiler/scope.rs', '        let simple = f.rest.is_none()', '        let simple = true || f.rest.is_none()'),
    # An array pattern's iterator is closed by a return that leaves it.
    ('src/compiler/codegen/stmt.rs', '                    self.close_iterator(it, done, false);',
     '                    let _ = (it, done);'),
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


# The test every case must fail: the J4.6 params probe's facts.
FILTERS = ('language_tests::parameters_and_patterns_read_as_the_reference_browser_gives_them',)
all_tests = set()
failed_ever = set()
r = subprocess.run(locked('silverjs sabotage params list', ['cargo', 'test', '-q', '--lib', '--', '--list']), cwd=root,
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
        r = subprocess.run(locked(f'silverjs sabotage params {n}', test, 900),
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
