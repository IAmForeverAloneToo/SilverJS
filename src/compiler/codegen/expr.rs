//! Expressions: each leaves its value in the accumulator.

use super::{Gen, Label, R, units, unsupported};
use crate::ast::{
    AssignOp, AssignTarget, BinaryOp, Expr, ExprId, LogicalOp, MemberProp, PropKey, PropKind, Span,
    Spreadable, UnaryOp, UpdateOp,
};
use crate::compiler::Constant;
use crate::compiler::bytecode::Op;
use crate::compiler::scope::Place;
use crate::number::number_to_string;

/// A step of the operator walk.
enum W {
    Eval(ExprId),
    /// Saves the accumulator, a binary operator's left operand.
    Save,
    Apply(BinaryOp, Span),
    Test(LogicalOp, Label),
    Bind(Label),
    Unary(UnaryOp, Span),
}

/// What a member chain's last access does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ChainMode {
    Value,
    Delete,
}

fn binary_op(op: BinaryOp) -> (Op, bool) {
    match op {
        BinaryOp::Eq => (Op::TestEq, false),
        BinaryOp::NotEq => (Op::TestEq, true),
        BinaryOp::StrictEq => (Op::TestStrictEq, false),
        BinaryOp::StrictNotEq => (Op::TestStrictEq, true),
        BinaryOp::Lt => (Op::TestLt, false),
        BinaryOp::LtEq => (Op::TestLtEq, false),
        BinaryOp::Gt => (Op::TestGt, false),
        BinaryOp::GtEq => (Op::TestGtEq, false),
        BinaryOp::Shl => (Op::Shl, false),
        BinaryOp::Shr => (Op::Shr, false),
        BinaryOp::UShr => (Op::UShr, false),
        BinaryOp::Add => (Op::Add, false),
        BinaryOp::Sub => (Op::Sub, false),
        BinaryOp::Mul => (Op::Mul, false),
        BinaryOp::Div => (Op::Div, false),
        BinaryOp::Mod => (Op::Mod, false),
        BinaryOp::Exp => (Op::Exp, false),
        BinaryOp::BitOr => (Op::BitOr, false),
        BinaryOp::BitXor => (Op::BitXor, false),
        BinaryOp::BitAnd => (Op::BitAnd, false),
        BinaryOp::In => (Op::TestIn, false),
        BinaryOp::Instanceof => (Op::TestInstanceof, false),
    }
}

/// A property key written in the source, as the string it names.
pub(super) fn static_key(key: &PropKey) -> Option<Box<[u16]>> {
    match key {
        PropKey::Ident(i) => Some(units(&i.name)),
        PropKey::String(s) => Some(s.clone()),
        PropKey::Number(n) => Some(units(&number_to_string(*n))),
        PropKey::BigInt { radix, digits } => {
            let n = u128::from_str_radix(digits, *radix).ok()?;
            Some(units(&n.to_string()))
        }
        PropKey::Computed(_) | PropKey::Private(_) => None,
    }
}

impl Gen<'_, '_> {
    pub(super) fn expr(&mut self, id: ExprId) -> R {
        self.expr_named(id, None)
    }

    fn unparen(&self, mut id: ExprId) -> ExprId {
        while let Expr::Paren(inner) = self.ast.expr(id).kind {
            id = inner;
        }
        id
    }

    /// Whether `id` is an anonymous function, which takes the name of
    /// what it is assigned to.
    pub(super) fn is_anonymous_function(&self, id: ExprId) -> bool {
        match self.ast.expr(self.unparen(id)).kind {
            Expr::Function(f) | Expr::Arrow(f) => self.ast.func(f).name.is_none(),
            _ => false,
        }
    }

