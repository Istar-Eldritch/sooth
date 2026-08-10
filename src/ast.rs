//! Sooth AST. Skeleton for Phase 0; grows as the language does.

/// A source location, 1-based (line, col), plus the id of the file it came
/// from within its assembled `Module` (0 for a single-file program or REPL
/// line, where the field is never read). Load-bearing beyond diagnostics:
/// `Module::instantiations`/`Module::builtin_overloads` key a whole build's
/// per-call-site records by `Span` alone, and two files' tokens can land on
/// the identical (line, col) by coincidence, so `module` is what keeps two
/// unrelated calls in different files from colliding on one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
    pub module: u32,
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
    /// The per-program interned reference registry: one entry per distinct
    /// `(referent, mutable)` shape, indexed by `RefId` and deduped
    /// structurally. Mirrors `owned_cells`, with mutability as a second key
    /// component so `&T` and `&!T` are separate entries.
    pub refs: Vec<RefDecl>,
    /// One entry per `extern:` declaration (R1), in source order. Registered
    /// into the ordinary word environment (`check::check`) like any other
    /// word signature, so every existing arity/type check applies to a call
    /// site unchanged; the declaration itself carries the C symbol string a
    /// call site never sees.
    pub externs: Vec<ExternDecl>,
    /// R14 (phase 4 slice 1): one entry per call site of a polymorphic word,
    /// keyed by the call site's `Span`, emitted by the checker and consumed by
    /// lowering. Empty for a program with no polymorphic calls.
    pub instantiations: std::collections::HashMap<Span, CallInst>,
    /// Phase 4 slice 8a phase 2 (R7): the call sites that resolved to a user
    /// overload of a builtin-named word (e.g. `+` on two `Vec2`), keyed by the
    /// call site's `Span`, valued by the resolved callee's Sooth name. A
    /// sparse map mirroring `instantiations`: lowering consults it before its
    /// name-directed builtin dispatch, so a recorded site emits an
    /// `Instr::Call` to the user word instead of the builtin instruction. The
    /// corpus produces no records, so its lowering is untouched byte-for-byte.
    pub builtin_overloads: std::collections::HashMap<Span, String>,
    /// Phase 4 slice 5a (R10): one entry per file in the import closure, in
    /// topological order, module 0 being the entry file. A single-file program
    /// (and every REPL session) has exactly one entry. Every `StructDecl`/
    /// `EnumDecl`/`WordDef`/`ExternDecl` carries an owning module id indexing
    /// this vector; the entry carries that module's qualifier->module import
    /// map and its parsed `export:` list.
    pub modules: Vec<ModuleInfo>,
}

/// Phase 4 slice 5a (R10): per-module resolution context assembled by the
/// driver's closure resolution. Carries the import map (a qualifier binds to
/// the module id of the file it names) and the parsed export list; the export
/// list is recorded from phase 1 but not enforced until phase 2.
#[derive(Debug, Default, Clone)]
pub struct ModuleInfo {
    pub imports: std::collections::HashMap<String, u32>,
    pub exports: Vec<(String, Span)>,
    /// Phase 4 slice 5a phase 4 (R20/R15c): unqualified names this module
    /// selectively imports, name -> the target module id it resolves to.
    /// Built from every import's `| name... |` clause; a name naming a type
    /// is exposed the same way (R15c), since a type and its generated words
    /// resolve through the ordinary unqualified type/word lookup once the
    /// base name is in this map, with no separate enumeration of its
    /// accessors needed.
    pub selective: std::collections::HashMap<String, u32>,
}

