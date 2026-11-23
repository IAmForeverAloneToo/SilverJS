"""SilverJS's interpreter sabotage: one break at a time, `cargo test`,
restored byte for byte. Prints which tests each break fails, then every
test no break failed (which must be none).

    python -I tools/sabotage_interp.py <repository root>

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
P = 'src/interp/ops.rs'
R = 'src/builtins/array.rs'
O = 'src/object.rs'
H = 'src/heap.rs'
S = 'src/string.rs'
cases = [
    # Unwinding and frames.
    (M, "top.context = context;", "let _ = context;"),
    (M, "self.vm.acc = self.vm.exception;", ""),
    (M, "if abrupt == Abrupt::Throw\n                && let Some(code)", "if true\n                && let Some(code)"),
    (M, "_ if !f.strict && this.is_nullish() => self.realm().global,", ""),
    (M, "if frame.construct && value.as_object().is_none() {", "if false {"),
    (M, "let recv = if op == Op::Call {\n                            self.reg(base, r.u())",
        "let recv = if op == Op::Call {\n                            { let _ = r.u(); Value::UNDEFINED }"),
    (M, "if self.vm.acc == Value::UNINITIALIZED {", "if false {"),
    (M, "if let Some(e) = copy {", "if let Some(e) = copy.filter(|_| false) {"),
    (M, "for _ in 0..depth {", "for _ in 0..depth.min(0) {"),
    (M, "let at = f.positions.iter().rev().find(|p| p.pc <= pc);",
        "let at = f.positions.iter().find(|p| p.pc <= pc);"),
    (M, ".is_some_and(|f| {\n                    matches!(", ".is_some_and(|f| {\n                    true || matches!("),
    (M, "if self.vm.frames.len() >= MAX_FRAMES {\n            return Err(self.throw_error(ErrorType::RangeError,",
        "if self.vm.frames.len() >= MAX_FRAMES {\n            return Err(self.throw_error(ErrorType::TypeError,"),
    # Operations.
    (P, "return x == y;", "return x.to_bits() == y.to_bits();"),
    (P, "if pa.as_string().is_some() || pb.as_string().is_some() {",
        "if pa.as_string().is_some() && pb.as_string().is_some() {"),
    (P, 'Some("0x" | "0X") => Some(16),', 'Some("0x" | "0X") => Some(10),'),
    (P, "let m = if m < 0.0 { m + 4_294_967_296.0 } else { m };", ""),
    (P, "if a.is_nullish() && b.is_nullish() {", "if false {"),
    (P, "if key == self.heap.length_key {", "if false {"),
    (P, "if b.constant {", "if false {"),
    (P, "if still {", "if true {"),
    (P, "let mut result = self.finish_function(f, length, &name, kind == FunctionKind::Normal);",
        "let mut result = self.finish_function(f, length, &name, false);"),
    (P, '                if strict {\n                    let k = self.key_display(key);\n                    Err(self\n                        .throw_error(ErrorType::TypeError, messages::read_only(&k, "#<Object>")))', '                if strict {\n                    let k = self.key_display(key);\n                    Err(self\n                        .throw_error(ErrorType::TypeError, messages::read_only(&k, "object")))'),
    # The realm's natives.
    (R, "agent.put_index(o, len + i as f64, v)?;", "agent.put_index(o, len, v)?;"),
    # The object model and the heap.
    (O, "Some(e) if e.flags & (WRITABLE | ACCESSOR) == WRITABLE => {", "Some(e) => {"),
    (H, "        self.internals.update_extra(before, after);\n", ""),
    # Strings built in shared buffers (J2.4b).
    (S, "let append = after_a && (!before_b || lb <= la);", "let append = after_a;"),
    (S, "if *end == buffer.borrow().used_end);", "if *end >= 0);"),
    (S, "if *start == buffer.borrow().used_start);", "if *start >= 0);"),
    (S, "(used + after).max(after * 2)", "after"),
    (S, "let p = start + isize::try_from(i).ok()?;", "let p = start + isize::try_from(i).ok()? + 1;"),
    (H, "if live * 4 >= capacity {", "if true {"),
    (H, "c.accounted = 0;", "let _ = &c;"),
]
env = dict(os.environ, CARGO_BUILD_JOBS='4')
quiet = os.environ.get('QUIET_SH')


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
    # The interpreter's own tests: no other can fail by these breaks.
    if line.endswith(': test') and line.split('::')[0] == 'interp_tests':
        all_tests.add(line[:-len(': test')])
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
        r = subprocess.run(['cargo', 'test', '-q', '--lib', '--', 'interp_tests'],
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
