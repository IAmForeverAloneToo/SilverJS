"""SilverJS's sabotage of Date (J4.3d): the time arithmetic, local time,
the string forms, the Date Time String Format and the other forms the
reference browser reads, and the methods. One break at a time,
`cargo test`, restored byte for byte. Prints which tests each break
fails, then every test no break failed (which must be none).

    python -I tools/sabotage_date.py <repository root>

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
D = 'src/date.rs'
BD = 'src/builtins/date.rs'
OB = 'src/builtins/object.rs'
cases = [
    # Days and fields.
    (D, '    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;', '    let doe = yoe * 365 + yoe / 4 + doy;'),
    (D, '    let m = if mp < 10 { mp + 3 } else { mp - 9 };', '    let m = if mp < 10 { mp + 2 } else { mp - 9 };'),
    (D, '        weekday: (d + 4).rem_euclid(7),', '        weekday: (d + 3).rem_euclid(7),'),
    (D, '    let ym = y + (m / 12.0).floor();', '    let ym = y + (m / 12.0).trunc();'),
    (D, '    t.trunc() + 0.0\n', '    t.trunc()\n'),
    # Local time: a repeated hour's earlier instant, a skipped hour's
    # offset from before.
    (D, '        (true, true) => a.min(b),', '        (true, true) => a.max(b),'),
    (D, '        (false, false) => a,', '        (false, false) => b,'),
    # The forms.
    (D, '    let minutes = (offset.abs() / MS_PER_MINUTE).floor() as i64;',
     '    let minutes = (offset.abs() / MS_PER_MINUTE).ceil() as i64;'),
    (D, '        format!("-{:04}", -y)', '        format!("{y:04}")'),
    (D, '    let year = if (0..=9999).contains(&f.year) {', '    let year = if (0..=99999).contains(&f.year) {'),
    # The Date Time String Format.
    (D, '            return Iso::Not(7);', '            return Iso::Not(0);'),
    (D, '        return Iso::Value(time_clip(make_date(day, 0.0)));',
     '        return Iso::Value(time_clip(utc(tz, make_date(day, 0.0))));'),
    (D, "    if (c.eat(b'Z') || c.eat(b'z')) && c.done() {", "    if c.eat(b'Z') && c.done() {"),
    # The other forms.
    (D, '        Some(v) if v < 50 => 2000.0 + v as f64,', '        Some(v) if v <= 50 => 2000.0 + v as f64,'),
    (D, '    let year_like = |v: u64| v > 31 || v == 0;', '    let year_like = |v: u64| v > 31;'),
    (D, '        None => 2001.0,', '        None => 2000.0,'),
    (D, '                if negative && tok.glued {', '                if negative {'),
    (D, '                    } else if let Some(Kind::Number { value: v, .. }) = at(k)\n                        && v < 60\n',
     '                    } else if let Some(Kind::Number { value: v, .. }) = at(k)\n'),
    (D, '                        .is_some_and(|t| t.kind == Kind::Colon && t.glued)',
     '                        .is_some_and(|t| t.kind == Kind::Colon)'),
    (D, '                            if at(k) == Some(Kind::Colon) {\n                                return f64::NAN;',
     '                            if false {\n                                return f64::NAN;'),
    (D, '            Kind::Close if read_number => return f64::NAN,', '            Kind::Close if false => return f64::NAN,'),
    (D, '                    zone = Some(if negative { -off } else { off });',
     '                    zone = Some(zone.unwrap_or(0.0) + if negative { -off } else { off });'),
    (D, '        || (0x5b..=0x60).contains(&c)\n', ''),
    (D, '    while let Some(&c) = s.get(i).filter(|&&c| c != 0) {', '    while let Some(&c) = s.get(i) {'),
    (D, '    let year = if f.year <= 0 { 1 - f.year } else { f.year };', '    let year = f.year;'),
    (D, '        "est" | "cdt" => -5.0,', '        "est" | "cdt" => -4.0,'),
    (D, "        h = h % 12 + if pm { 12 } else { 0 };", "        h += if pm { 12 } else { 0 };"),
    (D, '                } else if read_number {', '                } else if false {'),
    # The methods.
    (BD, '            if first != YEAR {', '            if false {'),
    (BD, '        if (0.0..=99.0).contains(&yi) {', '        if (0.0..=100.0).contains(&yi) {'),
    (BD, '    Ok(Value::number((t - local_time(agent.zone(), t)) / 60_000.0))',
     '    Ok(Value::number((local_time(agent.zone(), t) - t) / 60_000.0))'),
    (BD, '        "string" | "default" => true,', '        "string" => true,'),
    (BD, '        if tv.as_number().is_some_and(|n| !n.is_finite()) {',
     '        if tv.as_number().is_some_and(f64::is_nan) {'),
    (BD, '            match agent.date_value(value) {\n                Some((_, t)) => t,',
     '            match None::<(u32, f64)> {\n                Some((_, t)) => t,'),
    (BD, '            Some(clock) => clock(),', '            Some(_) => 0.0,'),
    (BD, '        return Err(agent.incompatible("Date.prototype.setTime", c.this));',
     '        return Err(agent.type_error(NOT_A_DATE));'),
    (OB, '            Some(ObjectKind::Date) => "Date",\n', ''),
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


FILTERS = ('date_tests::',)
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