    /// An expression, an anonymous function in it named `name`.
    pub(super) fn expr_named(&mut self, id: ExprId, name: Option<&[u16]>) -> R {
        let node = self.ast.expr(id);
        let span = node.span;
        self.check(span)?;
        match &node.kind {
            Expr::This => self.load_name("this", span, false)?,
            Expr::NewTarget => self.load_name("new.target", span, false)?,
            Expr::Ident(i) => self.load_name(&i.name, span, false)?,
            Expr::Null => self.op(Op::LdaNull),
            Expr::Bool(true) => self.op(Op::LdaTrue),
            Expr::Bool(false) => self.op(Op::LdaFalse),
            Expr::Number(n) => self.load_number(*n),
            Expr::BigInt { radix, digits } => {
                let c = self.constant(Constant::BigInt {
                    radix: *radix,
                    digits: digits.clone(),
                });
                self.emit(Op::LdaConst, &[c]);
            }
            Expr::String(s) => {
                let c = self.string(s);
                self.emit(Op::LdaConst, &[c]);
            }
            Expr::RegExp { body, flags } => {
                let b = self.name(body);
                let f = self.name(flags);
                self.pos(span);
                self.emit(Op::CreateRegExp, &[b, f]);
            }
            Expr::Template { quasis, exprs } => self.template(quasis, exprs, span)?,
            Expr::TaggedTemplate { .. } => return Err(unsupported("tagged templates", span)),
            Expr::Array(elems) => self.array(elems)?,
            Expr::Object(props) => self.object(props)?,
            Expr::Function(f) | Expr::Arrow(f) => self.closure(*f, name)?,
            Expr::Class(_) => return Err(unsupported("classes", span)),
            Expr::Unary { .. } | Expr::Binary { .. } | Expr::Logical { .. } => {
                self.operators(id)?
            }
            Expr::Update { op, prefix, target } => self.update(*op, *prefix, *target, span)?,
            Expr::PrivateIn { .. } => return Err(unsupported("private names", span)),
            Expr::Assign { op, target, value } => self.assign(*op, *target, *value, span)?,
            Expr::Conditional {
                test,
                consequent,
                alternate,
            } => {
                let other = self.new_label();
                let end = self.new_label();
                self.expr(*test)?;
                self.jump(Op::JumpIfFalse, other);
                self.expr(*consequent)?;
                self.jump(Op::Jump, end);
                self.bind(other);
                self.expr(*alternate)?;
                self.bind(end);
            }
            Expr::Sequence(v) => {
                for e in v {
                    self.expr(*e)?;
                }
            }
            Expr::Paren(e) => self.expr_named(*e, name)?,
            Expr::Member { .. } | Expr::Call { .. } | Expr::OptionalChain(_) => {
                self.chain(id, ChainMode::Value)?;
            }
            Expr::SuperMember(_) | Expr::SuperCall(_) => return Err(unsupported("super", span)),
            Expr::New { callee, args } => {
                self.expr(*callee)?;
                let c = self.alloc();
                self.reg_op(Op::Star, c);
                let (first, n) = self.args(args)?;
                let ic = self.ic();
                // The callee, which a "not a constructor" error quotes; a
                // stack frame's column is at `new`.
                self.pos_at(self.ast.expr(*callee).span, span.start);
                self.emit(Op::New, &[i64::from(c), i64::from(first), i64::from(n), ic]);
                self.free_n(first, n);
                self.free(c);
            }
            Expr::ImportMeta | Expr::Import { .. } => return Err(unsupported("modules", span)),
            Expr::Yield { .. } => return Err(unsupported("generators", span)),
            Expr::Await(_) => return Err(unsupported("async functions", span)),
        }
        Ok(())
    }

    pub(super) fn load_number(&mut self, n: f64) {
        let int = n as i32;
        if f64::from(int) == n && !(n == 0.0 && n.is_sign_negative()) {
            self.emit(Op::LdaSmi, &[i64::from(int)]);
        } else {
            let c = self.number(n);
            self.emit(Op::LdaConst, &[c]);
        }
    }

