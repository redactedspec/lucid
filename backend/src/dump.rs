//! The text form of the `LUCID_DUMP` stages, by decision 215, and the
//! `owned` stage, which is also what the backend reads, by decision 216. See
//! "Dumps" in docs/compiler.md for the form.

use crate::diag::{Grant, Source, Span};
use crate::mir::{self, Ty};

/// One node per line, indented two spaces per level. A line holds a name and
/// atoms, and the node's children follow it, one level deeper.
#[derive(Default)]
pub struct Out {
    text: String,
    depth: usize,
}

impl Out {
    pub fn line(&mut self, head: &str) -> &mut Self {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        for _ in 0..self.depth {
            self.text.push_str("  ");
        }
        self.text.push_str(head);
        self
    }

    pub fn atom(&mut self, atom: impl std::fmt::Display) -> &mut Self {
        self.text.push(' ');
        self.text += &atom.to_string();
        self
    }

    pub fn flag(&mut self, value: bool) -> &mut Self {
        self.atom(value)
    }

    pub fn text(&mut self, value: &str) -> &mut Self {
        self.text.push(' ');
        quote(&mut self.text, value);
        self
    }

    pub fn span(&mut self, span: Span) -> &mut Self {
        self.atom(span.start).atom(span.end)
    }

    pub fn ty(&mut self, ty: Ty) -> &mut Self {
        let word = match ty {
            Ty::Int => "int",
            Ty::Float => "float",
            Ty::Bool => "bool",
            Ty::Byte => "byte",
            Ty::I8 => "i8",
            Ty::I16 => "i16",
            Ty::I32 => "i32",
            Ty::U16 => "u16",
            Ty::U32 => "u32",
            Ty::U64 => "u64",
            Ty::F32 => "f32",
            Ty::None => "none",
            Ty::Pointer => "pointer",
            Ty::Text => "text",
            Ty::Socket => "socket",
            Ty::Database => "database",
            Ty::Sql => "sql",
            Ty::Error => "error",
            Ty::List(item) => return self.atom("list").ty(*item),
            Ty::Map(key, value) => return self.atom("map").ty(*key).ty(*value),
            Ty::Set(item) => return self.atom("set").ty(*item),
            Ty::Spawned(item) => return self.atom("spawned").ty(*item),
            Ty::Channel(item) => return self.atom("channel").ty(*item),
            Ty::Lock(item) => return self.atom("lock").ty(*item),
            Ty::Data(name, id, args) => return self.atom("data").text(name).atom(id).tys(args),
            Ty::Union(members) => return self.atom("union").tys(members),
            Ty::Any(name) => return self.atom("any").text(name),
            Ty::Task(params, returns) => return self.atom("task").tys(params).ty(*returns),
            Ty::Param(name, at) => return self.atom("param").text(name).atom(at),
        };
        self.atom(word)
    }

    pub fn tys(&mut self, tys: &[Ty]) -> &mut Self {
        self.atom(tys.len());
        tys.iter().fold(self, |out, &ty| out.ty(ty))
    }

    pub fn ints(&mut self, values: &[usize]) -> &mut Self {
        self.atom(values.len());
        values.iter().fold(self, |out, value| out.atom(value))
    }

    /// A child line `-` for a missing child.
    pub fn absent(&mut self) {
        self.line("-");
    }

    pub fn nest(&mut self, children: impl FnOnce(&mut Self)) {
        self.depth += 1;
        children(self);
        self.depth -= 1;
    }

    pub fn finish(mut self) -> String {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text
    }
}

