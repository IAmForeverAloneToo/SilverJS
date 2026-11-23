"""SilverJS's sabotage of proxies (J3.5b). One
break at a time, `cargo test`, restored byte for byte. Prints which tests
each break fails, then every test no break failed (which must be none).

    python -I tools/sabotage_proxy.py <repository root>

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
P = 'src/builtins/proxy.rs'
O = 'src/builtins/object.rs'
B = 'src/builtins/mod.rs'
H = 'src/object.rs'
A = 'src/builtins/array.rs'
X = 'src/interp/ops.rs'
M = 'src/interp/mod.rs'
cases = [
    # Making and revoking.
    (P, '        if target.as_object().is_none() || handler.as_object().is_none() {', '        if target.as_object().is_none() {'),
    (P, '        if callable {\n            cell.set_callable(Callable::Proxy);\n        }', '        let _ = callable;'),
    (P, '            .with_internal_slots(o, |s| *s = vec![Value::NULL, Value::NULL]);', '            .with_internal_slots(o, |_s| {});'),
    (P, '        if !self.is_callable(trap) {', '        if false {'),
    # The invariants.
    (P, '        if proto.as_object().is_none() && !proto.is_null() {', '        if false {'),
    (P, '        if proto != self.o_get_prototype_of(t)? {\n            return Err(self.type_error(messages::PROXY_PROTO_INVARIANT));', '        if false {\n            return Err(self.type_error(messages::PROXY_PROTO_INVARIANT));'),
    (P, '        if r != expected {', '        if false {'),
    (P, '        if r && self.o_is_extensible(t)? {', '        if false {'),
    (P, '        if r.as_object().is_none() && !r.is_undefined() {', '        if false {'),
    (P, '            if !current.configurable() {\n                let text = self.key_text(key);\n                return Err(self.type_error(messages::proxy_gopd_hide_nonconfigurable', '            if false {\n                let text = self.key_text(key);\n                return Err(self.type_error(messages::proxy_gopd_hide_nonconfigurable'),
    (P, '                if !extensible {\n                    let text = self.key_text(key);\n                    return Err(self.type_error(messages::proxy_define_nonextensible', '                if false {\n                    let text = self.key_text(key);\n                    return Err(self.type_error(messages::proxy_define_nonextensible'),
    (P, '            if !c.configurable() {\n                let text = self.key_text(key);\n                return Err(self.type_error(messages::proxy_has_nonconfigurable', '            if false {\n                let text = self.key_text(key);\n                return Err(self.type_error(messages::proxy_has_nonconfigurable'),
    (P, '            if !c.configurable() {\n                let text = self.key_text(key);\n                return Err(self.type_error(messages::proxy_delete_nonconfigurable', '            if false {\n                let text = self.key_text(key);\n                return Err(self.type_error(messages::proxy_delete_nonconfigurable'),
    (P, '            if v.as_string().is_none() && !symbol {', '            if false {'),
    (P, '            if keys.contains(&k) {', '            if false {'),
    (P, '        if extensible && non_configurable.is_empty() {', '        if extensible || !non_configurable.is_empty() {'),
    (P, '        if r.as_object().is_none() {\n            let text = self.display(r);\n            return Err(self.type_error(messages::proxy_construct_not_object', '        if false {\n            let text = self.display(r);\n            return Err(self.type_error(messages::proxy_construct_not_object'),
    (P, '            }) if !crate::object::same_value(&self.heap, v, value) => {', '            }) if false => {'),
    (P, '            }) if !crate::object::same_value(&self.heap, value, current) => {', '            }) if false => {'),
    # A proxy as the receiver of a set.
    (P, '            None => self.o_define_own_property(r, key, Descriptor::data(value)),', '            None => Ok(true),'),
    (H, '                if self.object(r).is_some_and(|c| c.kind == ObjectKind::Proxy) {\n                    return SetOutcome::Receiver(r);', '                if false {\n                    return SetOutcome::Receiver(r);'),
    # IsArray (section 7.2.2) through proxies.
    (B, '                ObjectKind::Proxy => at = c.internal().first().copied().unwrap_or(Value::NULL),', '                ObjectKind::Proxy => return false,'),
    (A, '        if !self.is_array_through(original) {', '        if !self.is_array(original) {'),
    (A, '    Ok(agent.is_array_through(v))', '    Ok(agent.is_array(v))'),
    (A, '                if d > 0.0 && self.is_array_through(v) {', '                if d > 0.0 && self.is_array(v) {'),
    (B, '            Some(i) => self.o_has_property(i, key),', '            Some(i) => Ok(self.heap.has_property(i, key)),'),
    # A proxy's target and handler kept alive by it alone.
    (H, '            out.extend(extra.internal.iter().filter(|v| **v != Value::HOLE));', '            out.extend(extra.internal.iter().filter(|v| **v != Value::HOLE && self.kind != ObjectKind::Proxy));'),
    # for-in through the traps.
    (X, '                if self.o_get_own_property(o, key)?.is_some_and(|p| enumerable(&p)) {', '                if true {'),
    # The reference browser's messages.
    (O, '        let text = if agent.is_proxy(i) {\n            messages::PROXY_PREVENT_FALSE', '        let text = if false {\n            messages::PROXY_PREVENT_FALSE'),
    (X, '            let text = if self.is_proxy(o) {\n                messages::proxy_delete_false', '            let text = if false {\n                messages::proxy_delete_false'),
    (M, '            || inner.starts_with("new ")', '            || inner.is_empty()'),
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


FILTERS = ('proxy_tests',)
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
