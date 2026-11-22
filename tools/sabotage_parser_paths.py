"""SilverJS's sabotage of the parser's lighter frames and faster paths
(after J2.5): one break at a time, `cargo test` of the lexer's and the
parser's tests, restored byte for byte. Prints which tests each break
fails; every break must be caught.

    python -I tools/sabotage_parser_paths.py <repository root>

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
M = 'src/parser/mod.rs'
L = 'src/lexer.rs'
cases = [
    # The token after the current one, kept by a peek. (`rescanned`
    # dropping it is not broken here: only an identifier peeks, and the
    # move past it takes the peek, so no source reaches a rescan with a
    # token kept. The drop guards a later peek elsewhere.)
    (M,'            Some((lexer, tok)) => {\n                self.lexer = lexer;\n                Ok(tok)', '            Some((_lexer, tok)) => {\n                Ok(tok)'),
    # Moving past a token.
    (M, '        let next = self.next_token()?;\n        self.prev_end = self.tok.end;\n        self.tok = next;\n        Ok(())',
        '        let next = self.next_token()?;\n        self.tok = next;\n        Ok(())'),
    # Names shared from the cache.
    (M, '        Some(Some(name)) if **name == *text => Name::clone(name),', '        Some(Some(name)) => Name::clone(name),'),
    # Short decimal integers read without text.
    (L, '        let plain = (1..=15).contains(&run)', '        let plain = (1..=25).contains(&run)'),
    (L, "            && (run == 1 || rest.first() != Some(&b'0'))\n", ''),
    (L, "!matches!(b, b'_' | b'.' | b'$' | b'\\\\')", "!matches!(b, b'_' | b'$' | b'\\\\')"),
    (L, "!matches!(b, b'_' | b'.' | b'$' | b'\\\\')", "!matches!(b, b'.' | b'$' | b'\\\\')"),
    (L, "!matches!(b, b'_' | b'.' | b'$' | b'\\\\')", "!matches!(b, b'_' | b'.' | b'\\\\')"),
    (L, "!matches!(b, b'_' | b'.' | b'$' | b'\\\\')", "!matches!(b, b'_' | b'.' | b'$')"),
    (L, '                b.is_ascii()\n                    && !b.is_ascii_alphanumeric()\n', '                b.is_ascii()\n'),
    (L, '                b.is_ascii()\n                    && !b.is_ascii_alphanumeric()\n', '                true\n                    && !b.is_ascii_alphanumeric()\n'),
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


FILTERS = ('lexer_tests', 'parser_tests')
missed = 0
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
        missed += 1
        continue
    failed = set(re.findall(r'^test (\S+) \.\.\. FAILED', out, re.M))
    failed |= set(re.findall(r'^    ((?:lexer|parser)_tests::\S+)$', out, re.M))
    caught = r.returncode != 0 or 'TIMEOUT' in out
    missed += 0 if caught else 1
    tag = 'CAUGHT' if caught else 'MISSED'
    print(f'{tag} {a.strip()[:55]!r} -> {sorted(failed)}', flush=True)
print(f'{len(cases) - missed} of {len(cases)} caught', flush=True)