/// A text in double quotes. Only `\`, `"`, and control characters are escaped,
/// so both compilers agree on every byte.
pub fn quote(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// As Lucid shows a float: the fewest digits that read back the same, with a
/// point before any exponent.
pub fn float(value: f64) -> String {
    let text = format!("{value:?}");
    match text.split_once('e') {
        Some((digits, exponent)) if !digits.contains('.') => format!("{digits}.0e{exponent}"),
        _ => text,
    }
}

/// The line `Source`: its file count and the grants.
pub fn source_head(out: &mut Out, source: &Source) {
    out.line("Source").atom(source.files.len());
    match &source.grants {
        Some(grants) => {
            out.atom(grants.len());
            for grant in grants {
                match grant {
                    Grant::All => _ = out.atom("all"),
                    Grant::List(items) => {
                        out.atom("list").atom(items.len());
                        items.iter().for_each(|item| _ = out.text(item));
                    }
                }
            }
        }
        None => _ = out.atom("-"),
    }
}

pub fn files(out: &mut Out, source: &Source) {
    for file in &source.files {
        out.line("File").text(file.path).atom(file.start);
    }
}

/// The program after `mono.rs`. With `source`, after `ownership.rs`, it is
/// what a backend reads, so it holds the files, their text, and the grants.
pub fn program(program: &mir::Program, source: Option<&Source>) -> String {
    let mut out = Out::default();
    if let Some(source) = source {
        source_head(&mut out, source);
        out.text(&source.text);
        out.nest(|out| files(out, source));
    }
    out.line("Program")
        .atom(program.datas.len())
        .atom(program.tasks.len())
        .atom(program.tests.len())
        .tys(program.types.all());
    out.nest(|out| {
        for shape in &program.datas {
            out.line("Shape").flag(shape.choice).atom(shape.variants.len());
            out.nest(|out| {
                for variant in &shape.variants {
                    out.line("Variant").text(&variant.name).atom(variant.fields.len());
                    for (name, ty) in &variant.fields {
                        out.text(name).ty(*ty);
                    }
                    out.nest(|out| {
                        variant.defaults.iter().for_each(|d| maybe_value(out, d.as_ref()));
                    });
                }
            });
        }
        program.tasks.iter().for_each(|t| task(out, t));
        task(out, &program.main);
        for test in &program.tests {
            out.line("Test").text(&test.name).atom(test.task).span(test.span);
        }
    });
    out.finish()
}

fn task(out: &mut Out, task: &mir::Task) {
    out.line("Task").text(task.name).span(task.span);
    out.atom(task.params).atom(task.type_params).ty(task.returns);
    out.tys(&task.locals).tys(&task.captures);
    out.ints(&task.change).ints(&task.take).ints(&task.borrowed);
    out.atom(task.body.len());
    out.nest(|out| mir_stmts(out, &task.body));
}

fn mir_stmts(out: &mut Out, stmts: &[mir::Stmt]) {
    stmts.iter().for_each(|s| mir_stmt(out, s));
}

fn mir_stmt(out: &mut Out, stmt: &mir::Stmt) {
    use mir::Stmt as S;
    let one = |out: &mut Out, value: &mir::Expr| out.nest(|out| value_node(out, value));
    match stmt {
        S::Set(local, value) => {
            out.line("Set").atom(local);
            one(out, value);
        }
        S::Bind(local, value) => {
            out.line("Bind").atom(local);
            one(out, value);
        }
        S::Update { local, op, args, span } => {
            out.line("Update").atom(local).atom(format!("{op:?}")).span(*span).atom(args.len());
            out.nest(|out| args.iter().for_each(|arg| value_node(out, arg)));
        }
        S::Clear(local) => _ = out.line("Clear").atom(local),
        S::Block(body) => {
            out.line("Block").atom(body.len());
            out.nest(|out| mir_stmts(out, body));
        }
        S::If { branches, otherwise } => {
            out.line("If").atom(branches.len()).atom(otherwise.len());
            out.nest(|out| {
                for (condition, body) in branches {
                    out.line("Branch").atom(body.len());
                    out.nest(|out| {
                        value_node(out, condition);
                        mir_stmts(out, body);
                    });
                }
                mir_stmts(out, otherwise);
            });
        }
        S::While { condition, body } => {
            out.line("While").atom(body.len());
            out.nest(|out| {
                value_node(out, condition);
                mir_stmts(out, body);
            });
        }
        S::For { local, start, end, inclusive, body } => {
            out.line("For").atom(local).flag(*inclusive).atom(body.len());
            out.nest(|out| {
                value_node(out, start);
                value_node(out, end);
                mir_stmts(out, body);
            });
        }
        S::Return(value) => {
            out.line("Return");
            one(out, value);
        }
        S::Exit(value) => {
            out.line("Exit");
            one(out, value);
        }
        S::Print(value) => {
            out.line("Print");
            one(out, value);
        }
        S::Expr(value) => {
            out.line("Expr");
            one(out, value);
        }
        S::Fail(value, span) => {
            out.line("Fail").span(*span);
            one(out, value);
        }
        S::Break => _ = out.line("Break"),
        S::Continue => _ = out.line("Continue"),
        S::Unreachable => _ = out.line("Unreachable"),
        S::PrintText(text) => _ = out.line("PrintText").text(text),
        S::Placeholder(span) => _ = out.line("Placeholder").span(*span),
        S::Trap(message, span) => _ = out.line("Trap").text(message).span(*span),
    }
}

fn maybe_value(out: &mut Out, value: Option<&mir::Expr>) {
    match value {
        Some(value) => value_node(out, value),
        None => out.absent(),
    }
}

fn key_tasks(out: &mut Out, keys: mir::KeyTasks) {
    match keys {
        Some((hash, equal)) => out.atom(hash).atom(equal),
        None => out.atom("-"),
    };
}

fn slots(out: &mut Out, fields: &[(usize, mir::Expr)]) {
    for (slot, value) in fields {
        out.line("Slot").atom(slot);
        out.nest(|out| value_node(out, value));
    }
}

fn value_node(out: &mut Out, value: &mir::Expr) {
    use mir::ExprKind as E;
    let v = value;
    match &value.kind {
        E::Int(n) => _ = head(out, "Int", v).atom(n),
        E::Float(x) => _ = head(out, "Float", v).atom(float(*x)),
        E::Bool(b) => _ = head(out, "Bool", v).flag(*b),
        E::None => _ = head(out, "None", v),
        E::Local(local) => _ = head(out, "Local", v).atom(local),
        E::Change(local) => _ = head(out, "Change", v).atom(local),
        E::Capture(at) => _ = head(out, "Capture", v).atom(at),
        E::Move(local) => _ = head(out, "Move", v).atom(local),
        E::Text(text) => _ = head(out, "Text", v).text(text),
        E::Call(task, args, values) => {
            head(out, "Call", v).atom(task).tys(args).atom(values.len());
            out.nest(|out| values.iter().for_each(|v| value_node(out, v)));
        }
        E::CallValue(callee, values) => {
            head(out, "CallValue", v).atom(values.len());
            out.nest(|out| {
                value_node(out, callee);
                values.iter().for_each(|v| value_node(out, v));
            });
        }
        E::Closure { task, args, counted, fields } => {
            head(out, "Closure", v).atom(task).tys(args).atom(counted).atom(fields.len());
            out.nest(|out| slots(out, fields));
        }
        E::New { tag, counted, fields } => {
            head(out, "New", v).atom(tag).atom(counted).atom(fields.len());
            out.nest(|out| slots(out, fields));
        }
        E::Join(values) | E::ListOf(values) => {
            let name = if matches!(value.kind, E::Join(_)) { "Join" } else { "ListOf" };
            head(out, name, v).atom(values.len());
            out.nest(|out| values.iter().for_each(|v| value_node(out, v)));
        }
        E::Foreign(symbol, values) | E::Method(symbol, values) => {
            let name = if matches!(value.kind, E::Foreign(..)) { "Foreign" } else { "Method" };
            head(out, name, v).text(symbol).atom(values.len());
            out.nest(|out| values.iter().for_each(|v| value_node(out, v)));
        }
        E::MapOf(entries, keys) => {
            head(out, "MapOf", v).atom(entries.len());
            key_tasks(out, *keys);
            out.nest(|out| {
                for (key, value) in entries {
                    out.line("Entry");
                    out.nest(|out| {
                        value_node(out, key);
                        value_node(out, value);
                    });
                }
            });
        }
        E::SetOf(inner, keys) => {
            head(out, "SetOf", v);
            key_tasks(out, *keys);
            nest_values(out, &[inner.as_ref()]);
        }
        E::Field(inner, slot) => {
            head(out, "Field", v).atom(slot);
            nest_values(out, &[inner.as_ref()]);
        }
        E::TagIn(inner, tags) => {
            head(out, "TagIn", v).ints(tags);
            nest_values(out, &[inner.as_ref()]);
        }
        E::Member { value: inner, variant, field } => {
            head(out, "Member", v).atom(variant).atom(field);
            nest_values(out, &[inner.as_ref()]);
        }
        E::Holds(inner, tys) => {
            head(out, "Holds", v).tys(tys);
            nest_values(out, &[inner.as_ref()]);
        }
        E::Temp(local, inner) => {
            head(out, "Temp", v).atom(local);
            nest_values(out, &[inner.as_ref()]);
        }
        E::Binary(op, left, right) | E::Wrapping(op, left, right) => {
            let name = if matches!(value.kind, E::Binary(..)) { "Binary" } else { "Wrapping" };
            head(out, name, v).atom(format!("{op:?}"));
            nest_values(out, &[left.as_ref(), right.as_ref()]);
        }
        E::Hash(op, left, right) => {
            head(out, "Hash", v).atom(format!("{op:?}"));
            nest_values(out, &[left.as_ref(), right.as_ref()]);
        }
        E::Seq(setup, inner) => {
            head(out, "Seq", v).atom(setup.len());
            out.nest(|out| {
                mir_stmts(out, setup);
                value_node(out, inner);
            });
        }
        E::Neg(a) => one(out, "Neg", v, a),
        E::Sqrt(a) => one(out, "Sqrt", v, a),
        E::Not(a) => one(out, "Not", v, a),
        E::Convert(a) => one(out, "Convert", v, a),
        E::ToText(a) => one(out, "ToText", v, a),
        E::TextQuote(a) => one(out, "TextQuote", v, a),
        E::TextLen(a) => one(out, "TextLen", v, a),
        E::ListPointer(a) => one(out, "ListPointer", v, a),
        E::TextIsEmpty(a) => one(out, "TextIsEmpty", v, a),
        E::TextByteLen(a) => one(out, "TextByteLen", v, a),
        E::TextBytes(a) => one(out, "TextBytes", v, a),
        E::Len(a) => one(out, "Len", v, a),
        E::Tag(a) => one(out, "Tag", v, a),
        E::Wrap(a) => one(out, "Wrap", v, a),
        E::View(a) => one(out, "View", v, a),
        E::Retain(a) => one(out, "Retain", v, a),
        E::TextCharAt(a, b) => two(out, "TextCharAt", v, a, b),
        E::ListGet(a, b) => two(out, "ListGet", v, a, b),
        E::ListHas(a, b) => two(out, "ListHas", v, a, b),
        E::MapGet(a, b) => two(out, "MapGet", v, a, b),
        E::MapHas(a, b) => two(out, "MapHas", v, a, b),
        E::MapNext(a, b) => two(out, "MapNext", v, a, b),
        E::MapKey(a, b) => two(out, "MapKey", v, a, b),
        E::MapValue(a, b) => two(out, "MapValue", v, a, b),
        E::MapFind(a, b) => two(out, "MapFind", v, a, b),
        E::If(a, b, c) => nest_values(head(out, "If", v), &[a.as_ref(), b.as_ref(), c.as_ref()]),
        E::TextSlice(a, b, c) => {
            nest_values(head(out, "TextSlice", v), &[a.as_ref(), b.as_ref(), c.as_ref()])
        }
        E::ListSlice(a, b, c) => {
            nest_values(head(out, "ListSlice", v), &[a.as_ref(), b.as_ref(), c.as_ref()])
        }
    }
}

fn head<'o>(out: &'o mut Out, name: &str, value: &mir::Expr) -> &'o mut Out {
    out.line(name).ty(value.ty).span(value.span)
}

