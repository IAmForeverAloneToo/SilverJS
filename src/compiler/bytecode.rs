//! The register machine's instructions (design note, section 1.2): an
//! opcode byte, then its operands. Registers, indices and immediates are
//! one byte each by default; a `Wide` prefix makes every such operand of
//! the next instruction two bytes and `ExtraWide` four. Jump offsets are
//! always four bytes, relative to the start of the jump's instruction
//! (its prefix, if any), so a forward jump is patched in place once its
//! target is known. Multi-byte operands are little-endian.
//!
//! The accumulator is implicit: most instructions read it, write it, or
//! both. A binary operator `Op r` computes `r op acc` into the
//! accumulator; a store (`Star`, `StaSlot`, `SetNamed`, ...) leaves the
//! accumulator as it was, so an assignment's value is still there after
//! it.

/// What an operand is, which says how it is encoded and printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// A register of the frame: `r3`.
    Reg,
    /// An unsigned index or count: a constant, a name, an inline-cache
    /// slot, a scope, a function, an argument count.
    Idx,
    /// A signed immediate (`LdaSmi`).
    Imm,
    /// A jump offset: four bytes, signed, from the instruction's start.
    Jump,
}

macro_rules! ops {
    ($($name:ident ( $($k:ident),* ) $doc:literal,)*) => {
        /// An instruction's opcode.
        #[repr(u8)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Op {
            $(#[doc = $doc] $name,)*
        }

        const ALL: &[Op] = &[$(Op::$name,)*];

        impl Op {
            /// The operands the instruction takes, in order.
            pub fn operands(self) -> &'static [Operand] {
                match self {
                    $(Op::$name => &[$(ops!(@kind $k)),*],)*
                }
            }

            /// The instruction's name, as the disassembler prints it.
            pub fn name(self) -> &'static str {
                match self {
                    $(Op::$name => stringify!($name),)*
                }
            }
        }
    };
    (@kind R) => { Operand::Reg };
    (@kind U) => { Operand::Idx };
    (@kind I) => { Operand::Imm };
    (@kind J) => { Operand::Jump };
}

