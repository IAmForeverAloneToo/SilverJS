//! The regexp engine (design section 1.6): a parsed pattern compiled to
//! instructions, and a backtracking VM that runs them over UTF-16 text.
//!
//! The backtrack stack is a heap vector: a choice point is a frame to
//! go back to, and every write of a capture or a register is undone
//! through a frame too, so that going back restores the state exactly.
//! A lookaround runs as a sub-run of its own, atomic as the spec has it;
//! a lookbehind's body is compiled to read leftwards. A counted
//! repetition keeps its count and where its iteration began in
//! registers, for the spec's empty check (an iteration past the minimum
//! that matched nothing fails). Every step is charged to the agent's
//! budget through `tick`, and a pattern without backreferences that has
//! backtracked [`MEMO_AFTER`] times stops retrying the states it has
//! seen fail (selective memoization), which bounds it by the pattern's
//! size times the input's.

use std::collections::HashSet;

/// The states seen at choice points once memoization has started: an
/// instruction, a position and the loop registers that still matter
/// there (see [`Run::seen`]).
type Memo = Option<HashSet<(u32, usize, Box<[i64]>)>>;

use super::casefold::{any_equivalent, canonicalize};
use super::charset::CharSet;
use super::parse::{Class, Node, Pattern};

/// Backtracks before memoization starts.
pub(crate) const MEMO_AFTER: u64 = 100_000;

/// A regexp's flags as the engine reads them.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Flags {
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    /// `u` or `v`: code points, and case folding by the UCD.
    pub unicode: bool,
}

#[derive(Debug, Clone)]
enum Inst {
    Char {
        c: u32,
        fold: bool,
        back: bool,
    },
    Any {
        dot_all: bool,
        back: bool,
    },
    Class {
        index: u32,
        fold: bool,
        negated: bool,
        back: bool,
    },
    Start {
        multiline: bool,
    },
    End {
        multiline: bool,
    },
    Boundary {
        negated: bool,
        fold: bool,
    },
    Save {
        slot: u32,
    },
    ClearCaptures {
        from: u32,
        to: u32,
    },
    Split {
        first: u32,
        second: u32,
    },
    Jmp {
        to: u32,
    },
    BackRef {
        groups: Box<[u32]>,
        fold: bool,
        back: bool,
    },
    /// A lookaround: its body follows, ending in [`Inst::LookEnd`]; on at
    /// `end`.
    Look {
        negated: bool,
        end: u32,
    },
    LookEnd,
    RepeatStart {
        reg: u32,
    },
    RepeatTest {
        reg: u32,
        min: u32,
        max: u32,
        greedy: bool,
        exit: u32,
    },
    RepeatMark {
        reg: u32,
    },
    RepeatNext {
        reg: u32,
        min: u32,
        test: u32,
    },
    Match,
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub(crate) struct Program {
    insts: Vec<Inst>,
    classes: Vec<CharSet>,
    /// The number of capturing groups.
    pub groups: u32,
    /// Each group's name, by index (0 unused).
    pub names: Vec<Option<String>>,
    regs: u32,
    loops: Vec<Loop>,
    backrefs: bool,
    unicode: bool,
}

/// A counted repetition: its [`Inst::RepeatTest`], the instruction after
/// it, its registers (count, then where the iteration began) and bounds.
#[derive(Debug, Clone, Copy)]
struct Loop {
    test: u32,
    exit: u32,
    reg: u32,
    min: u32,
    max: u32,
}

/// Why a run stopped before an answer: the budget's interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stopped;

/// The modifiers in force while compiling.
#[derive(Clone, Copy)]
struct Mode {
    fold: bool,
    multiline: bool,
    dot_all: bool,
    back: bool,
}

struct Compiler {
    insts: Vec<Inst>,
    classes: Vec<CharSet>,
    regs: u32,
    loops: Vec<Loop>,
    unicode: bool,
}

/// Compiles a parsed pattern; an error for what the engine cannot match
/// yet (a property escape before its tables).
pub(crate) fn compile(p: &Pattern, flags: Flags) -> Result<Program, String> {
    let mut c = Compiler {
        insts: Vec::new(),
        classes: Vec::new(),
        regs: 0,
        loops: Vec::new(),
        unicode: flags.unicode,
    };
    let mode = Mode {
        fold: flags.ignore_case,
        multiline: flags.multiline,
        dot_all: flags.dot_all,
        back: false,
    };
    c.emit(Inst::Save { slot: 0 });
    c.node(&p.root, mode)?;
    c.emit(Inst::Save { slot: 1 });
    c.emit(Inst::Match);
    Ok(Program {
        insts: c.insts,
        classes: c.classes,
        groups: p.groups,
        names: p.names.clone(),
        regs: c.regs,
        loops: c.loops,
        backrefs: p.backrefs,
        unicode: flags.unicode,
    })
}

impl Compiler {
    fn pc(&self) -> u32 {
        self.insts.len() as u32
    }

