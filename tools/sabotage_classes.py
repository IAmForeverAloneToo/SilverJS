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
X = 'src/compiler/codegen/expr.rs'
O = 'src/object.rs'
H = 'src/heap.rs'
SK = 'src/builtins/stack.rs'
EV = 'src/interp/eval.rs'
P = 'src/parser/mod.rs'
cases = [
    # Making a class.
    (I, '        if self.code_kind(unit, func) == Some(FunctionKind::DerivedConstructor) {', '        if false {'),
    (I, '        } else if parent.is_null() {', '        } else if false {'),
    (I, '            if pp.as_object().is_none() && !pp.is_null() {', '            if false {'),
    (I, '                self.heap.set_prototype_of(o, ctor_parent);', '                let _ = ctor_parent;'),
    (I, '                        value: Some(proto),\n                        writable: Some(false),', '                        value: Some(proto),\n                        writable: Some(true),'),
    (I, '                    s.push(proto);\n', '                    s.push(Value::UNDEFINED);\n'),
    (I, '            _ => Descriptor {\n                value: Some(f),\n                writable: Some(true),\n                enumerable: Some(false),',
        '            _ => Descriptor {\n                value: Some(f),\n                writable: Some(true),\n                enumerable: Some(true),'),
    (I, '            1 => Descriptor {\n                get: Some(f),', '            1 => Descriptor {\n                set: Some(f),'),
    (I, '        if !self.heap.define_own_property(o, key, desc) {', '        if !self.heap.define_own_property(o, key, desc) && false {'),
    # super.x.
    (I, '        self.get_value_with(base, key, this)', '        self.get_value_with(base, key, base)'),
    (I, '        self.set_value_with(base, key, value, this, strict)', '        self.set_value_with(base, key, value, base, strict)'),
    (I, '                    .map_or(Value::NULL, |o| self.heap.get_prototype_of(o));\n            }\n            Op::GetSuper => {', '                    .map_or(Value::NULL, |_| home);\n            }\n            Op::GetSuper => {'),
    (I, '            Got::Getter(g) => self.call_value(g, receiver, &[]),', '            Got::Getter(g) => self.call_value(g, base, &[]),'),
    (I, '            SetOutcome::Setter(s) => self.call_value(s, receiver, &[value]).map(|_| ()),', '            SetOutcome::Setter(s) => self.call_value(s, base, &[value]).map(|_| ()),'),
    # super(...).
    (I, '        if !self.is_constructor_value(parent) {\n            self.vm.stack.truncate(args);', '        if false {\n            self.vm.stack.truncate(args);'),
    (I, '        match self.begin_call_owned(parent, Value::UNDEFINED, args, Some(new_target))? {', '        match self.begin_call_owned(parent, Value::UNDEFINED, args, Some(parent))? {'),
    (I, '            let (from, to) = (f.args as usize, f.base as usize);', '            let (from, to) = (f.base as usize, f.base as usize);'),
    (I, '        for i in 0..n {\n            match self.get_value(array, PropertyKey::Index(i)) {', '        for i in 0..n.min(1) {\n            match self.get_value(array, PropertyKey::Index(i)) {'),
    (M, '                            _ => self.push_own_arguments(),', '                            _ => self.vm.stack.len(),'),
    (I, '                if self.vm.acc != Value::UNINITIALIZED {\n                    let e = self.throw_error(ErrorType::ReferenceError, messages::SUPER_TWICE);',
        '                if false {\n                    let e = self.throw_error(ErrorType::ReferenceError, messages::SUPER_TWICE);'),
    # A derived constructor's return.
    (I, '        if value.as_object().is_some() {\n            return Ok(value);', '        if false {\n            return Ok(value);'),
    (I, '        if !value.is_undefined() {\n            return Err(self.throw_error(ErrorType::TypeError, messages::DERIVED_RETURN));', '        if false {\n            return Err(self.throw_error(ErrorType::TypeError, messages::DERIVED_RETURN));'),
    (I, '        if this == Value::UNINITIALIZED {\n            return Err(self.this_before_super());', '        if false {\n            return Err(self.this_before_super());'),
    # The interpreter.
    (M, '        if !construct\n            && matches!(', '        if false\n            && matches!('),
    (M, '                            let e = if name == "this" {', '                            let e = if false {'),
    (M, '                        FunctionKind::Normal\n                            | FunctionKind::ClassConstructor\n                            | FunctionKind::DerivedConstructor',
        '                        FunctionKind::Normal'),
    (I, '                self.vm.acc = self.home_object(f);', '                self.vm.acc = Value::UNDEFINED;'),
    # Compiling.
    (CC, '            let target = if m.is_static { f } else { proto };', '            let target = if m.is_static { proto } else { proto };'),
    (CC, '            self.reg_op(Op::SetHomeObject, target);\n            self.pos(m.span);', '            self.pos(m.span);'),
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
    # J3.3b: private names.
    (I, '            let brand = if branded {', '            let brand = if false {'),
    (I, '        if !self\n            .heap\n            .private_add(o, PropertyKey::Symbol(i), Value::TRUE)\n        {',
        '        if self\n            .heap\n            .private_add(o, PropertyKey::Symbol(i), Value::TRUE)\n            && false\n        {'),
    (I, '            if !self.heap.private_add(o, k, value) {', '            if !self.heap.private_add(o, k, value) && false {'),
    (I, '            return obj == p.brand;', '            return true;'),
    (I, '            _ if !self.has_brand(obj, &p) => Err(self.receiver_error(&p)),\n            PrivateKind::Method => Ok(p.get),',
        '            PrivateKind::Method => Ok(p.get),'),
    (I, '            _ if !self.has_brand(obj, &p) => Err(self.receiver_error(&p)),\n            PrivateKind::Method => {',
        '            PrivateKind::Method => {'),
    (I, '                if p.get.is_undefined() {', '                if false {'),
    (I, '                if p.set.is_undefined() {', '                if false {'),
    (I, '                self.call_value(p.get, obj, &[])', '                self.call_value(p.get, Value::UNDEFINED, &[])'),
    (I, '                self.call_value(p.set, obj, &[value]).map(|_| ())', '                self.call_value(p.set, obj, &[]).map(|_| ())'),
    (I, '            PrivateKind::Field => self.heap.private_get(o, k).is_some(),', '            PrivateKind::Field => true,'),
    (I, '                Some(o) if self.heap.private_get(o, k).is_some() => {', '                Some(o) => {'),
    (I, '        p.is_static = is_static;', '        p.is_static = false;'),
    (O, '                    && self.symbols.get(s).is_none_or(|c| c.private.is_none())', '                    && true'),
    (O, '        if self.private_get(o, key).is_some() {\n            return false;', '        if false {\n            return false;'),
    (H, '            out.extend([p.get, p.set, p.brand]);\n', ''),
    # J3.3b: fields.
    (I, '                s.extend([key, init]);', '                s.extend([init, key]);'),
    (I, '            enumerable: Some(true),\n            configurable: Some(true),\n            ..Descriptor::default()\n        };\n        if !self.heap.define_own_property(o, k, desc) {',
        '            enumerable: Some(false),\n            configurable: Some(true),\n            ..Descriptor::default()\n        };\n        if !self.heap.define_own_property(o, k, desc) {'),
    (I, '            let message = if self.heap.objects.get(o).is_some_and(|c| !c.is_extensible()) {', '            let message = if false {'),
    (CC, '                self.emit(Op::CreatePrivateName, &[c]);', '                self.op(Op::LdaUndefined);'),
    (CC, '                            self.reg_op(Op::SetHomeObject, proto);', '                            let _ = proto;'),
    (CC, '                    self.reg_op(Op::SetHomeObject, f);\n                    let c = self.alloc();', '                    let c = self.alloc();'),
    (CC, '                let flags = how | (i64::from(m.is_static) * 4);', '                let flags = how;'),
    (CC, '            self.sh.field_names.insert(index, n);\n', ''),
    (CC, '            self.emit(Op::AddPrivateBrand, &[i64::from(this), i64::from(f)]);\n', ''),
    (CC, '                self.emit(Op::GetClassField, &[i64::from(f), i, 1]);', '                self.emit(Op::GetClassField, &[i64::from(f), i, 0]);'),
    (CC, '                self.emit(Op::DefineFieldKey, &[i64::from(f), i64::from(*k)]);\n', ''),
    (CC, '            self.init_instance(id, f, v, span, span.start)?;', '            let _ = (id, f);'),
    (C, '            self.init_instance(id, c, t, f.span, f.span.start)?;', '            let _ = (id, c, t);'),
    (X, '                self.reg_op(Op::HasPrivate, n);', '                self.op(Op::LdaTrue);'),
    (SC, '                            self.declare(s, &name, BindingKind::Private, 0, None, None);\n', ''),
    (SC, '                        self.a.ctor_class.insert(c, id);\n', ''),
    (SK, '        kind == Some(crate::compiler::FunctionKind::Initializer)', '        false'),
    (SK, "                n.starts_with('#') || n.starts_with(\"get #\") || n.starts_with(\"set #\")", '                n.is_empty()'),
    # J3.3b: super references in order, and eval in class code.
    (CC, '        self.reg_op(Op::GetSuperBase, home);\n        self.reg_op(Op::Star, home);\n', ''),
    (CC, '        if convert {', '        if false {'),
    (CC, '                i64::from(self.strict),', '                1,'),
    (I, '            SetOutcome::Done(false) if !strict => Ok(()),\n', ''),
    (SC, '            for name in ["this", "new.target", "arguments", "%home", "%fn"] {', '            for name in ["this", "new.target", "arguments"] {'),
    (SC, '                        self.declare(c, "%field", BindingKind::Var, 0, None, None);', '                        let _ = c;'),
    (EV, '            privates: self.private_names_around(frame.context),', '            privates: Vec::new(),'),
    (P, '            if !scope.declared.contains_key(&used.name) {', '            if false {'),
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
