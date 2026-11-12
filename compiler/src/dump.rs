//! `LUCID_DUMP`: each stage as text that both compilers write, by decision
//! 215. See "Dumps" in docs/compiler.md for the form.

use crate::ast;
use crate::diag::Source;
use crate::lexer;
use lucid_backend::dump::{Out, files, source_head};

/// A name of the tree: its text and span.
trait Names {
    fn name(&mut self, name: &ast::Name) -> &mut Self;
}

impl Names for Out {
    fn name(&mut self, name: &ast::Name) -> &mut Self {
        self.text(&name.text).span(name.span)
    }
}

/// The stages that `LUCID_DUMP` can name.
#[derive(Clone, Copy, PartialEq)]
pub enum Stage {
    Tokens,
    Ast,
    Resolved,
    Checked,
    Mir,
    Owned,
}

impl Stage {
    pub fn named(name: &str) -> Option<Stage> {
        match name {
            "tokens" => Some(Stage::Tokens),
            "ast" => Some(Stage::Ast),
            "resolved" => Some(Stage::Resolved),
            "checked" => Some(Stage::Checked),
            "mir" => Some(Stage::Mir),
            "owned" => Some(Stage::Owned),
            _ => None,
        }
    }
}

pub fn tokens(tokens: &[lexer::Token]) -> String {
    let mut out = Out::default();
    tokens.iter().for_each(|t| token(&mut out, t));
    out.finish()
}

fn token(out: &mut Out, item: &lexer::Token) {
    use lexer::TokenKind as K;
    let span = item.span;
    match &item.kind {
        K::Name(name) => _ = out.line("Name").span(span).text(name),
        K::Int(digits) => _ = out.line("Int").span(span).text(digits),
        K::Float(digits) => _ = out.line("Float").span(span).text(digits),
        K::Sym(sym) => _ = out.line("Sym").span(span).text(sym),
        K::Newline => _ = out.line("Newline").span(span),
        K::Indent => _ = out.line("Indent").span(span),
        K::Dedent => _ = out.line("Dedent").span(span),
        K::Eof => _ = out.line("Eof").span(span),
        K::Text(parts) => {
            out.line("Text").span(span).atom(parts.len());
            out.nest(|out| {
                for part in parts {
                    match part {
                        lexer::TextPart::Literal(text) => {
                            out.line("Literal").text(text);
                        }
                        lexer::TextPart::Expr(tokens) => {
                            out.line("Expr").atom(tokens.len());
                            out.nest(|out| tokens.iter().for_each(|t| token(out, t)));
                        }
                    }
                }
            });
        }
    }
}

pub fn ast(file: &ast::File) -> String {
    let mut out = Out::default();
    out.line("File").atom(file.stmts.len());
    out.nest(|out| stmts(out, &file.stmts));
    out.finish()
}

fn stmts(out: &mut Out, stmts: &[ast::Stmt]) {
    stmts.iter().for_each(|s| stmt(out, s));
}

fn count<T>(out: &mut Out, items: &Option<Vec<T>>) {
    match items {
        Some(items) => out.atom(items.len()),
        None => out.atom("-"),
    };
}

