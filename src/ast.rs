//! Sooth AST. Skeleton for Phase 0; grows as the language does.

/// A source location, 1-based (line, col).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Default)]
pub struct Module {
    pub words: Vec<WordDef>,
}

/// One REPL input unit: either a word definition or a bare term sequence
/// evaluated against the carried stack.
#[derive(Debug)]
pub enum Line {
    Def(WordDef),
    Expr(Vec<Term>),
}

#[derive(Debug)]
pub struct WordDef {
    pub name: String,
    pub effect: StackEffect,
    /// Names bound by `| ... |`, in effect order; empty if absent.
    pub locals: Vec<String>,
    pub body: Vec<Term>,
}

/// A checked stack effect, e.g. `( i64 i64 -- i64 )`. A slot may carry a name
/// (`a:i64`) as caller-facing documentation, but a slot bound by `| … |` stays a
/// bare type so a name is never written twice.
#[derive(Debug, Default)]
pub struct StackEffect {
    pub inputs: Vec<TypedSlot>,
    pub outputs: Vec<TypedSlot>,
}

#[derive(Debug)]
pub struct TypedSlot {
    pub name: Option<String>,
    pub ty: Type,
}

/// A frontend type. Slot types are concrete from Phase 2 Slice 1 onward; the
/// numeric tower, structs, enums, arrays, etc. are later slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    I64,
    Bool,
}

impl Type {
    /// Resolve a source type-name word to a `Type`, or `None` if unknown.
    pub fn from_name(name: &str) -> Option<Type> {
        match name {
            "i64" => Some(Type::I64),
            "bool" => Some(Type::Bool),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Type::I64 => "i64",
            Type::Bool => "bool",
        }
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug)]
pub struct Term {
    pub kind: TermKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum TermKind {
    IntLit(i64),
    /// A word invocation, or a reference to a named local.
    Call(String),
    If {
        then_branch: Vec<Term>,
        else_branch: Vec<Term>,
    },
}
