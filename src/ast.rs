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
    /// The per-program struct registry: one entry per `type:` declaration,
    /// indexed by `StructId`. Populated by the parser pre-pass (names) and
    /// then by the `type:` production (fields).
    pub structs: Vec<StructDecl>,
}

impl Module {
    /// Resolve a source type-name word to a `Type` against this module's
    /// struct registry. Thin wrapper over the free `resolve_type_name`, the
    /// one resolver shared with the parser so effect-slot and
    /// struct-field-type resolution can't drift apart.
    pub fn resolve_type_name(&self, name: &str) -> Option<Type> {
        resolve_type_name(&self.structs, name)
    }
}

/// Resolve a source type-name word to a `Type`: the scalar table first, then
/// `structs` (a struct registry, in `Module` or mid-parse). The single
/// implementation both `Module::resolve_type_name` and the parser call.
pub fn resolve_type_name(structs: &[StructDecl], name: &str) -> Option<Type> {
    Type::from_name(name).or_else(|| {
        structs
            .iter()
            .position(|s| s.name == name)
            .map(|idx| Type::Struct(StructId(idx), structs[idx].name_static))
    })
}

/// A registered struct: its declared name, an ordered `(field-name, Type)`
/// list, and the leaked `&'static str` copy of its name every `Type::Struct`
/// naming it carries directly, so a struct name renders without threading
/// the registry through every diagnostic-formatting call site.
#[derive(Debug)]
pub struct StructDecl {
    pub name: String,
    pub name_static: &'static str,
    pub fields: Vec<(String, Type)>,
    pub span: Span,
}

/// A small `Copy` index into `Module::structs`. Two `Type::Struct` values are
/// equal iff they name the same registered struct; the field is
/// `pub(crate)` so only frontend/IR code within this crate can mint one, tied
/// to a real registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructId(pub(crate) usize);

impl StructId {
    /// Mint a `StructId` for a registry position; crate-internal so an id is
    /// always tied to a real `Module::structs` entry.
    pub(crate) fn from_index(idx: usize) -> StructId {
        StructId(idx)
    }

    pub fn index(&self) -> usize {
        self.0
    }
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
/// `bool`, plus a user-declared `struct`. The eight integer
/// cases are table-generated (`INT_TYPES` below), not eight hand-written
/// variants, so a further width is one table row. `Type::Struct` carries a
/// `StructId` and the struct's leaked `&'static str` name so `Type` stays
/// `Copy` and self-renders without a registry (see `StructDecl::name_static`).
/// Enums, arrays, etc. are later slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Int(IntType),
    Float(FloatType),
    Bool,
    Struct(StructId, &'static str),
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

/// The `bits` width for a float type, mirroring `IntType`. Fields are
/// private so a `Type::Float` can only be built via `Type::from_name`, which
/// draws from `FLOAT_TYPES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloatType {
    bits: u8,
}

impl FloatType {
    /// The width in bits (`32`/`64`).
    pub fn bits(&self) -> u8 {
        self.bits
    }
}

/// `(name, bits)` rows for the two float types, driving
/// `Type::from_name`/`Type::name`.
const FLOAT_TYPES: [(&str, u8); 2] = [("f32", 32), ("f64", 64)];

impl Type {
    /// Sugar for the literal type (`i64`); kept to cut churn at call sites
    /// that only ever meant plain `i64`.
    pub const I64: Type = Type::Int(IntType {
        bits: 64,
        signed: true,
    });

    /// Sugar for the default float-literal type (`f64`, D5).
    pub const F64: Type = Type::Float(FloatType { bits: 64 });

