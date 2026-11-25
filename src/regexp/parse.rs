//! A pattern parsed to a tree: the early errors of sections 22.2.1 and
//! 13.2.7.2 (with Annex B.1.2's grammar outside Unicode mode) on the way.
//!
//! The parser reads the pattern once to count its capturing groups and
//! find whether any is named (which decides how `\k` and a decimal escape
//! read: section 22.2.1's reparse with [+NamedCaptureGroups]), then once
//! to build the tree. Groups and classes nest by recursion bounded at
//! [`MAX_DEPTH`].

use super::charset::CharSet;
use super::properties;
use crate::stack::StackGuard;
use crate::unicode::{is_id_continue, is_id_start};

/// How deep groups and classes may nest.
const MAX_DEPTH: u32 = 1000;

/// The largest code point.
pub(crate) const MAX_CODE_POINT: u32 = 0x10FFFF;

/// A pattern's tree.
#[derive(Debug, Clone)]
pub(crate) enum Node {
    Empty,
    /// A character (a code point in Unicode mode, else a code unit).
    Char(u32),
    /// `.`.
    Any,
    /// `^`.
    Start,
    /// `$`.
    End,
    /// `\b`, or `\B` when negated.
    WordBoundary {
        negated: bool,
    },
    Class(Box<Class>),
    /// A group; `capture` is its index (from 1) if it captures.
    Group {
        body: Box<Node>,
        capture: Option<u32>,
    },
    /// `(?ims-ims:...)`: the flags it turns on and off (1 `i`, 2 `m`,
    /// 4 `s`).
    Modifiers {
        add: u8,
        remove: u8,
        body: Box<Node>,
    },
    /// A lookahead, or a lookbehind; negated or not.
    Look {
        behind: bool,
        negated: bool,
        body: Box<Node>,
    },
    /// A backreference: the groups it may name (several for a name that
    /// stands twice in alternatives).
    BackRef(Vec<u32>),
    /// `\k<name>` while parsing, made a [`Node::BackRef`] once every
    /// group is known.
    NamedRef(String),
    /// A quantified atom; `groups` the captures it holds, from and to
    /// (exclusive), reset each iteration.
    Repeat {
        body: Box<Node>,
        min: u32,
        max: Option<u32>,
        greedy: bool,
        groups: (u32, u32),
    },
    Seq(Vec<Node>),
    Alt(Vec<Node>),
}

/// A character class: its characters, the strings a `v` class may hold,
/// and whether it is negated (applied at match time, after case
/// folding).
#[derive(Debug, Clone, Default)]
pub(crate) struct Class {
    pub negated: bool,
    pub set: CharSet,
    pub strings: Vec<Vec<u32>>,
    /// A property escape the engine has no table for yet.
    pub unsupported: bool,
    /// Whether a property of strings stands in it (MayContainStrings).
    pub string_property: bool,
}

/// A parsed pattern.
#[derive(Debug, Clone)]
pub(crate) struct Pattern {
    pub root: Node,
    /// The number of capturing groups.
    pub groups: u32,
    /// Each group's name, by index (0 unused).
    pub names: Vec<Option<String>>,
    /// Whether any backreference stands in it.
    pub backrefs: bool,
}

/// A class atom: one character, or a set (`\d`, `\p{...}`).
#[derive(Clone)]
enum ClassAtom {
    Char(u32),
    Set(Class),
}

const INVALID_SET: &str = "Invalid set operation in character class";
const UNTERMINATED_CLASS: &str = "Unterminated character class";
/// ClassSetReservedPunctuator: what `v` mode lets be escaped in a class.
const RESERVED_PUNCTUATORS: &str = "&-!#%,:;<=>@`~";

/// Parses a pattern: `unicode` for the `u` or `v` flag, `sets` for `v`,
/// `ignore_case` for `i` (which, with `u` or `v`, `\w` reads).
pub(crate) fn parse(
    pattern: &str,
    unicode: bool,
    sets: bool,
    ignore_case: bool,
    stack: StackGuard,
) -> Result<Pattern, String> {
    // In Unicode mode the pattern is code points; otherwise UTF-16 code
    // units, so that a surrogate pair is two characters (a class range
    // across one is out of order).
    let units: Vec<u32> = if unicode {
        pattern.chars().map(u32::from).collect()
    } else {
        pattern.encode_utf16().map(u32::from).collect()
    };
    parse_units(units, unicode, sets, ignore_case, stack)
}