/// Phase 4 slice 5a (R6): a parsed `import:` form:
/// `import: <qualifier> [ | <name>... | ] "<path>" ;`. The optional selective
/// name list is empty when the `| ... |` clause is absent (D9, phase 4). Spans
/// locate the `import:` keyword and each selective name for later diagnostics.
#[derive(Debug, Clone)]
pub struct Import {
    pub qualifier: String,
    pub selective: Vec<(String, Span)>,
    pub path: String,
    pub span: Span,
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

/// Phase 4 slice 5a (R8/R11): module-aware type-name resolution over the merged
/// registry. The scalar table wins first (a `Point` struct can no more shadow
/// `i64` here than in `resolve_type_name`). A bare name resolves against the
/// current module only (own-module-first; a same-named type in another module
/// is invisible unqualified until selective import, phase 4). A `q::Base` name
/// maps `q` through the current module's import map to a target module and
/// resolves `Base` there. Called by the parser while decl names are still raw,
/// so the `name ==` comparisons are raw-against-raw.
pub fn resolve_type_name_in_module(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    name: &str,
    module: u32,
    imports: &std::collections::HashMap<String, u32>,
    selective: &std::collections::HashMap<String, u32>,
) -> Option<Type> {
    if let Some(t) = Type::from_name(name) {
        return Some(t);
    }
    if let Some((qualifier, base)) = name.split_once("::") {
        let target = *imports.get(qualifier)?;
        return find_type_in_module(structs, enums, base, target);
    }
    find_type_in_module(structs, enums, name, module).or_else(|| {
        // R15c (phase 4): a selectively imported type's bare name resolves
        // unqualified against its target module, the same one unit its
        // generated words resolve through in `resolve.rs`'s `NameTables`.
        let target = *selective.get(name)?;
        find_type_in_module(structs, enums, name, target)
    })
}

fn find_type_in_module(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    name: &str,
    module: u32,
) -> Option<Type> {
    // R8d (slice 5b): match `name_static`, not `name`. A REPL-spliced imported
    // type tags its `.name` with an import epoch so the accessor/constructor
    // recognizers agree on one row per internal spelling, but its `.name_static`
    // stays the pretty user-typed base spelling; a type-position reference must
    // resolve against that. Behavior-preserving for every native call: this
    // resolver only ever runs pre-`resolve_modules`, where `.name == .name_static`
    // for every decl, so the switched field compares identically there.
    structs
        .iter()
        .enumerate()
        .find(|(_, s)| s.name_static == name && s.module == module)
        .map(|(idx, s)| Type::Struct(StructId(idx), s.name_static))
        .or_else(|| {
            enums
                .iter()
                .enumerate()
                .find(|(_, e)| e.name_static == name && e.module == module)
                .map(|(idx, e)| Type::Enum(EnumId(idx), e.name_static))
        })
}

/// A registered struct: its declared name, an ordered `(field-name, Type)`
/// list, and the leaked `&'static str` copy of its name every `Type::Struct`
/// naming it carries directly, so a struct name renders without threading
/// the registry through every diagnostic-formatting call site.
#[derive(Debug, Clone)]
pub struct StructDecl {
    pub name: String,
    pub name_static: &'static str,
    pub fields: Vec<(String, Type)>,
    pub span: Span,
    /// R1/R2/R3 (slice 8b): whether a user `: drop ( T -- )` overload was
    /// recognized for this struct. A separately *set* bit, never re-derived
    /// from the fields (which is exactly what it overrides), mirroring how
    /// the IR-side `StructLayout::is_linear` is a computed-once bit rather
    /// than a predicate. Recording it on the declaration is what makes the
    /// fact reach every `is_copy` call site, the layout fold, and the REPL's
    /// persistent registries without threading a table through any of them.
    pub has_drop_overload: bool,
    /// R10 (phase 4 slice 1): whether this is a synthesized *return bundle*
    /// rather than a user `type:` declaration — the aggregate a word with two
    /// or more outputs returns through. A separately set bit, never re-derived
    /// from the fields (nothing about them says "bundle"), mirroring
    /// `has_drop_overload`. It rides the ordinary struct registry so the layout
    /// pass sizes it like any other struct, and `StructLayout::bundle` carries
    /// it on to destructor synthesis, which skips a bundle: its fields are the
    /// caller's outputs, moved out by the unpack in the same breath, so drop
    /// glue here would double-free a linear one.
    pub is_bundle: bool,
    /// Phase 4 slice 5a (R10): the owning module id (index into
    /// `Module::modules`). `0` for a single-file program's decls and for a
    /// synthesized bundle. Two modules may each declare a `Point`; the id is
    /// how the merged registry keeps them apart (R12).
    pub module: u32,
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
    /// Phase 4 slice 5a (R10): the owning module id, mirroring
    /// `StructDecl::module`.
    pub module: u32,
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

/// Slice 9 (R2): the reserved registry position of the builtin `bool` enum.
/// Injected at index 0 of every assembled module's enum registry ahead of any
/// user enum (`bool_enum_decl`), so `Type::from_name("bool")` resolves to one
/// fixed `Type::Enum` without threading the registry through a pure resolver.
pub const BOOL_ENUM_ID: EnumId = EnumId(0);

/// Slice 9 (R2): the builtin `bool` enum declaration, `type: Bool | False | True ;`,
/// injected at `BOOL_ENUM_ID` in every assembled module. `False` is variant 0
/// and `True` variant 1 by declaration order, so their discriminants are the
/// `0`/`1` the retired `TermKind::BoolLit` produced (and the order the print
/// table `$boolstrs` indexes). Both variants carry an empty payload, so the
/// general zero-payload-enum layout rule lowers it to a bare scalar.
pub fn bool_enum_decl() -> EnumDecl {
    EnumDecl {
        name: "bool".to_string(),
        name_static: "bool",
        variants: vec![
            VariantDecl {
                name: "False".to_string(),
                name_static: "False",
                fields: Vec::new(),
                span: Span::default(),
            },
            VariantDecl {
                name: "True".to_string(),
                name_static: "True",
                fields: Vec::new(),
                span: Span::default(),
            },
        ],
        span: Span::default(),
        module: 0,
    }
}

/// Slice 9 phase 2 (R6): the library `.` overload for `bool`, injected into
/// every assembled module's `words` (and REPL session at startup) exactly as
/// `bool_enum_decl` injects the enum itself. Clause-matches `False`/`True` and
/// prints `false`/`true` including the trailing newline the retired
/// primitive `bool` printable row used to emit, by delegating to the still-
/// primitive `str` row -- reached at call sites through 8a's
/// `builtin_overloads` dispatch, not a checker builtin row.
pub fn bool_print_word_def() -> WordDef {
    fn clause(variant: &str, text: &str) -> Clause {
        Clause {
            variant: variant.to_string(),
            locals: Vec::new(),
            body: vec![
                Term {
                    kind: TermKind::StrLit(text.to_string()),
                    span: Span::default(),
                },
                Term {
                    kind: TermKind::Call(".".to_string()),
                    span: Span::default(),
                },
            ],
            span: Span::default(),
        }
    }
    WordDef {
        name: ".".to_string(),
        effect: StackEffect {
            inputs: vec![TypedSlot {
                name: None,
                ty: Type::BOOL,
            }],
            outputs: Vec::new(),
        },
        body: WordBody::Clauses(vec![clause("False", "false\n"), clause("True", "true\n")]),
        poly: None,
        module: 0,
        span: Span::default(),
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

/// A registered reference type: its referent type, whether it is mutable
/// (`&!T`) or shared (`&T`), and the leaked `&'static str` spelling every
/// `Type::Ref` naming it carries directly. Deduped structurally by
/// `(referent, mutable)`; a reference never owns, so unlike `OwnedCellDecl`
/// there is nothing to free.
#[derive(Debug)]
pub struct RefDecl {
    pub referent: Type,
    pub mutable: bool,
    pub name_static: &'static str,
}

/// A small `Copy` index into `Module::refs`, mirroring `OwnedCellId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RefId(pub(crate) usize);

impl RefId {
    /// Mint a `RefId` for a registry position; crate-internal so an id is
    /// always tied to a real `refs` registry entry.
    pub(crate) fn from_index(idx: usize) -> RefId {
        RefId(idx)
    }

    pub fn index(&self) -> usize {
        self.0
    }
}

/// Intern a `(referent, mutable)` reference shape into `refs`, deduping
/// structurally. Mirrors `intern_owned_cell_type`.
pub fn intern_ref_type(refs: &mut Vec<RefDecl>, referent: Type, mutable: bool) -> Type {
    if let Some(idx) = refs
        .iter()
        .position(|d| d.referent == referent && d.mutable == mutable)
    {
        return Type::Ref(RefId::from_index(idx), mutable, refs[idx].name_static);
    }
    let name = format!("&{}{}", if mutable { "!" } else { "" }, referent.name());
    let name_static: &'static str = Box::leak(name.into_boxed_str());
    let id = RefId::from_index(refs.len());
    refs.push(RefDecl {
        referent,
        mutable,
        name_static,
    });
    Type::Ref(id, mutable, name_static)
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

/// R10 (phase 4 slice 1): intern the synthesized return-bundle struct for a
/// word's `outputs` tuple (two or more outputs), deduping structurally by that
/// tuple exactly as `intern_array_type` dedups an array shape. The checker
/// calls this so the bundle is in `Module::structs` before the layout pass;
/// lowering only reads it back (`Structs::bundle_for`).
pub fn intern_bundle_struct(structs: &mut Vec<StructDecl>, outputs: &[Type]) -> StructId {
    if let Some(idx) = structs.iter().position(|d| {
        d.is_bundle
            && d.fields.len() == outputs.len()
            && d.fields.iter().zip(outputs).all(|((_, f), o)| f == o)
    }) {
        return StructId::from_index(idx);
    }
    let id = StructId::from_index(structs.len());
    // Positional, like the backend's array type symbols: an output tuple's own
    // spelling (`[i64 4]`, `&!Buf`) is not a legal QBE aggregate name, and the
    // name is never the dedup key.
    let name = format!("__ret_{}", structs.len());
    let name_static: &'static str = Box::leak(name.clone().into_boxed_str());
    structs.push(StructDecl {
        name,
        name_static,
        fields: outputs
            .iter()
            .enumerate()
            .map(|(i, ty)| (format!("f{i}"), *ty))
            .collect(),
        span: Span::default(),
        has_drop_overload: false,
        is_bundle: true,
        module: 0,
    });
    id
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
    /// The concrete stack effect. For a **polymorphic** word (`poly` is
    /// `Some`) this is left empty: the signature lives entirely in `poly`,
    /// and every concrete path (env registration, monomorphic body checking,
    /// bundle interning) skips such a word, so no variable is ever forced into
    /// a concrete `Type` slot (R4/S1).
    pub effect: StackEffect,
    pub body: WordBody,
    /// R4 (phase 4 slice 1): the polymorphic signature, present only when the
    /// declared effect mentions a type variable `'T`, a length variable `'N`,
    /// or the row variable `..s`. `None` for a monomorphic word, whose whole
    /// signature is `effect`.
    pub poly: Option<Box<PolySig>>,
    /// Phase 4 slice 5a (R10): the owning module id, mirroring
    /// `StructDecl::module`.
    pub module: u32,
    /// The declaration site (the word's name token), used by every
    /// diagnostic that must point at this word regardless of its body shape.
    /// Kept separate from `word_span`'s old first-term/first-clause fallback
    /// (which is `Span::default()`, i.e. line 0 col 0, for an empty body --
    /// `: main ( -- ) ;` and every other trivial stub word hit this) so a
    /// located error always has somewhere real to point.
    pub span: Span,
}

/// R3/R6 (phase 4 slice 1): a capability a type variable can be bounded by.
/// `Copy` gates `dup`/`over`; `Ord` gates `>`/`max`. Both are resolved at the
/// concrete instantiation by the existing predicates (`is_copy`, the numeric
/// tower), Kitten-style, with no trait objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Copy,
    Ord,
}

/// R4: an array count in a polymorphic type: a concrete length or a length
/// variable `'N` (index into `PolySig::len_var_names`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Len {
    Concrete(u32),
    Var(u32),
}

/// R4: a type in a polymorphic signature. A monomorphic sub-type folds to
/// `Concrete`; a variable-bearing array (`['T 'N]`, `[i64 'N]`, `['T 4]`)
/// stays `Array`. `Type` itself gains **no** variant (S1): the variable forms
/// live only here, in a word's declared effect and in call-site unification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolyType {
    Concrete(Type),
    /// A type variable (index into `PolySig::ty_var_names`).
    Var(u32),
    Array(Box<PolyType>, Len),
    /// Slice 6a (R5): a declared quotation effect whose rows may mention the
    /// signature's type/length variables (`[ 'T -- ]` where `'T` is the
    /// element variable). Folds to `Concrete(Type::Quotation(..))` when fully
    /// concrete (`raw_to_poly_type`); only a variable-bearing effect stays
    /// here.
    Quotation(Vec<PolyType>, Vec<PolyType>),
}

/// R4: a polymorphic stack effect. The variable id spaces are per-signature
/// (a `Var(0)` in one word is unrelated to a `Var(0)` in another); the
/// `*_var_names` tables carry each id's surface spelling for diagnostics.
#[derive(Debug, Clone)]
pub struct PolySig {
    /// The input row variable (`..s` at the deepest input slot), if any.
    pub row_in: Option<u32>,
    pub inputs: Vec<PolyType>,
    pub outputs: Vec<PolyType>,
    /// The output row variable, if any; the same id as `row_in` when the
    /// same `..s` name passes through.
    pub row_out: Option<u32>,
    pub bounds: Vec<(u32, Bound)>,
    pub ty_var_names: Vec<String>,
    pub len_var_names: Vec<String>,
    pub row_var_names: Vec<String>,
}

impl PolySig {
    /// Whether type variable `id` carries `bound`.
    pub fn has_bound(&self, id: u32, bound: Bound) -> bool {
        self.bounds.iter().any(|(v, b)| *v == id && *b == bound)
    }
}

/// R5/R14: a ground substitution `θ` resolved by unifying a `PolySig` against
/// a concrete call-site stack. Kept as sorted `(id, value)` vectors so it is
/// deterministic (the mangled symbol depends on it) and cheaply `Eq`
/// (specializations dedup structurally).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Subst {
    pub ty: Vec<(u32, Type)>,
    pub len: Vec<(u32, u32)>,
}

impl Subst {
    /// The concrete type variable `id` resolved to.
    pub fn ty_of(&self, id: u32) -> Option<Type> {
        self.ty.iter().find(|(v, _)| *v == id).map(|(_, t)| *t)
    }

