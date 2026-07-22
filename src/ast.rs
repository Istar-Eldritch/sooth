//! Sooth AST. Skeleton for Phase 0; grows as the language does.

#[derive(Debug, Default)]
pub struct Module {
    pub words: Vec<WordDef>,
}

#[derive(Debug)]
pub struct WordDef {
    pub name: String,
    pub effect: StackEffect,
    pub body: Vec<Term>,
}

/// A checked stack effect, e.g. `( a:int b:int -- int )`.
#[derive(Debug, Default)]
pub struct StackEffect {
    pub inputs: Vec<TypedSlot>,
    pub outputs: Vec<TypedSlot>,
}

#[derive(Debug)]
pub struct TypedSlot {
    pub name: Option<String>,
    pub ty: String,
}

#[derive(Debug)]
pub enum Term {
    IntLit(i64),
    /// A word invocation, or a reference to a named local.
    Call(String),
    If {
        then_branch: Vec<Term>,
        else_branch: Vec<Term>,
    },
    /// `begin ... until` (minimal loop form for Phase 0).
    BeginUntil {
        body: Vec<Term>,
    },
}