/// Parses a pattern given as UTF-16 code units (which may hold lone
/// surrogates, as a RegExp's source string may).
pub(crate) fn parse_utf16(
    pattern: &[u16],
    unicode: bool,
    sets: bool,
    ignore_case: bool,
    stack: StackGuard,
) -> Result<Pattern, String> {
    let units: Vec<u32> = if unicode {
        char::decode_utf16(pattern.iter().copied())
            .map(|r| match r {
                Ok(c) => u32::from(c),
                Err(e) => u32::from(e.unpaired_surrogate()),
            })
            .collect()
    } else {
        pattern.iter().map(|&u| u32::from(u)).collect()
    };
    parse_units(units, unicode, sets, ignore_case, stack)
}

fn parse_units(
    units: Vec<u32>,
    unicode: bool,
    sets: bool,
    ignore_case: bool,
    stack: StackGuard,
) -> Result<Pattern, String> {
    let (groups, named) = prescan(&units, sets);
    let mut p = Parser {
        s: units,
        i: 0,
        unicode,
        sets,
        named: unicode || named,
        groups,
        next_group: 1,
        names: Vec::new(),
        group_names: vec![None; groups as usize + 1],
        references: Vec::new(),
        path: Vec::new(),
        next_disjunction: 0,
        depth: 0,
        stack,
        backrefs: false,
        fold: ignore_case,
    };
    let mut root = p.disjunction()?;
    if p.i < p.s.len() {
        return Err("Unmatched ')'".to_owned());
    }
    for name in &p.references {
        if !p.names.iter().any(|(n, _)| n == name) {
            return Err("Invalid named capture referenced".to_owned());
        }
    }
    resolve_names(&mut root, &p.group_names);
    Ok(Pattern {
        root,
        groups,
        names: p.group_names,
        backrefs: p.backrefs,
    })
}

