"""SilverJS's sabotage of several realms per agent (J2.5c): one break at
a time, `cargo test`, restored byte for byte. Prints which tests each
break fails, then every test no break failed (which must be none).

    python -I tools/sabotage_realms.py <repository root>

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
R = 'src/interp/realm.rs'
A = 'src/builtins/array.rs'
B = 'src/builtins/mod.rs'
E = 'src/embed.rs'
G = 'src/agent.rs'
cases = [
    # Calls switch to the callee's realm and back.
    (M, '                let caller = self.enter_realm(realm);\n', '                let caller = self.vm.current;\n'),
    (M, '                self.vm.current = caller;\n                self.vm.natives.pop();', '                self.vm.natives.pop();'),
    (M, '        let caller_realm = self.enter_realm(realm);\n', '        let caller_realm = self.vm.current;\n'),
    (M, '                        self.vm.current = frame.caller_realm;\n', ''),
    (M, '            self.vm.frames.pop();\n            self.vm.current = frame.caller_realm;\n', '            self.vm.frames.pop();\n'),
    # What realm a function is of.
    (M, '            lines: std::cell::OnceCell::new(),\n            realm: self.vm.current,', '            lines: std::cell::OnceCell::new(),\n            realm: 0,'),
    (B, '                realm: self.vm.current,\n', '                realm: 0,\n'),
    (M, '                        .map_or(self.vm.current, |u| u.realm);\n                }', '                        .map_or(self.vm.current, |_| self.vm.current);\n                }'),
    (M, '                Some(Callable::Native { realm, .. }) => return realm,', '                Some(Callable::Native { .. }) => return self.vm.current,'),
    (M, '                Some(Callable::Native { realm, .. }) => return realm,\n                Some(Callable::Bound) => {\n                    at = at',
        '                Some(Callable::Native { realm, .. }) => return realm,\n                Some(Callable::Bound) => {\n                    let _ = at;\n                    return self.vm.current;\n                }\n                Some(Callable::Bound) if false => {\n                    at = at'),
    (M, '        let realm = self.function_realm(ctor);\n', '        let realm = self.vm.current;\n'),
    # ArraySpeciesCreate's realm check.
    (A, '            if realm != self.vm.current\n', '            if false && realm != self.vm.current\n'),
    (A, '                    .is_some_and(|r| r.intrinsics.array == c)', '                    .is_some_and(|_| true)'),
    # Making realms.
    (R, '        self.vm.current = if self.vm.realms.len() == 1 {\n            index\n        } else {\n            caller\n        };',
        '        let _ = caller;\n        self.vm.current = index;'),
    (M, '        for realm in &self.realms {\n            realm.trace(&mut out);', '        for realm in self.realms.iter().take(1) {\n            realm.trace(&mut out);'),
    (M, '        self.vm.realms.get_mut(self.vm.current as usize)\n', '        self.vm.realms.first_mut()\n'),
    # The embedder's realms.
    (E, '        self.agent_mut().vm.current = caller;\n', '        let _ = caller;\n'),
    (E, '        let known = (realm.0 as usize) < self.agent_ref().vm.realms.len();', '        let known = true;'),
    (E, '        Ok(RealmId(self.agent_ref().vm.current))', '        Ok(RealmId(0))'),
    (G, '        self.vm.random = crate::builtins::random_state(seed);\n', '        let _ = seed;\n'),
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


FILTERS = ('realm_tests', 'builtins_tests::a_seeded_math_random_repeats')
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