fn nest_values(out: &mut Out, values: &[&mir::Expr]) {
    out.nest(|out| values.iter().for_each(|v| value_node(out, v)));
}

fn one(out: &mut Out, name: &str, value: &mir::Expr, a: &mir::Expr) {
    nest_values(head(out, name, value), &[a]);
}

fn two(out: &mut Out, name: &str, value: &mir::Expr, a: &mir::Expr, b: &mir::Expr) {
    nest_values(head(out, name, value), &[a, b]);
}

/// Reads what `program` writes with a `Source`: the `owned` dump, which the
/// Lucid front end hands the backend, by decision 216. Names and types are
/// leaked, as `check.rs` leaks them. Text that `program` cannot write is an
/// error, never a guess.
pub fn read(text: &str) -> Result<(mir::Program, Source), String> {
    let mut atoms = Atoms { text, at: 0 };
    let source = read_source(&mut atoms)?;
    let program = read_program(&mut atoms)?;
    match atoms.next()? {
        None => Ok((program, source)),
        Some(extra) => Err(format!("text after the program: `{extra}`")),
    }
}

/// The atoms of a dump in order. Indentation and line breaks are space. A
/// quoted atom keeps its quotes, so `"-"` is a text, not an absent value.
struct Atoms<'t> {
    text: &'t str,
    at: usize,
}