    fn emit(&mut self, i: Inst) -> u32 {
        self.insts.push(i);
        self.pc() - 1
    }

    fn patch(&mut self, at: u32, f: impl FnOnce(&mut Inst)) {
        if let Some(i) = self.insts.get_mut(at as usize) {
            f(i);
        }
    }

    fn char(&mut self, ch: u32, mode: Mode) {
        let c = if mode.fold {
            canonicalize(ch, self.unicode)
        } else {
            ch
        };
        self.emit(Inst::Char {
            c,
            fold: mode.fold,
            back: mode.back,
        });
    }

    /// A sequence, read leftwards in a lookbehind.
    fn seq(&mut self, nodes: &[Node], mode: Mode) -> Result<(), String> {
        if mode.back {
            for n in nodes.iter().rev() {
                self.node(n, mode)?;
            }
        } else {
            for n in nodes {
                self.node(n, mode)?;
            }
        }
        Ok(())
    }

    /// Alternatives in order: each a split to the next.
    fn alt(&mut self, alts: &[Node], mode: Mode) -> Result<(), String> {
        let mut jumps = Vec::new();
        let count = alts.len();
        for (k, a) in alts.iter().enumerate() {
            if k + 1 < count {
                let split = self.emit(Inst::Split {
                    first: 0,
                    second: 0,
                });
                self.node(a, mode)?;
                jumps.push(self.emit(Inst::Jmp { to: 0 }));
                let next = self.pc();
                self.patch(split, |i| {
                    *i = Inst::Split {
                        first: split + 1,
                        second: next,
                    }
                });
            } else {
                self.node(a, mode)?;
            }
        }
        let end = self.pc();
        for j in jumps {
            self.patch(j, |i| *i = Inst::Jmp { to: end });
        }
        Ok(())
    }

    fn class(&mut self, class: &Class, mode: Mode) -> Result<(), String> {
        if class.unsupported {
            return Err("Unicode property escapes are not supported yet".to_owned());
        }
        let index = self.classes.len() as u32;
        self.classes.push(class.set.clone());
        if class.strings.is_empty() {
            self.emit(Inst::Class {
                index,
                fold: mode.fold,
                negated: class.negated,
                back: mode.back,
            });
            return Ok(());
        }
        // A `v` class with strings: the strings, longest first, then its
        // single characters.
        let mut strings = class.strings.clone();
        strings.sort_by_key(|s| std::cmp::Reverse(s.len()));
        let mut alts: Vec<Node> = strings
            .into_iter()
            .map(|s| Node::Seq(s.into_iter().map(Node::Char).collect()))
            .collect();
        let single = Class {
            strings: Vec::new(),
            ..class.clone()
        };
        if !single.set.is_empty() {
            alts.push(Node::Class(Box::new(single)));
        }
        self.alt(&alts, mode)
    }

