//! The syntax tree the parser builds (ECMA-262 sections 13 to 16).
//!
//! Nodes live in arenas, one vector per kind, and name each other by
//! `u32` ids ([`ExprId`], [`StmtId`], [`PatId`], [`FuncId`],
//! [`ClassId`]). A left-deep chain (`a + b + c ...`, `a.b.c ...`) is as
//! deep as it is long, and real scripts have chains of many thousands:
//! held in arenas, the tree is freed by dropping a few vectors, never by
//! a recursion as deep as the chain, and every pass over it walks it with
//! a stack of its own (the design's rule: nothing recurses on the Rust
//! stack without a bound).
//!
//! Names are `Box<str>` for now; atoms come with the heap (J2).

use crate::lexer::Keyword;

/// A node's bytes in the source, `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    /// From this span's start to `other`'s end.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start,
            end: other.end.max(self.start),
        }
    }
}

macro_rules! id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub u32);
    };
}

id!(ExprId, "An expression of the tree's arena.");
id!(StmtId, "A statement of the tree's arena.");
id!(
    PatId,
    "A binding or assignment pattern of the tree's arena."
);
id!(FuncId, "A function (of any kind) of the tree's arena.");
id!(ClassId, "A class of the tree's arena.");

/// An identifier as written: its name, escapes resolved, and its span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    pub name: Box<str>,
    pub span: Span,
}

/// A node and where it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Node<T> {
    pub kind: T,
    pub span: Span,
}

/// The arenas.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Ast {
    pub exprs: Vec<Node<Expr>>,
    pub stmts: Vec<Node<Stmt>>,
    pub pats: Vec<Node<Pat>>,
    pub funcs: Vec<Function>,
    pub classes: Vec<Class>,
}

/// Every id is made by the arena it names, by a push, and nothing is
/// ever removed from an arena: an id is in bounds by construction. A
/// lookup that missed would be a bug of the parser's, and panics.
#[allow(clippy::indexing_slicing)]
impl Ast {
    pub fn expr(&self, id: ExprId) -> &Node<Expr> {
        &self.exprs[id.0 as usize]
    }

    pub fn expr_mut(&mut self, id: ExprId) -> &mut Node<Expr> {
        &mut self.exprs[id.0 as usize]
    }

    pub fn stmt(&self, id: StmtId) -> &Node<Stmt> {
        &self.stmts[id.0 as usize]
    }

    pub fn pat(&self, id: PatId) -> &Node<Pat> {
        &self.pats[id.0 as usize]
    }

    pub fn func(&self, id: FuncId) -> &Function {
        &self.funcs[id.0 as usize]
    }

    pub fn func_mut(&mut self, id: FuncId) -> &mut Function {
        &mut self.funcs[id.0 as usize]
    }

    pub fn class(&self, id: ClassId) -> &Class {
        &self.classes[id.0 as usize]
    }
}

fn next_id(len: usize) -> u32 {
    // A source is at most 4 GiB and every node takes at least a byte of
    // it, but an arena's length is checked all the same.
    u32::try_from(len).unwrap_or(u32::MAX)
}

impl Ast {
    pub fn add_expr(&mut self, kind: Expr, span: Span) -> ExprId {
        let id = ExprId(next_id(self.exprs.len()));
        self.exprs.push(Node { kind, span });
        id
    }

    pub fn add_stmt(&mut self, kind: Stmt, span: Span) -> StmtId {
        let id = StmtId(next_id(self.stmts.len()));
        self.stmts.push(Node { kind, span });
        id
    }

    pub fn add_pat(&mut self, kind: Pat, span: Span) -> PatId {
        let id = PatId(next_id(self.pats.len()));
        self.pats.push(Node { kind, span });
        id
    }

    pub fn add_func(&mut self, func: Function) -> FuncId {
        let id = FuncId(next_id(self.funcs.len()));
        self.funcs.push(func);
        id
    }

    pub fn add_class(&mut self, class: Class) -> ClassId {
        let id = ClassId(next_id(self.classes.len()));
        self.classes.push(class);
        id
    }
}

/// A parsed script or module.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub ast: Ast,
    pub body: Vec<StmtId>,
    pub module: bool,
    /// A module, or a script whose directive prologue says "use strict".
    pub strict: bool,
    pub span: Span,
}

