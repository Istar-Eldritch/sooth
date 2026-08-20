//! Struct/enum/array/cell/ref memory layout: `LayoutBuilder`, the layout
//! registries (`Structs`/`Enums`/`Arrays`/`Cells`/`Refs`/`Registries`), and
//! `build_registries`. Depends only on `types`.

use super::*;

/// The computed memory layout of one struct, word-width-neutral: every
/// offset/size/align is derived from field widths, never a hardcoded machine
/// word. `name` is the leaked `&'static str` the backend emits as `:name`.
#[derive(Debug, Clone)]
pub struct StructLayout {
    pub name: &'static str,
    pub size: u32,
    pub align: u32,
    pub fields: Vec<FieldLayout>,
    /// Whether this struct is linear (any field is, transitively, or it has a
    /// user `drop` overload).
    pub is_linear: bool,
    /// R2 (slice 8b): whether a user `: drop ( T -- )` overload was recognized
    /// for this struct, copied from `StructDecl::has_drop_overload`. Read by
    /// `expand_path` (R7), which sees only `Registries` and so cannot reach the
    /// declaration.
    pub has_drop_overload: bool,
    /// R10 (phase 4 slice 1): whether this is a synthesized multi-output return
    /// bundle, copied from `StructDecl::is_bundle`. Two readers:
    /// `synthesize_aggregate_destructors` skips a bundle however linear its
    /// fields fold (they are the caller's outputs, moved out by the unpack, so
    /// glue here would double-free one), and `lower_call` uses it as the
    /// discriminator for the unpack branch — bundle presence, not a raw output
    /// count, since the REPL's registries intern no bundle at all.
    pub bundle: bool,
    /// R11 (slice 8b): the session-wide override epoch (`Session::override_epoch`),
    /// `None` on the build path and for the whole REPL session until its first
    /// `drop` override is ever defined. Set by the session after
    /// `build_registries`, since it is a fact about the session's redefinition
    /// history, not about the declaration. It lives on the layout because that
    /// is the one thing both `struct_drop_symbol` call sites (destructor
    /// synthesis and `emit_drop`) already reach, so both mint the same name
    /// for a given epoch.
    ///
    /// R11.2 originally suffixed only the *overridden* struct's own symbol
    /// with its own per-struct redefinition count. That left every enclosing
    /// aggregate's glue (a struct/enum/cell composing the overridden struct)
    /// unsuffixed even though its body's `Call` target changes across an
    /// override event, which under `RTLD_GLOBAL`'s first-loaded-wins
    /// resolution let a stale, pre-override callee stay wired in forever.
    /// Stamping the *same* session-wide epoch onto every linear struct/enum/
    /// cell once any override exists (rather than only the overridden one)
    /// mints every destructor a fresh, never-before-used symbol on every
    /// override event, sidestepping that staleness without computing which
    /// aggregates actually reach the override transitively.
    ///
    /// One exception, and it is the whole of R11.3: an *overridden* struct's
    /// own symbol carries the epoch its override was **defined** at, not the
    /// session's current one, so the symbol never changes while that override
    /// stands. The user body is then lowered exactly once, on its own
    /// declaring line, and every later line resolves the pinned symbol
    /// through `RTLD_GLOBAL` instead of re-lowering a body it never re-checks.
    pub drop_generation: Option<u64>,
}

/// Whether a field's `IrType` is linear: an owning cell directly, or a nested
/// aggregate whose own layout is linear. `ensure_struct`/`ensure_enum` cannot
/// call this: each computes its own `is_linear` inline while `layouts` is
/// still being built, before a nested field's entry exists here.
pub(super) fn field_is_linear(
    ty: IrType,
    structs: &Structs,
    enums: &Enums,
    arrays: &Arrays,
) -> bool {
    match ty {
        // Always linear whatever its payload, so no payload lookup.
        IrType::OwnedCell(_) => true,
        IrType::Struct(id) => structs.layouts[id.index()].is_linear,
        IrType::Enum(id) => enums.layouts[id.index()].is_linear,
        IrType::Array(id) => arrays.layouts[id.index()].is_linear,
        _ => false,
    }
}

/// R2 (slice 8b): the user `drop` overload body to compile as each struct's
/// destructor, keyed the way destructor synthesis itself is keyed — by
/// `StructId`, never by the shared literal name `"drop"`, so overrides for
/// distinct structs cannot collide. Borrowed rather than owned: the bodies live
/// in the module being lowered (or, at the REPL, in the session).
pub type DropOverrides<'a> = HashMap<StructId, DropOverride<'a>>;

/// What an overridden struct's destructor is *for the module being lowered*.
#[derive(Debug, Clone, Copy)]
pub enum DropOverride<'a> {
    /// Compile this body as the struct's destructor. The build path always
    /// uses this; at the REPL, only the line declaring the override does.
    Body(&'a WordDef),
    /// R11.3: emit no destructor for this struct at all. Its symbol is pinned
    /// to the epoch its override was defined at, so the body compiled on that
    /// line is already loaded `RTLD_GLOBAL` and resolves for every later line.
    /// Re-lowering the retained body here would resolve its callees against a
    /// *later* line's env, which the body was never checked against — a
    /// redefined callee of different arity panics lowering outright.
    AlreadyLoaded,
}

/// The synthesized per-type destructor symbol for a linear struct. `epoch`
/// is `Some` only at the REPL once its session holds at least one `drop`
/// override (R11, R11.2): from that point on, *every* linear struct's/enum's/
/// cell's destructor symbol carries the session's current override epoch, not
/// only the overridden struct's own, because an unrelated struct's body may
/// itself `Call` an overridden struct's destructor (or one composing it,
/// transitively) and that callee's body changes across an override event.
/// Leaving such a symbol unmangled would define one global symbol repeatedly
/// with a differing body, ambiguous under the REPL's `RTLD_GLOBAL` loading,
/// and (worse) `RTLD_GLOBAL` keeps whichever definition loaded *first*, so a
/// later, correct recompilation would silently never take effect. Before any
/// override exists in the session, epoch is `None` and every symbol stays
/// unsuffixed, identical to the build path.
///
/// R11.3: for an *overridden* struct the epoch passed here is the one its
/// override was defined at rather than the session's current one, pinning that
/// one symbol across later override events (`StructLayout::drop_generation`).
/// The two uses of the counter cannot collide: a struct emits glue only at
/// epochs strictly before its override exists, and its override's own symbol
/// at the defining epoch is the only thing minted for it from then on.
pub(super) fn struct_drop_symbol(id: StructId, epoch: Option<u64>) -> String {
    match epoch {
        Some(g) => format!("sooth_struct_drop_{}__gen{g}", id.index()),
        None => format!("sooth_struct_drop_{}", id.index()),
    }
}

/// The synthesized per-type destructor symbol for a linear enum: mirrors
/// `struct_drop_symbol`, one uniform naming scheme for both aggregate kinds.
pub(super) fn enum_drop_symbol(id: EnumId, epoch: Option<u64>) -> String {
    match epoch {
        Some(g) => format!("sooth_enum_drop_{}__gen{g}", id.index()),
        None => format!("sooth_enum_drop_{}", id.index()),
    }
}

/// Mirrors `struct_drop_symbol`/`enum_drop_symbol`, one uniform naming
/// scheme across all three kinds.
pub(super) fn cell_drop_symbol(id: OwnedCellId, epoch: Option<u64>) -> String {
    match epoch {
        Some(g) => format!("sooth_cell_drop_{}__gen{g}", id.index()),
        None => format!("sooth_cell_drop_{}", id.index()),
    }
}

/// One field's placement within its owning struct: its byte offset and its own
/// `IrType`/size/align (a nested struct contributes its whole size/align).
#[derive(Debug, Clone, Copy)]
pub struct FieldLayout {
    pub offset: u32,
    pub ty: IrType,
    pub size: u32,
    pub align: u32,
}

/// How a generated struct-word name lowers: the four kinds keyed off the
/// struct registry, distinguishing a struct-op call from a normal user-word
/// call in `lower_call`.
#[derive(Debug, Clone, Copy)]
pub(super) enum StructWord {
    Construct(StructId),
    Destructure(StructId),
}

