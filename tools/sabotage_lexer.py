"""SilverJS's lexer sabotage: one break at a time, `cargo test`,
restored byte for byte. Prints which tests each break fails, then every
test no break failed (which must be none).

    python -I tools/sabotage_lexer.py <repository root>

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
L = 'src/lexer.rs'
U = 'src/unicode.rs'
cases = [
    (L, "(Punct::UShrEq, 4)", "(Punct::UShr, 4)"),
    (L, "(Some(b'.'), d) if !d.is_some_and(|d| d.is_ascii_digit()) =>", "(Some(b'.'), d) if true =>"),
    (L, "Some(b'/') if !class =>", "Some(b'/') =>"),
    (L, "None | Some(b'\\n' | b'\\r') => return Err(self.error(UNTERMINATED_REGEXP, start)),",
        "None => return Err(self.error(UNTERMINATED_REGEXP, start)),"),
    (L, "                token.escaped = true;", "                token.escaped = false;"),
    # ZWNJ and ZWJ refused in identifiers. Dropping only the explicit
    # check would change nothing: ID_Continue holds both since Unicode
    # 15.1.
    (U, "c == ZWNJ || c == ZWJ || in_table(ID_CONTINUE, c)",
        "c != ZWNJ && c != ZWJ && in_table(ID_CONTINUE, c)"),
    (L, "self.slice(start + 1, self.pos).to_owned()", "self.slice(start, self.pos).to_owned()"),
    (L, "                text.push('.');\n", ""),
    (L, "    if sticky {", "    if false {"),
    (L, "all(|d| d < b'8')", "all(|d| d < b'9')"),
    (L, "if !(integer && bigint) {", "if !integer {"),
    (L, "                if after_separator {\n                    return Err(self.error(SEPARATOR_DOUBLED",
        "                if false {\n                    return Err(self.error(SEPARATOR_DOUBLED"),
    (L, "Some(b) if b.is_ascii_digit() || b == b'\\\\' => true,", "Some(b) if b == b'\\\\' => true,"),
    (L, "b'v' => 0x0B,", "b'v' => 0x0C,"),
    (L, "let max_digits = if b <= b'3' { 3 } else { 2 };", "let max_digits = if b <= b'3' { 2 } else { 2 };"),
    (L, "cooked.push(u16::from(b'\\n'));", "cooked.push(u16::from(b'\\r'));"),
    (L, "invalid.get_or_insert(error);", "let _ = error;"),
    (L, "self.template(start, brace.newline_before, false)", "self.template(start, brace.newline_before, true)"),
    (L, "                    if lines {", "                    if false {"),
    (U, "' ' | ZWNBSP => true,", "' ' => true,"),
    (L, 'if source.starts_with("#!") {', 'if source.starts_with("#?") {'),
    (L, "                    && line_start\n", "                    && true\n"),
    (L, "self.pos = (position as usize).min(self.bytes.len());", "self.pos = 0;"),
    (L, ".map(char::len_utf16)", ".map(char::len_utf8)"),
    (L, "SyntaxError::new(message, start, self.pos.max(start))", "SyntaxError::new(message, start, start)"),
    (U, "is_some_and(|&(lo, _)| lo <= c)", "is_some_and(|&(lo, _)| lo < c)"),
]
env = dict(os.environ, CARGO_BUILD_JOBS='4')
quiet = os.environ.get('QUIET_SH')


def wait_quiet():
    if quiet:
        # Resolved on PATH: on Windows a bare 'bash' would be looked up in
        # the system directory first.
        bash = shutil.which('bash') or 'bash'
        subprocess.run([bash, quiet, 'wait'], check=True)


all_tests = set()
failed_ever = set()
wait_quiet()
r = subprocess.run(['cargo', 'test', '-q', '--lib', '--', '--list'], cwd=root,
                   capture_output=True, text=True, env=env)
for line in r.stdout.splitlines():
    if line.endswith(': test'):
        all_tests.add(line[:-len(': test')])
print(f'{len(all_tests)} tests', flush=True)
for path, a, b in cases:
    full = root + path
    with open(full, 'rb') as f:
        src = f.read().decode('utf-8')
    if '\r\n' in src:
        a, b = a.replace('\n', '\r\n'), b.replace('\n', '\r\n')
    if src.count(a) != 1:
        print(f'NOT FOUND ({src.count(a)}) {path}: {a.strip()[:70]}', flush=True)
        continue
    shutil.copy(full, full + '.bak')
    with open(full, 'wb') as f:
        f.write(src.replace(a, b).encode('utf-8'))
    try:
        wait_quiet()
        r = subprocess.run(['cargo', 'test', '-q', '--lib'], cwd=root, capture_output=True,
                           text=True, encoding='utf-8', errors='replace', timeout=900, env=env)
        out = r.stdout + r.stderr
    finally:
        shutil.move(full + '.bak', full)
        os.utime(full)
    if 'test result' not in out:
        print('DID NOT RUN ' + a.strip()[:60] + ' :: ' + out[-400:].replace('\n', ' '), flush=True)
        continue
    failed = set(re.findall(r'^test (\S+) \.\.\. FAILED', out, re.M))
    failed |= set(re.findall(r'^    (\S+)$', out, re.M)) & all_tests
    failed_ever |= failed
    tag = 'CAUGHT' if r.returncode != 0 else 'MISSED'
    print(f'{tag} {a.strip()[:55]!r} -> {sorted(failed)}', flush=True)
print('never failed:', sorted(all_tests - failed_ever), flush=True)