    /// The concrete length variable `id` resolved to.
    pub fn len_of(&self, id: u32) -> Option<u32> {
        self.len.iter().find(|(v, _)| *v == id).map(|(_, n)| *n)
    }
}

/// R14: the per-call-site instantiation record the checker emits for a call to
/// a polymorphic word, keyed by the call site's `Span` in
/// `Module::instantiations`. Lowering reads exactly what the
/// name-keyed `Resolver`/`Arity` structurally cannot supply per call site: the
/// ground `θ`, the mangled callee symbol, the concrete output arity, the
/// ordered concrete output types, and the bundle `StructId` when the output
/// count is `>= 2`.
#[derive(Debug, Clone)]
pub struct CallInst {
    pub callee: String,
    pub subst: Subst,
    pub symbol: String,
    pub out_arity: usize,
    pub output_types: Vec<Type>,
    pub bundle: Option<StructId>,
    /// D1: `None` for a native instantiation; `Some(g)` for a REPL
    /// instantiation minted against a polymorphic word's generation `g`
    /// (Slice 2), the same `__gen{N}` device `mangled_symbol` uses for
    /// ordinary REPL words.
    pub generation: Option<u64>,
}

/// R9/R14: the mangled symbol for one instantiation `(word, θ)`. A pure,
/// deterministic function of its inputs with no lowering-order dependence, so
/// the checker's call-site table and the lowered `IrFunc.name` are minted from
/// one source of truth and can never disagree. Mirrors `struct_drop_symbol`'s
/// positional, id-based shape (a word name or a type spelling may hold
/// characters no QBE symbol admits, so both are sanitized here).
pub fn instantiation_symbol(word: &str, subst: &Subst, generation: Option<u64>) -> String {
    fn sanitize(s: &str) -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    }
    let mut parts = Vec::new();
    for (id, ty) in &subst.ty {
        parts.push(format!("t{id}_{}", sanitize(ty.name())));
    }
    for (id, n) in &subst.len {
        parts.push(format!("n{id}_{n}"));
    }
    let base = format!("sooth_mono_{}__{}", sanitize(word), parts.join("_"));
    match generation {
        None => base,
        Some(g) => format!("{base}__gen{g}"),
    }
}

