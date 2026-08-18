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
    /// Phase 4 slice 8a phase 2 (R7): the call sites that resolved to a user
    /// overload of a builtin-named word (e.g. `+` on two `Vec2`), keyed by the
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
    /// (and every REPL session) has exactly one entry. Every `StructDecl`/
    /// `EnumDecl`/`WordDef`/`ExternDecl` carries an owning module id indexing
    /// this vector; the entry carries that module's qualifier->module import
    /// map and its parsed `export:` list.
    pub modules: Vec<ModuleInfo>,
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
/// variables (`type: Box 'T ...`), parsed into its variable-scoped field
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
#[derive(Debug)]
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
    /// a hand-written concrete `type:` (R5).
    struct_keys: Vec<(usize, u32, Vec<Type>)>,
    enum_keys: Vec<(usize, u32, Vec<Type>)>,
    /// P7 slice 3a phase 2 (R2): the resolved `Type` each `struct_keys` entry
    /// minted, parallel by index. Reading `id`/`name` back from here (rather
    /// than recomputing `struct_base + i`) is what makes a downstream mint
    /// (after `struct_base` has been rebased past a parse-time batch) safe:
    /// an entry's real id is whatever was true the moment it was minted, not
    /// a function of the *current* base.
    struct_resolved: Vec<Type>,
    /// The enum twin of `struct_resolved`.
    enum_resolved: Vec<Type>,
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
/// processing order. Spelled the way `ArrayDecl`'s `[i64 4]` name is -- the
/// structural shape itself -- rather than through `instantiation_symbol`'s
/// sanitizing scheme: this name is registry identity and diagnostic
/// rendering (`sooth_mono_Box__t0_i64` would be a regression in every type
/// mismatch naming one), a sanitized join is lossy enough for two distinct
/// argument lists to collide, and the one QBE-facing use of a type name is
/// sanitized injectively at the emission site anyway. `[` is a lexer
/// delimiter, so no source type-name token can ever equal one of these.
/// `regs` is threaded through only to break a struct/enum argument's
/// bare-name tie, at whatever depth it sits (`type_arg_key`).
pub fn type_instantiation_name(base: &str, args: &[Type], regs: NameRegistries) -> String {
    let args: Vec<String> = args.iter().map(|t| type_arg_key(t, regs)).collect();
    format!("{base}[{}]", args.join(" "))
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

/// Substitute a generic declaration's field type against a use site's
/// concrete type arguments. `parse_generic_field_type_expr` admits exactly
/// two field forms -- a bare bound variable and a fully concrete type (D1
/// rules out an open application) -- so those are the two shapes here.
fn substitute_generic_field(pty: &PolyType, args: &[Type]) -> Type {
    match pty {
        PolyType::Concrete(t) => *t,
        PolyType::Var(v) => args[*v as usize],
        other => unreachable!("a generic `type:` field is never {other:?}"),
    }
}

impl GenericTypes {
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

    /// Read-only mint lookup: the already-resolved `Type` for one
    /// application of generic struct `idx`, if this exact `(idx, module,
    /// args)` key has ever been minted (parse-time or downstream). Used by
    /// lowering (`subst_polytype`), which only ever looks up an
    /// instantiation check has already minted, never mints one itself (the
    /// same division the array/ref arms already draw).
    pub fn lookup_struct(&self, idx: usize, module: u32, args: &[Type]) -> Option<Type> {
        self.struct_keys
            .iter()
            .position(|(gi, m, a)| *gi == idx && *m == module && a == args)
            .map(|i| self.struct_resolved[i])
    }

    /// The enum twin of `lookup_struct`.
    pub fn lookup_enum(&self, idx: usize, module: u32, args: &[Type]) -> Option<Type> {
        self.enum_keys
            .iter()
            .position(|(gi, m, a)| *gi == idx && *m == module && a == args)
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
    pub fn struct_instantiation_of(&self, id: StructId) -> Option<(usize, u32, &[Type])> {
        let i = self
            .struct_resolved
            .iter()
            .position(|t| matches!(t, Type::Struct(sid, _) if *sid == id))?;
        let (gi, m, args) = &self.struct_keys[i];
        Some((*gi, *m, args))
    }

    /// The enum twin of `struct_instantiation_of`.
    pub fn enum_instantiation_of(&self, id: EnumId) -> Option<(usize, u32, &[Type])> {
        let i = self
            .enum_resolved
            .iter()
            .position(|t| matches!(t, Type::Enum(eid, _) if *eid == id))?;
        let (gi, m, args) = &self.enum_keys[i];
        Some((*gi, *m, args))
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
    pub fn instantiate_struct(
        &mut self,
        idx: usize,
        args: &[Type],
        module: u32,
        regs: NameRegistries,
    ) -> Type {
        if let Some(ty) = self.lookup_struct(idx, module, args) {
            return ty;
        }
        let name = type_instantiation_name(&self.structs[idx].name, args, regs);
        let decl = &self.structs[idx];
        let fields: Vec<(String, Type)> = decl
            .fields
            .iter()
            .map(|(fname, pty)| (fname.clone(), substitute_generic_field(pty, args)))
            .collect();
        let span = decl.span;
        let name_static: &'static str = Box::leak(name.clone().into_boxed_str());
        let id = StructId::from_index(self.struct_base + self.inst_structs.len());
        let ty = Type::Struct(id, name_static);
        self.struct_keys.push((idx, module, args.to_vec()));
        self.struct_resolved.push(ty);
        self.inst_structs.push(StructDecl {
            name,
            name_static,
            fields,
            span,
            has_drop_overload: false,
            is_bundle: false,
            module,
        });
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
        module: u32,
        regs: NameRegistries,
    ) -> Type {
        if let Some(ty) = self.lookup_enum(idx, module, args) {
            return ty;
        }
        let name = type_instantiation_name(&self.enums[idx].name, args, regs);
        let decl = &self.enums[idx];
        let variants: Vec<VariantDecl> = decl
            .variants
            .iter()
            .map(|variant| {
                let vname = type_instantiation_name(&variant.name, args, regs);
                let display = format!("{name}.{}", generic_surface_name(&variant.name));
                VariantDecl {
                    name_static: Box::leak(vname.clone().into_boxed_str()),
                    name: vname,
                    display_static: Box::leak(display.into_boxed_str()),
                    fields: variant
                        .fields
                        .iter()
                        .map(|(fname, pty)| (fname.clone(), substitute_generic_field(pty, args)))
                        .collect(),
                    span: variant.span,
                }
            })
            .collect();
        let span = decl.span;
        let name_static: &'static str = Box::leak(name.clone().into_boxed_str());
        let id = EnumId::from_index(self.enum_base + self.inst_enums.len());
        let ty = Type::Enum(id, name_static);
        self.enum_keys.push((idx, module, args.to_vec()));
        self.enum_resolved.push(ty);
        self.inst_enums.push(EnumDecl {
            name,
            name_static,
            variants,
            span,
            module,
        });
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
                display_static: "bool.False",
                fields: Vec::new(),
                span: Span::default(),
            },
            VariantDecl {
                name: "True".to_string(),
                name_static: "True",
                display_static: "bool.True",
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
        declares_inline: false,
        module: 0,
        span: Span::default(),
        declared_globals: None,
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
    /// Slice 11 (R1): the declared `inline` keyword, spelled between the name
    /// and the effect. It makes "always spliced at the call site" a *declared*
    /// property rather than one inferred from the signature's shape, so a word
    /// taking no quotation can still mint no `IrFunc` and no call
    /// (`is_combinator`, the single predicate `check` and `ir::lower` share, is
    /// the only load-bearing reader). The guarantee is unconditional: a shape
    /// that cannot be spliced (a clause body, a variable-bearing signature) is
    /// a located error at the definition, never a silent fall-back to a real
    /// call.
    pub declares_inline: bool,
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
    /// (`&'T`, `&['T 4]`, and their `&!` twins): the referent, then whether
    /// it is mutable. There is deliberately no `RefId` -- the referent may be
    /// a variable, which no registry entry can name; the id is minted only
    /// when the referent grounds to a concrete `Type` (`apply_subst` /
    /// `subst_polytype`, via `intern_ref_type`), and a fully-concrete
    /// referent folds to `Concrete(Type::Ref(..))` at parse time. Mutability
    /// rides the variant for the same reason `Type::Ref` carries it: it is
    /// the classification bit (`Copy`-ness, store-vs-fetch, exclusivity),
    /// asked at sites that hold no registry.
    Ref(Box<PolyType>, bool),
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
        name: &'static str,
    },
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
    /// Phase 6 slice 2 (R1): one variant of an enum, standalone rather than
    /// carried inline as clause-body context -- the type Slice 3's eliminator
    /// binds an arm's payload to. Carries the owning `EnumId`, the variant's
    /// index into `EnumDecl.variants`, and a leaked `Enum.Variant` display
    /// name sourced once from `VariantDecl::display_static` (never a per-site
    /// `format!`+`Box::leak`, see `variant_type`), so two `Type::Variant`s for
    /// the same `(EnumId, vi)` are always byte-identical and compare equal.
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
        Type::Quotation(eff) | Type::InlineQuotation(eff) => Some(eff),
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
            Type::Variant(_, _, name) => name,
            Type::Array(_, name) => name,
            Type::OwnedCell(_, name) => name,
            Type::Ref(_, _, name) => name,
            Type::Usize => "usize",
            Type::Isize => "isize",
            Type::Str => "str",
            Type::Cstr => "cstr",
            Type::Quotation(eff) => eff.name_static,
            // The `~[ ... ]` spelling is baked into `name_static` by
            // `inline_quotation_type`, so this mirrors the `Quotation` arm.
            Type::InlineQuotation(eff) => eff.name_static,
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
    /// arm-to-variant routing and the IR's clause dispatch match against, and
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
    /// A word invocation, or a reference to a named local.
    Call(String),
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
    /// Slice 6h (D1): a body-level `[ Type ; Count ]` raw array constructor,
    /// carrying the parse-time-interned `Type::Array(id)` for the shape.
    /// Concrete-path only: `poly_term` rejects it eagerly (there is nowhere
    /// to intern a body-internal shape absent from a poly signature).
    ArrayCtor(Type),
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
            TermKind::Quotation(inner, is_inline, annot) => {
                let mut inner_bound = bound.clone();
                TermKind::Quotation(
                    rename_terms(inner, uid, &mut inner_bound),
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

    const EMPTY_REGS: NameRegistries<'static> = NameRegistries {
        structs: &[],
        enums: &[],
        arrays: &[],
        cells: &[],
        refs: &[],
    };

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
    #[test]
    fn is_quotation_type_accepts_both_variants_only() {
        let ord = quotation_type(vec![Type::I64], Vec::new());
        let inl = inline_quotation_type(vec![Type::I64], Vec::new());
        assert!(is_quotation_type(ord).is_some());
        assert!(is_quotation_type(inl).is_some());
        assert!(is_quotation_type(Type::I64).is_none());
        assert!(is_quotation_type(Type::Str).is_none());
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
            generic_structs: Vec::new(),
            generic_enums: Vec::new(),
            generics: GenericTypes::default(),
            externs: Vec::new(),
            instantiations: std::collections::HashMap::new(),
            builtin_overloads: std::collections::HashMap::new(),
            resolved_fields: std::collections::HashMap::new(),
            resolved_variant_fields: std::collections::HashMap::new(),
            modules: Vec::new(),
            statics: Vec::new(),
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
            generic_structs: Vec::new(),
            generic_enums: Vec::new(),
            generics: GenericTypes::default(),
            externs: Vec::new(),
            instantiations: std::collections::HashMap::new(),
            builtin_overloads: std::collections::HashMap::new(),
            resolved_fields: std::collections::HashMap::new(),
            resolved_variant_fields: std::collections::HashMap::new(),
            modules: Vec::new(),
            statics: Vec::new(),
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
            generic_structs: Vec::new(),
            generic_enums: Vec::new(),
            generics: GenericTypes::default(),
            externs: Vec::new(),
            instantiations: std::collections::HashMap::new(),
            builtin_overloads: std::collections::HashMap::new(),
            resolved_fields: std::collections::HashMap::new(),
            resolved_variant_fields: std::collections::HashMap::new(),
            modules: Vec::new(),
            statics: Vec::new(),
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
            fields: vec![("val".to_string(), PolyType::Var(0))],
            span: Span::default(),
            module: 0,
        };
        let mut generics = GenericTypes::with_bases(3, 1);
        generics.structs.push(decl);
        let a = generics.instantiate_struct(0, &[Type::I64], 0, EMPTY_REGS);
        let b = generics.instantiate_struct(0, &[Type::I64], 0, EMPTY_REGS);
        let c = generics.instantiate_struct(0, &[Type::BOOL], 0, EMPTY_REGS);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(generics.inst_structs.len(), 2);
        assert_eq!(a, Type::Struct(StructId::from_index(3), "Box[i64]"));
        assert_eq!(c, Type::Struct(StructId::from_index(4), "Box[bool]"));
        assert_eq!(
            generics.inst_structs[0].fields,
            vec![("val".to_string(), Type::I64)]
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
            fields: vec![("val".to_string(), PolyType::Var(0))],
            span: Span::default(),
            module: 0,
        };
        let mut structs: Vec<StructDecl> = Vec::new();
        let mut generics = GenericTypes::with_bases(structs.len(), 0);
        generics.structs.push(decl);

        // Parse-time: one instance, flushed onto the live registry exactly as
        // `assemble_module` does after the whole closure has parsed.
        let a = generics.instantiate_struct(0, &[Type::I64], 0, EMPTY_REGS);
        generics.flush_structs_into(&mut structs);
        generics.rebase(structs.len(), 0);

        // Downstream (check/lowering-time): a *different* argument list mints
        // a fresh entry, whose id must count from the post-flush length, not
        // from the stale base `a` was minted against.
        let b = generics.instantiate_struct(0, &[Type::BOOL], 0, EMPTY_REGS);
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
            vec![("val".to_string(), Type::BOOL)]
        );
    }

    /// The enum twin, including the `enum_base` the reserved `bool` entry
    /// forces every real program to have.
    #[test]
    fn instantiate_enum_dedups_and_counts_from_its_base() {
        let decl = GenericEnumDecl {
            name: "Res".to_string(),
            ty_var_names: vec!["'T".to_string()],
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
        let a = generics.instantiate_enum(0, &[Type::I64], 0, EMPTY_REGS);
        let b = generics.instantiate_enum(0, &[Type::I64], 0, EMPTY_REGS);
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
        generics.instantiate_enum(0, &[Type::I64], 0, EMPTY_REGS);
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
        generics.instantiate_enum(0, &[Type::I64], 0, EMPTY_REGS);
        let mono = variant_type(&generics.inst_enums, EnumId::from_index(0), 0);
        assert_eq!(mono.name(), "Res[i64].Ok");
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
        let local_name = type_instantiation_name("Box", &[local], regs);
        let imported_name = type_instantiation_name("Box", &[imported], regs);
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
                type_instantiation_name("Box", &[*a], regs),
                type_instantiation_name("Box", &[*b], regs),
                "a wrapped ambiguous argument must still render distinctly: {}",
                a.name()
            );
        }
        assert_eq!(
            type_instantiation_name("Box", &[wrapped[0].0], regs),
            "Box[&P.0]"
        );
        assert_eq!(
            type_instantiation_name("Box", &[wrapped[1].1], regs),
            "Box[^P.1]"
        );
        assert_eq!(
            type_instantiation_name("Box", &[wrapped[2].0], regs),
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
        assert_eq!(type_instantiation_name("Box", &[r], regs), "Box[&!i64]");
        assert_eq!(type_instantiation_name("Box", &[a], regs), "Box[[i64 4]]");
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
