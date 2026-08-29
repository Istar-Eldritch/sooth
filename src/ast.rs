//! Sooth AST. Skeleton for Phase 0; grows as the language does.

/// A source location, 1-based (line, col), plus the id of the file it came
/// from within its assembled `Module` (0 for a single-file program, where the
/// field is never read). Load-bearing beyond diagnostics:
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
    /// structurally, so two spellings of the same shape (e.g. two `array[i64 4]`
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
    /// P7 slice 3c (R1.2): the per-program interned slice registry, one entry
    /// per distinct `(element, mutable)` shape, indexed by `SliceId`. Keyed on
    /// mutability like `refs` rather than on a count like `arrays`: a slice's
    /// length is a runtime component of the value, never part of its type.
    pub slices: Vec<SliceDecl>,
    /// Phase 5 slice 1 (R1): every `type:` declaration whose header bound one
    /// or more type variables, parsed but not yet monomorphized. Empty for a
    /// program with no generic `type:` declaration.
    pub generic_structs: Vec<GenericStructDecl>,
    /// The enum twin of `generic_structs`.
    pub generic_enums: Vec<GenericEnumDecl>,
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
    /// P7.S3k (R2): the generic-to-generic calls each polymorphic body makes,
    /// keyed by the *caller's* name -- the word whose body contains them, not
    /// any instantiation of it, since a polymorphic body is walked once with
    /// its variables still rigid. Grounded per caller instantiation by
    /// composing each record's mapping with that instantiation's θ. Empty for
    /// a program whose generic words call no generic word.
    pub poly_cross_calls: std::collections::HashMap<String, Vec<PolyCrossCall>>,
    /// P7.S3k (R4): the monomorphs reached only *through* a generic body's
    /// call to another generic word, composed by the checker's transitive
    /// fixpoint. Flat rather than `Span`-keyed because `instantiations`
    /// structurally cannot hold them: one cross-call site in a generic body
    /// serves every instantiation of that body, so N callee monomorphs share
    /// one span. Symbol-deduped, so lowering emits one `IrFunc` per entry.
    /// Empty for a program whose generic words call no generic word.
    pub transitive_instantiations: Vec<CallInst>,
    /// P7.S3o (R1/R2): per-splice instantiation records for poly calls
    /// encountered inside a spliced combinator body, keyed by
    /// `(inline_uid, body_span)`. Each splice mints a unique `inline_uid`,
    /// so two splices of the same combinator at different types produce
    /// distinct keys even though the body terms share spans. The checker
    /// redirects `check_poly_call`'s `CallInst` here (instead of the
    /// span-keyed `instantiations`) when inside a splice, and lowering reads
    /// the per-splice record back through its stack of active `inline_uid`s.
    /// Empty for a program whose combinators call no polymorphic word.
    pub splice_records: std::collections::HashMap<(u32, Span), CallInst>,
    /// P7.S3o Phase 3: per-splice trait-member-call resolutions, keyed by
    /// `(inline_uid, body_span)` — the resolved `impl:` word's lowering symbol
    /// for each bare trait member call dispatched inside a spliced combinator
    /// body. Mirrors `splice_records` (same key shape, same per-splice
    /// scoping) but holds a bare member name → resolved symbol mapping rather
    /// a full `CallInst`: a bare member call is a concrete call to an `impl:`
    /// word, not a poly-word instantiation, so it lowers through
    /// `lower_resolved_word_call` rather than `lower_poly_call`. Empty for a
    /// program whose combinators call no bare trait member.
    pub splice_trait_calls: std::collections::HashMap<(u32, Span), String>,
    /// Phase 4 slice 8a phase 2 (R7): the call sites that resolved to a user
    /// overload of a builtin-named word (e.g. `add` on two `Vec2`), keyed by the
    /// call site's `Span`, valued by the resolved callee's Sooth name. A
    /// sparse map mirroring `instantiations`: lowering consults it before its
    /// name-directed builtin dispatch, so a recorded site emits an
    /// `Instr::Call` to the user word instead of the builtin instruction. The
    /// corpus produces no records, so its lowering is untouched byte-for-byte.
    pub builtin_overloads: std::collections::HashMap<Span, String>,
    /// Phase 7 slice 1 (R2): one entry per receiver-directed field projection
    /// (`&hp`/`&!hp`), keyed by the call site's `Span`, valued by the struct
    /// and field index the checker resolved it against. Lowering has no
    /// checker stack to re-derive the receiver type from, and a projection's
    /// name is not globally unique the way the fused `Sprite>hp` spelling was,
    /// so the checker's resolution is recorded here and read back per site.
    ///
    /// `Span` alone suffices as a key only because a generic body rejects a
    /// field projection outright (`poly_reference_word`) and is checked once,
    /// never re-walked per instantiation: every projection is therefore
    /// resolved in the monomorphic walk, where the receiver already carries
    /// the concrete `StructId`. Admitting a projection inside a generic body
    /// breaks that, and two instantiations of one call site would then share
    /// (and silently misdispatch through) a single entry.
    pub resolved_fields: std::collections::HashMap<Span, (StructId, usize)>,
    /// Phase 6 slice 3 (R6): the receiver-directed variant-field projections
    /// (`&r`/`&!r` against a `Type::Variant` receiver), keyed by the call
    /// site's `Span`, valued by the enum id, variant index and field index the
    /// checker resolved it against. Mirrors `resolved_fields` structurally
    /// (P7 slice 1's own instruction: "its own `EnumId`-keyed lowering-side
    /// table rather than a widened `resolved_fields`") rather than reusing it,
    /// since a variant field has no `StructId` to key under.
    pub resolved_variant_fields: std::collections::HashMap<Span, (EnumId, usize, usize)>,
    /// Phase 4 slice 5a (R10): one entry per file in the import closure, in
    /// topological order, module 0 being the entry file. A single-file program
    /// has exactly one entry. Every `StructDecl`/`EnumDecl`/`WordDef`/`ExternDecl`
    /// carries an owning module id indexing this vector; the entry carries that
    /// module's qualifier->module import map and its parsed `export:` list.
    pub modules: Vec<ModuleInfo>,
    /// P7.S3e (R3): the whole-program trait registry, mirroring
    /// `structs`/`enums`'s flat-`Vec` shape. Pre-seeded with `Copy`/`Ord`
    /// (R2) at `RESERVED_TRAIT_MODULE`, followed by every user `trait:`
    /// declaration across the whole import closure, indexed by `TraitId`.
    pub traits: Vec<TraitDecl>,
    /// P7.S3e (R4/R11): the whole-program `impl:` registry, one entry per
    /// `impl: Trait for Type ... ;` block, in source order.
    pub impls: Vec<ImplDecl>,
    /// Phase 7 slice 2 (D4): one entry per `static:` declaration, in source
    /// order. A static is never exported or imported (R2): its data symbol is
    /// module-scoped mangled exactly like a word's, and only the per-word
    /// `global:` clause on an exported word crosses a module boundary, never
    /// the static itself.
    pub statics: Vec<StaticDecl>,
    /// P7 slice 3a phase 2 (R2): the live generic instantiator, kept alive
    /// through check and lowering (rather than consumed at parse time, R4/D5's
    /// old behaviour) so a poly word's own construction can mint a monomorph
    /// on demand -- see `GenericTypes`'s own doc for the dedup-key invariant
    /// this depends on. `generic_structs`/`generic_enums` above stay as a
    /// read-only header snapshot for existing readers; this is the mutable
    /// instantiator those headers were drawn from.
    pub generics: GenericTypes,
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
    /// P8 S2 (R2): which of the compiler-provided intrinsics this module's
    /// `import: intrinsics ...` lines make visible to it.
    pub intrinsics: IntrinsicVisibility,
}

/// P8 S2 (R2): a module's view of the `intrinsics` module. The `BUILTIN_WORDS`
/// table itself does not move and is not per module; only visibility is gated,
/// so this says which of those names a body in this module may call bare.
///
/// Both `parser::parse` (every in-process test) and `driver::assemble_module`
/// set this field explicitly on every `ModuleInfo` they build, so nothing reads
/// the derived `Default`. It fails closed to
/// `None` anyway: a build path that forgot to set it should reject every
/// bare intrinsic rather than silently admit all of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum IntrinsicVisibility {
    All,
    /// `import: intrinsics | dup add | ;` -- only the listed names.
    Only(std::collections::HashSet<String>),
    /// No `import: intrinsics` line at all.
    #[default]
    None,
}

impl IntrinsicVisibility {
    /// Whether a bare call to the intrinsic `name` is visible here. The caller
    /// decides what counts as an intrinsic name (the gate set); this only
    /// answers the import question.
    pub fn admits(&self, name: &str) -> bool {
        match self {
            IntrinsicVisibility::All => true,
            IntrinsicVisibility::Only(names) => names.contains(name),
            IntrinsicVisibility::None => false,
        }
    }

    /// P7.S3q (R1): widen by one name. A hub contributes *names*, never its
    /// own bit, so this is the only way an import ever adds to a module's
    /// visibility: `All` can be reached by writing `import: intrinsics * ;`
    /// and no other way. Idempotent, since `Only` holds a set.
    pub fn admitting(self, name: &str) -> IntrinsicVisibility {
        match self {
            IntrinsicVisibility::All => IntrinsicVisibility::All,
            IntrinsicVisibility::Only(mut names) => {
                names.insert(name.to_string());
                IntrinsicVisibility::Only(names)
            }
            IntrinsicVisibility::None => {
                IntrinsicVisibility::Only(std::iter::once(name.to_string()).collect())
            }
        }
    }
}

/// P8 slice 1a (F2): where a module-name import's first segment is rooted.
/// Syntactic, never inferred: a `self::` prefix names the importing file's
/// own package, its absence names a `depends:` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportAnchor {
    Dependency,
    SelfPackage,
}

/// A path-derived module name as written in an import target, split on `::`.
/// For a `Dependency` anchor the first segment is the package name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleName {
    pub anchor: ImportAnchor,
    pub segments: Vec<String>,
}

impl ModuleName {
    /// The name as written, `self::` prefix included, for diagnostics.
    pub fn render(&self) -> String {
        let joined = self.segments.join("::");
        match self.anchor {
            ImportAnchor::SelfPackage => format!("self::{joined}"),
            ImportAnchor::Dependency => joined,
        }
    }
}

/// What an `import:` names: today's quoted path (manifest-less files only) or
/// a module name resolved against the package graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportTarget {
    Path(String),
    Module(ModuleName),
}

impl ImportTarget {
    pub fn render(&self) -> String {
        match self {
            ImportTarget::Path(p) => p.clone(),
            ImportTarget::Module(m) => m.render(),
        }
    }
}

/// How an import binds the names it brings in. A `Qualified` import always
/// carries a concrete qualifier (defaulted to the target's last segment when
/// the source elides it); `Wildcard` (S2's `import: intrinsics * ;`) carries
/// none at all.
///
/// Callers read this through `Import::qualifier`/`Import::selective` rather
/// than matching it directly, so the exhaustive match guarding a new variant
/// lives only in those two functions -- a deliberate trade against the
/// compile-error tripwire an exhaustive caller-side match would give. When S2
/// gives `Wildcard` its own visibility effect, audit every caller of
/// `qualifier`/`selective` by hand rather than relying on the compiler to
/// flag them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportBinding {
    Qualified {
        qualifier: String,
        /// The `| name... |` clause, empty when absent. Each name keeps its
        /// span: every R20/R21 selective-import diagnostic is located from it.
        selective: Vec<(String, Span)>,
    },
    Wildcard,
}

/// Phase 4 slice 5a (R6), regrammared by P8 slice 1a (OQ3): a parsed
/// `import:` form, `import: <target> [<qualifier>] [ | <name>... | ] ;`.
/// `span` locates the `import:` keyword for later diagnostics.
#[derive(Debug, Clone)]
pub struct Import {
    pub target: ImportTarget,
    pub binding: ImportBinding,
    pub span: Span,
}

impl Import {
    /// The bound qualifier, or `None` for a wildcard import (which binds no
    /// qualifier at all, rather than eliding one).
    pub fn qualifier(&self) -> Option<&str> {
        match &self.binding {
            ImportBinding::Qualified { qualifier, .. } => Some(qualifier),
            ImportBinding::Wildcard => None,
        }
    }

    /// The `| name... |` clause, empty for a wildcard import.
    pub fn selective(&self) -> &[(String, Span)] {
        match &self.binding {
            ImportBinding::Qualified { selective, .. } => selective,
            ImportBinding::Wildcard => &[],
        }
    }
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
    type_origin: &[std::collections::HashMap<String, u32>],
) -> Option<Type> {
    if let Some(t) = Type::from_name(name) {
        return Some(t);
    }
    if let Some((qualifier, base)) = name.split_once("::") {
        let target = *imports.get(qualifier)?;
        return find_type_in_module(structs, enums, base, target).or_else(|| {
            // P7.S3q-follow: `target` names the name on its own `export:`
            // list without declaring it itself -- a re-export, resolved to
            // its true declaring module ahead of time in `type_origin`.
            let origin = *type_origin.get(target as usize)?.get(base)?;
            find_type_in_module(structs, enums, base, origin)
        });
    }
    find_type_in_module(structs, enums, name, module).or_else(|| {
        // R15c (phase 4): a selectively imported type's bare name resolves
        // unqualified against its target module, the same one unit its
        // generated words resolve through in `resolve.rs`'s `NameTables`.
        let target = *selective.get(name)?;
        find_type_in_module(structs, enums, name, target).or_else(|| {
            let origin = *type_origin.get(target as usize)?.get(name)?;
            find_type_in_module(structs, enums, name, origin)
        })
    })
}

fn find_type_in_module(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    name: &str,
    module: u32,
) -> Option<Type> {
    // R8d (slice 5b): match `name_static`, not `name`. `resolve::mangle`
    // rewrites a decl's `.name` with a module suffix so the accessor/
    // constructor recognizers agree on one row per internal spelling, but its
    // `.name_static` stays the pretty user-typed base spelling; a type-position
    // reference must resolve against that. Behavior-preserving for every native call: this
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
    /// fact reach every `is_copy` call site, the layout fold, and any
    /// standalone registry without threading a table through any of them.
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
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct VariantDecl {
    pub name: String,
    pub name_static: &'static str,
    /// Phase 6 slice 2 (R1): the leaked `Enum.Variant` display name (e.g.
    /// `Shape.Circle`), built once at declaration time where the owning
    /// enum's name is in hand. The **sole** source `variant_type` reads to
    /// build a `Type::Variant` -- never re-derived per site -- so every
    /// `Type::Variant` for the same `(EnumId, vi)` compares equal.
    pub display_static: &'static str,
    pub fields: Vec<(String, Type)>,
    pub span: Span,
}

/// Phase 6 slice 2 (R1): the **sole** constructor of a `Type::Variant`. Reads
/// `enums[id].variants[vi].display_static`, never formats a fresh string, so
/// every construction of the same `(EnumId, vi)` is byte-identical and thus
/// compares equal under `Type`'s derived `PartialEq`.
pub fn variant_type(enums: &[EnumDecl], id: EnumId, vi: usize) -> Type {
    Type::Variant(id, vi, enums[id.index()].variants[vi].display_static)
}

/// Phase 5 slice 1 (R1, D5): a `type:` header that bound one or more type
/// variables (`type: Box['T] ...`), parsed into its variable-scoped field
/// list but not yet monomorphized -- minting a concrete `StructDecl` per
/// distinct application is Phase 2/3 of this slice. Deliberately a *separate*
/// type from `StructDecl` (not a `ty_var_names` field bolted onto it): the
/// concrete registry every existing layout/check/lowering pass walks stays
/// exactly the shape it is today. Nothing walks this registry until an
/// explicit application mints a concrete entry from it, so a generic type
/// declared but never applied compiles clean.
#[derive(Debug, Clone)]
pub struct GenericStructDecl {
    pub name: String,
    /// The header's bound variable names in binding (first-mention) order,
    /// each keeping its leading `'` (e.g. `"'T"`) -- the id space a field's
    /// `PolyType::Var` indexes into.
    pub ty_var_names: Vec<String>,
    /// P7.S6a (R3): the header's bound length-variable names, parallel to
    /// `ty_var_names` but in the separate length id space `Len::Var`
    /// indexes into.
    pub len_var_names: Vec<String>,
    pub fields: Vec<(String, PolyType)>,
    pub span: Span,
    pub module: u32,
}

/// The enum twin of `GenericStructDecl`, mirroring how `EnumDecl` sits
/// alongside `StructDecl`.
#[derive(Debug, Clone)]
pub struct GenericEnumDecl {
    pub name: String,
    pub ty_var_names: Vec<String>,
    pub len_var_names: Vec<String>,
    pub variants: Vec<GenericVariantDecl>,
    pub span: Span,
    pub module: u32,
}

/// One variant of a `GenericEnumDecl`, mirroring `VariantDecl`.
#[derive(Debug, Clone)]
pub struct GenericVariantDecl {
    pub name: String,
    pub fields: Vec<(String, PolyType)>,
    pub span: Span,
}

/// Phase 5 slice 1 (R2/R4/D5): the parse-time home of every generic `type:`
/// declaration and of the concrete struct/enum each distinct application of
/// one mints. Threaded `&mut` through `parse_bodies` beside the
/// `arrays`/`owned_cells`/`refs` registries and for the same reason those
/// are: an instantiation has no declared name a pre-pass could register
/// ahead of parsing, so the registry grows as field and slot type
/// expressions resolve.
///
/// `structs`/`enums` hold the variable-bearing declarations, registered for
/// every file in the closure by `parser::prepass_generic_typedefs` before any
/// body parses, so an application reaches a header in another module (slice 2,
/// OQ1). `inst_structs`/`inst_enums` hold ordinary concrete decls, appended
/// onto `Module::structs`/`Module::enums` once the whole closure has parsed.
/// `struct_base`/`enum_base` are those registries' post-pre-pass lengths, so
/// an instantiation's `StructId`/`EnumId` is final the moment it is minted:
/// the pre-pass has already registered every named `type:` in every file
/// before any body parses, so nothing can land between them afterwards.
#[derive(Debug, Clone)]
pub struct GenericTypes {
    pub structs: Vec<GenericStructDecl>,
    pub enums: Vec<GenericEnumDecl>,
    pub inst_structs: Vec<StructDecl>,
    pub inst_enums: Vec<EnumDecl>,
    /// Dedup identity for `inst_structs`/`inst_enums`, parallel by index:
    /// `(generic decl index, instantiating module, concrete arguments)`.
    /// `Type` is `Eq` over the real `StructId`/`EnumId`/etc. an argument
    /// carries, so this is injective where the rendered *name* the fix
    /// below still is not -- see `type_instantiation_name`. Kept off
    /// `StructDecl`/`EnumDecl` themselves so those stay shaped exactly like
    /// a hand-written concrete `type:` (R5). P7.S6a (R5): widened with a
    /// fourth, parallel length-argument list, so `Buffer[u8 256]` and
    /// `Buffer[u8 512]` mint distinct monomorphs instead of colliding.
    struct_keys: Vec<(usize, u32, Vec<Type>, Vec<Len>)>,
    enum_keys: Vec<(usize, u32, Vec<Type>, Vec<Len>)>,
    /// P7 slice 3a phase 2 (R2): the resolved `Type` each `struct_keys` entry
    /// minted, parallel by index. Reading `id`/`name` back from here (rather
    /// than recomputing `struct_base + i`) is what makes a downstream mint
    /// (after `struct_base` has been rebased past a parse-time batch) safe:
    /// an entry's real id is whatever was true the moment it was minted, not
    /// a function of the *current* base.
    struct_resolved: Vec<Type>,
    /// The enum twin of `struct_resolved`.
    enum_resolved: Vec<Type>,
    /// P7.S3n (R2): the `structs` indices that are still placeholders --
    /// registered by `parse_generic_typedefs`' stage (a) with an empty field
    /// list, so a self-reference inside a header's own field list has a
    /// header to find, but not yet filled by stage (b). A set of indices
    /// rather than a `bool` parallel to `structs`: a decl pushed with its
    /// fields already in place is simply absent, so nothing has to be kept
    /// in lockstep and a direct `structs.push` cannot desynchronise it.
    /// An instantiation minted against a pending header cannot compute its
    /// fields yet, so it lands on `deferred_structs` instead.
    struct_pending: Vec<usize>,
    /// The enum twin of `struct_pending`.
    enum_pending: Vec<usize>,
    /// P7.S3n (R2): `(inst_structs index, structs index, concrete
    /// arguments)` for every instantiation minted against a still-pending
    /// header. The `StructId` is already handed out and never changes; only
    /// the field list is owed, and `fill_struct_fields` pays it once that
    /// header's real fields are known.
    deferred_structs: Vec<(usize, usize, Vec<Type>, Vec<Len>)>,
    /// The enum twin of `deferred_structs`.
    deferred_enums: Vec<(usize, usize, Vec<Type>, Vec<Len>)>,
    struct_base: usize,
    enum_base: usize,
}

impl Default for GenericTypes {
    /// R2: an empty instantiator based at `(0, 0)`. Only correct for a
    /// `Module` that itself has no pre-existing structs/enums (a fresh test
    /// fixture); the real driver/parser paths always call `with_bases`
    /// explicitly instead, matching `with_bases`'s own doc.
    fn default() -> GenericTypes {
        GenericTypes::with_bases(0, 0)
    }
}

/// The concrete type registries an instantiation name is rendered against.
/// Grouped rather than passed one by one because `type_arg_key` needs all
/// five to walk a nested argument down to its leaf.
#[derive(Clone, Copy)]
pub struct NameRegistries<'a> {
    pub structs: &'a [StructDecl],
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ArrayDecl],
    pub cells: &'a [OwnedCellDecl],
    pub refs: &'a [RefDecl],
}