ops! {
    Wide() "Prefix: the next instruction's operands are two bytes.",
    ExtraWide() "Prefix: the next instruction's operands are four bytes.",

    LdaUndefined() "acc = undefined.",
    LdaNull() "acc = null.",
    LdaTrue() "acc = true.",
    LdaFalse() "acc = false.",
    LdaUninitialized() "acc = the uninitialized binding's marker (a TDZ).",
    LdaSmi(I) "acc = a small integer.",
    LdaConst(U) "acc = a constant of the function's pool.",
    Ldar(R) "acc = r.",
    Star(R) "r = acc.",
    Mov(R, R) "The second register = the first.",
    LdaThis() "acc = the frame's this value.",
    LdaNewTarget() "acc = the frame's new.target.",
    LdaClosure() "acc = the function being run.",
    LdaGlobalThis() "acc = the realm's global this value.",

    CheckTdz(U) "Throws a ReferenceError naming the constant if acc is uninitialized.",
    CheckTdzRegister(R, U) "The same check on a register; acc is kept.",
    CheckTdzSlot(U, U, U) "The same check on a slot (depth, slot, name); acc is kept.",
    ThrowConstAssign(U) "Throws the TypeError of an assignment to a constant.",
    ThrowInvalidLhs() "Throws the ReferenceError of an invalid assignment target.",

    LdaSlot(U, U) "acc = the slot of the environment `depth` hops out.",
    StaSlot(U, U) "That slot = acc.",
    LdaGlobal(U, U) "acc = the global named by the constant (name, ic); a ReferenceError if none.",
    LdaGlobalTypeof(U, U) "The same, undefined if none (for typeof).",
    StaGlobal(U, U) "The global named = acc (a ReferenceError if none, in strict code).",
    InitGlobalLexical(U) "Initializes the script's top-level let, const or class binding named.",
    LdaDynamic(U) "acc = the binding named, looked up through the environments at run time.",
    LdaDynamicTypeof(U) "The same, undefined if none.",
    StaDynamic(U) "The binding named, looked up at run time, = acc.",
    DeleteName(U) "acc = delete of the binding named (sloppy code).",

    PushContext(R, U) "r = the current environment; a new one of the scope given becomes current.",
    PopContext(R) "The environment in r becomes current.",
    StoreContext(R) "r = the current environment.",
    CreateFunctionContext(U) "At a function's entry: its environment, of the scope given.",
    CopyContext() "A copy of the current environment replaces it (a loop's next iteration).",
    PushWithContext(R) "r = the current environment; an object environment for acc becomes current.",

    CreateClosure(U) "acc = a closure of the unit's function given, over the current environment.",
    CreateArguments(U) "acc = the arguments object (1: mapped to the parameters' slots).",
    CreateRestParameter(U) "acc = an array of the arguments from the index given on.",
    CreateObject() "acc = a new ordinary object.",
    CreateArray() "acc = a new empty array.",
    CreateRegExp(U, U) "acc = a new RegExp of the constants given (body, flags).",

    GetNamed(R, U, U) "acc = r[name] (obj, name, ic).",
    GetKeyed(R, U) "acc = r[acc] (obj, ic).",
    SetNamed(R, U, U) "r[name] = acc (obj, name, ic).",
    SetKeyed(R, R, U) "r[key] = acc (obj, key, ic).",
    DefineField(R, U) "Defines r's own data property named = acc (a literal's).",
    DefineKeyed(R, R) "Defines r's own data property at key = acc.",
    DefineIndex(R, U) "Defines r's own element at the index given = acc (an array literal's).",
    SetArrayLength(R, U) "Sets the array r's length (an array literal's trailing holes).",
    DefineGetter(R, R) "Defines acc as r's getter at key.",
    DefineSetter(R, R) "Defines acc as r's setter at key.",
    SetProtoLiteral(R) "A literal's `__proto__: acc`: r's prototype = acc if an object or null.",
    CopyDataProperties(R) "Copies acc's own enumerable properties onto r (`...acc`).",
    SetFunctionName(R, U) "Names the function in acc by the key in r (prefix 0: none, 1: get, 2: set).",
    ToPropertyKey() "acc = ToPropertyKey(acc).",
    DeleteProperty(R) "acc = delete r[acc].",

    Add(R) "acc = r + acc.",
    Sub(R) "acc = r - acc.",
    Mul(R) "acc = r * acc.",
    Div(R) "acc = r / acc.",
    Mod(R) "acc = r % acc.",
    Exp(R) "acc = r ** acc.",
    BitOr(R) "acc = r | acc.",
    BitXor(R) "acc = r ^ acc.",
    BitAnd(R) "acc = r & acc.",
    Shl(R) "acc = r << acc.",
    Shr(R) "acc = r >> acc.",
    UShr(R) "acc = r >>> acc.",
    Inc() "acc = ToNumeric(acc) + 1.",
    Dec() "acc = ToNumeric(acc) - 1.",
    Negate() "acc = -acc.",
    BitNot() "acc = ~acc.",
    ToNumber() "acc = +acc.",
    ToNumeric() "acc = ToNumeric(acc) (a postfix update's old value).",
    Not() "acc = !acc.",
    TypeOf() "acc = typeof acc.",
    ToString() "acc = ToString(acc) (a template's substitution).",

    TestEq(R) "acc = r == acc.",
    TestStrictEq(R) "acc = r === acc.",
    TestLt(R) "acc = r < acc.",
    TestGt(R) "acc = r > acc.",
    TestLtEq(R) "acc = r <= acc.",
    TestGtEq(R) "acc = r >= acc.",
    TestIn(R) "acc = r in acc.",
    TestInstanceof(R) "acc = r instanceof acc.",

    Jump(J) "Jumps.",
    JumpIfTrue(J) "Jumps if acc is truthy.",
    JumpIfFalse(J) "Jumps if acc is falsy.",
    JumpIfNullish(J) "Jumps if acc is null or undefined.",
    JumpIfNotNullish(J) "Jumps unless acc is null or undefined.",
    JumpIfUndefined(J) "Jumps if acc is undefined.",
    JumpIfNotUndefined(J) "Jumps unless acc is undefined.",
    JumpLoop(J) "Jumps back to a loop's head, charging the step budget.",

    Call(R, R, R, U, U) "acc = callee(...args) with a receiver (callee, receiver, first arg, count, ic).",
    CallUndefined(R, R, U, U) "acc = callee(...args), receiver undefined (callee, first arg, count, ic).",
    CallEval(R, R, U) "A direct eval if the callee is %eval%, else a call (callee, first arg, count).",
    New(R, R, U, U) "acc = new callee(...args) (callee, first arg, count, ic).",
    Return() "Returns acc.",
    Throw() "Throws acc.",
    Debugger() "A debugger statement: nothing, without a debugger.",
    ForInEnumerate() "acc = an iterator over acc's enumerable string keys (for-in).",
    ForInNext(R) "acc = the iterator r's next key, or undefined when done.",

    GetIterator(R, R, U) "GetIterator(acc, sync): the iterator into r1, its next method into r2 (u: 1 names the value itself in an error, not its source).",
    IteratorNext(R, R, R, J) "IteratorStepValue (iterator, next, done): a jump if done already; else done = true, the step, at its end the jump, else acc = its value and done = false.",
    IteratorClose(R, R) "IteratorClose on a normal completion unless r2 (done) is true: return called, its result an object; acc kept.",
    IteratorCloseQuiet(R, R) "IteratorClose on a throw completion unless r2 (done) is true: return called, what it does ignored; acc kept.",
    ArrayPush(R) "Appends acc to the array r, at its length.",
    ArrayPushHole(R) "Lengthens the array r by a hole.",
    SpreadInto(R, U) "Appends every value acc's iterator gives to the array r (u: 1 for a call's arguments, whose error differs).",
    CallSpread(R, R, R, U) "acc = callee(...array) with a receiver (callee, receiver, arguments array, flags: 1 a direct eval if %eval%).",
    NewSpread(R, R) "acc = new callee(...array) (callee, arguments array).",
    CopyRest(R) "acc = a new object with acc's own enumerable properties but the property keys in the array r.",
    RequireObjectCoercible(U, U) "Throws a TypeError if acc is null or undefined (an object pattern's value; u1 its first property's name, or empty; u2: 1 names the value itself, not its source).",
}

