"""SilverJS's built-ins sabotage (J2.5): one break at a time, `cargo
test`, restored byte for byte. Prints which tests each break fails, then
every test no break failed (which must be none).

    python -I tools/sabotage_builtins.py <repository root>

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
B = 'src/builtins/'
MOD = B + 'mod.rs'
OBJ = B + 'object.rs'
FUN = B + 'function.rs'
ERR = B + 'error.rs'
PRI = B + 'primitive.rs'
STR = B + 'string.rs'
ARR = B + 'array.rs'
GLO = B + 'global.rs'
URI = B + 'uri.rs'
REF = B + 'reflect.rs'
NUM = 'src/number.rs'
O = 'src/object.rs'
H = 'src/heap.rs'
M = 'src/interp/mod.rs'
P = 'src/interp/ops.rs'
E = 'src/embed.rs'
cases = [
    # Tables and their order.
    (OBJ, 'method("seal", 1, seal),\n    method("create", 2, create),',
        'method("create", 2, create),\n    method("seal", 1, seal),'),
    (GLO, 'intrinsic("Boolean"),\n    intrinsic("String"),',
        'intrinsic("String"),\n    intrinsic("Boolean"),'),
    (MOD, 'enumerable: Some(e.attrs & ENUMERABLE != 0),', 'enumerable: Some(true),'),
    (REF, 'method("defineProperty", 3, define_property),\n    method("deleteProperty", 2, delete_property),',
        'method("deleteProperty", 2, delete_property),\n    method("defineProperty", 3, define_property),'),
    # Calls, bound functions and the Function constructor.
    (M, 'new_target = Some(if nt == callee { target } else { nt });', 'new_target = Some(nt);'),
    (FUN, '(n - bound_args).max(0.0)', 'n.max(0.0)'),
    (FUN, 'if !one_function(&format!("(function anonymous({params}\\n) {{\\n}})"))\n        ||',
        'if'),
    (MOD, 'let g = self.native_function(id, 0, &format!("get {fname}"))?;\n                    attrs(None, Some(g), Some(Value::UNDEFINED))',
        'let g = self.native_function(id, 0, &fname)?;\n                    attrs(None, Some(g), Some(Value::UNDEFINED))'),
    (FUN, 'if is_sloppy_function(agent, c.this) {\n        Ok(Value::NULL)', 'if false {\n        Ok(Value::NULL)'),
    (M, '_ if !f.strict && this.as_object().is_none() => match self.wrap(this, None) {',
        '_ if false => match self.wrap(this, None) {'),
    # Arguments objects.
    (O, 'let slot = internal.get(1 + i as usize)?.as_int()?;', 'let slot = internal.get(i as usize)?.as_int()?;'),
    (O, '            if desc.writable == Some(false) {\n                self.unmap(o, key);\n            }\n', ''),
    (O, '                if self.is_mapped(o, key) {\n                    self.unmap(o, key);\n                }\n', ''),
    # String wrappers.
    (O, 'enumerable: key != self.length_key,', 'enumerable: true,'),
    (O, '            all.push(self.length_key);\n', ''),
    (MOD, 'if !self.heap.unit_strings.contains_key(&u) {', 'if false {'),
    (H, '        stack.extend(self.unit_strings.values());\n', ''),
    # Conversions and symbols.
    (P, 'Some(true) => "string",', 'Some(true) => "default",'),
    (P, 'if !h.is_undefined() {\n            let r = self.call_value(h, target, &[value])?;',
        'if false {\n            let r = self.call_value(h, target, &[value])?;'),
    (OBJ, 'if tag.as_string().is_some() {', 'if false {'),
    (PRI, 'if let Some(&s) = agent.vm.registry.get(&units) {', 'if let Some(&s) = None::<&Value> {'),
    (OBJ, 'if !x.is_undefined() && !self.is_callable(x) {', 'if false {'),
    # Numbers.
    (NUM, "let up = digits.get(k).is_some_and(|&d| d >= b'5');", "let up = digits.get(k).is_some_and(|&d| d > b'5');"),
    (NUM, '        int_digits.reverse();\n', ''),
    (NUM, 'if k <= n && n <= 21 {', 'if k <= n && n < 21 {'),
    # Arrays.
    (ARR, 'let take_b = sort_compare(self, f, x, y)? == std::cmp::Ordering::Greater;',
        'let take_b = sort_compare(self, f, x, y)? != std::cmp::Ordering::Less;'),
    (ARR, 'if (a >= 0) == (b >= 0) && a.abs() < 10 && b.abs() < 10 {', 'if true {'),
    (ARR, 'if agent.vm.joining.contains(&o) {', 'if false {'),
    (P, 'if f64::from(len) != again {', 'if false {'),
    # Strings, errors, URIs, the embedder.
    (STR, '                0x26 => {\n                    out.extend_from_slice(matched);',
        '                0x26 => {\n                    let _ = matched;'),
    (ERR, 'if options.as_object().is_some() && agent.has_property(options, cause) {', 'if false {'),
    (URI, 'if in_set(u16::from(b), reserved) {', 'if false {'),
    (REF, 'let nt = if c.argc > 2 { agent.arg(c, 2) } else { f };', 'let nt = f;'),
    (E, '            agent.vm.exception = e;\n', ''),
    (FUN, 'let name = match agent.host_name(id) {', 'let name = match None::<&str> {'),
]

env = dict(os.environ)
quiet = env.get('QUIET_SH')


def wait_quiet():
    if quiet:
        bash = shutil.which('bash') or 'bash'
        subprocess.run([bash, quiet, 'wait'], capture_output=True)


SUITES = ('builtins_tests', 'number')
all_tests = set()
failed_ever = set()
wait_quiet()
r = subprocess.run(['cargo', 'test', '-q', '--lib', '--', '--list'], cwd=root,
                   capture_output=True, text=True, env=env)
for line in r.stdout.splitlines():
    if line.endswith(': test') and line.split('::')[0] in SUITES:
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
        r = subprocess.run(['cargo', 'test', '-q', '--lib', '--', 'builtins_tests', 'number::'],
                           cwd=root, capture_output=True, text=True, encoding='utf-8',
                           errors='replace', timeout=900, env=env)
        out = r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        out = 'test result: TIMEOUT'
    finally:
        shutil.move(full + '.bak', full)
        os.utime(full)
    if 'test result' not in out:
        print('DID NOT RUN ' + a.strip()[:60] + ' :: ' + out[-400:].replace('\n', ' '), flush=True)
        continue
    failed = set(re.findall(r'^test (\S+) \.\.\. FAILED', out, re.M))
    failed |= set(re.findall(r'^    (\S+)$', out, re.M)) & all_tests
    failed_ever |= failed
    tag = 'CAUGHT' if (r.returncode != 0 or 'TIMEOUT' in out) else 'MISSED'
    print(f'{tag} {a.strip()[:55]!r} -> {sorted(failed)}', flush=True)
print('never failed:', sorted(all_tests - failed_ever), flush=True)
