//! Scope analysis: where each binding lives.
//!
//! One iterative walk over the tree (a stack of work items, never a
//! recursion) makes the scopes, declares their bindings (hoisting `var`
//! and function declarations as ECMA-262's declaration instantiation
//! does) and records every name referenced and every direct `eval`.
//! Then the references are resolved against the finished scopes, and a
//! binding a closure, `eval` or `with` can reach is marked captured.
//! Last, each binding is given its place: a register of its function's
//! frame, a slot of its scope's environment if captured, or a global
//! looked up by name (a script's top-level declarations).
//!
//! `this` and `new.target` are resolved like names (the names cannot be
//! identifiers): an arrow's lie in the function around it, and are
//! stored in that function's environment only when an arrow captures
//! them. `arguments` is made in a function that references it.

use std::collections::HashMap;

use crate::ast::{
    AssignTarget, Body, ClassId, Expr, ExprId, ForHead, ForInit, FuncId, FuncKind, MemberProp, Pat,
    PatId, Program, PropKey, Spreadable, Stmt, StmtId, VarDecl, VarKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ScopeId(pub u32);

/// The node a scope belongs to, by which the code generator finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ScopeKey {
    Program,
    Func(FuncId),
    /// A named function expression's own name, around its function.
    FuncName(FuncId),
    /// A block, a `for` statement's `let` or `const` head, a `switch`'s
    /// cases, a `with`'s body.
    Stmt(StmtId),
    TryBlock(StmtId),
    Catch(StmtId),
    Finally(StmtId),
    Class(ClassId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeKind {
    Script,
    /// Eval code's top level: its own scope, whose `var`s and functions
    /// are its caller's when it is sloppy.
    Eval,
    Function {
        arrow: bool,
    },
    FuncName {
        function: ScopeId,
    },
    Block,
    Catch,
    Loop,
    Switch,
    With,
    Class,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindingKind {
    Var,
    Let,
    Const,
    /// A function declaration's name (var-like at a function's top level,
    /// lexical in a block).
    Function,
    Param,
    CatchParam,
    /// A named function expression's own name: immutable.
    FuncSelf,
    Class,
    This,
    NewTarget,
    Arguments,
    /// A method's home object (`%home`), which `super.x` reads from.
    Home,
    /// The running function itself (`%fn`), whose prototype `super()`
    /// constructs.
    FunctionObject,
    /// A class's private name (`#x`): made when the class is, before
    /// anything in it can read it.
    Private,
}

impl BindingKind {
    /// Starts uninitialized (a temporal dead zone).
    pub(crate) fn lexical(self) -> bool {
        matches!(
            self,
            BindingKind::Let | BindingKind::Const | BindingKind::Class
        )
    }
}

/// Where a binding lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Loc {
    Unassigned,
    Register(u32),
    /// A slot of its scope's environment (for a named function
    /// expression's own name, of its function's environment).
    Slot(u32),
    /// A script's top-level declaration, by name.
    Global,
    /// `this` or `new.target` read from the frame (not captured).
    Intrinsic,
    /// A sloppy eval's `var` or function, made in its caller's variable
    /// environment and found by name.
    Dynamic,
}

#[derive(Debug, Clone)]
pub(crate) struct Binding {
    pub name: Box<str>,
    pub kind: BindingKind,
    pub captured: bool,
    pub loc: Loc,
    /// Where its declaration ends: a reference from its own function past
    /// it, outside a `switch`, needs no TDZ check.
    pub tdz_end: u32,
    /// A parameter's position (the last, for a repeated name).
    pub param: Option<u32>,
    /// The function declaration that initializes it at its scope's entry
    /// (the last, for a repeated name).
    pub func: Option<FuncId>,
}

#[derive(Debug, Clone)]
pub(crate) struct Scope {
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>,
    /// The function it belongs to, an index of [`Analysis::functions`].
    pub func: u32,
    pub names: HashMap<Box<str>, u32>,
    pub bindings: Vec<Binding>,
    /// A sloppy direct `eval` may declare `var`s in it, or it is a
    /// `with`: a name not found by its end is looked up at run time.
    pub dynamic: bool,
    /// It is the variable scope of a direct `eval`.
    pub has_eval: bool,
    /// It has an environment at run time.
    pub has_env: bool,
    /// Its environment's slots: names, whether each is lexical, and
    /// what an assignment by name meets ([`ScopeInfo::constant`]).
    pub slots: Vec<(Box<str>, bool, u8)>,
}

#[derive(Debug, Clone)]
pub(crate) struct FunctionInfo {
    pub scope: ScopeId,
    pub func: Option<FuncId>,
    pub strict: bool,
    pub arrow: bool,
    pub param_count: u32,
    /// The registers its bindings take, parameters' included.
    pub registers: u32,
    /// Its `arguments` object maps to its parameters' slots.
    pub mapped_arguments: bool,
    /// A derived class's constructor: its `this` is bound by `super()`.
    pub derived: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Analysis {
    pub scopes: Vec<Scope>,
    pub functions: Vec<FunctionInfo>,
    pub keys: HashMap<ScopeKey, ScopeId>,
    pub func_index: HashMap<FuncId, u32>,
    /// Each class constructor's class, whose instance elements it (or
    /// its `super()`) initializes.
    pub ctor_class: HashMap<FuncId, ClassId>,
}

/// Where a name resolves, from a scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    Register(u32),
    /// The slot `slot` of the environment `depth` hops out.
    Slot {
        depth: u32,
        slot: u32,
    },
    Global,
    /// Looked up by name at run time (past a `with` or a sloppy `eval`).
    Dynamic,
    /// `this` or `new.target` of the frame.
    Intrinsic,
    /// `this` at a script's top level.
    GlobalThis,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Resolved {
    pub place: Place,
    pub kind: Option<BindingKind>,
    pub tdz_end: u32,
    pub same_function: bool,
    pub in_switch: bool,
    /// A derived class's constructor's `this`: uninitialized until its
    /// `super()`, so every read checks it.
    pub uninit_this: bool,
}

/// The names a non-arrow function's own scope holds for the arrows in
/// it: `this`, `new.target`, its home object and itself.
fn is_special(name: &str) -> bool {
    matches!(name, "this" | "new.target" | "%home" | "%fn")
}

/// Scopes, bindings and functions are made by pushes and never removed,
/// and named by the indices those pushes gave: in bounds by
/// construction, as the tree's ids are.
#[allow(clippy::indexing_slicing)]
impl Analysis {
    pub(crate) fn scope(&self, s: ScopeId) -> &Scope {
        &self.scopes[s.0 as usize]
    }

    fn scope_mut(&mut self, s: ScopeId) -> &mut Scope {
        &mut self.scopes[s.0 as usize]
    }

    pub(crate) fn function(&self, f: u32) -> &FunctionInfo {
        &self.functions[f as usize]
    }

    pub(crate) fn key(&self, key: ScopeKey) -> Option<ScopeId> {
        self.keys.get(&key).copied()
    }

    /// Where `name` resolves from `from`, as the code generator needs it.
    pub(crate) fn resolve(&self, from: ScopeId, name: &str) -> Resolved {
        let special = is_special(name);
        let here = self.scope(from).func;
        let mut depth = 0u32;
        let mut dynamic = false;
        let mut cur = Some(from);
        let none = |place| Resolved {
            place,
            kind: None,
            tdz_end: 0,
            same_function: false,
            in_switch: false,
            uninit_this: false,
        };
        while let Some(c) = cur {
            let scope = self.scope(c);
            if let Some(&i) = scope.names.get(name) {
                if dynamic && !special {
                    return none(Place::Dynamic);
                }
                let b = &scope.bindings[i as usize];
                let place = match b.loc {
                    Loc::Register(r) => Place::Register(r),
                    Loc::Slot(slot) => {
                        // A function's own name lives in the function's
                        // environment, which `depth` has already passed.
                        let depth = match scope.kind {
                            ScopeKind::FuncName { .. } => depth.saturating_sub(1),
                            _ => depth,
                        };
                        Place::Slot { depth, slot }
                    }
                    Loc::Global => Place::Global,
                    Loc::Dynamic => Place::Dynamic,
                    Loc::Intrinsic | Loc::Unassigned => Place::Intrinsic,
                };
                return Resolved {
                    place,
                    kind: Some(b.kind),
                    tdz_end: b.tdz_end,
                    same_function: scope.func == here,
                    in_switch: scope.kind == ScopeKind::Switch,
                    uninit_this: b.kind == BindingKind::This
                        && self.functions[scope.func as usize].derived,
                };
            }
            if special && scope.kind == (ScopeKind::Function { arrow: false }) {
                // Not referenced where the analysis looked: the frame's.
                return none(Place::Intrinsic);
            }
            if scope.dynamic && !special {
                dynamic = true;
            }
            if scope.has_env {
                depth += 1;
            }
            cur = scope.parent;
        }
        let eval = self
            .scopes
            .first()
            .is_some_and(|s| s.kind == ScopeKind::Eval);
        if name == "this" && !eval {
            none(Place::GlobalThis)
        } else if special && !(eval && matches!(name, "%home" | "%fn")) {
            // Eval code's `this` and `new.target`: its caller's, which
            // its frame is given. Its home object and function are its
            // caller's bindings, found by name.
            none(Place::Intrinsic)
        } else if dynamic || eval {
            none(Place::Dynamic)
        } else {
            none(Place::Global)
        }
    }
}

/// A walk's work item.
#[derive(Debug, Clone, Copy)]
enum Item {
    Stmt(StmtId),
    Expr(ExprId),
    Pat(PatId, Target),
    /// A function; `true` for an expression whose name binds inside it.
    Func(FuncId, bool),
    Class(ClassId),
    /// Makes a scope made earlier current (a `try`'s, a `switch`'s).
    Enter(ScopeId),
    Leave,
}

#[derive(Debug, Clone, Copy)]
enum Target {
    Declare(BindingKind, u32, Option<u32>),
    Assign,
}

struct Walker<'a> {
    program: &'a Program,
    a: Analysis,
    work: Vec<Item>,
    current: Vec<ScopeId>,
    refs: Vec<(ScopeId, crate::ast::Name)>,
    evals: Vec<ScopeId>,
}

/// Analyzes a script.
pub(crate) fn analyze(program: &Program) -> Analysis {
    analyze_as(program, ScopeKind::Script)
}

/// Analyzes eval code.
pub(crate) fn analyze_eval(program: &Program) -> Analysis {
    analyze_as(program, ScopeKind::Eval)
}

fn analyze_as(program: &Program, top_kind: ScopeKind) -> Analysis {
    let mut w = Walker {
        program,
        a: Analysis::default(),
        work: Vec::new(),
        current: Vec::new(),
        refs: Vec::new(),
        evals: Vec::new(),
    };
    w.a.functions.push(FunctionInfo {
        scope: ScopeId(0),
        func: None,
        strict: program.strict,
        arrow: false,
        param_count: 0,
        registers: 0,
        mapped_arguments: false,
        derived: false,
    });
    let top = w.new_scope(top_kind, None, 0, ScopeKey::Program);
    w.current.push(top);
    w.push_stmts(&program.body);
    w.run();
    w.resolve_all();
    w.place_all();
    w.a
}

/// As for [`Analysis`]: every index was given by a push.
#[allow(clippy::indexing_slicing)]
impl<'a> Walker<'a> {
    fn cur(&self) -> ScopeId {
        self.current.last().copied().unwrap_or(ScopeId(0))
    }

    fn new_scope(
        &mut self,
        kind: ScopeKind,
        parent: Option<ScopeId>,
        func: u32,
        key: ScopeKey,
    ) -> ScopeId {
        let id = ScopeId(u32::try_from(self.a.scopes.len()).unwrap_or(u32::MAX));
        self.a.scopes.push(Scope {
            kind,
            parent,
            func,
            names: HashMap::new(),
            bindings: Vec::new(),
            dynamic: kind == ScopeKind::With,
            has_eval: false,
            has_env: false,
            slots: Vec::new(),
        });
        self.a.keys.insert(key, id);
        id
    }

    /// Enters a scope of the current function; its `Leave` is pushed.
    fn enter(&mut self, kind: ScopeKind, key: ScopeKey) {
        let parent = self.cur();
        let func = self.a.scope(parent).func;
        let s = self.new_scope(kind, Some(parent), func, key);
        self.current.push(s);
        self.work.push(Item::Leave);
    }

    fn push_stmts(&mut self, stmts: &[StmtId]) {
        for s in stmts.iter().rev() {
            self.work.push(Item::Stmt(*s));
        }
    }

    /// The nearest scope `var` declarations go to.
    fn var_scope(&self, from: ScopeId) -> ScopeId {
        let mut s = from;
        loop {
            let scope = self.a.scope(s);
            match (scope.kind, scope.parent) {
                (ScopeKind::Script | ScopeKind::Eval | ScopeKind::Function { .. }, _)
                | (_, None) => return s,
                (_, Some(p)) => s = p,
            }
        }
    }

    fn declare(
        &mut self,
        s: ScopeId,
        name: &str,
        kind: BindingKind,
        tdz_end: u32,
        param: Option<u32>,
        func: Option<FuncId>,
    ) {
        let scope = self.a.scope_mut(s);
        if let Some(&i) = scope.names.get(name) {
            // A name declared again where that is allowed (`var` twice, a
            // function and a `var`, a repeated parameter): one binding.
            let b = &mut scope.bindings[i as usize];
            if param.is_some() {
                b.param = param;
            }
            if func.is_some() {
                b.func = func;
            }
            return;
        }
        let i = u32::try_from(scope.bindings.len()).unwrap_or(u32::MAX);
        scope.bindings.push(Binding {
            name: name.into(),
            kind,
            captured: false,
            loc: Loc::Unassigned,
            tdz_end,
            param,
            func,
        });
        scope.names.insert(name.into(), i);
    }

    fn var_decl(&mut self, decl: &VarDecl, end: u32) {
        let kind = match decl.kind {
            VarKind::Var => BindingKind::Var,
            VarKind::Let => BindingKind::Let,
            VarKind::Const | VarKind::Using | VarKind::AwaitUsing => BindingKind::Const,
        };
        for d in decl.decls.iter().rev() {
            if let Some(init) = d.init {
                self.work.push(Item::Expr(init));
            }
            self.work
                .push(Item::Pat(d.target, Target::Declare(kind, end, None)));
        }
    }

    fn run(&mut self) {
        let ast = &self.program.ast;
        while let Some(item) = self.work.pop() {
            match item {
                Item::Leave => {
                    self.current.pop();
                }
                Item::Enter(s) => self.current.push(s),
                Item::Stmt(id) => self.stmt(id),
                Item::Expr(id) => self.expr(id),
                Item::Pat(id, target) => match &ast.pat(id).kind {
                    Pat::Ident(ident) => match target {
                        Target::Declare(kind, end, param) => {
                            let s = if kind == BindingKind::Var {
                                self.var_scope(self.cur())
                            } else {
                                self.cur()
                            };
                            self.declare(s, &ident.name, kind, end, param, None);
                        }
                        Target::Assign => self.refs.push((self.cur(), ident.name.clone())),
                    },
                    Pat::Expr(e) => self.work.push(Item::Expr(*e)),
                    Pat::Object { props, rest } => {
                        let target = match target {
                            Target::Declare(kind, end, Some(_)) => Target::Declare(kind, end, None),
                            t => t,
                        };
                        if let Some(r) = rest {
                            self.work.push(Item::Pat(*r, target));
                        }
                        for p in props.iter().rev() {
                            self.work.push(Item::Pat(p.value.target, target));
                            if let Some(d) = p.value.default {
                                self.work.push(Item::Expr(d));
                            }
                            if let PropKey::Computed(e) = p.key {
                                self.work.push(Item::Expr(e));
                            }
                        }
                    }
                    Pat::Array { elems, rest } => {
                        let target = match target {
                            Target::Declare(kind, end, Some(_)) => Target::Declare(kind, end, None),
                            t => t,
                        };
                        if let Some(r) = rest {
                            self.work.push(Item::Pat(*r, target));
                        }
                        for e in elems.iter().rev().flatten() {
                            self.work.push(Item::Pat(e.target, target));
                            if let Some(d) = e.default {
                                self.work.push(Item::Expr(d));
                            }
                        }
                    }
                },
                Item::Func(id, named_expr) => self.func(id, named_expr),
                Item::Class(id) => {
                    let class = ast.class(id);
                    self.enter(ScopeKind::Class, ScopeKey::Class(id));
                    let s = self.cur();
                    for m in &class.members {
                        if let PropKey::Private(p) = &m.key {
                            let name = format!("#{}", p.name);
                            self.declare(s, &name, BindingKind::Private, 0, None, None);
                        }
                    }
                    if let Some(c) = class.constructor {
                        self.a.ctor_class.insert(c, id);
                    }
                    if let Some(name) = &class.name {
                        // Initialized when the class is made: its heritage
                        // and computed keys see it in its TDZ.
                        let end = class.span.end;
                        self.declare(s, &name.name, BindingKind::Const, end, None, None);
                    }
                    for m in class.members.iter().rev() {
                        if let Some(f) = m.value {
                            self.work.push(Item::Func(f, false));
                        }
                        if let PropKey::Computed(e) = m.key {
                            self.work.push(Item::Expr(e));
                        }
                    }
                    if let Some(c) = class.constructor {
                        self.work.push(Item::Func(c, false));
                    }
                    if let Some(e) = class.extends {
                        self.work.push(Item::Expr(e));
                    }
                }
            }
        }
    }

    fn stmt(&mut self, id: StmtId) {
        let ast = &self.program.ast;
        let node = ast.stmt(id);
        let end = node.span.end;
        match &node.kind {
            Stmt::Expr(e) | Stmt::Throw(e) => self.work.push(Item::Expr(*e)),
            Stmt::Return(e) => {
                if let Some(e) = e {
                    self.work.push(Item::Expr(*e));
                }
            }
            Stmt::Var(decl) => self.var_decl(decl, end),
            Stmt::Function(f) => {
                let func = ast.func(*f);
                let cur = self.cur();
                // Var-like at a function's top level, lexical in a block:
                // either way made at its scope's entry.
                if let Some(name) = &func.name {
                    self.declare(cur, &name.name, BindingKind::Function, 0, None, Some(*f));
                }
                self.work.push(Item::Func(*f, false));
            }
            Stmt::Class(c) => {
                if let Some(name) = &ast.class(*c).name {
                    let cur = self.cur();
                    self.declare(cur, &name.name, BindingKind::Class, end, None, None);
                }
                self.work.push(Item::Class(*c));
            }
            Stmt::Block(stmts) => {
                self.enter(ScopeKind::Block, ScopeKey::Stmt(id));
                self.push_stmts(stmts);
            }
            Stmt::Empty | Stmt::Debugger | Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::If {
                test,
                consequent,
                alternate,
            } => {
                if let Some(a) = alternate {
                    self.work.push(Item::Stmt(*a));
                }
                self.work.push(Item::Stmt(*consequent));
                self.work.push(Item::Expr(*test));
            }
            Stmt::For {
                init,
                test,
                update,
                body,
            } => {
                if let Some(ForInit::Var(decl)) = init
                    && decl.kind != VarKind::Var
                {
                    self.enter(ScopeKind::Loop, ScopeKey::Stmt(id));
                }
                self.work.push(Item::Stmt(*body));
                if let Some(u) = update {
                    self.work.push(Item::Expr(*u));
                }
                if let Some(t) = test {
                    self.work.push(Item::Expr(*t));
                }
                match init {
                    Some(ForInit::Var(decl)) => self.var_decl(decl, end),
                    Some(ForInit::Expr(e)) => self.work.push(Item::Expr(*e)),
                    None => {}
                }
            }
            Stmt::ForIn { left, right, body }
            | Stmt::ForOf {
                left, right, body, ..
            } => {
                if let ForHead::Var(decl) = left
                    && decl.kind != VarKind::Var
                {
                    self.enter(ScopeKind::Loop, ScopeKey::Stmt(id));
                }
                self.work.push(Item::Stmt(*body));
                self.work.push(Item::Expr(*right));
                match left {
                    ForHead::Var(decl) => self.var_decl(decl, end),
                    ForHead::Pattern(p) => self.work.push(Item::Pat(*p, Target::Assign)),
                }
            }
            Stmt::While { test, body } | Stmt::DoWhile { body, test } => {
                self.work.push(Item::Stmt(*body));
                self.work.push(Item::Expr(*test));
            }
            Stmt::Try {
                block,
                handler,
                finalizer,
            } => {
                // Three sibling scopes, entered in turn; pushed last first.
                if let Some(f) = finalizer {
                    let s = self.sibling(ScopeKind::Block, ScopeKey::Finally(id));
                    self.work.push(Item::Leave);
                    self.push_stmts(f);
                    self.work.push(Item::Enter(s));
                }
                if let Some(c) = handler {
                    let s = self.sibling(ScopeKind::Catch, ScopeKey::Catch(id));
                    self.work.push(Item::Leave);
                    self.push_stmts(&c.body);
                    if let Some(p) = c.param {
                        self.work.push(Item::Pat(
                            p,
                            Target::Declare(BindingKind::CatchParam, 0, None),
                        ));
                    }
                    self.work.push(Item::Enter(s));
                }
                let s = self.sibling(ScopeKind::Block, ScopeKey::TryBlock(id));
                self.work.push(Item::Leave);
                self.push_stmts(block);
                self.work.push(Item::Enter(s));
            }
            Stmt::Switch {
                discriminant,
                cases,
            } => {
                let s = self.sibling(ScopeKind::Switch, ScopeKey::Stmt(id));
                self.work.push(Item::Leave);
                for c in cases.iter().rev() {
                    self.push_stmts(&c.body);
                    if let Some(t) = c.test {
                        self.work.push(Item::Expr(t));
                    }
                }
                self.work.push(Item::Enter(s));
                self.work.push(Item::Expr(*discriminant));
            }
            Stmt::Labeled { body, .. } => self.work.push(Item::Stmt(*body)),
            Stmt::With { object, body } => {
                let s = self.sibling(ScopeKind::With, ScopeKey::Stmt(id));
                self.work.push(Item::Leave);
                self.work.push(Item::Stmt(*body));
                self.work.push(Item::Enter(s));
                self.work.push(Item::Expr(*object));
            }
            // Modules are refused before the analysis.
            Stmt::Import(_) | Stmt::Export(_) => {}
        }
    }

    /// A scope of the current function, made now, entered by an
    /// [`Item::Enter`] pushed after it.
    fn sibling(&mut self, kind: ScopeKind, key: ScopeKey) -> ScopeId {
        let parent = self.cur();
        let func = self.a.scope(parent).func;
        self.new_scope(kind, Some(parent), func, key)
    }

    fn func(&mut self, id: FuncId, named_expr: bool) {
        let ast = &self.program.ast;
        let f = ast.func(id);
        let arrow = f.kind == FuncKind::Arrow;
        let index = u32::try_from(self.a.functions.len()).unwrap_or(u32::MAX);
        let parent = self.cur();
        let name_scope = match (&f.name, named_expr) {
            (Some(_), true) => Some(self.new_scope(
                ScopeKind::FuncName {
                    function: ScopeId(0),
                },
                Some(parent),
                index,
                ScopeKey::FuncName(id),
            )),
            _ => None,
        };
        let fs = self.new_scope(
            ScopeKind::Function { arrow },
            Some(name_scope.unwrap_or(parent)),
            index,
            ScopeKey::Func(id),
        );
        self.a.functions.push(FunctionInfo {
            scope: fs,
            func: Some(id),
            strict: f.strict,
            arrow,
            param_count: u32::try_from(f.params.len()).unwrap_or(u32::MAX),
            registers: 0,
            mapped_arguments: false,
            derived: matches!(f.kind, FuncKind::Constructor { derived: true }),
        });
        self.a.func_index.insert(id, index);
        if let (Some(ns), Some(name)) = (name_scope, &f.name) {
            self.a.scope_mut(ns).kind = ScopeKind::FuncName { function: fs };
            self.declare(ns, &name.name, BindingKind::FuncSelf, 0, None, None);
            self.current.push(ns);
            self.work.push(Item::Leave);
        }
        self.current.push(fs);
        self.work.push(Item::Leave);
        match &f.body {
            Body::Block(stmts) => self.push_stmts(stmts),
            Body::Expr(e) => self.work.push(Item::Expr(*e)),
        }
        if let Some(r) = f.rest {
            self.work
                .push(Item::Pat(r, Target::Declare(BindingKind::Param, 0, None)));
        }
        for (i, p) in f.params.iter().enumerate().rev() {
            if let Some(d) = p.default {
                self.work.push(Item::Expr(d));
            }
            let pos = u32::try_from(i).unwrap_or(u32::MAX);
            self.work.push(Item::Pat(
                p.target,
                Target::Declare(BindingKind::Param, 0, Some(pos)),
            ));
        }
    }

    fn push_args(&mut self, args: &[Spreadable]) {
        for a in args.iter().rev() {
            match a {
                Spreadable::Expr(x) | Spreadable::Spread(x) => self.work.push(Item::Expr(*x)),
            }
        }
    }

    fn expr(&mut self, id: ExprId) {
        let ast = &self.program.ast;
        let cur = self.cur();
        match &ast.expr(id).kind {
            Expr::Ident(i) => self.refs.push((cur, i.name.clone())),
            Expr::This => self.refs.push((cur, "this".into())),
            Expr::NewTarget => self.refs.push((cur, "new.target".into())),
            Expr::Null
            | Expr::Bool(_)
            | Expr::Number(_)
            | Expr::BigInt { .. }
            | Expr::String(_)
            | Expr::RegExp { .. }
            | Expr::ImportMeta => {}
            Expr::Template { exprs, .. } => {
                for e in exprs.iter().rev() {
                    self.work.push(Item::Expr(*e));
                }
            }
            Expr::TaggedTemplate { tag, exprs, .. } => {
                for e in exprs.iter().rev() {
                    self.work.push(Item::Expr(*e));
                }
                self.work.push(Item::Expr(*tag));
            }
            Expr::Array(elems) => {
                for e in elems.iter().rev().flatten() {
                    match e {
                        Spreadable::Expr(x) | Spreadable::Spread(x) => {
                            self.work.push(Item::Expr(*x));
                        }
                    }
                }
            }
            Expr::Object(props) => {
                for p in props.iter().rev() {
                    self.work.push(Item::Expr(p.value));
                    if let PropKey::Computed(k) = p.key {
                        self.work.push(Item::Expr(k));
                    }
                }
            }
            Expr::Function(f) => self.work.push(Item::Func(*f, true)),
            Expr::Arrow(f) => self.work.push(Item::Func(*f, false)),
            Expr::Class(c) => self.work.push(Item::Class(*c)),
            Expr::PrivateIn { name, right } => {
                self.refs.push((cur, format!("#{}", name.name).into()));
                self.work.push(Item::Expr(*right));
            }
            Expr::Unary { arg: e, .. }
            | Expr::Update { target: e, .. }
            | Expr::Paren(e)
            | Expr::OptionalChain(e)
            | Expr::Await(e) => self.work.push(Item::Expr(*e)),
            Expr::Binary { left, right, .. } | Expr::Logical { left, right, .. } => {
                self.work.push(Item::Expr(*right));
                self.work.push(Item::Expr(*left));
            }
            Expr::Assign { target, value, .. } => {
                self.work.push(Item::Expr(*value));
                match target {
                    AssignTarget::Simple(e) => self.work.push(Item::Expr(*e)),
                    AssignTarget::Pattern(p) => self.work.push(Item::Pat(*p, Target::Assign)),
                }
            }
            Expr::Conditional {
                test,
                consequent,
                alternate,
            } => {
                self.work.push(Item::Expr(*alternate));
                self.work.push(Item::Expr(*consequent));
                self.work.push(Item::Expr(*test));
            }
            Expr::Sequence(v) => {
                for e in v.iter().rev() {
                    self.work.push(Item::Expr(*e));
                }
            }
            Expr::Member { object, prop, .. } => {
                match prop {
                    MemberProp::Computed(e) => self.work.push(Item::Expr(*e)),
                    MemberProp::Private(i) => {
                        self.refs.push((cur, format!("#{}", i.name).into()));
                    }
                    MemberProp::Ident(_) => {}
                }
                self.work.push(Item::Expr(*object));
            }
            Expr::SuperMember(prop) => {
                self.refs.push((cur, "this".into()));
                self.refs.push((cur, "%home".into()));
                if let MemberProp::Computed(e) = prop {
                    self.work.push(Item::Expr(*e));
                }
            }
            Expr::Call { callee, args, .. } => {
                if let Expr::Ident(i) = &ast.expr(*callee).kind
                    && &*i.name == "eval"
                {
                    self.evals.push(cur);
                }
                self.push_args(args);
                self.work.push(Item::Expr(*callee));
            }
            Expr::SuperCall(args) => {
                self.refs.push((cur, "this".into()));
                self.refs.push((cur, "new.target".into()));
                self.refs.push((cur, "%fn".into()));
                self.push_args(args);
            }
            Expr::SuperCallForward => {
                self.refs.push((cur, "this".into()));
                self.refs.push((cur, "new.target".into()));
                self.refs.push((cur, "%fn".into()));
            }
            Expr::New { callee, args } => {
                self.push_args(args);
                self.work.push(Item::Expr(*callee));
            }
            Expr::Import { source, options } => {
                if let Some(o) = options {
                    self.work.push(Item::Expr(*o));
                }
                self.work.push(Item::Expr(*source));
            }
            Expr::Yield { arg, .. } => {
                if let Some(a) = arg {
                    self.work.push(Item::Expr(*a));
                }
            }
        }
    }

    /// Resolves a reference: a binding of another function, or one past
    /// a dynamic scope (found by name at run time), is captured. `this`,
    /// `new.target` and `arguments` are made in the first non-arrow
    /// function they reach.
    fn mark(&mut self, from: ScopeId, name: &str) {
        let special = is_special(name);
        let here = self.a.scope(from).func;
        let mut dynamic = false;
        let mut cur = Some(from);
        while let Some(c) = cur {
            let scope = self.a.scope(c);
            let crossing = scope.func != here || (dynamic && !special);
            if let Some(&i) = scope.names.get(name) {
                if crossing {
                    self.a.scope_mut(c).bindings[i as usize].captured = true;
                }
                return;
            }
            // Eval code's top level holds its own `this` and `new.target`
            // (its caller's), for the arrows inside it.
            if scope.kind == (ScopeKind::Function { arrow: false })
                || (scope.kind == ScopeKind::Eval && special && !matches!(name, "%home" | "%fn"))
            {
                let kind = match name {
                    "this" => Some(BindingKind::This),
                    "new.target" => Some(BindingKind::NewTarget),
                    "arguments" => Some(BindingKind::Arguments),
                    "%home" => Some(BindingKind::Home),
                    "%fn" => Some(BindingKind::FunctionObject),
                    _ => None,
                };
                if let Some(kind) = kind {
                    self.declare(c, name, kind, 0, None, None);
                    if crossing && let Some(b) = self.a.scope_mut(c).bindings.last_mut() {
                        b.captured = true;
                    }
                    return;
                }
            }
            if scope.dynamic && !special {
                dynamic = true;
            }
            cur = scope.parent;
        }
    }

    fn resolve_all(&mut self) {
        let refs = std::mem::take(&mut self.refs);
        for (s, name) in &refs {
            self.mark(*s, name);
        }
        for s in std::mem::take(&mut self.evals) {
            // A direct eval: its code may name any binding around it, and
            // a sloppy one may declare `var`s in its variable scope.
            let vs = self.var_scope(s);
            let func = self.a.scope(vs).func;
            let strict = self.a.functions[func as usize].strict;
            let scope = self.a.scope_mut(vs);
            if scope.kind != ScopeKind::Script {
                scope.has_eval = true;
                scope.dynamic |= !strict;
            }
            for name in ["this", "new.target", "arguments", "%home", "%fn"] {
                self.mark(s, name);
            }
            // A field initializer's or static block's eval refuses
            // `arguments`: a marker its eval finds.
            let mut at = Some(s);
            while let Some(c) = at {
                let scope = self.a.scope(c);
                if scope.kind == (ScopeKind::Function { arrow: false }) {
                    let field = self.a.functions[scope.func as usize].func.is_some_and(|f| {
                        matches!(
                            self.program.ast.func(f).kind,
                            FuncKind::ClassField | FuncKind::StaticBlock
                        )
                    });
                    if field {
                        self.declare(c, "%field", BindingKind::Var, 0, None, None);
                    }
                    break;
                }
                at = scope.parent;
            }
            let mut cur = Some(s);
            while let Some(c) = cur {
                let scope = self.a.scope_mut(c);
                for b in &mut scope.bindings {
                    if !matches!(b.kind, BindingKind::This | BindingKind::NewTarget) {
                        b.captured = true;
                    }
                }
                cur = scope.parent;
            }
        }
        // A sloppy function with simple parameters maps its arguments
        // object to them, so they live in its environment.
        for fi in 0..self.a.functions.len() {
            let info = &self.a.functions[fi];
            let Some(fid) = info.func else { continue };
            if info.arrow || info.strict || !self.program.ast.func(fid).simple_params {
                continue;
            }
            let scope = self.a.scope_mut(info.scope);
            if !scope
                .bindings
                .iter()
                .any(|b| b.kind == BindingKind::Arguments)
            {
                continue;
            }
            for b in &mut scope.bindings {
                if b.kind == BindingKind::Param {
                    b.captured = true;
                }
            }
            self.a.functions[fi].mapped_arguments = true;
        }
    }

    fn place_all(&mut self) {
        for f in &mut self.a.functions {
            f.registers = f.param_count;
        }
        for si in 0..self.a.scopes.len() {
            let (kind, func) = (self.a.scopes[si].kind, self.a.scopes[si].func);
            if kind == ScopeKind::Script {
                for b in &mut self.a.scopes[si].bindings {
                    b.loc = Loc::Global;
                }
                continue;
            }
            // A sloppy eval's `var`s and functions are its caller's.
            let sloppy_eval = kind == ScopeKind::Eval && !self.a.functions[0].strict;
            let home = match kind {
                ScopeKind::FuncName { function } => function.0 as usize,
                _ => si,
            };
            for bi in 0..self.a.scopes[si].bindings.len() {
                let b = &self.a.scopes[si].bindings[bi];
                let (bkind, captured, param) = (b.kind, b.captured, b.param);
                let loc =
                    if sloppy_eval && matches!(bkind, BindingKind::Var | BindingKind::Function) {
                        Loc::Dynamic
                    } else if matches!(
                        bkind,
                        BindingKind::This
                            | BindingKind::NewTarget
                            | BindingKind::Home
                            | BindingKind::FunctionObject
                    ) && !captured
                        && !(bkind == BindingKind::This && self.a.functions[func as usize].derived)
                    {
                        Loc::Intrinsic
                    } else if captured {
                        let name = b.name.clone();
                        let slots = &mut self.a.scopes[home].slots;
                        let slot = u32::try_from(slots.len()).unwrap_or(u32::MAX);
                        let constant = match bkind {
                            BindingKind::Const => 1,
                            BindingKind::FuncSelf => 2,
                            _ => 0,
                        };
                        slots.push((name, bkind.lexical(), constant));
                        Loc::Slot(slot)
                    } else if let (BindingKind::Param, Some(p)) = (bkind, param) {
                        Loc::Register(p)
                    } else {
                        let f = &mut self.a.functions[func as usize];
                        f.registers += 1;
                        Loc::Register(f.registers - 1)
                    };
                self.a.scopes[si].bindings[bi].loc = loc;
            }
        }
        for s in &mut self.a.scopes {
            s.has_env = !s.slots.is_empty() || s.has_eval || s.kind == ScopeKind::With;
        }
    }
}
