"""SilverJS's sabotage of classes (J3.3a): classes made from their
heritage, methods and accessors with their home objects, super reads,
writes and calls, and derived constructors' this. One break at a time,
`cargo test`, restored byte for byte. Prints which tests each break
fails, then every test no break failed (which must be none).

    python -I tools/sabotage_classes.py <repository root>

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
I = 'src/interp/class.rs'
M = 'src/interp/mod.rs'
CC = 'src/compiler/codegen/class.rs'
C = 'src/compiler/codegen/mod.rs'
E = 'src/compiler/codegen/expr.rs'
ST = 'src/compiler/codegen/stmt.rs'
SC = 'src/compiler/scope.rs'
cases = [
    # Making a class.
    (I, '        if self.code_kind(unit, func) == Some(FunctionKind::DerivedConstructor) {', '        if false {'),
    (I, '        } else if parent.is_null() {', '        } else if false {'),
    (I, '            if pp.as_object().is_none() && !pp.is_null() {', '            if false {'),
    (I, '                self.heap.set_prototype_of(o, ctor_parent);', '                let _ = ctor_parent;'),
    (I, '                        value: Some(proto),\n                        writable: Some(false),', '                        value: Some(proto),\n                        writable: Some(true),'),
    (I, '            self.set_home_object(f, proto);\n            Ok((f, proto))', '            Ok((f, proto))'),
    (I, '            _ => Descriptor {\n                value: Some(f),\n                writable: Some(true),\n                enumerable: Some(false),',
        '            _ => Descriptor {\n                value: Some(f),\n                writable: Some(true),\n                enumerable: Some(true),'),
    (I, '            1 => Descriptor {\n                get: Some(f),', '            1 => Descriptor {\n                set: Some(f),'),
    (I, '        if !self.heap.define_own_property(o, key, desc) {', '        if !self.heap.define_own_property(o, key, desc) && false {'),
    # super.x.
    (I, '        self.get_value_with(base, key, this)', '        self.get_value_with(base, key, base)'),
    (I, '        self.set_value_with(base, key, value, this)', '        self.set_value_with(base, key, value, base)'),
    (I, '            .map_or(Value::NULL, |o| self.heap.get_prototype_of(o));\n        if base.is_nullish() {', '            .map_or(Value::NULL, |_| home);\n        if base.is_nullish() {'),
    (I, '            Got::Getter(g) => self.call_value(g, receiver, &[]),', '            Got::Getter(g) => self.call_value(g, base, &[]),'),
    (I, '            SetOutcome::Setter(s) => self.call_value(s, receiver, &[value]).map(|_| ()),', '            SetOutcome::Setter(s) => self.call_value(s, base, &[value]).map(|_| ()),'),
    # super(...).
    (I, '        if !self.is_constructor_value(parent) {\n            self.vm.stack.truncate(args);', '        if false {\n            self.vm.stack.truncate(args);'),
    (I, '        match self.begin_call_owned(parent, Value::UNDEFINED, args, Some(new_target))? {', '        match self.begin_call_owned(parent, Value::UNDEFINED, args, Some(parent))? {'),
    (I, '            let (from, to) = (f.args as usize, f.base as usize);', '            let (from, to) = (f.base as usize, f.base as usize);'),
    (I, '        for i in 0..n {\n            match self.get_value(array, PropertyKey::Index(i)) {', '        for i in 0..n.min(1) {\n            match self.get_value(array, PropertyKey::Index(i)) {'),
    (M, '                            _ => self.push_own_arguments(),', '                            _ => self.vm.stack.len(),'),
    (M, '                        if self.vm.acc != Value::UNINITIALIZED {\n                            let e =\n                                self.throw_error(ErrorType::ReferenceError, messages::SUPER_TWICE);',
        '                        if false {\n                            let e =\n                                self.throw_error(ErrorType::ReferenceError, messages::SUPER_TWICE);'),
    # A derived constructor's return.
    (I, '        if value.as_object().is_some() {\n            return Ok(value);', '        if false {\n            return Ok(value);'),
    (I, '        if !value.is_undefined() {\n            return Err(self.throw_error(ErrorType::TypeError, messages::DERIVED_RETURN));', '        if false {\n            return Err(self.throw_error(ErrorType::TypeError, messages::DERIVED_RETURN));'),
    (I, '        if this == Value::UNINITIALIZED {\n            return Err(self.this_before_super());', '        if false {\n            return Err(self.this_before_super());'),
    # The interpreter.
    (M, '        if !construct\n            && matches!(', '        if false\n            && matches!('),
    (M, '                            let e = if name == "this" {', '                            let e = if false {'),
    (M, '                        FunctionKind::Normal\n                            | FunctionKind::ClassConstructor\n                            | FunctionKind::DerivedConstructor',
        '                        FunctionKind::Normal'),
    (M, '                        self.vm.acc = self.home_object(f);', '                        self.vm.acc = Value::UNDEFINED;'),
    # Compiling.
    (CC, '            let target = if m.is_static { f } else { proto };', '            let target = if m.is_static { proto } else { proto };'),
    (CC, '            self.reg_op(Op::SetHomeObject, target);\n', ''),
    (CC, '        self.op(Op::CheckSuperOnce);\n', ''),
    (CC, '        self.reg_op(Op::Ldar, v);\n        self.store_raw("this")?;', '        self.reg_op(Op::Ldar, v);'),
    (CC, '                        1 => units("get ").into_vec(),', '                        1 => Vec::new(),'),
    (C, '        if derived {\n            self.pos(f.span);\n            self.derived_return()?;\n        }\n', ''),
    (ST, '        if self.is_derived() {\n            // Its value', '        if false {\n            // Its value'),
    (C, '                (BindingKind::This, Loc::Register(r)) => {\n                    self.op(Op::LdaThis);', '                (BindingKind::This, Loc::Register(r)) => {\n                    self.op(Op::LdaUndefined);'),
    (C, '                (BindingKind::FunctionObject, Loc::Slot(k)) => {\n                    self.op(Op::LdaClosure);', '                (BindingKind::FunctionObject, Loc::Slot(k)) => {\n                    self.op(Op::LdaUndefined);'),
    (C, '                (BindingKind::Home, Loc::Slot(k)) => {\n                    self.op(Op::LdaHomeObject);', '                (BindingKind::Home, Loc::Slot(k)) => {\n                    self.op(Op::LdaUndefined);'),
    (E, '                        if p.kind == PropKind::Method {\n                            self.reg_op(Op::SetHomeObject, o);', '                        if false {\n                            self.reg_op(Op::SetHomeObject, o);'),
    (E, '                    ) =>\n                {\n                    break;\n                }', '                    ) && false =>\n                {\n                    break;\n                }'),
    (ST, '                self.class(*c, None)?;\n                if let Some(n) = &self.ast.class(*c).name {', '                self.class(*c, None)?;\n                if let Some(n) = self.ast.class(*c).name.as_ref().filter(|_| false) {'),
    (SC, '                        let end = class.span.end;', '                        let end = 0;'),
    (SC, '                    uninit_this: b.kind == BindingKind::This\n                        && self.functions[scope.func as usize].derived,', '                    uninit_this: false,'),
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


FILTERS = ('class_tests',)
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