/// The IR's view of a program's structs: the per-`StructId` layout registry and
/// the generated-word name map (`S`/`S>` → `StructWord`). Built
/// once from the module and threaded into lowering; empty for a struct-free
/// program (the scalar paths never consult it).
#[derive(Debug, Default)]
pub struct Structs {
    pub layouts: Vec<StructLayout>,
    pub(super) words: HashMap<String, StructWord>,
    /// R10: the interned return bundles as `(output tuple, its id)`, the
    /// lookup a word's declared outputs go through to find the aggregate it
    /// returns. Keyed on the frontend `Type` tuple the checker interned by,
    /// not on the lowered `IrType`s (every reference collapses to `Ptr`, so
    /// those are not a faithful key). Empty when nothing interned one.
    pub bundles: Vec<(Vec<Type>, StructId)>,
}

/// The computed tagged layout of one enum (D3, M1), word-width-neutral: a
/// fixed `i32` discriminant tag placed first, then a payload region sized and
/// aligned to the largest variant. Each variant's fields are laid out within
/// the payload region (offsets relative to `payload_offset`) by the same
/// natural-alignment placement as struct fields; per-variant payloads overlay
/// the one region. `name` is the leaked `&'static str` the backend emits as
/// `:name`.
#[derive(Debug, Clone)]
pub struct EnumLayout {
    pub name: &'static str,
    pub tag_offset: u32,
    pub tag_ty: IrType,
    pub payload_offset: u32,
    pub size: u32,
    pub align: u32,
    pub variants: Vec<VariantLayout>,
    /// Slice 9 (D-A/R1): general zero-payload-enum scalar layout. Set when
    /// every variant's field list is empty; such an enum's value is a bare
    /// scalar discriminant (register-resident, `size`/`align` = 1, no
    /// payload region), never a memory aggregate. `Bool` is the first client
    /// (`type: Bool | False | True ;`), but this is computed generally from
    /// the variant shape, not keyed on any particular `EnumId`.
    pub is_scalar: bool,
    /// R7/R12 (Phase 4): whether this enum is linear (any variant's payload
    /// field is, transitively). A variant field's own `is_linear` is already
    /// resolved by the time this is computed (`place_fields` -> `size_align`
    /// recurses into nested fields first), so this is a one-shot fold, not a
    /// further recursion, mirroring `StructLayout::is_linear`.
    pub is_linear: bool,
    /// Mirrors `StructLayout::drop_generation`: the same session-wide override
    /// epoch, so an enum composing an overridden struct also gets a fresh
    /// destructor symbol per override event.
    pub drop_generation: Option<u64>,
}

/// One variant's payload placement: its fields laid out (first field deepest)
/// within the enum's shared payload region, each `FieldLayout::offset`
/// relative to `EnumLayout::payload_offset`.
#[derive(Debug, Clone)]
pub struct VariantLayout {
    pub fields: Vec<FieldLayout>,
}

/// The computed layout of one array type (D3, M2), word-width-neutral: the
/// element's `IrType`, the compile-time `count`, the per-element `stride`
/// (`round_up(elem_size, elem_align)`, so element `i` sits at `i * stride`),
/// the total `size` (`count * stride`), and the `align` (the element's align).
/// A `usize` element sizes from `WORD_WIDTH` (R15) via the same path as a
/// scalar field. `name` is the leaked `[T N]` spelling the backend emits
/// as `:name`.
#[derive(Debug, Clone)]
pub struct ArrayLayout {
    pub name: &'static str,
    pub elem: IrType,
    pub count: u32,
    pub stride: u32,
    pub size: u32,
    pub align: u32,
    pub is_linear: bool,
}

/// The IR's view of a program's arrays: the per-`ArrayId` layout registry.
/// Unlike `Structs`/`Enums` there is no generated-word name map: the array
/// words (`fill`/`len`) are generic and dispatched by name + operand type in
/// `lower_call`, not by a per-array symbol. Empty for an array-free program.
#[derive(Debug, Default)]
pub struct Arrays {
    pub layouts: Vec<ArrayLayout>,
}

/// How a generated enum-word name lowers, keyed off the enum registry
/// (parallel to `StructWord`, D10): a variant constructor naming its enum and
/// the variant's declaration index, or a whole-variant destructure (Phase 6
/// slice 3, R6). Per-field access is a receiver-directed projection
/// (`resolved_variant_fields`), not a name-fused word here.
///
/// `Eliminate` is the odd one out (Phase 6 slice 3, R5): it names the enum
/// rather than a variant, and it is the only entry `lower_enum_word` never
/// handles -- the call dispatch intercepts it first, since its lowering needs
/// the call's quotation operands, not just the enum's layout.
#[derive(Debug, Clone, Copy)]
pub(super) enum EnumWord {
    Construct(EnumId, usize),
    Destructure(EnumId, usize),
    Eliminate(EnumId),
}

/// The IR's view of a program's enums: the per-`EnumId` tagged-layout registry
/// and the variant-constructor name map (variant name → `EnumWord`). A
/// logically distinct registry from `Structs` (D10), built alongside it by
/// `build_registries`; empty for an enum-free program.
#[derive(Debug, Default)]
pub struct Enums {
    pub layouts: Vec<EnumLayout>,
    pub(super) words: HashMap<String, EnumWord>,
}

fn round_up(offset: u32, align: u32) -> u32 {
    offset.div_ceil(align) * align
}

/// The size/align of a scalar `IrType` at the default target word width
/// (`WORD_WIDTH`). Thin wrapper over `scalar_size_align_ww`; criterion 2's
/// structural test calls the `_ww` form directly with a flipped width to prove
/// `usize` sizing derives from the parameter, not a stray literal.
pub(super) fn scalar_size_align(ty: IrType) -> (u32, u32) {
    scalar_size_align_ww(ty, WORD_WIDTH)
}

/// The size/align of a scalar `IrType`, `usize` sized from the supplied
/// `word_width` (R15): `i8`/`u8`/`bool` = 1, `i16`/`u16` = 2, `i32`/`u32`/`f32`
/// = 4, `i64`/`u64`/`f64` = 8, `usize` = `word_width`. A `Ptr` is 8 (unused as
/// a field this slice). Never called on a `Struct`/`Enum`/`Array` (nested
/// aggregates resolve through the layout registry).
pub(super) fn scalar_size_align_ww(ty: IrType, word_width: u32) -> (u32, u32) {
    let bytes = match ty {
        IrType::Bool => 1,
        IrType::Int { bits, .. } => (bits / 8) as u32,
        IrType::Float { bits } => (bits / 8) as u32,
        IrType::Usize => word_width,
        IrType::Isize => word_width,
        // A cell is a pointer, so its width defers to `Ptr`'s convention; a
        // `Code` handle is one word too (a code pointer / table index).
        IrType::Ptr | IrType::OwnedCell(_) | IrType::Str | IrType::Cstr | IrType::Code => 8,
        IrType::Struct(_) => unreachable!("a struct field resolves via the layout registry"),
        IrType::Enum(_) => unreachable!("an enum field resolves via the layout registry"),
        IrType::Array(_) => unreachable!("an array field resolves via the layout registry"),
        IrType::Quotation(_) => {
            unreachable!("a quotation value resolves via `quotation_layout`, not a scalar")
        }
        // P7 slice 3c (R2.1): a slice is two words, not one, and its align is
        // a word rather than its size -- so it cannot be answered by this
        // function's `(bytes, bytes)` contract at all. Its figures come from
        // `slice_layout`, exactly as a quotation value's come from
        // `quotation_layout`. Mistaking a slice for `Str`'s single opaque word
        // is the specific failure the separate `IrType` variant exists to make
        // impossible, so this arm refuses rather than guesses.
        IrType::Slice(_) => unreachable!("a slice value resolves via `slice_layout`, not a scalar"),
    };
    (bytes, bytes)
}