fn stmt(out: &mut Out, stmt: &ast::Stmt) {
    use ast::StmtKind as S;
    let span = stmt.span;
    match &stmt.kind {
        S::Use { relative, path, alias } => {
            out.line("Use").span(span).flag(*relative).atom(path.len());
            path.iter().for_each(|name| _ = out.name(name));
            match alias {
                Some(alias) => out.name(alias),
                None => out.atom("-"),
            };
        }
        S::Test { name, body } => {
            out.line("Test").span(span).atom(body.len());
            out.nest(|out| {
                expr(out, name);
                stmts(out, body);
            });
        }
        S::Task(def) => {
            out.line("Task").span(span).flag(def.public);
            count(out, &def.body);
            out.nest(|out| {
                sig(out, &def.sig);
                def.body.iter().for_each(|body| stmts(out, body));
            });
        }
        S::Type(def) => {
            out.line("Type").span(span).flag(def.public).name(&def.name);
            out.atom(def.type_params.len());
            out.nest(|out| {
                def.type_params.iter().for_each(|p| type_param(out, p));
                match &def.body {
                    ast::TypeBody::Alias(ty) => {
                        out.line("Alias");
                        out.nest(|out| ty_node(out, ty));
                    }
                    ast::TypeBody::Fields(fields) => {
                        out.line("Fields").atom(fields.len());
                        out.nest(|out| {
                            for field in fields {
                                out.line("Field").name(&field.name);
                                out.nest(|out| {
                                    ty_node(out, &field.ty);
                                    maybe_expr(out, field.default.as_ref());
                                });
                            }
                        });
                    }
                }
            });
        }
        S::Choice(def) => {
            out.line("Choice").span(span).flag(def.public).name(&def.name);
            out.atom(def.type_params.len()).atom(def.variants.len());
            out.nest(|out| {
                def.type_params.iter().for_each(|p| type_param(out, p));
                for variant in &def.variants {
                    out.line("Variant").name(&variant.name).atom(variant.params.len());
                    out.nest(|out| variant.params.iter().for_each(|p| param(out, p)));
                }
            });
        }
        S::Trait(def) => {
            out.line("Trait").span(span).flag(def.public).name(&def.name);
            out.atom(def.type_params.len()).atom(def.tasks.len());
            out.nest(|out| {
                def.type_params.iter().for_each(|p| type_param(out, p));
                def.tasks.iter().for_each(|task| sig(out, task));
            });
        }
        S::Let { mutable, name, ty, value } => {
            out.line("Let").span(span).flag(*mutable).name(name);
            out.nest(|out| {
                maybe_ty(out, ty.as_ref());
                expr(out, value);
            });
        }
        S::If { branches, otherwise } => {
            out.line("If").span(span).atom(branches.len());
            count(out, otherwise);
            out.nest(|out| {
                for (condition, body) in branches {
                    out.line("Branch").atom(body.len());
                    out.nest(|out| {
                        expr(out, condition);
                        stmts(out, body);
                    });
                }
                otherwise.iter().for_each(|body| stmts(out, body));
            });
        }
        S::While { condition, body } => {
            out.line("While").span(span).atom(body.len());
            out.nest(|out| {
                expr(out, condition);
                stmts(out, body);
            });
        }
        S::For { change, name, iterable, body } => {
            out.line("For").span(span).flag(*change).name(name).atom(body.len());
            out.nest(|out| {
                expr(out, iterable);
                stmts(out, body);
            });
        }
        S::Return(value) => {
            out.line("Return").span(span);
            out.nest(|out| maybe_expr(out, value.as_ref()));
        }
        S::Print(value) => {
            out.line("Print").span(span);
            out.nest(|out| expr(out, value));
        }
        S::Expect(value) => {
            out.line("Expect").span(span);
            out.nest(|out| expr(out, value));
        }
        S::Expr(value) => {
            out.line("Expr").span(span);
            out.nest(|out| expr(out, value));
        }
        S::Unsafe(body) => {
            out.line("Unsafe").span(span).atom(body.len());
            out.nest(|out| stmts(out, body));
        }
        S::Break => _ = out.line("Break").span(span),
        S::Continue => _ = out.line("Continue").span(span),
        S::Placeholder => _ = out.line("Placeholder").span(span),
        S::Assign { target, op, value } => {
            out.line("Assign").span(span).atom(format!("{op:?}"));
            out.nest(|out| {
                expr(out, target);
                expr(out, value);
            });
        }
    }
}

fn sig(out: &mut Out, sig: &ast::TaskSig) {
    out.line("Sig").name(&sig.name).atom(sig.type_params.len()).atom(sig.params.len());
    out.nest(|out| {
        sig.type_params.iter().for_each(|p| type_param(out, p));
        sig.params.iter().for_each(|p| param(out, p));
        maybe_ty(out, sig.returns.as_ref());
    });
}

fn type_param(out: &mut Out, param: &ast::TypeParam) {
    out.line("TypeParam").name(&param.name).atom(param.bounds.len());
    out.nest(|out| param.bounds.iter().for_each(|ty| ty_node(out, ty)));
}

