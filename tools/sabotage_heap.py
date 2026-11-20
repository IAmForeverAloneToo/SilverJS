"""SilverJS's values, heap, handles and objects sabotage: one break at a time, `cargo test`,
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
V = 'src/value.rs'
H = 'src/heap.rs'
A = 'src/agent.rs'
O = 'src/object.rs'
T = 'src/string.rs'
cases = [
    # Values.
    (V, "if f.is_nan() {\n            Value(CANONICAL_NAN)", "if false {\n            Value(CANONICAL_NAN)"),
    (V, "!(i == 0 && f.is_sign_negative())", "true"),
    (V, "(self.0 >= TAGGED).then_some(", "(self.0 > TAGGED).then_some("),
    # Arenas and the collector.
    (H, "slot.generation = slot.generation.wrapping_add(1);", ""),
    (H, "pub(crate) fn get(&self, index: u32) -> Option<&T> {\n        let i = index as usize;\n        if i >= self.sweep && !self.marked(i) {",
        "pub(crate) fn get(&self, index: u32) -> Option<&T> {\n        let i = index as usize;\n        if false {"),
    (H, "// Made now, it outlives the sweep under way.\n        self.set_mark(index);",
        "// Made now, it outlives the sweep under way."),
    (H, "if self.objects.keep(i)\n                        && let Some(cell) = self.objects.get_raw(i)\n                    {\n                        cell.trace(&mut stack);",
        "if self.objects.keep(i)\n                        && let Some(cell) = self.objects.get_raw(i)\n                    {\n                        let _ = cell;"),
    (H, "self.strings.keep(i);", ""),
    (H, "self.dead = self.live - self.kept;\n        self.dead_extra = self.extra - self.kept_extra;",
        "self.dead = 0;\n        self.dead_extra = 0;"),
    # Handles and the agent.
    (A, "let roots: Vec<Value> = self.handles.iter().copied().chain(strong).collect();",
        "let _ = strong;\n        let roots: Vec<Value> = self.handles.iter().copied().collect();"),
    (A, ".filter(|r| r.strong)", ""),
    (A, "global.slot as usize) {\n            *entry = None;",
        "global.slot as usize) {\n            let _ = entry;"),
    (A, "assert_eq!(\n            agent, self.id,", "assert_ne!(\n            agent, 0,"),
    (A, "if self.agent.generation_of(value) != Some(weak.generation) {",
        "if self.agent.generation_of(value).is_none() {"),
    (A, "if self.heap.bytes() + bytes > self.limit {\n            return Err(OutOfMemory);",
        "if false {\n            return Err(OutOfMemory);"),
    (A, "if self.stress || wanted > self.next_collection", "if wanted > self.next_collection"),
    (A, "let result = f(&mut inner);\n        self.agent.handles.truncate(base);\n        result",
        "let result = f(&mut inner);\n        let _ = base;\n        result"),
    # Objects (J2.2).
    (O, "            if !cell.is_extensible() {\n                return false;\n            }\n            let prop",
        "            if false {\n                return false;\n            }\n            let prop"),
    (O, "            if desc.configurable == Some(true) {\n                return false;\n            }\n            if desc.enumerable",
        "            if false {\n                return false;\n            }\n            if desc.enumerable"),
    (O, "if x.is_nan() && y.is_nan() {", "if false {"),
    (O, "return x == y && x.is_sign_negative() == y.is_sign_negative();", "return x == y;"),
    (O, "if i >= length && !length_writable {", "if false {"),
    (O, "if i >= length {\n                    self.with_object(o, |c| c.extra_mut().length = i + 1);",
        "if false {\n                    self.with_object(o, |c| c.extra_mut().length = i + 1);"),
    (O, ".is_none_or(|p| p.configurable());", ".is_none_or(|_| true);"),
    (O, "if n < 0.0 || n.fract() != 0.0 || n >= 4_294_967_296.0 {", "if n < 0.0 {"),
    (O, "            Some(OwnProperty::Data {\n                writable: false, ..\n            }) => SetOutcome::Done(false),\n",
        ""),
    (O, "            if p == o {\n                return false;", "            if false {\n                return false;"),
    (O, ".filter(|e| matches!(e.key, PropertyKey::Atom(_)))", ".filter(|_| true)"),
    (O, "out.push(Value::reference(RefKind::Symbol, s));", "let _ = s;"),
    (O, "index.insert(key, u32::try_from(self.props.len() - 1).unwrap_or(0));", "let _ = index;"),
    (O, "self.props.remove(at);\n                    self.reindex();", "self.props.remove(at);"),
    (O, ".get(i as usize)\n                .filter(|v| **v != Value::HOLE)", ".get(i as usize)"),
    (O, "self.props.reserve_exact(self.props.len().max(1));", ""),
    (O, "Some(OwnProperty::Accessor { get, .. }) => return Got::Getter(get),",
        "Some(OwnProperty::Accessor { get, .. }) => return Got::Value(get),"),
    (O, "None => self.define_own_property(r, key, Descriptor::data(value)),",
        "None => self.define_own_property(r, key, Descriptor { value: Some(value), ..Descriptor::default() }),"),
    (O, "(Some(x), Some(y)) => x.same_units(y),", "(Some(_), Some(_)) => false,"),
    (T, "bytes.extend_from_slice(y);", ""),
    # Strings and keys (J2.2).
    (T, "units.iter().all(|&u| u < 256)", "false"),
    (T, "if first == u16::from(b'0') && !rest.is_empty() {", "if false {"),
    (T, "(n < u64::from(u32::MAX)).then_some", "(n <= u64::from(u32::MAX)).then_some"),
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
    # The heap's and values' own tests: no other can fail by these breaks.
    if line.endswith(': test') and line.split('::')[0] in ('heap_tests', 'value', 'object_tests', 'string'):
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