/// P7.S3n (R5): `NameRegistries` plus the write access substitution needs.
/// `substitute_generic_field` grounds a field that wraps a type variable
/// (`array['T 2]`, `^'T`, `Ent['K 'V]`), and grounding one *interns*: the array /
/// cell / ref shape it produces may be one nothing has registered yet, and
/// its `Generic` arm re-enters `instantiate_struct`/`instantiate_enum`, which
/// interns in turn. `NameRegistries` is `Copy` over immutable slices and can
/// intern nothing, so it cannot carry that pair.
///
/// A struct rather than five threaded parameters because the pair is mutually
/// recursive: `substitute_generic_field` calls the instantiator, which calls
/// back. Not `Copy` (it holds `&mut`), hence `reborrow` at every hop.
pub struct MutRegistries<'a> {
    pub structs: &'a [StructDecl],
    pub enums: &'a [EnumDecl],
    pub arrays: &'a mut Vec<ArrayDecl>,
    pub cells: &'a mut Vec<OwnedCellDecl>,
    pub refs: &'a mut Vec<RefDecl>,
}

impl MutRegistries<'_> {
    /// The read-only view `type_instantiation_name` needs, over the *live*
    /// registries -- not the `cells: &[]`/`refs: &[]` throwaway an earlier
    /// caller built, which renders a cell- or ref-payload argument wrong (or
    /// panics on its index) as soon as one exists to look up.
    pub fn names(&self) -> NameRegistries<'_> {
        NameRegistries {
            structs: self.structs,
            enums: self.enums,
            arrays: self.arrays,
            cells: self.cells,
            refs: self.refs,
        }
    }

    /// A shorter-lived copy for one recursive hop.
    pub fn reborrow(&mut self) -> MutRegistries<'_> {
        MutRegistries {
            structs: self.structs,
            enums: self.enums,
            arrays: self.arrays,
            cells: self.cells,
            refs: self.refs,
        }
    }
}

/// The instantiation-name spelling of one type argument. A primitive
/// (`i64`, `bool`, ...) has no id and its bare `Type::name()` is already
/// injective across the whole program, so it renders unchanged. A struct or
/// enum argument's bare name is *not* always injective: `Type::name()`
/// renders only the declared spelling, never the module, so two distinct
/// structs (or enums) sharing a bare name across modules -- two files each
/// declaring `type: P ...`, or one importing the other's `P` as a field
/// beside its own same-named `P` -- render identically. `structs`/`enums`
/// are the full merged pre-pass registries (every file's, fixed before any
/// body parses), so whether a given bare name is actually shared by more
/// than one declaration is itself a pure function of the argument and the
/// program's source files, independent of which instantiation runs first --
/// the determinism `type_instantiation_name`'s NFR requires. Only that
/// ambiguous case gets the registry-id suffix, so the ordinary (non-
/// colliding) case keeps exactly the plain spelling (`Box[bool]`,
/// `Box[Box[i64]]`) the NFR asks for.
///
/// An array/owned-cell/ref argument is *rebuilt* from its registry entry
/// rather than taking its own `name_static`: those spellings are built from
/// the same module-blind `Type::name()` at their interning sites
/// (`intern_ref_type` renders `&{referent.name()}`), so `&P` inherits the
/// bare-name ambiguity its referent has. Recursing puts the tie-break at
/// the leaf where the ids live. Order-independent for the same reason the
/// leaf case is: a registry entry's referent/payload/element never changes
/// once minted. A quotation argument keeps its own spelling: R1's
/// phantom-variable rejection puts every argument in a field, and a quotation
/// can never be a field, so its rendering is only ever read back inside the
/// diagnostic rejecting it.
fn type_arg_key(t: &Type, regs: NameRegistries) -> String {
    match t {
        Type::Struct(id, name) => {
            if regs.structs.iter().filter(|d| d.name == *name).count() > 1 {
                format!("{name}.{}", id.index())
            } else {
                name.to_string()
            }
        }
        Type::Enum(id, name) => {
            if regs.enums.iter().filter(|d| d.name == *name).count() > 1 {
                format!("{name}.{}", id.index())
            } else {
                name.to_string()
            }
        }
        Type::Ref(id, mutable, _) => format!(
            "&{}{}",
            if *mutable { "!" } else { "" },
            type_arg_key(&regs.refs[id.index()].referent, regs)
        ),
        Type::OwnedCell(id, _) => {
            format!("^{}", type_arg_key(&regs.cells[id.index()].payload, regs))
        }
        Type::Array(id, _) => {
            let decl = &regs.arrays[id.index()];
            format!("[{} {}]", type_arg_key(&decl.element, regs), decl.count)
        }
        _ => t.name().to_string(),
    }
}

/// R4: the registry name of one monomorphized instantiation, a pure function
/// of `(generic name, concrete type arguments)` with no dependence on
/// processing order. Spelled the way `ArrayDecl`'s `array[i64 4]` name is -- the
/// structural shape itself -- rather than through `instantiation_symbol`'s
/// sanitizing scheme: this name is registry identity and diagnostic
/// rendering (`sooth_mono_Box__t0_i64` would be a regression in every type
/// mismatch naming one), a sanitized join is lossy enough for two distinct
/// argument lists to collide, and the one QBE-facing use of a type name is
/// sanitized injectively at the emission site anyway. `[` is a lexer
/// delimiter, so no source type-name token can ever equal one of these.
/// `regs` is threaded through only to break a struct/enum argument's
/// bare-name tie, at whatever depth it sits (`type_arg_key`).
///
/// P7.S6a (R5): `lens` renders after every type argument, in the same
/// bracket, so `Buffer[u8 256]` and `Buffer[u8 512]` -- two distinct
/// monomorphs once `struct_keys`/`enum_keys` carry a length component --
/// also render distinct names, rather than colliding on `Buffer[u8]` for
/// both. Empty on a zero-length-arg call, so every existing generic type's
/// symbol renders byte-identical to before this ruling.
pub fn type_instantiation_name(
    base: &str,
    args: &[Type],
    lens: &[Len],
    regs: NameRegistries,
) -> String {
    let mut parts: Vec<String> = args.iter().map(|t| type_arg_key(t, regs)).collect();
    parts.extend(lens.iter().map(|l| match l {
        Len::Concrete(n) => n.to_string(),
        Len::Var(v) => format!("'N{v}"),
    }));
    format!("{base}[{}]", parts.join(" "))
}

/// D7: the bare surface spelling a monomorphized `StructDecl`/`EnumDecl`/
/// `VariantDecl`'s mangled `name` carries -- the generic header's (or
/// variant's) own declared name, with every `type_instantiation_name`
/// `[...]` suffix stripped. `[` is a lexer delimiter no hand-written `type:`
/// name can contain, so splitting on the first one recovers exactly the
/// declared name for both a non-generic decl (no `[` at all, returned
/// unchanged) and an instantiation, without a separate field threaded
/// through every `StructDecl`/`EnumDecl` construction site in the crate.
pub fn generic_surface_name(name: &str) -> &str {
    name.split('[')
        .next()
        .expect("split always yields at least one piece")
}

impl GenericTypes {
    /// Substitute a generic declaration's field type against a use site's
    /// concrete type arguments.
    ///
    /// P7.S3n (R4): a field may wrap the header's own variables to any depth
    /// (`array[array['T 2] 2]`, `^'T`, `&'T`, `Ent['K 'V]`), so this recurses and
    /// interns the ground shape at each level -- the same bottom-up grounding
    /// `apply_subst` performs for a word signature, and the reason it needs
    /// `MutRegistries` rather than `NameRegistries`. Its `Generic` arm
    /// re-enters the instantiator, which is what makes this pair mutually
    /// recursive; R6's mint-and-memo-before-substitute ordering is what makes
    /// a self-referential header terminate there.
    ///
    /// The remaining panic is truthful, not a deferral. Two shapes reach it
    /// and neither is constructible: a `PolyType::Quotation` (R7 rejects a
    /// quotation field naming a type variable at the parser, and a *concrete*
    /// quotation field folds to `Concrete` instead) and a `QuotLit` (a
    /// poly-body marker that never reaches a declaration).
    fn substitute_generic_field(
        &mut self,
        pty: &PolyType,
        args: &[Type],
        lens: &[Len],
        mut regs: MutRegistries,
    ) -> Type {
        match pty {
            PolyType::Concrete(t) => *t,
            PolyType::Var(v) => args[*v as usize],
            PolyType::Array(elem, Len::Concrete(count)) => {
                let elem = self.substitute_generic_field(elem, args, lens, regs.reborrow());
                intern_array_type(regs.arrays, elem, *count)
            }
            // P7.S6a (R4): a field's array count naming a header-bound
            // length variable (`type: Buffer['T 'N: Len] data array['T 'N] ;`)
            // resolves against the instantiation's own length-argument list,
            // exactly as `PolyType::Var` resolves against `args` above.
            //
            // `v` is in bounds for every parser-driven instantiation --
            // `parse_generic_field_application` (R2a), `resolve_type_or_
            // apply`/`parse_type_arguments` (R6) and `parse_poly_generic_
            // application` (R7), all in `src/parser.rs`, only collapse to a
            // concrete instantiation once every length is concrete. `src/
            // check/poly.rs`'s phase-3 `&[]` placeholders for a poly body's
            // own field access are the one exception, deferred to phase 5
            // (R8a).
            PolyType::Array(elem, Len::Var(v)) => {
                let elem = self.substitute_generic_field(elem, args, lens, regs.reborrow());
                let Len::Concrete(count) = lens[*v as usize] else {
                    unreachable!(
                        "an instantiation's own length-argument list is always concrete by the time a field is substituted (R2a's field application collapses eagerly only when every length is a literal)"
                    )
                };
                intern_array_type(regs.arrays, elem, count)
            }
            PolyType::Ref(referent, mutable) => {
                let referent = self.substitute_generic_field(referent, args, lens, regs.reborrow());
                intern_ref_type(regs.refs, referent, *mutable)
            }
            PolyType::OwnedCell(payload) => {
                let payload = self.substitute_generic_field(payload, args, lens, regs.reborrow());
                intern_owned_cell_type(regs.cells, payload)
            }
            // R4: ground every argument first, then mint (or memo-hit) the
            // monomorph for the header this field names. R8 has already
            // rejected a *growing* argument at parse time, so the
            // `(header, module, args)` set this can reach is finite.
            PolyType::Generic {
                is_enum,
                idx,
                module,
                args: header_args,
                len_args: header_len_args,
                name: _,
            } => {
                let mut concrete = Vec::with_capacity(header_args.len());
                for a in header_args {
                    concrete.push(self.substitute_generic_field(a, args, lens, regs.reborrow()));
                }
                let concrete_lens: Vec<Len> = header_len_args
                    .iter()
                    .map(|l| match l {
                        Len::Concrete(n) => Len::Concrete(*n),
                        Len::Var(v) => lens[*v as usize].clone(),
                    })
                    .collect();
                if *is_enum {
                    self.instantiate_enum(*idx as usize, &concrete, &concrete_lens, *module, regs)
                } else {
                    self.instantiate_struct(*idx as usize, &concrete, &concrete_lens, *module, regs)
                }
            }
            other => unreachable!(
                "a generic `type:` field cannot have shape {other:?}: a quotation field naming a type variable is rejected at the parser, a quotation-literal marker never reaches a declaration"
            ),
        }
    }

    /// A registry whose instantiations will be appended onto concrete
    /// registries of the given lengths. The only constructor: a `Default`
    /// would hand out `(0, 0)` bases silently, and a base that does not
    /// match the registry an instantiation is appended to mints a
    /// `StructId` pointing at some other declaration.
    pub fn with_bases(struct_base: usize, enum_base: usize) -> GenericTypes {
        GenericTypes {
            structs: Vec::new(),
            enums: Vec::new(),
            inst_structs: Vec::new(),
            inst_enums: Vec::new(),
            struct_keys: Vec::new(),
            enum_keys: Vec::new(),
            struct_resolved: Vec::new(),
            enum_resolved: Vec::new(),
            struct_pending: Vec::new(),
            enum_pending: Vec::new(),
            deferred_structs: Vec::new(),
            deferred_enums: Vec::new(),
            struct_base,
            enum_base,
        }
    }

    /// P7 slice 3a phase 2 (R2): re-point the base a *fresh* mint counts
    /// from, to the live registries' current length. Called right before a
    /// downstream (check/lowering-time) mint, after every earlier batch
    /// (parse-time or a prior downstream one) has already been flushed --
    /// otherwise a fresh mint's id would land inside the space an
    /// unflushed earlier batch still occupies (the id-collision trap R2
    /// documents). Never invalidates an *already-minted* entry's id: those
    /// are read back from `struct_resolved`/`enum_resolved`, never
    /// recomputed from the (now-advanced) base.
    pub fn rebase(&mut self, struct_len: usize, enum_len: usize) {
        self.struct_base = struct_len;
        self.enum_base = enum_len;
    }

    /// Move this batch's staged parse-time instantiations onto the live
    /// registry, in place (`mem::take` rather than draining by value), so
    /// `self` stays a fully valid, movable `GenericTypes` afterward -- the
    /// whole point of keeping it alive into check/lowering (R2).
    pub fn flush_structs_into(&mut self, live: &mut Vec<StructDecl>) {
        live.extend(std::mem::take(&mut self.inst_structs));
    }

    /// The enum twin of `flush_structs_into`.
    pub fn flush_enums_into(&mut self, live: &mut Vec<EnumDecl>) {
        live.extend(std::mem::take(&mut self.inst_enums));
    }

    /// P7.S12 (review, R1.1 fix): the decl for an `EnumId` this batch minted
    /// but has not flushed into `Module::enums` yet -- a poly body's own
    /// walk can mint one mid-walk (`GenericTypes::instantiate_enum`), and
    /// `enum_base + inst_enums.len()` is exactly the range `flush_enums_into`
    /// has not appended. `None` for an id already flushed (or a hand-written
    /// concrete enum), which the caller falls back to indexing `enums` for.
    pub fn enum_decl(&self, id: EnumId) -> Option<&EnumDecl> {
        id.index()
            .checked_sub(self.enum_base)
            .and_then(|i| self.inst_enums.get(i))
    }

    /// The struct twin of `enum_decl` -- a `StructId` this batch minted but
    /// has not flushed into `Module::structs` yet. `None` for an id already
    /// flushed (or a hand-written concrete struct).
    pub fn struct_decl(&self, id: StructId) -> Option<&StructDecl> {
        id.index()
            .checked_sub(self.struct_base)
            .and_then(|i| self.inst_structs.get(i))
    }

    /// Read-only mint lookup: the already-resolved `Type` for one
    /// application of generic struct `idx`, if this exact `(idx, module,
    /// args)` key has ever been minted (parse-time or downstream). Used by
    /// lowering (`subst_polytype`), which only ever looks up an
    /// instantiation check has already minted, never mints one itself (the
    /// same division the array/ref arms already draw).
    ///
    /// P7.S6a (R5): `lens` joins `(idx, module, args)` in the dedup key, so
    /// `Buffer[u8 256]` and `Buffer[u8 512]` -- identical `idx`/`module`/
    /// `args`, distinct lengths -- do not collide onto the same lookup hit.
    pub fn lookup_struct(
        &self,
        idx: usize,
        module: u32,
        args: &[Type],
        lens: &[Len],
    ) -> Option<Type> {
        self.struct_keys
            .iter()
            .position(|(gi, m, a, l)| *gi == idx && *m == module && a == args && l == lens)
            .map(|i| self.struct_resolved[i])
    }

    /// The enum twin of `lookup_struct`.
    pub fn lookup_enum(
        &self,
        idx: usize,
        module: u32,
        args: &[Type],
        lens: &[Len],
    ) -> Option<Type> {
        self.enum_keys
            .iter()
            .position(|(gi, m, a, l)| *gi == idx && *m == module && a == args && l == lens)
            .map(|i| self.enum_resolved[i])
    }

    /// R2: the reverse of `instantiate_struct`'s dedup lookup -- given a
    /// `StructId` some earlier mint (parse-time or downstream) already
    /// produced, the `(generic decl idx, instantiating module, concrete
    /// arguments)` it was minted from, if `id` names an instantiation at
    /// all (a hand-written concrete struct is never in `struct_resolved`,
    /// so this correctly answers `None` for one). `unify_poly_input`'s
    /// `Generic` arm uses this to unify a concrete stack operand against a
    /// declared `Result['T 'E]`-shaped input: it needs to recover the *args*
    /// a concrete `Result[i64 str]` was built from, to bind `'T`/`'E`
    /// against them.
    ///
    /// P7.S6a (R8a): `struct_keys`' entry tuple carries a fourth,
    /// length-argument element, now exposed as this function's own fourth
    /// return component alongside the type arguments.
    pub fn struct_instantiation_of(&self, id: StructId) -> Option<(usize, u32, &[Type], &[Len])> {
        let i = self
            .struct_resolved
            .iter()
            .position(|t| matches!(t, Type::Struct(sid, _) if *sid == id))?;
        let (gi, m, args, lens) = &self.struct_keys[i];
        Some((*gi, *m, args, lens))
    }