    /// Unary, binary and logical operators, with a stack of the
    /// compiler's own: the parser builds their chains in loops, so they
    /// may be as long as the source, left-deep or (`**`) right-deep.
    fn operators(&mut self, id: ExprId) -> R {
        let mut work = vec![W::Eval(id)];
        let mut saved: Vec<u32> = Vec::new();
        while let Some(w) = work.pop() {
            match w {
                W::Eval(e) => {
                    let node = self.ast.expr(e);
                    match &node.kind {
                        Expr::Binary { op, left, right } => {
                            work.push(W::Apply(*op, node.span));
                            work.push(W::Eval(*right));
                            work.push(W::Save);
                            work.push(W::Eval(*left));
                        }
                        Expr::Logical { op, left, right } => {
                            let end = self.new_label();
                            work.push(W::Bind(end));
                            work.push(W::Eval(*right));
                            work.push(W::Test(*op, end));
                            work.push(W::Eval(*left));
                        }
                        Expr::Unary { op, arg } => match op {
                            UnaryOp::Typeof
                                if matches!(
                                    self.ast.expr(self.unparen(*arg)).kind,
                                    Expr::Ident(_)
                                ) =>
                            {
                                let arg = self.unparen(*arg);
                                let argn = self.ast.expr(arg);
                                if let Expr::Ident(i) = &argn.kind {
                                    self.load_name(&i.name, argn.span, true)?;
                                }
                                self.op(Op::TypeOf);
                            }
                            UnaryOp::Delete => self.delete(*arg, node.span)?,
                            _ => {
                                work.push(W::Unary(*op, node.span));
                                work.push(W::Eval(*arg));
                            }
                        },
                        _ => self.expr(e)?,
                    }
                }
                W::Save => {
                    let r = self.alloc();
                    self.reg_op(Op::Star, r);
                    saved.push(r);
                }
                W::Apply(op, span) => {
                    let r = saved.pop().unwrap_or(0);
                    let (instr, negate) = binary_op(op);
                    self.pos(span);
                    self.reg_op(instr, r);
                    if negate {
                        self.op(Op::Not);
                    }
                    self.free(r);
                }
                W::Test(op, end) => self.jump(
                    match op {
                        LogicalOp::And => Op::JumpIfFalse,
                        LogicalOp::Or => Op::JumpIfTrue,
                        LogicalOp::Coalesce => Op::JumpIfNotNullish,
                    },
                    end,
                ),
                W::Bind(l) => self.bind(l),
                W::Unary(op, span) => match op {
                    UnaryOp::Minus => {
                        self.pos(span);
                        self.op(Op::Negate);
                    }
                    UnaryOp::Plus => {
                        self.pos(span);
                        self.op(Op::ToNumber);
                    }
                    UnaryOp::Not => self.op(Op::Not),
                    UnaryOp::BitNot => {
                        self.pos(span);
                        self.op(Op::BitNot);
                    }
                    UnaryOp::Typeof => self.op(Op::TypeOf),
                    UnaryOp::Void => self.op(Op::LdaUndefined),
                    UnaryOp::Delete => {}
                },
            }
        }
        Ok(())
    }

    fn delete(&mut self, arg: ExprId, span: Span) -> R {
        let arg = self.unparen(arg);
        let node = self.ast.expr(arg);
        match &node.kind {
            Expr::Member {
                object,
                prop,
                optional: false,
            } => {
                self.expr(*object)?;
                let o = self.alloc();
                self.reg_op(Op::Star, o);
                self.member_key(prop)?;
                self.pos(span);
                self.reg_op(Op::DeleteProperty, o);
                self.free(o);
            }
            Expr::OptionalChain(_) => self.chain(arg, ChainMode::Delete)?,
            Expr::Ident(i) => match self.resolve(&i.name).place {
                Place::Global | Place::Dynamic => {
                    let n = self.name(&i.name);
                    self.emit(Op::DeleteName, &[n]);
                }
                _ => self.op(Op::LdaFalse),
            },
            _ => {
                self.expr(arg)?;
                self.op(Op::LdaTrue);
            }
        }
        Ok(())
    }

    /// A member's key into the accumulator.
    fn member_key(&mut self, prop: &MemberProp) -> R {
        match prop {
            MemberProp::Ident(i) => {
                let n = self.name(&i.name);
                self.emit(Op::LdaConst, &[n]);
            }
            MemberProp::Computed(e) => self.expr(*e)?,
            MemberProp::Private(i) => return Err(unsupported("private names", i.span)),
        }
        Ok(())
    }

    /// Reads the property `prop` of the object in register `o`.
    /// Where a stack frame stopped at a call of `callee` points: the
    /// name called (a property's, for a method), else the arguments'
    /// opening parenthesis, as the reference browser's frames do.
    fn stack_at(&self, callee: ExprId) -> u32 {
        let node = self.ast.expr(callee);
        match &node.kind {
            Expr::Ident(i) => i.span.start,
            Expr::Member {
                prop: MemberProp::Ident(i) | MemberProp::Private(i),
                ..
            } => i.span.start,
            Expr::Member {
                prop: MemberProp::Computed(e),
                ..
            } => self.ast.expr(*e).span.start,
            _ => node.span.end,
        }
    }

