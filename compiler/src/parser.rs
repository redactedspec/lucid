//! Recursive-descent parser for docs/grammar.md. Each method is named after
//! the grammar rule it parses.

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::lexer::{MAX_DEPTH, TextPart as RawPart, Token, TokenKind};

/// An `Err` means a diagnostic is already recorded.
type Parsed<T> = Result<T, ()>;

pub fn parse(tokens: Vec<Token>) -> (File, Vec<Diagnostic>) {
    let mut parser = Parser { tokens, pos: 0, depth: 0, links: 0, diags: Vec::new() };
    let mut stmts = Vec::new();
    while *parser.kind() != TokenKind::Eof {
        parser.recover(&mut stmts, |p| p.statement(true));
    }
    (File { stmts }, parser.diags)
}

struct Parser {
    /// Ends with `Eof`, or with `}` while parsing an interpolation.
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    /// Operators and postfix forms so far in this item. See `link`.
    links: usize,
    diags: Vec<Diagnostic>,
}

/// Longest allowed chain of operators and postfix forms in one item.
const MAX_LINKS: usize = 1024;

const TOP_LEVEL_ONLY: &[&str] = &["use", "test", "public", "type", "choice", "trait"];

impl Parser {
    fn token(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn kind(&self) -> &TokenKind {
        &self.token().kind
    }

    fn kind_at(&self, offset: usize) -> &TokenKind {
        &self.tokens[(self.pos + offset).min(self.tokens.len() - 1)].kind
    }

    fn at(&self, sym: &str) -> bool {
        matches!(self.kind(), TokenKind::Sym(s) if *s == sym)
    }

    fn at_any(&self, syms: &[&str]) -> bool {
        syms.iter().any(|s| self.at(s))
    }

    fn start(&self) -> usize {
        self.token().span.start
    }

    /// The span from `start` to the end of the last consumed token.
    fn since(&self, start: usize) -> Span {
        let end = self.tokens[self.pos.min(self.tokens.len() - 1).saturating_sub(1)].span.end;
        Span { start, end: end.max(start) }
    }

    fn advance(&mut self) -> Span {
        let span = self.token().span;
        self.pos = (self.pos + 1).min(self.tokens.len() - 1);
        span
    }

    fn eat(&mut self, sym: &str) -> bool {
        let found = self.at(sym);
        if found {
            self.advance();
        }
        found
    }

    fn error<T>(
        &mut self,
        code: &'static str,
        message: impl Into<String>,
        span: Span,
    ) -> Parsed<T> {
        self.diags.push(Diagnostic::new(code, message, span));
        Err(())
    }

    fn expected<T>(&mut self, what: &str) -> Parsed<T> {
        let token = self.token();
        let (message, span) =
            (format!("expected {what}, found {}", token.kind.describe()), token.span);
        self.error("E0201", message, span)
    }

    fn expect(&mut self, sym: &str) -> Parsed<()> {
        if self.eat(sym) { Ok(()) } else { self.expected(&format!("`{sym}`")) }
    }

    fn expect_kind(&mut self, kind: TokenKind) -> Parsed<()> {
        if *self.kind() == kind {
            self.advance();
            Ok(())
        } else {
            self.expected(&kind.describe())
        }
    }

    /// Ends a parenthesized expression or type, where a `,` means a tuple.
    fn close_group(&mut self) -> Parsed<()> {
        if self.at(",") {
            let span = self.token().span;
            let note = "there are no tuples: use a `type` with named fields";
            self.diags.push(Diagnostic::new("E0201", "expected `)`, found `,`", span).note(note));
            return Err(());
        }
        self.expect(")")
    }

    fn newline(&mut self) -> Parsed<()> {
        self.expect_kind(TokenKind::Newline)
    }

    fn name(&mut self) -> Parsed<Name> {
        if let TokenKind::Name(text) = self.kind() {
            let text = text.clone();
            let span = self.advance();
            Ok(Name { text, span })
        } else if let TokenKind::Sym(word) = self.kind()
            && crate::lexer::KEYWORDS.contains(word)
        {
            let token = self.token();
            let message = format!("expected a name, found `{word}`");
            let note = format!("`{word}` is a keyword, so pick another name");
            self.diags.push(Diagnostic::new("E0201", message, token.span).note(note));
            Err(())
        } else {
            self.expected("a name")
        }
    }

    /// Runs `parse` one level deeper, and fails past `MAX_DEPTH`. Every
    /// recursive rule goes through here, so that source cannot overflow the stack.
    fn nested<T>(&mut self, parse: impl FnOnce(&mut Self) -> Parsed<T>) -> Parsed<T> {
        if self.depth == MAX_DEPTH {
            let span = self.token().span;
            return self.error("E0205", "nesting is too deep", span);
        }
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    /// Counts one link of a chain such as `a + a + ...`. Chains are loops, not
    /// recursion, but they still build a deep tree that later stages recurse over.
    fn link(&mut self) -> Parsed<()> {
        self.links += 1;
        if self.links > MAX_LINKS {
            let span = self.token().span;
            return self.error("E0205", format!("expression has over {MAX_LINKS} operators"), span);
        }
        Ok(())
    }

    /// Parses one item into `out`. After an error, skips to the next item.
    fn recover<T>(&mut self, out: &mut Vec<T>, parse: impl FnOnce(&mut Self) -> Parsed<T>) {
        let before = self.pos;
        self.links = 0;
        match parse(self) {
            Ok(item) => out.push(item),
            Err(()) => {
                self.synchronize();
                if self.pos == before {
                    self.advance();
                }
            }
        }
    }

    /// Skips the rest of the line, and any block under it.
    fn synchronize(&mut self) {
        let mut depth = 0;
        loop {
            match self.kind() {
                TokenKind::Eof => return,
                TokenKind::Indent => depth += 1,
                TokenKind::Dedent if depth == 0 => return,
                TokenKind::Dedent => {
                    depth -= 1;
                    if depth == 0 {
                        self.advance();
                        return;
                    }
                }
                TokenKind::Newline if depth == 0 => {
                    self.advance();
                    if *self.kind() != TokenKind::Indent {
                        return;
                    }
                    continue;
                }
                _ => {}
            }
            self.advance();
        }
    }

    /// Parses `INDENT { item } DEDENT`.
    fn indented<T>(&mut self, mut item: impl FnMut(&mut Self) -> Parsed<T>) -> Parsed<Vec<T>> {
        if self.expect_kind(TokenKind::Indent).is_err() {
            if let Some(last) = self.diags.last_mut() {
                last.notes.push("indent the block by 4 spaces".into());
            }
            return Err(());
        }
        let mut items = Vec::new();
        while !matches!(self.kind(), TokenKind::Dedent | TokenKind::Eof) {
            self.recover(&mut items, &mut item);
        }
        self.expect_kind(TokenKind::Dedent)?;
        Ok(items)
    }

    fn block(&mut self) -> Parsed<Vec<Stmt>> {
        self.indented(|p| p.statement(false))
    }

    fn statement(&mut self, top: bool) -> Parsed<Stmt> {
        self.nested(|p| {
            let start = p.start();
            let kind = p.statement_kind(top)?;
            Ok(Stmt { kind, span: p.since(start) })
        })
    }

    fn statement_kind(&mut self, top: bool) -> Parsed<StmtKind> {
        let sym = match self.kind() {
            TokenKind::Sym(sym) => *sym,
            _ => "",
        };
        if !top && TOP_LEVEL_ONLY.contains(&sym) {
            let span = self.token().span;
            return self.error("E0206", format!("`{sym}` is only allowed at top level"), span);
        }
        Ok(match sym {
            "use" => self.use_decl()?,
            "test" => self.test_def()?,
            "public" => {
                self.advance();
                self.declaration(true)?
            }
            "type" | "choice" | "trait" | "task" => self.declaration(false)?,
            "unsafe" if *self.kind_at(1) == TokenKind::Sym("extern") => self.declaration(false)?,
            "unsafe" => {
                self.advance();
                self.newline()?;
                StmtKind::Unsafe(self.block()?)
            }
            "let" | "var" => self.let_stmt()?,
            "if" => self.if_stmt()?,
            "while" => {
                self.advance();
                let condition = self.expression()?;
                self.newline()?;
                StmtKind::While { condition, body: self.block()? }
            }
            "for" => self.for_stmt()?,
            "match" => StmtKind::Expr(self.match_expr(false)?),
            "return" => self.return_stmt()?,
            "print" => self.print_stmt()?,
            "expect" => {
                self.advance();
                let value = self.expression()?;
                self.newline()?;
                StmtKind::Expect(value)
            }
            "break" | "continue" => {
                self.advance();
                self.newline()?;
                if sym == "break" { StmtKind::Break } else { StmtKind::Continue }
            }
            "..." => {
                self.advance();
                self.newline()?;
                StmtKind::Placeholder
            }
            _ => self.expr_stmt()?,
        })
    }

    fn declaration(&mut self, public: bool) -> Parsed<StmtKind> {
        if self.at("type") {
            self.type_def(public)
        } else if self.at("choice") {
            self.choice_def(public)
        } else if self.at("trait") {
            self.trait_def(public)
        } else if self.at_any(&["task", "unsafe"]) {
            self.task_def(public)
        } else {
            self.expected("a declaration")
        }
    }

    fn use_decl(&mut self) -> Parsed<StmtKind> {
        self.advance();
        let relative = self.eat("./");
        let mut path = vec![self.name()?];
        while self.eat(".") {
            path.push(self.name()?);
        }
        let alias = if self.eat("as") { Some(self.name()?) } else { None };
        self.newline()?;
        Ok(StmtKind::Use { relative, path, alias })
    }

    fn test_def(&mut self) -> Parsed<StmtKind> {
        self.advance();
        if !matches!(self.kind(), TokenKind::Text(_)) {
            return self.expected("text");
        }
        let name = self.primary()?;
        self.newline()?;
        Ok(StmtKind::Test { name, body: self.block()? })
    }

    fn task_def(&mut self, public: bool) -> Parsed<StmtKind> {
        if self.eat("unsafe") {
            self.expect("extern")?;
            let sig = self.task_sig()?;
            self.newline()?;
            return Ok(StmtKind::Task(TaskDef { public, sig, body: None }));
        }
        let sig = self.task_sig()?;
        self.newline()?;
        let body = Some(self.block()?);
        Ok(StmtKind::Task(TaskDef { public, sig, body }))
    }

    fn task_sig(&mut self) -> Parsed<TaskSig> {
        self.expect("task")?;
        let name = self.name()?;
        let type_params = self.type_params()?;
        self.expect("(")?;
        let params = self.params()?;
        self.expect(")")?;
        let returns = if self.eat("->") { Some(self.ty()?) } else { None };
        Ok(TaskSig { name, type_params, params, returns })
    }

    fn type_params(&mut self) -> Parsed<Vec<TypeParam>> {
        if !self.eat("[") {
            return Ok(Vec::new());
        }
        let mut params = Vec::new();
        loop {
            let name = self.name()?;
            let mut bounds = Vec::new();
            if self.eat(":") {
                bounds.push(self.ty()?);
                while self.eat("and") {
                    bounds.push(self.ty()?);
                }
            }
            params.push(TypeParam { name, bounds });
            if !self.eat(",") || self.at("]") {
                break;
            }
        }
        self.expect("]")?;
        Ok(params)
    }

    /// Parses `[ params ]`. The closing `)` stays for the caller.
    fn params(&mut self) -> Parsed<Vec<Param>> {
        let mut params = Vec::new();
        while !self.at(")") {
            params.push(self.param()?);
            if !self.eat(",") {
                break;
            }
        }
        Ok(params)
    }

    fn param(&mut self) -> Parsed<Param> {
        let mode = if self.eat("change") {
            ParamMode::Change
        } else if self.eat("take") {
            ParamMode::Take
        } else {
            ParamMode::Read
        };
        if self.at("self") {
            let span = self.advance();
            let ty = if self.eat(":") { Some(self.ty()?) } else { None };
            let name = Name { text: "self".into(), span };
            return Ok(Param { mode, name, ty, default: None });
        }
        let name = self.name()?;
        self.expect(":")?;
        let ty = Some(self.ty()?);
        let default = if self.eat("=") { Some(self.expression()?) } else { None };
        Ok(Param { mode, name, ty, default })
    }

    fn type_def(&mut self, public: bool) -> Parsed<StmtKind> {
        self.advance();
        let name = self.name()?;
        let type_params = self.type_params()?;
        let body = if self.eat("=") {
            let alias = self.ty()?;
            self.newline()?;
            TypeBody::Alias(alias)
        } else {
            self.newline()?;
            TypeBody::Fields(self.indented(Self::field)?)
        };
        Ok(StmtKind::Type(TypeDef { public, name, type_params, body }))
    }

    fn field(&mut self) -> Parsed<Field> {
        let name = self.name()?;
        self.expect(":")?;
        let ty = self.ty()?;
        let default = if self.eat("=") { Some(self.expression()?) } else { None };
        self.newline()?;
        Ok(Field { name, ty, default })
    }

    fn choice_def(&mut self, public: bool) -> Parsed<StmtKind> {
        self.advance();
        let name = self.name()?;
        let type_params = self.type_params()?;
        self.newline()?;
        let variants = self.indented(Self::variant)?;
        Ok(StmtKind::Choice(ChoiceDef { public, name, type_params, variants }))
    }

    fn variant(&mut self) -> Parsed<Variant> {
        let name = self.name()?;
        let mut params = Vec::new();
        if self.eat("(") {
            params = self.params()?;
            self.expect(")")?;
        }
        self.newline()?;
        Ok(Variant { name, params })
    }

    fn trait_def(&mut self, public: bool) -> Parsed<StmtKind> {
        self.advance();
        let name = self.name()?;
        let type_params = self.type_params()?;
        self.newline()?;
        let tasks = self.indented(|p| {
            let sig = p.task_sig()?;
            p.newline()?;
            Ok(sig)
        })?;
        Ok(StmtKind::Trait(TraitDef { public, name, type_params, tasks }))
    }

    fn ty(&mut self) -> Parsed<Type> {
        self.nested(|p| {
            let start = p.start();
            let first = p.type_term()?;
            if !p.at("or") {
                return Ok(first);
            }
            let mut terms = vec![first];
            while p.eat("or") {
                terms.push(p.type_term()?);
            }
            Ok(Type { kind: TypeKind::Union(terms), span: p.since(start) })
        })
    }

    fn type_term(&mut self) -> Parsed<Type> {
        let start = self.start();
        let inner = self.type_prefix()?;
        if self.eat("?") {
            let kind = TypeKind::Optional(Box::new(inner));
            return Ok(Type { kind, span: self.since(start) });
        }
        Ok(inner)
    }

    fn type_prefix(&mut self) -> Parsed<Type> {
        let start = self.start();
        if !self.at_any(&["shared", "weak", "any"]) {
            return self.type_atom();
        }
        self.nested(|p| {
            let wrap = match p.kind() {
                TokenKind::Sym("shared") => TypeKind::Shared,
                TokenKind::Sym("weak") => TypeKind::Weak,
                _ => TypeKind::Any,
            };
            p.advance();
            let inner = p.type_prefix()?;
            Ok(Type { kind: wrap(Box::new(inner)), span: p.since(start) })
        })
    }

    fn type_atom(&mut self) -> Parsed<Type> {
        let start = self.start();
        let kind = if self.eat("none") {
            TypeKind::None
        } else if self.eat("task") {
            self.expect("(")?;
            let mut params = Vec::new();
            if !self.at(")") {
                params.push(self.ty()?);
                while self.eat(",") {
                    params.push(self.ty()?);
                }
            }
            self.expect(")")?;
            let returns = if self.eat("->") { Some(Box::new(self.ty()?)) } else { None };
            TypeKind::Task { params, returns }
        } else if self.eat("(") {
            let inner = self.ty()?;
            self.close_group()?;
            return Ok(inner);
        } else if matches!(self.kind(), TokenKind::Name(_)) {
            let mut path = vec![self.name()?];
            while self.eat(".") {
                path.push(self.name()?);
            }
            let mut args = Vec::new();
            if self.eat("[") {
                loop {
                    args.push(self.ty()?);
                    if !self.eat(",") || self.at("]") {
                        break;
                    }
                }
                self.expect("]")?;
            }
            TypeKind::Named { path, args }
        } else {
            return self.expected("a type");
        };
        Ok(Type { kind, span: self.since(start) })
    }

    fn let_stmt(&mut self) -> Parsed<StmtKind> {
        let mutable = self.at("var");
        self.advance();
        let name = self.name()?;
        let ty = if self.eat(":") { Some(self.ty()?) } else { None };
        self.expect("=")?;
        let value = self.value()?;
        Ok(StmtKind::Let { mutable, name, ty, value })
    }

    fn value(&mut self) -> Parsed<Expr> {
        if self.at("match") {
            return self.match_expr(true);
        }
        let value = self.expression()?;
        self.newline()?;
        Ok(value)
    }

    fn if_stmt(&mut self) -> Parsed<StmtKind> {
        self.advance();
        let mut branches = vec![self.branch()?];
        let mut otherwise = None;
        while self.eat("else") {
            if self.eat("if") {
                branches.push(self.branch()?);
            } else {
                self.newline()?;
                otherwise = Some(self.block()?);
                break;
            }
        }
        Ok(StmtKind::If { branches, otherwise })
    }

    fn branch(&mut self) -> Parsed<(Expr, Vec<Stmt>)> {
        let condition = self.expression()?;
        self.newline()?;
        Ok((condition, self.block()?))
    }

    fn for_stmt(&mut self) -> Parsed<StmtKind> {
        self.advance();
        let change = self.eat("change");
        let name = self.name()?;
        self.expect("in")?;
        let iterable = self.expression()?;
        self.newline()?;
        Ok(StmtKind::For { change, name, iterable, body: self.block()? })
    }

    fn match_expr(&mut self, as_value: bool) -> Parsed<Expr> {
        self.nested(|p| {
            let start = p.start();
            p.advance();
            let subject = Box::new(p.expression()?);
            p.newline()?;
            let arms = p.indented(|p| p.arm(as_value))?;
            Ok(Expr { kind: ExprKind::Match { subject, arms }, span: p.since(start) })
        })
    }

    fn arm(&mut self, as_value: bool) -> Parsed<Arm> {
        let pattern = self.pattern()?;
        if !self.at("->") {
            let token = self.token();
            let message = format!("expected `->`, found {}", token.kind.describe());
            let note = "an arm is `pattern -> value`, or `pattern ->` then an indented block";
            self.diags.push(Diagnostic::new("E0201", message, token.span).note(note));
            return Err(());
        }
        self.advance();
        let start = self.start();
        let body = if *self.kind() == TokenKind::Newline {
            if as_value {
                let span = self.token().span;
                self.diags.push(
                    Diagnostic::new(
                        "E0203",
                        "a `match` used as a value cannot have a block arm",
                        span,
                    )
                    .note("put the arm in one expression, or assign inside a `match` statement"),
                );
            }
            self.advance();
            ArmBody::Block(self.block()?)
        } else if self.at("print") {
            let kind = self.print_stmt()?;
            ArmBody::Stmt(Box::new(Stmt { kind, span: self.since(start) }))
        } else if self.at("return") {
            let kind = self.return_stmt()?;
            ArmBody::Stmt(Box::new(Stmt { kind, span: self.since(start) }))
        } else if self.at("match") {
            let span = self.token().span;
            let note = "put a nested `match` in an indented block under `pattern ->`";
            let message = "expected an expression, found `match`";
            self.diags.push(Diagnostic::new("E0201", message, span).note(note));
            return Err(());
        } else {
            let value = self.expression()?;
            self.newline()?;
            ArmBody::Expr(value)
        };
        Ok(Arm { pattern, body })
    }

    fn pattern(&mut self) -> Parsed<Pattern> {
        let start = self.start();
        let kind = match self.kind() {
            TokenKind::Name(name) if name == "_" => {
                self.advance();
                PatternKind::Wildcard
            }
            TokenKind::Name(_) => {
                let name = self.name()?;
                if self.eat(":") {
                    PatternKind::Typed { name, ty: self.ty()? }
                } else if self.eat("(") {
                    let mut bindings = Vec::new();
                    if !self.at(")") {
                        bindings.push(self.name()?);
                        while self.eat(",") {
                            bindings.push(self.name()?);
                        }
                    }
                    self.expect(")")?;
                    PatternKind::Variant { name, bindings }
                } else {
                    PatternKind::Name(name)
                }
            }
            TokenKind::Int(_) | TokenKind::Float(_) | TokenKind::Text(_) => {
                PatternKind::Literal(self.primary()?)
            }
            TokenKind::Sym("true" | "false") => PatternKind::Literal(self.primary()?),
            TokenKind::Sym("-") => {
                let Some(kind) = self.negative_literal() else {
                    return self.expected("a pattern");
                };
                self.advance();
                self.advance();
                PatternKind::Literal(Expr { kind, span: self.since(start) })
            }
            TokenKind::Sym("none") => {
                self.advance();
                PatternKind::None
            }
            _ => return self.expected("a pattern"),
        };
        Ok(Pattern { kind, span: self.since(start) })
    }

    fn return_stmt(&mut self) -> Parsed<StmtKind> {
        self.advance();
        if *self.kind() == TokenKind::Newline {
            self.advance();
            return Ok(StmtKind::Return(None));
        }
        Ok(StmtKind::Return(Some(self.value()?)))
    }

    fn print_stmt(&mut self) -> Parsed<StmtKind> {
        self.advance();
        let value = self.expression()?;
        self.newline()?;
        Ok(StmtKind::Print(value))
    }

    fn expr_stmt(&mut self) -> Parsed<StmtKind> {
        let start = self.start();
        let target = self.expression()?;
        let op = match self.kind() {
            TokenKind::Sym("=") => AssignOp::Set,
            TokenKind::Sym("+=") => AssignOp::Add,
            TokenKind::Sym("-=") => AssignOp::Sub,
            TokenKind::Sym("*=") => AssignOp::Mul,
            TokenKind::Sym("/=") => AssignOp::Div,
            _ => {
                self.newline()?;
                return Ok(StmtKind::Expr(target));
            }
        };
        // A parenthesized target has a span that starts after this statement does.
        let assignable = matches!(
            target.kind,
            ExprKind::Name(_)
                | ExprKind::SelfValue
                | ExprKind::Field { .. }
                | ExprKind::Index { .. }
        );
        if !assignable || target.span.start != start {
            return self.error("E0202", "can only assign to a name, field, or index", target.span);
        }
        self.advance();
        let value = self.expression()?;
        self.newline()?;
        Ok(StmtKind::Assign { target, op, value })
    }

    fn expression(&mut self) -> Parsed<Expr> {
        self.nested(|p| {
            if p.at_lambda() {
                return p.lambda();
            }
            if p.at("if") {
                return p.if_expr();
            }
            let start = p.start();
            let left = p.logic_or()?;
            if p.eat("else") {
                let right = p.expression()?;
                return Ok(p.binary(BinaryOp::Fallback, left, right, start));
            }
            Ok(left)
        })
    }

    fn at_lambda(&self) -> bool {
        match self.kind() {
            TokenKind::Name(_) => *self.kind_at(1) == TokenKind::Sym("=>"),
            TokenKind::Sym("(") => {
                let mut depth = 0;
                for (i, token) in self.tokens[self.pos..].iter().enumerate() {
                    match token.kind {
                        TokenKind::Sym("(") => depth += 1,
                        TokenKind::Sym(")") => {
                            depth -= 1;
                            if depth == 0 {
                                return *self.kind_at(i + 1) == TokenKind::Sym("=>");
                            }
                        }
                        TokenKind::Newline => return false,
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn lambda(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let mut params = Vec::new();
        if self.eat("(") {
            if !self.at(")") {
                params.push(self.name()?);
                while self.eat(",") {
                    params.push(self.name()?);
                }
            }
            self.expect(")")?;
        } else {
            params.push(self.name()?);
        }
        self.expect("=>")?;
        let body = Box::new(self.expression()?);
        Ok(Expr { kind: ExprKind::Lambda { params, body }, span: self.since(start) })
    }

    fn if_expr(&mut self) -> Parsed<Expr> {
        let start = self.start();
        self.advance();
        let condition = Box::new(self.expression()?);
        self.expect("then")?;
        let then = Box::new(self.logic_or()?);
        self.expect("else")?;
        let otherwise = Box::new(self.expression()?);
        Ok(Expr { kind: ExprKind::If { condition, then, otherwise }, span: self.since(start) })
    }

    fn binary(&self, op: BinaryOp, left: Expr, right: Expr, start: usize) -> Expr {
        let kind = ExprKind::Binary(op, Box::new(left), Box::new(right));
        Expr { kind, span: self.since(start) }
    }

    fn logic_or(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let mut left = self.logic_and()?;
        while self.eat("or") {
            self.link()?;
            let right = self.logic_and()?;
            left = self.binary(BinaryOp::Or, left, right, start);
        }
        Ok(left)
    }

    fn logic_and(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let mut left = self.logic_not()?;
        while self.eat("and") {
            self.link()?;
            let right = self.logic_not()?;
            left = self.binary(BinaryOp::And, left, right, start);
        }
        Ok(left)
    }

    fn logic_not(&mut self) -> Parsed<Expr> {
        let start = self.start();
        if self.eat("not") {
            let inner = self.nested(Self::logic_not)?;
            let kind = ExprKind::Unary(UnaryOp::Not, Box::new(inner));
            return Ok(Expr { kind, span: self.since(start) });
        }
        self.compare()
    }

    fn compare_op(&self) -> Option<(BinaryOp, usize)> {
        let op = match self.kind() {
            TokenKind::Sym("==") => BinaryOp::Eq,
            TokenKind::Sym("!=") => BinaryOp::Ne,
            TokenKind::Sym("<") => BinaryOp::Lt,
            TokenKind::Sym("<=") => BinaryOp::Le,
            TokenKind::Sym(">") => BinaryOp::Gt,
            TokenKind::Sym(">=") => BinaryOp::Ge,
            TokenKind::Sym("in") => BinaryOp::In,
            TokenKind::Sym("not") if *self.kind_at(1) == TokenKind::Sym("in") => {
                return Some((BinaryOp::NotIn, 2));
            }
            _ => return None,
        };
        Some((op, 1))
    }

    fn compare(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let left = self.range()?;
        let expr = if let Some((op, width)) = self.compare_op() {
            self.pos += width;
            let right = self.range()?;
            self.binary(op, left, right, start)
        } else if self.eat("is") {
            let negated = self.eat("not");
            let target = if self.eat("none") {
                IsTarget::None
            } else if self.eat("empty") {
                IsTarget::Empty
            } else {
                IsTarget::Type(self.nested(Self::type_term)?)
            };
            let kind = ExprKind::Is { value: Box::new(left), negated, target };
            Expr { kind, span: self.since(start) }
        } else {
            return Ok(left);
        };
        if self.compare_op().is_some() || self.at("is") {
            let span = self.token().span;
            return self.chained(span, "comparisons cannot be chained", "join them with `and`");
        }
        Ok(expr)
    }

    fn chained<T>(&mut self, span: Span, message: &str, note: &str) -> Parsed<T> {
        self.diags.push(Diagnostic::new("E0204", message, span).note(note));
        Err(())
    }

    fn range(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let left = self.bit_or()?;
        let op = if self.eat("..") {
            BinaryOp::Range
        } else if self.eat("..=") {
            BinaryOp::RangeInclusive
        } else {
            return Ok(left);
        };
        let right = self.bit_or()?;
        if self.at_any(&["..", "..="]) {
            let span = self.token().span;
            return self.chained(span, "ranges cannot be chained", "use one `..` or `..=`");
        }
        Ok(self.binary(op, left, right, start))
    }

    fn bit_or(&mut self) -> Parsed<Expr> {
        self.left_assoc(&[("|", BinaryOp::BitOr)], Self::bit_xor)
    }

    fn bit_xor(&mut self) -> Parsed<Expr> {
        self.left_assoc(&[("^", BinaryOp::BitXor)], Self::bit_and)
    }

    fn bit_and(&mut self) -> Parsed<Expr> {
        self.left_assoc(&[("&", BinaryOp::BitAnd)], Self::shift)
    }

    fn shift(&mut self) -> Parsed<Expr> {
        self.left_assoc(&[("<<", BinaryOp::Shl), (">>", BinaryOp::Shr)], Self::sum)
    }

    fn left_assoc(
        &mut self,
        ops: &[(&str, BinaryOp)],
        next: fn(&mut Self) -> Parsed<Expr>,
    ) -> Parsed<Expr> {
        let start = self.start();
        let mut left = next(self)?;
        while let Some(&(_, op)) = ops.iter().find(|(sym, _)| self.at(sym)) {
            self.link()?;
            self.advance();
            let right = next(self)?;
            left = self.binary(op, left, right, start);
        }
        Ok(left)
    }

    fn sum(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let mut left = self.product()?;
        loop {
            let op = match self.kind() {
                TokenKind::Sym("+") => BinaryOp::Add,
                TokenKind::Sym("-") => BinaryOp::Sub,
                _ => return Ok(left),
            };
            self.link()?;
            self.advance();
            let right = self.product()?;
            left = self.binary(op, left, right, start);
        }
    }

    fn product(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let mut left = self.typed()?;
        loop {
            let op = match self.kind() {
                TokenKind::Sym("*") => BinaryOp::Mul,
                TokenKind::Sym("/") => BinaryOp::Div,
                TokenKind::Sym("//") => BinaryOp::FloorDiv,
                TokenKind::Sym("%") => BinaryOp::Rem,
                _ => return Ok(left),
            };
            self.link()?;
            self.advance();
            let right = self.typed()?;
            left = self.binary(op, left, right, start);
        }
    }

    fn unary(&mut self) -> Parsed<Expr> {
        let start = self.start();
        if let Some(kind) = self.negative_literal() {
            self.advance();
            self.advance();
            return Ok(Expr { kind, span: self.since(start) });
        }
        let op = match self.kind() {
            TokenKind::Sym("-") => UnaryOp::Neg,
            TokenKind::Sym("~") => UnaryOp::BitNot,
            TokenKind::Sym("try") => UnaryOp::Try,
            TokenKind::Sym("spawn") => UnaryOp::Spawn,
            TokenKind::Sym("shared") => UnaryOp::Shared,
            TokenKind::Sym("weak") => UnaryOp::Weak,
            _ => return self.postfix(),
        };
        self.advance();
        let inner = self.nested(Self::unary)?;
        Ok(Expr { kind: ExprKind::Unary(op, Box::new(inner)), span: self.since(start) })
    }

    /// A `-` joins a number with no postfix after it, so that the lowest `int`
    /// can be written. With a postfix, `-` applies to the postfix result.
    fn negative_literal(&self) -> Option<ExprKind> {
        if !self.at("-") || matches!(self.kind_at(2), TokenKind::Sym("(" | "[" | ".")) {
            return None;
        }
        match self.kind_at(1) {
            TokenKind::Int(lexeme) => Some(ExprKind::Int(format!("-{lexeme}"))),
            TokenKind::Float(lexeme) => Some(ExprKind::Float(format!("-{lexeme}"))),
            _ => None,
        }
    }

    fn postfix(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let mut expr = self.primary()?;
        while self.at_any(&["(", "[", "."]) {
            self.link()?;
            let kind = if self.eat("(") {
                let args = self.args()?;
                self.expect(")")?;
                ExprKind::Call { callee: Box::new(expr), args }
            } else if self.eat("[") {
                let mut indices = vec![self.expression()?];
                while self.eat(",") {
                    indices.push(self.expression()?);
                }
                self.expect("]")?;
                ExprKind::Index { base: Box::new(expr), indices }
            } else {
                self.advance();
                ExprKind::Field { base: Box::new(expr), name: self.name()? }
            };
            expr = Expr { kind, span: self.since(start) };
        }
        Ok(expr)
    }

    fn args(&mut self) -> Parsed<Vec<Arg>> {
        let mut args = Vec::new();
        while !self.at(")") {
            args.push(self.arg()?);
            if !self.eat(",") {
                break;
            }
        }
        Ok(args)
    }

    fn arg(&mut self) -> Parsed<Arg> {
        if matches!(self.kind(), TokenKind::Name(_)) && *self.kind_at(1) == TokenKind::Sym(":") {
            let name = Some(self.name()?);
            self.advance();
            return Ok(Arg { name, change: false, value: self.expression()? });
        }
        let change = self.eat("change");
        Ok(Arg { name: None, change, value: self.expression()? })
    }

    /// Parses `unary [ "?" ]`. See "Type arguments" in docs/grammar.md.
    fn typed(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let value = self.unary()?;
        if !self.eat("?") {
            return Ok(value);
        }
        Ok(Expr { kind: ExprKind::Optional(Box::new(value)), span: self.since(start) })
    }

    fn primary(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let first = self.pos;
        let kind = match self.kind() {
            TokenKind::Name(name) => ExprKind::Name(name.clone()),
            TokenKind::Int(lexeme) => ExprKind::Int(lexeme.clone()),
            TokenKind::Float(lexeme) => ExprKind::Float(lexeme.clone()),
            TokenKind::Sym("any" | "task") => {
                let ty = self.type_prefix()?;
                return Ok(Expr { kind: ExprKind::Type(ty), span: self.since(start) });
            }
            TokenKind::Sym("self") => ExprKind::SelfValue,
            TokenKind::Sym("none") => ExprKind::None,
            TokenKind::Sym("true") => ExprKind::Bool(true),
            TokenKind::Sym("false") => ExprKind::Bool(false),
            TokenKind::Text(_) => {
                let parts = self.take_text();
                self.advance();
                ExprKind::Text(self.text_parts(parts)?)
            }
            TokenKind::Sym("(") => {
                self.advance();
                let inner = self.expression()?;
                self.close_group()?;
                return Ok(inner);
            }
            TokenKind::Sym("[") => {
                self.advance();
                let mut items = Vec::new();
                while !self.at("]") {
                    items.push(self.expression()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("]")?;
                ExprKind::List(items)
            }
            TokenKind::Sym("{") => {
                self.advance();
                let mut entries = Vec::new();
                while !self.at("}") {
                    let key = self.expression()?;
                    self.expect(":")?;
                    entries.push((key, self.expression()?));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("}")?;
                ExprKind::Map(entries)
            }
            _ => return self.expected("an expression"),
        };
        if self.pos == first {
            self.advance();
        }
        Ok(Expr { kind, span: self.since(start) })
    }

    /// Moves the parts out of the current text token. The parser never reads
    /// a token twice, so nothing needs them after this.
    fn take_text(&mut self) -> Vec<RawPart> {
        let index = self.pos.min(self.tokens.len() - 1);
        match &mut self.tokens[index].kind {
            TokenKind::Text(parts) => std::mem::take(parts),
            _ => Vec::new(),
        }
    }

    fn text_parts(&mut self, parts: Vec<RawPart>) -> Parsed<Vec<TextPart>> {
        let mut out = Vec::new();
        for part in parts {
            out.push(match part {
                RawPart::Literal(text) => TextPart::Literal(text),
                RawPart::Expr(tokens) => TextPart::Expr(self.interpolated(tokens)?),
            });
        }
        Ok(out)
    }

    /// Parses the tokens of one `{expression}` in place of the main stream.
    fn interpolated(&mut self, tokens: Vec<Token>) -> Parsed<Expr> {
        let outer = std::mem::replace(&mut self.tokens, tokens);
        let pos = std::mem::replace(&mut self.pos, 0);
        let before = self.diags.len();
        let result = self.expression().and_then(|expr| {
            self.expect("}")?;
            Ok(expr)
        });
        if let Some(diag) = self.diags.get_mut(before)
            && diag.code == "E0201"
        {
            diag.notes
                .push("`{` in text starts an interpolation; write `{{` for a literal brace".into());
        }
        self.tokens = outer;
        self.pos = pos;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn parsed(src: &str) -> (File, Vec<&'static str>) {
        let (tokens, lex_diags) = lex(src);
        let (file, diags) = parse(tokens);
        (file, lex_diags.iter().chain(&diags).map(|d| d.code).collect())
    }

    fn codes(src: &str) -> Vec<&'static str> {
        parsed(src).1
    }

    fn ok(src: &str) {
        assert_eq!(codes(src), Vec::<&str>::new(), "{src}");
    }

    /// Fully parenthesized form of an expression, to test the tree shape.
    fn shape(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Name(n) | ExprKind::Int(n) | ExprKind::Float(n) => n.clone(),
            ExprKind::Unary(op, x) => format!("({op:?} {})", shape(x)),
            ExprKind::Binary(op, a, b) => format!("({op:?} {} {})", shape(a), shape(b)),
            ExprKind::Is { value, negated, target } => {
                let target = match target {
                    IsTarget::None => "none",
                    IsTarget::Empty => "empty",
                    IsTarget::Type(_) => "type",
                };
                format!("(is{} {} {target})", if *negated { "-not" } else { "" }, shape(value))
            }
            ExprKind::If { condition, then, otherwise } => {
                format!("(if {} {} {})", shape(condition), shape(then), shape(otherwise))
            }
            ExprKind::Lambda { params, body } => {
                let names: Vec<&str> = params.iter().map(|p| p.text.as_str()).collect();
                format!("(lambda [{}] {})", names.join(" "), shape(body))
            }
            ExprKind::Call { callee, args } => {
                let args: Vec<String> = args
                    .iter()
                    .map(|a| {
                        let name =
                            a.name.as_ref().map_or(String::new(), |n| format!("{}:", n.text));
                        let change = if a.change { "change " } else { "" };
                        format!(" {name}{change}{}", shape(&a.value))
                    })
                    .collect();
                format!("(call {}{})", shape(callee), args.concat())
            }
            ExprKind::Index { base, indices } => {
                let indices: Vec<String> = indices.iter().map(shape).collect();
                format!("(index {} {})", shape(base), indices.join(" "))
            }
            ExprKind::Field { base, name } => format!("(field {} {})", shape(base), name.text),
            _ => "?".into(),
        }
    }

    fn expr(src: &str) -> String {
        let (file, codes) = parsed(src);
        assert!(codes.is_empty(), "{src}: {codes:?}");
        match &file.stmts[0].kind {
            StmtKind::Expr(e) => shape(e),
            _ => panic!("not an expression: {src}"),
        }
    }

    #[test]
    fn precedence_follows_the_table() {
        assert_eq!(expr("a + b * c"), "(Add a (Mul b c))");
        assert_eq!(expr("(a + b) * c"), "(Mul (Add a b) c)");
        assert_eq!(expr("a - b - c"), "(Sub (Sub a b) c)");
        assert_eq!(expr("a // b % c"), "(Rem (FloorDiv a b) c)");
        assert_eq!(expr("a | b ^ c & d << e + f"), "(BitOr a (BitXor b (BitAnd c (Shl d (Add e f)))))");
        assert_eq!(expr("a & 1 == 0"), "(Eq (BitAnd a 1) 0)");
        assert_eq!(expr("a >> b << c"), "(Shl (Shr a b) c)");
        assert_eq!(expr("0..a | b"), "(Range 0 (BitOr a b))");
        assert_eq!(expr("~a & b"), "(BitAnd (BitNot a) b)");
        assert_eq!(expr("a or b and c"), "(Or a (And b c))");
        assert_eq!(expr("not a == b"), "(Not (Eq a b))");
        assert_eq!(expr("a + 1 < b * 2"), "(Lt (Add a 1) (Mul b 2))");
        assert_eq!(expr("a..b + 1"), "(Range a (Add b 1))");
        assert_eq!(expr("a < b or c"), "(Or (Lt a b) c)");
        assert_eq!(expr("-a * b"), "(Mul (Neg a) b)");
        assert_eq!(expr("a or b else c or d"), "(Fallback (Or a b) (Or c d))");
        assert_eq!(expr("a else b else c"), "(Fallback a (Fallback b c))");
    }

    #[test]
    fn prefix_operators_cover_the_postfix_chain() {
        assert_eq!(expr("try a(b).c()"), "(Try (call (field (call a b) c)))");
        assert_eq!(expr("(try a).b"), "(field (Try a) b)");
        assert_eq!(expr("spawn f(x)"), "(Spawn (call f x))");
        assert_eq!(expr("shared T(a: 1)"), "(Shared (call T a:1))");
    }

    #[test]
    fn postfix_forms() {
        assert_eq!(expr("a.b[1](c)"), "(call (index (field a b) 1) c)");
        assert_eq!(expr("f[int](x)"), "(call (index f int) x)");
        assert_eq!(expr("m[a, b]"), "(index m a b)");
        assert_eq!(expr("f(change x, y, k: 1,)"), "(call f change x y k:1)");
    }

    #[test]
    fn is_and_in() {
        assert_eq!(expr("x is none"), "(is x none)");
        assert_eq!(expr("x is not empty"), "(is-not x empty)");
        assert_eq!(expr("x is text or y > 3"), "(Or (is x type) (Gt y 3))");
        assert_eq!(expr("x is int? and y"), "(And (is x type) y)");
        assert_eq!(expr("a in b"), "(In a b)");
        assert_eq!(expr("a not in b"), "(NotIn a b)");
    }

    #[test]
    fn if_expressions_and_lambdas() {
        assert_eq!(expr("f(if a then b else c)"), "(call f (if a b c))");
        assert_eq!(expr("f(if a then b else c else d)"), "(call f (if a b (Fallback c d)))");
        assert_eq!(expr("f(x => x + 1)"), "(call f (lambda [x] (Add x 1)))");
        assert_eq!(expr("f((a, b) => a)"), "(call f (lambda [a b] a))");
        assert_eq!(expr("f(() => 1)"), "(call f (lambda [] 1))");
        assert_eq!(expr("f((a))"), "(call f a)");
        assert_eq!(expr("f(a => b => a)"), "(call f (lambda [a] (lambda [b] a)))");
    }

    #[test]
    fn interpolation_is_parsed() {
        let (file, codes) = parsed("print \"a {b + c} d\"\n");
        assert!(codes.is_empty());
        let StmtKind::Print(value) = &file.stmts[0].kind else { panic!() };
        let ExprKind::Text(parts) = &value.kind else { panic!() };
        let TextPart::Expr(inner) = &parts[1] else { panic!() };
        assert_eq!(shape(inner), "(Add b c)");
        assert_eq!(self::codes("print \"{a b}\"\n"), ["E0201"]);
        assert_eq!(self::codes("print \"{}\"\n"), ["E0201"]);
    }

    #[test]
    fn statements_parse() {
        ok("let a: int = 1\nvar b = 2\nb += 1\nb -= 1\nb *= 1\nb /= 1\na.b = 1\na[0] = 2\n");
        ok("(a).b = 1\n");
        ok("if a\n    b\nelse if c\n    d\nelse\n    e\n");
        ok("while a\n    break\nfor i in 0..3\n    continue\nfor change x in y\n    ...\n");
        ok("unsafe\n    let n = 1\nexpect a == 1\nprint a\n");
        ok("task f()\n    task g()\n        return\n    return g()\n");
    }

    #[test]
    fn declarations_parse() {
        ok("use std.http\nuse std.json as js\nuse ./models\nuse ./models.User\n");
        ok("public task add[T: Add, U,](a: T, change b: U, take c: int = 1,) -> T\n    return a\n");
        ok("task f(self, change self: User, c: task(int, int) -> int)\n    ...\n");
        ok("unsafe extern task strlen(s: pointer[byte]) -> u64\n");
        ok("public unsafe extern task f()\n");
        ok("type Id = int\npublic type P[A, B]\n    left: A\n    right: B = 1\n");
        ok("choice Shape\n    Circle(radius: float)\n    Point\n");
        ok("trait T\n    task f(self) -> text\n    task g()\n");
        ok("test \"adds\"\n    expect 1 == 1\n");
    }

    #[test]
    fn types_parse() {
        ok("let a: weak Node? = 1\n");
        ok("let a: A or B or none = 1\n");
        ok("let a: (A or B)? = 1\n");
        ok("let a: list[shared Node] = 1\n");
        ok("let a: map[text, any Show,] = 1\n");
        ok("let a: http.Request = 1\n");
        ok("let a: task() = 1\n");
    }

    #[test]
    fn match_forms_parse() {
        ok(
            "match a\n    1 -> b\n    1.5 -> print c\n    \"x\" -> return\n    true ->\n        d\n    none -> e\n    _ -> f\n",
        );
        ok(
            "match a\n    Circle(r) -> r\n    Pair(a, b) -> a\n    Empty() -> b\n    Point -> c\n    e: Error -> d\n    f: A or B -> g\n",
        );
        ok("let a = match b\n    _ -> 1\nprint a\n");
        ok("task f()\n    return match x\n        _ -> 1\n");
        ok("match a\n    _ -> return match b\n        _ -> 1\n");
    }

    #[test]
    fn errors_have_codes() {
        assert_eq!(codes("let = 1\n"), ["E0201"]);
        assert_eq!(codes("1 + 2 = 3\n"), ["E0202"]);
        assert_eq!(codes("f() = 3\n"), ["E0202"]);
        assert_eq!(codes("(a) = 3\n"), ["E0202"]);
        assert_eq!(codes("(a.b) += 3\n"), ["E0202"]);
        assert_eq!(codes("if a then b else c\n"), ["E0201"]);
        assert_eq!(codes("let a = match b\n    _ ->\n        c\n"), ["E0203"]);
        assert_eq!(codes("a < b < c\n"), ["E0204"]);
        assert_eq!(codes("a == b is none\n"), ["E0204"]);
        assert_eq!(codes("a..b..c\n"), ["E0204"]);
        assert_eq!(codes("if a\n    use b\n"), ["E0206"]);
    }

    #[test]
    fn missing_block_says_how_to_fix_it() {
        let (tokens, _) = lex("if a\nb\n");
        let (_, diags) = parse(tokens);
        assert_eq!(diags[0].notes, ["indent the block by 4 spaces"]);
    }

    #[test]
    fn recovery_reports_every_bad_line() {
        assert_eq!(codes("let = 1\nprint\nlet ok = 2\nlet = 3\n"), ["E0201"; 3]);
        assert_eq!(codes("if\n    let = 1\nlet = 2\n"), ["E0201", "E0201"]);
        assert_eq!(codes("task f()\n    let = 1\n    let = 2\n"), ["E0201", "E0201"]);
        assert_eq!(codes("type T\n    1\n    a: int\n    2\n"), ["E0201", "E0201"]);
    }

    #[test]
    fn broken_input_ends_with_errors() {
        for src in [
            ")", "(", "[", "{", "a(", "a.", "\n    a", "else", "match", "task", "\"{", "->", "1 +",
            "    ",
        ] {
            let (_, _) = parsed(src);
        }
        assert!(!codes("    a\n").is_empty());
    }
}