fn param(out: &mut Out, param: &ast::Param) {
    out.line("Param").atom(format!("{:?}", param.mode)).name(&param.name);
    out.nest(|out| {
        maybe_ty(out, param.ty.as_ref());
        maybe_expr(out, param.default.as_ref());
    });
}

fn maybe_ty(out: &mut Out, ty: Option<&ast::Type>) {
    match ty {
        Some(ty) => ty_node(out, ty),
        None => out.absent(),
    }
}

fn ty_node(out: &mut Out, ty: &ast::Type) {
    use ast::TypeKind as T;
    let span = ty.span;
    match &ty.kind {
        T::Named { path, args } => {
            out.line("Named").span(span).atom(path.len());
            path.iter().for_each(|name| _ = out.name(name));
            out.atom(args.len());
            out.nest(|out| args.iter().for_each(|ty| ty_node(out, ty)));
        }
        T::None => _ = out.line("None").span(span),
        T::Task { params, returns } => {
            out.line("Task").span(span).atom(params.len());
            out.nest(|out| {
                params.iter().for_each(|ty| ty_node(out, ty));
                maybe_ty(out, returns.as_deref());
            });
        }
        T::Union(members) => {
            out.line("Union").span(span).atom(members.len());
            out.nest(|out| members.iter().for_each(|ty| ty_node(out, ty)));
        }
        T::Optional(inner) | T::Shared(inner) | T::Weak(inner) | T::Any(inner) => {
            let head = match &ty.kind {
                T::Optional(_) => "Optional",
                T::Shared(_) => "Shared",
                T::Weak(_) => "Weak",
                _ => "Any",
            };
            out.line(head).span(span);
            out.nest(|out| ty_node(out, inner));
        }
    }
}

fn maybe_expr(out: &mut Out, value: Option<&ast::Expr>) {
    match value {
        Some(value) => expr(out, value),
        None => out.absent(),
    }
}

fn expr(out: &mut Out, value: &ast::Expr) {
    use ast::ExprKind as E;
    let span = value.span;
    match &value.kind {
        E::Name(name) => _ = out.line("Name").span(span).text(name),
        E::SelfValue => _ = out.line("SelfValue").span(span),
        E::None => _ = out.line("None").span(span),
        E::Bool(value) => _ = out.line("Bool").span(span).flag(*value),
        E::Int(digits) => _ = out.line("Int").span(span).text(digits),
        E::Float(digits) => _ = out.line("Float").span(span).text(digits),
        E::Text(parts) => {
            out.line("Text").span(span).atom(parts.len());
            out.nest(|out| {
                for part in parts {
                    match part {
                        ast::TextPart::Literal(text) => _ = out.line("Literal").text(text),
                        ast::TextPart::Expr(value) => expr(out, value),
                    }
                }
            });
        }
        E::List(items) => {
            out.line("List").span(span).atom(items.len());
            out.nest(|out| items.iter().for_each(|item| expr(out, item)));
        }
        E::Map(entries) => {
            out.line("Map").span(span).atom(entries.len());
            out.nest(|out| {
                for (key, value) in entries {
                    out.line("Entry");
                    out.nest(|out| {
                        expr(out, key);
                        expr(out, value);
                    });
                }
            });
        }
        E::Unary(op, inner) => {
            out.line("Unary").span(span).atom(format!("{op:?}"));
            out.nest(|out| expr(out, inner));
        }
        E::Binary(op, left, right) => {
            out.line("Binary").span(span).atom(format!("{op:?}"));
            out.nest(|out| {
                expr(out, left);
                expr(out, right);
            });
        }
        E::Is { value, negated, target } => {
            let kind = match target {
                ast::IsTarget::None => "None",
                ast::IsTarget::Empty => "Empty",
                ast::IsTarget::Type(_) => "Type",
            };
            out.line("Is").span(span).flag(*negated).atom(kind);
            out.nest(|out| {
                expr(out, value);
                if let ast::IsTarget::Type(ty) = target {
                    ty_node(out, ty);
                }
            });
        }
        E::If { condition, then, otherwise } => {
            out.line("If").span(span);
            out.nest(|out| {
                expr(out, condition);
                expr(out, then);
                expr(out, otherwise);
            });
        }
        E::Lambda { params, body } => {
            out.line("Lambda").span(span).atom(params.len());
            params.iter().for_each(|name| _ = out.name(name));
            out.nest(|out| expr(out, body));
        }
        E::Call { callee, args } => {
            out.line("Call").span(span).atom(args.len());
            out.nest(|out| {
                expr(out, callee);
                for arg in args {
                    out.line("Arg");
                    match &arg.name {
                        Some(name) => out.name(name),
                        None => out.atom("-"),
                    };
                    out.flag(arg.change);
                    out.nest(|out| expr(out, &arg.value));
                }
            });
        }
        E::Index { base, indices } => {
            out.line("Index").span(span).atom(indices.len());
            out.nest(|out| {
                expr(out, base);
                indices.iter().for_each(|index| expr(out, index));
            });
        }
        E::Field { base, name } => {
            out.line("Field").span(span).name(name);
            out.nest(|out| expr(out, base));
        }
        E::Optional(inner) => {
            out.line("Optional").span(span);
            out.nest(|out| expr(out, inner));
        }
        E::Type(ty) => {
            out.line("Type").span(span);
            out.nest(|out| ty_node(out, ty));
        }
        E::Match { subject, arms } => {
            out.line("Match").span(span).atom(arms.len());
            out.nest(|out| {
                expr(out, subject);
                arms.iter().for_each(|a| arm(out, a));
            });
        }
    }
}