    fn get(&mut self, o: u32, prop: &MemberProp, span: Span) -> R {
        match prop {
            MemberProp::Ident(i) => {
                let n = self.name(&i.name);
                let ic = self.ic();
                self.pos_at(span, i.span.start);
                self.emit(Op::GetNamed, &[i64::from(o), n, ic]);
            }
            MemberProp::Computed(e) => {
                let at = self.ast.expr(*e).span.start;
                self.expr(*e)?;
                let ic = self.ic();
                self.pos_at(span, at);
                self.emit(Op::GetKeyed, &[i64::from(o), ic]);
            }
            MemberProp::Private(i) => return Err(unsupported("private names", i.span)),
        }
        Ok(())
    }

    /// Arguments into consecutive registers: the first and the count.
    fn args(&mut self, args: &[Spreadable]) -> R<(u32, u32)> {
        let n = u32::try_from(args.len()).unwrap_or(u32::MAX);
        let first = self.alloc_n(n);
        for (i, a) in args.iter().enumerate() {
            match a {
                Spreadable::Expr(e) => {
                    self.expr(*e)?;
                    self.reg_op(Op::Star, first + u32::try_from(i).unwrap_or(0));
                }
                Spreadable::Spread(e) => {
                    return Err(unsupported("spread", self.ast.expr(*e).span));
                }
            }
        }
        Ok((first, n))
    }

    /// A chain of member accesses, calls and optional chains, walked
    /// from its base up: the parser builds it in a loop, so it is as
    /// long as the source makes it.
    fn chain(&mut self, id: ExprId, mode: ChainMode) -> R {
        // The links, top first, and the base.
        let mut links = Vec::new();
        let mut e = id;
        loop {
            match &self.ast.expr(e).kind {
                Expr::Member { object, .. } => {
                    links.push(e);
                    e = *object;
                }
                Expr::Call { callee, .. } => {
                    links.push(e);
                    // `(a.b)()` calls with `a` as this.
                    let inner = self.unparen(*callee);
                    e = if matches!(self.ast.expr(inner).kind, Expr::Member { .. }) {
                        inner
                    } else {
                        *callee
                    };
                }
                Expr::OptionalChain(inner) => {
                    links.push(e);
                    e = *inner;
                }
                _ => break,
            }
        }
        // Where each link's `?.` goes: the nearest optional chain above.
        let mut targets: Vec<Option<Label>> = Vec::with_capacity(links.len());
        let mut chain_ends: Vec<(usize, Label)> = Vec::new();
        let mut current = None;
        for (i, link) in links.iter().enumerate() {
            if matches!(self.ast.expr(*link).kind, Expr::OptionalChain(_)) {
                let l = self.new_label();
                chain_ends.push((i, l));
                current = Some(l);
            }
            targets.push(current);
        }
        // The topmost member access, which a delete deletes.
        let delete_at = links
            .iter()
            .position(|l| !matches!(self.ast.expr(*l).kind, Expr::OptionalChain(_)));
        self.expr(e)?;
        let mut i = links.len();
        while i > 0 {
            i -= 1;
            let Some(&link) = links.get(i) else { break };
            let node = self.ast.expr(link);
            let target = targets.get(i).copied().flatten();
            match &node.kind {
                Expr::OptionalChain(_) => {
                    if let Some((_, l)) = chain_ends.iter().find(|(at, _)| *at == i) {
                        let done = self.new_label();
                        self.jump(Op::Jump, done);
                        self.bind(*l);
                        self.op(if mode == ChainMode::Delete {
                            Op::LdaTrue
                        } else {
                            Op::LdaUndefined
                        });
                        self.bind(done);
                    }
                }
                Expr::Member { prop, optional, .. } => {
                    if *optional && let Some(t) = target {
                        self.jump(Op::JumpIfNullish, t);
                    }
                    // A method call: the object is the call's receiver.
                    let call_above = i > 0
                        && links
                            .get(i - 1)
                            .is_some_and(|up| match &self.ast.expr(*up).kind {
                                Expr::Call { callee, .. } => self.unparen(*callee) == link,
                                _ => false,
                            });
                    if call_above {
                        let Some(&up) = links.get(i - 1) else { break };
                        let Expr::Call {
                            args,
                            optional: call_optional,
                            callee,
                        } = &self.ast.expr(up).kind
                        else {
                            continue;
                        };
                        let recv = self.alloc();
                        self.reg_op(Op::Star, recv);
                        self.get(recv, prop, node.span)?;
                        if *call_optional && let Some(t) = targets.get(i - 1).copied().flatten() {
                            self.jump(Op::JumpIfNullish, t);
                        }
                        let f = self.alloc();
                        self.reg_op(Op::Star, f);
                        let (first, n) = self.args(args)?;
                        let ic = self.ic();
                        let at = self.stack_at(*callee);
                        self.pos_at(self.ast.expr(*callee).span, at);
                        self.emit(
                            Op::Call,
                            &[
                                i64::from(f),
                                i64::from(recv),
                                i64::from(first),
                                i64::from(n),
                                ic,
                            ],
                        );
                        self.free_n(first, n);
                        self.free(f);
                        self.free(recv);
                        i -= 1;
                        continue;
                    }
                    let o = self.alloc();
                    self.reg_op(Op::Star, o);
                    if mode == ChainMode::Delete && delete_at == Some(i) {
                        self.member_key(prop)?;
                        self.pos(node.span);
                        self.reg_op(Op::DeleteProperty, o);
                    } else {
                        self.get(o, prop, node.span)?;
                    }
                    self.free(o);
                }
                Expr::Call {
                    callee,
                    args,
                    optional,
                } => {
                    if *optional && let Some(t) = target {
                        self.jump(Op::JumpIfNullish, t);
                    }
                    let f = self.alloc();
                    self.reg_op(Op::Star, f);
                    let (first, n) = self.args(args)?;
                    let callee_node = self.ast.expr(*callee);
                    let at = self.stack_at(*callee);
                    self.pos_at(callee_node.span, at);
                    // `eval?.(x)` is never a direct eval.
                    let direct_eval = !*optional
                        && matches!(&callee_node.kind, Expr::Ident(i) if &*i.name == "eval");
                    if direct_eval {
                        self.emit(
                            Op::CallEval,
                            &[i64::from(f), i64::from(first), i64::from(n)],
                        );
                    } else {
                        let ic = self.ic();
                        self.emit(
                            Op::CallUndefined,
                            &[i64::from(f), i64::from(first), i64::from(n), ic],
                        );
                    }
                    self.free_n(first, n);
                    self.free(f);
                }
                _ => {}
            }
        }
        if mode == ChainMode::Delete && delete_at.is_none() {
            self.op(Op::LdaTrue);
        }
        Ok(())
    }

