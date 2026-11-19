//! Regular expressions: the early errors of a pattern and its flags
//! (sections 13.2.7.2 and 22.2.1, with Annex B.1.2's grammar for
//! patterns without `u` or `v`). The matcher comes with the built-ins
//! (J4); this module holds the syntax both will share.
//!
//! The checker reads the pattern once to count its capturing groups and
//! find whether any is named (which decides how `\k` and a decimal escape
//! read: section 22.2.1's reparse with [+NamedCaptureGroups]), then once
//! to check it. Groups and classes nest by recursion bounded at
//! [`MAX_DEPTH`].

use crate::stack::StackGuard;
use crate::unicode::{is_id_continue, is_id_start};

mod properties;

/// How deep groups and classes may nest.
const MAX_DEPTH: u32 = 1000;

/// The early errors of a RegularExpressionLiteral.
pub(crate) fn check_literal(body: &str, flags: &str, stack: StackGuard) -> Result<(), String> {
    check_flags(flags).map_err(|()| "Invalid regular expression flags".to_owned())?;
    let unicode = flags.contains('u') || flags.contains('v');
    let sets = flags.contains('v');
    check_pattern(body, unicode, sets, stack)
        .map_err(|m| format!("Invalid regular expression: /{body}/{flags}: {m}"))
}

/// The flags' rules: each one of `dgimsuyv`, none twice, not both `u`
/// and `v`.
pub(crate) fn check_flags(flags: &str) -> Result<(), ()> {
    let mut seen = 0u32;
    for c in flags.chars() {
        let bit = match c {
            'd' => 1,
            'g' => 2,
            'i' => 4,
            'm' => 8,
            's' => 16,
            'u' => 32,
            'y' => 64,
            'v' => 128,
            _ => return Err(()),
        };
        if seen & bit != 0 {
            return Err(());
        }
        seen |= bit;
    }
    if seen & 32 != 0 && seen & 128 != 0 {
        return Err(());
    }
    Ok(())
}

/// Checks a pattern: `unicode` for the `u` or `v` flag, `sets` for `v`.
pub(crate) fn check_pattern(
    pattern: &str,
    unicode: bool,
    sets: bool,
    stack: StackGuard,
) -> Result<(), String> {
    // In Unicode mode the pattern is code points; otherwise UTF-16 code
    // units, so that a surrogate pair is two characters (a class range
    // across one is out of order).
    let units: Vec<u32> = if unicode {
        pattern.chars().map(u32::from).collect()
    } else {
        pattern.encode_utf16().map(u32::from).collect()
    };
    let (groups, named) = prescan(&units, sets);
    let mut checker = Checker {
        s: units,
        i: 0,
        unicode,
        sets,
        named: unicode || named,
        groups,
        names: Vec::new(),
        references: Vec::new(),
        path: Vec::new(),
        next_disjunction: 0,
        depth: 0,
        stack,
    };
    checker.disjunction()?;
    if checker.i < checker.s.len() {
        return Err("Unmatched ')'".to_owned());
    }
    for name in &checker.references {
        if !checker.names.iter().any(|(n, _)| n == name) {
            return Err("Invalid named capture referenced".to_owned());
        }
    }
    Ok(())
}