impl<'t> Atoms<'t> {
    fn next(&mut self) -> Result<Option<&'t str>, String> {
        let bytes = self.text.as_bytes();
        let mut start = self.at;
        while start < bytes.len() && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        if start == bytes.len() {
            self.at = start;
            return Ok(None);
        }
        let mut end = start;
        if bytes[start] == b'"' {
            end += 1;
            loop {
                match bytes.get(end) {
                    None => return Err("a text with no closing quote".into()),
                    Some(b'\\') => end += 2,
                    Some(b'"') => break end += 1,
                    Some(_) => end += 1,
                }
            }
        } else {
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
                end += 1;
            }
        }
        self.at = end.min(bytes.len());
        Ok(Some(&self.text[start..self.at]))
    }

    fn atom(&mut self) -> Result<&'t str, String> {
        self.next()?.ok_or_else(|| "the dump ends too soon".into())
    }

    /// True, and past it, if the next atom is `-`.
    fn absent(&mut self) -> Result<bool, String> {
        let at = self.at;
        if self.next()? == Some("-") {
            return Ok(true);
        }
        self.at = at;
        Ok(false)
    }

    fn word(&mut self, want: &str) -> Result<(), String> {
        match self.atom()? {
            got if got == want => Ok(()),
            got => Err(format!("expected `{want}`, found `{got}`")),
        }
    }

    fn number<N: std::str::FromStr>(&mut self) -> Result<N, String> {
        let atom = self.atom()?;
        atom.parse().map_err(|_| format!("expected a number, found `{atom}`"))
    }

    fn flag(&mut self) -> Result<bool, String> {
        match self.atom()? {
            "true" => Ok(true),
            "false" => Ok(false),
            got => Err(format!("expected `true` or `false`, found `{got}`")),
        }
    }

    fn text(&mut self) -> Result<String, String> {
        let atom = self.atom()?;
        let Some(inner) = atom.strip_prefix('"').and_then(|a| a.strip_suffix('"')) else {
            return Err(format!("expected a text, found `{atom}`"));
        };
        let bad = || "a bad escape in a text".to_string();
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let code = chars.as_str().strip_prefix('{').and_then(|r| r.split_once('}'));
                    let (hex, after) = code.ok_or_else(bad)?;
                    let c = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
                    out.push(c.ok_or_else(bad)?);
                    chars = after.chars();
                }
                _ => return Err(bad()),
            }
        }
        Ok(out)
    }

    fn name(&mut self) -> Result<&'static str, String> {
        Ok(self.text()?.leak())
    }

    fn span(&mut self) -> Result<Span, String> {
        Ok(Span { start: self.number()?, end: self.number()? })
    }

    fn ty(&mut self) -> Result<Ty, String> {
        let one = |atoms: &mut Self| atoms.ty().map(mir::inner);
        Ok(match self.atom()? {
            "int" => Ty::Int,
            "float" => Ty::Float,
            "bool" => Ty::Bool,
            "byte" => Ty::Byte,
            "i8" => Ty::I8,
            "i16" => Ty::I16,
            "i32" => Ty::I32,
            "u16" => Ty::U16,
            "u32" => Ty::U32,
            "u64" => Ty::U64,
            "f32" => Ty::F32,
            "none" => Ty::None,
            "pointer" => Ty::Pointer,
            "text" => Ty::Text,
            "socket" => Ty::Socket,
            "database" => Ty::Database,
            "sql" => Ty::Sql,
            "error" => Ty::Error,
            "list" => Ty::List(one(self)?),
            "map" => Ty::Map(one(self)?, one(self)?),
            "set" => Ty::Set(one(self)?),
            "spawned" => Ty::Spawned(one(self)?),
            "channel" => Ty::Channel(one(self)?),
            "lock" => Ty::Lock(one(self)?),
            "data" => Ty::Data(self.name()?, self.number()?, self.tys()?.leak()),
            "union" => Ty::Union(self.tys()?.leak()),
            "any" => Ty::Any(self.name()?),
            "task" => Ty::Task(self.tys()?.leak(), one(self)?),
            "param" => Ty::Param(self.name()?, self.number()?),
            got => return Err(format!("expected a type, found `{got}`")),
        })
    }

    fn tys(&mut self) -> Result<Vec<Ty>, String> {
        let count: usize = self.number()?;
        (0..count).map(|_| self.ty()).collect()
    }

    fn ints(&mut self) -> Result<Vec<usize>, String> {
        let count: usize = self.number()?;
        (0..count).map(|_| self.number()).collect()
    }
}