    fn assign(&mut self, op: AssignOp, target: AssignTarget, value: ExprId, span: Span) -> R {
        let AssignTarget::Simple(t) = target else {
            return Err(unsupported("destructuring", span));
        };
        let t = self.unparen(t);
        let tnode = self.ast.expr(t);
        match &tnode.kind {
            Expr::Ident(i) => {
                let name = &i.name;
                let inferred = units(name);
                match op {
                    AssignOp::Assign => {
                        self.expr_named(value, Some(&inferred))?;
                        self.store_name(name, tnode.span, false)?;
                    }
                    AssignOp::Binary(b) => {
                        self.load_name(name, tnode.span, false)?;
                        let r = self.alloc();
                        self.reg_op(Op::Star, r);
                        self.expr(value)?;
                        let (instr, _) = binary_op(b);
                        self.pos(span);
                        self.reg_op(instr, r);
                        self.free(r);
                        self.store_name(name, tnode.span, false)?;
                    }
                    AssignOp::Logical(l) => {
                        let end = self.new_label();
                        self.load_name(name, tnode.span, false)?;
                        self.jump(logical_jump(l), end);
                        self.expr_named(value, Some(&inferred))?;
                        self.store_name(name, tnode.span, false)?;
                        self.bind(end);
                    }
                }
            }
            Expr::Member {
                object,
                prop,
                optional: false,
            } => {
                self.expr(*object)?;
                let o = self.alloc();
                self.reg_op(Op::Star, o);
                let key = match prop {
                    MemberProp::Computed(k) => {
                        self.expr(*k)?;
                        let r = self.alloc();
                        self.reg_op(Op::Star, r);
                        Some(r)
                    }
                    MemberProp::Ident(_) => None,
                    MemberProp::Private(i) => return Err(unsupported("private names", i.span)),
                };
                let get = |g: &mut Self| -> R {
                    match (prop, key) {
                        (MemberProp::Ident(i), _) => {
                            let n = g.name(&i.name);
                            let ic = g.ic();
                            g.pos(tnode.span);
                            g.emit(Op::GetNamed, &[i64::from(o), n, ic]);
                        }
                        (_, Some(k)) => {
                            g.reg_op(Op::Ldar, k);
                            let ic = g.ic();
                            g.pos(tnode.span);
                            g.emit(Op::GetKeyed, &[i64::from(o), ic]);
                        }
                        _ => {}
                    }
                    Ok(())
                };
                match op {
                    AssignOp::Assign => {
                        self.expr(value)?;
                        self.set(o, prop, key, tnode.span);
                    }
                    AssignOp::Binary(b) => {
                        get(self)?;
                        let r = self.alloc();
                        self.reg_op(Op::Star, r);
                        self.expr(value)?;
                        let (instr, _) = binary_op(b);
                        self.pos(span);
                        self.reg_op(instr, r);
                        self.free(r);
                        self.set(o, prop, key, tnode.span);
                    }
                    AssignOp::Logical(l) => {
                        let end = self.new_label();
                        get(self)?;
                        self.jump(logical_jump(l), end);
                        self.expr(value)?;
                        self.set(o, prop, key, tnode.span);
                        self.bind(end);
                    }
                }
                if let Some(k) = key {
                    self.free(k);
                }
                self.free(o);
            }
            _ => {
                // `f() = 1`: run, then a ReferenceError (web compatible).
                self.expr(t)?;
                self.pos(tnode.span);
                self.op(Op::ThrowInvalidLhs);
            }
        }
        Ok(())
    }