/// The carried-stack bytes a slot of `ty` occupies. A scalar stays a
/// byte-identical 8-byte cell, so every scalar-only line marshals exactly as
/// before; a struct or enum occupies its aggregate size rounded up to a
/// multiple of 8 so the next slot stays 8-aligned. Cumulative sums give each
/// carried slot's byte offset in the buffer.
pub fn carried_slot_bytes(ty: IrType, structs: &Structs, enums: &Enums, arrays: &Arrays) -> u32 {
    match ty {
        IrType::Struct(id) => round_up(structs.layouts[id.index()].size, 8),
        IrType::Enum(id) => round_up(enums.layouts[id.index()].size, 8),
        IrType::Array(id) => round_up(arrays.layouts[id.index()].size, 8),
        // A quotation value is a two-slot aggregate; it marshals like any
        // aggregate, its size rounded up so the next slot stays 8-aligned.
        IrType::Quotation(_) => round_up(quotation_layout(WORD_WIDTH).size, 8),
        // P7 slice 3c (R2.2): a slice marshals as its two-slot `{ptr, len}`
        // aggregate, like `Quotation` and unlike the 8-byte scalar arm below
        // -- `Str`'s one-word answer would truncate the length.
        IrType::Slice(_) => round_up(slice_layout(WORD_WIDTH).size, 8),
        IrType::Int { .. }
        | IrType::Float { .. }
        | IrType::Bool
        | IrType::Usize
        | IrType::Isize
        | IrType::Ptr
        | IrType::OwnedCell(_)
        | IrType::Str
        | IrType::Cstr
        | IrType::Code => 8,
    }
}

impl Structs {
    /// Build just the struct registry (no enums). A thin wrapper over
    /// `build_registries` for struct-only callers; a struct with an enum field
    /// needs the full `build_registries` (its enums must be present to size
    /// the field, D9). Test-only: its only caller is `test_helpers::structs_of`.
    #[cfg(test)]
    pub(super) fn from_structs(structs: &[StructDecl]) -> Structs {
        build_registries(structs, &[], &[], &[], &[]).0
    }

    /// R10: the synthesized bundle struct a word with these declared outputs
    /// returns, or `None` when none was interned for the tuple — a word with
    /// fewer than two outputs, or any registry the checker never interned into
    /// (the REPL's), which then keeps its pre-slice single-value lowering.
    pub(super) fn bundle_for(&self, outputs: &[Type]) -> Option<StructId> {
        self.bundles
            .iter()
            .find(|(tys, _)| tys == outputs)
            .map(|(_, id)| *id)
    }
}

/// The IR's view of a program's owning cells: the per-`OwnedCellId` payload
/// `IrType`.
#[derive(Debug, Default)]
pub struct Cells {
    pub payload: Vec<IrType>,
    /// Mirrors `StructLayout::drop_generation`, parallel to `payload` since a
    /// cell has no per-item layout struct of its own to carry the field on.
    pub drop_generations: Vec<Option<u64>>,
}

/// The IR's view of a program's reference types: the per-`RefId` referent
/// `IrType`. Every reference lowers to `IrType::Ptr`, so this is the
/// only place the referent shape survives into lowering — it seeds
/// `FuncBuilder::ref_inner` for a word's reference-typed parameters.
#[derive(Debug, Default)]
pub struct Refs {
    pub referent: Vec<IrType>,
}

/// P7 slice 3c (R2.1): the IR's view of a program's slice types. Every slice
/// *value* shares one layout (`slice_layout`), so what is per-`SliceId` here
/// is only what lowering needs to reach the buffer behind the view: the
/// element's `IrType` (the referent an indexed element reference records) and
/// its `stride` (the same figure `ArrayLayout` computes, since a view over
/// `[T N]` walks that array's own elements). The backend sees none of it --
/// the element type is erased there exactly as it is for the `Ptr` every `&T`
/// becomes.
#[derive(Debug, Default)]
pub struct Slices {
    pub elem: Vec<IrType>,
    pub stride: Vec<u32>,
    /// Whether the view is mutable. Not an ABI or layout figure: it is the
    /// second half of the interning key, so a construction site can find the
    /// `SliceId` the checker already minted for the shape it is building.
    pub mutable: Vec<bool>,
}

/// Build the slice registry against the already-built aggregate registries.
/// Separate from `build_registries` because a slice is never a field or an
/// element of anything (R5 bans it from every such position), so it takes no
/// part in the layout DFS and cannot make another layout depend on it.
pub fn build_slices(
    decls: &[SliceDecl],
    structs: &Structs,
    enums: &Enums,
    arrays: &Arrays,
) -> Slices {
    let mut out = Slices::default();
    for decl in decls {
        let elem = ir_type_of(decl.element);
        let (size, align) = match elem {
            IrType::Struct(id) => {
                let l = &structs.layouts[id.index()];
                (l.size, l.align)
            }
            IrType::Enum(id) => {
                let l = &enums.layouts[id.index()];
                (l.size, l.align)
            }
            IrType::Array(id) => {
                let l = &arrays.layouts[id.index()];
                (l.size, l.align)
            }
            other => scalar_size_align(other),
        };
        out.elem.push(elem);
        out.stride.push(round_up(size, align));
        out.mutable.push(decl.mutable);
    }
    out
}

/// The shared empty slice registry, mirroring `empty_statics`. Test-only:
/// every production lowering path builds a real one from `Module::slices`,
/// but a unit test that hand-builds a `Registries` needs an empty stand-in
/// with a `'static` lifetime.
#[cfg(test)]
pub fn empty_slices() -> &'static Slices {
    static EMPTY: std::sync::OnceLock<Slices> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Slices::default)
}

/// Phase 7 slice 2 (R1): the IR's view of a module's `static:` declarations --
/// each static's referent `IrType`, keyed by the module-mangled name a borrow
/// site spells (the same string that is its data symbol). Doubles as the
/// "is this name a static" test `lower_reference_word` needs once a local
/// lookup has missed, and as the referent `push_reference` records.
#[derive(Debug, Default)]
pub struct Statics {
    pub referent: std::collections::HashMap<String, IrType>,
}

/// Phase 7 slice 2 (D1/D3): the two lowering-side views of a module's
/// `static:` declarations, built together so the name a borrow site looks up
/// and the symbol the backend defines can only ever be the same string. The
/// `StaticData` come out in source order, so the emitted preamble is
/// deterministic. `enums` supplies a scalar enum's (i.e. `bool`'s) width,
/// which `scalar_size_align` deliberately refuses to guess.
pub fn build_statics(decls: &[StaticDecl], enums: &Enums) -> (Statics, Vec<StaticData>) {
    let mut table = Statics::default();
    let mut data = Vec::with_capacity(decls.len());
    for decl in decls {
        let ty = ir_type_of(decl.ty);
        table.referent.insert(decl.name.clone(), ty);
        let size = match ty {
            IrType::Enum(id) => enums.layouts[id.index()].size,
            other => scalar_size_align(other).0,
        };
        let init = match &decl.init {
            // The elided initialiser is the type's zero: `0`, `false`, and for
            // `str` the empty string, which is a descriptor like any other.
            StaticInit::Zero if ty == IrType::Str => StaticValue::Str(String::new()),
            StaticInit::Zero => StaticValue::Int(0),
            StaticInit::Int(n) => StaticValue::Int(*n),
            StaticInit::Bool(b) => StaticValue::Int(*b as i64),
            StaticInit::Str(s) => StaticValue::Str(s.clone()),
        };
        data.push(StaticData {
            symbol: decl.name.clone(),
            size,
            init,
        });
    }
    (table, data)
}

/// The shared empty static table, for every lowering path with no module
/// statics to see (the REPL, destructor synthesis, unit tests), so a
/// `Registries` can be built without each caller owning a `Statics`.
pub fn empty_statics() -> &'static Statics {
    static EMPTY: std::sync::OnceLock<Statics> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Statics::default)
}

/// The registries bundled as one `Copy` handle, so lowering and
/// destructor synthesis pass one argument instead of six (mirrors the
/// backend's `Layouts`). The registries stay logically separate types; this
/// only co-locates references to them.
#[derive(Debug, Clone, Copy)]
pub struct Registries<'a> {
    pub structs: &'a Structs,
    pub enums: &'a Enums,
    pub arrays: &'a Arrays,
    pub cells: &'a Cells,
    pub refs: &'a Refs,
    pub slices: &'a Slices,
    pub statics: &'a Statics,
}

/// Build the struct and enum layout + generated-word registries from a
/// program's declarations (the build path passes `&module.structs` /
/// `&module.enums`, the REPL passes its accumulated registries). The layout
/// pass is a single combined DFS so a struct field of enum type and a variant
/// field of struct/enum type are sized via the peer registry (D9); the
/// registries themselves stay logically separate (D10). Recursion is already
/// rejected by the checker, so the memoized layout recursion terminates.
pub fn build_registries(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
) -> (Structs, Enums, Arrays, Cells, Refs) {
    build_registries_ww(structs, enums, arrays, cells, refs, WORD_WIDTH)
}