    /// The enum twin of `struct_instantiation_of`.
    pub fn enum_instantiation_of(&self, id: EnumId) -> Option<(usize, u32, &[Type], &[Len])> {
        let i = self
            .enum_resolved
            .iter()
            .position(|t| matches!(t, Type::Enum(eid, _) if *eid == id))?;
        let (gi, m, args, lens) = &self.enum_keys[i];
        Some((*gi, *m, args, lens))
    }

    /// P7.S3n (R2): register a generic struct header with an empty field
    /// list and mark it pending, so a self-reference *inside that header's
    /// own field list* has a header to resolve against. `parse_generic_
    /// typedefs`' stage (b) fills the fields through `fill_struct_fields`.
    pub fn push_struct_placeholder(&mut self, decl: GenericStructDecl) -> usize {
        self.structs.push(decl);
        let idx = self.structs.len() - 1;
        self.struct_pending.push(idx);
        idx
    }

    /// The enum twin of `push_struct_placeholder`.
    pub fn push_enum_placeholder(&mut self, decl: GenericEnumDecl) -> usize {
        self.enums.push(decl);
        let idx = self.enums.len() - 1;
        self.enum_pending.push(idx);
        idx
    }

    /// P7.S3n (R2): fill placeholder header `idx`'s real field list, clear
    /// its pending flag, and pay off every instantiation that was minted
    /// against it while it was still pending -- their `StructId`s were handed
    /// out already, so only `fields` is recomputed, in place.
    pub fn fill_struct_fields(
        &mut self,
        idx: usize,
        fields: Vec<(String, PolyType)>,
        mut regs: MutRegistries,
    ) {
        self.structs[idx].fields = fields;
        self.struct_pending.retain(|p| *p != idx);
        let owed: Vec<(usize, Vec<Type>, Vec<Len>)> = self
            .deferred_structs
            .iter()
            .filter(|(_, header, _, _)| *header == idx)
            .map(|(inst, _, args, lens)| (*inst, args.clone(), lens.clone()))
            .collect();
        self.deferred_structs
            .retain(|(_, header, _, _)| *header != idx);
        for (inst, args, lens) in owed {
            let fields = self.substituted_struct_fields(idx, &args, &lens, regs.reborrow());
            self.inst_structs[inst].fields = fields;
        }
    }

    /// The enum twin of `fill_struct_fields`. Unlike the struct side this
    /// needs `regs`: a monomorphized variant's *name* carries the argument
    /// spelling (`Ok[i64]`), and a placeholder header had no variants at
    /// all, so the whole `VariantDecl` list -- names included -- is built
    /// here rather than only its field types being replaced.
    pub fn fill_enum_variants(
        &mut self,
        idx: usize,
        variants: Vec<GenericVariantDecl>,
        mut regs: MutRegistries,
    ) {
        self.enums[idx].variants = variants;
        self.enum_pending.retain(|p| *p != idx);
        let owed: Vec<(usize, Vec<Type>, Vec<Len>)> = self
            .deferred_enums
            .iter()
            .filter(|(_, header, _, _)| *header == idx)
            .map(|(inst, _, args, lens)| (*inst, args.clone(), lens.clone()))
            .collect();
        self.deferred_enums
            .retain(|(_, header, _, _)| *header != idx);
        for (inst, args, lens) in owed {
            let name = self.inst_enums[inst].name.clone();
            let variants =
                self.substituted_enum_variants(idx, &args, &lens, &name, regs.reborrow());
            self.inst_enums[inst].variants = variants;
        }
    }

    /// One instantiation's concrete field list: header `idx`'s declared
    /// fields with `args` substituted in. Split out of `instantiate_struct`
    /// so `fill_struct_fields` can recompute exactly the same list for an
    /// instantiation that was minted before the header had any fields.
    ///
    /// R6: the field list is **cloned** before substituting, not
    /// `mem::take`n. Substitution needs `&mut self` (its `Generic` arm
    /// re-enters the instantiator), so the borrow of `self.structs[idx]`
    /// cannot stay live -- but taking the declaration's own list would leave
    /// the header fieldless for a re-entrant instantiation at a *different*
    /// argument list, which is exactly the permuting self-reference case.
    fn substituted_struct_fields(
        &mut self,
        idx: usize,
        args: &[Type],
        lens: &[Len],
        mut regs: MutRegistries,
    ) -> Vec<(String, Type)> {
        let fields = self.structs[idx].fields.clone();
        let mut out = Vec::with_capacity(fields.len());
        for (fname, pty) in &fields {
            let ty = self.substitute_generic_field(pty, args, lens, regs.reborrow());
            out.push((fname.clone(), ty));
        }
        out
    }

    /// The enum twin of `substituted_struct_fields`. `name` is the enclosing
    /// instantiation's own mangled name, which each variant's `display`
    /// spelling is built from. An explicit loop over cloned variants for the
    /// same reason the struct twin clones: a `.map()` closure would hold the
    /// borrow of `self.enums[idx]` live across a `&mut self` substitution.
    fn substituted_enum_variants(
        &mut self,
        idx: usize,
        args: &[Type],
        lens: &[Len],
        name: &str,
        mut regs: MutRegistries,
    ) -> Vec<VariantDecl> {
        let variants = self.enums[idx].variants.clone();
        let mut out = Vec::with_capacity(variants.len());
        for variant in &variants {
            let vname = type_instantiation_name(&variant.name, args, lens, regs.names());
            let display = format!("{name}.{}", generic_surface_name(&variant.name));
            let mut fields = Vec::with_capacity(variant.fields.len());
            for (fname, pty) in &variant.fields {
                let ty = self.substitute_generic_field(pty, args, lens, regs.reborrow());
                fields.push((fname.clone(), ty));
            }
            out.push(VariantDecl {
                name_static: Box::leak(vname.clone().into_boxed_str()),
                name: vname,
                display_static: Box::leak(display.into_boxed_str()),
                fields,
                span: variant.span,
            });
        }
        out
    }

    /// The generic struct declaration `name` names in `module`, if any.
    /// `module` is the *declaring* module: an application spells it out
    /// through an import qualifier, exactly as a concrete cross-module type
    /// reference does.
    pub fn find_struct(&self, name: &str, module: u32) -> Option<usize> {
        self.structs
            .iter()
            .position(|d| d.name == name && d.module == module)
    }

    /// The enum twin of `find_struct`.
    pub fn find_enum(&self, name: &str, module: u32) -> Option<usize> {
        self.enums
            .iter()
            .position(|d| d.name == name && d.module == module)
    }

    /// R4/R5: mint (or find) the concrete struct for one application of
    /// generic struct `idx`, deduped structurally on `(generic decl idx,
    /// module, concrete arguments)` -- compared by `Type`'s own `Eq`, which
    /// is exact over the `StructId`/`EnumId`/etc. an argument carries. This
    /// is deliberately *not* the rendered `type_instantiation_name` string:
    /// that name is built from `Type::name()`, which two distinct arguments
    /// (two structs sharing a bare declared name across modules) can render
    /// identically, so deduping on the string would silently collapse them
    /// into one `StructId` with the wrong layout. The result is an ordinary
    /// `StructDecl`, indistinguishable from a hand-written concrete `type:`
    /// of the same shape.
    ///
    /// P7.S6a (R5): takes a length-argument list alongside `args`, so
    /// `Buffer[u8 256]` and `Buffer[u8 512]` mint distinct monomorphs
    /// instead of colliding on `struct_keys`' old `(idx, module, args)`
    /// key. `lens` is expected concrete (`Len::Concrete`) by the time a real
    /// mint reaches here -- R6/R7's parse-time application never produces a
    /// `Len::Var` in this position -- but two check-time callers
    /// (`poly_construct_generic`, `apply_subst`'s `Generic`/`GenericVariant`
    /// arms) still pass an explicit empty placeholder in this phase, ahead
    /// of R8a's real `subst.len`-resolved value (see those call sites).
    pub fn instantiate_struct(
        &mut self,
        idx: usize,
        args: &[Type],
        lens: &[Len],
        module: u32,
        mut regs: MutRegistries,
    ) -> Type {
        if let Some(ty) = self.lookup_struct(idx, module, args, lens) {
            return ty;
        }
        let name = type_instantiation_name(&self.structs[idx].name, args, lens, regs.names());
        let span = self.structs[idx].span;
        let name_static: &'static str = Box::leak(name.clone().into_boxed_str());
        let id = StructId::from_index(self.struct_base + self.inst_structs.len());
        let ty = Type::Struct(id, name_static);
        // R6: mint the id, the memo key, the resolved type and a *fieldless
        // placeholder decl* before substituting anything. A field that names
        // this same header at these same arguments re-enters here during that
        // substitution; with the memo already in place it hits the lookup
        // above and returns this id instead of recursing forever. All four
        // pushes stay in lockstep: `struct_keys`, `struct_resolved` and
        // `inst_structs` are parallel vectors, and the minted id is
        // `struct_base + inst_structs.len()`.
        self.struct_keys
            .push((idx, module, args.to_vec(), lens.to_vec()));
        self.struct_resolved.push(ty);
        let inst = self.inst_structs.len();
        self.inst_structs.push(StructDecl {
            name,
            name_static,
            fields: Vec::new(),
            span,
            has_drop_overload: false,
            is_bundle: false,
            module,
        });
        // P7.S3n (R2): a header still being *registered* has no declared
        // fields to substitute yet, so computing them now would permanently
        // memoize a fieldless struct with no diagnostic. The id is already
        // handed out -- a `Type::Struct` is an opaque handle -- so only the
        // field list is owed, to `fill_struct_fields`.
        if self.struct_pending.contains(&idx) {
            self.deferred_structs
                .push((inst, idx, args.to_vec(), lens.to_vec()));
        } else {
            let fields = self.substituted_struct_fields(idx, args, lens, regs.reborrow());
            self.inst_structs[inst].fields = fields;
        }
        ty
    }

    /// The enum twin of `instantiate_struct`. A variant's name carries the
    /// same argument spelling as its enum's (`Ok[i64 str]`): a variant name
    /// keys the generated-constructor `Sig` and the lowering-side variant
    /// word map, so two instantiations sharing a bare `Ok` would silently
    /// clobber each other there exactly as two `Box` constructors would.
    pub fn instantiate_enum(
        &mut self,
        idx: usize,
        args: &[Type],
        lens: &[Len],
        module: u32,
        mut regs: MutRegistries,
    ) -> Type {
        if let Some(ty) = self.lookup_enum(idx, module, args, lens) {
            return ty;
        }
        let name = type_instantiation_name(&self.enums[idx].name, args, lens, regs.names());
        let span = self.enums[idx].span;
        let name_static: &'static str = Box::leak(name.clone().into_boxed_str());
        let id = EnumId::from_index(self.enum_base + self.inst_enums.len());
        let ty = Type::Enum(id, name_static);
        // R6/R2: the struct twin's reasoning and its three-vector lockstep,
        // verbatim.
        self.enum_keys
            .push((idx, module, args.to_vec(), lens.to_vec()));
        self.enum_resolved.push(ty);
        let inst = self.inst_enums.len();
        self.inst_enums.push(EnumDecl {
            name: name.clone(),
            name_static,
            variants: Vec::new(),
            span,
            module,
        });
        if self.enum_pending.contains(&idx) {
            self.deferred_enums
                .push((inst, idx, args.to_vec(), lens.to_vec()));
        } else {
            let variants = self.substituted_enum_variants(idx, args, lens, &name, regs.reborrow());
            self.inst_enums[inst].variants = variants;
        }
        ty
    }
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

/// P7 slice 3i: the source spelling of the boolean type, named once so the
/// registry lookup and every hand-written bool-keyed arm cannot drift from
/// `lib/bool.sth`'s declaration.
pub const BOOL_TYPE_NAME: &str = "Bool";

/// P7 slice 3i (R4): the boolean type this build resolved -- the enum named
/// `bool`, declared once as ordinary source in `core::bool` and reached like any
/// other imported type. It is looked up rather than baked in as a constant
/// because its registry position is discovery-order dependent: no reserved slot
/// is held for it.
///
/// The shape test is load-bearing, not decoration. It is what the callers that
/// treat a bool as a register-resident scalar (the logical operators, the
/// `extern:` boundary set) rest on, so a same-named enum carrying a payload
/// cannot inherit that treatment by naming alone.
///
/// First match wins, and this reads the whole merged registry rather than one
/// module's import scope (which is what keeps it out of every `check_term`
/// signature). A program that declares its own payload-free `bool` ahead of
/// `core::bool`'s in discovery order therefore takes the logical operators with
/// it: `true false and` there is a refused `and` (an operand-mismatch
/// diagnostic naming `bool` twice, confusingly), never a mistyped or
/// miscompiled one -- both candidates are 1-byte scalar enums, and every other
/// bool-keyed decision resolves through the module's own imports
/// (`parse_static_decl`) or the type's name and layout (`:stack` rendering).
pub fn resolve_bool_type(enums: &[EnumDecl]) -> Option<Type> {
    enums
        .iter()
        .position(|e| {
            e.name_static == BOOL_TYPE_NAME
                && e.variants.len() == 2
                && e.variants.iter().all(|v| v.fields.is_empty())
        })
        .map(|idx| Type::Enum(EnumId(idx), enums[idx].name_static))
}

/// A registered array type: its element type, compile-time count, and the
/// leaked `&'static str` spelling `array[T N]` every `Type::Array` naming it
/// carries directly (mirrors `StructDecl::name_static`). Interned and deduped
/// structurally by `(element, count)` shape (D3, M1): two spellings of the
/// same shape share one `ArrayDecl`/`ArrayId`.
#[derive(Debug, Clone)]
pub struct ArrayDecl {
    pub element: Type,
    pub count: u32,
    pub name_static: &'static str,
}

/// A registered owning-cell type: its payload type and the leaked `&'static
/// str` spelling `^T` every `Type::OwnedCell` naming it carries directly.
/// Deduped structurally by payload shape; unlike `ArrayDecl` there is no
/// count, since a cell holds exactly one value.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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

/// A registered slice type: the element it views, whether the view is
/// mutable (`!Slice[T]`) or shared (`Slice[T]`), and the leaked `&'static
/// str` spelling every `Type::Slice` naming it carries directly. Deduped
/// structurally by `(element, mutable)`, mirroring `RefDecl`: a slice views
/// storage it does not own, so like a reference there is nothing to free.
/// There is no count -- the length is a runtime component of the value, which
/// is the whole difference from `ArrayDecl`.
#[derive(Debug, Clone)]
pub struct SliceDecl {
    pub element: Type,
    pub mutable: bool,
    pub name_static: &'static str,
}

/// A small `Copy` index into `Module::slices`, mirroring `RefId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SliceId(pub(crate) usize);

impl SliceId {
    /// Mint a `SliceId` for a registry position; crate-internal so an id is
    /// always tied to a real `slices` registry entry.
    pub(crate) fn from_index(idx: usize) -> SliceId {
        SliceId(idx)
    }

    pub fn index(&self) -> usize {
        self.0
    }
}

/// Intern an `(element, mutable)` slice shape into `slices`, deduping
/// structurally. Keyed on mutability like `intern_ref_type` and unlike
/// `intern_array_type`, so a shared and a mutable view of the same element
/// are distinct types, each byte-identical to its own kind.
pub fn intern_slice_type(slices: &mut Vec<SliceDecl>, element: Type, mutable: bool) -> Type {
    if let Some(idx) = slices
        .iter()
        .position(|d| d.element == element && d.mutable == mutable)
    {
        return Type::Slice(SliceId::from_index(idx), mutable, slices[idx].name_static);
    }
    let name = format!(
        "{}Slice[{}]",
        if mutable { "!" } else { "" },
        element.name()
    );
    let name_static: &'static str = Box::leak(name.into_boxed_str());
    let id = SliceId::from_index(slices.len());
    slices.push(SliceDecl {
        element,
        mutable,
        name_static,
    });
    Type::Slice(id, mutable, name_static)
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
    let name = format!("array[{} {}]", element.name(), count);
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
    // spelling (`array[i64 4]`, `&!Buf`) is not a legal QBE aggregate name, and the
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

/// R1: the builtin words `check_term` dispatches by name, in its probe chain,
/// *before* the word environment is consulted at all. They are absent from
/// `builtin_table` (empty today, since every builtin dispatches on the
/// concrete operand type rather than a fixed signature), so an `extern:`
/// naming one would be registered, never looked up, and silently do nothing. The `^`-led owning-cell words and the `@`/`!`/`+!` access
/// words are dispatched in the same chain but are rejected earlier, against
/// the declaration's name in the parser, so they are not repeated here.
///
/// This table and the two predicates below live in `ast` rather than `check`
/// because `parser` needs them too (P7.S3r R4 rejects a `trait:` member spelled
/// as a name-dispatched builtin), and `ast` is the lowest module both already
/// depend on.
pub(crate) const BUILTIN_WORDS: &[&str] = &[
    // check_shuffle
    "dup",
    "drop",
    "swap",
    "over",
    "rot", // check_operator
    "add",
    "sub",
    "mul",
    "div",
    "mod",
    "and",
    "or",
    "xor",
    "not",
    "shl",
    "shr",
    // Slice 10c (R-P3-3): the comparison primitives, each yielding the 32-bit
    // flag `branch` consumes.
    "ueq",
    "ult",
    "ugt",
    "ulte",
    "ugte",
    "une",
    // The six surface comparison names are `lib/` words now, not name-
    // dispatched builtins, but they stay listed: this set is also what stops
    // a bare tail call being read as a call to the enclosing word
    // (`has_self_tail_call`), and a trailing `lt` inside a user's own `Vec2 lt`
    // is still far more often the library `lt` on two scalars than a self-call.
    "eq",
    "lt",
    "gt",
    "lte",
    "gte",
    "ne",
    ".",
    // Slice 10c (R-P3-1/R-P3-2): the two control/discriminant primitives.
    "branch",
    "tag",
    "max",
    "max-total", // check_str_word
    "len",
    "cstr", // check_array_word (`len` is shared with `check_str_word`)
    "fill",
];

/// R1: whether `name` is dispatched as a builtin ahead of any environment
/// lookup. Beyond the fixed names, `check_operator` claims every `>`-prefixed
/// name with a non-empty remainder as a numeric conversion (`>u8`), erroring
/// on an unrecognised target type rather than falling through, so no such
/// name can reach a registered signature either. A bare `>` with no suffix
/// falls through this filter; the comparison operator, now spelled `gt`, is
/// in the list separately.
pub(crate) fn is_builtin_word_name(name: &str) -> bool {
    BUILTIN_WORDS.contains(&name) || name.strip_prefix('>').is_some_and(|rest| !rest.is_empty())
}

/// The names `check_term` really does dispatch ahead of the word environment:
/// `is_builtin_word_name` *minus* the six surface comparisons. Those six are
/// `lib/` words: they left `BUILTIN_TABLE` in slice 10c and are listed in
/// `BUILTIN_WORDS` only so `has_self_tail_call` does not read a trailing `lt`
/// as a self-call.
///
/// Two consumers. P8 S2 (R2): the set the `intrinsics` import gates -- the six
/// live in `core::cmp`, so gating them would answer an unimported `lt` with
/// "add `import: intrinsics *`", pointing at the wrong module. P7.S3r (R4): the
/// set a `trait:` member name may not be spelled as, since an impl body binds
/// its own member name ahead of module scope and would shadow the builtin
/// there; excluding the six keeps an `eq`/`lt` member legal, shadowing only a
/// library word.
///
/// `.` is *not* in that exclusion set. It is a genuine table intrinsic (a
/// `Print` row per printable type, dispatched by `check_operator`) and does not
/// move to `core`, so a bare `.` with no `intrinsics` import is correctly the
/// import error.
pub(crate) fn is_name_dispatched_builtin(name: &str) -> bool {
    if matches!(name, "eq" | "lt" | "gt" | "lte" | "gte" | "ne") {
        return false;
    }
    is_builtin_word_name(name)
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
    /// A word's body: a term sequence. Entry locals are not a field here: a
    /// `| names |` binding is a `TermKind::Bind` term like any other, and the
    /// entry position is just the first one (R1).
    pub body: Vec<Term>,
    /// R4 (phase 4 slice 1): the polymorphic signature, present only when the
    /// declared effect mentions a type variable `'T`, a length variable `'N`,
    /// or the row variable `..s`. `None` for a monomorphic word, whose whole
    /// signature is `effect`.
    pub poly: Option<Box<PolySig>>,
    /// Slice 11 (R1): the declared `inline` keyword, spelled between the name
    /// and the effect. It makes "always spliced at the call site" a *declared*
    /// property rather than one inferred from the signature's shape, so a word
    /// taking no quotation can still mint no `IrFunc` and no call
    /// (`is_combinator`, the single predicate `check` and `ir::lower` share, is
    /// the only load-bearing reader). The guarantee is unconditional: a shape
    /// that cannot be spliced (`main`, a builtin-operator name) is a located
    /// error at the definition, never a silent fall-back to a real call.
    pub declares_inline: bool,
    /// Phase 4 slice 5a (R10): the owning module id, mirroring
    /// `StructDecl::module`.
    pub module: u32,
    /// The declaration site (the word's name token), used by every
    /// diagnostic that must point at this word regardless of its body shape.
    /// Kept separate from `word_span`'s old first-term fallback
    /// (which is `Span::default()`, i.e. line 0 col 0, for an empty body --
    /// `: main ( -- ) ;` and every other trivial stub word hit this) so a
    /// located error always has somewhere real to point.
    pub span: Span,
    /// Phase 7 slice 2 (D2/D4): the word's own trailing `global:` clause,
    /// sitting right after the effect's closing `)` and before the body --
    /// `None` when no clause was written (byte-for-byte unchanged parse, the
    /// additive regression guarantee). `Some(vec![])` is not representable: a
    /// bare `global:` with no entry is a located parse error.
    pub declared_globals: Option<Vec<GlobalEntry>>,
}

/// Phase 7 slice 2 (D1/D4): one `static:` module-level declaration -- a
/// never-owned, never-moved, never-dropped place reached only through the
/// existing `&`/`&!` sigil. Scalar types only this slice (`i64`/`u32`/`bool`/
/// `str`); a struct-typed static is rejected at the declaration (OQ1, deferred
/// to Phase 9). No `Type` variant is added for this: a static's ref is exactly
/// `&T`/`&!T` for its declared `T`, interned the same way any other reference
/// is (D4, decision 3).
#[derive(Debug, Clone)]
pub struct StaticDecl {
    pub name: String,
    pub ty: Type,
    pub init: StaticInit,
    /// Phase 4 slice 5a (R10) twin: the owning module id, mirroring
    /// `StructDecl::module`. A static is module-private (R2): never exported,
    /// never imported.
    pub module: u32,
    pub span: Span,
}

/// D1/D3: a static's initialiser, elided or a single literal -- no
/// arithmetic, no reference to another static, no struct-literal aggregate.
/// `Zero` is the type's zero value: `0` for an integer, `false` for `bool`,
/// and the empty string `""` for `str`.
#[derive(Debug, Clone, PartialEq)]
pub enum StaticInit {
    Zero,
    Int(i64),
    Bool(bool),
    Str(String),
}

/// D2: one `NAME mode` entry of a word's `global:` clause.
#[derive(Debug, Clone)]
pub struct GlobalEntry {
    pub name: String,
    pub mode: GlobalMode,
    /// The entry's `NAME` token, for the exact-match diagnostic (R6).
    pub span: Span,
}

/// D2: a `global:` entry's declared access mode. Decision 5: mode is derived
/// from the body by the checker, never independently authored -- this is what
/// the declared clause is checked *against*, not trusted verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalMode {
    R,
    W,
}

/// R3/R6 (phase 4 slice 1): a capability a type variable can be bounded by.
/// `Copy` gates `dup`/`over`, resolved at the concrete instantiation by the
/// existing predicate (`is_copy`), Kitten-style, with no trait objects.
///
/// P7.S3e (R6): `User` is the second variant -- a `trait:` declaration
/// satisfied nominally by an `impl:` binding, resolved at the concrete
/// instantiation by a whole-program `(TraitId, Type)` registry lookup instead
/// of a predicate. P7.S3s: `Ord` used to be a third, reserved predicate
/// variant (numeric-tower membership); it is now an ordinary library trait
/// declared in `core::cmp` and satisfied through `User` like any other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bound {
    Copy,
    User(TraitId),
}