    fn set(&mut self, o: u32, prop: &MemberProp, key: Option<u32>, span: Span) {
        let ic = self.ic();
        self.pos(span);
        match (prop, key) {
            (MemberProp::Ident(i), _) => {
                let n = self.name(&i.name);
                self.emit(Op::SetNamed, &[i64::from(o), n, ic]);
            }
            (_, Some(k)) => self.emit(Op::SetKeyed, &[i64::from(o), i64::from(k), ic]),
            _ => {}
        }
    }

    fn update(&mut self, op: UpdateOp, prefix: bool, target: ExprId, span: Span) -> R {
        let step = match op {
            UpdateOp::Increment => Op::Inc,
            UpdateOp::Decrement => Op::Dec,
        };
        let t = self.unparen(target);
        let tnode = self.ast.expr(t);
        match &tnode.kind {
            Expr::Ident(i) => {
                self.load_name(&i.name, tnode.span, false)?;
                self.pos(span);
                if prefix {
                    self.op(step);
                    self.store_name(&i.name, tnode.span, false)?;
                } else {
                    self.op(Op::ToNumeric);
                    let old = self.alloc();
                    self.reg_op(Op::Star, old);
                    self.op(step);
                    self.store_name(&i.name, tnode.span, false)?;
                    self.reg_op(Op::Ldar, old);
                    self.free(old);
                }
            }
            Expr::Member {
                object,
                prop,
                optional: false,
            } => {
                self.expr(*object)?;
                let o = self.alloc();
                self.reg_op(Op::Star, o);
                let key = match prop {
                    MemberProp::Computed(k) => {
                        self.expr(*k)?;
                        let r = self.alloc();
                        self.reg_op(Op::Star, r);
                        Some(r)
                    }
                    MemberProp::Ident(_) => None,
                    MemberProp::Private(i) => return Err(unsupported("private names", i.span)),
                };
                if let Some(k) = key {
                    self.reg_op(Op::Ldar, k);
                    let ic = self.ic();
                    self.pos(tnode.span);
                    self.emit(Op::GetKeyed, &[i64::from(o), ic]);
                } else {
                    self.get(o, prop, tnode.span)?;
                }
                self.pos(span);
                let old = if prefix {
                    None
                } else {
                    self.op(Op::ToNumeric);
                    let r = self.alloc();
                    self.reg_op(Op::Star, r);
                    Some(r)
                };
                self.op(step);
                self.set(o, prop, key, tnode.span);
                if let Some(r) = old {
                    self.reg_op(Op::Ldar, r);
                    self.free(r);
                }
                if let Some(k) = key {
                    self.free(k);
                }
                self.free(o);
            }
            _ => {
                self.expr(t)?;
                self.pos(tnode.span);
                self.op(Op::ThrowInvalidLhs);
            }
        }
        Ok(())
    }