/// `build_registries` with an explicit target word width (R15). Production
/// callers use `build_registries`; criterion 2's structural test flips
/// `word_width` here to prove a `usize`-embedding aggregate resizes with the
/// parameter (no stray literal `8`).
pub(super) fn build_registries_ww(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    word_width: u32,
) -> (Structs, Enums, Arrays, Cells, Refs) {
    let mut lb = LayoutBuilder {
        structs,
        enums,
        arrays,
        word_width,
        struct_memo: vec![None; structs.len()],
        enum_memo: vec![None; enums.len()],
        array_memo: vec![None; arrays.len()],
    };
    for i in 0..structs.len() {
        lb.ensure_struct(i);
    }
    for i in 0..enums.len() {
        lb.ensure_enum(i);
    }
    for i in 0..arrays.len() {
        lb.ensure_array(i);
    }
    let struct_layouts: Vec<StructLayout> = lb
        .struct_memo
        .into_iter()
        .map(|l| l.expect("layout"))
        .collect();
    let enum_layouts: Vec<EnumLayout> = lb
        .enum_memo
        .into_iter()
        .map(|l| l.expect("layout"))
        .collect();
    let array_layouts: Vec<ArrayLayout> = lb
        .array_memo
        .into_iter()
        .map(|l| l.expect("layout"))
        .collect();

    let mut swords = HashMap::new();
    // R10: a synthesized bundle is an ABI detail with no source spelling, so it
    // contributes no generated words; lowering reaches its pack and unpack
    // through `StructWord` directly, never by name.
    //
    // D7: each generated word is keyed under both `decl.name` (the mangled
    // spelling, e.g. `Box[i64]>val` -- what a resolved-overload call site's
    // `builtin_overloads` symbol names when two instantiations share a bare
    // surface name) and the bare surface spelling (`Box>val` -- the only
    // spelling a source term can ever contain, `[` being a lexer delimiter).
    // The surface key collides across instantiations sharing one surface
    // name, but that is only ever consulted by the unambiguous
    // single-instantiation case, where exactly one insert reaches it; an
    // ambiguous call site's checker-resolved symbol is looked up under the
    // (unique) mangled key instead (`lower_call`'s `builtin_overloads` arm).
    for (idx, decl) in structs.iter().enumerate().filter(|(_, d)| !d.is_bundle) {
        let id = StructId::from_index(idx);
        let surface = generic_surface_name(&decl.name);
        let mut insert = |mangled_key: String, surface_key: String, sw: StructWord| {
            if surface_key != mangled_key {
                swords.insert(surface_key, sw);
            }
            swords.insert(mangled_key, sw);
        };
        insert(
            decl.name.clone(),
            surface.to_string(),
            StructWord::Construct(id),
        );
        insert(
            format!("{}>", decl.name),
            format!("{surface}>"),
            StructWord::Destructure(id),
        );
    }

    let mut ewords = HashMap::new();
    for (idx, decl) in enums.iter().enumerate() {
        let id = EnumId::from_index(idx);
        // D7 (adopting the struct registry's dual-key `insert` closure,
        // R6): each generated word keys under both the mangled and bare
        // surface spelling, skipping the surface insert when they agree.
        // Shared by the per-variant constructors/destructures below and the
        // per-enum eliminator (Phase 6 slice 3, R5).
        let mut insert = |mangled_key: String, surface_key: String, ew: EnumWord| {
            if surface_key != mangled_key {
                ewords.insert(surface_key, ew);
            }
            ewords.insert(mangled_key, ew);
        };
        for (vi, variant) in decl.variants.iter().enumerate() {
            let surface = generic_surface_name(&variant.name);
            insert(
                variant.name.clone(),
                surface.to_string(),
                EnumWord::Construct(id, vi),
            );
            insert(
                format!("{}>", variant.name),
                format!("{surface}>"),
                EnumWord::Destructure(id, vi),
            );
        }
        // Phase 6 slice 3 (R5): the eliminator, keyed on the *enum*'s own name
        // under the same dual spelling the checker registers it by
        // (`enum_eliminator_sigs`: mangled symbol, bare surface env key).
        let enum_surface = generic_surface_name(&decl.name);
        insert(
            format!("{}?", decl.name),
            format!("{enum_surface}?"),
            EnumWord::Eliminate(id),
        );
    }

    let cell_payloads: Vec<IrType> = cells.iter().map(|d| ir_type_of(d.payload)).collect();
    let cell_drop_generations = vec![None; cell_payloads.len()];
    let ref_referents: Vec<IrType> = refs.iter().map(|d| ir_type_of(d.referent)).collect();

    let bundles: Vec<(Vec<Type>, StructId)> = structs
        .iter()
        .enumerate()
        .filter(|(_, d)| d.is_bundle)
        .map(|(idx, d)| {
            (
                d.fields.iter().map(|(_, ty)| *ty).collect(),
                StructId::from_index(idx),
            )
        })
        .collect();

    (
        Structs {
            layouts: struct_layouts,
            words: swords,
            bundles,
        },
        Enums {
            layouts: enum_layouts,
            words: ewords,
        },
        Arrays {
            layouts: array_layouts,
        },
        Cells {
            payload: cell_payloads,
            drop_generations: cell_drop_generations,
        },
        Refs {
            referent: ref_referents,
        },
    )
}

/// The shared field-placement + memoized layout core over the combined
/// struct+enum type graph. `ensure_struct`/`ensure_enum` fill their memo slot,
/// recursing into nested-aggregate fields first via `size_align`; `place_fields`
/// is the natural-alignment placement reused by both a struct body and each
/// variant's payload.
struct LayoutBuilder<'a> {
    structs: &'a [StructDecl],
    enums: &'a [EnumDecl],
    arrays: &'a [ArrayDecl],
    word_width: u32,
    struct_memo: Vec<Option<StructLayout>>,
    enum_memo: Vec<Option<EnumLayout>>,
    array_memo: Vec<Option<ArrayLayout>>,
}

