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
    /// The per-program enum registry, parallel to `structs` and indexed by
    /// `EnumId`. A logically distinct registry from `structs` (D10): shares
    /// the layout/resolution machinery, not the struct registry's storage.
    pub enums: Vec<EnumDecl>,
    /// The per-program interned array-type registry (D3, M1): one entry per
    /// distinct `(element, count)` shape, indexed by `ArrayId` and deduped
    /// structurally, so two spellings of the same shape (e.g. two `[i64 4]`
    /// occurrences) share one entry. Populated during type resolution
    /// (`intern_array_type`), not by a name pre-pass: an array shape has no
    /// declared name to scan for ahead of parsing.
    pub arrays: Vec<ArrayDecl>,
    /// The per-program interned owning-cell registry: one entry per distinct
    /// payload-type shape, indexed by `OwnedCellId` and deduped structurally.
    /// Unlike `arrays` there is no count: a cell holds exactly one value.
    pub owned_cells: Vec<OwnedCellDecl>,
}

impl Module {
    /// Resolve a source type-name word to a `Type` against this module's
    /// struct and enum registries. Thin wrapper over the free
    /// `resolve_type_name`, the one resolver shared with the parser so
    /// effect-slot and field-type resolution can't drift apart.
    pub fn resolve_type_name(&self, name: &str) -> Option<Type> {
        resolve_type_name(&self.structs, &self.enums, name)
    }

    /// Intern an array shape `(element, count)` against this module's array
    /// registry. Thin wrapper over the free `intern_array_type`.
    pub fn intern_array_type(&mut self, element: Type, count: u32) -> Type {
        intern_array_type(&mut self.arrays, element, count)
    }

    /// Intern an owning-cell payload shape against this module's owned-cell
    /// registry. Thin wrapper over the free `intern_owned_cell_type`.
    pub fn intern_owned_cell_type(&mut self, payload: Type) -> Type {
        intern_owned_cell_type(&mut self.owned_cells, payload)
    }
}