fn read_source(atoms: &mut Atoms) -> Result<Source, String> {
    atoms.word("Source")?;
    let count: usize = atoms.number()?;
    let grant = |atoms: &mut Atoms| match atoms.atom()? {
        "all" => Ok(Grant::All),
        "list" => {
            let count: usize = atoms.number()?;
            Ok(Grant::List((0..count).map(|_| atoms.text()).collect::<Result<_, _>>()?))
        }
        got => Err(format!("expected a grant, found `{got}`")),
    };
    let grants = if atoms.absent()? {
        None
    } else {
        let count: usize = atoms.number()?;
        Some((0..count).map(|_| grant(atoms)).collect::<Result<_, _>>()?)
    };
    let mut source = Source { text: atoms.text()?, grants, ..Source::default() };
    for _ in 0..count {
        atoms.word("File")?;
        let path = atoms.name()?;
        source.files.push(crate::diag::SourceFile { path, start: atoms.number()? });
    }
    Ok(source)
}

fn read_program(atoms: &mut Atoms) -> Result<mir::Program, String> {
    atoms.word("Program")?;
    let (datas, tasks, tests): (usize, usize, usize) =
        (atoms.number()?, atoms.number()?, atoms.number()?);
    let types = mir::Types::from(atoms.tys()?);
    let datas = (0..datas).map(|_| read_shape(atoms)).collect::<Result<_, _>>()?;
    let tasks = (0..tasks).map(|_| read_task(atoms)).collect::<Result<_, _>>()?;
    let main = read_task(atoms)?;
    let test = |atoms: &mut Atoms| {
        atoms.word("Test")?;
        let name = atoms.text()?;
        Ok::<_, String>(mir::TestCase { name, task: atoms.number()?, span: atoms.span()? })
    };
    let tests = (0..tests).map(|_| test(atoms)).collect::<Result<_, _>>()?;
    Ok(mir::Program {
        tasks,
        tests,
        unsafe_sites: Vec::new(),
        main,
        types,
        datas,
        methods: Default::default(),
        refs: Vec::new(),
    })
}

