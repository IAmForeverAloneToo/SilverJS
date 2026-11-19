"""SilverJS's parser sabotage: one break at a time, `cargo test`,
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
    (E, "if next.newline_before {\n                return Err(self.unexpected_token(&next));",
        "if false {\n                return Err(self.unexpected_token(&next));"),
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
    (K, "here().abs_diff(self.base) > self.budget", "false"),
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