/// Resolve a source type-name word to a `Type`: the scalar table first, then
/// `structs`, then `enums` (a struct/enum registry pair, in `Module` or
/// mid-parse). The single implementation both `Module::resolve_type_name` and
/// the parser call.
pub fn resolve_type_name(structs: &[StructDecl], enums: &[EnumDecl], name: &str) -> Option<Type> {
    Type::from_name(name)
        .or_else(|| {
            structs
                .iter()
                .position(|s| s.name == name)
                .map(|idx| Type::Struct(StructId(idx), structs[idx].name_static))
        })
        .or_else(|| {
            enums
                .iter()
                .position(|e| e.name == name)
                .map(|idx| Type::Enum(EnumId(idx), enums[idx].name_static))
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

/// A registered enum: its declared name, its ordered variants, and the
/// leaked `&'static str` copy of its name every `Type::Enum` naming it
/// carries directly (mirrors `StructDecl::name_static`).
#[derive(Debug)]
pub struct EnumDecl {
    pub name: String,
    pub name_static: &'static str,
    pub variants: Vec<VariantDecl>,
    pub span: Span,
}

/// One variant of an `EnumDecl`: its declared name, the leaked `&'static
/// str` copy of that name, and its ordered `(field-name, Type)` list (empty
/// for a zero-field variant).
#[derive(Debug)]
pub struct VariantDecl {
    pub name: String,
    pub name_static: &'static str,
    pub fields: Vec<(String, Type)>,
    pub span: Span,
}

/// A small `Copy` index into `Module::enums`, mirroring `StructId`. Two
/// `Type::Enum` values are equal iff they name the same registered enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumId(pub(crate) usize);

impl EnumId {
    /// Mint an `EnumId` for a registry position; crate-internal so an id is
    /// always tied to a real `Module::enums` entry.
    pub(crate) fn from_index(idx: usize) -> EnumId {
        EnumId(idx)
    }

    pub fn index(&self) -> usize {
        self.0
    }
}

/// A registered array type: its element type, compile-time count, and the
/// leaked `&'static str` spelling `[T N]` every `Type::Array` naming it
/// carries directly (mirrors `StructDecl::name_static`). Interned and deduped
/// structurally by `(element, count)` shape (D3, M1): two spellings of the
/// same shape share one `ArrayDecl`/`ArrayId`.
#[derive(Debug)]
pub struct ArrayDecl {
    pub element: Type,
    pub count: u32,
    pub name_static: &'static str,
}

/// A registered owning-cell type: its payload type and the leaked `&'static
/// str` spelling `^T` every `Type::OwnedCell` naming it carries directly.
/// Deduped structurally by payload shape; unlike `ArrayDecl` there is no
/// count, since a cell holds exactly one value.
#[derive(Debug)]
pub struct OwnedCellDecl {
    pub payload: Type,
    pub name_static: &'static str,
}

/// A small `Copy` index into `Module::owned_cells`, mirroring `ArrayId`. Two
/// `Type::OwnedCell` values are equal iff they name the same interned shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OwnedCellId(pub(crate) usize);

impl OwnedCellId {
    /// Mint an `OwnedCellId` for a registry position; crate-internal so an id
    /// is always tied to a real `owned_cells` registry entry.
    pub(crate) fn from_index(idx: usize) -> OwnedCellId {
        OwnedCellId(idx)
    }

    pub fn index(&self) -> usize {
        self.0
    }
}

/// Intern an owning-cell payload shape into `cells`, deduping structurally:
/// two calls with the same payload type return the same `OwnedCellId`.
/// Mirrors `intern_array_type`.
pub fn intern_owned_cell_type(cells: &mut Vec<OwnedCellDecl>, payload: Type) -> Type {
    if let Some(idx) = cells.iter().position(|d| d.payload == payload) {
        return Type::OwnedCell(OwnedCellId::from_index(idx), cells[idx].name_static);
    }
    let name = format!("^{}", payload.name());
    let name_static: &'static str = Box::leak(name.into_boxed_str());
    let id = OwnedCellId::from_index(cells.len());
    cells.push(OwnedCellDecl {
        payload,
        name_static,
    });
    Type::OwnedCell(id, name_static)
}

/// A small `Copy` index into `Module::arrays`, mirroring `StructId`/`EnumId`.
/// Two `Type::Array` values are equal iff they name the same interned shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArrayId(pub(crate) usize);

impl ArrayId {
    /// Mint an `ArrayId` for a registry position; crate-internal so an id is
    /// always tied to a real `arrays` registry entry.
    pub(crate) fn from_index(idx: usize) -> ArrayId {
        ArrayId(idx)
    }

    pub fn index(&self) -> usize {
        self.0
    }
}

/// Intern an array shape `(element, count)` into `arrays`, deduping
/// structurally: two calls with the same shape return the same `ArrayId`
/// (D3, M1). Interning mutates `arrays`, so callers thread it as `&mut Vec`
/// rather than through an otherwise-`&self` type resolver: unlike a struct or
/// enum name, an array shape has no declared name a pre-pass could register
/// ahead of parsing, so the registry grows as type expressions resolve.
pub fn intern_array_type(arrays: &mut Vec<ArrayDecl>, element: Type, count: u32) -> Type {
    if let Some(idx) = arrays
        .iter()
        .position(|d| d.element == element && d.count == count)
    {
        return Type::Array(ArrayId::from_index(idx), arrays[idx].name_static);
    }
    let name = format!("[{} {}]", element.name(), count);
    let name_static: &'static str = Box::leak(name.into_boxed_str());
    let id = ArrayId::from_index(arrays.len());
    arrays.push(ArrayDecl {
        element,
        count,
        name_static,
    });
    Type::Array(id, name_static)
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
    pub body: WordBody,
}

/// A word's body: either a term sequence with optional entry locals (the
/// Slice 0-3 form), or a clause list (a clause-style eliminator over the
/// word's enum top input, D4). A clause-style word has no word-entry locals
/// (D8).
#[derive(Debug)]
pub enum WordBody {
    Terms {
        /// Names bound by `| ... |`, in effect order; empty if absent.
        locals: Vec<String>,
        terms: Vec<Term>,
    },
    Clauses(Vec<Clause>),
}

/// One `|`-led clause of a clause-style word (D4): the matched variant name,
/// its optional clause-body `| names |` locals (payload then the stack below,
/// D7), and the body terms.
#[derive(Debug)]
pub struct Clause {
    pub variant: String,
    pub locals: Vec<String>,
    pub body: Vec<Term>,
    pub span: Span,
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
/// `bool`, plus a user-declared `struct`/`enum`/array. The eight integer
/// cases are table-generated (`INT_TYPES` below), not eight hand-written
/// variants, so a further width is one table row. `Type::Struct` carries a
/// `StructId` and the struct's leaked `&'static str` name so `Type` stays
/// `Copy` and self-renders without a registry (see `StructDecl::name_static`).
/// `Type::Array` mirrors this: an `ArrayId` into the interned `(element,
/// count)` registry plus the leaked `[T N]` spelling (D2, D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Int(IntType),
    Float(FloatType),
    Bool,
    Struct(StructId, &'static str),
    Enum(EnumId, &'static str),
    Array(ArrayId, &'static str),
    /// A single-value owning heap cell: a compiler-known type constructor,
    /// not a generic, one interned registry entry per concrete payload
    /// shape. Mirrors `Type::Array`. Always linear regardless of payload;
    /// see `is_copy`.
    OwnedCell(OwnedCellId, &'static str),
    /// The target-width unsigned integer (D7): distinct from every fixed-width
    /// `uN` in `INT_TYPES`, its size/align comes from the target word-width
    /// parameter (IR-side, Phase 3), never a hardcoded width here. The
    /// integer-tower operators (`+ - * mod and or xor not shl shr = < > <= >=
    /// <> .`, plus conversions) extend to it via `is_int`/`is_numeric`; the
    /// checker's D8 literal-coercion carve-out (a bare integer literal fills a
    /// `usize` position without an explicit `>usize`) lives in `check.rs`, not
    /// here, since `Type` carries no notion of "fresh literal".
    Usize,
    /// The target-width *signed* integer, mirroring `Usize` exactly: same
    /// word-width-derived size/align, same D8 literal-coercion carve-out, but
    /// prints and computes as signed.
    Isize,
    /// The test-only linear drop-spy primitive, spelled `__spy`: carries an
    /// `i64` tag, and its compiler-known destructor prints `drop <tag>`, so
    /// drop count, order, and timing are golden-observable.
    Spy,
}

/// The source spelling of the drop-spy type and of its constructor word (R6):
/// one name for both, resolved as a type by `Type::from_name` and as a word by
/// `check::builtin_table`.
pub const SPY_NAME: &str = "__spy";

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
        if name == "usize" {
            return Some(Type::Usize);
        }
        if name == "isize" {
            return Some(Type::Isize);
        }
        if name == SPY_NAME {
            return Some(Type::Spy);
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

    /// Whether this type is one of the eight integer types, or `usize` (not
    /// `bool`): every integer-tower operator (`mod`/`and`/`or`/`xor`/`not`/
    /// `shl`/`shr`) admits `usize` alongside the fixed widths (D7).
    pub fn is_int(&self) -> bool {
        matches!(self, Type::Int(_) | Type::Usize | Type::Isize)
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
            Type::Enum(_, name) => name,
            Type::Array(_, name) => name,
            Type::OwnedCell(_, name) => name,
            Type::Usize => "usize",
            Type::Isize => "isize",
            Type::Spy => SPY_NAME,
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
    fn type_spy_resolves_by_its_internal_name_and_is_not_numeric() {
        assert_eq!(Type::from_name(SPY_NAME), Some(Type::Spy));
        assert_eq!(Type::Spy.name(), "__spy");
        assert!(!Type::Spy.is_numeric());
        assert!(!Type::Spy.is_int());
        assert!(!Type::Spy.is_bool());
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
            "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "bool", "usize",
            "isize",
        ];
        for name in names {
            let ty = Type::from_name(name).unwrap();
            assert_eq!(ty.name(), name);
            assert_eq!(ty.to_string(), name);
        }
    }

    #[test]
    fn type_from_name_usize_expected() {
        assert_eq!(Type::from_name("usize"), Some(Type::Usize));
    }

    #[test]
    fn type_usize_is_int_and_numeric_not_float() {
        assert!(Type::Usize.is_int());
        assert!(Type::Usize.is_numeric());
        assert!(!Type::Usize.is_float());
        assert!(!Type::Usize.is_bool());
    }

    #[test]
    fn type_usize_distinct_from_every_int_width() {
        for name in ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"] {
            assert_ne!(Type::Usize, Type::from_name(name).unwrap());
        }
    }

    #[test]
    fn type_from_name_isize_expected() {
        assert_eq!(Type::from_name("isize"), Some(Type::Isize));
    }

    #[test]
    fn type_isize_is_int_and_numeric_not_float() {
        assert!(Type::Isize.is_int());
        assert!(Type::Isize.is_numeric());
        assert!(!Type::Isize.is_float());
        assert!(!Type::Isize.is_bool());
    }

    #[test]
    fn type_isize_distinct_from_usize_and_every_int_width() {
        assert_ne!(Type::Isize, Type::Usize);
        for name in ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"] {
            assert_ne!(Type::Isize, Type::from_name(name).unwrap());
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
            enums: Vec::new(),
            arrays: Vec::new(),
            owned_cells: Vec::new(),
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

    fn module_with_enum(name: &str, variants: Vec<VariantDecl>) -> Module {
        let name_static: &'static str = Box::leak(name.to_string().into_boxed_str());
        Module {
            words: Vec::new(),
            structs: Vec::new(),
            enums: vec![EnumDecl {
                name: name.to_string(),
                name_static,
                variants,
                span: Span::default(),
            }],
            arrays: Vec::new(),
            owned_cells: Vec::new(),
        }
    }

    fn variant(name: &str, fields: Vec<(String, Type)>) -> VariantDecl {
        VariantDecl {
            name: name.to_string(),
            name_static: Box::leak(name.to_string().into_boxed_str()),
            fields,
            span: Span::default(),
        }
    }

    #[test]
    fn module_resolve_type_name_finds_registered_enum() {
        let module = module_with_enum(
            "Shape",
            vec![variant("Circle", vec![("r".to_string(), Type::F64)])],
        );
        let ty = module.resolve_type_name("Shape").unwrap();
        match ty {
            Type::Enum(id, name) => {
                assert_eq!(id, EnumId(0));
                assert_eq!(name, "Shape");
            }
            other => panic!("expected Type::Enum, got {other:?}"),
        }
        assert_eq!(ty.name(), "Shape");
        assert_eq!(ty.to_string(), "Shape");
    }

    #[test]
    fn module_resolve_type_name_tries_structs_before_enums() {
        // A name registered as both a struct and an enum resolves to the
        // struct: struct-then-enum is only a stable tie-break order, the
        // checker's duplicate-name check (X2) rejects this collision outright.
        let name_static: &'static str = Box::leak("Dup".to_string().into_boxed_str());
        let module = Module {
            words: Vec::new(),
            structs: vec![StructDecl {
                name: "Dup".to_string(),
                name_static,
                fields: Vec::new(),
                span: Span::default(),
            }],
            enums: vec![EnumDecl {
                name: "Dup".to_string(),
                name_static,
                variants: vec![variant("V", vec![])],
                span: Span::default(),
            }],
            arrays: Vec::new(),
            owned_cells: Vec::new(),
        };
        assert!(matches!(
            module.resolve_type_name("Dup"),
            Some(Type::Struct(_, _))
        ));
    }

    #[test]
    fn type_enum_equality_is_by_enum_id() {
        let a = Type::Enum(EnumId(0), "Shape");
        let b = Type::Enum(EnumId(0), "Shape");
        let c = Type::Enum(EnumId(1), "Cmd");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn intern_array_type_same_shape_dedups_expected() {
        let mut arrays = Vec::new();
        let a = intern_array_type(&mut arrays, Type::I64, 4);
        let b = intern_array_type(&mut arrays, Type::I64, 4);
        assert_eq!(a, b);
        assert_eq!(arrays.len(), 1);
        match a {
            Type::Array(id, name) => {
                assert_eq!(id, ArrayId(0));
                assert_eq!(name, "[i64 4]");
            }
            other => panic!("expected Type::Array, got {other:?}"),
        }
        assert_eq!(a.to_string(), "[i64 4]");
    }

    #[test]
    fn intern_array_type_different_shapes_are_distinct_expected() {
        let mut arrays = Vec::new();
        let a = intern_array_type(&mut arrays, Type::I64, 4);
        let b = intern_array_type(&mut arrays, Type::I64, 8);
        let c = intern_array_type(&mut arrays, Type::F64, 4);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(arrays.len(), 3);
    }

    #[test]
    fn intern_array_type_nested_renders_bracket_within_bracket_expected() {
        let mut arrays = Vec::new();
        let inner = intern_array_type(&mut arrays, Type::I64, 4);
        let outer = intern_array_type(&mut arrays, inner, 4);
        assert_eq!(outer.to_string(), "[[i64 4] 4]");
    }
}