fn read_shape(atoms: &mut Atoms) -> Result<mir::Shape, String> {
    atoms.word("Shape")?;
    let choice = atoms.flag()?;
    let count: usize = atoms.number()?;
    let mut variants = Vec::with_capacity(count);
    for _ in 0..count {
        atoms.word("Variant")?;
        let name = atoms.text()?;
        let count: usize = atoms.number()?;
        let field = |atoms: &mut Atoms| Ok::<_, String>((atoms.text()?, atoms.ty()?));
        let fields = (0..count).map(|_| field(atoms)).collect::<Result<Vec<_>, _>>()?;
        let default = |atoms: &mut Atoms| {
            if atoms.absent()? { Ok(None) } else { expr(atoms).map(Some) }
        };
        let defaults = (0..count).map(|_| default(atoms)).collect::<Result<_, _>>()?;
        variants.push(mir::ShapeVariant { name, fields, defaults });
    }
    Ok(mir::Shape { variants, choice })
}

fn read_task(atoms: &mut Atoms) -> Result<mir::Task, String> {
    atoms.word("Task")?;
    let (name, span) = (atoms.name()?, atoms.span()?);
    let (params, type_params, returns) = (atoms.number()?, atoms.number()?, atoms.ty()?);
    let (locals, captures) = (atoms.tys()?, atoms.tys()?);
    let (change, take, borrowed) = (atoms.ints()?, atoms.ints()?, atoms.ints()?);
    let count = atoms.number()?;
    let body = stmts(atoms, count)?;
    Ok(mir::Task {
        name,
        params,
        type_params,
        returns,
        locals,
        body,
        span,
        captures,
        change,
        take,
        borrowed,
    })
}

fn stmts(atoms: &mut Atoms, count: usize) -> Result<Vec<mir::Stmt>, String> {
    (0..count).map(|_| stmt(atoms)).collect()
}

fn stmt(atoms: &mut Atoms) -> Result<mir::Stmt, String> {
    use mir::Stmt as S;
    Ok(match atoms.atom()? {
        "Set" => S::Set(atoms.number()?, expr(atoms)?),
        "Bind" => S::Bind(atoms.number()?, expr(atoms)?),
        "Update" => {
            let (local, op) = (atoms.number()?, debug_named(atoms.atom()?, &UPDATES)?);
            let (span, count) = (atoms.span()?, atoms.number()?);
            S::Update { local, op, args: exprs(atoms, count)?, span }
        }
        "Clear" => S::Clear(atoms.number()?),
        "Block" => {
            let count = atoms.number()?;
            S::Block(stmts(atoms, count)?)
        }
        "If" => {
            let (count, otherwise): (usize, usize) = (atoms.number()?, atoms.number()?);
            let mut branches = Vec::with_capacity(count);
            for _ in 0..count {
                atoms.word("Branch")?;
                let body = atoms.number()?;
                branches.push((expr(atoms)?, stmts(atoms, body)?));
            }
            S::If { branches, otherwise: stmts(atoms, otherwise)? }
        }
        "While" => {
            let body = atoms.number()?;
            S::While { condition: expr(atoms)?, body: stmts(atoms, body)? }
        }
        "For" => {
            let (local, inclusive, body) = (atoms.number()?, atoms.flag()?, atoms.number()?);
            let (start, end) = (expr(atoms)?, expr(atoms)?);
            S::For { local, start, end, inclusive, body: stmts(atoms, body)? }
        }
        "Return" => S::Return(expr(atoms)?),
        "Exit" => S::Exit(expr(atoms)?),
        "Print" => S::Print(expr(atoms)?),
        "Expr" => S::Expr(expr(atoms)?),
        "Fail" => {
            let span = atoms.span()?;
            S::Fail(expr(atoms)?, span)
        }
        "Break" => S::Break,
        "Continue" => S::Continue,
        "Unreachable" => S::Unreachable,
        "PrintText" => S::PrintText(atoms.text()?),
        "Placeholder" => S::Placeholder(atoms.span()?),
        "Trap" => S::Trap(atoms.name()?, atoms.span()?),
        got => return Err(format!("expected a statement, found `{got}`")),
    })
}

