//! Modules (section 16.2): imports, exports and their early errors
//! (duplicate exports, exports of names the module does not declare,
//! module export names that are not well-formed Unicode).

use super::{Parser, utf16_text};
use crate::SyntaxError;
use crate::ast::{
    ExportDecl, ExportSpec, Ident, ImportAttribute, ImportDecl, ImportPhase, ImportSpec,
    ModuleString, Span, Stmt, StmtId,
};
use crate::lexer::{Keyword, Punct, TokenKind};

/// What the module's exports name, checked at its end.
#[derive(Default)]
struct Exports {
    /// Every exported name, for duplicates.
    names: Vec<(ModuleString, Span)>,
    /// `export { x }` without `from`: names the module must declare.
    locals: Vec<Ident>,
}

fn utf16(s: &str) -> ModuleString {
    s.encode_utf16().collect()
}

impl Parser<'_> {
    /// A module's items (section 16.2.1).
    pub(super) fn module_body(&mut self) -> Result<Vec<StmtId>, SyntaxError> {
        let mut body = Vec::new();
        let mut exports = Exports::default();
        while !self.at_eof() {
            let item = if self.at_keyword(Keyword::Import) && self.import_is_declaration()? {
                self.import_declaration()?
            } else if self.at_keyword(Keyword::Export) {
                self.export_declaration(&mut exports)?
            } else {
                self.statement_list_item(true)?
            };
            body.push(item);
        }
        for (i, (name, span)) in exports.names.iter().enumerate() {
            if exports.names.iter().take(i).any(|(n, _)| n == name) {
                return Err(
                    self.error(format!("Duplicate export of '{}'", utf16_text(name)), *span)
                );
            }
        }
        for local in &exports.locals {
            if !self.scopes.top_declares(&local.name) {
                return Err(self.error(
                    format!("Export '{}' is not defined in module", local.name),
                    local.span,
                ));
            }
        }
        Ok(body)
    }

    /// A string literal naming a module.
    fn module_specifier(&mut self) -> Result<ModuleString, SyntaxError> {
        if self.tok.kind != TokenKind::String {
            return Err(self.unexpected());
        }
        let tok = self.advance()?;
        Ok(Self::string_value(&tok))
    }

    /// `from "specifier"`.
    fn source_clause(&mut self) -> Result<ModuleString, SyntaxError> {
        if !self.at_contextual("from") {
            return Err(self.unexpected());
        }
        self.advance()?;
        self.module_specifier()
    }

    /// `with { type: "json" }`, if there.
    fn import_attributes(&mut self) -> Result<Vec<ImportAttribute>, SyntaxError> {
        let mut attributes: Vec<ImportAttribute> = Vec::new();
        if !self.eat_keyword(Keyword::With)? {
            return Ok(attributes);
        }
        self.expect(Punct::LBrace)?;
        while !self.at(Punct::RBrace) {
            let start = self.tok.start;
            let key = match self.tok.kind {
                TokenKind::String => {
                    let tok = self.advance()?;
                    Self::string_value(&tok)
                }
                _ if self.at_identifier_name() => utf16(&self.identifier_name()?.name),
                _ => return Err(self.unexpected()),
            };
            self.expect(Punct::Colon)?;
            let value = self.module_specifier()?;
            let span = Span::new(start, self.prev_end);
            if attributes.iter().any(|a| a.key == key) {
                return Err(self.error(
                    format!("Import attribute has duplicate key '{}'", utf16_text(&key)),
                    span,
                ));
            }
            attributes.push(ImportAttribute { key, value, span });
            if !self.at(Punct::RBrace) {
                self.expect(Punct::Comma)?;
            }
        }
        self.expect(Punct::RBrace)?;
        Ok(attributes)
    }

    /// A ModuleExportName: an IdentifierName or a well-formed string.
    fn module_export_name(&mut self) -> Result<(ModuleString, Span, bool), SyntaxError> {
        let span = self.tok_span();
        if self.tok.kind == TokenKind::String {
            let tok = self.advance()?;
            let value = Self::string_value(&tok);
            if char::decode_utf16(value.iter().copied()).any(|c| c.is_err()) {
                return Err(self.error(
                    "Invalid module export name: contains unpaired surrogate",
                    span,
                ));
            }
            return Ok((value, span, true));
        }
        if !self.at_identifier_name() {
            return Err(self.unexpected());
        }
        let name = self.identifier_name()?;
        Ok((utf16(&name.name), span, false))
    }

    /// A local binding of an import, declared in the module's scope.
    fn imported_binding(&mut self) -> Result<Ident, SyntaxError> {
        let ident = self.identifier(true)?;
        self.scopes.declare_lexical(&ident.name, ident.span)?;
        Ok(ident)
    }

    fn import_declaration(&mut self) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        self.advance()?;
        if self.tok.kind == TokenKind::String {
            let source = self.module_specifier()?;
            let attributes = self.import_attributes()?;
            self.semicolon()?;
            return Ok(self.ast.add_stmt(
                Stmt::Import(ImportDecl {
                    specs: Vec::new(),
                    source,
                    attributes,
                    phase: ImportPhase::Evaluation,
                }),
                Span::new(start, self.prev_end),
            ));
        }
        let mut specs = Vec::new();
        let mut needs_more = true;
        if self.at_identifier() {
            specs.push(ImportSpec::Default(self.imported_binding()?));
            needs_more = self.eat(Punct::Comma)?;
        }
        if needs_more {
            if self.eat(Punct::Star)? {
                if !self.at_contextual("as") {
                    return Err(self.unexpected());
                }
                self.advance()?;
                specs.push(ImportSpec::Namespace(self.imported_binding()?));
            } else if self.eat(Punct::LBrace)? {
                while !self.at(Punct::RBrace) {
                    let name_tok = self.tok.clone();
                    let (name, _, is_string) = self.module_export_name()?;
                    let local = if self.at_contextual("as") {
                        self.advance()?;
                        self.imported_binding()?
                    } else {
                        if is_string || name_tok.kind != TokenKind::Identifier {
                            return Err(self.unexpected_token(&name_tok));
                        }
                        let ident = Ident {
                            name: name_tok.name(self.source).into(),
                            span: Span::new(name_tok.start, name_tok.end),
                        };
                        self.check_identifier(&ident.name, ident.span, true)?;
                        self.scopes.declare_lexical(&ident.name, ident.span)?;
                        ident
                    };
                    specs.push(ImportSpec::Named { name, local });
                    if !self.at(Punct::RBrace) {
                        self.expect(Punct::Comma)?;
                    }
                }
                self.expect(Punct::RBrace)?;
            } else {
                return Err(self.unexpected());
            }
        }
        let source = self.source_clause()?;
        let attributes = self.import_attributes()?;
        self.semicolon()?;
        Ok(self.ast.add_stmt(
            Stmt::Import(ImportDecl {
                specs,
                source,
                attributes,
                phase: ImportPhase::Evaluation,
            }),
            Span::new(start, self.prev_end),
        ))
    }

    fn export_declaration(&mut self, exports: &mut Exports) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        self.advance()?;
        let decl = match self.tok.kind {
            TokenKind::Punct(Punct::Star) => {
                self.advance()?;
                let exported = if self.at_contextual("as") {
                    self.advance()?;
                    let (name, span, _) = self.module_export_name()?;
                    exports.names.push((name.clone(), span));
                    Some(name)
                } else {
                    None
                };
                let source = self.source_clause()?;
                let attributes = self.import_attributes()?;
                self.semicolon()?;
                ExportDecl::All {
                    exported,
                    source,
                    attributes,
                }
            }
            TokenKind::Punct(Punct::LBrace) => self.export_named(exports)?,
            TokenKind::Keyword(Keyword::Default) => {
                exports.names.push((utf16("default"), self.tok_span()));
                self.advance()?;
                self.export_default()?
            }
            _ => {
                let stmt = match self.tok.kind {
                    TokenKind::Keyword(
                        Keyword::Var | Keyword::Const | Keyword::Function | Keyword::Class,
                    ) => self.statement_list_item(true)?,
                    TokenKind::Identifier
                        if (self.at_contextual("let") && self.let_declares()?)
                            || (self.at_contextual("async") && self.async_function_next()?) =>
                    {
                        self.statement_list_item(true)?
                    }
                    _ => return Err(self.unexpected()),
                };
                for (name, span) in self.declared_names(stmt) {
                    exports.names.push((utf16(&name), span));
                }
                ExportDecl::Decl(stmt)
            }
        };
        Ok(self
            .ast
            .add_stmt(Stmt::Export(decl), Span::new(start, self.prev_end)))
    }

    /// The names a declaration statement binds.
    fn declared_names(&self, stmt: StmtId) -> Vec<(Box<str>, Span)> {
        match &self.ast.stmt(stmt).kind {
            Stmt::Var(decl) => decl
                .decls
                .iter()
                .flat_map(|d| self.bound_names(d.target))
                .map(|i| (i.name, i.span))
                .collect(),
            Stmt::Function(f) => self
                .ast
                .func(*f)
                .name
                .iter()
                .map(|i| (i.name.clone(), i.span))
                .collect(),
            Stmt::Class(c) => self
                .ast
                .class(*c)
                .name
                .iter()
                .map(|i| (i.name.clone(), i.span))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `export { a, b as c } [from "m"];`
    fn export_named(&mut self, exports: &mut Exports) -> Result<ExportDecl, SyntaxError> {
        self.advance()?;
        // (local, its token's kind was an identifier, exported)
        let mut specs = Vec::new();
        let mut locals = Vec::new();
        while !self.at(Punct::RBrace) {
            let start = self.tok.start;
            let local_tok = self.tok.clone();
            let (local, local_span, local_is_string) = self.module_export_name()?;
            let (exported, exported_span) = if self.at_contextual("as") {
                self.advance()?;
                let (name, span, _) = self.module_export_name()?;
                (name, span)
            } else {
                (local.clone(), local_span)
            };
            locals.push((local_tok, local_is_string));
            specs.push(ExportSpec {
                local,
                exported: exported.clone(),
                span: Span::new(start, self.prev_end),
            });
            exports.names.push((exported, exported_span));
            if !self.at(Punct::RBrace) {
                self.expect(Punct::Comma)?;
            }
        }
        self.expect(Punct::RBrace)?;
        let source = if self.at_contextual("from") {
            Some(self.source_clause()?)
        } else {
            None
        };
        let attributes = if source.is_some() {
            self.import_attributes()?
        } else {
            Vec::new()
        };
        if source.is_none() {
            // Local names must be references the module can bind.
            for (tok, is_string) in locals {
                let span = Span::new(tok.start, tok.end);
                if is_string || tok.kind != TokenKind::Identifier {
                    return Err(self.unexpected_token(&tok));
                }
                let name: Box<str> = tok.name(self.source).into();
                self.check_identifier(&name, span, false)?;
                exports.locals.push(Ident { name, span });
            }
        }
        self.semicolon()?;
        Ok(ExportDecl::Named {
            specs,
            source,
            attributes,
        })
    }

    /// What follows `export default`.
    fn export_default(&mut self) -> Result<ExportDecl, SyntaxError> {
        if self.at_keyword(Keyword::Function) {
            let stmt = self.function_declaration_named(false, false, true)?;
            return Ok(ExportDecl::DefaultDecl(stmt));
        }
        if self.at_contextual("async") && self.async_function_next()? {
            let stmt = self.function_declaration_named(true, false, true)?;
            return Ok(ExportDecl::DefaultDecl(stmt));
        }
        if self.at_keyword(Keyword::Class) {
            let stmt = self.class_declaration(true)?;
            return Ok(ExportDecl::DefaultDecl(stmt));
        }
        let e = self.with_context(|cx| cx.in_ = true, |p| p.assignment())?;
        self.semicolon()?;
        Ok(ExportDecl::DefaultExpr(e))
    }
}