/// A template's literal piece.
#[derive(Debug, Clone, PartialEq)]
pub struct Quasi {
    /// `None` for an invalid escape in a tagged template.
    pub cooked: Option<Box<[u16]>>,
    pub raw: Box<str>,
    pub span: Span,
}

/// An argument or array element, spread or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spreadable {
    Expr(ExprId),
    Spread(ExprId),
}

/// A property's key (section 13.2.5).
#[derive(Debug, Clone, PartialEq)]
pub enum PropKey {
    /// An IdentifierName, a string or a number, as the property name it
    /// makes; numbers are kept as written (`f64`) for the canonical
    /// string the compiler makes.
    Ident(Ident),
    String(Box<[u16]>),
    Number(f64),
    BigInt {
        radix: u32,
        digits: Box<str>,
    },
    Computed(ExprId),
    /// `#name` in a class (not in an object literal).
    Private(Ident),
}

/// What an object literal's property is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    /// `key: value`.
    Init,
    /// `name` alone: the value is the identifier.
    Shorthand,
    Method,
    Getter,
    Setter,
    /// `...expr`: the key is unused.
    Spread,
}

/// An object literal's property.
#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    pub key: PropKey,
    pub kind: PropKind,
    /// The value: an expression, or for methods and accessors a
    /// [`Expr::Function`].
    pub value: ExprId,
    pub span: Span,
}

/// A member access's property.
#[derive(Debug, Clone, PartialEq)]
pub enum MemberProp {
    Ident(Ident),
    Computed(ExprId),
    Private(Ident),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Minus,
    Plus,
    Not,
    BitNot,
    Typeof,
    Void,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOp {
    Increment,
    Decrement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Eq,
    NotEq,
    StrictEq,
    StrictNotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Shl,
    Shr,
    UShr,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    BitOr,
    BitXor,
    BitAnd,
    In,
    Instanceof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
    Coalesce,
}

/// `=`, or a compound assignment's operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Binary(BinaryOp),
    Logical(LogicalOp),
}

/// An expression (section 13).
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    This,
    Ident(Ident),
    Null,
    Bool(bool),
    Number(f64),
    BigInt {
        radix: u32,
        digits: Box<str>,
    },
    String(Box<[u16]>),
    RegExp {
        body: Box<str>,
        flags: Box<str>,
    },
    /// `quasis` has one more element than `exprs`.
    Template {
        quasis: Vec<Quasi>,
        exprs: Vec<ExprId>,
    },
    TaggedTemplate {
        tag: ExprId,
        quasis: Vec<Quasi>,
        exprs: Vec<ExprId>,
    },
    /// `None` is a hole.
    Array(Vec<Option<Spreadable>>),
    Object(Vec<Prop>),
    Function(FuncId),
    Arrow(FuncId),
    Class(ClassId),
    Unary {
        op: UnaryOp,
        arg: ExprId,
    },
    Update {
        op: UpdateOp,
        prefix: bool,
        target: ExprId,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Logical {
        op: LogicalOp,
        left: ExprId,
        right: ExprId,
    },
    /// `#name in object`.
    PrivateIn {
        name: Ident,
        right: ExprId,
    },
    Assign {
        op: AssignOp,
        target: AssignTarget,
        value: ExprId,
    },
    Conditional {
        test: ExprId,
        consequent: ExprId,
        alternate: ExprId,
    },
    Sequence(Vec<ExprId>),
    /// Kept so that `(a) = 1` and `({a}) = 1` can be told apart, and for
    /// a function's name inference; the compiler sees through it.
    Paren(ExprId),
    Member {
        object: ExprId,
        prop: MemberProp,
        /// `?.` before this access.
        optional: bool,
    },
    SuperMember(MemberProp),
    Call {
        callee: ExprId,
        args: Vec<Spreadable>,
        optional: bool,
    },
    SuperCall(Vec<Spreadable>),
    New {
        callee: ExprId,
        args: Vec<Spreadable>,
    },
    /// The whole of an optional chain, `a?.b.c`: where a short circuit
    /// ends.
    OptionalChain(ExprId),
    NewTarget,
    ImportMeta,
    /// `import(specifier, options)`.
    Import {
        source: ExprId,
        options: Option<ExprId>,
    },
    Yield {
        arg: Option<ExprId>,
        delegate: bool,
    },
    Await(ExprId),
}

