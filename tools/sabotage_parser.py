"""SilverJS's parser and RegExp sabotage: one break at a time, `cargo test`,
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
S = 'src/parser/stmt.rs'
E = 'src/parser/expr.rs'
F = 'src/parser/func.rs'
P = 'src/parser/pattern.rs'
C = 'src/parser/scope.rs'
M = 'src/parser/mod.rs'
K = 'src/stack.rs'
D = 'src/parser/module.rs'
R = 'src/regexp.rs'
RP = 'src/regexp/properties.rs'
cases = [
    (S, "if !self.cx.return_ {", "if false {"),
    (S, "None if !self.in_loop =>", "None if false =>"),
    (S, "if !self.labels.iter().any(|l| l.name == label.name) {", "if false {"),
    (S, "if self.labels.iter().any(|l| l.name == label.name) {", "if false {"),
    (S, "if has_default {", "if false {"),
    (S, "if kind != VarKind::Var && kind != VarKind::Let {", "if false {"),
    (S, 'if &*name.name == "let" {', "if false {"),
    (S, "pending = tok.strict_error.clone();", "pending = None;"),
    (S, "Place::IfBody if sloppy =>", "Place::IfBody if false =>"),
    (S, "if self.tok.newline_before {\n                    return Err(self.error(\"Illegal newline after throw\"",
        "if false {\n                    return Err(self.error(\"Illegal newline after throw\""),
    (E, "top_prec > prec || (top_prec == prec && op != BinOp::Binary(BinaryOp::Exp))",
        "top_prec > prec || top_prec == prec"),
    (E, "if mixes(self, left) || mixes(self, right) {", "if false {"),
    (E, "Expr::Unary { .. } | Expr::Await(_)", "Expr::Await(_)"),
    (E, 'if self.cx.strict && (&*id.name == "eval"', 'if false && (&*id.name == "eval"'),
    (E, "cover.init = Some(self.tok_span());", "let _ = 0;"),
    (E, "cover.double_proto = Some(prop.span);", "let _ = 0;"),
    (E, "|| index + 1 != items.len() || rest_comma {", "|| index + 1 != items.len() {"),
    (E, "if self.tok.newline_before {\n            return Ok(expr);",
        "if false {\n            return Ok(expr);"),
    (E, "if next.newline_before {\n                let next = next.clone();\n                return Err(self.unexpected_token(&next));",
        "if false {\n                let next = next.clone();\n                return Err(self.unexpected_token(&next));"),
    (F, "if use_strict && !params.simple {", "if false {"),
    (F, "let duplicates_allowed = !strict && params.simple && kind == FuncKind::Normal;",
        "let duplicates_allowed = true;"),
    (F, "let sloppy_plain = !self.cx.strict && !is_async && !is_generator;",
        "let sloppy_plain = !is_async && !is_generator;"),
    (P, "if i + 1 != count || self.rest_not_last.contains(&x) {", "if i + 1 != count {"),
    (C, "if scope.lexical.contains(name) || scope.vars.contains(name) || clashes_param {",
        "if scope.lexical.contains(name) || clashes_param {"),
    (C, "                && !simple\n", "                && false\n"),
    (C, "let twice_allowed = sloppy_plain && scope.sloppy_functions.contains(name);",
        "let twice_allowed = scope.lexical.contains(name);"),
    (M, "if self.cx.yield_ || self.cx.strict {", "if self.cx.yield_ {"),
    (M, '"arguments" | "eval" if binding && self.cx.strict =>', '"arguments" | "eval" if false =>'),
    (M, "if self.at(Punct::RBrace) || self.at_eof() || self.tok.newline_before {",
        "if self.at(Punct::RBrace) || self.at_eof() {"),
    (K, "self.used() > self.budget", "false"),
    # Classes and private names.
    (F, "self.cx.super_call = matches!(kind, FuncKind::Constructor { derived: true });",
        "self.cx.super_call = matches!(kind, FuncKind::Constructor { .. });"),
    (F, "let saved = self.cx;\n        self.cx.strict = true;\n        let result = self.class_parts",
        "let saved = self.cx;\n        let result = self.class_parts"),
    (F, "            if self.is_bare_arrow(e) {\n                let span = self.ast.expr(e).span;\n                return Err(self.error(\"Malformed arrow function parameter list\", span));\n            }\n            Some(e)",
        "            Some(e)"),
    (F, "if scope.declared.contains_key(&used.name) {", "if true {"),
    (F, "&& &*p.name == \"constructor\"", "&& false"),
    (F, "let is_constructor = !is_static && key_name == Some(\"constructor\");",
        "let is_constructor = key_name == Some(\"constructor\");"),
    (F, "Some(\"Class constructor may not be an accessor\")", "None"),
    (F, "if constructor.is_some() {", "if false {"),
    (F, "if key_name == Some(\"constructor\") {", "if false {"),
    (F, "                    a == b\n", "                    true\n"),
    (F, "self.cx.class_field = true;", "self.cx.class_field = false;"),
    (F, "self.cx.static_block = static_block;", "self.cx.static_block = false;"),
    (E, "self.use_private(&name)?;\n            self.bump()?;\n            return Ok(MemberProp::Private(name));",
        "self.bump()?;\n            return Ok(MemberProp::Private(name));"),
    (E, "&& !(name != \"async\" && next.is(Punct::Star));", ";"),
    # Modules.
    (D, "if !seen.insert(name) {", "if false {"),
    (D, "if !self.scopes.top_declares(&local.name) {", "if false {"),
    (D, "if attributes.iter().any(|a| a.key == key) {", "if false {"),
    (D, "if char::decode_utf16(value.iter().copied()).any(|c| c.is_err()) {", "if false {"),
    (D, "if is_string || name_tok.kind != TokenKind::Identifier {", "if false {"),
    (D, "if is_string || tok.kind != TokenKind::Identifier {", "if false {"),
    (M, "            strict: module,\n", "            strict: false,\n"),
    (M, "            await_: module,\n", "            await_: false,\n"),
    (E, "                if !self.module {\n", "                if false {\n"),
    # RegExp patterns' early errors.
    (R, "if seen & 32 != 0 && seen & 128 != 0 {", "if false {"),
    (R, "if !checker.names.iter().any(|(n, _)| n == name) {", "if false {"),
    (R, "if self.quantifier()? && !quantifiable {", "if self.quantifier()? && false {"),
    (R, "if max.is_some_and(|max| max < min) {", "if false {"),
    (R, "            !self.unicode\n", "            true\n"),
    (R, "if clash {", "if false {"),
    (R, "if n <= u64::from(self.groups) {", "if true {"),
    (R, "if is_syntax_char(ch(c)) || c == '/' {", "if true {"),
    (R, "(ClassAtom::Char(a), ClassAtom::Char(b)) if a > b =>",
        "(ClassAtom::Char(a), ClassAtom::Char(b)) if false =>"),
    (R, "_ if self.unicode => return Err(\"Invalid character class\".to_owned()),",
        "_ if false => return Err(\"Invalid character class\".to_owned()),"),
    (R, "if negated && strings {", "if false {"),
    (R, "None if self.sets && !negated && properties::string_property(&name) => Ok(true),",
        "None if properties::string_property(&name) => Ok(true),"),
    (R, "'}' | ']' if self.unicode =>", "'}' | ']' if false =>"),
    (R, "if \"&!#$%*+,.:;<=>?@^`~\".contains(ch_) && self.peek_at(1) == Some(c) {", "if false {"),
    (R, "':' if any => return Ok(()),", "':' => return Ok(()),"),
    (RP, "\"Script\" | \"sc\" | \"Script_Extensions\" | \"scx\" => SCRIPT.binary_search(&value).is_ok(),",
        "\"Script\" | \"sc\" | \"Script_Extensions\" | \"scx\" => true,"),
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
    # The parser's own tests: the lexer's cannot fail by a parser break.
    if line.endswith(': test') and line.startswith('parser_tests::'):
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
        r = subprocess.run(['cargo', 'test', '-q', '--lib'], cwd=root, capture_output=True,
                           text=True, encoding='utf-8', errors='replace', timeout=900, env=env)
        out = r.stdout + r.stderr
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
    tag = 'CAUGHT' if r.returncode != 0 else 'MISSED'
    print(f'{tag} {a.strip()[:55]!r} -> {sorted(failed)}', flush=True)
print('never failed:', sorted(all_tests - failed_ever), flush=True)