const UPDATES: [mir::Update; 10] = {
    use mir::Update as U;
    [
        U::ListPush,
        U::ListSet,
        U::ListSwap,
        U::MapSet,
        U::MapRemove,
        U::FieldSet,
        U::UnionSet,
        U::ListPop,
        U::Take,
        U::Put,
    ]
};

const OPERATORS: [mir::BinaryOp; 24] = {
    use mir::BinaryOp as B;
    [
        B::Fallback,
        B::Or,
        B::And,
        B::Eq,
        B::Ne,
        B::Lt,
        B::Le,
        B::Gt,
        B::Ge,
        B::In,
        B::NotIn,
        B::Range,
        B::RangeInclusive,
        B::Add,
        B::Sub,
        B::Mul,
        B::Div,
        B::FloorDiv,
        B::Rem,
        B::BitAnd,
        B::BitOr,
        B::BitXor,
        B::Shl,
        B::Shr,
    ]
};

const HASHES: [mir::HashOp; 3] = [mir::HashOp::Int, mir::HashOp::Text, mir::HashOp::Sum];

/// The item of `all` whose `Debug` name `program` wrote.
fn debug_named<T: Copy + std::fmt::Debug>(word: &str, all: &[T]) -> Result<T, String> {
    let found = all.iter().find(|item| format!("{item:?}") == word);
    found.copied().ok_or_else(|| format!("expected an operation, found `{word}`"))
}

fn read_keys(atoms: &mut Atoms) -> Result<mir::KeyTasks, String> {
    if atoms.absent()? { Ok(None) } else { Ok(Some((atoms.number()?, atoms.number()?))) }
}

fn exprs(atoms: &mut Atoms, count: usize) -> Result<Vec<mir::Expr>, String> {
    (0..count).map(|_| expr(atoms)).collect()
}

fn slots_of(atoms: &mut Atoms, count: usize) -> Result<Vec<(usize, mir::Expr)>, String> {
    let slot = |atoms: &mut Atoms| {
        atoms.word("Slot")?;
        Ok((atoms.number()?, expr(atoms)?))
    };
    (0..count).map(|_| slot(atoms)).collect()
}