impl Op {
    pub fn from_byte(b: u8) -> Option<Op> {
        ALL.get(usize::from(b)).copied()
    }
}

/// A decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instr {
    pub op: Op,
    /// Its operands, in order; the rest are 0. Jump offsets are signed.
    pub operands: [i64; 5],
    /// Its first byte (its prefix's, if any).
    pub pc: u32,
    /// Its bytes, prefix included.
    pub len: u32,
}

impl Instr {
    /// A jump's target.
    pub fn target(&self) -> Option<u32> {
        let i = self
            .op
            .operands()
            .iter()
            .position(|k| *k == Operand::Jump)?;
        let offset = *self.operands.get(i)?;
        u32::try_from(i64::from(self.pc) + offset).ok()
    }
}

/// Reads the instruction at `pc`; `None` past the end or on bytes that
/// are not one (a unit the compiler wrote always decodes).
pub fn decode(code: &[u8], pc: u32) -> Option<Instr> {
    let start = pc as usize;
    let mut at = start;
    let mut first = Op::from_byte(*code.get(at)?)?;
    at += 1;
    let scale = match first {
        Op::Wide => 2,
        Op::ExtraWide => 4,
        _ => 1,
    };
    if scale > 1 {
        first = Op::from_byte(*code.get(at)?)?;
        at += 1;
        if matches!(first, Op::Wide | Op::ExtraWide) {
            return None;
        }
    }
    let mut operands = [0i64; 5];
    for (slot, kind) in operands.iter_mut().zip(first.operands()) {
        let width = if *kind == Operand::Jump { 4 } else { scale };
        let bytes = code.get(at..at + width)?;
        at += width;
        let mut raw = 0u64;
        for (i, b) in bytes.iter().enumerate() {
            raw |= u64::from(*b) << (8 * i);
        }
        *slot = if matches!(kind, Operand::Imm | Operand::Jump) {
            // Sign-extended from its width.
            let shift = 64 - 8 * width as u32;
            ((raw << shift) as i64) >> shift
        } else {
            raw as i64
        };
    }
    Some(Instr {
        op: first,
        operands,
        pc,
        len: u32::try_from(at - start).ok()?,
    })
}

/// The bytes one operand takes at `scale` (1, 2 or 4).
fn fits(value: i64, kind: Operand, scale: usize) -> bool {
    match (kind, scale) {
        (Operand::Jump, _) => true,
        (Operand::Imm, 1) => i8::try_from(value).is_ok(),
        (Operand::Imm, 2) => i16::try_from(value).is_ok(),
        (Operand::Imm, _) => i32::try_from(value).is_ok(),
        (_, 1) => u8::try_from(value).is_ok(),
        (_, 2) => u16::try_from(value).is_ok(),
        _ => u32::try_from(value).is_ok(),
    }
}

/// Appends an instruction with its operands, choosing the narrowest
/// scale they all fit; returns the byte offsets of its jump operand, if
/// it has one, for patching.
pub(crate) fn encode(code: &mut Vec<u8>, op: Op, operands: &[i64]) -> Option<usize> {
    let kinds = op.operands();
    debug_assert_eq!(kinds.len(), operands.len(), "{op:?}");
    let scale = [1usize, 2, 4]
        .into_iter()
        .find(|&s| operands.iter().zip(kinds).all(|(v, k)| fits(*v, *k, s)))
        .unwrap_or(4);
    match scale {
        2 => code.push(Op::Wide as u8),
        4 => code.push(Op::ExtraWide as u8),
        _ => {}
    }
    code.push(op as u8);
    let mut jump_at = None;
    for (value, kind) in operands.iter().zip(kinds) {
        let width = if *kind == Operand::Jump {
            jump_at = Some(code.len());
            4
        } else {
            scale
        };
        let raw = *value as u64;
        for i in 0..width {
            code.push((raw >> (8 * i)) as u8);
        }
    }
    jump_at
}

/// Writes a jump's four-byte offset at `at`.
pub(crate) fn patch_jump(code: &mut [u8], at: usize, offset: i32) {
    for (i, b) in offset.to_le_bytes().into_iter().enumerate() {
        if let Some(slot) = code.get_mut(at + i) {
            *slot = b;
        }
    }
}