/// What an assignment assigns to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignTarget {
    /// An identifier or a member expression.
    Simple(ExprId),
    /// A destructuring pattern (`=` only).
    Pattern(PatId),
}

/// An element of an array pattern, or a parameter: a target and its
/// default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatElem {
    pub target: PatId,
    pub default: Option<ExprId>,
}

/// A property of an object pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct PatProp {
    pub key: PropKey,
    pub value: PatElem,
    /// `{a}` or `{a = 1}`: the key is the binding.
    pub shorthand: bool,
    pub span: Span,
}

/// A binding pattern (declarations, parameters) or an assignment
/// pattern (destructuring assignment, `for (... of ...)` heads).
#[derive(Debug, Clone, PartialEq)]
pub enum Pat {
    Ident(Ident),
    /// In an assignment pattern: a member expression, `[a.b] = c`.
    Expr(ExprId),
    Object {
        props: Vec<PatProp>,
        rest: Option<PatId>,
    },
    /// `None` is an elision.
    Array {
        elems: Vec<Option<PatElem>>,
        rest: Option<PatId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Var,
    Let,
    Const,
    Using,
    AwaitUsing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declarator {
    pub target: PatId,
    pub init: Option<ExprId>,
}

/// A declaration of `var`, `let`, `const`, `using` or `await using`.
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub kind: VarKind,
    pub decls: Vec<Declarator>,
}

/// A `for` statement's first part.
#[derive(Debug, Clone, PartialEq)]
pub enum ForInit {
    Var(VarDecl),
    Expr(ExprId),
}

/// The left of `for (... in ...)` and `for (... of ...)`.
#[derive(Debug, Clone, PartialEq)]
pub enum ForHead {
    /// One declarator without an initializer (but Annex B's
    /// `for (var x = 1 in o)`, which keeps it).
    Var(VarDecl),
    Pattern(PatId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Catch {
    pub param: Option<PatId>,
    pub body: Vec<StmtId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    /// `None` for `default`.
    pub test: Option<ExprId>,
    pub body: Vec<StmtId>,
    pub span: Span,
}

/// A string as a module names it, in UTF-16 (a module export name may
/// be any string, section 16.2.2).
pub type ModuleString = Box<[u16]>;

/// `with { type: "json" }` after a module specifier.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportAttribute {
    pub key: ModuleString,
    pub value: ModuleString,
    pub span: Span,
}

/// What an import binds.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportSpec {
    /// `import x from`.
    Default(Ident),
    /// `import * as x from`.
    Namespace(Ident),
    /// `import { name as local }`.
    Named { name: ModuleString, local: Ident },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl {
    pub specs: Vec<ImportSpec>,
    pub source: ModuleString,
    pub attributes: Vec<ImportAttribute>,
    /// `import source x from`, or `import defer * as x from`.
    pub phase: ImportPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportPhase {
    Evaluation,
    Source,
    Defer,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportSpec {
    /// The local name, or the imported name in a re-export.
    pub local: ModuleString,
    pub exported: ModuleString,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportDecl {
    /// `export { a as b }`, or `export { a } from "m"`.
    Named {
        specs: Vec<ExportSpec>,
        source: Option<ModuleString>,
        attributes: Vec<ImportAttribute>,
    },
    /// `export * from "m"` or `export * as ns from "m"`.
    All {
        exported: Option<ModuleString>,
        source: ModuleString,
        attributes: Vec<ImportAttribute>,
    },
    /// `export var/let/const/function/class ...`.
    Decl(StmtId),
    /// `export default function/class ...` (maybe anonymous).
    DefaultDecl(StmtId),
    /// `export default expression;`.
    DefaultExpr(ExprId),
}

/// A statement or declaration (sections 14 to 16).
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Expr(ExprId),
    Var(VarDecl),
    Function(FuncId),
    Class(ClassId),
    Block(Vec<StmtId>),
    Empty,
    If {
        test: ExprId,
        consequent: StmtId,
        alternate: Option<StmtId>,
    },
    For {
        init: Option<ForInit>,
        test: Option<ExprId>,
        update: Option<ExprId>,
        body: StmtId,
    },
    ForIn {
        left: ForHead,
        right: ExprId,
        body: StmtId,
    },
    ForOf {
        left: ForHead,
        right: ExprId,
        body: StmtId,
        is_await: bool,
    },
    While {
        test: ExprId,
        body: StmtId,
    },
    DoWhile {
        body: StmtId,
        test: ExprId,
    },
    Return(Option<ExprId>),
    Break(Option<Ident>),
    Continue(Option<Ident>),
    Throw(ExprId),
    Try {
        block: Vec<StmtId>,
        handler: Option<Catch>,
        finalizer: Option<Vec<StmtId>>,
    },
    Switch {
        discriminant: ExprId,
        cases: Vec<Case>,
    },
    Labeled {
        label: Ident,
        body: StmtId,
    },
    With {
        object: ExprId,
        body: StmtId,
    },
    Debugger,
    Import(ImportDecl),
    Export(ExportDecl),
}

/// What a function is, which decides what its body may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuncKind {
    /// A declaration or expression.
    Normal,
    Arrow,
    /// An object literal's or class's method.
    Method,
    Getter,
    Setter,
    /// A class's constructor; `derived` for a class with `extends`.
    Constructor {
        derived: bool,
    },
    /// A class field's initializer or static block, compiled as a method
    /// of the class.
    ClassField,
    StaticBlock,
}

/// A parameter: a target and its default.
pub type Param = PatElem;

/// A function's body.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Block(Vec<StmtId>),
    /// An arrow's concise body, or a field's initializer.
    Expr(ExprId),
}

/// A function of any kind.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub kind: FuncKind,
    pub name: Option<Ident>,
    pub params: Vec<Param>,
    pub rest: Option<PatId>,
    pub body: Body,
    pub is_async: bool,
    pub is_generator: bool,
    /// Strict by its context or its own directive.
    pub strict: bool,
    /// No defaults, patterns or rest (section 15.1.3's IsSimpleParameterList).
    pub simple_params: bool,
    /// The source text, `Function.prototype.toString`'s answer.
    pub span: Span,
}

/// What a class member is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    Method,
    Getter,
    Setter,
    Field,
    /// `accessor x` (decorators' auto-accessors).
    Accessor,
    StaticBlock,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassMember {
    pub kind: MemberKind,
    pub is_static: bool,
    /// Unused for a static block.
    pub key: PropKey,
    /// A method's function; a field's initializer (a
    /// [`FuncKind::ClassField`] function), or none; a static block's body.
    pub value: Option<FuncId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: Option<Ident>,
    pub extends: Option<ExprId>,
    pub constructor: Option<FuncId>,
    pub members: Vec<ClassMember>,
    pub span: Span,
}

/// A keyword's text, for error messages.
pub fn keyword_text(k: Keyword) -> &'static str {
    match k {
        Keyword::Break => "break",
        Keyword::Case => "case",
        Keyword::Catch => "catch",
        Keyword::Class => "class",
        Keyword::Const => "const",
        Keyword::Continue => "continue",
        Keyword::Debugger => "debugger",
        Keyword::Default => "default",
        Keyword::Delete => "delete",
        Keyword::Do => "do",
        Keyword::Else => "else",
        Keyword::Enum => "enum",
        Keyword::Export => "export",
        Keyword::Extends => "extends",
        Keyword::False => "false",
        Keyword::Finally => "finally",
        Keyword::For => "for",
        Keyword::Function => "function",
        Keyword::If => "if",
        Keyword::Import => "import",
        Keyword::In => "in",
        Keyword::Instanceof => "instanceof",
        Keyword::New => "new",
        Keyword::Null => "null",
        Keyword::Return => "return",
        Keyword::Super => "super",
        Keyword::Switch => "switch",
        Keyword::This => "this",
        Keyword::Throw => "throw",
        Keyword::True => "true",
        Keyword::Try => "try",
        Keyword::Typeof => "typeof",
        Keyword::Var => "var",
        Keyword::Void => "void",
        Keyword::While => "while",
        Keyword::With => "with",
    }
}