/// Counts the capturing groups and finds whether any is named, skipping
/// escapes and classes.
fn prescan(s: &[u32], sets: bool) -> (u32, bool) {
    let mut groups = 0u32;
    let mut named = false;
    let mut class_depth = 0u32;
    let mut i = 0;
    while let Some(&c) = s.get(i) {
        match char::from_u32(c).unwrap_or('\0') {
            '\\' => i += 1,
            '[' if class_depth == 0 || sets => class_depth += 1,
            ']' if class_depth > 0 => class_depth -= 1,
            '(' if class_depth == 0 => {
                if s.get(i + 1) != Some(&ch('?')) {
                    groups += 1;
                } else if s.get(i + 2) == Some(&ch('<'))
                    && !matches!(
                        s.get(i + 3).and_then(|&c| char::from_u32(c)),
                        Some('=' | '!')
                    )
                {
                    groups += 1;
                    named = true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (groups, named)
}

/// A class atom: one character, or a set (`\d`, `\p{...}`).
#[derive(Clone, Copy)]
enum ClassAtom {
    Char(u32),
    Set,
}

const INVALID_SET: &str = "Invalid set operation in character class";
const UNTERMINATED_CLASS: &str = "Unterminated character class";
/// ClassSetReservedPunctuator: what `v` mode lets be escaped in a class.
const RESERVED_PUNCTUATORS: &str = "&-!#%,:;<=>@`~";

struct Checker {
    s: Vec<u32>,
    i: usize,
    unicode: bool,
    sets: bool,
    /// [+NamedCaptureGroups]: Unicode mode, or a pattern with a named
    /// group.
    named: bool,
    groups: u32,
    /// Each named group and the alternatives it is in, to allow a name
    /// twice only in alternatives that cannot both match (section
    /// 22.2.1.1's MightBothParticipate).
    names: Vec<(String, Vec<(u32, u32)>)>,
    references: Vec<String>,
    /// The (disjunction, alternative) pairs around the current point.
    path: Vec<(u32, u32)>,
    next_disjunction: u32,
    depth: u32,
    stack: StackGuard,
}

fn ch(c: char) -> u32 {
    u32::from(c)
}

fn is_syntax_char(c: u32) -> bool {
    char::from_u32(c).is_some_and(|c| "^$\\.*+?()[]{}|".contains(c))
}

fn digit(c: Option<u32>, radix: u32) -> Option<u32> {
    char::from_u32(c?)?.to_digit(radix)
}

impl Checker {
    fn peek(&self) -> Option<u32> {
        self.s.get(self.i).copied()
    }

    /// The current character; outside Unicode mode a surrogate code unit
    /// is no `char` and reads as U+FFFD, an ordinary character (its own
    /// value is read with [`Checker::peek`] where it matters).
    fn peek_char(&self) -> Option<char> {
        self.peek()
            .map(|c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER))
    }

    fn peek_at(&self, ahead: usize) -> Option<u32> {
        self.s.get(self.i + ahead).copied()
    }

    fn at(&self, c: char) -> bool {
        self.peek() == Some(ch(c))
    }

    fn eat(&mut self, c: char) -> bool {
        if self.at(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn eat_str(&mut self, s: &str) -> bool {
        let matches = s
            .chars()
            .enumerate()
            .all(|(k, c)| self.peek_at(k) == Some(ch(c)));
        if matches {
            self.i += s.chars().count();
        }
        matches
    }

    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH || self.stack.exceeded() {
            return Err("Regular expression too large".to_owned());
        }
        let result = f(self);
        self.depth -= 1;
        result
    }

    fn disjunction(&mut self) -> Result<(), String> {
        let id = self.next_disjunction;
        self.next_disjunction += 1;
        self.path.push((id, 0));
        let result = self.disjunction_parts();
        self.path.pop();
        result
    }

    fn disjunction_parts(&mut self) -> Result<(), String> {
        self.alternative()?;
        let mut alt = 0;
        while self.eat('|') {
            alt += 1;
            if let Some(top) = self.path.last_mut() {
                top.1 = alt;
            }
            self.alternative()?;
        }
        Ok(())
    }

    fn alternative(&mut self) -> Result<(), String> {
        while let Some(c) = self.peek() {
            if c == ch('|') || c == ch(')') {
                break;
            }
            self.term()?;
        }
        Ok(())
    }

    fn term(&mut self) -> Result<(), String> {
        let Some(c) = self.peek_char() else {
            return Ok(());
        };
        let quantifiable = match c {
            '^' | '$' => {
                self.i += 1;
                false
            }
            '\\' if matches!(self.peek_at(1).and_then(char::from_u32), Some('b' | 'B')) => {
                self.i += 2;
                false
            }
            '(' => self.group()?,
            '[' => {
                self.i += 1;
                self.class()?;
                true
            }
            '.' => {
                self.i += 1;
                true
            }
            '\\' => {
                self.i += 1;
                self.atom_escape()?;
                true
            }
            '*' | '+' | '?' => return Err("Nothing to repeat".to_owned()),
            '{' => {
                if self.unicode {
                    return Err("Lone quantifier brackets".to_owned());
                }
                // Annex B: a `{` that begins no quantifier is itself.
                if self.quantifier_at(self.i).is_some() {
                    return Err("Nothing to repeat".to_owned());
                }
                self.i += 1;
                true
            }
            '}' | ']' if self.unicode => return Err("Lone quantifier brackets".to_owned()),
            _ => {
                self.i += 1;
                true
            }
        };
        if self.quantifier()? && !quantifiable {
            return Err("Nothing to repeat".to_owned());
        }
        Ok(())
    }

    /// `{n}`, `{n,}` or `{n,m}` at `at`: its length and bounds.
    fn quantifier_at(&self, at: usize) -> Option<(usize, u64, Option<u64>)> {
        let mut j = at;
        if self.s.get(j) != Some(&ch('{')) {
            return None;
        }
        j += 1;
        let digits = |j: &mut usize| -> Option<u64> {
            let start = *j;
            let mut n: u64 = 0;
            while let Some(d) = digit(self.s.get(*j).copied(), 10) {
                n = n.saturating_mul(10).saturating_add(u64::from(d));
                *j += 1;
            }
            (*j > start).then_some(n)
        };
        let min = digits(&mut j)?;
        let max = if self.s.get(j) == Some(&ch(',')) {
            j += 1;
            digits(&mut j)
        } else {
            Some(min)
        };
        if self.s.get(j) != Some(&ch('}')) {
            return None;
        }
        Some((j + 1 - at, min, max))
    }

    /// A quantifier after an atom, if one is there.
    fn quantifier(&mut self) -> Result<bool, String> {
        match self.peek_char() {
            Some('*' | '+' | '?') => self.i += 1,
            Some('{') => match self.quantifier_at(self.i) {
                Some((len, min, max)) => {
                    if max.is_some_and(|max| max < min) {
                        return Err("numbers out of order in {} quantifier".to_owned());
                    }
                    self.i += len;
                }
                None if self.unicode => return Err("Incomplete quantifier".to_owned()),
                None => return Ok(false),
            },
            _ => return Ok(false),
        }
        self.eat('?');
        Ok(true)
    }

    /// A group of any kind, from its `(`; whether a quantifier may follow.
    fn group(&mut self) -> Result<bool, String> {
        self.i += 1;
        let quantifiable = if self.eat_str("?=") || self.eat_str("?!") {
            // Annex B lets a lookahead be quantified outside Unicode mode.
            !self.unicode
        } else if self.eat_str("?<=") || self.eat_str("?<!") {
            false
        } else if self.eat_str("?:") {
            true
        } else if self.eat_str("?<") {
            let name = self.group_name()?;
            let path = &self.path;
            let clash = self.names.iter().any(|(n, other)| {
                *n == name
                    && !other
                        .iter()
                        .any(|(d, a)| path.iter().any(|(d2, a2)| d == d2 && a != a2))
            });
            if clash {
                return Err("Duplicate capture group name".to_owned());
            }
            self.names.push((name, self.path.clone()));
            true
        } else if self.eat('?') {
            self.modifiers()?;
            true
        } else {
            true
        };
        self.nested(|c| c.disjunction())?;
        if !self.eat(')') {
            return Err("Unterminated group".to_owned());
        }
        Ok(quantifiable)
    }

    /// `ims-ims:` after a group's `(?`: no flag twice, at least one.
    fn modifiers(&mut self) -> Result<(), String> {
        let mut seen = 0u8;
        let mut any = false;
        let mut dash = false;
        loop {
            let Some(c) = self.peek_char() else {
                return Err("Invalid group".to_owned());
            };
            self.i += 1;
            let bit = match c {
                'i' => 1,
                'm' => 2,
                's' => 4,
                '-' if !dash => {
                    dash = true;
                    continue;
                }
                ':' if any => return Ok(()),
                ':' if dash => return Err("Invalid flags in modifiers".to_owned()),
                _ => return Err("Invalid group".to_owned()),
            };
            if seen & bit != 0 {
                return Err("Repeated flag in modifiers".to_owned());
            }
            seen |= bit;
            any = true;
        }
    }

    /// A GroupName after `(?<` or `\k<`, through its `>`.
    fn group_name(&mut self) -> Result<String, String> {
        let bad = || "Invalid capture group name".to_owned();
        let mut name = String::new();
        loop {
            let c = self.peek().ok_or_else(bad)?;
            self.i += 1;
            if c == ch('>') {
                break;
            }
            let c = if c == ch('\\') {
                if !self.eat('u') {
                    return Err(bad());
                }
                self.unicode_escape_in_name().ok_or_else(bad)?
            } else if (0xD800..0xDC00).contains(&c)
                && let Some(lo) = self.peek().filter(|lo| (0xDC00..0xE000).contains(lo))
            {
                self.i += 1;
                0x10000 + ((c - 0xD800) << 10) + (lo - 0xDC00)
            } else {
                c
            };
            let c = char::from_u32(c).ok_or_else(bad)?;
            let ok = if name.is_empty() {
                c == '$' || c == '_' || is_id_start(c)
            } else {
                c == '$' || c == '\u{200C}' || c == '\u{200D}' || is_id_continue(c)
            };
            if !ok {
                return Err(bad());
            }
            name.push(c);
        }
        if name.is_empty() {
            return Err(bad());
        }
        Ok(name)
    }

    /// `\u` in a group name, after the `u`: `XXXX`, a surrogate pair of
    /// them, or `{X...}`, in any mode (RegExpIdentifierName).
    fn unicode_escape_in_name(&mut self) -> Option<u32> {
        if self.eat('{') {
            let v = self.hex_digits()?;
            return (self.eat('}') && v <= 0x10FFFF).then_some(v);
        }
        let hi = self.hex_exact(4)?;
        Some(self.trail_surrogate(hi))
    }

    /// After a lead surrogate `hi` read from `\uXXXX`, a `\uXXXX` trail
    /// surrogate makes one code point with it.
    fn trail_surrogate(&mut self, hi: u32) -> u32 {
        if (0xD800..0xDC00).contains(&hi)
            && self.peek() == Some(ch('\\'))
            && self.peek_at(1) == Some(ch('u'))
        {
            let save = self.i;
            self.i += 2;
            if let Some(lo) = self.hex_exact(4).filter(|lo| (0xDC00..0xE000).contains(lo)) {
                return 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
            }
            self.i = save;
        }
        hi
    }

    /// Exactly `n` hex digits.
    fn hex_exact(&mut self, n: usize) -> Option<u32> {
        let mut v = 0u32;
        for k in 0..n {
            v = v * 16 + digit(self.peek_at(k), 16)?;
        }
        self.i += n;
        Some(v)
    }

    /// One or more hex digits, saturating.
    fn hex_digits(&mut self) -> Option<u32> {
        let start = self.i;
        let mut v = 0u32;
        while let Some(d) = digit(self.peek(), 16) {
            v = v.saturating_mul(16).saturating_add(d);
            self.i += 1;
        }
        (self.i > start).then_some(v)
    }

    /// An AtomEscape after its `\`.
    fn atom_escape(&mut self) -> Result<(), String> {
        match self.peek_char() {
            None => Err("\\ at end of pattern".to_owned()),
            Some('1'..='9') => {
                let start = self.i;
                let mut n: u64 = 0;
                while let Some(d) = digit(self.peek(), 10) {
                    n = n.saturating_mul(10).saturating_add(u64::from(d));
                    self.i += 1;
                }
                if n <= u64::from(self.groups) {
                    return Ok(());
                }
                if self.unicode {
                    return Err("Invalid escape".to_owned());
                }
                // Annex B: a legacy octal escape, or `\8` / `\9` as
                // themselves.
                self.i = start;
                self.legacy_octal();
                Ok(())
            }
            Some('k') if self.named => {
                self.i += 1;
                if !self.eat('<') {
                    return Err("Invalid named reference".to_owned());
                }
                let name = self
                    .group_name()
                    .map_err(|_| "Invalid named reference".to_owned())?;
                self.references.push(name);
                Ok(())
            }
            Some(_) => self.character_or_class_escape(false).map(|_| ()),
        }
    }

    /// Annex B's LegacyOctalEscapeSequence, or a digit 8 or 9 read as
    /// itself.
    fn legacy_octal(&mut self) -> u32 {
        let first = digit(self.peek(), 10).unwrap_or(0);
        self.i += 1;
        if first >= 8 {
            return ch('0') + first;
        }
        let mut v = first;
        let max_digits = if first <= 3 { 3 } else { 2 };
        for _ in 1..max_digits {
            match digit(self.peek(), 8) {
                Some(d) => {
                    v = v * 8 + d;
                    self.i += 1;
                }
                None => break,
            }
        }
        v
    }

    /// A CharacterEscape or CharacterClassEscape after its `\` (in a
    /// class when `in_class`): the character, or a set.
    fn character_or_class_escape(&mut self, in_class: bool) -> Result<ClassAtom, String> {
        let Some(raw) = self.peek() else {
            return Err("\\ at end of pattern".to_owned());
        };
        let c = char::from_u32(raw).unwrap_or(char::REPLACEMENT_CHARACTER);
        self.i += 1;
        let value = match c {
            'd' | 'D' | 's' | 'S' | 'w' | 'W' => return Ok(ClassAtom::Set),
            'p' | 'P' if self.unicode => {
                self.property(c == 'P')?;
                return Ok(ClassAtom::Set);
            }
            'f' => 0x0C,
            'n' => 0x0A,
            'r' => 0x0D,
            't' => 0x09,
            'v' => 0x0B,
            'c' => match self.peek_char() {
                Some(l) if l.is_ascii_alphabetic() => {
                    self.i += 1;
                    u32::from(l) % 32
                }
                // Annex B's ClassControlLetter: a digit or `_` in a class.
                Some(l) if in_class && !self.unicode && (l.is_ascii_digit() || l == '_') => {
                    self.i += 1;
                    u32::from(l) % 32
                }
                _ if self.unicode => return Err("Invalid unicode escape".to_owned()),
                _ => {
                    // Annex B: `\c` is a backslash, and the `c` reads again.
                    self.i -= 1;
                    ch('\\')
                }
            },
            '0' if !self.peek_char().is_some_and(|d| d.is_ascii_digit()) => 0,
            '0'..='9' => {
                if self.unicode {
                    return Err(if in_class {
                        "Invalid class escape".to_owned()
                    } else {
                        "Invalid decimal escape".to_owned()
                    });
                }
                self.i -= 1;
                self.legacy_octal()
            }
            'x' => match self.hex_exact(2) {
                Some(v) => v,
                None if self.unicode => return Err("Invalid escape".to_owned()),
                None => ch('x'),
            },
            'u' => {
                if self.unicode && self.eat('{') {
                    match self.hex_digits() {
                        Some(v) if v <= 0x10FFFF && self.eat('}') => v,
                        _ => return Err("Invalid Unicode escape".to_owned()),
                    }
                } else {
                    match self.hex_exact(4) {
                        Some(hi) if self.unicode => self.trail_surrogate(hi),
                        Some(hi) => hi,
                        None if self.unicode => return Err("Invalid Unicode escape".to_owned()),
                        None => ch('u'),
                    }
                }
            }
            'b' if in_class => 0x08,
            '-' if in_class && self.unicode => ch('-'),
            _ if self.unicode => {
                if is_syntax_char(ch(c)) || c == '/' {
                    ch(c)
                } else {
                    return Err("Invalid escape".to_owned());
                }
            }
            // Annex B's IdentityEscape: anything but `c` (and `k` with
            // named groups, read before).
            _ => raw,
        };
        Ok(ClassAtom::Char(value))
    }

    /// `{Name}` or `{Name=Value}` after `\p` or `\P` (`negated`); whether
    /// it is a property of strings (only `\p` may be, with `v`).
    fn property(&mut self, negated: bool) -> Result<bool, String> {
        let bad = || "Invalid property name".to_owned();
        if !self.eat('{') {
            return Err(bad());
        }
        let mut name = String::new();
        let mut value: Option<String> = None;
        loop {
            let c = self.peek_char().ok_or_else(bad)?;
            self.i += 1;
            match c {
                '}' => break,
                '=' if value.is_none() => value = Some(String::new()),
                c if c.is_ascii_alphanumeric() || c == '_' => match &mut value {
                    Some(v) => v.push(c),
                    None => name.push(c),
                },
                _ => return Err(bad()),
            }
        }
        match value {
            Some(value) if properties::valid_value(&name, &value) => Ok(false),
            Some(_) => Err(bad()),
            None if properties::lone_name(&name) => Ok(false),
            None if self.sets && !negated && properties::string_property(&name) => Ok(true),
            None => Err(bad()),
        }
    }

    /// A character class after its `[`, through its `]`.
    fn class(&mut self) -> Result<(), String> {
        if self.sets {
            return self.nested(|c| c.class_set_contents()).map(|_| ());
        }
        self.eat('^');
        loop {
            let Some(c) = self.peek() else {
                return Err(UNTERMINATED_CLASS.to_owned());
            };
            if c == ch(']') {
                self.i += 1;
                return Ok(());
            }
            let first = self.class_atom()?;
            if self.at('-') && self.peek_at(1).is_some_and(|c| c != ch(']')) {
                self.i += 1;
                let second = self.class_atom()?;
                match (first, second) {
                    (ClassAtom::Char(a), ClassAtom::Char(b)) if a > b => {
                        return Err("Range out of order in character class".to_owned());
                    }
                    (ClassAtom::Char(_), ClassAtom::Char(_)) => {}
                    // Annex B: a set beside `-` outside Unicode mode makes
                    // the `-` a character.
                    _ if self.unicode => return Err("Invalid character class".to_owned()),
                    _ => {}
                }
            }
        }
    }

    /// One ClassAtom outside `v` mode.
    fn class_atom(&mut self) -> Result<ClassAtom, String> {
        let Some(c) = self.peek() else {
            return Err(UNTERMINATED_CLASS.to_owned());
        };
        self.i += 1;
        if c == ch('\\') {
            return self.character_or_class_escape(true);
        }
        Ok(ClassAtom::Char(c))
    }

    /// A `v`-mode class's contents after its `[`, through its `]`: whether
    /// it may contain strings (section 22.2.1.6's MayContainStrings).
    fn class_set_contents(&mut self) -> Result<bool, String> {
        let negated = self.eat('^');
        if self.eat(']') {
            return Ok(false);
        }
        let (first, first_char) = self.class_set_operand()?;
        let strings = if self.at('&') && self.peek_at(1) == Some(ch('&')) {
            // ClassIntersection: operands joined by `&&`.
            let mut all = first;
            while self.eat_str("&&") {
                if self.at('&') {
                    return Err(INVALID_SET.to_owned());
                }
                let (s, _) = self.class_set_operand()?;
                all &= s;
                if !self.at(']') && !(self.at('&') && self.peek_at(1) == Some(ch('&'))) {
                    return Err(INVALID_SET.to_owned());
                }
            }
            all
        } else if self.at('-') && self.peek_at(1) == Some(ch('-')) {
            // ClassSubtraction: operands joined by `--`.
            while self.eat_str("--") {
                self.class_set_operand()?;
                if !self.at(']') && !(self.at('-') && self.peek_at(1) == Some(ch('-'))) {
                    return Err(INVALID_SET.to_owned());
                }
            }
            first
        } else {
            self.class_union(first, first_char)?
        };
        if !self.eat(']') {
            return Err(if self.peek().is_none() {
                UNTERMINATED_CLASS.to_owned()
            } else {
                INVALID_SET.to_owned()
            });
        }
        if negated && strings {
            return Err("Negated character class may contain strings".to_owned());
        }
        Ok(strings)
    }

    /// The rest of a ClassUnion: operands and ranges, up to its `]`.
    fn class_union(&mut self, first: bool, first_char: Option<u32>) -> Result<bool, String> {
        let mut any = first;
        let mut prev = first_char;
        loop {
            match self.peek_char() {
                None | Some(']') => return Ok(any),
                Some('-') => {
                    let Some(a) = prev.take() else {
                        return Err(INVALID_SET.to_owned());
                    };
                    self.i += 1;
                    let (_, b) = self.class_set_operand()?;
                    let Some(b) = b else {
                        return Err(INVALID_SET.to_owned());
                    };
                    if a > b {
                        return Err("Range out of order in character class".to_owned());
                    }
                }
                Some('&') if self.peek_at(1) == Some(ch('&')) => {
                    return Err(INVALID_SET.to_owned());
                }
                Some(_) => {
                    let (s, c) = self.class_set_operand()?;
                    any |= s;
                    prev = c;
                }
            }
        }
    }

    /// One ClassSetOperand: a nested class, an escape, `\q{...}` or a
    /// character. Whether it may contain strings, and its character when
    /// it is one (the end of a range).
    fn class_set_operand(&mut self) -> Result<(bool, Option<u32>), String> {
        let Some(c) = self.peek_char() else {
            return Err(UNTERMINATED_CLASS.to_owned());
        };
        if c == '[' {
            self.i += 1;
            return Ok((self.nested(|c| c.class_set_contents())?, None));
        }
        if c != '\\' {
            return self.class_set_character().map(|c| (false, Some(c)));
        }
        self.i += 1;
        let Some(e) = self.peek_char() else {
            return Err("\\ at end of pattern".to_owned());
        };
        if e == 'q' && self.peek_at(1) == Some(ch('{')) {
            self.i += 2;
            return Ok((self.class_strings()?, None));
        }
        if matches!(e, 'p' | 'P') {
            self.i += 1;
            return Ok((self.property(e == 'P')?, None));
        }
        if RESERVED_PUNCTUATORS.contains(e) {
            self.i += 1;
            return Ok((false, Some(ch(e))));
        }
        match self.character_or_class_escape(true)? {
            ClassAtom::Char(v) => Ok((false, Some(v))),
            ClassAtom::Set => Ok((false, None)),
        }
    }

    /// A ClassSetCharacter that is not an escape: no ClassSetSyntaxCharacter
    /// and no reserved doubled punctuator.
    fn class_set_character(&mut self) -> Result<u32, String> {
        let Some(c) = self.peek() else {
            return Err(UNTERMINATED_CLASS.to_owned());
        };
        let ch_ = char::from_u32(c).unwrap_or('\0');
        if "()[]{}/-\\|".contains(ch_) {
            return Err(INVALID_SET.to_owned());
        }
        if "&!#$%*+,.:;<=>?@^`~".contains(ch_) && self.peek_at(1) == Some(c) {
            return Err(INVALID_SET.to_owned());
        }
        self.i += 1;
        Ok(c)
    }

    /// `\q{...}`'s strings after its `{`, through its `}`: whether any is
    /// not exactly one character.
    fn class_strings(&mut self) -> Result<bool, String> {
        let mut strings = false;
        let mut len = 0usize;
        loop {
            let Some(c) = self.peek_char() else {
                return Err(UNTERMINATED_CLASS.to_owned());
            };
            match c {
                '}' => {
                    self.i += 1;
                    return Ok(strings || len != 1);
                }
                '|' => {
                    self.i += 1;
                    strings |= len != 1;
                    len = 0;
                    continue;
                }
                '\\' => {
                    self.i += 1;
                    if self
                        .peek_char()
                        .is_some_and(|e| RESERVED_PUNCTUATORS.contains(e))
                    {
                        self.i += 1;
                    } else if let ClassAtom::Set = self.character_or_class_escape(true)? {
                        return Err("Invalid escape".to_owned());
                    }
                }
                _ => {
                    self.class_set_character()?;
                }
            }
            len += 1;
        }
    }
}