/// Slice 8a fix 1 (R1): the distinct lowering symbol for every word in
/// `words`, aligned by index. Equal to the word's own name (already
/// module-mangled by `resolve::mangle` by the time this runs), except within
/// a run of same-named candidates -- an overload set, which R1's widened
/// `(module, name, input_types)` duplicate-word key admits -- where each
/// candidate gets a deterministic `$$N` suffix, `N` counting occurrences in
/// declaration order, so two overloads of one name never collide on a single
/// QBE symbol the way two same-named words used to before the `qbe_name`
/// fix. Shared by `check::check` (which records a resolved call site's
/// symbol here) and `ir::lower` (which mints each overloaded `WordDef`'s
/// `IrFunc` under it), so the two can never disagree about which symbol a
/// given candidate lowers under.
///
/// `drop`-named words are exempt from grouping: `find_drop_overloads`
/// dispatches every `drop` call by the operand's `StructId`, never by name
/// (`resolve.rs`'s own doc on this), so however many `drop`s a module
/// declares, they never collide regardless of count, and suffixing them
/// would only be inert churn.
pub fn overload_symbols(words: &[WordDef]) -> Vec<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for w in words {
        if w.name != "drop" {
            *counts.entry(w.name.as_str()).or_insert(0) += 1;
        }
    }
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    words
        .iter()
        .map(|w| {
            if w.name == "drop" || counts.get(w.name.as_str()).copied().unwrap_or(0) <= 1 {
                return w.name.clone();
            }
            let i = seen.entry(w.name.as_str()).or_insert(0);
            let sym = format!("{}$${i}", w.name);
            *i += 1;
            sym
        })
        .collect()
}