/// Named backreferences made the indices of their groups.
fn resolve_names(node: &mut Node, names: &[Option<String>]) {
    match node {
        Node::NamedRef(name) => {
            let indices = names
                .iter()
                .enumerate()
                .filter(|(_, n)| n.as_deref() == Some(name.as_str()))
                .filter_map(|(k, _)| u32::try_from(k).ok())
                .collect();
            *node = Node::BackRef(indices);
        }
        Node::Group { body, .. }
        | Node::Modifiers { body, .. }
        | Node::Look { body, .. }
        | Node::Repeat { body, .. } => resolve_names(body, names),
        Node::Seq(v) | Node::Alt(v) => {
            for n in v {
                resolve_names(n, names);
            }
        }
        _ => {}
    }
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

struct Parser {
    s: Vec<u32>,
    i: usize,
    unicode: bool,
    sets: bool,
    /// [+NamedCaptureGroups]: Unicode mode, or a pattern with a named
    /// group.
    named: bool,
    groups: u32,
    next_group: u32,
    /// Each named group and the alternatives it is in, to allow a name
    /// twice only in alternatives that cannot both match (section
    /// 22.2.1.1's MightBothParticipate).
    names: Vec<(String, Vec<(u32, u32)>)>,
    group_names: Vec<Option<String>>,
    /// Named references, resolved once every group is known.
    references: Vec<String>,
    /// The (disjunction, alternative) pairs around the current point.
    path: Vec<(u32, u32)>,
    next_disjunction: u32,
    depth: u32,
    stack: StackGuard,
    backrefs: bool,
    /// The `i` in force, which modifiers change.
    fold: bool,
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

/// The characters `\d`, `\s` and `\w` stand for (section 22.2.2.9's
/// CharacterClassEscape): `\w`'s WordCharacters, with `u` or `v` and `i`
/// (`fold_words`), take the long s and the Kelvin sign too.
pub(crate) fn class_escape(c: char, fold_words: bool) -> CharSet {
    let base = match c.to_ascii_lowercase() {
        'd' => CharSet::of(&[(0x30, 0x39)]),
        'w' if fold_words => CharSet::of(&[
            (0x30, 0x39),
            (0x41, 0x5A),
            (0x5F, 0x5F),
            (0x61, 0x7A),
            (0x17F, 0x17F),
            (0x212A, 0x212A),
        ]),
        'w' => CharSet::of(&[(0x30, 0x39), (0x41, 0x5A), (0x5F, 0x5F), (0x61, 0x7A)]),
        _ => CharSet::of(&[
            (0x09, 0x0D),
            (0x20, 0x20),
            (0xA0, 0xA0),
            (0x1680, 0x1680),
            (0x2000, 0x200A),
            (0x2028, 0x2029),
            (0x202F, 0x202F),
            (0x205F, 0x205F),
            (0x3000, 0x3000),
            (0xFEFF, 0xFEFF),
        ]),
    };
    if c.is_ascii_uppercase() {
        base.complement(MAX_CODE_POINT)
    } else {
        base
    }
}

impl Parser {
    fn peek(&self) -> Option<u32> {
        self.s.get(self.i).copied()
    }

    /// The current character; outside Unicode mode a surrogate code unit
    /// is no `char` and reads as U+FFFD, an ordinary character (its own
    /// value is read with [`Parser::peek`] where it matters).
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

    fn disjunction(&mut self) -> Result<Node, String> {
        let id = self.next_disjunction;
        self.next_disjunction += 1;
        self.path.push((id, 0));
        let result = self.disjunction_parts();
        self.path.pop();
        result
    }

    fn disjunction_parts(&mut self) -> Result<Node, String> {
        let mut alts = vec![self.alternative()?];
        let mut alt = 0;
        while self.eat('|') {
            alt += 1;
            if let Some(top) = self.path.last_mut() {
                top.1 = alt;
            }
            alts.push(self.alternative()?);
        }
        Ok(if alts.len() == 1 {
            alts.pop().unwrap_or(Node::Empty)
        } else {
            Node::Alt(alts)
        })
    }

    fn alternative(&mut self) -> Result<Node, String> {
        let mut terms = Vec::new();
        while let Some(c) = self.peek() {
            if c == ch('|') || c == ch(')') {
                break;
            }
            terms.push(self.term()?);
        }
        Ok(match terms.len() {
            0 => Node::Empty,
            1 => terms.pop().unwrap_or(Node::Empty),
            _ => Node::Seq(terms),
        })
    }

    fn term(&mut self) -> Result<Node, String> {
        let Some(c) = self.peek_char() else {
            return Ok(Node::Empty);
        };
        let first_group = self.next_group;
        let (atom, quantifiable) = match c {
            '^' => {
                self.i += 1;
                (Node::Start, false)
            }
            '$' => {
                self.i += 1;
                (Node::End, false)
            }
            '\\' if matches!(self.peek_at(1).and_then(char::from_u32), Some('b' | 'B')) => {
                let negated = self.peek_at(1) == Some(ch('B'));
                self.i += 2;
                (Node::WordBoundary { negated }, false)
            }
            '(' => self.group()?,
            '[' => {
                self.i += 1;
                (Node::Class(Box::new(self.class()?)), true)
            }
            '.' => {
                self.i += 1;
                (Node::Any, true)
            }
            '\\' => {
                self.i += 1;
                (self.atom_escape()?, true)
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
                (Node::Char(ch('{')), true)
            }
            '}' | ']' if self.unicode => return Err("Lone quantifier brackets".to_owned()),
            _ => {
                let c = self.peek().unwrap_or(0);
                self.i += 1;
                (Node::Char(c), true)
            }
        };
        match self.quantifier()? {
            Some(_) if !quantifiable => Err("Nothing to repeat".to_owned()),
            Some((min, max, greedy)) => Ok(Node::Repeat {
                body: Box::new(atom),
                min,
                max,
                greedy,
                groups: (first_group, self.next_group),
            }),
            None => Ok(atom),
        }
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

    /// A quantifier after an atom, if one is there: its bounds (clamped
    /// to u32, as large counts behave alike) and whether it is greedy.
    fn quantifier(&mut self) -> Result<Option<(u32, Option<u32>, bool)>, String> {
        let clamp = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
        let (min, max) = match self.peek_char() {
            Some('*') => {
                self.i += 1;
                (0, None)
            }
            Some('+') => {
                self.i += 1;
                (1, None)
            }
            Some('?') => {
                self.i += 1;
                (0, Some(1))
            }
            Some('{') => match self.quantifier_at(self.i) {
                Some((len, min, max)) => {
                    if max.is_some_and(|max| max < min) {
                        return Err("numbers out of order in {} quantifier".to_owned());
                    }
                    self.i += len;
                    (clamp(min), max.map(clamp))
                }
                None if self.unicode => return Err("Incomplete quantifier".to_owned()),
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        let greedy = !self.eat('?');
        Ok(Some((min, max, greedy)))
    }

    /// A group of any kind, from its `(`; whether a quantifier may follow.
    fn group(&mut self) -> Result<(Node, bool), String> {
        self.i += 1;
        enum Kind {
            Look(bool, bool),
            Plain,
            Capture(Option<String>),
            Modifiers(u8, u8),
        }
        let kind = if self.eat_str("?=") {
            Kind::Look(false, false)
        } else if self.eat_str("?!") {
            Kind::Look(false, true)
        } else if self.eat_str("?<=") {
            Kind::Look(true, false)
        } else if self.eat_str("?<!") {
            Kind::Look(true, true)
        } else if self.eat_str("?:") {
            Kind::Plain
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
            self.names.push((name.clone(), self.path.clone()));
            Kind::Capture(Some(name))
        } else if self.eat('?') {
            let (add, remove) = self.modifiers()?;
            Kind::Modifiers(add, remove)
        } else {
            Kind::Capture(None)
        };
        let capture = match &kind {
            Kind::Capture(name) => {
                let index = self.next_group;
                self.next_group += 1;
                if let Some(slot) = self.group_names.get_mut(index as usize) {
                    *slot = name.clone();
                }
                Some(index)
            }
            _ => None,
        };
        let outer = self.fold;
        if let Kind::Modifiers(add, remove) = &kind {
            if add & 1 != 0 {
                self.fold = true;
            }
            if remove & 1 != 0 {
                self.fold = false;
            }
        }
        let body = self.nested(|c| c.disjunction());
        self.fold = outer;
        let body = Box::new(body?);
        if !self.eat(')') {
            return Err("Unterminated group".to_owned());
        }
        Ok(match kind {
            // Annex B lets a lookahead be quantified outside Unicode mode.
            Kind::Look(behind, negated) => (
                Node::Look {
                    behind,
                    negated,
                    body,
                },
                !behind && !self.unicode,
            ),
            Kind::Plain => (
                Node::Group {
                    body,
                    capture: None,
                },
                true,
            ),
            Kind::Capture(_) => (Node::Group { body, capture }, true),
            Kind::Modifiers(add, remove) => (Node::Modifiers { add, remove, body }, true),
        })
    }

    /// `ims-ims:` after a group's `(?`: no flag twice, at least one; the
    /// flags turned on and off.
    fn modifiers(&mut self) -> Result<(u8, u8), String> {
        let mut seen = 0u8;
        let (mut add, mut remove) = (0u8, 0u8);
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
                ':' if any => return Ok((add, remove)),
                ':' if dash => return Err("Invalid flags in modifiers".to_owned()),
                _ => return Err("Invalid group".to_owned()),
            };
            if seen & bit != 0 {
                return Err("Repeated flag in modifiers".to_owned());
            }
            seen |= bit;
            if dash {
                remove |= bit;
            } else {
                add |= bit;
            }
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
            return (self.eat('}') && v <= MAX_CODE_POINT).then_some(v);
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
    fn atom_escape(&mut self) -> Result<Node, String> {
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
                    self.backrefs = true;
                    return Ok(Node::BackRef(vec![u32::try_from(n).unwrap_or(0)]));
                }
                if self.unicode {
                    return Err("Invalid escape".to_owned());
                }
                // Annex B: a legacy octal escape, or `\8` / `\9` as
                // themselves.
                self.i = start;
                Ok(Node::Char(self.legacy_octal()))
            }
            Some('k') if self.named => {
                self.i += 1;
                if !self.eat('<') {
                    return Err("Invalid named reference".to_owned());
                }
                let name = self
                    .group_name()
                    .map_err(|_| "Invalid named reference".to_owned())?;
                self.references.push(name.clone());
                self.backrefs = true;
                Ok(Node::NamedRef(name))
            }
            Some(_) => Ok(match self.character_or_class_escape(false)? {
                ClassAtom::Char(c) => Node::Char(c),
                ClassAtom::Set(class) => Node::Class(Box::new(class)),
            }),
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
            'd' | 'D' | 's' | 'S' | 'w' | 'W' => {
                return Ok(ClassAtom::Set(Class {
                    set: class_escape(c, self.unicode && self.fold),
                    ..Class::default()
                }));
            }
            'p' | 'P' if self.unicode => {
                self.property(c == 'P')?;
                return Ok(ClassAtom::Set(Class {
                    unsupported: true,
                    ..Class::default()
                }));
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
                        Some(v) if v <= MAX_CODE_POINT && self.eat('}') => v,
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
    fn class(&mut self) -> Result<Class, String> {
        if self.sets {
            return self.nested(|c| c.class_set_contents());
        }
        let mut out = Class {
            negated: self.eat('^'),
            ..Class::default()
        };
        loop {
            let Some(c) = self.peek() else {
                return Err(UNTERMINATED_CLASS.to_owned());
            };
            if c == ch(']') {
                self.i += 1;
                return Ok(out);
            }
            let first = self.class_atom()?;
            if self.at('-') && self.peek_at(1).is_some_and(|c| c != ch(']')) {
                self.i += 1;
                let second = self.class_atom()?;
                match (first, second) {
                    (ClassAtom::Char(a), ClassAtom::Char(b)) if a > b => {
                        return Err("Range out of order in character class".to_owned());
                    }
                    (ClassAtom::Char(a), ClassAtom::Char(b)) => out.set.add_range(a, b),
                    // Annex B: a set beside `-` outside Unicode mode makes
                    // the `-` a character.
                    _ if self.unicode => return Err("Invalid character class".to_owned()),
                    (a, b) => {
                        add_atom(&mut out, a);
                        out.set.add(ch('-'));
                        add_atom(&mut out, b);
                    }
                }
            } else {
                add_atom(&mut out, first);
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

    /// A `v`-mode class's contents after its `[`, through its `]`.
    fn class_set_contents(&mut self) -> Result<Class, String> {
        let negated = self.eat('^');
        if self.eat(']') {
            return Ok(Class {
                negated,
                ..Class::default()
            });
        }
        let (first, first_char) = self.class_set_operand()?;
        let mut value = if self.at('&') && self.peek_at(1) == Some(ch('&')) {
            // ClassIntersection: operands joined by `&&`.
            let mut all = first;
            while self.eat_str("&&") {
                if self.at('&') {
                    return Err(INVALID_SET.to_owned());
                }
                let (s, _) = self.class_set_operand()?;
                all = intersect(&all, &s);
                if !self.at(']') && !(self.at('&') && self.peek_at(1) == Some(ch('&'))) {
                    return Err(INVALID_SET.to_owned());
                }
            }
            all
        } else if self.at('-') && self.peek_at(1) == Some(ch('-')) {
            // ClassSubtraction: operands joined by `--`.
            let mut all = first;
            while self.eat_str("--") {
                let (s, _) = self.class_set_operand()?;
                all = subtract(&all, &s);
                if !self.at(']') && !(self.at('-') && self.peek_at(1) == Some(ch('-'))) {
                    return Err(INVALID_SET.to_owned());
                }
            }
            all
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
        let strings = value.string_property || value.strings.iter().any(|s| s.len() != 1);
        if negated && strings {
            return Err("Negated character class may contain strings".to_owned());
        }
        value.negated = negated;
        Ok(value)
    }

    /// The rest of a ClassUnion: operands and ranges, up to its `]`.
    fn class_union(&mut self, first: Class, first_char: Option<u32>) -> Result<Class, String> {
        let mut all = first;
        let mut prev = first_char;
        loop {
            match self.peek_char() {
                None | Some(']') => return Ok(all),
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
                    all.set.add_range(a, b);
                }
                Some('&') if self.peek_at(1) == Some(ch('&')) => {
                    return Err(INVALID_SET.to_owned());
                }
                Some(_) => {
                    let (s, c) = self.class_set_operand()?;
                    union(&mut all, s);
                    prev = c;
                }
            }
        }
    }

    /// One ClassSetOperand: a nested class, an escape, `\q{...}` or a
    /// character; its character when it is one (the end of a range).
    fn class_set_operand(&mut self) -> Result<(Class, Option<u32>), String> {
        let Some(c) = self.peek_char() else {
            return Err(UNTERMINATED_CLASS.to_owned());
        };
        if c == '[' {
            self.i += 1;
            let inner = self.nested(|c| c.class_set_contents())?;
            return Ok((resolve_negation(inner), None));
        }
        if c != '\\' {
            return self.class_set_character().map(|c| (single(c), Some(c)));
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
            let strings = self.property(e == 'P')?;
            return Ok((
                Class {
                    unsupported: true,
                    string_property: strings,
                    ..Class::default()
                },
                None,
            ));
        }
        if RESERVED_PUNCTUATORS.contains(e) {
            self.i += 1;
            return Ok((single(ch(e)), Some(ch(e))));
        }
        match self.character_or_class_escape(true)? {
            ClassAtom::Char(v) => Ok((single(v), Some(v))),
            ClassAtom::Set(s) => Ok((s, None)),
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

    /// `\q{...}`'s strings after its `{`, through its `}`.
    fn class_strings(&mut self) -> Result<Class, String> {
        let mut out = Class::default();
        let mut current: Vec<u32> = Vec::new();
        loop {
            let Some(c) = self.peek_char() else {
                return Err(UNTERMINATED_CLASS.to_owned());
            };
            match c {
                '}' | '|' => {
                    self.i += 1;
                    let s = std::mem::take(&mut current);
                    match s.as_slice() {
                        [one] => out.set.add(*one),
                        _ => out.strings.push(s),
                    }
                    if c == '}' {
                        return Ok(out);
                    }
                    continue;
                }
                '\\' => {
                    self.i += 1;
                    if let Some(e) = self
                        .peek_char()
                        .filter(|e| RESERVED_PUNCTUATORS.contains(*e))
                    {
                        self.i += 1;
                        current.push(ch(e));
                    } else {
                        match self.character_or_class_escape(true)? {
                            ClassAtom::Char(v) => current.push(v),
                            ClassAtom::Set(_) => return Err("Invalid escape".to_owned()),
                        }
                    }
                }
                _ => {
                    let v = self.class_set_character()?;
                    current.push(v);
                }
            }
        }
    }
}

fn single(c: u32) -> Class {
    let mut out = Class::default();
    out.set.add(c);
    out
}

fn add_atom(out: &mut Class, a: ClassAtom) {
    match a {
        ClassAtom::Char(c) => out.set.add(c),
        ClassAtom::Set(s) => union(out, s),
    }
}

/// A nested negated class made its complement (a `v` class may nest).
fn resolve_negation(c: Class) -> Class {
    if !c.negated {
        return c;
    }
    Class {
        negated: false,
        set: c.set.complement(MAX_CODE_POINT),
        strings: Vec::new(),
        unsupported: c.unsupported,
        string_property: false,
    }
}

fn union(a: &mut Class, b: Class) {
    let b = resolve_negation(b);
    a.set.union(&b.set);
    for s in b.strings {
        if !a.strings.contains(&s) {
            a.strings.push(s);
        }
    }
    a.unsupported |= b.unsupported;
    a.string_property |= b.string_property;
}

fn intersect(a: &Class, b: &Class) -> Class {
    let b = resolve_negation(b.clone());
    Class {
        negated: false,
        set: a.set.intersect(&b.set),
        strings: a
            .strings
            .iter()
            .filter(|s| b.strings.contains(s))
            .cloned()
            .collect(),
        unsupported: a.unsupported || b.unsupported,
        string_property: a.string_property && b.string_property,
    }
}

fn subtract(a: &Class, b: &Class) -> Class {
    let b = resolve_negation(b.clone());
    Class {
        negated: false,
        set: a.set.subtract(&b.set),
        strings: a
            .strings
            .iter()
            .filter(|s| !b.strings.contains(s))
            .cloned()
            .collect(),
        unsupported: a.unsupported || b.unsupported,
        string_property: a.string_property,
    }
}
