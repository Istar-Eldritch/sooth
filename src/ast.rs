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

/// A frontend type: the fixed-width integer tower (`i8..i64`, `u8..u64`) plus
/// `bool`. The eight integer cases are table-generated (`INT_TYPES` below), not
/// eight hand-written variants, so a further width is one table row. Structs,
/// enums, arrays, etc. are later slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Int(IntType),
    Bool,
}

/// The `(bits, signed)` pair for an integer type. Fields are private so a
/// `Type::Int` can only be built via `Type::from_name`/`Type::I64`, both of
/// which draw from `INT_TYPES`; an off-table width is then unconstructable
/// rather than merely a documented invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntType {
    bits: u8,
    signed: bool,
}

/// `(name, bits, signed)` rows for the eight integer types, driving
/// `Type::from_name`/`Type::name`.
const INT_TYPES: [(&str, u8, bool); 8] = [
    ("i8", 8, true),
    ("i16", 16, true),
    ("i32", 32, true),
    ("i64", 64, true),
    ("u8", 8, false),
    ("u16", 16, false),
    ("u32", 32, false),
    ("u64", 64, false),
];

impl Type {
    /// Sugar for the literal type (`i64`); kept to cut churn at call sites
    /// that only ever meant plain `i64`.
    pub const I64: Type = Type::Int(IntType {
        bits: 64,
        signed: true,
    });

    /// Resolve a source type-name word to a `Type`, or `None` if unknown.
    pub fn from_name(name: &str) -> Option<Type> {
        if name == "bool" {
            return Some(Type::Bool);
        }
        INT_TYPES
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, bits, signed)| {
                Type::Int(IntType {
                    bits: *bits,
                    signed: *signed,
                })
            })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Type::Bool => "bool",
            Type::Int(IntType { bits, signed }) => INT_TYPES
                .iter()
                .find(|(_, b, s)| b == bits && s == signed)
                .map(|(n, _, _)| *n)
                .expect("Type::Int is always constructed from an INT_TYPES row"),
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
    BoolLit(bool),
    /// A word invocation, or a reference to a named local.
    Call(String),
    If {
        then_branch: Vec<Term>,
        else_branch: Vec<Term>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_from_name_each_width_expected() {
        let cases: &[(&str, u8, bool)] = &[
            ("i8", 8, true),
            ("i16", 16, true),
            ("i32", 32, true),
            ("i64", 64, true),
            ("u8", 8, false),
            ("u16", 16, false),
            ("u32", 32, false),
            ("u64", 64, false),
        ];
        for (name, bits, signed) in cases {
            assert_eq!(
                Type::from_name(name),
                Some(Type::Int(IntType {
                    bits: *bits,
                    signed: *signed
                })),
                "resolving {name}"
            );
        }
        assert_eq!(Type::from_name("bool"), Some(Type::Bool));
    }

    #[test]
    fn type_unknown_name_none_expected() {
        assert_eq!(Type::from_name("i128"), None);
        assert_eq!(Type::from_name("u128"), None);
        assert_eq!(Type::from_name("f32"), None);
        assert_eq!(Type::from_name("foo"), None);
    }

    #[test]
    fn type_display_roundtrip_expected() {
        let names = ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "bool"];
        for name in names {
            let ty = Type::from_name(name).unwrap();
            assert_eq!(ty.name(), name);
            assert_eq!(ty.to_string(), name);
        }
    }
}