/// One `extern:` declaration (R1): a typed foreign-call binding. `symbol` is
/// the explicit C symbol string, kept separate from `name` because a Sooth
/// word name may use characters C cannot (`&!S>fi`), and because binding a
/// C name like `open` to a differently-named Sooth word must be possible.
#[derive(Debug)]
pub struct ExternDecl {
    pub name: String,
    pub symbol: String,
    pub effect: StackEffect,
    pub span: Span,
    /// Phase 4 slice 5a (R10): the owning module id, mirroring
    /// `StructDecl::module`.
    pub module: u32,
}

/// A word's body: either a term sequence, or a clause list (a clause-style
/// eliminator over the word's enum top input, D4). Entry locals are not a
/// field here: a `| names |` binding is a `TermKind::Bind` term like any
/// other, and the entry position is just the first one (R1). A clause-style
/// word has no word-entry locals (D8).
#[derive(Debug)]
pub enum WordBody {
    Terms { terms: Vec<Term> },
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
    Struct(StructId, &'static str),
    Enum(EnumId, &'static str),
    Array(ArrayId, &'static str),
    /// A single-value owning heap cell: a compiler-known type constructor,
    /// not a generic, one interned registry entry per concrete payload
    /// shape. Mirrors `Type::Array`. Always linear regardless of payload;
    /// see `is_copy`.
    OwnedCell(OwnedCellId, &'static str),
    /// A borrowing reference, `&T` (shared) or `&!T` (mutable): a `RefId` into
    /// the interned `(referent, mutable)` registry, the mutability, and the
    /// leaked `&T`/`&!T` spelling. Mutability is carried in the variant as
    /// well as in the registry entry because it is the *classification* bit
    /// (`is_copy`, linearity, store-vs-fetch), asked at sites that hold no
    /// registry; the referent, asked only where a projection or access is
    /// being typed, stays behind the id so `Type` remains `Copy`.
    Ref(RefId, bool, &'static str),
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
    /// Pointer + length, and the length is the only thing it promises (R4):
    /// authoritative for every Sooth-side use, never discovered by scanning.
    /// Deliberately *not* `byte[len] == 0`; the terminator behind every `str`
    /// in this slice comes from literal lowering (R6) and is a precondition of
    /// the `cstr` conversion (R7), so a later `str` viewing part of a buffer
    /// breaks one word rather than this type. `Copy` (R10), never seen through
    /// by `contains_reference` (its `Ptr` component is opaque, not a
    /// `Type::Ref`), and constructible only by a literal (R11), which is
    /// what makes both of those sound.
    Str,
    /// Pointer-only, NUL-terminated, length unknown (Zig's `[*:0]const u8`,
    /// R5): what a C `char*` parameter wants and what one hands back. `Copy`
    /// like `Str`, for the same reason.
    Cstr,
    /// Slice 6a (R4): a quotation effect type `[ <inputs> -- <outputs> ]`, the
    /// type a word declares for a quotation parameter. Holds a `&'static`
    /// `QuotEffect` carrying the declared input/output rows and the leaked
    /// `[ ... -- ... ]` spelling, so `Type` stays `Copy` and self-renders like
    /// every other variant. Structural `PartialEq` through the reference gives
    /// value equality (what unification needs) with no interning table to
    /// thread. **No "statically known" bit** (D6): the type says only "a
    /// quotation of this effect", never that a literal is known here; knownness
    /// stays on the checker's `Slot.quot`. Never lowered to an `IrType` this
    /// slice: a quotation-taking word mints no standalone `IrFunc` (R20), so
    /// this type never reaches the backend (the runtime representation is
    /// slice 7).
    Quotation(&'static QuotEffect),
}

/// Slice 6a (R4): a declared quotation effect, the payload behind
/// `Type::Quotation`. Leaked as a `&'static` (like `ArrayDecl::name_static`)
/// rather than threaded through a per-module registry, since it needs no dedup
/// key beyond its own structural equality. Derived `PartialEq`/`Eq` give the
/// value equality unification relies on; `name_static` is a pure function of
/// the rows, so comparing it too is harmless.
#[derive(Debug, PartialEq, Eq)]
pub struct QuotEffect {
    pub inputs: Vec<Type>,
    pub outputs: Vec<Type>,
    pub name_static: &'static str,
}

/// Build a `Type::Quotation` for a declared effect, leaking its rows and its
/// rendered `[ ... -- ... ]` spelling. Two quotation types with equal rows
/// compare equal through the `&'static` reference, so a repeated spelling is
/// harmless duplication, never a correctness hazard.
pub fn quotation_type(inputs: Vec<Type>, outputs: Vec<Type>) -> Type {
    let name = render_quotation_effect(&inputs, &outputs);
    let name_static: &'static str = Box::leak(name.into_boxed_str());
    let eff: &'static QuotEffect = Box::leak(Box::new(QuotEffect {
        inputs,
        outputs,
        name_static,
    }));
    Type::Quotation(eff)
}

/// Render a quotation effect's spelling `[ <in>... -- <out>... ]`. The nil
/// effect renders `[ -- ]`.
fn render_quotation_effect(inputs: &[Type], outputs: &[Type]) -> String {
    let mut s = String::from("[ ");
    for t in inputs {
        s.push_str(t.name());
        s.push(' ');
    }
    s.push_str("--");
    for t in outputs {
        s.push(' ');
        s.push_str(t.name());
    }
    s.push_str(" ]");
    s
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

    /// Slice 9 (D-A/R2): `bool` is no longer a primitive scalar type but the
    /// two-variant zero-payload enum `type: Bool | False | True ;`, injected
    /// at the reserved head of every module's enum registry (`BOOL_ENUM_ID`).
    /// `Type::from_name("bool")` and every checker/IR spelling of the boolean
    /// type is this one canonical `Type::Enum`; its representation stays a
    /// register-resident scalar through the general zero-payload-enum layout
    /// rule (`ir::EnumLayout::scalar`), never a per-`Bool` carve-out.
    pub const BOOL: Type = Type::Enum(BOOL_ENUM_ID, "bool");

    /// Resolve a source type-name word to a `Type`, or `None` if unknown.
    pub fn from_name(name: &str) -> Option<Type> {
        if name == "bool" {
            return Some(Type::BOOL);
        }
        if name == "usize" {
            return Some(Type::Usize);
        }
        if name == "isize" {
            return Some(Type::Isize);
        }
        if name == "str" {
            return Some(Type::Str);
        }
        if name == "cstr" {
            return Some(Type::Cstr);
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

    /// Whether this type is `bool` (the reserved zero-payload enum).
    pub fn is_bool(&self) -> bool {
        *self == Type::BOOL
    }

    /// Whether this type is a reference (`&T` or `&!T`).
    pub fn is_ref(&self) -> bool {
        matches!(self, Type::Ref(..))
    }

    /// Whether a value of this type lives in memory rather than in an SSA
    /// temporary: the four shapes that have an address, and so the four that
    /// can be borrowed or denoted by a second name.
    pub fn is_aggregate(&self) -> bool {
        // `bool` is an enum at the surface but a register-resident scalar in
        // its representation (the general zero-payload-enum layout rule), so
        // it has no address and is not borrowable, exactly as the retired
        // primitive `Type::Bool` was not.
        if *self == Type::BOOL {
            return false;
        }
        matches!(
            self,
            Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..)
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
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
            Type::Ref(_, _, name) => name,
            Type::Usize => "usize",
            Type::Isize => "isize",
            Type::Str => "str",
            Type::Cstr => "cstr",
            Type::Quotation(eff) => eff.name_static,
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

#[derive(Debug, Clone)]
pub struct Term {
    pub kind: TermKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TermKind {
    IntLit(i64),
    FloatLit(f64),
    /// A `"..."` string literal (R6): type `str`, decoded content already
    /// escape-resolved by the lexer.
    StrLit(String),
    /// A word invocation, or a reference to a named local.
    Call(String),
    /// A `| names |` binding (R1): pops one value per name at the point it
    /// appears, leftmost name taking the deepest value. Its extent is the rest
    /// of the enclosing block (R2), so no closing term is needed.
    Bind(Vec<String>),
    If {
        then_branch: Vec<Term>,
        else_branch: Vec<Term>,
        /// The `else` token, when present: the `then` arm's terminator, and so
        /// where a name bound in that arm goes out of scope (R2, R6).
        else_span: Option<Span>,
        /// The `end` token: the `else` arm's terminator, and the `then` arm's
        /// too when there is no `else`.
        end_span: Span,
    },
    /// A `[ ... ]` quotation literal (R1): an ordered term list, nested by
    /// construction since the element list is parsed with `parse_terms`.
    /// Compile-time-only marker in this slice (D1): never a runtime value.
    Quotation(Vec<Term>),
}

/// R18/R21: clone a combinator body, appending a unique per-inline suffix to
/// every name a `| ... |` binds and to every later reference to such a name,
/// so the spliced body's locals are fresh in the caller's scope and a
/// passed-down quotation literal keeps capturing its *definition*-scope
/// binding under transitive inlining. A `Call` that is not a body-bound local
/// (a word, a builtin, another combinator, a cast) is left untouched. Scoping
/// follows the language's: a bind's extent is the rest of its block, and a
/// nested quotation or `if` arm inherits the outer binds by value. Both the
/// checker's and lowering's inliners call this with the same `uid` discipline,
/// so a body they both splice is renamed identically.
pub fn alpha_rename_locals(terms: &[Term], uid: u32) -> Vec<Term> {
    let mut bound: Vec<String> = Vec::new();
    rename_terms(terms, uid, &mut bound)
}

fn rename_local(name: &str, uid: u32) -> String {
    format!("{name}{INLINE_SUFFIX}{uid}")
}

/// The private separator `alpha_rename_locals` appends to an inlined local's
/// source name. A renamed local never reaches a user diagnostic: a combinator
/// body is checked standalone at its definition (R17), so any error about its
/// own locals surfaces there with the source spelling and aborts compilation
/// before any splice can rename them; the renamed spelling exists only for
/// collision-free lookup during the splice and its lowering.
const INLINE_SUFFIX: &str = "__inl";

/// Rename a `Call` naming a body-bound local. A borrow reads its local through
/// a `&`/`&!` sigil (`&arr`, `&!arr`), so the sigil is split off, the local
/// part renamed if bound, and the sigil re-attached; a `Call` that is not a
/// bound local (a word, `&>`, a cast) is returned unchanged.
fn rename_call(name: &str, uid: u32, bound: &[String]) -> String {
    let is_bound = |n: &str| bound.iter().any(|b| b == n);
    if let Some(inner) = name.strip_prefix("&!") {
        if is_bound(inner) {
            return format!("&!{}", rename_local(inner, uid));
        }
    } else if let Some(inner) = name.strip_prefix('&') {
        if is_bound(inner) {
            return format!("&{}", rename_local(inner, uid));
        }
    } else if is_bound(name) {
        return rename_local(name, uid);
    }
    name.to_string()
}

fn rename_terms(terms: &[Term], uid: u32, bound: &mut Vec<String>) -> Vec<Term> {
    let start = bound.len();
    let mut out = Vec::with_capacity(terms.len());
    for term in terms {
        let kind = match &term.kind {
            TermKind::Bind(names) => {
                let renamed = names
                    .iter()
                    .map(|n| {
                        bound.push(n.clone());
                        rename_local(n, uid)
                    })
                    .collect();
                TermKind::Bind(renamed)
            }
            TermKind::Call(name) => TermKind::Call(rename_call(name, uid, bound)),
            TermKind::Quotation(inner) => {
                let mut inner_bound = bound.clone();
                TermKind::Quotation(rename_terms(inner, uid, &mut inner_bound))
            }
            TermKind::If {
                then_branch,
                else_branch,
                else_span,
                end_span,
            } => {
                let mut tb = bound.clone();
                let mut eb = bound.clone();
                TermKind::If {
                    then_branch: rename_terms(then_branch, uid, &mut tb),
                    else_branch: rename_terms(else_branch, uid, &mut eb),
                    else_span: *else_span,
                    end_span: *end_span,
                }
            }
            other => other.clone(),
        };
        out.push(Term {
            kind,
            span: term.span,
        });
    }
    bound.truncate(start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// U8 (slice 5b, R8d): `find_type_in_module` matches on `name_static` with
    /// module gating. A single-module lookup is unaffected, and two decls that
    /// share a `name_static` but sit in different modules disambiguate by their
    /// module id (the REPL splice's epoch-tagged `.name` plays no part in
    /// type-position resolution).
    #[test]
    fn find_type_in_module_matches_name_static_module_gated() {
        let mk = |name: &'static str, name_static: &'static str, module: u32| StructDecl {
            name: name.to_string(),
            name_static,
            fields: Vec::new(),
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module,
        };
        // Two structs share `name_static` "Point" but sit in modules 0 and 1,
        // and their `.name` fields are tagged apart the way the REPL splice
        // tags an epoch onto an imported type.
        let structs = vec![mk("Point", "Point", 0), mk("q::Point__import3", "Point", 1)];
        let enums: Vec<EnumDecl> = Vec::new();

        // Module 0's lookup finds index 0 (a single-module lookup is exactly
        // this, and is unaffected: `name_static` equals `name` there).
        match find_type_in_module(&structs, &enums, "Point", 0) {
            Some(Type::Struct(id, _)) => assert_eq!(id.index(), 0),
            other => panic!("expected module 0's Point at index 0, got {other:?}"),
        }
        // Module 1's lookup finds index 1, disambiguated purely by module id
        // even though the tagged `.name` ("q::Point__import3") never matches
        // the queried "Point".
        match find_type_in_module(&structs, &enums, "Point", 1) {
            Some(Type::Struct(id, _)) => assert_eq!(id.index(), 1),
            other => panic!("expected module 1's Point at index 1, got {other:?}"),
        }
        // A name absent from the queried module is `None`, not a stray hit on
        // the other module's same-named decl.
        assert!(find_type_in_module(&structs, &enums, "Point", 2).is_none());
    }

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
        assert_eq!(Type::from_name("bool"), Some(Type::BOOL));
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
        assert!(!Type::BOOL.is_numeric());
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
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            }],
            enums: Vec::new(),
            arrays: Vec::new(),
            owned_cells: Vec::new(),
            refs: Vec::new(),
            externs: Vec::new(),
            instantiations: std::collections::HashMap::new(),
            builtin_overloads: std::collections::HashMap::new(),
            modules: Vec::new(),
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

    /// U4 (R11): module-aware resolution prefers the current module for a bare
    /// name, and maps a `q::Base` through the import map to the qualified
    /// module. Two modules each declare `Foo`; from module 0 a bare `Foo`
    /// finds module 0's, `lib::Foo` finds module 1's.
    #[test]
    fn type_resolution_prefers_own_module_then_qualifier() {
        let mk = |name: &'static str, module: u32| StructDecl {
            name: name.to_string(),
            name_static: name,
            fields: Vec::new(),
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module,
        };
        // Module 0's Foo is index 0, module 1's Foo is index 1.
        let structs = vec![mk("Foo", 0), mk("Foo", 1)];
        let mut imports = std::collections::HashMap::new();
        imports.insert("lib".to_string(), 1u32);
        let no_selective = std::collections::HashMap::new();

        let bare =
            resolve_type_name_in_module(&structs, &[], "Foo", 0, &imports, &no_selective).unwrap();
        assert_eq!(bare, Type::Struct(StructId(0), "Foo"), "own module first");
        let qualified =
            resolve_type_name_in_module(&structs, &[], "lib::Foo", 0, &imports, &no_selective)
                .unwrap();
        assert_eq!(
            qualified,
            Type::Struct(StructId(1), "Foo"),
            "qualifier maps to the imported module"
        );
        // An unmapped qualifier resolves to nothing.
        assert!(resolve_type_name_in_module(
            &structs,
            &[],
            "nope::Foo",
            0,
            &imports,
            &no_selective
        )
        .is_none());
        // R15c: a bare name absent from the own module resolves against a
        // module it is selectively imported from.
        let mut selective = std::collections::HashMap::new();
        selective.insert("Foo".to_string(), 1u32);
        let via_selective =
            resolve_type_name_in_module(&[mk("Foo", 1)], &[], "Foo", 0, &imports, &selective)
                .unwrap();
        assert_eq!(
            via_selective,
            Type::Struct(StructId(0), "Foo"),
            "a selectively imported type resolves bare against its source module"
        );
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
                module: 0,
            }],
            arrays: Vec::new(),
            owned_cells: Vec::new(),
            refs: Vec::new(),
            externs: Vec::new(),
            instantiations: std::collections::HashMap::new(),
            builtin_overloads: std::collections::HashMap::new(),
            modules: Vec::new(),
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
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            }],
            enums: vec![EnumDecl {
                name: "Dup".to_string(),
                name_static,
                variants: vec![variant("V", vec![])],
                span: Span::default(),
                module: 0,
            }],
            arrays: Vec::new(),
            owned_cells: Vec::new(),
            refs: Vec::new(),
            externs: Vec::new(),
            instantiations: std::collections::HashMap::new(),
            builtin_overloads: std::collections::HashMap::new(),
            modules: Vec::new(),
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
    fn intern_bundle_struct_same_tuple_dedups_expected() {
        let mut structs = Vec::new();
        let a = intern_bundle_struct(&mut structs, &[Type::I64, Type::BOOL]);
        let b = intern_bundle_struct(&mut structs, &[Type::I64, Type::BOOL]);
        assert_eq!(a, b);
        assert_eq!(structs.len(), 1);
        assert!(structs[0].is_bundle);
        assert_eq!(
            structs[0].fields,
            vec![
                ("f0".to_string(), Type::I64),
                ("f1".to_string(), Type::BOOL)
            ]
        );
    }

    #[test]
    fn intern_bundle_struct_distinct_tuples_and_orders_are_distinct_expected() {
        // Two outputs of the same types in the other order are a different
        // bundle: the tuple is ordered, deepest output first.
        let mut structs = Vec::new();
        let a = intern_bundle_struct(&mut structs, &[Type::I64, Type::BOOL]);
        let b = intern_bundle_struct(&mut structs, &[Type::BOOL, Type::I64]);
        let c = intern_bundle_struct(&mut structs, &[Type::I64, Type::BOOL, Type::I64]);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(structs.len(), 3);
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

    #[test]
    fn instantiation_symbol_none_reproduces_native_spelling_expected() {
        let mut subst = Subst::default();
        subst.ty.push((0, Type::I64));
        assert_eq!(
            instantiation_symbol("id", &subst, None),
            "sooth_mono_id__t0_i64"
        );
    }

    #[test]
    fn instantiation_symbol_some_appends_gen_component_expected() {
        let mut subst = Subst::default();
        subst.ty.push((0, Type::I64));
        assert_eq!(
            instantiation_symbol("id", &subst, Some(0)),
            "sooth_mono_id__t0_i64__gen0"
        );
    }

    #[test]
    fn instantiation_symbol_distinct_generations_are_distinct_symbols_expected() {
        let mut subst = Subst::default();
        subst.ty.push((0, Type::I64));
        assert_ne!(
            instantiation_symbol("id", &subst, Some(0)),
            instantiation_symbol("id", &subst, Some(1))
        );
    }

    fn bare_word(name: &str) -> WordDef {
        WordDef {
            name: name.to_string(),
            effect: StackEffect {
                inputs: Vec::new(),
                outputs: Vec::new(),
            },
            body: WordBody::Terms { terms: Vec::new() },
            poly: None,
            module: 0,
            span: Span::default(),
        }
    }

    #[test]
    fn overload_symbols_non_overloaded_names_keep_their_bare_name() {
        let words = vec![bare_word("foo"), bare_word("bar")];
        assert_eq!(overload_symbols(&words), vec!["foo", "bar"]);
    }

    #[test]
    fn overload_symbols_an_overload_set_gets_distinct_suffixed_symbols() {
        let words = vec![bare_word("show"), bare_word("show"), bare_word("other")];
        let syms = overload_symbols(&words);
        assert_eq!(syms.len(), 3);
        assert_ne!(
            syms[0], syms[1],
            "two `show`s get distinct symbols: {syms:?}"
        );
        assert_eq!(syms[2], "other", "an unrelated name is untouched: {syms:?}");
        assert!(syms[0].starts_with("show"));
        assert!(syms[1].starts_with("show"));
    }

    #[test]
    fn overload_symbols_drop_is_exempt_regardless_of_count() {
        let words = vec![bare_word("drop"), bare_word("drop"), bare_word("drop")];
        assert_eq!(overload_symbols(&words), vec!["drop", "drop", "drop"]);
    }
}