impl LayoutBuilder<'_> {
    /// The size/align of a field of frontend type `ty`, sizing a nested struct
    /// or enum via its layout (computed on demand) and a scalar via its width.
    fn size_align(&mut self, ty: Type) -> (u32, u32) {
        match ty {
            Type::Struct(id, _) => {
                self.ensure_struct(id.index());
                let l = self.struct_memo[id.index()].as_ref().expect("inner layout");
                (l.size, l.align)
            }
            Type::Enum(id, _) => {
                self.ensure_enum(id.index());
                let l = self.enum_memo[id.index()].as_ref().expect("inner layout");
                (l.size, l.align)
            }
            Type::Array(id, _) => {
                self.ensure_array(id.index());
                let l = self.array_memo[id.index()].as_ref().expect("inner layout");
                (l.size, l.align)
            }
            // Slice 7a: a quotation field/element is the fixed two-slot value
            // aggregate, word-width-derived (`quotation_layout`), not a scalar
            // -- `scalar_size_align_ww` deliberately panics on it.
            Type::Quotation(_) => {
                let l = quotation_layout(self.word_width);
                (l.size, l.align)
            }
            _ => scalar_size_align_ww(ir_type_of(ty), self.word_width),
        }
    }

    /// Place `fields` at natural alignment (first field deepest), returning the
    /// per-field layouts, the total size (rounded up to the aggregate align),
    /// and that align (min 1).
    fn place_fields(&mut self, fields: &[(String, Type)]) -> (Vec<FieldLayout>, u32, u32) {
        let mut offset = 0u32;
        let mut align = 1u32;
        let mut out = Vec::with_capacity(fields.len());
        for (_, field_ty) in fields {
            let ir_ty = ir_type_of(*field_ty);
            let (size, falign) = self.size_align(*field_ty);
            let off = round_up(offset, falign);
            out.push(FieldLayout {
                offset: off,
                ty: ir_ty,
                size,
                align: falign,
            });
            offset = off + size;
            align = align.max(falign);
        }
        (out, round_up(offset, align), align)
    }

    fn ensure_struct(&mut self, idx: usize) {
        if self.struct_memo[idx].is_some() {
            return;
        }
        let structs = self.structs;
        let (fields, size, align) = self.place_fields(&structs[idx].fields);
        // R7: linear iff any field is, transitively. A nested aggregate
        // field's own `is_linear` is already memoized (`place_fields` ->
        // `size_align` ensures it first), so this is a plain fold over the
        // just-placed fields, not a further recursion.
        //
        // R2 (slice 8b): a user `drop` overload forces the bit regardless of
        // what the fields fold to. This is the IR's own, separately computed
        // linearity, not `check`'s: without the force, an all-`Copy`-fields
        // resource would get no synthesized destructor at all (filtered out of
        // `synthesize_aggregate_destructors`) and `emit_drop`'s guard would
        // discard it silently, so its override would never run.
        let has_drop_overload = structs[idx].has_drop_overload;
        let is_linear =
            has_drop_overload || fields.iter().any(|f| self.layout_field_is_linear(f.ty));
        self.struct_memo[idx] = Some(StructLayout {
            name: structs[idx].name_static,
            size,
            align,
            is_linear,
            has_drop_overload,
            // R10: carried through unchanged; a bundle is sized and laid out
            // exactly like a user struct, and differs only in getting no
            // destructor.
            bundle: structs[idx].is_bundle,
            // R11: the build path never suffixes a destructor symbol; the
            // REPL sets this from its own override registry after the build.
            drop_generation: None,
            fields,
        });
    }

    fn ensure_enum(&mut self, idx: usize) {
        if self.enum_memo[idx].is_some() {
            return;
        }
        let enums = self.enums;
        // Slice 9 (D-A/R1): general zero-payload-enum scalar layout. An enum
        // every variant of which declares no fields needs no payload region
        // at all -- the discriminant *is* the value -- so it lowers to a
        // bare 1-byte scalar (matching the old primitive `Bool`'s own
        // `scalar_size_align_ww` width), not the tagged aggregate below.
        // `Bool` is this rule's first client, not a carve-out of it: any
        // all-unit-variant user enum gets the same layout.
        let is_scalar = enums[idx].variants.iter().all(|v| v.fields.is_empty());
        if is_scalar {
            let variants = enums[idx]
                .variants
                .iter()
                .map(|_| VariantLayout { fields: Vec::new() })
                .collect();
            self.enum_memo[idx] = Some(EnumLayout {
                name: enums[idx].name_static,
                tag_offset: 0,
                tag_ty: IrType::Bool,
                payload_offset: 0,
                size: 1,
                align: 1,
                variants,
                is_scalar: true,
                is_linear: false,
                drop_generation: None,
            });
            return;
        }
        // The tag is a fixed i32 (M1), placed first; the payload follows at the
        // largest variant's alignment, so a tag narrower than that align gets
        // padded up to `payload_offset` (the round-up criterion 2 exercises).
        let tag_ty = IrType::Int {
            bits: 32,
            signed: true,
        };
        let (tag_size, tag_align) = scalar_size_align_ww(tag_ty, self.word_width);
        let mut variants = Vec::with_capacity(enums[idx].variants.len());
        let mut payload_align = 1u32;
        let mut max_payload = 0u32;
        for variant in &enums[idx].variants {
            let (fields, vsize, valign) = self.place_fields(&variant.fields);
            payload_align = payload_align.max(valign);
            max_payload = max_payload.max(vsize);
            variants.push(VariantLayout { fields });
        }
        let payload_offset = round_up(tag_size, payload_align);
        let align = tag_align.max(payload_align);
        let size = round_up(payload_offset + max_payload, align);
        // R7/R12: an enum is linear iff any variant's payload field is,
        // transitively (mirrors the struct fold above).
        let is_linear = variants
            .iter()
            .flat_map(|v| v.fields.iter())
            .any(|f| self.layout_field_is_linear(f.ty));
        self.enum_memo[idx] = Some(EnumLayout {
            name: enums[idx].name_static,
            tag_offset: 0,
            tag_ty,
            payload_offset,
            size,
            align,
            variants,
            is_scalar: false,
            is_linear,
            // R11: the build path never suffixes a destructor symbol; the
            // REPL sets this from its own override epoch after the build.
            drop_generation: None,
        });
    }

    /// Whether a just-laid-out field's `IrType` is linear (R7): an owning
    /// cell directly, or a nested struct/enum whose own memoized layout is
    /// linear. Shared by the struct and enum `is_linear` folds; both call
    /// sites have already ensured the nested aggregate's memo entry via
    /// `size_align`.
    fn layout_field_is_linear(&self, ty: IrType) -> bool {
        match ty {
            // Always linear whatever its payload, so no payload lookup.
            IrType::OwnedCell(_) => true,
            IrType::Struct(id) => {
                self.struct_memo[id.index()]
                    .as_ref()
                    .expect("nested struct field already laid out")
                    .is_linear
            }
            IrType::Enum(id) => {
                self.enum_memo[id.index()]
                    .as_ref()
                    .expect("nested enum field already laid out")
                    .is_linear
            }
            IrType::Array(id) => {
                self.array_memo[id.index()]
                    .as_ref()
                    .expect("nested array field already laid out")
                    .is_linear
            }
            _ => false,
        }
    }

    /// Compute one array's layout (M2): the element's size/align (recursing
    /// into a nested aggregate element via `size_align`), the per-element
    /// `stride = round_up(elem_size, elem_align)`, `align = elem_align`, and
    /// `size = count * stride`. Memoized like structs/enums so a nested-array
    /// element resolves once.
    fn ensure_array(&mut self, idx: usize) {
        if self.array_memo[idx].is_some() {
            return;
        }
        let element = self.arrays[idx].element;
        let count = self.arrays[idx].count;
        let elem = ir_type_of(element);
        let (elem_size, elem_align) = self.size_align(element);
        let stride = round_up(elem_size, elem_align);
        // R7: an array is linear iff its element is, transitively; `size_align`
        // above already ensured a nested struct/enum/array element's memo.
        let is_linear = self.layout_field_is_linear(elem);
        self.array_memo[idx] = Some(ArrayLayout {
            name: self.arrays[idx].name_static,
            elem,
            count,
            stride,
            size: stride * count,
            align: elem_align.max(1),
            is_linear,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::ir::test_helpers::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    #[test]
    fn ir_registers_overridden_struct_as_linear_despite_all_copy_fields() {
        // Criterion 20/R2: `StructLayout::is_linear` is the IR's own,
        // separately computed bit, folded from declared field types alone --
        // for a scalar-only resource that fold says `Copy`, so the override
        // has to force it. Without the force, no destructor would be
        // synthesized for `File` at all and `emit_drop`'s guard would discard
        // an `f drop` silently.
        let overridden = structs_of(&format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;"));
        let file = layout(&overridden, "File");
        assert!(file.is_linear);
        assert!(file.has_drop_overload);

        let plain = structs_of("type: File fd i64 ; : main ( -- ) 1 File drop ;");
        assert!(!layout(&plain, "File").is_linear);
    }

    #[test]
    fn enum_registry_keys_a_destructure_word_per_variant() {
        // Phase 6 slice 3 (R6): the only thing connecting a surface name to
        // the new lowering arm. Asserted through `enums_of` (which builds the
        // registry from source) rather than by calling `lower_enum_word`
        // directly, since the two lowering units hand-build their `EnumWord`
        // and so would stay green if this insert vanished entirely.
        let enums = enums_of(
            "type: Shape | Circle r i64 p i64 | Dot ;\n\
             : main ( -- ) ;\n",
        );
        // `bool` is injected as enum 0 ahead of any user enum
        // (`BOOL_ENUM_ID`), so `Shape` is enum 1.
        let id = EnumId::from_index(1);
        for (vi, name) in ["Circle", "Dot"].iter().enumerate() {
            assert!(
                matches!(enums.words.get(*name), Some(EnumWord::Construct(got_id, got_vi)) if *got_id == id && *got_vi == vi),
                "`{name}` should construct variant {vi}: {:?}",
                enums.words.get(*name)
            );
            let destructure = format!("{name}>");
            assert!(
                matches!(enums.words.get(&destructure), Some(EnumWord::Destructure(got_id, got_vi)) if *got_id == id && *got_vi == vi),
                "`{destructure}` should destructure variant {vi}: {:?}",
                enums.words.get(&destructure)
            );
        }
    }

    #[test]
    fn build_statics_widths_and_zero_values_follow_the_declared_type() {
        // D1/D3: the slot width is the declared type's, and an elided
        // initialiser is that type's zero -- `0`, `false`, and for `str` the
        // empty string, which is a descriptor like any other content.
        // Source order is preserved so the emitted preamble is deterministic.
        let ir = lower_src(
            "static: N i64 = 10 ;\n\
             static: W u32 ;\n\
             static: F bool = true ;\n\
             static: G bool ;\n\
             static: T str = \"hi\" ;\n\
             static: E str ;\n\
             : main ( -- ) ;",
        );
        let seen: Vec<(&str, u32, &StaticValue)> = ir
            .statics
            .iter()
            .map(|s| (s.symbol.as_str(), s.size, &s.init))
            .collect();
        let expected: Vec<(&str, u32, StaticValue)> = vec![
            ("N", 8, StaticValue::Int(10)),
            ("W", 4, StaticValue::Int(0)),
            // `bool` is an all-unit-variant enum, one byte, so its width comes
            // from the enum registry rather than `scalar_size_align`.
            ("F", 1, StaticValue::Int(1)),
            ("G", 1, StaticValue::Int(0)),
            ("T", 8, StaticValue::Str("hi".to_string())),
            ("E", 8, StaticValue::Str(String::new())),
        ];
        assert_eq!(seen.len(), expected.len(), "{seen:?}");
        for ((sym, size, init), (esym, esize, einit)) in seen.iter().zip(&expected) {
            assert_eq!((sym, size), (esym, esize), "{seen:?}");
            match (init, einit) {
                (StaticValue::Int(a), StaticValue::Int(b)) => assert_eq!(a, b, "{sym}"),
                (StaticValue::Str(a), StaticValue::Str(b)) => assert_eq!(a, b, "{sym}"),
                _ => panic!("{sym}: {init:?} is not {einit:?}"),
            }
        }
    }

    #[test]
    fn build_statics_keys_the_borrow_table_by_the_same_name_it_emits() {
        // The one invariant tying the two halves together: the name a borrow
        // site looks up and the symbol the backend defines are the same string,
        // so a module-mangled static cannot address storage that was never laid
        // down.
        let (table, data) = build_statics(
            &[StaticDecl {
                name: "COUNT".to_string(),
                ty: Type::I64,
                init: StaticInit::Zero,
                module: 0,
                span: Span::default(),
            }],
            &Enums::default(),
        );
        assert_eq!(data[0].symbol, "COUNT");
        assert_eq!(table.referent.get("COUNT"), Some(&IrType::I64));
    }

    #[test]
    fn carried_slot_bytes_scalar_is_eight_struct_is_aligned_aggregate() {
        // A scalar always occupies a byte-identical 8-byte carried cell (so
        // every scalar-only line marshals unchanged); a struct occupies its
        // aggregate size rounded up to a multiple of 8.
        let s = structs_of("type: Pair a i8 b i8 ;\ntype: Vec2 x i64 y i64 ;");
        assert_eq!(
            carried_slot_bytes(IrType::I64, &s, &Enums::default(), &Arrays::default()),
            8
        );
        assert_eq!(
            carried_slot_bytes(IrType::Bool, &s, &Enums::default(), &Arrays::default()),
            8
        );
        // Pair is two i8s = 2 bytes, rounded up to one 8-byte cell.
        assert_eq!(
            carried_slot_bytes(
                IrType::Struct(StructId::from_index(0)),
                &s,
                &Enums::default(),
                &Arrays::default()
            ),
            8
        );
        // Vec2 is two i64s = 16 bytes, already a multiple of 8.
        assert_eq!(
            carried_slot_bytes(
                IrType::Struct(StructId::from_index(1)),
                &s,
                &Enums::default(),
                &Arrays::default()
            ),
            16
        );
    }

    #[test]
    fn word_width_parameter_sizes_size_types_not_a_literal_eight() {
        // Criterion 2 (structural): both size types' size/align derive from the
        // word width parameter, not a hardcoded `8`. At the default width it is
        // 8; flipping the parameter to 4 changes the derived size of a bare
        // `usize`/`isize` and of an aggregate that embeds one, proving no stray
        // literal.
        assert_eq!(scalar_size_align(IrType::Usize), (8, 8));
        assert_eq!(scalar_size_align_ww(IrType::Usize, 8), (8, 8));
        assert_eq!(scalar_size_align_ww(IrType::Usize, 4), (4, 4));
        assert_eq!(scalar_size_align(IrType::Isize), (8, 8));
        assert_eq!(scalar_size_align_ww(IrType::Isize, 8), (8, 8));
        assert_eq!(scalar_size_align_ww(IrType::Isize, 4), (4, 4));

        // A struct with two `usize` fields and an array of `usize`: both resize
        // with the parameter.
        let m = module_of(": w ( [usize 4] -- ) drop ;\ntype: Cursor a usize b usize ;");
        let (s8, _, a8, ..) =
            build_registries_ww(&m.structs, &m.enums, &m.arrays, &m.owned_cells, &m.refs, 8);
        let (s4, _, a4, ..) =
            build_registries_ww(&m.structs, &m.enums, &m.arrays, &m.owned_cells, &m.refs, 4);
        assert_eq!(s8.layouts[0].size, 16, "two usize fields at width 8");
        assert_eq!(s4.layouts[0].size, 8, "two usize fields at width 4");
        assert_eq!(a8.layouts[0].size, 32, "[usize 4] at width 8");
        assert_eq!(a4.layouts[0].size, 16, "[usize 4] at width 4");
    }

    #[test]
    fn array_layout_stride_size_align_from_element() {
        // M2: `stride = round_up(elem_size, elem_align)`, `size = count*stride`,
        // `align = elem_align`. An `i64` element: stride 8, size 32, align 8.
        let a = arrays_of(": w ( [i64 4] -- ) drop ;");
        assert_eq!((a.layouts[0].stride, a.layouts[0].size), (8, 32));
        assert_eq!(a.layouts[0].align, 8);
        // A sub-word `u8` element: stride 1, size 3, align 1.
        let b = arrays_of(": w ( [u8 3] -- ) drop ;");
        assert_eq!(
            (b.layouts[0].stride, b.layouts[0].size, b.layouts[0].align),
            (1, 3, 1)
        );
    }

    #[test]
    fn array_layout_nested_array_of_array_sizes_via_registry() {
        // M3: `[[i64 4] 2]` sizes its element (the inner `[i64 4]`, 32 bytes)
        // via the registry: outer stride 32, size 64, align 8.
        let a = arrays_of(": w ( [[i64 4] 2] -- ) drop ;");
        let outer = a.layouts.iter().find(|l| l.name == "[[i64 4] 2]").unwrap();
        assert_eq!((outer.stride, outer.size, outer.align), (32, 64, 8));
    }

    #[test]
    fn carried_slot_bytes_array_is_aligned_aggregate() {
        // R16/M2: a carried array slot occupies its size rounded up to a
        // multiple of 8. `[u8 3]` is 3 bytes, rounding up to one 8-byte cell.
        let a = arrays_of(": w ( [u8 3] -- ) drop ;");
        assert_eq!(
            carried_slot_bytes(
                IrType::Array(ArrayId::from_index(0)),
                &Structs::default(),
                &Enums::default(),
                &a
            ),
            8
        );
    }

    /// P7 slice 3c (R2.2): a carried slice slot is its whole 16-byte
    /// `{ptr, len}` aggregate, not the 8-byte scalar cell `Str` gets. Getting
    /// this wrong shifts every slot above it in the carried buffer.
    #[test]
    fn carried_slot_bytes_slice_is_aligned_aggregate() {
        let mut slices = Vec::new();
        let slice = match ir_type_of(crate::ast::intern_slice_type(&mut slices, Type::I64, false)) {
            IrType::Slice(id) => IrType::Slice(id),
            other => panic!("expected an IrType::Slice, got {other:?}"),
        };
        assert_eq!(
            carried_slot_bytes(
                slice,
                &Structs::default(),
                &Enums::default(),
                &Arrays::default()
            ),
            16
        );
        // ...and the figure comes from `slice_layout`, which is two words
        // wide, so it is 16 for the same reason a two-`i64` struct is.
        assert_eq!(slice_layout(WORD_WIDTH).size, 16);
    }

    /// P7 slice 3c (R2.1): the per-`SliceId` registry carries what lowering
    /// needs to reach *behind* the view -- the element `IrType` an indexed
    /// element reference records, and the element stride `&>`/`subslice`
    /// offset by. The stride must be the same figure `ArrayLayout` computes
    /// for the very buffer the view is cut from, or an index walks off-pitch;
    /// the struct element is what makes that non-trivial (a scalar's stride is
    /// its width either way).
    #[test]
    fn build_slices_records_element_type_and_array_matching_stride() {
        let src = "type: Pair a i64 b i64 ;\n: f ( [Pair 4] -- ) drop ;\n: main ( -- ) ;\n";
        let tokens = crate::lexer::lex(src).unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        let (structs, enums, arrays, cells, refs) = build_registries(
            &module.structs,
            &module.enums,
            &module.arrays,
            &module.owned_cells,
            &module.refs,
        );
        let _ = (cells, refs);
        let pair = module
            .structs
            .iter()
            .position(|s| s.name == "Pair")
            .expect("Pair is declared");
        let mut decls = Vec::new();
        crate::ast::intern_slice_type(&mut decls, module.arrays[0].element, false);
        let slices = build_slices(&decls, &structs, &enums, &arrays);
        assert_eq!(
            slices.elem,
            vec![IrType::Struct(StructId::from_index(pair))]
        );
        assert_eq!(slices.stride, vec![arrays.layouts[0].stride]);
        assert_eq!(slices.stride, vec![16]);
        assert_eq!(slices.mutable, vec![false]);
    }

    /// P7 slice 3c (R2.1): the scalar sizer refuses a slice rather than
    /// guessing. It cannot answer one: its `(bytes, bytes)` contract would
    /// report a 16-byte align for a value aligned to a word, and the *tempting*
    /// wrong answer (`Str`'s single opaque word) silently drops the length.
    #[test]
    #[should_panic(expected = "a slice value resolves via `slice_layout`, not a scalar")]
    fn scalar_size_align_refuses_a_slice() {
        let mut slices = Vec::new();
        let slice = ir_type_of(crate::ast::intern_slice_type(&mut slices, Type::I64, false));
        scalar_size_align(slice);
    }

    #[test]
    fn struct_layout_flat_i64_fields_offsets_and_size() {
        let s = structs_of("type: Vec2 x i64 y i64 ;");
        let v = layout(&s, "Vec2");
        assert_eq!(v.size, 16);
        assert_eq!(v.align, 8);
        assert_eq!(v.fields[0].offset, 0);
        assert_eq!(v.fields[1].offset, 8);
    }

    #[test]
    fn struct_layout_packed_subword_fields_natural_alignment() {
        // Two `i8`s pack at 0 and 1; the `i64` aligns to 8; whole size 16.
        let s = structs_of("type: Packed p i8 q i8 r i64 ;");
        let p = layout(&s, "Packed");
        assert_eq!(
            (p.fields[0].offset, p.fields[1].offset, p.fields[2].offset),
            (0, 1, 8)
        );
        assert_eq!((p.size, p.align), (16, 8));
    }

    #[test]
    fn struct_layout_nested_uses_inner_size_and_align() {
        let s = structs_of("type: Vec2 x i64 y i64 ; type: Segment from Vec2 to Vec2 ;");
        let seg = layout(&s, "Segment");
        assert_eq!((seg.fields[0].offset, seg.fields[1].offset), (0, 16));
        assert_eq!((seg.size, seg.align), (32, 8));
    }

    #[test]
    fn struct_layout_zero_field_is_size_0_align_1() {
        let s = structs_of("type: Unit ;");
        let u = layout(&s, "Unit");
        assert_eq!((u.size, u.align), (0, 1));
        assert!(u.fields.is_empty());
    }

    #[test]
    fn enum_layout_tag_first_payload_at_max_variant_align() {
        // R13/M1: an i32 tag at offset 0, the payload rounded up to the
        // largest variant's align (8, for the f64 fields), so the tag's 4
        // trailing bytes are padding; size = payload_offset(8) + max payload
        // (Rect's two f64s = 16) = 24; align 8.
        let e = enums_of("type: Shape | Circle r f64 | Rect w f64 h f64 ;");
        let s = enum_layout(&e, "Shape");
        assert_eq!(s.tag_offset, 0);
        assert_eq!(
            s.tag_ty,
            IrType::Int {
                bits: 32,
                signed: true
            }
        );
        assert_eq!(s.payload_offset, 8);
        assert_eq!((s.size, s.align), (24, 8));
        // Circle: one f64 at payload-relative 0; Rect: two f64s at 0 and 8.
        assert_eq!(s.variants[0].fields[0].offset, 0);
        assert_eq!(
            (
                s.variants[1].fields[0].offset,
                s.variants[1].fields[1].offset
            ),
            (0, 8)
        );
    }

    #[test]
    fn zero_payload_enum_lowers_to_scalar_discriminant() {
        // R1 (D-A): the general rule -- any enum whose every variant carries
        // an empty payload lowers to a bare 1-byte scalar discriminant, no
        // payload region, no memory aggregate. Exercised on a *non-`Bool`*
        // enum, so this proves the rule is general, not a `Bool` carve-out.
        let e = enums_of("type: Dir | N | E | S | W ;");
        let d = enum_layout(&e, "Dir");
        assert!(d.is_scalar);
        assert_eq!(d.payload_offset, 0);
        assert_eq!((d.size, d.align), (1, 1));
        assert_eq!(d.variants.len(), 4);
        assert!(d.variants.iter().all(|v| v.fields.is_empty()));
    }

    #[test]
    fn payload_bearing_enum_layout_unchanged() {
        // R1: an enum with at least one payload-bearing variant keeps the
        // pre-existing tagged-aggregate layout untouched by the scalar rule.
        let e = enums_of("type: Shape | Circle r f64 | Rect w f64 h f64 ;");
        let s = enum_layout(&e, "Shape");
        assert!(!s.is_scalar);
        assert_eq!(s.payload_offset, 8);
        assert_eq!((s.size, s.align), (24, 8));
    }

    #[test]
    fn enum_layout_mixed_variant_field_widths_pack_within_payload() {
        // A variant with sub-word + i64 fields packs at natural alignment
        // within the payload; the largest variant sizes the payload.
        let e = enums_of("type: E | A x i8 y i64 | B v i16 ;");
        let s = enum_layout(&e, "E");
        // A: i8 at 0, i64 aligned to 8 -> offset 8, variant size 16, align 8.
        assert_eq!(
            (
                s.variants[0].fields[0].offset,
                s.variants[0].fields[1].offset
            ),
            (0, 8)
        );
        // payload align 8 (A's i64), payload_offset 8, max payload 16, size 24.
        assert_eq!(s.payload_offset, 8);
        assert_eq!((s.size, s.align), (24, 8));
    }

    #[test]
    fn enum_layout_nested_struct_payload_sized_via_combined_registry() {
        // D9: a variant field of struct type is sized via its layout (16 for a
        // two-f64 Vec2), not `scalar_size_align`.
        let (structs, enums, _arrays, _cells, _refs) = {
            let src = "type: Vec2 x f64 y f64 ; type: Shape | Dot p Vec2 | Unit ;";
            let tokens = lex(src).unwrap();
            let mut module = parse(&tokens).unwrap();
            check(&mut module).unwrap();
            build_registries(
                &module.structs,
                &module.enums,
                &module.arrays,
                &module.owned_cells,
                &module.refs,
            )
        };
        let _ = structs;
        let s = enum_layout(&enums, "Shape");
        // Dot's Vec2 payload: 16 bytes at payload-relative 0; payload align 8.
        assert_eq!(s.variants[0].fields[0].size, 16);
        assert_eq!(s.payload_offset, 8);
        assert_eq!((s.size, s.align), (24, 8));
    }

    #[test]
    fn struct_field_of_enum_type_sized_via_combined_registry() {
        // D9: a struct field of enum type is sized via the enum's layout, not
        // `scalar_size_align`; the struct places the next field past it.
        let (structs, _enums, _arrays, _cells, _refs) = {
            let src =
                "type: Shape | Circle r f64 | Rect w f64 h f64 ; type: Tagged k Shape n i64 ;";
            let tokens = lex(src).unwrap();
            let mut module = parse(&tokens).unwrap();
            check(&mut module).unwrap();
            build_registries(
                &module.structs,
                &module.enums,
                &module.arrays,
                &module.owned_cells,
                &module.refs,
            )
        };
        let t = layout(&structs, "Tagged");
        // Shape is 24 bytes align 8: k at 0 (size 24), n (i64) at 24; size 32.
        assert_eq!((t.fields[0].offset, t.fields[0].size), (0, 24));
        assert_eq!(t.fields[1].offset, 24);
        assert_eq!((t.size, t.align), (32, 8));
    }

    #[test]
    fn carried_slot_bytes_enum_is_aligned_aggregate() {
        // R17: a carried enum slot occupies its size rounded up to a multiple
        // of 8. Shape is 24 bytes (already a multiple of 8); a tag-only enum
        // (4 bytes pre-Slice-9, now a 1-byte scalar) rounds up to one 8-byte
        // cell either way. `enums_of` parses through the full pipeline, so
        // `bool` occupies the reserved index 0 (Slice 9, R2) ahead of the
        // source's own `Shape`/`Dir`.
        let e = enums_of("type: Shape | Circle r f64 | Rect w f64 h f64 ; type: Dir | N | S ;");
        assert_eq!(
            carried_slot_bytes(
                IrType::Enum(EnumId::from_index(1)),
                &Structs::default(),
                &e,
                &Arrays::default()
            ),
            24
        );
        assert_eq!(
            carried_slot_bytes(
                IrType::Enum(EnumId::from_index(2)),
                &Structs::default(),
                &e,
                &Arrays::default()
            ),
            8
        );
    }

    // Phase 3 Slice 1, Phase 2: struct linearity + the synthesized destructor
    // (R7/R9/R11/R12).

    #[test]
    fn struct_layout_is_linear_iff_a_field_is_transitively() {
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Plain x i64 y i64 ; \
             type: Holds a Spy b i64 ; \
             type: Wraps h Holds ; \
             : w ( -- ) ;"
        ));
        assert!(ir.structs[0].is_linear, "Spy has a drop overload");
        assert!(!ir.structs[1].is_linear, "Plain has no linear field");
        assert!(ir.structs[2].is_linear, "Holds carries a Spy directly");
        assert!(ir.structs[3].is_linear, "Wraps carries one transitively");
    }

    #[test]
    fn struct_with_owned_cell_field_is_linear_and_pointer_sized() {
        // R4/R17: a cell is linear whatever its payload, so a struct holding one
        // is linear and gets drop glue; its field is a pointer, sized by the
        // same convention as `Ptr` rather than a second width assumption.
        let ir = lower_src("type: Boxed b ^i64 ; : w ( -- ) ;");
        let layout = &ir.structs[0];
        assert!(layout.is_linear, "a cell field makes its struct linear");
        assert_eq!((layout.size, layout.align), (8, 8));
        assert!(
            matches!(layout.fields[0].ty, IrType::OwnedCell(_)),
            "a cell field keeps its own `IrType`, not a bare `Ptr`: {:?}",
            layout.fields[0].ty
        );
        assert_eq!(scalar_size_align(layout.fields[0].ty), (8, 8));
    }

    #[test]
    fn struct_linearity_agrees_across_the_checker_and_both_lowering_folds() {
        // Linearity is decided in three places over the same field lists:
        // `check::is_copy` walks `Type`, `ensure_struct` folds `IrType` inline
        // while `layouts` is still being built, and `field_is_linear` is what
        // every drop-glue site consults. If they ever disagree the checker
        // gates a `dup` the lowering then emits no glue for (or the reverse),
        // so pin all three rather than trusting three hand-kept matches.
        let src = format!(
            "{SPY_DEF}type: Plain x i64 y i64 ; \
                   type: Holds a Spy b i64 ; \
                   type: Wraps h Holds ; \
                   type: Deep w Wraps p Plain ; \
                   type: Item | Empty | Full v Spy ; \
                   type: EnumInStruct e Item ; \
                   type: StructInEnum | Some h Holds | None ; \
                   type: EnumInEnum | Inner i EnumInStruct | Outer ; \
                   type: PlainArr xs [i64 4] ; \
                   type: Boxed b ^i64 ; \
                   type: BoxedPlain p ^Plain ; \
                   type: MaybeBoxed | Full b ^i64 | Empty ; \
                   : w ( -- ) ;"
        );
        let tokens = lex(&src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        // `SpyArr` (a `[Spy 4]` field) is spliced in directly rather than
        // through source: Item 1's array-type-use rejection means no source
        // program can spell this declaration any more, but the predicate
        // must still be correct on the type alone. Reuses the real `Spy`
        // struct from `SPY_DEF` (already `has_drop_overload`, set by `check`
        // above) rather than hand-building a fixture, since `SPY_DEF` is
        // always prepended first and so is always struct index 0.
        let spy_id = StructId::from_index(0);
        let spy_name_static = module.structs[spy_id.index()].name_static;
        let spy_ty = Type::Struct(spy_id, spy_name_static);
        let spy_array_id = ArrayId::from_index(module.arrays.len());
        let spy_array_name: &'static str = "[Spy 4]";
        module.arrays.push(ArrayDecl {
            element: spy_ty,
            count: 4,
            name_static: spy_array_name,
        });
        module.structs.push(StructDecl {
            name: "SpyArr".to_string(),
            name_static: "SpyArr",
            fields: vec![("xs".to_string(), Type::Array(spy_array_id, spy_array_name))],
            span: crate::ast::Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        });
        let (structs, enums, arrays, ..) = build_registries(
            &module.structs,
            &module.enums,
            &module.arrays,
            &module.owned_cells,
            &module.refs,
        );
        for (idx, layout) in structs.layouts.iter().enumerate() {
            let ty = Type::Struct(StructId::from_index(idx), layout.name);
            assert_eq!(
                crate::check::is_copy(ty, &module.structs, &module.enums, &module.arrays),
                !layout.is_linear,
                "`{}`: checker says Copy={}, `ensure_struct` says linear={}",
                layout.name,
                crate::check::is_copy(ty, &module.structs, &module.enums, &module.arrays),
                layout.is_linear
            );
            // `Spy` itself is excluded here: it is linear purely because of
            // its `has_drop_overload` bit (an override on all-Copy fields),
            // not because any field is `field_is_linear`, a distinct case
            // already pinned by
            // `ir_registers_overridden_struct_as_linear_despite_all_copy_fields`.
            if idx != spy_id.index() {
                assert_eq!(
                    layout
                        .fields
                        .iter()
                        .any(|f| field_is_linear(f.ty, &structs, &enums, &arrays)),
                    layout.is_linear,
                    "`{}`: `field_is_linear` disagrees with the `ensure_struct` fold",
                    layout.name
                );
            }
        }
        // R7/R12 (Phase 4): the same three-way pin, over the enum registry's
        // `Type::Enum` arm of `is_copy` and the variant-payload fold
        // (`ensure_enum`/`layout_field_is_linear`), including transitivity
        // through a struct-in-enum and an enum-in-enum.
        for (idx, layout) in enums.layouts.iter().enumerate() {
            let ty = Type::Enum(EnumId::from_index(idx), layout.name);
            assert_eq!(
                crate::check::is_copy(ty, &module.structs, &module.enums, &module.arrays),
                !layout.is_linear,
                "`{}`: checker says Copy={}, `ensure_enum` says linear={}",
                layout.name,
                crate::check::is_copy(ty, &module.structs, &module.enums, &module.arrays),
                layout.is_linear
            );
            assert_eq!(
                layout
                    .variants
                    .iter()
                    .flat_map(|v| v.fields.iter())
                    .any(|f| field_is_linear(f.ty, &structs, &enums, &arrays)),
                layout.is_linear,
                "`{}`: `field_is_linear` disagrees with the `ensure_enum` fold",
                layout.name
            );
        }
        // Criterion (item 3): an array field is linear iff its element is,
        // transitively; `PlainArr` (an `[i64 4]` field) stays Copy, `SpyArr`
        // (a `[Spy 4]` field, spliced in above) is linear even though no
        // source program can declare that field any more, so the predicate
        // must be correct on the type alone.
        let plain_arr_idx = structs
            .layouts
            .iter()
            .position(|l| l.name == "PlainArr")
            .unwrap();
        let spy_arr_idx = structs
            .layouts
            .iter()
            .position(|l| l.name == "SpyArr")
            .unwrap();
        assert!(!structs.layouts[plain_arr_idx].is_linear);
        assert!(structs.layouts[spy_arr_idx].is_linear);
        let plain_arr_ty = Type::Struct(
            StructId::from_index(plain_arr_idx),
            structs.layouts[plain_arr_idx].name,
        );
        let spy_arr_ty = Type::Struct(
            StructId::from_index(spy_arr_idx),
            structs.layouts[spy_arr_idx].name,
        );
        assert!(crate::check::is_copy(
            plain_arr_ty,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        assert!(!crate::check::is_copy(
            spy_arr_ty,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
    }
}