    /// Resolve a source type-name word to a `Type`, or `None` if unknown.
    pub fn from_name(name: &str) -> Option<Type> {
        if name == "bool" {
            return Some(Type::Bool);
        }
        if let Some((_, bits)) = FLOAT_TYPES.iter().find(|(n, _)| *n == name) {
            return Some(Type::Float(FloatType { bits: *bits }));
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

    /// Whether this type is one of the eight integer types (not `bool`).
    pub fn is_int(&self) -> bool {
        matches!(self, Type::Int(_))
    }

    /// Whether this type is one of the two float types.
    pub fn is_float(&self) -> bool {
        matches!(self, Type::Float(_))
    }

    /// Whether this type is numeric (int or float, not `bool`).
    pub fn is_numeric(&self) -> bool {
        self.is_int() || self.is_float()
    }

    /// Whether this type is `bool`.
    pub fn is_bool(&self) -> bool {
        matches!(self, Type::Bool)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Type::Bool => "bool",
            Type::Int(IntType { bits, signed }) => INT_TYPES
                .iter()
                .find(|(_, b, s)| b == bits && s == signed)
                .map(|(n, _, _)| *n)
                .expect("Type::Int is always constructed from an INT_TYPES row"),
            Type::Float(FloatType { bits }) => FLOAT_TYPES
                .iter()
                .find(|(_, b)| b == bits)
                .map(|(n, _)| *n)
                .expect("Type::Float is always constructed from a FLOAT_TYPES row"),
            Type::Struct(_, name) => name,
        }
    }
}

impl IntType {
    /// The width in bits (`8`/`16`/`32`/`64`).
    pub fn bits(&self) -> u8 {
        self.bits
    }

    /// Whether the type is signed.
    pub fn signed(&self) -> bool {
        self.signed
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
    FloatLit(f64),
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
        assert_eq!(Type::from_name("f128"), None);
        assert_eq!(Type::from_name("foo"), None);
    }

    #[test]
    fn type_display_roundtrip_expected() {
        let names = [
            "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "bool",
        ];
        for name in names {
            let ty = Type::from_name(name).unwrap();
            assert_eq!(ty.name(), name);
            assert_eq!(ty.to_string(), name);
        }
    }

    #[test]
    fn type_from_name_float_widths_expected() {
        let cases: &[(&str, u8)] = &[("f32", 32), ("f64", 64)];
        for (name, bits) in cases {
            assert_eq!(
                Type::from_name(name),
                Some(Type::Float(FloatType { bits: *bits })),
                "resolving {name}"
            );
        }
    }

    #[test]
    fn type_is_float_and_is_numeric_expected() {
        assert!(Type::from_name("f32").unwrap().is_float());
        assert!(Type::from_name("f64").unwrap().is_numeric());
        assert!(Type::from_name("i64").unwrap().is_numeric());
        assert!(!Type::from_name("i64").unwrap().is_float());
        assert!(!Type::Bool.is_numeric());
    }

    fn module_with_struct(name: &str, fields: Vec<(String, Type)>) -> Module {
        let name_static: &'static str = Box::leak(name.to_string().into_boxed_str());
        Module {
            words: Vec::new(),
            structs: vec![StructDecl {
                name: name.to_string(),
                name_static,
                fields,
                span: Span::default(),
            }],
        }
    }

    #[test]
    fn module_resolve_type_name_finds_registered_struct() {
        let module = module_with_struct("Vec2", vec![("x".to_string(), Type::I64)]);
        let ty = module.resolve_type_name("Vec2").unwrap();
        match ty {
            Type::Struct(id, name) => {
                assert_eq!(id, StructId(0));
                assert_eq!(name, "Vec2");
            }
            other => panic!("expected Type::Struct, got {other:?}"),
        }
        assert_eq!(ty.name(), "Vec2");
        assert_eq!(ty.to_string(), "Vec2");
    }

    #[test]
    fn module_resolve_type_name_prefers_scalar_table() {
        // A struct named like a scalar can't shadow it: `from_name` is tried
        // first, so `i64` always resolves to the scalar even if a
        // (nonsensical) struct of that name were registered.
        let module = module_with_struct("i64", vec![]);
        assert_eq!(module.resolve_type_name("i64"), Some(Type::I64));
    }

    #[test]
    fn module_resolve_type_name_unknown_is_none() {
        let module = Module::default();
        assert_eq!(module.resolve_type_name("Nope"), None);
    }

    #[test]
    fn type_struct_equality_is_by_struct_id() {
        let a = Type::Struct(StructId(0), "Vec2");
        let b = Type::Struct(StructId(0), "Vec2");
        let c = Type::Struct(StructId(1), "Segment");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