fn expr(atoms: &mut Atoms) -> Result<mir::Expr, String> {
    use mir::ExprKind as E;
    let name = atoms.atom()?;
    let (ty, span) = (atoms.ty()?, atoms.span()?);
    let b = |atoms: &mut Atoms| expr(atoms).map(Box::new);
    let kind = match name {
        "Int" => E::Int(atoms.number()?),
        "Float" => E::Float(atoms.number()?),
        "Bool" => E::Bool(atoms.flag()?),
        "None" => E::None,
        "Local" => E::Local(atoms.number()?),
        "Change" => E::Change(atoms.number()?),
        "Capture" => E::Capture(atoms.number()?),
        "Move" => E::Move(atoms.number()?),
        "Text" => E::Text(atoms.text()?),
        "Call" => {
            let (task, args, count) = (atoms.number()?, atoms.tys()?.leak(), atoms.number()?);
            E::Call(task, args, exprs(atoms, count)?)
        }
        "CallValue" => {
            let count = atoms.number()?;
            E::CallValue(b(atoms)?, exprs(atoms, count)?)
        }
        "Closure" => {
            let (task, args) = (atoms.number()?, atoms.tys()?.leak());
            let (counted, count) = (atoms.number()?, atoms.number()?);
            E::Closure { task, args, counted, fields: slots_of(atoms, count)? }
        }
        "New" => {
            let (tag, counted, count) = (atoms.number()?, atoms.number()?, atoms.number()?);
            E::New { tag, counted, fields: slots_of(atoms, count)? }
        }
        "Join" | "ListOf" => {
            let count = atoms.number()?;
            let values = exprs(atoms, count)?;
            if name == "Join" { E::Join(values) } else { E::ListOf(values) }
        }
        "Foreign" | "Method" => {
            let (symbol, count) = (atoms.name()?, atoms.number()?);
            let values = exprs(atoms, count)?;
            if name == "Foreign" { E::Foreign(symbol, values) } else { E::Method(symbol, values) }
        }
        "MapOf" => {
            let count: usize = atoms.number()?;
            let keys = read_keys(atoms)?;
            let entry = |atoms: &mut Atoms| {
                atoms.word("Entry")?;
                Ok((expr(atoms)?, expr(atoms)?))
            };
            E::MapOf((0..count).map(|_| entry(atoms)).collect::<Result<_, String>>()?, keys)
        }
        "SetOf" => {
            let keys = read_keys(atoms)?;
            E::SetOf(b(atoms)?, keys)
        }
        "Field" => {
            let slot = atoms.number()?;
            E::Field(b(atoms)?, slot)
        }
        "TagIn" => {
            let tags = atoms.ints()?;
            E::TagIn(b(atoms)?, tags)
        }
        "Member" => {
            let (variant, field) = (atoms.number()?, atoms.number()?);
            E::Member { value: b(atoms)?, variant, field }
        }
        "Holds" => {
            let tys = atoms.tys()?;
            E::Holds(b(atoms)?, tys)
        }
        "Temp" => {
            let local = atoms.number()?;
            E::Temp(local, b(atoms)?)
        }
        "Binary" | "Wrapping" => {
            let op = debug_named(atoms.atom()?, &OPERATORS)?;
            let (left, right) = (b(atoms)?, b(atoms)?);
            if name == "Binary" { E::Binary(op, left, right) } else { E::Wrapping(op, left, right) }
        }
        "Hash" => E::Hash(debug_named(atoms.atom()?, &HASHES)?, b(atoms)?, b(atoms)?),
        "Seq" => {
            let count = atoms.number()?;
            E::Seq(stmts(atoms, count)?, b(atoms)?)
        }
        "Neg" => E::Neg(b(atoms)?),
        "Sqrt" => E::Sqrt(b(atoms)?),
        "Not" => E::Not(b(atoms)?),
        "Convert" => E::Convert(b(atoms)?),
        "ToText" => E::ToText(b(atoms)?),
        "TextQuote" => E::TextQuote(b(atoms)?),
        "TextLen" => E::TextLen(b(atoms)?),
        "ListPointer" => E::ListPointer(b(atoms)?),
        "TextIsEmpty" => E::TextIsEmpty(b(atoms)?),
        "TextByteLen" => E::TextByteLen(b(atoms)?),
        "TextBytes" => E::TextBytes(b(atoms)?),
        "Len" => E::Len(b(atoms)?),
        "Tag" => E::Tag(b(atoms)?),
        "Wrap" => E::Wrap(b(atoms)?),
        "View" => E::View(b(atoms)?),
        "Retain" => E::Retain(b(atoms)?),
        "TextCharAt" => E::TextCharAt(b(atoms)?, b(atoms)?),
        "ListGet" => E::ListGet(b(atoms)?, b(atoms)?),
        "ListHas" => E::ListHas(b(atoms)?, b(atoms)?),
        "MapGet" => E::MapGet(b(atoms)?, b(atoms)?),
        "MapHas" => E::MapHas(b(atoms)?, b(atoms)?),
        "MapNext" => E::MapNext(b(atoms)?, b(atoms)?),
        "MapKey" => E::MapKey(b(atoms)?, b(atoms)?),
        "MapValue" => E::MapValue(b(atoms)?, b(atoms)?),
        "MapFind" => E::MapFind(b(atoms)?, b(atoms)?),
        "If" => E::If(b(atoms)?, b(atoms)?, b(atoms)?),
        "TextSlice" => E::TextSlice(b(atoms)?, b(atoms)?, b(atoms)?),
        "ListSlice" => E::ListSlice(b(atoms)?, b(atoms)?, b(atoms)?),
        got => return Err(format!("expected a value, found `{got}`")),
    };
    Ok(mir::Expr { kind, ty, span })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_escape_only_what_they_must() {
        let mut out = String::new();
        quote(&mut out, "a\"b\\c\nd\u{1}\u{7f}é");
        assert_eq!(out, r#""a\"b\\c\nd\u{1}\u{7f}é""#);
    }

    #[test]
    fn floats_read_back() {
        assert_eq!(float(1e21), "1.0e21");
        assert_eq!(float(0.1), "0.1");
        assert_eq!(float(2.0), "2.0");
    }
}