/// P7.S3e (R3): a whole-program index into `Module::traits`, mirroring
/// `StructId`/`EnumId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraitId(pub(crate) usize);

impl TraitId {
    /// Mint a `TraitId` for a registry position; crate-internal so an id is
    /// always tied to a real `Module::traits` entry.
    pub(crate) fn from_index(idx: usize) -> TraitId {
        TraitId(idx)
    }

    pub fn index(&self) -> usize {
        self.0
    }
}

/// P7.S3e (R2, decision 1): the two shapes a `Module::traits` entry can take.
/// `Predicate` is a pre-seeded `Copy`/`Ord` entry (R2): satisfaction still
/// runs `poly_is_copy`/`is_ord` unchanged, never an `impl:` lookup. `Nominal`
/// is a user `trait:` declaration, satisfied by a whole-program `impl:`
/// registry lookup keyed by `(TraitId, Type)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraitKind {
    Predicate(Bound),
    Nominal,
}

/// P7.S3e (R2, decision 2/fresh review): the reserved sentinel `module` value
/// carried by a pre-seeded `Copy`/`Ord` entry. It collides with every real
/// module for duplicate-name purposes (a user `trait: Copy` in any module is
/// rejected) but participates in no orphan-rule or export-gating check, which
/// both compare against a real declaring module.
pub const RESERVED_TRAIT_MODULE: u32 = u32::MAX;

/// P7.S3e (R3): a whole-program trait declaration, mirroring `StructDecl`'s
/// flat-registry shape (`module: u32` per entry, indexed by `TraitId`).
#[derive(Debug, Clone)]
pub struct TraitDecl {
    pub name: String,
    pub kind: TraitKind,
    /// R1 (single-type-variable traits only): every member's signature
    /// shares one implicit type variable, id 0 in its own `PolySig`.
    pub members: Vec<TraitMember>,
    pub module: u32,
    pub span: Span,
}

/// P7.S3e (R3): one required member of a trait -- a name and a signature over
/// the trait's own (single, implicit) type variable.
#[derive(Debug, Clone)]
pub struct TraitMember {
    pub name: String,
    pub sig: PolySig,
    /// P7.S3s-follow: set by the optional `inline` keyword between the member
    /// name and its `(` in `trait: ... ;`, read by `parse_impl_member_body` so
    /// every `impl:` body satisfying this member is spliced at its call sites
    /// instead of costing a call frame.
    pub declares_inline: bool,
}

/// P7.S3e (R4/R8): ground a trait member's declared `PolyType` (over the
/// trait's sole implicit type variable, id 0) against a concrete `impl:`
/// target, interning a fresh array/reference shape if the grounded shape is
/// new (deduped by `intern_array_type`/`intern_ref_type`, so this never
/// double-registers one already interned elsewhere). Trait member
/// signatures are restricted to concrete/array/reference shapes over `'T`
/// (`parse_trait_member_effect` rejects anything else at declaration time),
/// so every other `PolyType` shape is unreachable here.
///
/// P7.S3r (R2): the single grounding rule. The parser grounds a trait member
/// here to synthesize the impl body's word, and `check::poly` grounds the same
/// member here to render it in a diagnostic, so the effect a body is checked
/// against and the effect an error names cannot drift apart.
pub fn ground_member_type(
    pty: &PolyType,
    target: Type,
    arrays: &mut Vec<ArrayDecl>,
    refs: &mut Vec<RefDecl>,
) -> Type {
    match pty {
        PolyType::Concrete(t) => *t,
        PolyType::Var(_) => target,
        PolyType::Array(elem, Len::Concrete(n)) => {
            let elem_ty = ground_member_type(elem, target, arrays, refs);
            intern_array_type(arrays, elem_ty, *n)
        }
        PolyType::Ref(referent, mutable) => {
            let r = ground_member_type(referent, target, arrays, refs);
            intern_ref_type(refs, r, *mutable)
        }
        _ => unreachable!(
            "trait member signatures are restricted to concrete/array/reference shapes over 'T (parse_trait_member_effect rejects the rest)"
        ),
    }
}

/// P7.S4 (R5): ground a trait member's declared `PolyType` (over the trait's
/// sole implicit type variable, id 0) against a *generic* `impl:` target —
/// binding `PolyType::Var(0)` to the whole target `PolyType` and recursing
/// over every other shape. Unlike `ground_member_type` (which grounds to a
/// concrete `Type`), this returns a `PolyType` over the impl's own variables,
/// yielding the polymorphic member word's `PolySig`.
pub fn ground_member_poly(pty: &PolyType, target: &PolyType) -> PolyType {
    match pty {
        PolyType::Concrete(t) => PolyType::Concrete(*t),
        PolyType::Var(_) => target.clone(),
        PolyType::Array(elem, len) => {
            PolyType::Array(Box::new(ground_member_poly(elem, target)), len.clone())
        }
        PolyType::Ref(referent, mutable) => {
            PolyType::Ref(Box::new(ground_member_poly(referent, target)), *mutable)
        }
        PolyType::OwnedCell(payload) => {
            PolyType::OwnedCell(Box::new(ground_member_poly(payload, target)))
        }
        PolyType::Generic {
            is_enum,
            idx,
            module,
            args,
            len_args,
            name,
        } => PolyType::Generic {
            is_enum: *is_enum,
            idx: *idx,
            module: *module,
            args: args.iter().map(|a| ground_member_poly(a, target)).collect(),
            len_args: len_args.clone(),
            name,
        },
        PolyType::Quotation(ins, outs, is_inline, row_in, row_out) => PolyType::Quotation(
            ins.iter().map(|p| ground_member_poly(p, target)).collect(),
            outs.iter().map(|p| ground_member_poly(p, target)).collect(),
            *is_inline,
            *row_in,
            *row_out,
        ),
        PolyType::QuotLit => {
            unreachable!("a quotation-literal marker never reaches a signature")
        }
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row (R3.5); a trait member signature never carries one"
        ),
    }
}

/// The shared, lazily-built `Copy` entry (`seed_predicate_traits`), for a
/// caller that only ever needs the reserved predicate table and no user
/// `trait:` declarations -- the type pre-pass parsing a signature that can
/// still name a bound (`'T: Copy`) before any user trait is registered, and
/// `check/poly.rs`'s scratch contexts, which carry no user bound at all.
/// P7.S3s: `Ord` used to be pre-seeded here too; it is now an ordinary
/// library trait declared in `core::cmp` (R8).
pub fn predicate_traits() -> &'static [TraitDecl] {
    static TRAITS: std::sync::OnceLock<Vec<TraitDecl>> = std::sync::OnceLock::new();
    TRAITS.get_or_init(seed_predicate_traits)
}

/// Pre-seed the whole-program trait registry with `Copy` as a
/// `Predicate`-kind entry (R2), so `parse_capabilities` looks it up through
/// the same trait-table mechanism a user trait uses, rather than a bespoke
/// reserved-word check. Called once, before any user `trait:` declaration is
/// registered.
pub fn seed_predicate_traits() -> Vec<TraitDecl> {
    vec![TraitDecl {
        name: "Copy".to_string(),
        kind: TraitKind::Predicate(Bound::Copy),
        members: Vec::new(),
        module: RESERVED_TRAIT_MODULE,
        span: Span::default(),
    }]
}

/// P7.S4 (R1): the target of an `impl:` declaration, carrying the pattern
/// the registry matches against together with the impl's own variable name
/// tables (mirroring `PolySig`'s `ty_var_names`/`len_var_names`). A concrete
/// target (`Point`, `array[i64 4]`) folds to `PolyType::Concrete(t)` and behaves
/// exactly as before; a generic target (`['T N]`, `'T`, `Box['T]`) carries
/// variables and the member word is polymorphic.
#[derive(Debug, Clone)]
pub struct ImplTarget {
    pub pattern: PolyType,
    pub ty_var_names: Vec<String>,
    pub len_var_names: Vec<String>,
    /// P7.S4b (R2): bounds declared on the impl's own type variables via a
    /// `where`-clause (`impl: Show for array['T 'N] where 'T: Show`). Each pair's
    /// `u32` is an index into `ty_var_names`, mirroring `PolySig::bounds`.
    /// Empty when no `where`-clause is present (today's behaviour).
    pub bounds: Vec<(u32, Bound)>,
}

impl ImplTarget {
    /// Whether this is a concrete target — a `PolyType::Concrete(t)` — which
    /// keeps the existing monomorphic member-word path. Everything else
    /// (`Var`, `Array`, `Ref`, `OwnedCell`, `Generic`, `Quotation`) is generic.
    pub fn is_concrete(&self) -> bool {
        matches!(self.pattern, PolyType::Concrete(_))
    }

    /// The concrete `Type` of a concrete target, or `None` for a generic one.
    ///
    /// P7.S12 phase 2 (R3.4): `pattern` is an `impl:` target pattern, a shape
    /// with no eliminator arm to ground it in, and `GenericVariant` is
    /// unconstructible outside one (R3.5). The `_ =>` here can never see a
    /// `GenericVariant`; no conversion needed.
    pub fn concrete_ty(&self) -> Option<Type> {
        match self.pattern {
            PolyType::Concrete(t) => Some(t),
            _ => None,
        }
    }
}

/// One `impl: Trait for Type ... ;` declaration. `bindings` is a name map
/// (member name -> implementing word name), populated by the parser's
/// body-form desugar: each `: member ... ;` inside the block synthesizes a
/// top-level word carrying the trait member's signature grounded at the
/// `impl:` target, and records the (member, synth-name) pair here.
///
/// P7.S4 (R1): the target is an `ImplTarget` (a `PolyType` pattern plus the
/// impl's own variable name tables), not a bare `Type`. A concrete target
/// (`PolyType::Concrete(t)`) keeps the existing monomorphic path; a generic
/// target (`['T N]`, `'T`) carries variables and the member word is
/// polymorphic (`poly: Some(..)`).
#[derive(Debug, Clone)]
pub struct ImplDecl {
    pub trait_id: TraitId,
    pub target: ImplTarget,
    pub module: u32,
    pub span: Span,
    pub bindings: Vec<(String, String)>,
    /// P7.S3e (R8): each binding's implementing word as an index into
    /// `Module::words`, member name -> index. Filled by `check_impl_decls`,
    /// which resolves it pre-mangle, where a binding's raw word name and a
    /// `WordDef::name` still agree; a bound-directed call site then mints the
    /// symbol from `overload_symbols` so it is byte-identical to the one
    /// lowering emits. Empty on any path that skips that check (a standalone
    /// probe, which declares no `impl:`), so nothing resolves there.
    pub resolved: Vec<(String, usize)>,
}

/// R4: an array count in a polymorphic type: a concrete length or a length
/// variable `'N` (index into `PolySig::len_var_names`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Len {
    Concrete(u32),
    Var(u32),
}

/// R4: a type in a polymorphic signature. A monomorphic sub-type folds to
/// `Concrete`; a variable-bearing array (`array['T 'N]`, `array[i64 'N]`, `array['T 4]`)
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
    /// here. The trailing `bool` is Slice 10a (R1): whether the effect was
    /// declared with the `~` sigil, so the concrete fold and `apply_subst`
    /// know to ground it to `Type::InlineQuotation` rather than
    /// `Type::Quotation`. The two trailing `Option<u32>`s are Slice 10a
    /// (R7): the input/output row variable, if any, in the signature's own
    /// row id space (`PolySig::row_in`/`row_out`) -- a row inside a
    /// quotation effect can only ever denote the signature's top-level row
    /// (R4), so it shares that id space rather than minting its own.
    Quotation(Vec<PolyType>, Vec<PolyType>, bool, Option<u32>, Option<u32>),
    /// Slice 13 (R-A1/D1): a reference whose referent is still polymorphic
    /// (`&'T`, `&array['T 4]`, and their `&!` twins): the referent, then whether
    /// it is mutable. There is deliberately no `RefId` -- the referent may be
    /// a variable, which no registry entry can name; the id is minted only
    /// when the referent grounds to a concrete `Type` (`apply_subst` /
    /// `subst_polytype`, via `intern_ref_type`), and a fully-concrete
    /// referent folds to `Concrete(Type::Ref(..))` at parse time. Mutability
    /// rides the variant for the same reason `Type::Ref` carries it: it is
    /// the classification bit (`Copy`-ness, store-vs-fetch, exclusivity),
    /// asked at sites that hold no registry.
    Ref(Box<PolyType>, bool),
    /// P7.S3n (R3): an owning cell whose payload is still polymorphic
    /// (`^'T`, `^array['T 4]`), deferred for exactly the reason `Ref` documents:
    /// the payload may be a variable, which no registry entry can name, so
    /// the `OwnedCellId` is minted only once the payload grounds
    /// (`apply_subst` / `subst_polytype`, via `intern_owned_cell_type`), and
    /// a fully-concrete payload folds to `Concrete(Type::OwnedCell(..))` at
    /// parse time.
    OwnedCell(Box<PolyType>),
    /// P7 slice 3b (R2): the compile-only marker a quotation *literal* written
    /// inside a non-inline polymorphic body occupies on the poly walk's
    /// virtual stack. Deliberately carries no effect: two literals with one
    /// effect would be one `PolyType`, erasing the per-literal identity the
    /// eliminator needs to pick an arm's body -- that identity rides
    /// `PolySlot::quot` instead. It is not a value type, so every predicate
    /// answers "no" for it (`poly_is_copy`, `is_reference_slot`) and every
    /// type-directed operation rejects it. Minted only by `poly_term`'s
    /// quotation arm and consumed only by `poly_eliminator_call`; a literal
    /// still on the stack at word exit is rejected, so it can never reach a
    /// declared signature -- which is what makes the arms for it outside the
    /// poly walk unreachable rather than merely unexercised.
    QuotLit,
    /// P7 slice 3a (R1/D2): a generic type applied to the enclosing
    /// signature's own variables (`Result['T 'E]`), deferred exactly as
    /// `Ref` defers its `RefId`: there is no `StructId`/`EnumId` to mint
    /// until a substitution grounds every argument. `idx` indexes
    /// `GenericTypes::structs`/`enums` per `is_enum`; `module` is the
    /// *instantiating* module, the third component of `struct_keys`/
    /// `enum_keys`, captured at the naming site. `args` is recursive so
    /// depth > 1 is representable, but v1 rejects it at the parse fold
    /// (D5). `name` is the header's own declared spelling, cached purely
    /// for diagnostics -- mirroring `StructDecl::name_static` -- and carries
    /// no identity of its own: whether two `Generic`s name the same header
    /// is answered by `is_enum`/`idx`/`module` alone.
    Generic {
        is_enum: bool,
        idx: u32,
        module: u32,
        args: Vec<PolyType>,
        /// P7.S6a (R3): the header's own length-argument list, parallel to
        /// `args` -- a `Len::Var` here indexes the *enclosing signature's*
        /// `PolySig::len_var_names`, exactly as `args`' own `PolyType::Var`
        /// does for `ty_var_names`.
        len_args: Vec<Len>,
        name: &'static str,
    },
    /// P7.S12 (R3): a generic enum's variant narrowed by an eliminator arm
    /// (`Option['T]`'s `Some`), the poly twin of `Type::Variant`. Identity is
    /// `(idx, module, vi)`, mirroring `Generic`'s own `(is_enum, idx, module)`
    /// -- `idx`/`module` index the same `GenericTypes::enums` header a
    /// `Generic { is_enum: true, .. }` scrutinee names, `vi` the variant
    /// within it. `args` is the scrutinee's own argument list, carried
    /// forward unchanged (R5.4): nothing re-unifies, since the scrutinee
    /// already carries the substitution. A separate variant rather than a
    /// flag on `Generic`: every predicate that must reject a *variant*
    /// (escape, `Copy`, projection) has to see it without reasoning about a
    /// boolean. `name` is the leaked `Enum.Variant` display spelling
    /// (`generic_variant_type`, the sole constructor), diagnostics only, and
    /// unconstructible outside an eliminator arm's own input row (R3.5): no
    /// parse route, no signature spelling, no constructor elsewhere.
    GenericVariant {
        idx: u32,
        module: u32,
        vi: usize,
        args: Vec<PolyType>,
        /// P7.S6a (R3): carried forward unchanged from the scrutinee's own
        /// `Generic { .. }.len_args`, mirroring `args`' own carry-forward.
        len_args: Vec<Len>,
        name: &'static str,
    },
}

