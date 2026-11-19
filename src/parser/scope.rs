//! The declarations of each scope being parsed, for the early errors
//! that compare them (sections 8.2's LexicallyDeclaredNames and
//! VarDeclaredNames as the rules of sections 14 to 16 use them, and
//! Annex B.3's relaxations for sloppy code).

use std::collections::HashSet;

use crate::SyntaxError;
use crate::ast::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeKind {
    Script,
    Module,
    /// A function's parameters and body, or a class static block.
    Function,
    Block,
    /// A catch clause's parameter and block; `simple` when the
    /// parameter is one identifier (Annex B.3.4 lets a `var` redeclare
    /// it).
    Catch {
        simple: bool,
    },
}

#[derive(Debug)]
struct Scope {
    kind: ScopeKind,
    lexical: HashSet<Box<str>>,
    /// Names declared by `var` here or in a block within, and functions
    /// declared at a function's or script's top level.
    vars: HashSet<Box<str>>,
    /// A function's parameters, or a catch clause's.
    params: HashSet<Box<str>>,
    /// Plain functions declared in this block in sloppy code, which
    /// Annex B.3.2.4 lets be declared twice.
    sloppy_functions: HashSet<Box<str>>,
}

#[derive(Debug)]
pub(crate) struct Scopes {
    stack: Vec<Scope>,
}

fn redeclared(name: &str, span: Span) -> SyntaxError {
    SyntaxError {
        message: format!("Identifier '{name}' has already been declared"),
        start: span.start,
        end: span.end,
    }
}

impl Scopes {
    pub(crate) fn new(module: bool) -> Scopes {
        let mut scopes = Scopes { stack: Vec::new() };
        scopes.push(if module {
            ScopeKind::Module
        } else {
            ScopeKind::Script
        });
        scopes
    }

    pub(crate) fn push(&mut self, kind: ScopeKind) {
        self.stack.push(Scope {
            kind,
            lexical: HashSet::new(),
            vars: HashSet::new(),
            params: HashSet::new(),
            sloppy_functions: HashSet::new(),
        });
    }

    pub(crate) fn pop(&mut self) {
        // The top scope stays: it is popped by nothing.
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    pub(crate) fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Nothing is left to check at the end; kept as the place for it.
    pub(crate) fn finish_top(&mut self) -> Result<(), SyntaxError> {
        Ok(())
    }

    /// Whether the top scope declares `name` (a module's exports name
    /// only what it declares, section 16.2.1.1).
    pub(crate) fn top_declares(&self, name: &str) -> bool {
        self.stack
            .first()
            .is_some_and(|s| s.lexical.contains(name) || s.vars.contains(name))
    }

    fn current(&mut self) -> Option<&mut Scope> {
        self.stack.last_mut()
    }

    /// A parameter of the current function or catch clause; false when
    /// the name was already one (whether that is an error is the
    /// caller's to say).
    pub(crate) fn declare_param(&mut self, name: &str) -> bool {
        match self.current() {
            Some(scope) => scope.params.insert(name.into()),
            None => true,
        }
    }

    /// A `let`, `const`, `using` or `class` declaration.
    pub(crate) fn declare_lexical(&mut self, name: &str, span: Span) -> Result<(), SyntaxError> {
        let Some(scope) = self.current() else {
            return Ok(());
        };
        let clashes_param = matches!(scope.kind, ScopeKind::Function | ScopeKind::Catch { .. })
            && scope.params.contains(name);
        if scope.lexical.contains(name) || scope.vars.contains(name) || clashes_param {
            return Err(redeclared(name, span));
        }
        scope.lexical.insert(name.into());
        Ok(())
    }

    /// A `var` declaration: hoisted through every block to the function,
    /// clashing with any lexical declaration of the name on the way, and
    /// with a catch parameter unless it is one plain name (Annex B.3.4,
    /// which since ES2023 relaxes `for (var e of ...)` too).
    pub(crate) fn declare_var(&mut self, name: &str, span: Span) -> Result<(), SyntaxError> {
        for scope in self.stack.iter_mut().rev() {
            if scope.lexical.contains(name) {
                return Err(redeclared(name, span));
            }
            if let ScopeKind::Catch { simple } = scope.kind
                && scope.params.contains(name)
                && !simple
            {
                return Err(redeclared(name, span));
            }
            scope.vars.insert(name.into());
            if matches!(
                scope.kind,
                ScopeKind::Function | ScopeKind::Script | ScopeKind::Module
            ) {
                break;
            }
        }
        Ok(())
    }

    /// A function declaration: var-like at a function's or script's top
    /// level, lexical in a block or at a module's top level, where sloppy
    /// code may declare a plain function twice (Annex B.3.2.4).
    pub(crate) fn declare_function(
        &mut self,
        name: &str,
        span: Span,
        sloppy_plain: bool,
    ) -> Result<(), SyntaxError> {
        let Some(scope) = self.current() else {
            return Ok(());
        };
        match scope.kind {
            ScopeKind::Function | ScopeKind::Script => {
                if scope.lexical.contains(name) {
                    return Err(redeclared(name, span));
                }
                scope.vars.insert(name.into());
            }
            ScopeKind::Module | ScopeKind::Block | ScopeKind::Catch { .. } => {
                let twice_allowed = sloppy_plain && scope.sloppy_functions.contains(name);
                let clashes_param =
                    matches!(scope.kind, ScopeKind::Catch { .. }) && scope.params.contains(name);
                if (scope.lexical.contains(name) && !twice_allowed)
                    || scope.vars.contains(name)
                    || clashes_param
                {
                    return Err(redeclared(name, span));
                }
                scope.lexical.insert(name.into());
                if sloppy_plain {
                    scope.sloppy_functions.insert(name.into());
                } else {
                    // A later plain function of the name is no longer the
                    // second of two plain ones.
                    scope.sloppy_functions.remove(name);
                }
            }
        }
        Ok(())
    }
}