fn arm(out: &mut Out, arm: &ast::Arm) {
    out.line("Arm");
    match &arm.body {
        ast::ArmBody::Expr(_) => out.atom("Expr"),
        ast::ArmBody::Stmt(_) => out.atom("Stmt"),
        ast::ArmBody::Block(body) => out.atom("Block").atom(body.len()),
    };
    out.nest(|out| {
        pattern(out, &arm.pattern);
        match &arm.body {
            ast::ArmBody::Expr(value) => expr(out, value),
            ast::ArmBody::Stmt(body) => stmt(out, body),
            ast::ArmBody::Block(body) => stmts(out, body),
        }
    });
}

fn pattern(out: &mut Out, pattern: &ast::Pattern) {
    use ast::PatternKind as P;
    let span = pattern.span;
    match &pattern.kind {
        P::Wildcard => _ = out.line("Wildcard").span(span),
        P::None => _ = out.line("None").span(span),
        P::Literal(value) => {
            out.line("Literal").span(span);
            out.nest(|out| expr(out, value));
        }
        P::Name(name) => _ = out.line("Name").span(span).name(name),
        P::Variant { name, bindings } => {
            out.line("Variant").span(span).name(name).atom(bindings.len());
            bindings.iter().for_each(|name| _ = out.name(name));
        }
        P::Typed { name, ty } => {
            out.line("Typed").span(span).name(name);
            out.nest(|out| ty_node(out, ty));
        }
    }
}

/// What `resolve.rs` gives: the files read, what it records about them, and
/// the joined tree, if loading got that far. Printed even with errors.
pub fn resolved(source: &Source, file: Option<&ast::File>) -> String {
    let mut out = Out::default();
    source_head(&mut out, source);
    out.nest(|out| {
        files(out, source);
        for (start, manifest) in &source.no_unsafe {
            out.line("NoUnsafe").atom(start).text(manifest);
        }
        for (start, package) in &source.packages {
            out.line("Package").atom(start).atom(package);
        }
        match source.web_only {
            Some(span) => _ = out.line("WebOnly").span(span),
            None => out.absent(),
        }
        match source.host_only {
            Some((span, name)) => _ = out.line("HostOnly").span(span).text(name),
            None => out.absent(),
        }
    });
    match file {
        Some(file) => {
            out.line("File").atom(file.stmts.len());
            out.nest(|out| stmts(out, &file.stmts));
        }
        None => out.absent(),
    }
    out.finish()
}