/// P7.S12 (R3.2): the **sole** constructor of a `PolyType::GenericVariant`,
/// mirroring `variant_type`'s role for `Type::Variant`: it leaks
/// `{header name}.{variant surface name}` once, so two constructions of the
/// same `(idx, module, vi)` compare equal (`PolyType` derives `PartialEq` +
/// `Eq`, and `&'static str` equality is content-based regardless of the
/// leak, exactly as `Generic`'s own `name` field already relies on).
pub fn generic_variant_type(
    generics: &GenericTypes,
    idx: u32,
    module: u32,
    vi: usize,
    args: Vec<PolyType>,
    len_args: Vec<Len>,
) -> PolyType {
    let decl = &generics.enums[idx as usize];
    let variant = &decl.variants[vi];
    let display = format!("{}.{}", decl.name, generic_surface_name(&variant.name));
    PolyType::GenericVariant {
        idx,
        module,
        vi,
        args,
        len_args,
        name: Box::leak(display.into_boxed_str()),
    }
}

/// P7.S12 (R4): substitute a generic enum variant field's declared
/// `PolyType` against the eliminated scrutinee's own argument list,
/// symbolically -- an adaptation of `ground_member_poly`'s
/// `PolyType -> PolyType` walk (substituting `Var` against a single target
/// there, against an indexed argument list here), sized to what a variant
/// field can actually be rather than to `ground_member_poly`'s wider arm set.
///
/// R4.2: the identical arm set to `poly_bind_construction_arg`
/// (`src/check/poly.rs`), its dual -- construction binds header variables
/// from operands, destructure applies them to fields -- which is required: a
/// field shape one accepts and the other rejects is a defect. A generic enum
/// *variant* field can only be `Var` or `Concrete` at HEAD (measured:
/// `array['A 2]`, `Inner['A]`, `Cell2['A]`, `&'A` and `^'A` are all parser
/// rejections for a variant field; generic *struct* fields admit a wider set,
/// which is why `substitute_generic_field` carries it -- this function is
/// called only on enum variant fields).
///
/// R4.3: takes no `MutRegistries` and interns nothing -- folding an
/// all-`Concrete` result is `apply_subst`'s job at grounding time, which
/// needs the instantiator this function deliberately does not hold.
pub fn substitute_generic_variant_field(field_pty: &PolyType, args: &[PolyType]) -> PolyType {
    match field_pty {
        PolyType::Var(v) => args[*v as usize].clone(),
        PolyType::Concrete(t) => PolyType::Concrete(*t),
        other => unreachable!(
            "a generic enum variant field is never {other:?}: `array['A N]`, `Inner['A]`, `&'A` and `^'A` are all parser rejections for a variant field"
        ),
    }
}

/// R4: a polymorphic stack effect. The variable id spaces are per-signature
/// (a `Var(0)` in one word is unrelated to a `Var(0)` in another); the
/// `*_var_names` tables carry each id's surface spelling for diagnostics.
///
/// P7.S3e (R8): `Eq` because a call site identifies its callee by name *and*
/// signature together when reading back the obligations the pre-pass recorded
/// for that body -- a polymorphic overload set shares one name across two
/// signatures, and each obligation's variable id indexes its own signature.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// P7.S3f (R2): the positions among the declared inputs that materialized
    /// a `Known` literal against a ground `Type::Quotation` parameter at this
    /// call site (R1's spared case), paired with that position's declared
    /// effect. Lowering materializes the caller's phantom quotation argument
    /// into a real `(code, env)` aggregate at each of these slots, mirroring
    /// the concrete boundary's own `Arity::quot_inputs` -- an abstract
    /// `PolyType::Quotation` position never reaches here (out of scope, L1).
    pub quot_inputs: Vec<(usize, &'static QuotEffect)>,
    /// P7.S3e (R8/R9): the trait-member calls inside the callee's own body
    /// that this instantiation's `θ` resolves, span -> the implementing word's
    /// lowering symbol. A pure function of `(callee, θ)`: the
    /// spans are the callee body's, never the caller's, so two call sites
    /// sharing a `(callee, θ)` record identical maps and the symbol-dedup step
    /// in lowering may read either one.
    pub trait_calls: std::collections::HashMap<Span, String>,
    /// P7.S3k (R4): the calls to *other* generic words inside this callee's
    /// own body, span -> the fully composed `CallInst` of the callee that
    /// call reaches at this θ. A `CallInst` rather than a bare symbol (unlike
    /// `trait_calls`) because lowering the cross-call needs the composed
    /// θ's output shape and bundle, not just a name. Nested copies are
    /// deliberately left empty: they route one `Instr::Call`, they do not
    /// lower their own callee's body -- `Module::transitive_instantiations`
    /// holds the authoritative, `poly_calls`-populated copy for that.
    /// Empty on every monomorphic word and on every instantiation whose body
    /// calls no generic word, so the existing corpus lowers unchanged.
    pub poly_calls: std::collections::HashMap<Span, CallInst>,
    /// P7.S12 (R1.2): the generated struct/enum word call sites inside the
    /// callee's own body that this instantiation's θ resolves, span -> the
    /// concrete `EnumId` that call site constructs or eliminates at this
    /// monomorph. A pure function of `(callee, θ)` over the *callee body's*
    /// spans, mirroring `trait_calls` for the same reason: two call sites
    /// sharing a `(callee, θ)` record identical maps. Empty on every
    /// monomorphic word and on every instantiation whose body reaches no
    /// generated enum word, so the existing corpus lowers unchanged.
    pub enum_words: std::collections::HashMap<Span, EnumId>,
}

/// P7.S3k (R2): what one *callee* type variable was matched to at a
/// generic-to-generic call site. The caller's own variables are still
/// abstract when the mapping is built, so an image is never a ground `Subst`
/// entry: it is either a concrete type the caller supplied outright, or one
/// of the caller's own rigid variables, to be grounded later against the
/// caller's own θ. R6's growth rule is what keeps the set this small -- a
/// compound image mentioning a caller variable (`Box['T]`, `array['T 4]`) is a
/// located rejection at the call site, so no type constructor ever needs
/// representing here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Image {
    Concrete(Type),
    /// An index into the **caller's** `PolySig::ty_var_names`.
    CallerVar(u32),
}

/// P7.S3k (R2): one generic-to-generic call, recorded symbolically as the
/// caller's own body is walked. Deliberately not a `CallInst`: at walk time
/// the caller has no θ of its own, so there is no ground substitution to
/// record and no symbol to mint. Composing `mapping` with a concrete θ of the
/// caller is what grounds it into a real instantiation of `callee`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolyCrossCall {
    pub callee: String,
    pub span: Span,
    /// Callee type-variable id -> its image in the caller's world, in the
    /// order the callee's declared inputs first mention each variable (the
    /// push order `unify_poly_input` uses for a ground `Subst`, so a composed
    /// θ orders its entries the way the concrete path would have).
    pub mapping: Vec<(u32, Image)>,
}

