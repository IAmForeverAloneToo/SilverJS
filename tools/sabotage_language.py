"""SilverJS's sabotage of tagged templates and Annex B.3.2 (J3.5a). One
break at a time, `cargo test`, restored byte for byte. Prints which tests
each break fails, then every test no break failed (which must be none).

    python -I tools/sabotage_language.py <repository root>

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
M = 'src/interp/mod.rs'
SC = 'src/compiler/scope.rs'
cases = [
    # Tagged templates.
    (M, '        if let Some(v) = self.realm().templates.get(&site) {', '        if let Some(v) = self.realm().templates.get(&site).filter(|_| false) {'),
    (M, '            self.freeze_value(template)?;', '            let _ = template;'),
    (M, '            self.freeze_value(raw_array)?;', '            let _ = raw_array;'),
    (M, '                        value: Some(raw_array),\n                        writable: Some(false),\n                        enumerable: Some(false),', '                        value: Some(raw_array),\n                        writable: Some(false),\n                        enumerable: Some(true),'),
    (M, '                    None => Value::UNDEFINED,\n                };\n                self.vm.temps.push(s);\n                cooks.push(s);', '                    None => self.realm().strings.empty,\n                };\n                self.vm.temps.push(s);\n                cooks.push(s);'),
    # Annex B.3.2.
    (SC, '            self.a.annex_b.insert(fid);', '            let _ = fid;'),
    (SC, '                BindingKind::Let | BindingKind::Const | BindingKind::Class => true,', '                BindingKind::Let | BindingKind::Const | BindingKind::Class => false,'),
    (SC, '                BindingKind::Param => at_top,', '                BindingKind::Param => false,'),
    (SC, '            if f.is_async || f.is_generator || self.a.functions[around as usize].strict {', '            if f.is_async || f.is_generator {'),
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


FILTERS = ('language_tests',)
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