    fn node(&mut self, n: &Node, mode: Mode) -> Result<(), String> {
        match n {
            Node::Empty => {}
            Node::Char(c) => self.char(*c, mode),
            Node::Any => {
                self.emit(Inst::Any {
                    dot_all: mode.dot_all,
                    back: mode.back,
                });
            }
            Node::Start => {
                self.emit(Inst::Start {
                    multiline: mode.multiline,
                });
            }
            Node::End => {
                self.emit(Inst::End {
                    multiline: mode.multiline,
                });
            }
            Node::WordBoundary { negated } => {
                self.emit(Inst::Boundary {
                    negated: *negated,
                    fold: mode.fold,
                });
            }
            Node::Class(c) => self.class(c, mode)?,
            Node::Group { body, capture } => match capture {
                Some(g) => {
                    // Leftwards, the end is reached first.
                    let (first, second) = if mode.back {
                        (g * 2 + 1, g * 2)
                    } else {
                        (g * 2, g * 2 + 1)
                    };
                    self.emit(Inst::Save { slot: first });
                    self.node(body, mode)?;
                    self.emit(Inst::Save { slot: second });
                }
                None => self.node(body, mode)?,
            },
            Node::Modifiers { add, remove, body } => {
                let mut m = mode;
                if add & 1 != 0 {
                    m.fold = true;
                }
                if remove & 1 != 0 {
                    m.fold = false;
                }
                if add & 2 != 0 {
                    m.multiline = true;
                }
                if remove & 2 != 0 {
                    m.multiline = false;
                }
                if add & 4 != 0 {
                    m.dot_all = true;
                }
                if remove & 4 != 0 {
                    m.dot_all = false;
                }
                self.node(body, m)?;
            }
            Node::Look {
                behind,
                negated,
                body,
            } => {
                let at = self.emit(Inst::Look {
                    negated: *negated,
                    end: 0,
                });
                let m = Mode {
                    back: *behind,
                    ..mode
                };
                self.node(body, m)?;
                self.emit(Inst::LookEnd);
                let end = self.pc();
                self.patch(at, |i| {
                    *i = Inst::Look {
                        negated: *negated,
                        end,
                    }
                });
            }
            Node::BackRef(groups) => {
                self.emit(Inst::BackRef {
                    groups: groups.clone().into_boxed_slice(),
                    fold: mode.fold,
                    back: mode.back,
                });
            }
            Node::NamedRef(_) => {}
            Node::Repeat {
                body,
                min,
                max,
                greedy,
                groups,
            } => {
                if *max == Some(0) {
                    return Ok(());
                }
                let reg = self.regs;
                self.regs += 2;
                self.emit(Inst::RepeatStart { reg });
                let test = self.emit(Inst::RepeatTest {
                    reg,
                    min: *min,
                    max: max.unwrap_or(u32::MAX),
                    greedy: *greedy,
                    exit: 0,
                });
                self.emit(Inst::RepeatMark { reg });
                if groups.1 > groups.0 {
                    self.emit(Inst::ClearCaptures {
                        from: groups.0,
                        to: groups.1,
                    });
                }
                self.node(body, mode)?;
                self.emit(Inst::RepeatNext {
                    reg,
                    min: *min,
                    test,
                });
                let exit = self.pc();
                let (min, max, greedy) = (*min, max.unwrap_or(u32::MAX), *greedy);
                self.loops.push(Loop {
                    test,
                    exit,
                    reg,
                    min,
                    max,
                });
                self.patch(test, |i| {
                    *i = Inst::RepeatTest {
                        reg,
                        min,
                        max,
                        greedy,
                        exit,
                    }
                });
            }
            Node::Seq(v) => self.seq(v, mode)?,
            Node::Alt(v) => self.alt(v, mode)?,
        }
        Ok(())
    }
}

/// A frame of the backtrack stack.
enum Frame {
    /// A choice point: go on at `pc` from `pos`.
    Retry { pc: u32, pos: usize },
    /// A capture slot's earlier value.
    Cap { slot: u32, old: i64 },
    /// A register's earlier value.
    Reg { index: u32, old: i64 },
}

fn is_line_terminator(c: u32) -> bool {
    matches!(c, 0x0A | 0x0D | 0x2028 | 0x2029)
}

fn is_basic_word(c: u32) -> bool {
    matches!(c, 0x30..=0x39 | 0x41..=0x5A | 0x5F | 0x61..=0x7A)
}

/// A run's state: the text, the captures, the registers, and the budget.
struct Run<'a, 't> {
    p: &'a Program,
    text: &'a [u16],
    caps: Vec<i64>,
    regs: Vec<i64>,
    stack: Vec<Frame>,
    tick: &'t mut dyn FnMut() -> bool,
    backtracks: u64,
}

impl Program {
    /// The capture slots after a match at `start` (`sticky`: only there),
    /// or from the first index at or past it where one is found: two per
    /// group, group 0 the whole match, -1 for one that did not take
    /// part. `None` when there is no match.
    pub(crate) fn exec(
        &self,
        text: &[u16],
        start: usize,
        sticky: bool,
        tick: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Vec<i64>>, Stopped> {
        let slots = (self.groups as usize + 1) * 2;
        let mut run = Run {
            p: self,
            text,
            caps: vec![-1; slots],
            regs: vec![-1; self.regs as usize],
            stack: Vec::new(),
            tick,
            backtracks: 0,
        };
        // One memo for every start: without backreferences whether a
        // state can still reach a match does not depend on where the
        // attempt began, so a state that failed from one start fails
        // from all (which keeps `/a*b/` on a long run of `a`s linear).
        let mut memo: Memo = None;
        let mut at = start;
        while at <= text.len() {
            run.caps.iter_mut().for_each(|c| *c = -1);
            if run.matches(0, at, &mut memo)?.is_some() {
                return Ok(Some(run.caps));
            }
            if sticky {
                break;
            }
            at = self.advance(text, at);
        }
        Ok(None)
    }

    /// AdvanceStringIndex: past a surrogate pair in Unicode mode.
    pub(crate) fn advance(&self, text: &[u16], at: usize) -> usize {
        if self.unicode
            && text.get(at).is_some_and(|&u| (0xD800..0xDC00).contains(&u))
            && text
                .get(at + 1)
                .is_some_and(|&u| (0xDC00..0xE000).contains(&u))
        {
            at + 2
        } else {
            at + 1
        }
    }
}

impl Run<'_, '_> {
    /// The character after `pos` (or before it, `back`) and its length.
    fn read(&self, pos: usize, back: bool) -> Option<(u32, usize)> {
        let t = self.text;
        if back {
            let lo = u32::from(*t.get(pos.checked_sub(1)?)?);
            if self.p.unicode
                && (0xDC00..0xE000).contains(&lo)
                && let Some(&hi) = pos.checked_sub(2).and_then(|i| t.get(i))
                && (0xD800..0xDC00).contains(&hi)
            {
                return Some((
                    0x10000 + ((u32::from(hi) - 0xD800) << 10) + (lo - 0xDC00),
                    2,
                ));
            }
            return Some((lo, 1));
        }
        let hi = u32::from(*t.get(pos)?);
        if self.p.unicode
            && (0xD800..0xDC00).contains(&hi)
            && let Some(&lo) = t.get(pos + 1)
            && (0xDC00..0xE000).contains(&lo)
        {
            return Some((
                0x10000 + ((hi - 0xD800) << 10) + (u32::from(lo) - 0xDC00),
                2,
            ));
        }
        Some((hi, 1))
    }