/// R9/R14: the mangled symbol for one instantiation `(word, θ)`. A pure,
/// deterministic function of its inputs with no lowering-order dependence, so
/// the checker's call-site table and the lowered `IrFunc.name` are minted from
/// one source of truth and can never disagree. Mirrors `struct_drop_symbol`'s
/// positional, id-based shape (a word name or a type spelling may hold
/// characters no QBE symbol admits, so both are sanitized here).
pub fn instantiation_symbol(word: &str, subst: &Subst) -> String {
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
    format!("sooth_mono_{}__{}", sanitize(word), parts.join("_"))
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
/// word name may use characters C cannot (`^|>`), and because binding a
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

/// One arm of a tag dispatch: the variant it handles, its body terms, and the
/// span of the call it belongs to. Built by `lower_eliminator` from an
/// eliminator call's variant-tagged quotation operands; it is a lowering
/// vehicle, not surface syntax.
#[derive(Debug)]
pub struct Clause {
    pub variant: String,
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
/// count)` registry plus the leaked `array[T N]` spelling (D2, D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Int(IntType),
    Float(FloatType),
    Struct(StructId, &'static str),
    Enum(EnumId, &'static str),
    /// Phase 6 slice 2 (R1): one variant of an enum, standalone -- the type
    /// Slice 3's eliminator narrows a scrutinee to inside an arm. Carries the
    /// owning `EnumId`, the variant's index into `EnumDecl.variants`, and a
    /// leaked `Enum.Variant` display
    /// name sourced once from `VariantDecl::display_static` (never a
    /// per-site `format!`+`Box::leak`, see `variant_type`), so two
    /// `Type::Variant`s for the same `(EnumId, vi)` are always byte-identical
    /// and compare equal.
    Variant(EnumId, usize, &'static str),
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
    /// P7 slice 3c (R1.1): a borrowed, length-carrying view over a buffer,
    /// `Slice[T]` (shared) or `!Slice[T]` (mutable): a `SliceId` into the
    /// interned `(element, mutable)` registry, the mutability, and the leaked
    /// spelling. Its own variant rather than a fat `Type::Ref`, so every rule a
    /// reference gets for free must be ported deliberately -- and every
    /// representation site that assumed one word (`IrType::Ptr`) fails to
    /// compile instead of silently mis-lowering a two-word value.
    ///
    /// Mutability is carried inline for the same reason `Type::Ref` carries it:
    /// it is the *classification* bit (`is_copy`, linearity), asked at sites
    /// that hold no registry. The element, asked only where an access is being
    /// typed, stays behind the id so `Type` remains `Copy`. Second-class:
    /// non-owning, input-only, and banned from a declared output by
    /// `contains_reference` exactly as a `&T` is.
    Slice(SliceId, bool, &'static str),
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
    /// Slice 10a (R1): the inline-only quotation type `~[ ... ]`. Same payload
    /// as `Type::Quotation`, but it **cannot be materialized**: no runtime
    /// representation, never stored in a field, returned, captured, widened to
    /// an ordinary `[ ... ]`, nor reaching the backend. A `call` on it is
    /// statically always a splice, never a runtime dispatch. Structural
    /// `PartialEq` gives `InlineQuotation(e) != Quotation(e)` for free, so every
    /// materialization boundary rejects a `~` by type inequality *before* the
    /// boundary, and `ir_type_of` never sees one (its arm is `unreachable!`).
    /// Its `name_static` carries the `~[ ... ]` spelling (see
    /// `inline_quotation_type`), so the two variants also render distinctly.
    InlineQuotation(&'static QuotEffect),
    /// P7.S3h: the owning quotation type `owning [ ... ]`. Same payload as
    /// `Type::Quotation`, and it materializes exactly as one does, but it is
    /// **linear**: it carries a disposal obligation for whatever the closure
    /// captured. Two consuming uses discharge it, running different code:
    /// `call` runs the body (which disposes the captures itself), and P7.S3v's
    /// `drop` runs the value's per-construction-site disposer instead,
    /// discarding the closure unexecuted. The type names the obligation and
    /// nothing else -- never where the env lives -- so inline, static and heap
    /// storage can all land behind one signature. Structural `PartialEq` gives
    /// `OwningQuotation(e) != Quotation(e)` for free, so every materialization
    /// boundary and `if`-join separates the two by type inequality. Its
    /// `name_static` carries the `owning [ ... ]` spelling (see
    /// `owning_quotation_type`).
    ///
    /// P7.S3v (R6): legal as a struct field, an enum variant field and an
    /// owned-cell payload -- the three positions whose container synthesizes a
    /// destructor, which reaches the disposer through `emit_drop`. Still
    /// rejected as an array/slice element (P7.S5), behind a reference, and at
    /// an `extern:` boundary: none of those owns what it names.
    OwningQuotation(&'static QuotEffect),
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

/// Slice 10a (R1): build a `Type::InlineQuotation` for a declared `~` effect.
/// Mirrors `quotation_type`, but the leaked spelling is prefixed with `~`, so
/// the effect's `name_static` reads `~[ ... -- ... ]` and the two variants
/// never share a `&'static QuotEffect` (their `name_static` fields differ), on
/// top of already differing by variant tag.
pub fn inline_quotation_type(inputs: Vec<Type>, outputs: Vec<Type>) -> Type {
    let name = format!("~{}", render_quotation_effect(&inputs, &outputs));
    let name_static: &'static str = Box::leak(name.into_boxed_str());
    let eff: &'static QuotEffect = Box::leak(Box::new(QuotEffect {
        inputs,
        outputs,
        name_static,
    }));
    Type::InlineQuotation(eff)
}

/// P7.S3h: build a `Type::OwningQuotation` for a declared `owning` effect.
/// Mirrors `inline_quotation_type`, but the leaked spelling is prefixed with
/// `owning `, so the effect's `name_static` reads `owning [ ... -- ... ]` and
/// no two of the three quotation variants ever share a `&'static QuotEffect`.
pub fn owning_quotation_type(inputs: Vec<Type>, outputs: Vec<Type>) -> Type {
    let name = format!(
        "{OWNING_QUOTATION_KEYWORD} {}",
        render_quotation_effect(&inputs, &outputs)
    );
    let name_static: &'static str = Box::leak(name.into_boxed_str());
    let eff: &'static QuotEffect = Box::leak(Box::new(QuotEffect {
        inputs,
        outputs,
        name_static,
    }));
    Type::OwningQuotation(eff)
}

/// P7.S3h: the one surface spelling of the owning-quotation prefix. Not a
/// lexer delimiter and not a registered type name: it is intercepted by name
/// at every type-position entry, ahead of every user type lookup, and reserved
/// so a `type:` declaration cannot shadow it.
pub const OWNING_QUOTATION_KEYWORD: &str = "owning";

/// Slice 10a (R1): the effect behind either quotation type variant. Returns
/// `Some(eff)` for both `Type::Quotation` and `Type::InlineQuotation`, so every
/// enabling and routing site that must treat a `~` like an ordinary quotation
/// routes through one accessor rather than a second pattern arm a later reader
/// can forget. Two ICE-class defects were traced to exactly that omission, so
/// the accessor is the version that cannot be missed a third time. It is
/// deliberately **not** used at the four materialization boundaries (word
/// output, `&!` store, declared parameter, `if`-join), which reject a `~` by
/// type inequality; it **is** used at the declaration-position rejections and
/// the capture-admission guard, which must actively reject a `~` (they fail
/// open otherwise).
pub fn is_quotation_type(ty: Type) -> Option<&'static QuotEffect> {
    match ty {
        Type::Quotation(eff) | Type::InlineQuotation(eff) | Type::OwningQuotation(eff) => Some(eff),
        _ => None,
    }
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

    /// Slice 10c (R-P3-1): the machine-level condition flag `branch` consumes,
    /// `tag` produces and the comparison primitives yield. A 32-bit unsigned
    /// integer rather than a target-width one, so no conversion sits between a
    /// comparison, a discriminant read and a conditional jump, and so the flag
    /// is the same width on a 32- and a 64-bit target.
    pub const U32: Type = Type::Int(IntType {
        bits: 32,
        signed: false,
    });

    /// Resolve a source type-name word to a `Type`, or `None` if unknown.
    ///
    /// P7 slice 3i: `bool` is deliberately absent. It is `core::bool`'s enum, so
    /// it resolves through the registry like any other declared type -- which is
    /// what makes it require an `import:` -- and a caller needing the boolean
    /// type asks `resolve_bool_type` for it.
    pub fn from_name(name: &str) -> Option<Type> {
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

    /// Whether this type is a reference (`&T` or `&!T`), or the other
    /// borrowed, second-class, non-owning shape: a `Slice[T]` view (R1.4).
    /// Load-bearing in both directions for a slice. It is what makes a slice
    /// *input* legal (`check_reference_free_signature` only rejects a
    /// reference-bearing type that is not itself one), and what keeps a slice
    /// out of move tracking (`is_linear`), so a view expires silently and is
    /// owed no `drop`. A slice *output* stays rejected regardless: that loop
    /// tests `contains_reference` alone.
    pub fn is_ref(&self) -> bool {
        matches!(self, Type::Ref(..) | Type::Slice(..))
    }

    /// Whether a value of this type lives in memory rather than in an SSA
    /// temporary: the four shapes that have an address, and so the four that
    /// can be borrowed or denoted by a second name.
    pub fn is_aggregate(&self) -> bool {
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
            Type::Variant(_, _, name) => name,
            Type::Array(_, name) => name,
            Type::OwnedCell(_, name) => name,
            Type::Ref(_, _, name) => name,
            Type::Slice(_, _, name) => name,
            Type::Usize => "usize",
            Type::Isize => "isize",
            Type::Str => "str",
            Type::Cstr => "cstr",
            Type::Quotation(eff) => eff.name_static,
            // The `~[ ... ]` and `owning [ ... ]` spellings are baked into
            // `name_static` by `inline_quotation_type`/`owning_quotation_type`,
            // so these mirror the `Quotation` arm.
            Type::InlineQuotation(eff) | Type::OwningQuotation(eff) => eff.name_static,
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

/// Phase 6 slice 1 (D4): the optional effect a quotation literal declares
/// inside its own brackets, `[ ( ..a T -- ..b U ) term* ]`. Self-contained:
/// unlike a `PolyType::Quotation` inside a `PolySig`, an annotation has no
/// enclosing signature to borrow an id space from, so its type- and
/// row-variable ids are minted per literal (a `Var(0)` in one literal's
/// annotation is unrelated to a `Var(0)` in another's). A fully concrete
/// annotation leaves both rows `None` and both name tables empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotAnnot {
    pub inputs: Vec<PolyType>,
    pub outputs: Vec<PolyType>,
    pub row_in: Option<u32>,
    pub row_out: Option<u32>,
    pub ty_var_names: Vec<String>,
    pub row_var_names: Vec<String>,
    /// The annotation's opening `(`, where a body/annotation disagreement is
    /// reported.
    pub span: Span,
    /// Phase 6 slice 3b (R1/R2): the routing tag a leading `Variant`/
    /// `&Variant`/`&!Variant` token names. `None` for every plain (non-arm)
    /// annotation.
    pub variant_tag: Option<VariantTag>,
}

/// Phase 6 slice 3b (R2): an eliminator arm's routing tag. The parser
/// recognizes it by name alone -- a bare variant name is not typeable at
/// parse time for a generic enum, whose variants have no concrete
/// `Type::Variant` until an instantiation supplies its arguments -- so the
/// mode the user wrote rides here as data rather than on an interned
/// `Type::Ref`. The checker types the tag against the scrutinee's own enum
/// and the IR reads the mode straight off `mode`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantTag {
    /// The bare variant name, sigil stripped: the spelling both the checker's
    /// arm-to-variant routing and the IR's tag dispatch match against, and
    /// the one a `type:` declaration writes.
    pub name: String,
    pub mode: VariantTagMode,
}

/// The mode an arm's tag was written in: `Variant`, `&Variant`, `&!Variant`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantTagMode {
    Owning,
    Ref,
    RefMut,
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
    /// A word invocation, or a reference to a named local. The `Vec<Type>` is
    /// P7.S3t (R3): an explicit call-site type instantiation, `f[Point]`,
    /// parsed from a bracket glued to the word (R2) and empty for every call
    /// written without one. Only the polymorphic-call route consumes it; every
    /// other route a `Call` can take rejects a non-empty list rather than drop
    /// it (a dropped instantiation would be a wrong-symbol link, not a
    /// diagnostic), which is why the list widens this variant instead of
    /// arriving as a new one every existing arm would silently keep matching
    /// past.
    Call(String, Vec<Type>),
    /// A `| names |` binding (R1): pops one value per name at the point it
    /// appears, leftmost name taking the deepest value. Its extent is the rest
    /// of the enclosing block (R2), so no closing term is needed.
    Bind(Vec<String>),
    /// A `[ ... ]` or `~[ ... ]` quotation literal (R1): an ordered term
    /// list, nested by construction since the element list is parsed with
    /// `parse_terms`. Compile-time-only marker in this slice (D1): never a
    /// runtime value. The `bool` is the literal's own spelling (Slice 12,
    /// R-C1): `true` for a `~[ ... ]` inline-only literal, `false` for an
    /// ordinary `[ ... ]`. Checked against the consuming parameter's declared
    /// flavour at each argument-matching site (R-C2), independent of
    /// `Type::InlineQuotation`/`Type::Quotation`, which describe the
    /// *parameter*. The `Option<QuotAnnot>` is Phase 6 slice 1 (D4): the
    /// effect the literal declares inside its own brackets, `None` for every
    /// unannotated literal.
    Quotation(Vec<Term>, bool, Option<QuotAnnot>),
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
    rename_terms(terms, uid, INLINE_SUFFIX, &mut bound)
}

/// P7.S3s-follow: rename an `inline` trait member's body for a splice that
/// reuses the *enclosing* splice's uid. A member splice cannot mint a fresh
/// `inline_uid` (that would desynchronize lowering's counter from the
/// checker's and make the next splice's `splice_records`/`splice_trait_calls`
/// lookups miss), so its uid is not unique on its own. A separate suffix makes
/// the member body's locals disjoint from the enclosing body's by
/// construction: without it, an enclosing `| x |` and a member `| x |` both
/// rename to `x__inl{uid}` and the name-keyed local lookups in
/// `func_builder` resolve a member read to the enclosing value, silently
/// producing a wrong result.
pub fn alpha_rename_member_locals(terms: &[Term], uid: u32) -> Vec<Term> {
    let mut bound: Vec<String> = Vec::new();
    rename_terms(terms, uid, MEMBER_SPLICE_SUFFIX, &mut bound)
}

fn rename_local(name: &str, uid: u32, suffix: &str) -> String {
    format!("{name}{suffix}{uid}")
}

/// The private separator `alpha_rename_locals` appends to an inlined local's
/// source name. A renamed local never reaches a user diagnostic: a combinator
/// body is checked standalone at its definition (R17), so any error about its
/// own locals surfaces there with the source spelling and aborts compilation
/// before any splice can rename them; the renamed spelling exists only for
/// collision-free lookup during the splice and its lowering.
const INLINE_SUFFIX: &str = "__inl";

/// `alpha_rename_member_locals`'s separator. Disjoint from `INLINE_SUFFIX`
/// because a member splice and an inline splice genuinely share one uid: a
/// member body is spliced under the member word's own check-time seed
/// (`word_idx * crate::check::INLINE_UID_STRIDE`), and that seed is also the
/// uid the *first* combinator splice nested inside that body mints, so both
/// renames would land on the same `{name}{suffix}{uid}`. `FuncBuilder`'s
/// `locals` is scanned front-to-back, so the nested body's read of its own
/// local would find the member body's instead: a silent wrong answer, not a
/// panic. Witnessed by
/// `ord_inline_cmp_member_local_colliding_with_a_nested_splices_local_reads_its_own`.
const MEMBER_SPLICE_SUFFIX: &str = "__mem";

/// Rename a `Call` naming a body-bound local. A borrow reads its local through
/// a `&`/`&!` sigil (`&arr`, `&!arr`), so the sigil is split off, the local
/// part renamed if bound, and the sigil re-attached; a `Call` that is not a
/// bound local (a word, `&>`, a cast) is returned unchanged.
fn rename_call(name: &str, uid: u32, suffix: &str, bound: &[String]) -> String {
    let is_bound = |n: &str| bound.iter().any(|b| b == n);
    if let Some(inner) = name.strip_prefix("&!") {
        if is_bound(inner) {
            return format!("&!{}", rename_local(inner, uid, suffix));
        }
    } else if let Some(inner) = name.strip_prefix('&') {
        if is_bound(inner) {
            return format!("&{}", rename_local(inner, uid, suffix));
        }
    } else if is_bound(name) {
        return rename_local(name, uid, suffix);
    }
    name.to_string()
}

fn rename_terms(terms: &[Term], uid: u32, suffix: &str, bound: &mut Vec<String>) -> Vec<Term> {
    let start = bound.len();
    let mut out = Vec::with_capacity(terms.len());
    for term in terms {
        let kind = match &term.kind {
            TermKind::Bind(names) => {
                let renamed = names
                    .iter()
                    .map(|n| {
                        bound.push(n.clone());
                        rename_local(n, uid, suffix)
                    })
                    .collect();
                TermKind::Bind(renamed)
            }
            TermKind::Call(name, type_args) => {
                TermKind::Call(rename_call(name, uid, suffix, bound), type_args.clone())
            }
            TermKind::Quotation(inner, is_inline, annot) => {
                let mut inner_bound = bound.clone();
                TermKind::Quotation(
                    rename_terms(inner, uid, suffix, &mut inner_bound),
                    *is_inline,
                    annot.clone(),
                )
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

    /// P8 S2 (R2): the gate set is `is_builtin_word_name` minus exactly the six
    /// surface comparisons -- no wider and no narrower. `.` is the case that
    /// makes the difference load-bearing: both r1 reviews put it in the
    /// exclusion set, but it is a real `BUILTIN_TABLE` intrinsic that does not
    /// move to `core`, so gating it is correct and excluding it would let a bare
    /// `.` through with no import at all.
    #[test]
    fn the_gate_set_excludes_exactly_the_six_surface_comparisons() {
        for name in ["eq", "lt", "gt", "lte", "gte", "ne"] {
            assert!(
                is_builtin_word_name(name),
                "`{name}` stays in BUILTIN_WORDS for `has_self_tail_call`"
            );
            assert!(
                !is_name_dispatched_builtin(name),
                "`{name}` is a `core` word"
            );
        }
        for name in BUILTIN_WORDS
            .iter()
            .copied()
            .filter(|n| !matches!(*n, "eq" | "lt" | "gt" | "lte" | "gte" | "ne"))
            .chain([">u8", ">usize"])
        {
            assert!(
                is_name_dispatched_builtin(name),
                "`{name}` is name-dispatched"
            );
        }
        assert!(is_name_dispatched_builtin("."), "`.` is a real intrinsic");
        // Quotation application is its own arm in `check_term`/`poly_call_term`,
        // not a table entry, so this predicate does not cover it and must not be
        // widened to: it is also the set the `intrinsics` import gates (P8 S2 R2),
        // and bare `call` is not import-gated. `parse_trait`'s member-name
        // rejection therefore tests `call` separately (P7.S3p).
        assert!(
            !is_name_dispatched_builtin("call"),
            "`call` is not in the table"
        );
    }

    const EMPTY_REGS: NameRegistries<'static> = NameRegistries {
        structs: &[],
        enums: &[],
        arrays: &[],
        cells: &[],
        refs: &[],
    };

    /// The mutable twin of `EMPTY_REGS`, owning the three interning vecs a
    /// `MutRegistries` borrows -- it holds `&mut`, so it cannot be a `const`
    /// the way the read-only view can. Kept live across a test's calls so an
    /// interned shape from one instantiation is visible to the next.
    #[derive(Default)]
    struct ScratchRegs {
        arrays: Vec<ArrayDecl>,
        cells: Vec<OwnedCellDecl>,
        refs: Vec<RefDecl>,
    }

    impl ScratchRegs {
        fn regs(&mut self) -> MutRegistries<'_> {
            MutRegistries {
                structs: &[],
                enums: &[],
                arrays: &mut self.arrays,
                cells: &mut self.cells,
                refs: &mut self.refs,
            }
        }
    }

    /// Slice 10a (R1/R10): a `~` renders `~[ ... -- ... ]`, the ordinary
    /// quotation renders `[ ... -- ... ]`, distinguished only by the sigil.
    #[test]
    fn inline_quotation_type_name_carries_the_tilde() {
        let ord = quotation_type(vec![Type::I64], vec![Type::I64]);
        let inl = inline_quotation_type(vec![Type::I64], vec![Type::I64]);
        assert_eq!(ord.name(), "[ i64 -- i64 ]");
        assert_eq!(inl.name(), "~[ i64 -- i64 ]");
    }

    /// Slice 10a (R1): the accessor sees through both quotation variants and
    /// nothing else, so every enabling/routing site routes through one place.
    /// P7.S3h: three variants now. The `owning` row is load-bearing twice
    /// over: `call` reaches `check_abstract_quotation_call` through this
    /// accessor, and the registry audit dispatches on it. P7.S3v (R6)
    /// narrowed that second half -- the owning flavour is now admitted as a
    /// struct field, an enum variant field and an owned-cell payload (each its
    /// own carve-out), and rejected through this accessor only as an array
    /// element and a reference referent.
    #[test]
    fn is_quotation_type_accepts_all_three_variants_only() {
        let ord = quotation_type(vec![Type::I64], Vec::new());
        let inl = inline_quotation_type(vec![Type::I64], Vec::new());
        let own = owning_quotation_type(vec![Type::I64], Vec::new());
        assert!(is_quotation_type(ord).is_some());
        assert!(is_quotation_type(inl).is_some());
        assert!(is_quotation_type(own).is_some());
        assert!(is_quotation_type(Type::I64).is_none());
        assert!(is_quotation_type(Type::Str).is_none());
    }

    /// P7.S3h: `owning [ ... ]` renders with its keyword, and is structurally
    /// unequal to both the plain and the `~` quotation of the same rows -- the
    /// property every materialization boundary and `if`-join relies on to keep
    /// the two flavours apart with no code of its own.
    #[test]
    fn owning_quotation_type_renders_and_never_equals_its_siblings() {
        let ord = quotation_type(vec![Type::I64], vec![Type::I64]);
        let inl = inline_quotation_type(vec![Type::I64], vec![Type::I64]);
        let own = owning_quotation_type(vec![Type::I64], vec![Type::I64]);
        assert_eq!(own.name(), "owning [ i64 -- i64 ]");
        assert_ne!(own, ord);
        assert_ne!(own, inl);
        // ...and it equals itself, so the inequality is the variant tag, not a
        // per-call fresh leak of the payload.
        assert_eq!(own, owning_quotation_type(vec![Type::I64], vec![Type::I64]));
    }

    /// Slice 10a (R3): structural `PartialEq` makes a `~` and an ordinary
    /// quotation of the *same rows* unequal, so no equality site coerces
    /// between them -- the enforcement is free from the variant tag.
    #[test]
    fn inline_and_ordinary_quotation_are_never_equal() {
        let ord = quotation_type(vec![Type::I64], vec![Type::I64]);
        let inl = inline_quotation_type(vec![Type::I64], vec![Type::I64]);
        assert_ne!(ord, inl);
        // ...and each equals itself, so the inequality is the variant, not a
        // per-call fresh leak of the payload.
        assert_eq!(inl, inline_quotation_type(vec![Type::I64], vec![Type::I64]));
    }

    /// U8 (slice 5b, R8d): `find_type_in_module` matches on `name_static` with
    /// module gating. A single-module lookup is unaffected, and two decls that
    /// share a `name_static` but sit in different modules disambiguate by their
    /// module id (`resolve::mangle`'s module-suffixed `.name` plays no part in
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
        // and their `.name` fields are tagged apart the way `resolve::mangle`
        // suffixes an imported type's name with its module id.
        let structs = vec![mk("Point", "Point", 0), mk("Point__m1", "Point", 1)];
        let enums: Vec<EnumDecl> = Vec::new();

        // Module 0's lookup finds index 0 (a single-module lookup is exactly
        // this, and is unaffected: `name_static` equals `name` there).
        match find_type_in_module(&structs, &enums, "Point", 0) {
            Some(Type::Struct(id, _)) => assert_eq!(id.index(), 0),
            other => panic!("expected module 0's Point at index 0, got {other:?}"),
        }
        // Module 1's lookup finds index 1, disambiguated purely by module id
        // even though the tagged `.name` ("Point__m1") never matches
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
        // P7 slice 3i (R1): `bool` is deliberately *not* a scalar-table name.
        // It is `core::bool`'s enum, so it resolves through the registry (and
        // therefore only through an `import:`), which is what
        // `resolve_bool_type` reads and what makes an unimported `bool` a
        // located `unknown type`.
        assert_eq!(Type::from_name("Bool"), None);
    }

    /// A `bool`-shaped registry: the declaration `lib/bool.sth` holds, built
    /// here so the pure-`ast` tests below need no parse.
    fn bool_registry() -> Vec<EnumDecl> {
        let variant = |name: &'static str| VariantDecl {
            name: name.to_string(),
            name_static: name,
            display_static: "Bool.V",
            fields: Vec::new(),
            span: Span::default(),
        };
        vec![EnumDecl {
            name: BOOL_TYPE_NAME.to_string(),
            name_static: BOOL_TYPE_NAME,
            variants: vec![variant("False"), variant("True")],
            span: Span::default(),
            module: 0,
        }]
    }

    #[test]
    fn resolve_bool_type_finds_the_declared_enum_at_its_own_position() {
        // R4: no slot is reserved, so the answer is wherever the declaration
        // landed -- here behind a user enum that was declared first.
        let mut enums = vec![EnumDecl {
            name: "Shape".to_string(),
            name_static: "Shape",
            variants: Vec::new(),
            span: Span::default(),
            module: 0,
        }];
        enums.extend(bool_registry());
        assert_eq!(
            resolve_bool_type(&enums),
            Some(Type::Enum(EnumId(1), BOOL_TYPE_NAME))
        );
        assert_eq!(resolve_bool_type(&[]), None);
    }

    #[test]
    fn resolve_bool_type_rejects_a_same_named_enum_carrying_a_payload() {
        // R4: the callers that resolve this treat the answer as a
        // register-resident scalar (the logical operators, the `extern:`
        // boundary set), so a payload-carrying enum that merely shares the
        // name is not it.
        let mut enums = bool_registry();
        enums[0].variants[1].fields = vec![("n".to_string(), Type::I64)];
        assert_eq!(resolve_bool_type(&enums), None);
    }

    #[test]
    fn resolve_bool_type_rejects_a_same_named_enum_with_a_third_variant() {
        // A third payload-free variant is still an `and`/`or`/`xor`/`not`
        // hazard: those lower on the assumption of exactly two discriminants
        // (`xor 1` for `not`), so a same-named 3-variant enum must not be
        // mistaken for the logical bool.
        let mut enums = bool_registry();
        enums[0].variants.push(VariantDecl {
            name: "Maybe".to_string(),
            name_static: "Maybe",
            display_static: "Bool.V",
            fields: Vec::new(),
            span: Span::default(),
        });
        assert_eq!(resolve_bool_type(&enums), None);
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
            "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "usize", "isize",
        ];
        for name in names {
            let ty = Type::from_name(name).unwrap();
            assert_eq!(ty.name(), name);
            assert_eq!(ty.to_string(), name);
        }
        // `bool` is not a scalar-table name (P7 slice 3i), but it renders and
        // round-trips through the same two methods.
        let bool_ty = resolve_bool_type(&bool_registry()).unwrap();
        assert_eq!(bool_ty.name(), BOOL_TYPE_NAME);
        assert_eq!(bool_ty.to_string(), BOOL_TYPE_NAME);
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
        assert_ne!(Type::Usize, resolve_bool_type(&bool_registry()).unwrap());
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
        assert_ne!(Type::Isize, resolve_bool_type(&bool_registry()).unwrap());
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
        assert!(!resolve_bool_type(&bool_registry()).unwrap().is_numeric());
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
            slices: Vec::new(),
            generic_structs: Vec::new(),
            generic_enums: Vec::new(),
            generics: GenericTypes::default(),
            externs: Vec::new(),
            instantiations: std::collections::HashMap::new(),
            poly_cross_calls: std::collections::HashMap::new(),
            transitive_instantiations: Vec::new(),
            splice_records: std::collections::HashMap::new(),
            splice_trait_calls: std::collections::HashMap::new(),
            builtin_overloads: std::collections::HashMap::new(),
            resolved_fields: std::collections::HashMap::new(),
            resolved_variant_fields: std::collections::HashMap::new(),
            modules: Vec::new(),
            statics: Vec::new(),
            traits: Vec::new(),
            impls: Vec::new(),
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

        let no_origin: Vec<std::collections::HashMap<String, u32>> = Vec::new();
        let bare = resolve_type_name_in_module(
            &structs,
            &[],
            "Foo",
            0,
            &imports,
            &no_selective,
            &no_origin,
        )
        .unwrap();
        assert_eq!(bare, Type::Struct(StructId(0), "Foo"), "own module first");
        let qualified = resolve_type_name_in_module(
            &structs,
            &[],
            "lib::Foo",
            0,
            &imports,
            &no_selective,
            &no_origin,
        )
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
            &no_selective,
            &no_origin,
        )
        .is_none());
        // R15c: a bare name absent from the own module resolves against a
        // module it is selectively imported from.
        let mut selective = std::collections::HashMap::new();
        selective.insert("Foo".to_string(), 1u32);
        let via_selective = resolve_type_name_in_module(
            &[mk("Foo", 1)],
            &[],
            "Foo",
            0,
            &imports,
            &selective,
            &no_origin,
        )
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
            slices: Vec::new(),
            generic_structs: Vec::new(),
            generic_enums: Vec::new(),
            generics: GenericTypes::default(),
            externs: Vec::new(),
            instantiations: std::collections::HashMap::new(),
            poly_cross_calls: std::collections::HashMap::new(),
            transitive_instantiations: Vec::new(),
            splice_records: std::collections::HashMap::new(),
            splice_trait_calls: std::collections::HashMap::new(),
            builtin_overloads: std::collections::HashMap::new(),
            resolved_fields: std::collections::HashMap::new(),
            resolved_variant_fields: std::collections::HashMap::new(),
            modules: Vec::new(),
            statics: Vec::new(),
            traits: Vec::new(),
            impls: Vec::new(),
        }
    }

    fn variant(name: &str, fields: Vec<(String, Type)>) -> VariantDecl {
        let name_static: &'static str = Box::leak(name.to_string().into_boxed_str());
        VariantDecl {
            name: name.to_string(),
            name_static,
            // Placeholder: the caller (`module_with_enum`) holds the owning
            // enum's name, not this builder, per R1's out-of-scope test sites.
            display_static: name_static,
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
            slices: Vec::new(),
            generic_structs: Vec::new(),
            generic_enums: Vec::new(),
            generics: GenericTypes::default(),
            externs: Vec::new(),
            instantiations: std::collections::HashMap::new(),
            poly_cross_calls: std::collections::HashMap::new(),
            transitive_instantiations: Vec::new(),
            splice_records: std::collections::HashMap::new(),
            splice_trait_calls: std::collections::HashMap::new(),
            builtin_overloads: std::collections::HashMap::new(),
            resolved_fields: std::collections::HashMap::new(),
            resolved_variant_fields: std::collections::HashMap::new(),
            modules: Vec::new(),
            statics: Vec::new(),
            traits: Vec::new(),
            impls: Vec::new(),
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
    fn intern_array_type_renders_named_array() {
        // R8/R9: `intern_array_type` mints `array[i64 4]`, the new spelling
        // that every diagnostic and pretty-printer picks up through
        // `name_static`.
        let mut arrays = Vec::new();
        let a = intern_array_type(&mut arrays, Type::I64, 4);
        assert_eq!(a.name(), "array[i64 4]");
        assert_eq!(a.to_string(), "array[i64 4]");
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
                assert_eq!(name, "array[i64 4]");
            }
            other => panic!("expected Type::Array, got {other:?}"),
        }
        assert_eq!(a.to_string(), "array[i64 4]");
    }

    /// P7 slice 3c (R1.1/R1.3): the two spellings, and that `Type::name`
    /// renders the element rather than an opaque tag.
    #[test]
    fn slice_type_name_renders_element() {
        let mut slices = Vec::new();
        let shared = intern_slice_type(&mut slices, Type::I64, false);
        let mutable = intern_slice_type(&mut slices, Type::I64, true);
        assert_eq!(shared.name(), "Slice[i64]");
        assert_eq!(mutable.name(), "!Slice[i64]");
        assert_eq!(shared.to_string(), "Slice[i64]");
    }

    /// P7 slice 3c (R1.4): the arm that makes a slice *input* legal at
    /// declaration and keeps a slice out of move tracking. Delete it and the
    /// slice's whole consumer shape is rejected by
    /// `check_reference_free_signature`.
    #[test]
    fn is_ref_true_for_slice() {
        let mut slices = Vec::new();
        let shared = intern_slice_type(&mut slices, Type::I64, false);
        let mutable = intern_slice_type(&mut slices, Type::I64, true);
        assert!(shared.is_ref());
        assert!(mutable.is_ref());
        // ...and it is not an aggregate: it has no address of its own to
        // borrow, so `&s` never forms a reference to a view.
        assert!(!shared.is_aggregate());
    }

    /// P7 slice 3c (R1.2): the registry key is `(element, mutable)`, like
    /// `intern_ref_type` and unlike `intern_array_type` -- a shared and a
    /// mutable view of one element are distinct types, and a repeated spelling
    /// of either dedups to one entry.
    #[test]
    fn slice_interns_by_element_and_mutability() {
        let mut slices = Vec::new();
        let a = intern_slice_type(&mut slices, Type::I64, false);
        let b = intern_slice_type(&mut slices, Type::I64, false);
        let m = intern_slice_type(&mut slices, Type::I64, true);
        let other = intern_slice_type(&mut slices, Type::F64, false);
        assert_eq!(a, b);
        assert_ne!(a, m);
        assert_ne!(a, other);
        assert_eq!(slices.len(), 3);
        match a {
            Type::Slice(id, mutable, name) => {
                assert_eq!(id, SliceId(0));
                assert!(!mutable);
                assert_eq!(name, "Slice[i64]");
            }
            other => panic!("expected Type::Slice, got {other:?}"),
        }
        assert_eq!(slices[2].element, Type::F64);
    }

    #[test]
    fn intern_bundle_struct_same_tuple_dedups_expected() {
        let mut structs = Vec::new();
        let a = intern_bundle_struct(&mut structs, &[Type::I64, Type::U32]);
        let b = intern_bundle_struct(&mut structs, &[Type::I64, Type::U32]);
        assert_eq!(a, b);
        assert_eq!(structs.len(), 1);
        assert!(structs[0].is_bundle);
        assert_eq!(
            structs[0].fields,
            vec![("f0".to_string(), Type::I64), ("f1".to_string(), Type::U32)]
        );
    }

    /// Phase 5 slice 1 (R4/D5): a minted instantiation's `StructId` counts
    /// from the concrete registry's post-pre-pass length, so the ids the
    /// parser hands out stay valid once the instantiations are appended onto
    /// that registry. With a base of `0` this arithmetic is invisible, which
    /// is exactly how an off-by-a-base bug would hide.
    #[test]
    fn instantiate_struct_dedups_and_counts_from_its_base() {
        let decl = GenericStructDecl {
            name: "Box".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec![],
            fields: vec![("val".to_string(), PolyType::Var(0))],
            span: Span::default(),
            module: 0,
        };
        let mut generics = GenericTypes::with_bases(3, 1);
        generics.structs.push(decl);
        let mut scratch = ScratchRegs::default();
        let a = generics.instantiate_struct(0, &[Type::I64], &[], 0, scratch.regs());
        let b = generics.instantiate_struct(0, &[Type::I64], &[], 0, scratch.regs());
        let c = generics.instantiate_struct(0, &[Type::U32], &[], 0, scratch.regs());
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(generics.inst_structs.len(), 2);
        assert_eq!(a, Type::Struct(StructId::from_index(3), "Box[i64]"));
        assert_eq!(c, Type::Struct(StructId::from_index(4), "Box[u32]"));
        assert_eq!(
            generics.inst_structs[0].fields,
            vec![("val".to_string(), Type::I64)]
        );
    }

    /// A one-type-variable, one-length-variable generic struct header with a
    /// `data array['T 'N]` field -- the `Buffer` fixture R5's distinct-
    /// monomorph tests instantiate.
    fn buffer_header() -> GenericStructDecl {
        GenericStructDecl {
            name: "Buffer".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec!["'N".to_string()],
            fields: vec![(
                "data".to_string(),
                PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0)),
            )],
            span: Span::default(),
            module: 0,
        }
    }

    /// P7.S6a (R5): `Buffer[u8 256]` and `Buffer[u8 512]` mint distinct
    /// monomorphs -- the collision `struct_keys`' old `Vec<Type>`-only key
    /// would silently produce, since the type argument alone (`u8`) is
    /// identical between the two.
    #[test]
    fn instantiate_struct_distinct_lengths_mint_distinct_monomorphs() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let mut scratch = ScratchRegs::default();
        let a =
            generics.instantiate_struct(0, &[Type::U32], &[Len::Concrete(256)], 0, scratch.regs());
        let b =
            generics.instantiate_struct(0, &[Type::U32], &[Len::Concrete(512)], 0, scratch.regs());
        assert_ne!(a, b, "distinct lengths must mint distinct StructIds");
        assert_eq!(generics.inst_structs.len(), 2);
        assert_eq!(generics.struct_keys.len(), 2);
    }

    /// The dedup floor R5's widening must not break: two applications at the
    /// *same* length still hit one monomorph.
    #[test]
    fn instantiate_struct_same_length_dedups() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let mut scratch = ScratchRegs::default();
        let a =
            generics.instantiate_struct(0, &[Type::U32], &[Len::Concrete(256)], 0, scratch.regs());
        let b =
            generics.instantiate_struct(0, &[Type::U32], &[Len::Concrete(256)], 0, scratch.regs());
        assert_eq!(a, b);
        assert_eq!(generics.inst_structs.len(), 1);
    }

    /// P7.S6a (R5): the mangled symbol carries the length, and two distinct
    /// lengths render distinct names -- the "differs", not just "contains",
    /// clause a dropped-length-in-the-renderer-only mutation needs to fail
    /// against (mutation 2's own dedup key can stay fixed while this still
    /// catches a renderer-only regression).
    #[test]
    fn type_instantiation_name_renders_length_args() {
        let arrays = Vec::new();
        let enums = Vec::new();
        let structs = Vec::new();
        let cells = Vec::new();
        let refs = Vec::new();
        let regs = NameRegistries {
            structs: &structs,
            enums: &enums,
            arrays: &arrays,
            cells: &cells,
            refs: &refs,
        };
        let name_256 = type_instantiation_name("Buffer", &[Type::U32], &[Len::Concrete(256)], regs);
        let name_512 = type_instantiation_name("Buffer", &[Type::U32], &[Len::Concrete(512)], regs);
        assert!(name_256.contains("256"));
        assert_ne!(name_256, name_512);
    }

    /// P7.S11-follow: the struct twin of `enum_decl` -- a minted-but-unflushed
    /// `StructId` reads back the pending decl.
    #[test]
    fn generic_types_struct_decl_reads_an_unflushed_mint() {
        let decl = GenericStructDecl {
            name: "Box".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec![],
            fields: vec![("val".to_string(), PolyType::Var(0))],
            span: Span::default(),
            module: 0,
        };
        let mut generics = GenericTypes::with_bases(3, 1);
        generics.structs.push(decl);
        let mut scratch = ScratchRegs::default();
        let a = generics.instantiate_struct(0, &[Type::I64], &[], 0, scratch.regs());
        let Type::Struct(id, _) = a else {
            panic!("expected a Type::Struct")
        };
        let found = generics
            .struct_decl(id)
            .expect("a minted-but-unflushed id must resolve");
        assert_eq!(found.fields, vec![("val".to_string(), Type::I64)]);
    }

    /// A flushed id (or a hand-written concrete struct) is out of this
    /// batch's pending range, so `struct_decl` returns `None` and the caller
    /// falls back to indexing the live `structs` slice.
    #[test]
    fn generic_types_struct_decl_none_for_a_flushed_id() {
        let decl = GenericStructDecl {
            name: "Box".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec![],
            fields: vec![("val".to_string(), PolyType::Var(0))],
            span: Span::default(),
            module: 0,
        };
        let mut structs: Vec<StructDecl> = Vec::new();
        let mut generics = GenericTypes::with_bases(structs.len(), 0);
        generics.structs.push(decl);
        let mut scratch = ScratchRegs::default();
        let a = generics.instantiate_struct(0, &[Type::I64], &[], 0, scratch.regs());
        generics.flush_structs_into(&mut structs);
        generics.rebase(structs.len(), 0);
        let Type::Struct(id, _) = a else {
            panic!("expected a Type::Struct")
        };
        assert!(generics.struct_decl(id).is_none());
    }

    /// A one-variable generic struct header with a single field of the given
    /// shape, the fixture every R4 substitution test below instantiates.
    fn header_with_field(name: &'static str, field: PolyType) -> GenericStructDecl {
        GenericStructDecl {
            name: name.to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec![],
            fields: vec![("f".to_string(), field)],
            span: Span::default(),
            module: 0,
        }
    }

    /// P7.S3n (R4): `items array['T 2]` at `'T = i64` grounds to the interned
    /// `array[i64 2]` shape -- the array arm, which panicked in `unreachable!`
    /// before phase 2.
    #[test]
    fn substitute_generic_field_array_of_ty_var_interns_concrete_array() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(header_with_field(
            "Pair",
            PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(2)),
        ));
        let mut scratch = ScratchRegs::default();
        generics.instantiate_struct(0, &[Type::I64], &[], 0, scratch.regs());
        let (_, ty) = &generics.inst_structs[0].fields[0];
        let Type::Array(id, _) = ty else {
            panic!("an array field grounds to Type::Array: {ty:?}")
        };
        assert_eq!(scratch.arrays[id.index()].element, Type::I64);
        assert_eq!(scratch.arrays[id.index()].count, 2);
    }

    /// P7.S6a (R4): `data array['T 'N]` at `'N = 3` grounds the field's own
    /// `Len::Var` to the instantiation's length-argument list -- the arm
    /// that used to be `unreachable!()` before this slice (N3's own doc,
    /// now stale).
    #[test]
    fn substitute_generic_field_array_of_len_var_interns_concrete_count() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(header_with_field(
            "Buffer",
            PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0)),
        ));
        let mut scratch = ScratchRegs::default();
        generics.instantiate_struct(0, &[Type::U32], &[Len::Concrete(3)], 0, scratch.regs());
        let (_, ty) = &generics.inst_structs[0].fields[0];
        let Type::Array(id, _) = ty else {
            panic!("a length-variable array field grounds to Type::Array: {ty:?}")
        };
        assert_eq!(scratch.arrays[id.index()].element, Type::U32);
        assert_eq!(scratch.arrays[id.index()].count, 3);
    }

    /// P7.S6a (R4): a nested `PolyType::Generic` field (the shape
    /// `parse_generic_field_application` produces) whose own `len_args`
    /// contains a `Len::Var` grounds correctly when the *outer* header is
    /// instantiated -- the `Generic` arm's own length forwarding, not just
    /// the `Array` arm's.
    #[test]
    fn substitute_generic_field_nested_generic_forwards_its_own_len_args() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(GenericStructDecl {
            name: "Inner".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec!["'N".to_string()],
            fields: vec![(
                "data".to_string(),
                PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0)),
            )],
            span: Span::default(),
            module: 0,
        });
        generics.structs.push(GenericStructDecl {
            name: "Outer".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec!["'N".to_string()],
            fields: vec![(
                "inner".to_string(),
                PolyType::Generic {
                    is_enum: false,
                    idx: 0,
                    module: 0,
                    args: vec![PolyType::Var(0)],
                    len_args: vec![Len::Var(0)],
                    name: "Inner",
                },
            )],
            span: Span::default(),
            module: 0,
        });
        let mut scratch = ScratchRegs::default();
        generics.instantiate_struct(1, &[Type::U32], &[Len::Concrete(5)], 0, scratch.regs());
        let (_, outer_field) = &generics.inst_structs[0].fields[0];
        let Type::Struct(inner_id, _) = outer_field else {
            panic!("the nested generic field grounds to Type::Struct: {outer_field:?}")
        };
        let inner_decl = &generics.inst_structs[inner_id.index()];
        let (_, inner_field) = &inner_decl.fields[0];
        let Type::Array(array_id, _) = inner_field else {
            panic!("the inner header's own field grounds to Type::Array: {inner_field:?}")
        };
        assert_eq!(scratch.arrays[array_id.index()].count, 5);
    }

    /// The nesting claim, which the single-level test cannot make: the arm has
    /// to recurse, not merely look one level down.
    #[test]
    fn substitute_generic_field_nested_array_of_ty_var_interns_nested_array() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(header_with_field(
            "NestArr",
            PolyType::Array(
                Box::new(PolyType::Array(
                    Box::new(PolyType::Var(0)),
                    Len::Concrete(2),
                )),
                Len::Concrete(3),
            ),
        ));
        let mut scratch = ScratchRegs::default();
        generics.instantiate_struct(0, &[Type::U32], &[], 0, scratch.regs());
        let (_, ty) = &generics.inst_structs[0].fields[0];
        let Type::Array(outer, _) = ty else {
            panic!("expected Type::Array: {ty:?}")
        };
        let outer = &scratch.arrays[outer.index()];
        assert_eq!(outer.count, 3);
        let Type::Array(inner, _) = outer.element else {
            panic!("the outer element is itself an array: {:?}", outer.element)
        };
        assert_eq!(scratch.arrays[inner.index()].element, Type::U32);
        assert_eq!(scratch.arrays[inner.index()].count, 2);
    }

    /// R4/R3: `c ^'T` grounds through `intern_owned_cell_type`. The cell arm
    /// is what makes a self-referential generic type possible at all, so its
    /// substitution is load-bearing rather than one shape among five.
    #[test]
    fn substitute_generic_field_owned_cell_of_ty_var_interns_concrete_cell() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(header_with_field(
            "Cell",
            PolyType::OwnedCell(Box::new(PolyType::Var(0))),
        ));
        let mut scratch = ScratchRegs::default();
        generics.instantiate_struct(0, &[Type::F64], &[], 0, scratch.regs());
        let (_, ty) = &generics.inst_structs[0].fields[0];
        let Type::OwnedCell(id, _) = ty else {
            panic!("a `^` field grounds to Type::OwnedCell: {ty:?}")
        };
        assert_eq!(scratch.cells[id.index()].payload, Type::F64);
    }

    /// R4/R10: `r &'T` grounds to an interned `Type::Ref`. The *declaration*
    /// is then rejected by the no-stored-reference rule downstream (a build
    /// test pins that), which it can only reach by substituting to a real
    /// reference type first -- hence the arm, for a shape that never builds.
    #[test]
    fn substitute_generic_field_ref_of_ty_var_interns_concrete_ref() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(header_with_field(
            "Box",
            PolyType::Ref(Box::new(PolyType::Var(0)), false),
        ));
        let mut scratch = ScratchRegs::default();
        generics.instantiate_struct(0, &[Type::I64], &[], 0, scratch.regs());
        let (_, ty) = &generics.inst_structs[0].fields[0];
        let Type::Ref(id, mutable, _) = ty else {
            panic!("a `&` field grounds to Type::Ref: {ty:?}")
        };
        assert!(!mutable);
        assert_eq!(scratch.refs[id.index()].referent, Type::I64);
    }

    /// R4: the enum twin, which shares the arms but not the path -- a
    /// variant's fields go through `substituted_enum_variants`, and a
    /// substitution correct for structs and skipped for variants would pass
    /// every test above.
    #[test]
    fn substitute_generic_variant_field_array_of_ty_var_interns_concrete_array() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.enums.push(GenericEnumDecl {
            name: "Holder".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec![],
            variants: vec![GenericVariantDecl {
                name: "Some".to_string(),
                fields: vec![(
                    "xs".to_string(),
                    PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(2)),
                )],
                span: Span::default(),
            }],
            span: Span::default(),
            module: 0,
        });
        let mut scratch = ScratchRegs::default();
        generics.instantiate_enum(0, &[Type::I64], &[], 0, scratch.regs());
        let (_, ty) = &generics.inst_enums[0].variants[0].fields[0];
        let Type::Array(id, _) = ty else {
            panic!("a variant's array field grounds to Type::Array: {ty:?}")
        };
        assert_eq!(scratch.arrays[id.index()].element, Type::I64);
    }

    /// R6, asserted directly rather than only through the hang it prevents:
    /// `type: L['T] v 'T next ^L['T] ;` at `'T = i64` re-enters `instantiate_struct`
    /// for the *same* `(idx, module, args)` while substituting its own field.
    /// The memo key, the resolved type and the placeholder decl are pushed
    /// before that substitution runs, so the re-entry hits the dedup lookup
    /// and reads back the very id this call minted. Restore the old
    /// substitute-then-mint order and this recurses until the stack dies.
    #[test]
    fn instantiate_struct_pushes_memo_key_before_substituting_fields() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(header_with_field(
            "L",
            PolyType::OwnedCell(Box::new(PolyType::Generic {
                is_enum: false,
                idx: 0,
                module: 0,
                args: vec![PolyType::Var(0)],
                len_args: vec![],
                name: "L",
            })),
        ));
        let mut scratch = ScratchRegs::default();
        let ty = generics.instantiate_struct(0, &[Type::I64], &[], 0, scratch.regs());
        assert_eq!(
            generics.inst_structs.len(),
            1,
            "the self-reference must memo-hit, not mint a second instantiation"
        );
        let (_, field) = &generics.inst_structs[0].fields[0];
        let Type::OwnedCell(id, _) = field else {
            panic!("the field is a cell: {field:?}")
        };
        assert_eq!(
            scratch.cells[id.index()].payload,
            ty,
            "the cell payload is the id this very call minted"
        );
    }

    /// R8's permuting case, at the layer that has to terminate: `A['V 'K]`
    /// swaps its arguments each hop, so the closure is two instantiations,
    /// reached in either order, and the memo is what closes the cycle. Two
    /// entries rather than one -- a memo keyed on the header alone (ignoring
    /// `args`) would collapse them and give `A[i64 str]` `A[str i64]`'s
    /// layout.
    #[test]
    fn instantiate_struct_permuting_self_reference_terminates_at_two_entries() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(GenericStructDecl {
            name: "A".to_string(),
            ty_var_names: vec!["'K".to_string(), "'V".to_string()],
            len_var_names: vec![],
            fields: vec![(
                "next".to_string(),
                PolyType::OwnedCell(Box::new(PolyType::Generic {
                    is_enum: false,
                    idx: 0,
                    module: 0,
                    args: vec![PolyType::Var(1), PolyType::Var(0)],
                    len_args: vec![],
                    name: "A",
                })),
            )],
            span: Span::default(),
            module: 0,
        });
        let mut scratch = ScratchRegs::default();
        let a = generics.instantiate_struct(0, &[Type::I64, Type::U32], &[], 0, scratch.regs());
        assert_eq!(generics.inst_structs.len(), 2);
        let swapped = generics
            .lookup_struct(0, 0, &[Type::U32, Type::I64], &[])
            .expect("the swapped instantiation was minted by the recursion");
        assert_ne!(a, swapped);
        let payload_of = |ty: &Type| {
            let Type::OwnedCell(id, _) = ty else {
                panic!("the field is a cell: {ty:?}")
            };
            scratch.cells[id.index()].payload
        };
        let Type::Struct(a_id, _) = a else {
            panic!("expected a struct")
        };
        let Type::Struct(swapped_id, _) = swapped else {
            panic!("expected a struct")
        };
        assert_eq!(
            payload_of(&generics.inst_structs[a_id.index()].fields[0].1),
            swapped,
            "A[i64 u32]'s next points at A[u32 i64]"
        );
        assert_eq!(
            payload_of(&generics.inst_structs[swapped_id.index()].fields[0].1),
            a,
            "and back again, which is what makes the closure finite"
        );
    }

    /// P7 slice 3a phase 2 (R2): the id-collision trap the spec's own review
    /// caught, pinned as a mutation-testable guard. Mint one parse-time
    /// instance the ordinary way (into a live `structs` vec via `flush_
    /// structs_into`/`rebase`, exactly as `driver::assemble_module` does),
    /// then mint a *second, distinct* instantiation of the same header
    /// downstream (after the flush+rebase, as check/lowering would): the
    /// naive bug -- reusing the stale `struct_base` without rebasing, or
    /// minting into a `inst_structs` nobody flushes -- makes the second id
    /// collide with the first's, silently sharing one `StructId` for two
    /// distinct field layouts. A single mint in isolation cannot catch
    /// this: only an *interleaved* sequence (mint, flush, mint again) can.
    #[test]
    fn interleaved_downstream_mint_id_differs_from_parsetime_instance() {
        let decl = GenericStructDecl {
            name: "Box".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec![],
            fields: vec![("val".to_string(), PolyType::Var(0))],
            span: Span::default(),
            module: 0,
        };
        let mut structs: Vec<StructDecl> = Vec::new();
        let mut generics = GenericTypes::with_bases(structs.len(), 0);
        generics.structs.push(decl);

        // Parse-time: one instance, flushed onto the live registry exactly as
        // `assemble_module` does after the whole closure has parsed.
        let mut scratch = ScratchRegs::default();
        let a = generics.instantiate_struct(0, &[Type::I64], &[], 0, scratch.regs());
        generics.flush_structs_into(&mut structs);
        generics.rebase(structs.len(), 0);

        // Downstream (check/lowering-time): a *different* argument list mints
        // a fresh entry, whose id must count from the post-flush length, not
        // from the stale base `a` was minted against.
        let b = generics.instantiate_struct(0, &[Type::U32], &[], 0, scratch.regs());
        generics.flush_structs_into(&mut structs);

        assert_ne!(a, b, "a downstream mint of a distinct instantiation must not collide with the earlier parse-time one");
        let Type::Struct(a_id, _) = a else {
            panic!("expected a Type::Struct")
        };
        let Type::Struct(b_id, _) = b else {
            panic!("expected a Type::Struct")
        };
        assert_ne!(a_id, b_id);
        assert_eq!(structs.len(), 2);
        assert_eq!(
            structs[a_id.index()].fields,
            vec![("val".to_string(), Type::I64)]
        );
        assert_eq!(
            structs[b_id.index()].fields,
            vec![("val".to_string(), Type::U32)]
        );
    }

    /// The enum twin, over a non-zero `enum_base` so the instantiation's id is
    /// counted from the base rather than from zero.
    #[test]
    fn instantiate_enum_dedups_and_counts_from_its_base() {
        let decl = GenericEnumDecl {
            name: "Res".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec![],
            variants: vec![GenericVariantDecl {
                name: "Ok".to_string(),
                fields: vec![("val".to_string(), PolyType::Var(0))],
                span: Span::default(),
            }],
            span: Span::default(),
            module: 0,
        };
        let mut generics = GenericTypes::with_bases(0, 1);
        generics.enums.push(decl);
        let mut scratch = ScratchRegs::default();
        let a = generics.instantiate_enum(0, &[Type::I64], &[], 0, scratch.regs());
        let b = generics.instantiate_enum(0, &[Type::I64], &[], 0, scratch.regs());
        assert_eq!(a, b);
        assert_eq!(generics.inst_enums.len(), 1);
        assert_eq!(a, Type::Enum(EnumId::from_index(1), "Res[i64]"));
        assert_eq!(generics.inst_enums[0].variants[0].name, "Ok[i64]");
    }

    /// Phase 6 slice 2 (R1): a monomorphized generic enum's variant carries
    /// the enum's mangled name but the variant's *bare surface* name --
    /// `Res[i64].Ok`, never `Res[i64].Ok[i64]`.
    #[test]
    fn instantiate_enum_variant_display_static_uses_bare_variant_name() {
        let decl = GenericEnumDecl {
            name: "Res".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec![],
            variants: vec![GenericVariantDecl {
                name: "Ok".to_string(),
                fields: vec![("val".to_string(), PolyType::Var(0))],
                span: Span::default(),
            }],
            span: Span::default(),
            module: 0,
        };
        let mut generics = GenericTypes::with_bases(0, 1);
        generics.enums.push(decl);
        let mut scratch = ScratchRegs::default();
        generics.instantiate_enum(0, &[Type::I64], &[], 0, scratch.regs());
        assert_eq!(
            generics.inst_enums[0].variants[0].display_static,
            "Res[i64].Ok"
        );
    }

    /// Phase 6 slice 2 (R1): `Type::name()`/`Display` return the leaked
    /// `Enum.Variant` name directly, with no registry lookup.
    #[test]
    fn type_variant_name_and_display_render_enum_dot_variant() {
        let ty = Type::Variant(EnumId::from_index(0), 1, "Shape.Circle");
        assert_eq!(ty.name(), "Shape.Circle");
        assert_eq!(ty.to_string(), "Shape.Circle");
    }

    /// Phase 6 slice 2 (R1): `variant_type` is the sole constructor, reading
    /// `display_static` off the registry entry rather than reformatting it,
    /// for both a concrete and a monomorphized generic enum; two calls for
    /// the same `(EnumId, vi)` build byte-identical, and thus equal,
    /// `Type::Variant`s. The concrete half uses the `variant()` placeholder
    /// builder (bare `display_static`), so it does not itself cover the
    /// `Shape.Circle` naming rule — that's asserted in `parser.rs`.
    #[test]
    fn variant_type_reads_display_static_and_is_stable_across_calls() {
        let enums = vec![EnumDecl {
            name: "Shape".to_string(),
            name_static: "Shape",
            variants: vec![variant("Circle", vec![("r".to_string(), Type::F64)])],
            span: Span::default(),
            module: 0,
        }];
        let a = variant_type(&enums, EnumId::from_index(0), 0);
        let b = variant_type(&enums, EnumId::from_index(0), 0);
        assert_eq!(a, b);
        assert_eq!(a.name(), "Circle");

        let decl = GenericEnumDecl {
            name: "Res".to_string(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec![],
            variants: vec![GenericVariantDecl {
                name: "Ok".to_string(),
                fields: vec![("val".to_string(), PolyType::Var(0))],
                span: Span::default(),
            }],
            span: Span::default(),
            module: 0,
        };
        let mut generics = GenericTypes::with_bases(0, 1);
        generics.enums.push(decl);
        let mut scratch = ScratchRegs::default();
        generics.instantiate_enum(0, &[Type::I64], &[], 0, scratch.regs());
        let mono = variant_type(&generics.inst_enums, EnumId::from_index(0), 0);
        assert_eq!(mono.name(), "Res[i64].Ok");
    }

    /// P7.S12 (R3.2): `generic_variant_type` is the sole constructor, mirroring
    /// `variant_type`'s stability property -- two constructions of the same
    /// `(idx, module, vi)` compare equal, even though each leaks its own
    /// display string (`PolyType` derives `PartialEq`/`Eq`, and `&'static str`
    /// equality is content-based).
    #[test]
    fn generic_variant_type_is_stable_across_calls_and_renders_enum_dot_variant() {
        let decl = GenericEnumDecl {
            name: "Pair".to_string(),
            ty_var_names: vec!["'A".to_string()],
            len_var_names: vec![],
            variants: vec![GenericVariantDecl {
                name: "One".to_string(),
                fields: vec![("val".to_string(), PolyType::Var(0))],
                span: Span::default(),
            }],
            span: Span::default(),
            module: 0,
        };
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.enums.push(decl);
        let args = vec![PolyType::Var(0)];
        let a = generic_variant_type(&generics, 0, 0, 0, args.clone(), vec![]);
        let b = generic_variant_type(&generics, 0, 0, 0, args, vec![]);
        assert_eq!(a, b);
        let PolyType::GenericVariant { name, .. } = a else {
            panic!("generic_variant_type always returns GenericVariant: {a:?}")
        };
        assert_eq!(name, "Pair.One");
    }

    /// P7.S6a (R3): a `PolyType::Generic` with a non-empty `len_args`,
    /// narrowed into a `PolyType::GenericVariant` via `generic_variant_type`,
    /// carries the same `len_args` forward unchanged -- must fail if
    /// `len_args` is dropped at the `Operative`/`GenericVariant` boundary.
    #[test]
    fn generic_variant_type_carries_len_args_from_its_scrutinee() {
        let decl = GenericEnumDecl {
            name: "Buffer".to_string(),
            ty_var_names: vec!["'A".to_string()],
            len_var_names: vec!["'N".to_string()],
            variants: vec![GenericVariantDecl {
                name: "Full".to_string(),
                fields: vec![("val".to_string(), PolyType::Var(0))],
                span: Span::default(),
            }],
            span: Span::default(),
            module: 0,
        };
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.enums.push(decl);
        let scrutinee_len_args = vec![Len::Var(0)];
        let pt = generic_variant_type(
            &generics,
            0,
            0,
            0,
            vec![PolyType::Var(0)],
            scrutinee_len_args.clone(),
        );
        let PolyType::GenericVariant { len_args, .. } = pt else {
            panic!("generic_variant_type always returns GenericVariant: {pt:?}")
        };
        assert_eq!(len_args, scrutinee_len_args);
    }

    /// P7.S6a (R3): `ground_member_poly`'s `Generic` arm clone-forwards
    /// `len_args` unchanged, exactly as its neighboring `Array` arm already
    /// clones a bare array's `len` through -- named per CLAUDE.md's "every
    /// stage function gets a happy-path test", not left as an unwitnessed
    /// mechanical claim.
    #[test]
    fn ground_member_poly_generic_arm_clones_len_args_unchanged() {
        let pty = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Concrete(4), Len::Var(0)],
            name: "Buffer",
        };
        let target = PolyType::Concrete(Type::I64);
        let grounded = ground_member_poly(&pty, &target);
        let PolyType::Generic { len_args, .. } = grounded else {
            panic!("ground_member_poly's Generic arm stays Generic: {grounded:?}")
        };
        assert_eq!(len_args, vec![Len::Concrete(4), Len::Var(0)]);
    }

    /// P7.S12 (R4.1/R4.3): a `Var` field resolves positionally against the
    /// scrutinee's own argument list; a `Concrete` field passes through
    /// unchanged. No-interning is a signature property here (the function
    /// takes no registries to intern into), not something this test can
    /// independently verify.
    #[test]
    fn substitute_generic_variant_field_var_and_concrete_arms() {
        let args = vec![PolyType::Concrete(Type::U32)];

        let var_field = substitute_generic_variant_field(&PolyType::Var(0), &args);
        assert_eq!(var_field, PolyType::Concrete(Type::U32));

        let concrete_field =
            substitute_generic_variant_field(&PolyType::Concrete(Type::I64), &args);
        assert_eq!(concrete_field, PolyType::Concrete(Type::I64));
    }

    /// Round-2 review fix (R4): a struct argument's bare name is the plain
    /// spelling when it is unique in the merged registry -- the ordinary
    /// case, and the one the NFR's `Box[i64]`-over-`sooth_mono_...` argument
    /// rests on.
    #[test]
    fn type_instantiation_name_unambiguous_struct_arg_stays_bare() {
        let structs = vec![StructDecl {
            name: "P".to_string(),
            name_static: "P",
            fields: vec![("x".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        }];
        let arg = Type::Struct(StructId::from_index(0), "P");
        assert_eq!(
            type_instantiation_name(
                "Box",
                &[arg],
                &[],
                NameRegistries {
                    structs: &structs,
                    ..EMPTY_REGS
                }
            ),
            "Box[P]"
        );
    }

    /// The bug this fix exists for: two structs (a local `P` and an imported
    /// `P`) share a bare name but are distinct registry entries. Deduping or
    /// naming an instantiation on `Type::name()` alone cannot tell them
    /// apart; the merged `structs` table can, since it holds both entries.
    #[test]
    fn type_instantiation_name_ambiguous_struct_arg_gets_disambiguated() {
        let mk = |module: u32| StructDecl {
            name: "P".to_string(),
            name_static: "P",
            fields: vec![("x".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module,
        };
        let structs = vec![mk(0), mk(1)];
        let local = Type::Struct(StructId::from_index(0), "P");
        let imported = Type::Struct(StructId::from_index(1), "P");
        let regs = NameRegistries {
            structs: &structs,
            ..EMPTY_REGS
        };
        let local_name = type_instantiation_name("Box", &[local], &[], regs);
        let imported_name = type_instantiation_name("Box", &[imported], &[], regs);
        assert_ne!(
            local_name, imported_name,
            "two structs sharing a bare name must not render the same instantiation name"
        );
        assert_eq!(local_name, "Box[P.0]");
        assert_eq!(imported_name, "Box[P.1]");
    }

    /// The same ambiguity one indirection down. `intern_ref_type` builds
    /// `&P` from the same module-blind `Type::name()` the tie-break exists
    /// to work around, and `intern_owned_cell_type`/`intern_array_type` do
    /// the same, so a wrapped argument only renders injectively if the
    /// tie-break recurses into the registry entry instead of trusting its
    /// baked-in spelling.
    #[test]
    fn type_instantiation_name_ambiguous_wrapped_struct_arg_gets_disambiguated() {
        let mk = |module: u32| StructDecl {
            name: "P".to_string(),
            name_static: "P",
            fields: vec![("x".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module,
        };
        let structs = vec![mk(0), mk(1)];
        let local = Type::Struct(StructId::from_index(0), "P");
        let imported = Type::Struct(StructId::from_index(1), "P");

        let mut refs = Vec::new();
        let mut cells = Vec::new();
        let mut arrays = Vec::new();
        let wrapped: Vec<(Type, Type)> = vec![
            (
                intern_ref_type(&mut refs, local, false),
                intern_ref_type(&mut refs, imported, false),
            ),
            (
                intern_owned_cell_type(&mut cells, local),
                intern_owned_cell_type(&mut cells, imported),
            ),
            (
                intern_array_type(&mut arrays, local, 2),
                intern_array_type(&mut arrays, imported, 2),
            ),
        ];
        let regs = NameRegistries {
            structs: &structs,
            enums: &[],
            arrays: &arrays,
            cells: &cells,
            refs: &refs,
        };
        for (a, b) in &wrapped {
            assert_eq!(
                a.name(),
                b.name(),
                "the interned spellings collide, which is the premise"
            );
            assert_ne!(
                type_instantiation_name("Box", &[*a], &[], regs),
                type_instantiation_name("Box", &[*b], &[], regs),
                "a wrapped ambiguous argument must still render distinctly: {}",
                a.name()
            );
        }
        assert_eq!(
            type_instantiation_name("Box", &[wrapped[0].0], &[], regs),
            "Box[&P.0]"
        );
        assert_eq!(
            type_instantiation_name("Box", &[wrapped[1].1], &[], regs),
            "Box[^P.1]"
        );
        assert_eq!(
            type_instantiation_name("Box", &[wrapped[2].0], &[], regs),
            "Box[[P.0 2]]"
        );
    }

    /// The unambiguous twin: a wrapped argument keeps its plain structural
    /// spelling, so the recursion above costs the ordinary case nothing.
    #[test]
    fn type_instantiation_name_unambiguous_wrapped_arg_stays_bare() {
        let mut refs = Vec::new();
        let mut arrays = Vec::new();
        let r = intern_ref_type(&mut refs, Type::I64, true);
        let a = intern_array_type(&mut arrays, Type::I64, 4);
        let regs = NameRegistries {
            arrays: &arrays,
            refs: &refs,
            ..EMPTY_REGS
        };
        assert_eq!(
            type_instantiation_name("Box", &[r], &[], regs),
            "Box[&!i64]"
        );
        assert_eq!(
            type_instantiation_name("Box", &[a], &[], regs),
            "Box[[i64 4]]"
        );
    }

    #[test]
    fn intern_bundle_struct_distinct_tuples_and_orders_are_distinct_expected() {
        // Two outputs of the same types in the other order are a different
        // bundle: the tuple is ordered, deepest output first.
        let mut structs = Vec::new();
        let a = intern_bundle_struct(&mut structs, &[Type::I64, Type::U32]);
        let b = intern_bundle_struct(&mut structs, &[Type::U32, Type::I64]);
        let c = intern_bundle_struct(&mut structs, &[Type::I64, Type::U32, Type::I64]);
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
        assert_eq!(outer.to_string(), "array[array[i64 4] 4]");
    }

    #[test]
    fn instantiation_symbol_reproduces_native_spelling_expected() {
        let mut subst = Subst::default();
        subst.ty.push((0, Type::I64));
        assert_eq!(instantiation_symbol("id", &subst), "sooth_mono_id__t0_i64");
    }

    fn bare_word(name: &str) -> WordDef {
        WordDef {
            name: name.to_string(),
            effect: StackEffect {
                inputs: Vec::new(),
                outputs: Vec::new(),
            },
            body: Vec::new(),
            poly: None,
            declares_inline: false,
            module: 0,
            span: Span::default(),
            declared_globals: None,
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