    fn template(&mut self, quasis: &[crate::ast::Quasi], exprs: &[ExprId], span: Span) -> R {
        let cooked = |q: &crate::ast::Quasi| q.cooked.clone();
        let Some(first) = quasis.first().and_then(cooked) else {
            return Err(unsupported("tagged templates", span));
        };
        let c = self.string(&first);
        self.emit(Op::LdaConst, &[c]);
        for (i, e) in exprs.iter().enumerate() {
            let r = self.alloc();
            self.reg_op(Op::Star, r);
            self.expr(*e)?;
            self.pos(self.ast.expr(*e).span);
            self.op(Op::ToString);
            self.reg_op(Op::Add, r);
            if let Some(q) = quasis.get(i + 1).and_then(cooked)
                && !q.is_empty()
            {
                self.reg_op(Op::Star, r);
                let c = self.string(&q);
                self.emit(Op::LdaConst, &[c]);
                self.reg_op(Op::Add, r);
            }
            self.free(r);
        }
        Ok(())
    }

    fn array(&mut self, elems: &[Option<Spreadable>]) -> R {
        self.op(Op::CreateArray);
        let a = self.alloc();
        self.reg_op(Op::Star, a);
        for (i, el) in elems.iter().enumerate() {
            match el {
                None => {}
                Some(Spreadable::Expr(e)) => {
                    self.expr(*e)?;
                    self.emit(Op::DefineIndex, &[i64::from(a), i as i64]);
                }
                Some(Spreadable::Spread(e)) => {
                    return Err(unsupported("spread", self.ast.expr(*e).span));
                }
            }
        }
        if matches!(elems.last(), Some(None)) {
            self.emit(Op::SetArrayLength, &[i64::from(a), elems.len() as i64]);
        }
        self.reg_op(Op::Ldar, a);
        self.free(a);
        Ok(())
    }

    fn object(&mut self, props: &[crate::ast::Prop]) -> R {
        self.op(Op::CreateObject);
        let o = self.alloc();
        self.reg_op(Op::Star, o);
        for p in props {
            if p.kind == PropKind::Spread {
                self.expr(p.value)?;
                self.pos(p.span);
                self.reg_op(Op::CopyDataProperties, o);
                continue;
            }
            let prefix: i64 = match p.kind {
                PropKind::Getter => 1,
                PropKind::Setter => 2,
                _ => 0,
            };
            match static_key(&p.key) {
                Some(key) => {
                    let proto = p.kind == PropKind::Init && &*key == units("__proto__").as_ref();
                    if proto {
                        self.expr(p.value)?;
                        self.reg_op(Op::SetProtoLiteral, o);
                    } else if prefix == 0 {
                        self.expr_named(p.value, Some(&key))?;
                        let n = self.string(&key);
                        self.emit(Op::DefineField, &[i64::from(o), n]);
                    } else {
                        let mut name = units(if prefix == 1 { "get " } else { "set " }).into_vec();
                        name.extend_from_slice(&key);
                        let n = self.string(&key);
                        self.emit(Op::LdaConst, &[n]);
                        let k = self.alloc();
                        self.reg_op(Op::Star, k);
                        self.expr_named(p.value, Some(&name))?;
                        self.accessor(o, k, prefix);
                        self.free(k);
                    }
                }
                None => {
                    let PropKey::Computed(key) = p.key else {
                        return Err(unsupported("private names", p.span));
                    };
                    self.expr(key)?;
                    self.op(Op::ToPropertyKey);
                    let k = self.alloc();
                    self.reg_op(Op::Star, k);
                    self.expr(p.value)?;
                    if self.is_anonymous_function(p.value) {
                        self.emit(Op::SetFunctionName, &[i64::from(k), prefix]);
                    }
                    if prefix == 0 {
                        self.emit(Op::DefineKeyed, &[i64::from(o), i64::from(k)]);
                    } else {
                        self.accessor(o, k, prefix);
                    }
                    self.free(k);
                }
            }
        }
        self.reg_op(Op::Ldar, o);
        self.free(o);
        Ok(())
    }

    fn accessor(&mut self, o: u32, k: u32, prefix: i64) {
        let op = if prefix == 1 {
            Op::DefineGetter
        } else {
            Op::DefineSetter
        };
        self.emit(op, &[i64::from(o), i64::from(k)]);
    }
}

fn logical_jump(op: LogicalOp) -> Op {
    match op {
        LogicalOp::And => Op::JumpIfFalse,
        LogicalOp::Or => Op::JumpIfTrue,
        LogicalOp::Coalesce => Op::JumpIfNotNullish,
    }
}