    fn set_cap(&mut self, slot: u32, v: i64) {
        if let Some(c) = self.caps.get_mut(slot as usize) {
            let old = *c;
            *c = v;
            self.stack.push(Frame::Cap { slot, old });
        }
    }

    fn set_reg(&mut self, index: u32, v: i64) {
        if let Some(r) = self.regs.get_mut(index as usize) {
            let old = *r;
            *r = v;
            self.stack.push(Frame::Reg { index, old });
        }
    }

    fn reg(&self, index: u32) -> i64 {
        self.regs.get(index as usize).copied().unwrap_or(-1)
    }

    fn is_word(&self, c: Option<(u32, usize)>, fold: bool) -> bool {
        c.is_some_and(|(c, _)| {
            is_basic_word(c) || (fold && self.p.unicode && is_basic_word(canonicalize(c, true)))
        })
    }

    /// Runs from `pc` at `pos` until [`Inst::Match`] (or, for a
    /// lookaround's body, its [`Inst::LookEnd`]): the end position, or
    /// `None` when every way fails. The stack below `base` is the
    /// caller's.
    fn matches(&mut self, pc: u32, pos: usize, memo: &mut Memo) -> Result<Option<usize>, Stopped> {
        let base = self.stack.len();
        let (mut pc, mut pos) = (pc, pos);
        loop {
            if (self.tick)() {
                return Err(Stopped);
            }
            let Some(inst) = self.p.insts.get(pc as usize) else {
                return Ok(None);
            };
            let ok = match inst {
                Inst::Match | Inst::LookEnd => {
                    self.stack.truncate(base);
                    return Ok(Some(pos));
                }
                Inst::Char { c, fold, back } => match self.read(pos, *back) {
                    Some((x, n)) if x == *c || (*fold && canonicalize(x, self.p.unicode) == *c) => {
                        pos = if *back { pos - n } else { pos + n };
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::Any { dot_all, back } => match self.read(pos, *back) {
                    Some((x, n)) if *dot_all || !is_line_terminator(x) => {
                        pos = if *back { pos - n } else { pos + n };
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::Class {
                    index,
                    fold,
                    negated,
                    back,
                } => match self.read(pos, *back) {
                    Some((x, n)) => {
                        let set = self.p.classes.get(*index as usize);
                        let found = set.is_some_and(|s| {
                            s.contains(x)
                                || (*fold && any_equivalent(x, self.p.unicode, |e| s.contains(e)))
                        });
                        if found != *negated {
                            pos = if *back { pos - n } else { pos + n };
                            pc += 1;
                            true
                        } else {
                            false
                        }
                    }
                    None => false,
                },
                Inst::Start { multiline } => {
                    let ok = pos == 0
                        || (*multiline
                            && self
                                .read(pos, true)
                                .is_some_and(|(c, _)| is_line_terminator(c)));
                    pc += 1;
                    ok
                }
                Inst::End { multiline } => {
                    let ok = pos == self.text.len()
                        || (*multiline
                            && self
                                .read(pos, false)
                                .is_some_and(|(c, _)| is_line_terminator(c)));
                    pc += 1;
                    ok
                }
                Inst::Boundary { negated, fold } => {
                    let a = self.is_word(self.read(pos, true), *fold);
                    let b = self.is_word(self.read(pos, false), *fold);
                    pc += 1;
                    (a != b) != *negated
                }
                Inst::Save { slot } => {
                    self.set_cap(*slot, pos as i64);
                    pc += 1;
                    true
                }
                Inst::ClearCaptures { from, to } => {
                    for g in *from..*to {
                        self.set_cap(g * 2, -1);
                        self.set_cap(g * 2 + 1, -1);
                    }
                    pc += 1;
                    true
                }
                Inst::Split { first, second } => {
                    if self.seen(memo, pc, pos) {
                        false
                    } else {
                        self.stack.push(Frame::Retry { pc: *second, pos });
                        pc = *first;
                        true
                    }
                }
                Inst::Jmp { to } => {
                    pc = *to;
                    true
                }
                Inst::BackRef { groups, fold, back } => {
                    // The last of its groups that took part (one, unless
                    // the name stands twice).
                    let span = groups.iter().rev().find_map(|&g| {
                        let s = *self.caps.get(g as usize * 2)?;
                        let e = *self.caps.get(g as usize * 2 + 1)?;
                        (s >= 0 && e >= 0).then_some((s as usize, e as usize))
                    });
                    match span {
                        None => {
                            pc += 1;
                            true
                        }
                        Some((s, e)) => match self.backref(s, e, pos, *fold, *back) {
                            Some(p) => {
                                pos = p;
                                pc += 1;
                                true
                            }
                            None => false,
                        },
                    }
                }
                Inst::Look { negated, end } => {
                    let (negated, end) = (*negated, *end);
                    let saved = self.caps.clone();
                    let saved_regs = self.regs.clone();
                    // A lookaround's body keeps a memo of its own: it is
                    // run afresh from each state that reaches it.
                    let found = self.matches(pc + 1, pos, &mut None)?;
                    let inner = std::mem::replace(&mut self.caps, saved);
                    self.regs = saved_regs;
                    match (found, negated) {
                        (Some(_), false) => {
                            for (slot, v) in inner.into_iter().enumerate() {
                                if self.caps.get(slot) != Some(&v) {
                                    self.set_cap(slot as u32, v);
                                }
                            }
                            pc = end;
                            true
                        }
                        (None, true) => {
                            pc = end;
                            true
                        }
                        _ => false,
                    }
                }
                Inst::RepeatStart { reg } => {
                    self.set_reg(*reg, 0);
                    pc += 1;
                    true
                }
                Inst::RepeatTest {
                    reg,
                    min,
                    max,
                    greedy,
                    exit,
                } => {
                    let count = self.reg(*reg);
                    if count < i64::from(*min) {
                        pc += 1;
                        true
                    } else if *max != u32::MAX && count >= i64::from(*max) {
                        pc = *exit;
                        true
                    } else if self.seen(memo, pc, pos) {
                        false
                    } else if *greedy {
                        self.stack.push(Frame::Retry { pc: *exit, pos });
                        pc += 1;
                        true
                    } else {
                        self.stack.push(Frame::Retry { pc: pc + 1, pos });
                        pc = *exit;
                        true
                    }
                }
                Inst::RepeatMark { reg } => {
                    self.set_reg(reg + 1, pos as i64);
                    pc += 1;
                    true
                }
                Inst::RepeatNext { reg, min, test } => {
                    let count = self.reg(*reg);
                    if count >= i64::from(*min) && self.reg(reg + 1) == pos as i64 {
                        false
                    } else {
                        self.set_reg(*reg, count + 1);
                        pc = *test;
                        true
                    }
                }
            };
            if ok {
                continue;
            }
            // Back to the last choice point, undoing the writes since.
            loop {
                if self.stack.len() <= base {
                    return Ok(None);
                }
                match self.stack.pop() {
                    Some(Frame::Retry { pc: p, pos: q }) => {
                        self.backtracks += 1;
                        pc = p;
                        pos = q;
                        break;
                    }
                    Some(Frame::Cap { slot, old }) => {
                        if let Some(c) = self.caps.get_mut(slot as usize) {
                            *c = old;
                        }
                    }
                    Some(Frame::Reg { index, old }) => {
                        if let Some(r) = self.regs.get_mut(index as usize) {
                            *r = old;
                        }
                    }
                    None => return Ok(None),
                }
            }
        }
    }

    /// Selective memoization: whether the state at a choice point was
    /// seen before (and so failed), once the run has backtracked enough
    /// and the pattern has no backreference. A state is the instruction,
    /// the position and what can still steer the run from there: the
    /// registers of the loops `pc` is in (another loop's are set afresh
    /// before they are read). A count past an unbounded loop's minimum
    /// acts as the minimum. Where the iteration began matters only inside
    /// the body (the loop's test sets it afresh), and there only as
    /// whether the iteration has read anything yet, for the empty check:
    /// a run's position moves one way, so once past the start it never
    /// meets it again. Keeping the start itself would make the states of
    /// k nested loops the input's length to the power k + 1. Captures
    /// never steer a run without backreferences.
    fn seen(&mut self, memo: &mut Memo, pc: u32, pos: usize) -> bool {
        if self.p.backrefs || self.backtracks < MEMO_AFTER {
            return false;
        }
        let mut key = Vec::new();
        for l in &self.p.loops {
            if (l.test..l.exit).contains(&pc) {
                let count = self.reg(l.reg);
                key.push(if l.max == u32::MAX {
                    count.min(i64::from(l.min))
                } else {
                    count
                });
                if pc > l.test {
                    key.push(i64::from(self.reg(l.reg + 1) == pos as i64));
                }
            }
        }
        let set = memo.get_or_insert_with(HashSet::new);
        !set.insert((pc, pos, key.into_boxed_slice()))
    }

    /// A backreference to `s..e` at `pos`: the position past it, or
    /// `None` if the text there differs.
    fn backref(&self, s: usize, e: usize, pos: usize, fold: bool, back: bool) -> Option<usize> {
        let len = e.checked_sub(s)?;
        let (from, to) = if back {
            (pos.checked_sub(len)?, pos)
        } else {
            (pos, pos.checked_add(len)?)
        };
        let want = self.text.get(s..e)?;
        let got = self.text.get(from..to)?;
        let same = if fold {
            // Character by character, as Canonicalize reads them.
            let a = decode(want, self.p.unicode);
            let b = decode(got, self.p.unicode);
            a.len() == b.len()
                && a.iter().zip(&b).all(|(&x, &y)| {
                    canonicalize(x, self.p.unicode) == canonicalize(y, self.p.unicode)
                })
        } else {
            want == got
        };
        same.then_some(if back { from } else { to })
    }
}

/// Code units as characters: code points in Unicode mode.
fn decode(units: &[u16], unicode: bool) -> Vec<u32> {
    if !unicode {
        return units.iter().map(|&u| u32::from(u)).collect();
    }
    char::decode_utf16(units.iter().copied())
        .map(|r| match r {
            Ok(c) => u32::from(c),
            Err(e) => u32::from(e.unpaired_surrogate()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Flags, compile};
    use crate::regexp::parse::parse;
    use crate::stack::StackGuard;

    /// The match of `pattern` (flags as letters) on `text`: the index,
    /// then each group's text or `u`, joined by commas; `-` for none.
    fn m(pattern: &str, flags: &str, text: &str) -> String {
        let f = Flags {
            ignore_case: flags.contains('i'),
            multiline: flags.contains('m'),
            dot_all: flags.contains('s'),
            unicode: flags.contains('u') || flags.contains('v'),
        };
        let sets = flags.contains('v');
        let p = parse(
            pattern,
            f.unicode,
            sets,
            f.ignore_case,
            StackGuard::new(1 << 20),
        )
        .expect("parse");
        let prog = compile(&p, f).expect("compile");
        let units: Vec<u16> = text.encode_utf16().collect();
        // A run past this many steps has lost its bound: it fails rather
        // than runs on.
        let mut steps = 0_u64;
        let mut tick = || {
            steps += 1;
            steps > 20_000_000
        };
        let Some(caps) = prog.exec(&units, 0, false, &mut tick).expect("run") else {
            return "-".to_owned();
        };
        let mut out = vec![caps[0].to_string()];
        for g in 0..caps.len() / 2 {
            let (s, e) = (caps[g * 2], caps[g * 2 + 1]);
            out.push(if s < 0 || e < 0 {
                "u".to_owned()
            } else {
                String::from_utf16_lossy(&units[s as usize..e as usize])
            });
        }
        out.join(",")
    }

    /// The character of code point `cp`.
    fn c(cp: u32) -> String {
        char::from_u32(cp).map(String::from).unwrap_or_default()
    }

    #[test]
    fn the_specs_examples_match_as_it_says() {
        // Section 22.2.2's notes.
        assert_eq!(m("a|ab", "", "abc"), "0,a");
        assert_eq!(m("((a)|(ab))((c)|(bc))", "", "abc"), "0,abc,a,a,u,bc,u,bc");
        assert_eq!(m("a[a-z]{2,4}", "", "abcdefghi"), "0,abcde");
        assert_eq!(m("a[a-z]{2,4}?", "", "abcdefghi"), "0,abc");
        assert_eq!(m("(aa|aabaac|ba|b|c)*", "", "aabaac"), "0,aaba,ba");
        assert_eq!(
            m("(z)((a+)?(b+)?(c))*", "", "zaacbbbcac"),
            "0,zaacbbbcac,z,ac,a,u,c"
        );
        assert_eq!(m("(a*)*", "", "b"), "0,,u");
        assert_eq!(m(r"(a*)b\1+", "", "baaaac"), "0,b,");
        assert_eq!(m("(?=(a+))", "", "baaabac"), "1,,aaa");
        assert_eq!(m(r"(?=(a+))a*b\1", "", "baaabac"), "3,aba,a");
        assert_eq!(
            m(r"(.*?)a(?!(a+)b\2c)\2(.*)", "", "baaabaac"),
            "0,baaabaac,ba,u,abaac"
        );
    }

    #[test]
    fn lookbehind_reads_leftwards() {
        assert_eq!(m(r"(?<=\$)\d+(\.\d*)?", "", "cost $10.53"), "6,10.53,.53");
        assert_eq!(m(r"(?<=(\d+)(\d+))$", "", "1053"), "4,,1,053");
        assert_eq!(m("(?<!a)b", "", "abcb"), "3,b");
        // The proposal's: leftwards, a backreference reads its group only
        // when the group stands to its right.
        assert_eq!(m(r"(?<=(o)d\1)r", "", "hodor"), "-");
        assert_eq!(m(r"(?<=\1d(o))r", "", "hodor"), "4,r,o");
    }

    #[test]
    fn flags_fold_lines_and_code_points() {
        let long_s = c(0x17F);
        let smile = c(0x1F600);
        assert_eq!(m("abc", "i", "xABC"), "1,ABC");
        assert_eq!(m("[a-z]+", "i", "QxY1"), "0,QxY");
        assert_eq!(m(&long_s, "i", "s"), "-");
        assert_eq!(m(&long_s, "ui", "S"), "0,S");
        assert_eq!(m(r"\w", "ui", &long_s), format!("0,{long_s}"));
        assert_eq!(m(r"\W", "ui", &long_s), "-");
        assert_eq!(m("^b$", "m", "a\nb\nc"), "2,b");
        assert_eq!(m("^b$", "", "a\nb\nc"), "-");
        assert_eq!(m("a.c", "", "a\nc"), "-");
        assert_eq!(m("a.c", "s", "a\nc"), "0,a\nc");
        assert_eq!(m("^.$", "u", &smile), format!("0,{smile}"));
        assert_eq!(m("^..$", "", &smile), format!("0,{smile}"));
        assert_eq!(m(r"\bfoo\b", "", "a foo."), "2,foo");
        assert_eq!(m("(?i:a)b", "", "AB"), "-");
        assert_eq!(m("(?i:a)b", "", "Ab"), "0,Ab");
        assert_eq!(m(r"(?<x>a)|(?<x>b)\k<x>", "", "bb"), "0,bb,u,b");
        assert_eq!(m(r"[\q{abc|d}x]", "v", "zabc"), "1,abc");
    }

    #[test]
    fn a_catastrophic_pattern_finishes() {
        // Exponential without memoization: 2^40 ways to fail.
        let text = format!("{}!", "a".repeat(40));
        assert_eq!(m("^(a+)+$", "", &text), "-");
        assert_eq!(m("(a*)*b", "", &text), "-");
        assert_eq!(m("(a|aa)+c", "", &text), "-");
    }

    #[test]
    fn the_memo_carries_over_from_start_to_start() {
        // `a*b` on n `a`s: quadratic if each start tried afresh, linear
        // once the states failed from one start are skipped from the next.
        let n = 100_000;
        let p = parse("a*b", false, false, false, StackGuard::new(1 << 20)).expect("parse");
        let prog = compile(&p, Flags::default()).expect("compile");
        let units: Vec<u16> = "a".repeat(n).encode_utf16().collect();
        // The run stops itself past the bound, so that a quadratic one
        // fails at once rather than running for minutes.
        let limit = 20 * n as u64;
        let mut steps = 0_u64;
        let mut tick = || {
            steps += 1;
            steps > limit
        };
        assert_eq!(prog.exec(&units, 0, false, &mut tick), Ok(None));
    }

    #[test]
    fn nested_loops_stay_linear_once_memoized() {
        // Three nested loops that can split the a's every way (the
        // fuzzer's find): with each iteration's start in the memo's key
        // the states grow as n^4; with only whether it has read anything,
        // as n.
        let n = 300;
        let p = parse(
            "(?:(?:[^c]+){0,})*[^a-c]",
            false,
            false,
            false,
            StackGuard::new(1 << 20),
        )
        .expect("parse");
        let prog = compile(&p, Flags::default()).expect("compile");
        let units: Vec<u16> = "a".repeat(n).encode_utf16().collect();
        let limit = 20_000_000_u64;
        let mut steps = 0_u64;
        let mut tick = || {
            steps += 1;
            steps > limit
        };
        assert_eq!(prog.exec(&units, 0, false, &mut tick), Ok(None));
    }
}
