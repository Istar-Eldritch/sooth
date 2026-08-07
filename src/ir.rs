//! Backend-neutral IR.
//!
//! The compile-time virtual stack is lowered to SSA-shaped values here, and each
//! word becomes a function taking N inputs and returning M outputs. Control words
//! become basic blocks and branches. This IR feeds QBE today and a WASM sibling
//! lowering later, so it stays neutral: in particular `Ptr` is an opaque handle,
//! never assumed to be a native `u64`, so QBE (native pointers) and WASM
//! (linear-memory offsets) can each concretise it.

use std::collections::HashMap;
use std::mem;

use crate::ast::{
    ArrayDecl, ArrayId, CallInst, Clause, EnumDecl, EnumId, Len, Module, OwnedCellDecl,
    OwnedCellId, PolySig, PolyType, RefDecl, Span, StackEffect, StructDecl, StructId, Subst, Term,
    TermKind, Type, TypedSlot, WordBody, WordDef,
};

/// The single target word-width parameter (R15, M2): the byte width of a
/// target machine word, from which `usize` size/align and every array/aggregate
/// offset that embeds a `usize` derive. It is `8` for the QBE/x86-64 target
/// today; `Ptr` retrofits to the same parameter in Slice 7. Every layout path
/// routes through this rather than a literal `8`, so a re-target is one edit
/// (criterion 2's structural test flips it to prove no stray literal remains).
pub const WORD_WIDTH: u32 = 8;

/// The runtime out-of-bounds trap helper (R19/D6): Sooth's first runtime
/// failure path. A dynamic array index that fails its `index < N` guard calls
/// this symbol, which prints a located len+index message to stderr and exits
/// nonzero. The backend emits the definition; the IR references it by name so
/// both sides agree on one symbol.
pub const OOB_TRAP_SYMBOL: &str = "sooth_oob_trap";

/// The heap allocator's acquire half: `allocate(n) -> ptr`, a compiler-emitted
/// shim over `malloc` that traps on a NULL return and requests `max(n, 1)`
/// bytes. The language never sees libc, only this interface.
pub const ALLOC_SYMBOL: &str = "sooth_alloc";

/// The heap allocator's release half: `free(ptr, n)`. The size is not needed
/// by `free` itself; it is what the allocation trace reports.
pub const FREE_SYMBOL: &str = "sooth_free";

/// The environment variable gating the allocation trace. Unset or empty
/// prints nothing, since a real program using `^` must stay silent by
/// default.
pub const TRACE_ALLOC_ENV: &str = "SOOTH_TRACE_ALLOC";

#[derive(Debug, Default)]
pub struct IrModule {
    pub funcs: Vec<IrFunc>,
    /// Per-struct memory layout, indexed by `StructId`. The backend emits
    /// a `type :S = { … }` per entry and reads field offsets/widths from it;
    /// empty for a struct-free module (or a single-func REPL emit).
    pub structs: Vec<StructLayout>,
    /// Per-enum tagged layout, indexed by `EnumId`. The backend emits an
    /// opaque byte-blob `type :E = align A { b N }` per entry (D3, R15) and
    /// reads tag/payload offsets from it; empty for an enum-free module.
    pub enums: Vec<EnumLayout>,
    /// Per-array element layout, indexed by `ArrayId`. The backend emits an
    /// opaque byte-blob `type :A = align N { b S }` per entry (R20) and reads
    /// element stride from it; empty for an array-free module.
    pub arrays: Vec<ArrayLayout>,
}

#[derive(Debug)]
pub struct IrFunc {
    pub name: String,
    pub params: Vec<IrType>,
    pub ret: Option<IrType>,
    pub blocks: Vec<Block>,
    /// The `IrType` of each SSA value in the function, indexed by `Value.0`.
    pub value_types: Vec<IrType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrType {
    /// A fixed-width integer carrying its `bits` and `signed`. The backend
    /// derives the QBE register class (`w`/`l`) and signed-vs-unsigned op from
    /// these; the IR itself stays backend-neutral (a WASM lowering reads
    /// `bits`/`signed`, never `w`/`l`).
    Int {
        bits: u8,
        signed: bool,
    },
    /// A float carrying its `bits` (32/64). The backend derives the QBE
    /// register class (`s`/`d`); the IR itself never spells it (a WASM lowering
    /// reads `bits`, R13/NF2). Floats fill their register exactly, so no
    /// sub-word canonicalization ever applies.
    Float {
        bits: u8,
    },
    Bool,
    /// A user-declared struct, keyed by a small `Copy` `StructId` into the
    /// module's `StructLayout` registry; the layout (offsets/size/align) lives
    /// there, not inlined, so `IrType` stays `Copy`. At runtime a struct value
    /// is a pointer to its aggregate storage; the backend spells it `:S` in
    /// ABI positions (params/returns/call args) and `l` (a pointer) in a
    /// register.
    Struct(StructId),
    /// A user-declared enum (sum type), keyed by a small `Copy` `EnumId` into
    /// the module's `EnumLayout` registry; the tagged layout lives there, not
    /// inlined, so `IrType` stays `Copy`. At runtime an enum value is a
    /// pointer to its aggregate storage (a tag + a max-variant payload); the
    /// backend spells it `:E` in ABI positions and `l` (a pointer) in a
    /// register, exactly like `Struct`.
    Enum(EnumId),
    /// A fixed-size array (D3), keyed by a small `Copy` `ArrayId` into the
    /// module's `ArrayLayout` registry; the element stride/size/align live
    /// there, not inlined, so `IrType` stays `Copy`. At runtime an array value
    /// is a pointer to its inline aggregate storage, spelled `:A` in ABI
    /// positions and `l` (a pointer) in a register, exactly like `Struct`.
    Array(ArrayId),
    /// The target-width unsigned integer (D7): its size/align come from the
    /// `WORD_WIDTH` parameter, never a hardcoded literal. The backend derives
    /// its register class (`l` today) and unsigned ops the same way it does for
    /// a `u64`, but the *width* flows from the parameter (R15).
    Usize,
    /// The target-width *signed* integer, mirroring `Usize`: same
    /// word-width-derived size/align (`norm_scalar_ww`), but the backend
    /// derives its ops and printing from a signed `i64`-like type rather than
    /// an unsigned `u64`-like one.
    Isize,
    /// Opaque handle (backend-neutral-IR invariant): a native pointer under QBE,
    /// a linear-memory offset under a future WASM lowering. Used by the line
    /// wrapper's `%stack` parameter.
    Ptr,
    /// An owning heap cell `^T`, keyed by the `OwnedCellId` of its interned
    /// payload shape. A pointer everywhere the backend touches it, but
    /// distinct from `Ptr` in the IR: `drop` dispatches on a value's
    /// `IrType`, and dispatch must not key off a bare pointer.
    OwnedCell(OwnedCellId),
    /// `str` (R4): the opaque address of a static `{ptr, len}` descriptor
    /// (never runtime-allocated, R11), so at runtime it is one pointer, like
    /// `Ptr`, but distinct in the IR so `.`/`len`/`cstr` can dispatch on it.
    Str,
    /// `cstr` (R5): a bare NUL-terminated byte pointer. Identical at runtime
    /// to `Ptr`, distinct in the IR for the same dispatch reason as `Str`.
    Cstr,
}

impl IrType {
    /// The `i64` integer type; the literal type and the carried-slot width.
    pub const I64: IrType = IrType::Int {
        bits: 64,
        signed: true,
    };
}

/// Map a frontend `Type` to its `IrType`.
pub fn ir_type_of(ty: Type) -> IrType {
    match ty {
        Type::Int(it) => IrType::Int {
            bits: it.bits(),
            signed: it.signed(),
        },
        Type::Float(ft) => IrType::Float { bits: ft.bits() },
        Type::Bool => IrType::Bool,
        // The layout lives in the module's `StructLayout` registry; the
        // `IrType` carries only the `StructId` so it stays `Copy`.
        Type::Struct(id, _) => IrType::Struct(id),
        // The tagged layout lives in the module's `EnumLayout` registry; the
        // `IrType` carries only the `EnumId` so it stays `Copy`.
        Type::Enum(id, _) => IrType::Enum(id),
        // The element stride/size lives in the module's `ArrayLayout`
        // registry; the `IrType` carries only the `ArrayId` so it stays `Copy`.
        Type::Array(id, _) => IrType::Array(id),
        // The payload shape lives in the module's owning-cell registry; the
        // `IrType` carries only the `OwnedCellId` so it stays `Copy`.
        Type::OwnedCell(id, _) => IrType::OwnedCell(id),
        // A reference is always the opaque handle, never the referent's
        // own aggregate type. QBE's C-ABI classification passes a `:Buf`-
        // spelled parameter *by value*, so a `&!Buf` mapped to
        // `IrType::Struct` would have a callee mutating a caller-side
        // temporary. The referent shape the lowerer needs for a projection or
        // an access is tracked per-`Value` (`FuncBuilder::ref_inner`), not in
        // the type.
        Type::Ref(..) => IrType::Ptr,
        Type::Usize => IrType::Usize,
        Type::Isize => IrType::Isize,
        Type::Str => IrType::Str,
        Type::Cstr => IrType::Cstr,
        // Slice 6a (R7): a quotation type has no runtime representation this
        // slice (D6). A quotation-taking word mints no standalone `IrFunc`
        // (R20) and is inlined at every call site, so its declared effect
        // never reaches the backend; the audit (R7a) rejects a quotation type
        // at every position that would layout or lower one before this is
        // reached. Slice 7 lifts this with a `(code, env)` runtime value.
        Type::Quotation(_) => {
            unreachable!("a quotation type has no IrType this slice (R7/R7a/R20; slice 7)")
        }
    }
}

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
fn field_is_linear(ty: IrType, structs: &Structs, enums: &Enums, arrays: &Arrays) -> bool {
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
fn struct_drop_symbol(id: StructId, epoch: Option<u64>) -> String {
    match epoch {
        Some(g) => format!("sooth_struct_drop_{}__gen{g}", id.index()),
        None => format!("sooth_struct_drop_{}", id.index()),
    }
}

/// The synthesized per-type destructor symbol for a linear enum: mirrors
/// `struct_drop_symbol`, one uniform naming scheme for both aggregate kinds.
fn enum_drop_symbol(id: EnumId, epoch: Option<u64>) -> String {
    match epoch {
        Some(g) => format!("sooth_enum_drop_{}__gen{g}", id.index()),
        None => format!("sooth_enum_drop_{}", id.index()),
    }
}

/// Mirrors `struct_drop_symbol`/`enum_drop_symbol`, one uniform naming
/// scheme across all three kinds.
fn cell_drop_symbol(id: OwnedCellId, epoch: Option<u64>) -> String {
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
pub enum StructWord {
    Construct(StructId),
    Get(StructId, usize),
    Set(StructId, usize),
    Destructure(StructId),
    /// `S|>fi` (R10): a non-consuming `( S -- S field )` peek, Copy fields
    /// only (the checker rejects a linear field before this is ever reached).
    Peek(StructId, usize),
}

/// The IR's view of a program's structs: the per-`StructId` layout registry and
/// the generated-word name map (`S`/`S>`/`S>fi`/`S<fi`/`S|>fi` → `StructWord`). Built
/// once from the module and threaded into lowering; empty for a struct-free
/// program (the scalar paths never consult it).
#[derive(Debug, Default)]
pub struct Structs {
    pub layouts: Vec<StructLayout>,
    pub words: HashMap<String, StructWord>,
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
/// the variant's declaration index. Enums have no getter/setter/destructure
/// (D2: elimination is clause-style, Phase 4).
#[derive(Debug, Clone, Copy)]
pub enum EnumWord {
    Construct(EnumId, usize),
}

/// The IR's view of a program's enums: the per-`EnumId` tagged-layout registry
/// and the variant-constructor name map (variant name → `EnumWord`). A
/// logically distinct registry from `Structs` (D10), built alongside it by
/// `build_registries`; empty for an enum-free program.
#[derive(Debug, Default)]
pub struct Enums {
    pub layouts: Vec<EnumLayout>,
    pub words: HashMap<String, EnumWord>,
}

fn round_up(offset: u32, align: u32) -> u32 {
    offset.div_ceil(align) * align
}

/// The size/align of a scalar `IrType` at the default target word width
/// (`WORD_WIDTH`). Thin wrapper over `scalar_size_align_ww`; criterion 2's
/// structural test calls the `_ww` form directly with a flipped width to prove
/// `usize` sizing derives from the parameter, not a stray literal.
fn scalar_size_align(ty: IrType) -> (u32, u32) {
    scalar_size_align_ww(ty, WORD_WIDTH)
}

/// The size/align of a scalar `IrType`, `usize` sized from the supplied
/// `word_width` (R15): `i8`/`u8`/`bool` = 1, `i16`/`u16` = 2, `i32`/`u32`/`f32`
/// = 4, `i64`/`u64`/`f64` = 8, `usize` = `word_width`. A `Ptr` is 8 (unused as
/// a field this slice). Never called on a `Struct`/`Enum`/`Array` (nested
/// aggregates resolve through the layout registry).
fn scalar_size_align_ww(ty: IrType, word_width: u32) -> (u32, u32) {
    let bytes = match ty {
        IrType::Bool => 1,
        IrType::Int { bits, .. } => (bits / 8) as u32,
        IrType::Float { bits } => (bits / 8) as u32,
        IrType::Usize => word_width,
        IrType::Isize => word_width,
        // A cell is a pointer, so its width defers to `Ptr`'s convention.
        IrType::Ptr | IrType::OwnedCell(_) | IrType::Str | IrType::Cstr => 8,
        IrType::Struct(_) => unreachable!("a struct field resolves via the layout registry"),
        IrType::Enum(_) => unreachable!("an enum field resolves via the layout registry"),
        IrType::Array(_) => unreachable!("an array field resolves via the layout registry"),
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
        IrType::Int { .. }
        | IrType::Float { .. }
        | IrType::Bool
        | IrType::Usize
        | IrType::Isize
        | IrType::Ptr
        | IrType::OwnedCell(_)
        | IrType::Str
        | IrType::Cstr => 8,
    }
}

impl Structs {
    /// Build just the struct registry (no enums). A thin wrapper over
    /// `build_registries` for struct-only callers; a struct with an enum field
    /// needs the full `build_registries` (its enums must be present to size
    /// the field, D9).
    pub fn from_structs(structs: &[StructDecl]) -> Structs {
        build_registries(structs, &[], &[], &[], &[]).0
    }

    /// R10: the synthesized bundle struct a word with these declared outputs
    /// returns, or `None` when none was interned for the tuple — a word with
    /// fewer than two outputs, or any registry the checker never interned into
    /// (the REPL's), which then keeps its pre-slice single-value lowering.
    pub fn bundle_for(&self, outputs: &[Type]) -> Option<StructId> {
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

/// The four registries bundled as one `Copy` handle, so lowering and
/// destructor synthesis pass one argument instead of four (mirrors the
/// backend's `Layouts`). The registries stay logically separate types; this
/// only co-locates references to them.
#[derive(Debug, Clone, Copy)]
pub struct Registries<'a> {
    pub structs: &'a Structs,
    pub enums: &'a Enums,
    pub arrays: &'a Arrays,
    pub cells: &'a Cells,
    pub refs: &'a Refs,
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
pub fn build_registries_ww(
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
    for (idx, decl) in structs.iter().enumerate().filter(|(_, d)| !d.is_bundle) {
        let id = StructId::from_index(idx);
        swords.insert(decl.name.clone(), StructWord::Construct(id));
        swords.insert(format!("{}>", decl.name), StructWord::Destructure(id));
        for (fi, (fname, _)) in decl.fields.iter().enumerate() {
            swords.insert(format!("{}>{}", decl.name, fname), StructWord::Get(id, fi));
            swords.insert(format!("{}<{}", decl.name, fname), StructWord::Set(id, fi));
            swords.insert(
                format!("{}|>{}", decl.name, fname),
                StructWord::Peek(id, fi),
            );
        }
    }

    let mut ewords = HashMap::new();
    for (idx, decl) in enums.iter().enumerate() {
        let id = EnumId::from_index(idx);
        for (vi, variant) in decl.variants.iter().enumerate() {
            ewords.insert(variant.name.clone(), EnumWord::Construct(id, vi));
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value(pub u32);

/// R12: an index into `FuncBuilder::quot_defs`, the per-function table of
/// quotation-literal bodies. A quotation lowers to a phantom `Value` (no
/// defining `Instr`) mapped to its `QuotId`; `call`/`times` splice the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QuotId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockId(pub u32);

#[derive(Debug)]
pub struct Block {
    pub id: BlockId,
    pub instrs: Vec<Instr>,
    pub term: Terminator,
}

#[derive(Debug)]
pub enum Instr {
    Const(Value, i64),
    /// A float constant carrying its `f64` value. Distinct from `Const`
    /// so the backend emits a QBE float constant rather than reinterpreting an
    /// integer bit-payload; the `Value`'s `IrType` picks the `s`/`d` register.
    ConstF(Value, f64),
    Bin(Value, BinOp, Value, Value),
    Cmp(Value, CmpOp, Value, Value),
    Call(Option<Value>, String, Vec<Value>),
    /// `.`: print one value followed by a newline. Type-directed at the
    /// backend (not here, IR stays neutral): the value's own `IrType` (looked
    /// up via `value_types`) picks signed/unsigned decimal, `%g` float, or
    /// `true`/`false`, the same way `Cmp`/`Shr` dispatch on operand type.
    Print(Value),
    Phi(Value, Vec<(BlockId, Value)>),
    /// `dst: Ptr = base + bytes`. Keeps `Ptr` opaque (no native-width assumption).
    PtrOffset(Value, Value, i64),
    /// `dst: Ptr = base + index*stride` (R17): the dynamic element-addressing
    /// op. `base` is an aggregate `Value`, `index` a runtime `usize` `Value`,
    /// `stride` the compile-time constant from `ArrayLayout`. Yields an opaque
    /// element place; keeps `Ptr` opaque (no pointer-as-`u64` arithmetic in the
    /// IR), the backend concretises `base + index*stride`.
    ElemAddr(Value, Value, Value, i64),
    /// `dst: Int = *ptr`.
    Load(Value, Value),
    /// `*ptr = val` (Int).
    Store(Value, Value),
    /// `dst: Struct = alloc(size, align)`: a frame-local aggregate slot.
    /// The two operands are the whole-struct byte size and alignment from the
    /// layout registry.
    Alloc(Value, u32, u32),
    /// `blit src -> dst, size`: copy `size` bytes between two aggregate
    /// pointers — the byte-copy `dup`, the setter's copy-all, and a
    /// nested-struct field store.
    Blit(Value, Value, u32),
    /// `dst = *ptr` at the field's exact width, the load op picked from
    /// `dst`'s scalar `IrType` (`loadsb`/`loadub`/`loadsh`/…). Distinct from the
    /// 8-byte-slot `Load` so a field read never over-reads its neighbour.
    FieldLoad(Value, Value),
    /// `*ptr = val` at `val`'s exact width, the store op picked from
    /// `val`'s scalar `IrType` (`storeb`/`storeh`/`storew`/`storel`/…).
    /// Distinct from the 8-byte-slot `Store` so a field write never clobbers
    /// its neighbour.
    FieldStore(Value, Value),
    /// `dst = convert(src)` between two integer types (`>iN`/`>uN`). The two
    /// `IrType`s carry the widths and signedness the backend needs to pick
    /// sign/zero-extend (widen), truncate-and-canonicalize (narrow), or relabel
    /// (same width); the frontend never spells the QBE op (R14).
    Conv(Value, Value),
    /// `dst: Str = &<static descriptor for this literal's content>` (R6): the
    /// backend emits the byte data (trailing NUL not counted in length, which
    /// is what gives a literal-rooted `str` a terminator for R7 to rely on)
    /// and the `{ptr, len}` descriptor once per distinct content, and takes the
    /// descriptor's address here.
    StrLit(Value, String),
    /// `dst: Usize = src`'s carried length (R8). States intent rather than an
    /// offset: the descriptor's byte layout is decided once, in the backend,
    /// by `emit_str_literal`, which is also the only place that then needs to
    /// know where the length word sits (keeps `Ptr[T]` opaque here).
    StrLen(Value, Value),
    /// `dst: Cstr = src`'s bytes pointer, discarding the length (R7). Mirrors
    /// `StrLen`: no offset spelled here, for the same reason.
    StrPtr(Value, Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    /// Float division (`/`); present only for float operands (there is no
    /// integer `/`, checker-guaranteed, R16).
    Div,
    Rem,
    And,
    Or,
    Xor,
    /// Left shift; the rhs is always an `i64` shift count regardless of the
    /// lhs's integer width (checker-guaranteed).
    Shl,
    /// Right shift; the backend derives logical vs arithmetic from the
    /// result's signedness, same pattern as `CmpOp` deriving signed vs
    /// unsigned from the operand type. The rhs is always an `i64` count.
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Lt,
    Gt,
    Le,
    Ge,
    Ne,
}

#[derive(Debug)]
pub enum Terminator {
    Ret(Option<Value>),
    Jnz(Value, BlockId, BlockId),
    Jmp(BlockId),
}

/// Declared signature of a user word or `extern:` declaration: (input count,
/// output count, output `IrType` if any). The build path derives this from
/// declared slot types; the REPL derives it from the checker's typed env. A
/// `None` output type (e.g. a word with no output) is treated as
/// `IrType::Int` by callers.
pub type Arity = (usize, usize, Option<IrType>);

/// Maps a called user-word name to the symbol it is emitted/linked as. The build
/// path uses identity; the REPL supplies generation-mangled symbols so a unit
/// links against the words it was compiled against.
pub type Resolver<'a> = &'a dyn Fn(&str) -> String;

pub fn lower(module: &Module) -> Result<IrModule, String> {
    // R1/R2: recognized here, ahead of `build_registries`, rather than
    // trusted from `StructDecl::has_drop_overload` -- `check::check` sets
    // that bit as a side effect on `module.structs`, but `lower` takes
    // `&Module` and has no way to require that it already ran. Recomputing
    // the registry and forcing the bit on a local copy makes `lower` correct
    // against a module that never went through `check` (layout would
    // otherwise fold the struct non-linear, no destructor would be
    // synthesized, and `overrides` below would silently go unused). The one
    // registry is reused for the layout pass, the `env`/lowering filter, and
    // the override map, so there is a single source of truth for which
    // words are drop overloads.
    let drop_overloads = crate::check::find_drop_overloads(&module.words, &module.structs)?;
    let drop_overload_indices: std::collections::HashSet<usize> =
        drop_overloads.values().copied().collect();
    // R9: a polymorphic word carries no concrete `Sig`, is never called by its
    // plain name (every call site resolves through the R14 instantiation
    // table), and lowers not once but once per recorded instantiation below.
    // So it is excluded from the plain-name env and per-word pass, exactly as
    // a `drop` overload is.
    let poly_indices: std::collections::HashSet<usize> = module
        .words
        .iter()
        .enumerate()
        .filter(|(_, w)| w.poly.is_some())
        .map(|(idx, _)| idx)
        .collect();
    // R11/R14: the fixed input arity of each polymorphic word, name-keyed. A
    // call site pops this many args (the row prefix, if any, stays on the
    // caller's stack, S2); it is constant across a word's instantiations, so
    // it lives here rather than per-`CallInst`.
    let poly_arities: HashMap<String, usize> = module
        .words
        .iter()
        .filter_map(|w| {
            w.poly
                .as_ref()
                .map(|sig| (w.name.clone(), sig.inputs.len()))
        })
        .collect();
    // R20: a monomorphic quotation-taking word (a combinator) mints no
    // standalone `IrFunc`: every call to it is inlined (R19, the splice in
    // `lower_call`), so it is excluded from both the plain-name env and the
    // per-word pass, exactly as a poly word or a `drop` overload is. Its body
    // is registered in `combinator_bodies` so the inliner can splice it.
    let combinator_indices: std::collections::HashSet<usize> = module
        .words
        .iter()
        .enumerate()
        .filter(|(_, w)| crate::check::is_combinator(w))
        .map(|(idx, _)| idx)
        .collect();
    let combinator_bodies: HashMap<String, Vec<Term>> = module
        .words
        .iter()
        .filter(|w| crate::check::is_combinator(w))
        .map(|w| match &w.body {
            WordBody::Terms { terms } => (w.name.clone(), terms.clone()),
            WordBody::Clauses(_) => unreachable!("a combinator is `WordBody::Terms` (R18)"),
        })
        .collect();
    let mut structs_forced: Vec<StructDecl> = module.structs.to_vec();
    for id in drop_overloads.keys() {
        structs_forced[id.index()].has_drop_overload = true;
    }

    let (structs, enums, arrays, cells, refs) = build_registries(
        &structs_forced,
        &module.enums,
        &module.arrays,
        &module.owned_cells,
        &module.refs,
    );
    // R1: a recognized `drop` overload is excluded from the lowering env,
    // same as `check`'s own env (`check.rs::check`): its body is compiled
    // under the struct's destructor symbol, never called by the literal
    // name `"drop"`.
    let mut env: HashMap<String, Arity> = module
        .words
        .iter()
        .enumerate()
        .filter(|(idx, _)| {
            !drop_overload_indices.contains(idx)
                && !poly_indices.contains(idx)
                && !combinator_indices.contains(idx)
        })
        .map(|(_, w)| {
            let ret_ty = word_ret_ty(&w.effect.outputs, &structs);
            (
                w.name.clone(),
                (w.effect.inputs.len(), w.effect.outputs.len(), ret_ty),
            )
        })
        .collect();
    // R1: an `extern:` declaration is registered into the same lowering env
    // as a user word, keyed by its Sooth name, so an ordinary `Instr::Call`
    // covers the call site; only the emitted symbol (R1's declared C symbol)
    // differs.
    let mut extern_symbols: HashMap<String, String> = HashMap::new();
    for decl in &module.externs {
        let ret_ty = decl.effect.outputs.first().map(|slot| ir_type_of(slot.ty));
        env.insert(
            decl.name.clone(),
            (decl.effect.inputs.len(), decl.effect.outputs.len(), ret_ty),
        );
        extern_symbols.insert(decl.name.clone(), decl.symbol.clone());
    }
    let resolve = |name: &str| {
        extern_symbols
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    };
    let regs = Registries {
        structs: &structs,
        enums: &enums,
        arrays: &arrays,
        cells: &cells,
        refs: &refs,
    };

    // R1: a recognized `drop` overload is excluded from this generic
    // per-word lowering pass -- unfiltered, it would compile to a QBE
    // function literally named `drop`, and a second override in the same
    // module would collide with it under the identical symbol. The override's
    // body is instead compiled by `synthesize_aggregate_destructors` (R2)
    // into the struct's own destructor symbol.
    let mut funcs: Vec<IrFunc> = module
        .words
        .iter()
        .enumerate()
        .filter(|(idx, _)| {
            !drop_overload_indices.contains(idx)
                && !poly_indices.contains(idx)
                && !combinator_indices.contains(idx)
        })
        .map(|(_, w)| {
            let self_tail = crate::check::has_self_tail_call(w);
            lower_word_parts(
                &w.name,
                &w.effect,
                &w.body,
                self_tail,
                &env,
                &resolve,
                regs,
                &module.instantiations,
                &poly_arities,
                &combinator_bodies,
            )
        })
        .collect();

    // R9: one monomorphized `IrFunc` per distinct recorded instantiation.
    // Every call site of a polymorphic word wrote a `CallInst` keyed by its
    // span, carrying the symbol the checker minted for its own R14 table entry.
    // `IrFunc.name` here is *not* read from that field: `instantiation_symbol`
    // is called again on `(word, θ)`, the same pure function the checker called,
    // so the emitted symbol and the call site's `Instr::Call` target are two
    // independent computations that can only agree because the function is
    // deterministic, not because one was copied from the other. θ is ground,
    // so the substituted effect carries concrete array types with concrete
    // `N` and the body lowers with no length-variable handling (length
    // polymorphism is discharged here).
    let poly_words: HashMap<&str, &WordDef> = module
        .words
        .iter()
        .filter(|w| w.poly.is_some())
        .map(|w| (w.name.as_str(), w))
        .collect();
    // Dedup by symbol and sort, so the monomorphized funcs emit in a fixed
    // order regardless of `instantiations`' randomized HashMap iteration --
    // the rest of the module emits deterministically from `Vec`-ordered words,
    // and the IL should too.
    let mut distinct: Vec<(String, &CallInst)> = Vec::new();
    let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    for inst in module.instantiations.values() {
        let symbol = crate::ast::instantiation_symbol(&inst.callee, &inst.subst, inst.generation);
        if emitted.insert(symbol.clone()) {
            distinct.push((symbol, inst));
        }
    }
    distinct.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (symbol, inst) in distinct {
        let word = poly_words[inst.callee.as_str()];
        let sig = word
            .poly
            .as_ref()
            .expect("a recorded callee is polymorphic");
        let effect = concrete_effect(sig, &inst.subst, &module.arrays);
        // R7/R14: a self-recursive polymorphic word is a nested polymorphic
        // call (the body calling the very word being instantiated), out of
        // scope this slice; `self_tail` stays `false` here rather than
        // reusing `has_self_tail_call` (which only recognizes a plain-name
        // `Call`, never a `CallInst` lookup), so such a body still lowers
        // correctly as an ordinary recursive call, just without the
        // loop/back-edge transform a monomorphic self-tail word gets.
        funcs.push(lower_word_parts(
            &symbol,
            &effect,
            &word.body,
            false,
            &env,
            &resolve,
            regs,
            &module.instantiations,
            &poly_arities,
            &combinator_bodies,
        ));
    }

    // R2: the override's body, by reference, keyed the way synthesis is keyed.
    // The REPL builds the same map from its own session-level store instead of
    // from a module's `words` (R11).
    let overrides: DropOverrides = drop_overloads
        .iter()
        .map(|(id, idx)| (*id, DropOverride::Body(&module.words[*idx])))
        .collect();

    // R12: append a synthesized destructor for every linear struct/enum type
    // (the drop-glue home decided in Phase 4, used starting here): `drop`
    // calls it as a plain `Call` (R16).
    funcs.extend(synthesize_aggregate_destructors(
        &env, &resolve, regs, &overrides,
    ));

    Ok(IrModule {
        funcs,
        structs: structs.layouts,
        enums: enums.layouts,
        arrays: arrays.layouts,
    })
}

/// Every linear struct's and enum's synthesized destructor, one `IrFunc` per
/// type. The REPL redefines these per line; safe because type redefinition is
/// rejected, so every generation's glue is identical. If type redefinition is
/// ever allowed, add a generation suffix, matching word symbols.
///
/// R11 (slice 8b): a *user* `drop` override is where that premise fails --
/// redefining one at the REPL puts a different body under the same symbol.
/// R11.2 additionally suffixes every *other* linear struct's/enum's/cell's
/// destructor too, once the session holds any override at all: any of them
/// may `Call` an overridden struct's destructor (directly, or transitively
/// through a further composed aggregate), so their own body's callee changes
/// across an override event exactly as the overridden struct's own body does.
/// All three symbol kinds carry the *same* session-wide override epoch
/// (`StructLayout`/`EnumLayout::drop_generation`, `Cells::drop_generations`,
/// set by the session), so every destructor in a session that has ever seen
/// an override mints a fresh, never-before-loaded symbol per override event
/// -- the cheap alternative to computing exactly which aggregates reach the
/// override. Before any override, epoch is `None` everywhere and every
/// symbol stays unsuffixed, unchanged from the build path.
///
/// R2 (slice 8b): a struct in `overrides` gets the user's own `drop` body under
/// that same symbol instead of the synthesized field glue. Every caller of the
/// destructor already goes through `struct_drop_symbol` (`emit_drop`, and
/// `drop_level_fields` through it), so substituting the body here is the whole
/// of dispatch: no call site resolves a `drop` overload by name.
///
/// R11.3: an `AlreadyLoaded` entry gets no destructor emitted at all — the
/// REPL marks every override but the one being declared that way, since each
/// override's symbol is pinned to its defining epoch and its body must be
/// lowered once, against the env it was checked against.
pub fn synthesize_aggregate_destructors(
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    overrides: &DropOverrides,
) -> Vec<IrFunc> {
    let Registries {
        structs,
        enums,
        cells,
        ..
    } = regs;
    let struct_destructors = structs
        .layouts
        .iter()
        .enumerate()
        // R10/R11: a linear *bundle* gets no glue. Its fields are the caller's
        // outputs, moved out by the unpack the instant the call returns, so a
        // destructor for the shell would free a linear one a second time.
        .filter(|(_, layout)| layout.is_linear && !layout.bundle)
        .filter_map(|(idx, _)| {
            let id = StructId::from_index(idx);
            match overrides.get(&id) {
                Some(DropOverride::Body(word)) => Some(synthesize_struct_destructor_override(
                    id, word, env, resolve, regs,
                )),
                Some(DropOverride::AlreadyLoaded) => None,
                None => Some(synthesize_struct_destructor(id, env, resolve, regs)),
            }
        });
    let enum_destructors = enums
        .layouts
        .iter()
        .enumerate()
        .filter(|(_, layout)| layout.is_linear)
        .map(|(idx, _)| synthesize_enum_destructor(EnumId::from_index(idx), env, resolve, regs));
    // Every cell gets a destructor, not just those whose filter would
    // require a linear payload: `drop` on any cell must free it.
    let cell_destructors = cells.payload.iter().enumerate().map(|(idx, _)| {
        synthesize_cell_destructor(OwnedCellId::from_index(idx), env, resolve, regs)
    });
    struct_destructors
        .chain(enum_destructors)
        .chain(cell_destructors)
        .collect()
}

/// One step of the route a fused destructor loop walks from a type back to
/// itself. A tree, not a flat list: an enum's variants are mutually
/// exclusive at runtime, so each may independently continue toward `Self`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PathStep {
    /// Project a `Struct`/`Enum` field of the current aggregate byval
    /// (`field_value`, no free).
    Project { field: usize },
    /// Materialize a `^T` field's payload and free the cell
    /// (`load_owned_payload` + `free`). `field` is `None` when the current
    /// type *is* the cell (the inner step of `^^Self`) rather than an
    /// aggregate holding it; a struct can hold two fields of the same cell
    /// type, so the index is not derivable from `cell`.
    Unwrap {
        field: Option<usize>,
        cell: OwnedCellId,
    },
    /// The path reached an enum, at the entry type or any intermediate point
    /// alike: dispatch on its tag. `None` for a variant that does not
    /// continue toward `Self` (drop its fields, leave the loop); `Some` for
    /// one that does, via its own further steps. More than one variant may
    /// continue: a tagged value is only ever one variant, so this is not the
    /// simultaneously-live multi-edge case a struct's own field choice must
    /// narrow. Always the last step of the sequence containing it, since what
    /// follows a dispatch lives inside each variant's own continuation.
    Branch {
        enum_id: EnumId,
        variants: Vec<Option<Vec<PathStep>>>,
    },
}

/// The route a fused destructor loop walks from `self_ty` back to `self_ty`,
/// or `None` for a type on no cycle. A fresh pass over `Registries`, not a
/// reuse of the checker's cycle graph: that graph cuts `^` edges entirely,
/// but this needs to see exactly them.
///
/// The search starts at `expand_path`, never `find_path`: `self_ty` is seeded
/// into `visited`, and `find_path`'s trivial `current == target` match would
/// otherwise succeed before `self_ty`'s own fields were ever examined. Only a
/// *subsequent* arrival back at `self_ty`, via at least one step, is a cycle.
fn recursive_disposal_path(self_ty: IrType, regs: Registries) -> Option<Vec<PathStep>> {
    expand_path(self_ty, self_ty, &mut vec![self_ty], regs)
}

/// One recursive hop of the walk: the trivial-match and cycle-prune checks
/// that must fire for every hop but not for the outermost one (hence the
/// split from `expand_path`). The target check precedes the prune check
/// because the entry type is itself in `visited`.
fn find_path(
    current: IrType,
    target: IrType,
    visited: &mut Vec<IrType>,
    regs: Registries,
) -> Option<Vec<PathStep>> {
    if current == target {
        return Some(Vec::new());
    }
    if visited.contains(&current) {
        return None;
    }
    visited.push(current);
    let found = expand_path(current, target, visited, regs);
    visited.pop();
    found
}

/// Search `current`'s own structure for a continuation toward `target`. A
/// cell counts as a type in its own right, so `^^Self` steps through the
/// inner cell rather than treating it as a dead end.
fn expand_path(
    current: IrType,
    target: IrType,
    visited: &mut Vec<IrType>,
    regs: Registries,
) -> Option<Vec<PathStep>> {
    match current {
        // R7 (slice 8b): a struct with a user `drop` overload is a dead end for
        // *another* type's search, exactly as a `Copy` scalar field is. The
        // fused loop inlines every intermediate type's field projection instead
        // of calling its destructor, so routing a cycle through an overridden
        // struct would bypass the user's body and leak its resource silently.
        // The `current != target` carve-out is for the search's own root: an
        // overridden struct's own destructor is its override regardless (R2), so
        // whether a path back to itself exists is moot there.
        IrType::Struct(id)
            if current != target && regs.structs.layouts[id.index()].has_drop_overload =>
        {
            None
        }
        IrType::Struct(id) => {
            let fields = &regs.structs.layouts[id.index()].fields;
            expand_fields(fields, target, visited, regs)
        }
        IrType::Enum(id) => {
            let variants: Vec<Option<Vec<PathStep>>> = regs.enums.layouts[id.index()]
                .variants
                .iter()
                .map(|v| {
                    // A copy of `visited` per variant: one variant's
                    // abandoned attempt must not poison a sibling's search.
                    let mut seen = visited.clone();
                    expand_fields(&v.fields, target, &mut seen, regs)
                })
                .collect();
            variants.iter().any(Option::is_some).then(|| {
                vec![PathStep::Branch {
                    enum_id: id,
                    variants,
                }]
            })
        }
        // `cells.payload[c] == target` needs no case of its own: `find_path`
        // matches it and returns an empty tail, which this prepend turns into
        // a lone `Unwrap`.
        IrType::OwnedCell(c) => find_path(regs.cells.payload[c.index()], target, visited, regs)
            .map(|rest| {
                prepend(
                    PathStep::Unwrap {
                        field: None,
                        cell: c,
                    },
                    rest,
                )
            }),
        _ => None,
    }
}

/// Try one struct's fields, or one enum variant's, in reverse declaration
/// order; the first candidate whose own sub-walk reaches `target` wins.
/// Backtracking, not a syntactic guess: a field is only chosen once a
/// complete path through it is known to exist.
///
/// A direct `^target` field is tried, in reverse order, before any other
/// field: this is today's fusable shape, and it must keep winning even when
/// a *later*-declared field also reaches `target`, only indirectly. Without
/// this tier, declaring an indirect-but-successful field after a direct one
/// flips which edge the reverse scan below picks, silently lengthening the
/// fused loop's path with an extra unwrap step and hoisted slot per level of
/// indirection, for a shape that a direct edge would have reached in one.
///
/// Reverse order generalizes the old direct-edge rule's last-field tie-break
/// to every struct level of the walk. This is the one restriction on a
/// struct with two fields that could each reach `target`: it picks exactly
/// one, since both may be live in one node instance at once (Phase 6's
/// worklist case). The non-chosen fields are dropped like any other field, not
/// marked. Looping the last child rather than the first is what makes a
/// right-leaning shape constant-stack and a left-leaning one still O(depth)
/// (documented, not fixed). Arrays are absent deliberately: `[^T N]` is
/// rejected outright, so an array can never launder an edge.
fn expand_fields(
    fields: &[FieldLayout],
    target: IrType,
    visited: &mut Vec<IrType>,
    regs: Registries,
) -> Option<Vec<PathStep>> {
    for (fi, field) in fields.iter().enumerate().rev() {
        if let IrType::OwnedCell(c) = field.ty {
            if regs.cells.payload[c.index()] == target {
                return Some(vec![PathStep::Unwrap {
                    field: Some(fi),
                    cell: c,
                }]);
            }
        }
    }

    for (fi, field) in fields.iter().enumerate().rev() {
        match field.ty {
            IrType::OwnedCell(c) => {
                let payload = regs.cells.payload[c.index()];
                if let Some(rest) = find_path(payload, target, visited, regs) {
                    return Some(prepend(
                        PathStep::Unwrap {
                            field: Some(fi),
                            cell: c,
                        },
                        rest,
                    ));
                }
            }
            IrType::Struct(_) | IrType::Enum(_) => {
                if let Some(rest) = find_path(field.ty, target, visited, regs) {
                    return Some(prepend(PathStep::Project { field: fi }, rest));
                }
            }
            _ => {}
        }
    }
    None
}

fn prepend(step: PathStep, rest: Vec<PathStep>) -> Vec<PathStep> {
    let mut path = vec![step];
    path.extend(rest);
    path
}

/// R12: synthesize struct `id`'s destructor, called by `drop` on any value of
/// that type: drop each linear field, in declaration order. Built via a
/// bare `FuncBuilder` (no locals, no tail-call machinery) reusing the same
/// `field_value`/`emit_drop` a `drop`, `S>fi`, and `S<fi` use, so "how a field
/// is disposed" stays in one place.
///
/// A struct on a disposal cycle (a `^Self` field, or a longer route back to
/// itself through other types) is disposed by one fused loop that walks the
/// whole route, instead of a mutually recursive `cell_drop`/`struct_drop`
/// chain. An all-struct cycle has no base case, so its loop is exit-less and
/// the trailing `Ret` is skipped; such a shape is uninhabited, so that is
/// about not crashing the emitter rather than about a program that runs.
fn synthesize_struct_destructor(
    id: StructId,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
) -> IrFunc {
    let structs = regs.structs;
    let self_ty = IrType::Struct(id);
    let mut b = FuncBuilder::new(env, resolve, regs, String::new());
    let param = b.fresh_value(self_ty);
    let fields = structs.layouts[id.index()].fields.clone();
    match recursive_disposal_path(self_ty, regs) {
        // A struct's own path always starts at one of its own fields: only an
        // enum expands into a `Branch`, and this level is not one.
        Some(path) => {
            // R1a: the aggregate-staging transform is gated OFF here; the fused
            // destructor loop is correct by its own ordered hoisted-slot reuse
            // and must stay byte-for-byte.
            let node = b.begin_loop(&[param], false)[0];
            b.emit_field_level(node, &fields, &path);
            b.finalize_loop();
        }
        None => b.drop_level_fields(param, &fields, None),
    }
    // A back-edge or a dispatch arm already sealed the final block, and a
    // second seal would emit a duplicate `BlockId`.
    if !b.terminated {
        b.seal_block(Terminator::Ret(None));
    }
    IrFunc {
        name: struct_drop_symbol(id, structs.layouts[id.index()].drop_generation),
        params: vec![self_ty],
        ret: None,
        blocks: b.blocks,
        value_types: b.value_types,
    }
}

/// R2 (slice 8b): struct `id`'s destructor *is* the user's `drop` body. Lowered
/// by exactly the machinery any other word body gets, then renamed to the
/// destructor symbol every existing call site already calls — the override
/// replaces `synthesize_struct_destructor`'s field glue rather than running
/// before or alongside it (R5), so there is no glue left to compose with.
fn synthesize_struct_destructor_override(
    id: StructId,
    word: &WordDef,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
) -> IrFunc {
    IrFunc {
        name: struct_drop_symbol(id, regs.structs.layouts[id.index()].drop_generation),
        ..lower_word(
            word,
            env,
            resolve,
            regs,
            empty_instantiations(),
            empty_poly_arities(),
        )
    }
}

/// R12 (Phase 4): synthesize enum `id`'s destructor, called by `drop` on any
/// value of that type. Unlike the struct case (a fixed field list), an enum's
/// active variant is a runtime fact, so the destructor tag-dispatches (its
/// own `Jnz` chain, the same compare-chain shape `lower_clauses` uses for a
/// clause-style word's scrutinee) and then drops only the dispatched variant's
/// linear payload fields. Every variant gets its own block even if none of its
/// fields are linear (an empty block that just returns), so the dispatch
/// shape stays uniform regardless of which variants happen to carry a linear
/// field.
///
/// If the enum is on a disposal cycle, the whole destructor becomes one fused
/// loop: the dispatch reads the loop-carried node instead of the param, a
/// variant that continues toward `Self` walks its own route and back-edges to
/// the header, and a variant that does not returns. That is the base case, so
/// an inhabited recursive enum (`Nil`/`Cons`) disposes in constant stack,
/// however long the route back to itself is.
fn synthesize_enum_destructor(
    id: EnumId,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
) -> IrFunc {
    let enums = regs.enums;
    let self_ty = IrType::Enum(id);
    let mut b = FuncBuilder::new(env, resolve, regs, String::new());
    let param = b.fresh_value(self_ty);
    match recursive_disposal_path(self_ty, regs) {
        // An enum's own path is always one top-level `Branch` (`expand_path`
        // builds no other shape for an enum), so the loop's whole body is
        // that dispatch.
        Some(path) => {
            // R1a: aggregate staging gated OFF (see `synthesize_struct_destructor`).
            let node = b.begin_loop(&[param], false)[0];
            b.emit_path_steps(node, &path);
            b.finalize_loop();
        }
        // No cycle: the same dispatch, every variant a base case.
        None => {
            let base_cases = vec![None; enums.layouts[id.index()].variants.len()];
            b.emit_branch(param, id, &base_cases);
        }
    }

    IrFunc {
        name: enum_drop_symbol(id, enums.layouts[id.index()].drop_generation),
        params: vec![self_ty],
        ret: None,
        blocks: b.blocks,
        value_types: b.value_types,
    }
}

/// Copy the payload out (if linear), free the cell, then drop the copied-out
/// payload. The block is freed before the payload's own destructor runs, so
/// the free must come after the copyout (`load_owned_payload` never touches
/// the block again) but before the drop.
fn synthesize_cell_destructor(
    id: OwnedCellId,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
) -> IrFunc {
    let Registries {
        structs,
        enums,
        arrays,
        cells,
        ..
    } = regs;
    let mut b = FuncBuilder::new(env, resolve, regs, String::new());
    let param = b.fresh_value(IrType::OwnedCell(id));
    let payload_ty = cells.payload[id.index()];
    let is_linear = field_is_linear(payload_ty, structs, enums, arrays);
    let payload = is_linear.then(|| b.load_owned_payload(param, payload_ty));
    let size = b.value_size(payload_ty);
    let size_v = b.fresh_value(IrType::I64);
    b.push_instr(Instr::Const(size_v, size as i64));
    b.push_instr(Instr::Call(
        None,
        FREE_SYMBOL.to_string(),
        vec![param, size_v],
    ));
    if let Some(payload) = payload {
        b.emit_drop(payload);
    }
    b.seal_block(Terminator::Ret(None));
    IrFunc {
        name: cell_drop_symbol(id, cells.drop_generations[id.index()]),
        params: vec![IrType::OwnedCell(id)],
        ret: None,
        blocks: b.blocks,
        value_types: b.value_types,
    }
}

/// Lower a bare REPL line to a uniform-signature wrapper `sooth_line_{seq}`
/// `(Ptr stack, Int top) -> Int`. The prologue loads the whole carried stack
/// (`entry_depth` slots) from the buffer, the body runs in registers exactly
/// like a word, the epilogue stores the resulting output slots back, and it
/// returns the advanced top `top + (out_bytes - in_bytes)`.
///
/// Carried slots are size-aware per slot: a scalar occupies a
/// byte-identical 8-byte cell (so every scalar-only line marshals exactly as
/// before), a struct occupies its aggregate size (`carried_slot_bytes`); each
/// slot sits at the cumulative byte offset of the slots below it. A struct
/// slot is copied by an aggregate `blit` out of the buffer into a fresh frame
/// slot on entry and back into the buffer on exit, so the line body owns the
/// value independently of the persistent buffer.
///
/// `entry_types` names each carried slot's true frontend `Type` (one per
/// `entry_depth` slot). Q2 (Slice 2): a scalar buffer slot always stays an
/// 8-byte `l`-width store (canonicalization, R15, keeps its low `bits`
/// authoritative), but a scalar slot narrower or differently-signed than
/// `i64` is relabeled to its real `IrType` right after the load, via the same
/// `Conv` the conversion words use, so a later op in this line sees the
/// correct operand type (e.g. homogeneous `+` against another `u8`) instead
/// of a stale `i64`.
///
/// Returns the `IrFunc`, the emitted output slot count `M`, and `out_bytes`
/// (the number of buffer bytes the epilogue actually wrote), so the caller
/// sizes its buffer from the same numbers the wrapper uses rather than from a
/// separately-computed depth that could in principle diverge.
#[allow(clippy::too_many_arguments)]
pub fn lower_line(
    seq: u64,
    terms: &[Term],
    entry_depth: usize,
    entry_types: &[Type],
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    instantiations: &HashMap<Span, CallInst>,
    poly_arities: &HashMap<String, usize>,
) -> (IrFunc, usize, usize) {
    debug_assert_eq!(entry_types.len(), entry_depth);
    // A REPL line has no word name to self-tail-call against.
    let mut b = FuncBuilder::new(env, resolve, regs, String::new());
    // R7 (Slice 2): a call to a retained polymorphic word resolves through the
    // instantiation table keyed by its call-site span, not the name-keyed env.
    b.instantiations = instantiations;
    b.poly_arities = poly_arities;

    // Params occupy the first value ids: %v0 = stack base (Ptr), %v1 = top (Int).
    let base = b.fresh_value(IrType::Ptr);
    let top = b.fresh_value(IrType::I64);

    // Prologue: load each carried slot from its cumulative byte offset, deepest
    // (slot 0) first. A struct is copied out of the buffer into a fresh frame
    // slot; a scalar loads its 8-byte cell exactly as before.
    let mut stack = Vec::with_capacity(entry_depth);
    let mut in_bytes = 0u32;
    for ty in entry_types {
        let slot_ty = ir_type_of(*ty);
        let ptr = b.fresh_value(IrType::Ptr);
        b.push_instr(Instr::PtrOffset(ptr, base, in_bytes as i64));
        // A float slot loads directly at its `s`/`d` width (R20): the backend
        // picks `loadd`/`loads` from the value's float `IrType`, so the bits
        // re-enter as a true float and need no integer `Conv`-relabel (that
        // path is integer-only). An integer slot narrower/differently-signed
        // than `i64` still relabels via `Conv`; a `Bool` slot needs none (`jnz`
        // reads any register, and its stored 0/1 is valid `l`-content).
        match slot_ty {
            IrType::Struct(id) => {
                let dst = b.alloc_struct(id);
                let size = b.structs.layouts[id.index()].size;
                if size > 0 {
                    b.push_instr(Instr::Blit(ptr, dst, size));
                }
                stack.push(dst);
            }
            IrType::Enum(id) => {
                let dst = b.alloc_enum(id);
                let size = b.enums.layouts[id.index()].size;
                if size > 0 {
                    b.push_instr(Instr::Blit(ptr, dst, size));
                }
                stack.push(dst);
            }
            IrType::Array(id) => {
                let dst = b.alloc_array(id);
                let size = b.arrays.layouts[id.index()].size;
                if size > 0 {
                    b.push_instr(Instr::Blit(ptr, dst, size));
                }
                stack.push(dst);
            }
            IrType::Float { .. } => {
                let v = b.fresh_value(slot_ty);
                b.push_instr(Instr::Load(v, ptr));
                stack.push(v);
            }
            IrType::Int { .. } if slot_ty != IrType::I64 => {
                let v = b.fresh_value(IrType::I64);
                b.push_instr(Instr::Load(v, ptr));
                let relabeled = b.fresh_value(slot_ty);
                b.push_instr(Instr::Conv(relabeled, v));
                stack.push(relabeled);
            }
            // Every remaining carried slot loads directly at its own
            // `IrType` and needs no relabeling: `i64`, `Bool`, `Usize` and
            // `Isize` all fill the full 8-byte cell as-is, and `OwnedCell`,
            // `Str`, `Cstr` are all a bare pointer. Keeping the type (rather
            // than degrading to a bare `I64`) is what lets a later `drop`
            // still find `OwnedCell`'s destructor, a later `len`/`.`/`cstr`
            // dispatch on `Str`/`Cstr`, and `.`/comparisons treat a
            // `Bool`/`Usize` slot correctly instead of as a signed `i64`.
            IrType::Int { .. }
            | IrType::Bool
            | IrType::Usize
            | IrType::Isize
            | IrType::OwnedCell(_)
            | IrType::Str
            | IrType::Cstr => {
                let v = b.fresh_value(slot_ty);
                b.push_instr(Instr::Load(v, ptr));
                stack.push(v);
            }
            // The REPL's residual-stack check rejects a line that leaves a
            // reference on the stack (check.rs's "a reference cannot be stored:
            // the line leaves `&P` on the stack" diagnostic, tests/phase3_refs.rs),
            // so a `Type::Ref` can never reach the carried-slot buffer at all.
            IrType::Ptr => unreachable!("a reference can never be a carried slot"),
        }
        in_bytes += carried_slot_bytes(slot_ty, b.structs, b.enums, b.arrays);
    }
    b.stack = stack;

    // A REPL expr line is not a word body, so nothing is in self-tail position.
    b.lower_terms(terms, false);

    // Epilogue: store each result slot back to the buffer at its cumulative
    // byte offset. A scalar 8-byte cell is written at the value's own width: a
    // float via `stores`/`stored`, an integer or `Bool` via `storel` (a `Bool`
    // widening to `l`, its stored 0/1 valid `l`-content). A struct is copied
    // back into the buffer by an aggregate `blit`.
    let out = mem::take(&mut b.stack);
    let m = out.len();
    let mut out_bytes = 0u32;
    for v in &out {
        let vty = b.value_type(*v);
        let ptr = b.fresh_value(IrType::Ptr);
        b.push_instr(Instr::PtrOffset(ptr, base, out_bytes as i64));
        match vty {
            IrType::Struct(id) => {
                let size = b.structs.layouts[id.index()].size;
                if size > 0 {
                    b.push_instr(Instr::Blit(*v, ptr, size));
                }
            }
            IrType::Enum(id) => {
                let size = b.enums.layouts[id.index()].size;
                if size > 0 {
                    b.push_instr(Instr::Blit(*v, ptr, size));
                }
            }
            IrType::Array(id) => {
                let size = b.arrays.layouts[id.index()].size;
                if size > 0 {
                    b.push_instr(Instr::Blit(*v, ptr, size));
                }
            }
            _ => b.push_instr(Instr::Store(ptr, *v)),
        }
        out_bytes += carried_slot_bytes(vty, b.structs, b.enums, b.arrays);
    }

    // Return the advanced top as a byte delta; (out_bytes - in_bytes) may be
    // negative.
    let delta = out_bytes as i64 - in_bytes as i64;
    let delta_val = b.fresh_value(IrType::I64);
    b.push_instr(Instr::Const(delta_val, delta));
    let new_top = b.fresh_value(IrType::I64);
    b.push_instr(Instr::Bin(new_top, BinOp::Add, top, delta_val));
    b.seal_block(Terminator::Ret(Some(new_top)));

    let func = IrFunc {
        name: format!("sooth_line_{seq}"),
        params: vec![IrType::Ptr, IrType::I64],
        ret: Some(IrType::I64),
        blocks: b.blocks,
        value_types: b.value_types,
    };
    (func, m, out_bytes as usize)
}

/// R10: the `IrType` a word returns — its one output, or the synthesized
/// bundle struct for two or more. The single derivation both the lowering env's
/// `ret_ty` and `lower_word`'s own `ret` go through, so a caller reading the
/// env and the callee it calls can never disagree about the return shape.
/// Falls back to the first output where no bundle was interned (the REPL's
/// registries, D2): that path keeps its pre-slice lowering rather than
/// half-entering the bundle ABI.
fn word_ret_ty(outputs: &[TypedSlot], structs: &Structs) -> Option<IrType> {
    match bundle_of(outputs, structs) {
        Some(id) => Some(IrType::Struct(id)),
        None => outputs.first().map(|slot| ir_type_of(slot.ty)),
    }
}

/// R10: the bundle a word with these declared outputs returns through.
fn bundle_of(outputs: &[TypedSlot], structs: &Structs) -> Option<StructId> {
    if outputs.len() < 2 {
        return None;
    }
    let tys: Vec<Type> = outputs.iter().map(|slot| slot.ty).collect();
    structs.bundle_for(&tys)
}

/// R9: build the concrete `StackEffect` of one instantiation `(word, θ)`,
/// substituting the ground `θ` into the polymorphic signature's fixed inputs
/// and outputs. The row variable (`..s`) is not materialized: it is a
/// pass-through that stays on the caller's stack (S2), so it never enters the
/// monomorphized function's frame.
fn concrete_effect(sig: &PolySig, subst: &Subst, arrays: &[ArrayDecl]) -> StackEffect {
    let slot = |pt: &PolyType| TypedSlot {
        name: None,
        ty: subst_polytype(pt, subst, arrays),
    };
    StackEffect {
        inputs: sig.inputs.iter().map(&slot).collect(),
        outputs: sig.outputs.iter().map(&slot).collect(),
    }
}

/// R9: apply a ground `θ` to a `PolyType`, yielding a concrete `Type`. A
/// variable resolves through `θ`; a variable-bearing array folds to its already
/// interned concrete shape (the caller pushed that shape, so it exists in the
/// module's array registry — lowering only reads it, it never interns).
fn subst_polytype(pt: &PolyType, subst: &Subst, arrays: &[ArrayDecl]) -> Type {
    match pt {
        PolyType::Concrete(t) => *t,
        PolyType::Var(v) => subst
            .ty_of(*v)
            .expect("checked: unification bound every input type variable"),
        PolyType::Array(elem, len) => {
            let element = subst_polytype(elem, subst, arrays);
            let count = match len {
                Len::Concrete(k) => *k,
                Len::Var(ln) => subst
                    .len_of(*ln)
                    .expect("checked: unification bound every length variable"),
            };
            let idx = arrays
                .iter()
                .position(|d| d.element == element && d.count == count)
                .expect("checked: the concrete array shape was interned at the call site");
            Type::Array(ArrayId::from_index(idx), arrays[idx].name_static)
        }
        // Slice 6a (R7): a quotation-taking word is never monomorphized to a
        // standalone `IrFunc` (R20), so no `θ` is ever applied to a declared
        // quotation effect at lowering. Unreachable, guarded by R7a's audit
        // and R20u.
        PolyType::Quotation(..) => {
            unreachable!("a quotation effect never reaches monomorphized lowering (R7/R20)")
        }
    }
}

/// Lower a single word body against an external env/resolver. The REPL uses
/// this directly (renaming the returned `IrFunc.name` to a mangled symbol)
/// so a definition compiles against previously-loaded words. A REPL line has
/// no polymorphic words (D2), so its calls carry no instantiation table.
pub(crate) fn lower_word(
    word: &WordDef,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    instantiations: &HashMap<Span, CallInst>,
    poly_arities: &HashMap<String, usize>,
) -> IrFunc {
    let self_tail = crate::check::has_self_tail_call(word);
    lower_word_parts(
        &word.name,
        &word.effect,
        &word.body,
        self_tail,
        env,
        resolve,
        regs,
        instantiations,
        poly_arities,
        empty_combinators(),
    )
}

/// R7 (Slice 2): lower one REPL polymorphic-word instantiation `(word, θ)`
/// into a monomorphized `IrFunc` under its mangled `symbol`. The body is the
/// retained polymorphic word's own body, checked once at its defining line;
/// `resolve` is the frozen defining-line snapshot (D3), not the instantiating
/// line's env, so an unrelated later redefinition of a callee cannot change
/// this body's meaning. Nested polymorphic calls are out of scope (Slice 1
/// R14), so the body carries no instantiation table of its own.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_instantiation(
    symbol: &str,
    sig: &PolySig,
    subst: &Subst,
    body: &WordBody,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    arrays: &[ArrayDecl],
) -> IrFunc {
    let effect = concrete_effect(sig, subst, arrays);
    lower_word_parts(
        symbol,
        &effect,
        body,
        false,
        env,
        resolve,
        regs,
        empty_instantiations(),
        empty_poly_arities(),
        empty_combinators(),
    )
}

/// The shared word-body lowering, parameterized by name/effect/body so a
/// monomorphized instantiation (R9) can lower a polymorphic word's body under
/// its mangled symbol against a `θ`-substituted concrete effect. The
/// instantiation table and poly-arity map thread through so a call to a
/// polymorphic word inside this body resolves to its per-site symbol (R14).
#[allow(clippy::too_many_arguments)]
fn lower_word_parts(
    name: &str,
    effect: &StackEffect,
    body: &WordBody,
    self_tail: bool,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    instantiations: &HashMap<Span, CallInst>,
    poly_arities: &HashMap<String, usize>,
    combinators: &HashMap<String, Vec<Term>>,
) -> IrFunc {
    let params: Vec<IrType> = effect.inputs.iter().map(|s| ir_type_of(s.ty)).collect();
    let bundle = bundle_of(&effect.outputs, regs.structs);
    let ret = word_ret_ty(&effect.outputs, regs.structs);

    let mut b = FuncBuilder::new(env, resolve, regs, name.to_string());
    b.instantiations = instantiations;
    b.poly_arities = poly_arities;
    b.combinators = combinators;

    // Params occupy the first N value ids; leftmost input is deepest.
    // (b.cur_word_name is set above for R7's self-tail-call detection.)
    let params_values: Vec<Value> = params.iter().map(|ty| b.fresh_value(*ty)).collect();

    // R6: a self-tail-recursive word lowers to a loop. The entry block binds
    // the params and jumps to a header carrying one phi per loop-carried slot;
    // the body reads the phi outputs so each iteration rebinds them. A word
    // with no tail self-call lowers exactly as before (no header, no phi).
    let entry_values = if self_tail {
        // R1a: aggregate staging gated ON for the user self-tail-call loop.
        b.begin_loop(&params_values, true)
    } else {
        params_values
    };

    // A reference parameter arrives as an opaque `Ptr`, so the referent
    // shape every projection and access needs comes from the declared type,
    // not from the value. Seeded against `entry_values` so a loop reads it off
    // the header phi output the body actually uses.
    for (slot, value) in effect.inputs.iter().zip(&entry_values) {
        if let Type::Ref(id, _, _) = slot.ty {
            b.ref_inner.insert(*value, regs.refs.referent[id.index()]);
        }
    }

    match body {
        WordBody::Terms { terms } => {
            // Every input starts on the stack (D6: the header phi outputs when
            // looping); an entry `| ... |` binding pops from it like any other
            // binding term.
            b.stack = entry_values;
            b.lower_terms(terms, self_tail);
        }
        WordBody::Clauses(clauses) => {
            let scrutinee_ty = effect
                .inputs
                .last()
                .expect("clause word has a scrutinee input")
                .ty;
            b.lower_clauses(clauses, &entry_values, scrutinee_ty)
        }
    }

    // R8: back-patch the header phis with the collected back-edge operands.
    if self_tail {
        b.finalize_loop();
    }

    // The fall-through (base-case) block returns; a body that ended entirely in
    // back-edges is already terminated and needs no Ret.
    if !b.terminated {
        // R10: two or more outputs leave the frame packed into the bundle,
        // deepest output in the first field; one or none is the single value
        // (or nothing) it always was.
        let result = match bundle {
            Some(id) => Some(b.pack_bundle(id)),
            None if ret.is_some() => b.stack.pop(),
            None => None,
        };
        b.seal_block(Terminator::Ret(result));
    }

    IrFunc {
        name: name.to_string(),
        params,
        ret,
        blocks: b.blocks,
        value_types: b.value_types,
    }
}

/// A shared empty instantiation table for lowering paths with no polymorphic
/// call sites (the REPL, D2; destructor synthesis; unit tests), so
/// `FuncBuilder::new` can hand out a valid reference without every caller
/// threading one.
fn empty_instantiations() -> &'static HashMap<Span, CallInst> {
    static EMPTY: std::sync::OnceLock<HashMap<Span, CallInst>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

/// The poly-arity companion of `empty_instantiations`.
fn empty_poly_arities() -> &'static HashMap<String, usize> {
    static EMPTY: std::sync::OnceLock<HashMap<String, usize>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

/// R19: the combinator-body companion of `empty_instantiations`. A path with
/// no monomorphic quotation-taking words to inline (the REPL, D2; destructor
/// synthesis; unit tests) hands out this empty map.
fn empty_combinators() -> &'static HashMap<String, Vec<Term>> {
    static EMPTY: std::sync::OnceLock<HashMap<String, Vec<Term>>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

fn is_aggregate(ty: IrType) -> bool {
    matches!(ty, IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_))
}

/// Per-carried-slot loop metadata (R2), in full carried-slot order. A scalar
/// keeps its header phi; an aggregate carries no header phi but a stable
/// entry-hoisted slot (the pointer the body reads every iteration) plus a
/// staging temp and blit `size` for the back-edge read-before-write copy (R4).
enum CarriedSlot {
    Scalar {
        phi: Value,
    },
    Aggregate {
        stable: Value,
        temp: Value,
        size: u32,
    },
}

struct FuncBuilder<'a> {
    env: &'a HashMap<String, Arity>,
    resolve: Resolver<'a>,
    structs: &'a Structs,
    enums: &'a Enums,
    arrays: &'a Arrays,
    cells: &'a Cells,
    /// The per-`RefId` referent `IrType`: needed to resolve a
    /// reference-mode clause scrutinee's `EnumId` when the referent itself is
    /// an enum.
    refs: &'a Refs,
    /// R14: the per-call-site instantiation table. A `Call` term whose span
    /// keys an entry here is a call to a polymorphic word and resolves to that
    /// entry's mangled symbol and per-θ output shape, not the name-keyed
    /// `env`/`resolve`. Empty on the REPL/destructor/test paths.
    instantiations: &'a HashMap<Span, CallInst>,
    /// R14: the fixed input arity of each polymorphic word, name-keyed. How
    /// many args a polymorphic call pops (the `CallInst` carries the output
    /// shape, but the input count is name-constant across θ, so it lives here).
    poly_arities: &'a HashMap<String, usize>,
    /// R19/R20: monomorphic quotation-taking words (combinators), name-keyed
    /// to their bodies. A `Call` of such a name is spliced in place rather
    /// than lowered to an `Instr::Call`, mirroring the checker's inliner
    /// (R18): the callee mints no `IrFunc` (it is absent from `funcs`/`env`),
    /// so its only reachable form is the splice. Empty on the REPL/destructor/
    /// test paths.
    combinators: &'a HashMap<String, Vec<Term>>,
    /// Name of the word currently being lowered, used by the tail-call ->
    /// back-edge transform (R7) to recognize a self-call.
    cur_word_name: String,
    /// The loop header block (R6), `Some` iff this word is self-tail-recursive
    /// and is being lowered as a loop. Tail self-calls back-edge to it (R7).
    header: Option<BlockId>,
    /// Per loop-carried slot metadata (input arity many), in full slot order
    /// (R2). A scalar slot carries its header phi; an aggregate slot carries
    /// its entry-hoisted stable slot, staging temp, and blit size instead of a
    /// phi. `finalize_loop` dispatches on this per slot.
    carried_slots: Vec<CarriedSlot>,
    /// Collected back-edges (R8): each is `(pred block, one arg value per
    /// carried slot)`. Finalized into the header phis after the body lowers,
    /// since the operands are only known on the back-edges.
    back_edges: Vec<(BlockId, Vec<Value>)>,
    /// The loop's entry block (the block that ran before `begin_loop`'s jump
    /// to the header), `Some` alongside `header`. An `Alloc` emitted while
    /// looping is hoisted here (R6 constant-stack corollary): QBE's `alloc*`
    /// bumps the frame pointer on every execution and never reclaims it
    /// within a function, so an aggregate constructed on the back-edge (e.g.
    /// a clause's variant re-scrutinee) would otherwise grow the frame by one
    /// slot per iteration and blow the stack well before the loop's constant-
    /// stack guarantee is exercised. Hoisting reserves one fixed slot per
    /// static alloc site, reused (overwritten) every iteration instead. This is
    /// safe even when a loop constructs an inline aggregate into a same-site
    /// slot before reading the prior iteration's value, because a carried
    /// aggregate is snapshotted onto its own stable slot on the back-edge (R4),
    /// so hoisting no longer depends on the body's read-before-overwrite order.
    entry_block: Option<BlockId>,
    /// Whether the current block has already been sealed (by a back-edge Jmp or
    /// another terminator), so no fall-through Ret/Jmp should follow.
    terminated: bool,
    blocks: Vec<Block>,
    cur_id: BlockId,
    cur_instrs: Vec<Instr>,
    next_value: u32,
    next_block: u32,
    stack: Vec<Value>,
    /// The names in scope, innermost-last (R2, R10): leaving a block truncates
    /// this to its length at block entry. A bound value is SSA and outlives the
    /// name, so teardown frees nothing.
    locals: Vec<(String, Value)>,
    value_types: Vec<IrType>,
    /// Compile-time integer value of each `Const`-defined `Value`, for the
    /// `fill` count (M1: the count is a checker-guaranteed literal) and the
    /// element/array-shape lookup. A shuffle reuses a value id, so a duped
    /// literal keeps its recorded value.
    const_vals: HashMap<Value, i64>,
    /// The referent `IrType` of every reference-typed `Value`. A
    /// reference lowers to the opaque `IrType::Ptr`, which deliberately says
    /// nothing about what it points at, so the shape a projection or an access
    /// needs — a field offset, an element stride, an aggregate's blit size —
    /// is carried here instead. Seeded from a word's declared reference
    /// parameters and extended by each projection.
    ref_inner: HashMap<Value, IrType>,
    /// R12: the quotation-literal body table, indexed by `QuotId`. A quotation
    /// literal lowers to a phantom `Value` that defines no `Instr`; its body is
    /// interned here and spliced in place at `call`/`times` (D5 fusion), never
    /// emitted as a runtime code value.
    quot_defs: Vec<Vec<Term>>,
    /// R12: the phantom quotation `Value` -> its `QuotId`. A shuffle/bind moves
    /// the phantom verbatim (`self.locals`/`self.stack` carry `Value` ids), so
    /// no `Binding` analogue is needed here (D2); `call`/`times` resolve the
    /// body through this map.
    quot_bodies: HashMap<Value, QuotId>,
    /// R18/R21: a monotonic per-function suffix counter, mirroring the
    /// checker's, so a combinator body spliced here is alpha-renamed exactly as
    /// it was for checking. Without it a passed-down literal's captured name
    /// would rebind to an inner combinator's same-named local (dynamic, not
    /// lexical, capture).
    inline_uid: u32,
}

impl<'a> FuncBuilder<'a> {
    fn new(
        env: &'a HashMap<String, Arity>,
        resolve: Resolver<'a>,
        regs: Registries<'a>,
        cur_word_name: String,
    ) -> Self {
        let Registries {
            structs,
            enums,
            arrays,
            cells,
            refs,
        } = regs;
        FuncBuilder {
            env,
            resolve,
            structs,
            enums,
            arrays,
            cells,
            refs,
            instantiations: empty_instantiations(),
            poly_arities: empty_poly_arities(),
            combinators: empty_combinators(),
            cur_word_name,
            header: None,
            carried_slots: Vec::new(),
            back_edges: Vec::new(),
            entry_block: None,
            terminated: false,
            blocks: Vec::new(),
            cur_id: BlockId(0),
            cur_instrs: Vec::new(),
            next_value: 0,
            next_block: 1, // block 0 is the entry, already current
            stack: Vec::new(),
            locals: Vec::new(),
            value_types: Vec::new(),
            const_vals: HashMap::new(),
            ref_inner: HashMap::new(),
            quot_defs: Vec::new(),
            quot_bodies: HashMap::new(),
            inline_uid: 0,
        }
    }

    fn fresh_value(&mut self, ty: IrType) -> Value {
        let v = Value(self.next_value);
        self.next_value += 1;
        self.value_types.push(ty);
        v
    }

    fn value_type(&self, v: Value) -> IrType {
        self.value_types[v.0 as usize]
    }

    fn fresh_block(&mut self) -> BlockId {
        let b = BlockId(self.next_block);
        self.next_block += 1;
        b
    }

    fn push_instr(&mut self, instr: Instr) {
        self.cur_instrs.push(instr);
    }

    /// Hoist an `Alloc` (or a carried-aggregate init `Blit`, R3) into the entry
    /// block while looping (`entry_block` is `Some`); otherwise emit it into the
    /// current block. It appends whatever `Instr` it is given, not only an
    /// `Alloc`. See `entry_block`'s doc comment for why a loop body must never
    /// alloc.
    fn push_alloc(&mut self, instr: Instr) {
        match self.entry_block {
            Some(entry) => {
                let block = self
                    .blocks
                    .iter_mut()
                    .find(|b| b.id == entry)
                    .expect("entry block");
                block.instrs.push(instr);
            }
            None => self.push_instr(instr),
        }
    }

    /// Seal the current block with `term` and append it to the function.
    fn seal_block(&mut self, term: Terminator) {
        let instrs = mem::take(&mut self.cur_instrs);
        self.blocks.push(Block {
            id: self.cur_id,
            instrs,
            term,
        });
    }

    /// Begin a fresh (empty) block; `cur_instrs` is already empty after a seal.
    fn start_block(&mut self, id: BlockId) {
        self.cur_id = id;
    }

    /// R6/R1-R3: open the loop shape. The current (entry) block binds `params`,
    /// jumps to a fresh header, and the header carries one phi per *scalar*
    /// carried slot, each seeded with the entry arm `(entry, param)`. Returns
    /// the values the body reads instead of the raw params (a scalar's phi
    /// output, an aggregate's stable-slot pointer). An input arity of 0 yields
    /// a header with zero phis (just a back-edge target), handled without
    /// special-casing.
    ///
    /// When `stage_aggregates` is on (the user self-tail-call loop, R1a), each
    /// aggregate-typed carried slot instead gets an entry-hoisted stable slot
    /// (no header phi, R2), an entry-arm init blit copying the incoming param
    /// into it (R3), and a staging temp for the back-edge read-before-write
    /// copy (R4). When it is off (the two fused destructor synthesizers), every
    /// slot takes the scalar path, keeping their lowering byte-for-byte.
    ///
    /// A base case that returns the carried aggregate returns a pointer into
    /// this frame's stable slot; that is safe only because an aggregate return
    /// lowers to `ret %ptr` under a `:S`/`:E`/`:A` return type and QBE copies
    /// the aggregate out by value at the boundary, as the by-value
    /// aggregate-return ABI already relies on.
    fn begin_loop(&mut self, params: &[Value], stage_aggregates: bool) -> Vec<Value> {
        let entry = self.cur_id;
        let header = self.fresh_block();
        self.seal_block(Terminator::Jmp(header));
        self.start_block(header);
        self.header = Some(header);
        self.entry_block = Some(entry);
        let mut outs = Vec::with_capacity(params.len());
        for &p in params {
            let ty = self.value_type(p);
            if stage_aggregates && is_aggregate(ty) {
                // R1: one entry-hoisted stable slot (the pointer the body reads)
                // and one staging temp per aggregate slot; both route through
                // `push_alloc` into the already-sealed entry block.
                let size = self.value_size(ty);
                let stable = self.alloc_aggregate(ty);
                let temp = self.alloc_aggregate(ty);
                // R3: seed the stable slot with the incoming param once, before
                // the loop runs, so iteration 1 reads an initialised value. A
                // zero-size aggregate has no bytes to copy.
                if size > 0 {
                    self.push_alloc(Instr::Blit(p, stable, size));
                }
                self.carried_slots
                    .push(CarriedSlot::Aggregate { stable, temp, size });
                outs.push(stable);
            } else {
                let out = self.fresh_value(ty);
                self.push_instr(Instr::Phi(out, vec![(entry, p)]));
                self.carried_slots.push(CarriedSlot::Scalar { phi: out });
                outs.push(out);
            }
        }
        outs
    }

    /// R8/R4: after the body lowers, finalize the loop. A scalar slot gets each
    /// collected back-edge's operand appended to its header phi. An aggregate
    /// slot instead gets a read-before-write staging blit pair appended to each
    /// back-edge's predecessor block: a forwarded-in-place arg (exactly its own
    /// stable slot) emits nothing, every other arg is snapshotted into its temp
    /// (read phase) before being stored into its stable slot (write phase), so
    /// an arg that reads a stable slot (a swap) or points into one (an interior
    /// `field_value` pointer) is copied out before any store lands, with no
    /// aliasing analysis. The scalar phi back-patch mutates the header while the
    /// staging blits append to predecessor blocks, so the two run as separate
    /// passes rather than under one borrow.
    fn finalize_loop(&mut self) {
        let header = self.header.expect("finalize_loop: loop mode");
        let slots = mem::take(&mut self.carried_slots);
        let back_edges = mem::take(&mut self.back_edges);
        // Pass 1: scalar phi back-patch, header block only.
        let block = self
            .blocks
            .iter_mut()
            .find(|b| b.id == header)
            .expect("header block");
        for instr in &mut block.instrs {
            if let Instr::Phi(v, arms) = instr {
                if let Some(slot) = slots
                    .iter()
                    .position(|s| matches!(s, CarriedSlot::Scalar { phi } if *phi == *v))
                {
                    for (pred, vals) in &back_edges {
                        arms.push((*pred, vals[slot]));
                    }
                }
            }
        }
        // Pass 2: aggregate staging blits, per predecessor block. All read-phase
        // snapshots precede all write-phase stores; the predecessor is already
        // sealed with its `Jmp` to the header, so appending to `block.instrs`
        // lands the blits before the stored terminator.
        for (pred, vals) in &back_edges {
            let mut reads = Vec::new();
            let mut writes = Vec::new();
            for (slot, meta) in slots.iter().enumerate() {
                if let CarriedSlot::Aggregate { stable, temp, size } = *meta {
                    if size == 0 || vals[slot] == stable {
                        continue;
                    }
                    reads.push(Instr::Blit(vals[slot], temp, size));
                    writes.push(Instr::Blit(temp, stable, size));
                }
            }
            if reads.is_empty() {
                continue;
            }
            let block = self
                .blocks
                .iter_mut()
                .find(|b| b.id == *pred)
                .expect("back-edge predecessor block");
            block.instrs.append(&mut reads);
            block.instrs.append(&mut writes);
        }
    }

    fn lower_terms(&mut self, terms: &[Term], tail: bool) {
        // Only the final term of a body can be in tail position (R1); a term
        // followed by any further term is not. This positional `tail` threading
        // is the same syntactic rule as the checker's `tail_position_calls`
        // (src/check.rs); the two must stay in lockstep if the rule changes.
        let last = terms.len().wrapping_sub(1);
        for (i, term) in terms.iter().enumerate() {
            self.lower_term(term, tail && i == last);
        }
    }

    fn lower_term(&mut self, term: &Term, tail: bool) {
        match &term.kind {
            TermKind::IntLit(n) => {
                let v = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(v, *n));
                self.const_vals.insert(v, *n);
                self.stack.push(v);
            }
            TermKind::FloatLit(x) => {
                let v = self.fresh_value(IrType::Float { bits: 64 });
                self.push_instr(Instr::ConstF(v, *x));
                self.stack.push(v);
            }
            TermKind::BoolLit(b) => {
                let v = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Const(v, if *b { 1 } else { 0 }));
                self.stack.push(v);
            }
            TermKind::StrLit(s) => {
                let v = self.fresh_value(IrType::Str);
                self.push_instr(Instr::StrLit(v, s.clone()));
                self.stack.push(v);
            }
            TermKind::Call(name) => self.lower_call(name, term.span, tail),
            TermKind::Bind(names) => {
                // R10: a binding is a compile-time rebinding of SSA values, so
                // it emits nothing. Leftmost name takes the deepest value.
                let bound = self.stack.split_off(self.stack.len() - names.len());
                for (name, value) in names.iter().zip(bound) {
                    self.locals.push((name.clone(), value));
                }
            }
            TermKind::If {
                then_branch,
                else_branch,
                ..
            } => self.lower_if(then_branch, else_branch, tail),
            // R12: a quotation literal interns its body and lowers to a phantom
            // `Value` with a placeholder `IrType` and *no* `Instr`. The checker
            // guarantees this phantom reaches only `call`/`times`/shuffle/bind
            // (R7's join rejection keeps it out of a `Phi`), so it never enters
            // an operand, terminator, or runtime code value. `I64` is the
            // plainest non-aggregate placeholder (the IR side has no
            // `if`-condition concern, so the checker's `Cstr` choice does not
            // bind here).
            TermKind::Quotation(body) => {
                let id = QuotId(self.quot_defs.len());
                self.quot_defs.push(body.clone());
                let v = self.fresh_value(IrType::I64);
                self.quot_bodies.insert(v, id);
                self.stack.push(v);
            }
        }
    }

    fn lower_call(&mut self, name: &str, span: Span, tail: bool) {
        let line = span.line;
        if let Some(&(_, value)) = self.locals.iter().find(|(n, _)| n == name) {
            self.stack.push(value); // i64 is Copy; reuse the value id.
            return;
        }
        // R14/R11: a call to a polymorphic word resolves entirely through the
        // instantiation table keyed by this call site's span, never the
        // name-keyed `env`/`resolve` (which cannot distinguish one θ from
        // another). This is checked before the builtin/user dispatch below
        // because a polymorphic callee is always a user word whose name is
        // none of the builtins.
        if let Some(inst) = self.instantiations.get(&span).cloned() {
            self.lower_poly_call(&inst);
            return;
        }
        match name {
            // R13: `call`-of-literal fusion. Pop the phantom quotation `Value`,
            // resolve its body, and lower the body's terms in place, emitting
            // no `Instr::Call` and creating no runtime code value: `[ 1 + ]
            // call` lowers exactly as `1 +` (D5). `tail = false` is
            // load-bearing: the checker never sanctions a spliced term as a
            // self-tail call (R6/R13), so lowering must not back-edge here.
            "call" => {
                let v = self.stack.pop().expect("call: quotation on stack");
                let id = self.quot_bodies[&v];
                let body = self.quot_defs[id.0].clone();
                // The body is a block: a name it binds is out of scope after
                // the splice, and the front-first local resolver would else
                // read a stale entry on a later same-named bind. Mirror the
                // `if` arm's save-and-truncate.
                let locals_depth = self.locals.len();
                self.lower_terms(&body, false);
                self.locals.truncate(locals_depth);
            }
            // R14: `times` lowers into a constant-stack loop, reusing
            // `begin_loop`/`finalize_loop` (D6). A synthesized index drives a
            // header `Jnz(index < count)`; the body reads the index as its top
            // input and returns the row on the back-edge (R18). `tail = false`
            // for the same reason as `call`.
            "times" => {
                // R14 step 0: the checker rejects a nested `times` (R18), so no
                // loop is open here; a `debug_assert` records that guarantee.
                debug_assert!(
                    self.header.is_none(),
                    "checker (R18) rejects a `times` nested in a loop"
                );
                // R15: `finalize_loop` clears only `carried_slots`/`back_edges`,
                // never `header`/`entry_block`, so save all four and restore
                // them after the loop, or a later `Alloc` in the same word
                // would wrongly hoist into this dead `times` entry block.
                let saved_header = self.header;
                let saved_entry = self.entry_block;
                let saved_carried = mem::take(&mut self.carried_slots);
                let saved_back_edges = mem::take(&mut self.back_edges);

                let qv = self.stack.pop().expect("times: quotation on stack");
                let id = self.quot_bodies[&qv];
                let body = self.quot_defs[id.0].clone();
                let count = self.stack.pop().expect("times: count on stack");

                // Synthesize the induction variable seeded 0; the row is the
                // remaining stack. `stage_aggregates = true` (R17): a carried
                // aggregate rides slice 3's entry-hoisted stable slot, and the
                // index gets a scalar phi.
                let seed = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(seed, 0));
                self.const_vals.insert(seed, 0);
                let mut params = mem::take(&mut self.stack);
                params.push(seed);
                let outs = self.begin_loop(&params, true);
                let index_phi = *outs.last().expect("times: index phi");
                let row_phis: Vec<Value> = outs[..outs.len() - 1].to_vec();

                // Header (current after `begin_loop`): loop while index < count.
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Lt, index_phi, count));
                let body_block = self.fresh_block();
                let exit_block = self.fresh_block();
                self.seal_block(Terminator::Jnz(cmp, body_block, exit_block));

                // Body: the row plus the index (top input), spliced `tail =
                // false`. `entry_block` stays `Some` across the splice, so an
                // aggregate the body constructs hoists its `Alloc` into the
                // entry block (R17), not the per-iteration body block.
                self.start_block(body_block);
                self.terminated = false;
                self.stack = row_phis;
                self.stack.push(index_phi);
                let locals_depth = self.locals.len();
                self.lower_terms(&body, false);
                self.locals.truncate(locals_depth);

                // Back-edge: the body's result row plus index + 1.
                let one = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(one, 1));
                self.const_vals.insert(one, 1);
                let index_next = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Bin(index_next, BinOp::Add, index_phi, one));
                // With `tail = false` and no `Return` in a body, nothing can
                // terminate the body block, so a double seal is impossible.
                debug_assert!(
                    !self.terminated,
                    "a `tail = false` `times` body cannot terminate"
                );
                let mut args = mem::take(&mut self.stack);
                args.push(index_next);
                self.back_edges.push((self.cur_id, args));
                self.seal_block(Terminator::Jmp(self.header.expect("times loop header")));

                // Back-patch the scalar phis (row scalars + index) and append
                // the aggregate staging blits on the back-edge (unchanged from
                // slice 3).
                self.finalize_loop();

                // Exit: the carried row (scalar header-phi outputs / aggregate
                // stable slots), minus the trailing index. Reset `terminated`
                // (the body seal set it) or every term after the `times` is
                // silently dropped.
                self.start_block(exit_block);
                self.terminated = false;
                let mut exit_stack = outs;
                exit_stack.pop();
                self.stack = exit_stack;

                // R15: restore the pre-`times` loop state so the `times`
                // composes with a later `Alloc` or a second sequential `times`.
                self.header = saved_header;
                self.entry_block = saved_entry;
                self.carried_slots = saved_carried;
                self.back_edges = saved_back_edges;
            }
            "dup" => {
                let top = *self.stack.last().expect("dup: non-empty stack");
                // A scalar is `Copy`: reuse the value id (dup emits nothing). A
                // struct or enum is copied by value: alloc a fresh slot and
                // blit the bytes, so mutating the copy leaves the original
                // intact (an enum is all-Copy too, D3).
                match self.value_type(top) {
                    IrType::Struct(id) => {
                        let copy = self.alloc_struct(id);
                        let size = self.structs.layouts[id.index()].size;
                        if size > 0 {
                            self.push_instr(Instr::Blit(top, copy, size));
                        }
                        self.stack.push(copy);
                    }
                    IrType::Enum(id) => {
                        let copy = self.alloc_enum(id);
                        let size = self.enums.layouts[id.index()].size;
                        if size > 0 {
                            self.push_instr(Instr::Blit(top, copy, size));
                        }
                        self.stack.push(copy);
                    }
                    IrType::Array(id) => {
                        let copy = self.alloc_array(id);
                        let size = self.arrays.layouts[id.index()].size;
                        if size > 0 {
                            self.push_instr(Instr::Blit(top, copy, size));
                        }
                        self.stack.push(copy);
                    }
                    _ => self.stack.push(top),
                }
            }
            "drop" => {
                let v = self.stack.pop().expect("drop: non-empty stack");
                self.emit_drop(v);
            }
            "swap" => {
                let n = self.stack.len();
                self.stack.swap(n - 1, n - 2);
            }
            "over" => {
                let below = self.stack[self.stack.len() - 2];
                self.stack.push(below);
            }
            "rot" => {
                // a b c -> b c a
                let n = self.stack.len();
                let a = self.stack[n - 3];
                self.stack[n - 3] = self.stack[n - 2];
                self.stack[n - 2] = self.stack[n - 1];
                self.stack[n - 1] = a;
            }
            "+" | "-" | "*" | "/" | "mod" | "and" | "or" | "xor" | "shl" | "shr" => {
                let op = match name {
                    "+" => BinOp::Add,
                    "-" => BinOp::Sub,
                    "*" => BinOp::Mul,
                    "/" => BinOp::Div,
                    "mod" => BinOp::Rem,
                    "and" => BinOp::And,
                    "or" => BinOp::Or,
                    "xor" => BinOp::Xor,
                    "shl" => BinOp::Shl,
                    _ => BinOp::Shr,
                };
                let rhs = self.stack.pop().expect("bin: rhs");
                let lhs = self.stack.pop().expect("bin: lhs");
                // Arithmetic/bitwise ops are homogeneous in their result
                // (checker-guaranteed): the result carries the lhs's type, so
                // the backend picks its width. `shl`/`shr`'s rhs is always an
                // `i64` count, not the lhs's type.
                let ty = self.value_type(lhs);
                let v = self.fresh_value(ty);
                self.push_instr(Instr::Bin(v, op, lhs, rhs));
                self.stack.push(v);
            }
            "not" => {
                // No unary QBE op: `not` is `xor operand, mask`. On an integer,
                // complement is `xor operand, -1` at the operand's own width
                // (`-1` is all-ones at any width in two's complement, so it
                // works whether the register is `w` or `l`). On a `bool`,
                // `not` is logical negation of a canonical 0/1 value, which
                // flips only the low bit (`xor operand, 1`); `xor -1` would
                // give -1/-2, not 0/1.
                let operand = self.stack.pop().expect("not: operand");
                let ty = self.value_type(operand);
                let mask: i64 = if ty == IrType::Bool { 1 } else { -1 };
                let mask_v = self.fresh_value(ty);
                self.push_instr(Instr::Const(mask_v, mask));
                let v = self.fresh_value(ty);
                self.push_instr(Instr::Bin(v, BinOp::Xor, operand, mask_v));
                self.stack.push(v);
            }
            "=" | "<" | ">" | "<=" | ">=" | "<>" => {
                let op = match name {
                    "=" => CmpOp::Eq,
                    "<" => CmpOp::Lt,
                    ">" => CmpOp::Gt,
                    "<=" => CmpOp::Le,
                    ">=" => CmpOp::Ge,
                    _ => CmpOp::Ne,
                };
                let rhs = self.stack.pop().expect("cmp: rhs");
                let lhs = self.stack.pop().expect("cmp: lhs");
                let v = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(v, op, lhs, rhs));
                self.stack.push(v);
            }
            // R12 (S6): `max` over the integer tower, inline compare-and-select
            // (`Cmp(Gt)` plus a two-block phi-join), no `Instr::Call`, no
            // monomorphization.
            "max" => {
                let rhs = self.stack.pop().expect("max: rhs");
                let lhs = self.stack.pop().expect("max: lhs");
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Gt, lhs, rhs));
                let v = self.emit_select(cmp, |_| lhs, |_| rhs);
                self.stack.push(v);
            }
            // R13 (S6): `max-total` over `f32`/`f64`, ordered by the
            // `total_cmp` bit-pattern rule (map each operand's IEEE bits to a
            // monotone unsigned key — flip every bit if the sign bit is set,
            // else flip only the sign bit — then integer-compare the keys),
            // so no float `>` is ever emitted.
            "max-total" => {
                let rhs = self.stack.pop().expect("max-total: rhs");
                let lhs = self.stack.pop().expect("max-total: lhs");
                let bits: u8 = match self.value_type(lhs) {
                    IrType::Float { bits } => bits,
                    other => unreachable!("checked: max-total operand is a float, got {other:?}"),
                };
                let lhs_key = self.total_order_key(lhs, bits);
                let rhs_key = self.total_order_key(rhs, bits);
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Gt, lhs_key, rhs_key));
                let v = self.emit_select(cmp, |_| lhs, |_| rhs);
                self.stack.push(v);
            }
            "." => {
                let v = self.stack.pop().expect("print: value");
                self.push_instr(Instr::Print(v));
            }
            "fill" => self.lower_array_word(name),
            "len" => {
                let top = *self.stack.last().expect("len: operand");
                if self.value_type(top) == IrType::Str {
                    // R8: consuming, unlike the array `len` fold: the
                    // length is carried at runtime, not derivable from the
                    // type.
                    self.stack.pop();
                    let v = self.fresh_value(IrType::Usize);
                    self.push_instr(Instr::StrLen(v, top));
                    self.stack.push(v);
                } else {
                    self.lower_array_word(name);
                }
            }
            "cstr" => {
                // R7: discard the length, keep the bytes pointer.
                let s = self.stack.pop().expect("cstr: str operand");
                let v = self.fresh_value(IrType::Cstr);
                self.push_instr(Instr::StrPtr(v, s));
                self.stack.push(v);
            }
            "^" | "^>" | "^|>" => self.lower_owned_cell_word(name),
            "@" | "!" | "+!" => self.lower_access_word(name),
            _ => {
                // R19: a call to a monomorphic combinator is inlined, not
                // lowered to an `Instr::Call` -- the callee mints no `IrFunc`
                // (R20), so its only reachable form is this splice. The
                // caller's quotation literals sit on `self.stack` as phantom
                // `Value`s already (a `TermKind::Quotation` earlier in this
                // body recorded each `Value -> QuotId`), so the spliced body's
                // own `call`/`times` resolves them with no extra plumbing.
                // `tail = false` and the locals-truncate mirror the `call`
                // splice above. Checked before the `&`/conversion/struct
                // dispatch since a combinator name is an ordinary word name.
                if let Some(body) = self.combinators.get(name) {
                    // R18/R21: alpha-rename the callee body identically to the
                    // checker, so its `| ... |` locals are fresh and a
                    // passed-down literal keeps its lexical capture under
                    // transitive inlining.
                    let uid = self.inline_uid;
                    self.inline_uid += 1;
                    let body = crate::ast::alpha_rename_locals(body, uid);
                    let locals_depth = self.locals.len();
                    self.lower_terms(&body, false);
                    self.locals.truncate(locals_depth);
                    return;
                }
                // Every `&`-led word: the two prefix borrow operators and the
                // reference-mode accessor family.
                if name.starts_with('&') {
                    self.lower_reference_word(name, line);
                    return;
                }
                // A conversion word `>iN`/`>uN`/`>f32`/`>f64`
                // (checker-guaranteed numeric source): pop one, push the
                // target-typed result. The backend reads the two `IrType`s to
                // pick the int/float conversion op (R18).
                if let Some(target) = name
                    .strip_prefix('>')
                    .filter(|r| !r.is_empty())
                    .and_then(Type::from_name)
                    .filter(Type::is_numeric)
                {
                    let src = self.stack.pop().expect("conv: source");
                    let dst = self.fresh_value(ir_type_of(target));
                    self.push_instr(Instr::Conv(dst, src));
                    self.stack.push(dst);
                    return;
                }
                // A generated struct word (`S`/`S>`/`S>fi`/`S<fi`/`S|>fi`) lowers to
                // alloc/blit/field-load-store inline, not a normal call.
                if let Some(&sw) = self.structs.words.get(name) {
                    self.lower_struct_word(sw);
                    return;
                }
                // A variant constructor lowers to alloc + tag store + field
                // stores inline, parallel to a struct constructor (R14/R15).
                if let Some(&ew) = self.enums.words.get(name) {
                    self.lower_enum_word(ew);
                    return;
                }
                // R7: a tail-position self-call is a back-edge to the loop
                // header, not a real call. `self.header` is `Some` iff the word
                // is self-tail-recursive (R6), and `tail` marks the syntactic
                // tail position (R1); a non-tail self-call (R10) falls through
                // to the ordinary `Instr::Call` below. Pop the args as the
                // back-edge phi operands (one per carried slot; a self-call's
                // input arity is the word's own signature, so the count always
                // matches the header phi count) and jump.
                //
                // R11: the back-edge is the defined destructor insertion point
                // for this iteration's non-forwarded affine values; in Phase 2
                // every type is `Copy`, so the drop set is empty and no drop
                // glue is emitted here.
                if tail && self.header.is_some() && name == self.cur_word_name {
                    let (in_arity, ..) = *self.env.get(name).expect("checked user word exists");
                    let split = self.stack.len() - in_arity;
                    let args = self.stack.split_off(split);
                    self.back_edges.push((self.cur_id, args));
                    self.seal_block(Terminator::Jmp(self.header.expect("loop header")));
                    self.terminated = true;
                    return;
                }
                let (in_arity, out_arity, ret_ty) =
                    *self.env.get(name).expect("checked user word exists");
                let split = self.stack.len() - in_arity;
                let args = self.stack.split_off(split);
                // R11: a multi-output callee returns one bundle, unpacked back
                // onto the stack below, so the lowering stack matches the
                // stack the checker verified. The discriminator is the
                // bundle's own flag, not `out_arity >= 2`: the REPL's env
                // derives a multi-output `ret_ty` from the first output alone
                // and interns no bundle, and must not enter this branch.
                let bundle = match ret_ty {
                    Some(IrType::Struct(id)) if self.structs.layouts[id.index()].bundle => Some(id),
                    _ => None,
                };
                let ret = if out_arity == 1 || bundle.is_some() {
                    Some(self.fresh_value(ret_ty.unwrap_or(IrType::I64)))
                } else {
                    None
                };
                let sym = (self.resolve)(name);
                self.push_instr(Instr::Call(ret, sym, args));
                if let Some(v) = ret {
                    self.stack.push(v);
                }
                if let Some(id) = bundle {
                    self.unpack_bundle(id);
                }
            }
        }
    }

    /// Push a reference `Value` (always `IrType::Ptr`) and record what it
    /// points at, since the `IrType` deliberately no longer says.
    fn push_reference(&mut self, ptr: Value, referent: IrType) {
        self.ref_inner.insert(ptr, referent);
        self.stack.push(ptr);
    }

    /// The referent shape of a reference `Value`.
    fn referent_of(&self, ptr: Value) -> IrType {
        *self
            .ref_inner
            .get(&ptr)
            .expect("checked: every reference value records its referent")
    }

    /// Lower a `&`-led word. No new `Instr` variant: a struct
    /// field projection is a `PtrOffset`, an array element projection an
    /// `ElemAddr` behind a runtime bounds guard, and a cell payload
    /// projection a `Load` of the pointer the place holds.
    fn lower_reference_word(&mut self, name: &str, line: u32) {
        let mutable = name.starts_with("&!");
        let rest = &name[if mutable { 2 } else { 1 }..];
        match rest {
            ">" => {
                let index = self.stack.pop().expect("&>: index");
                let base = self.stack.pop().expect("&>: array reference");
                let IrType::Array(id) = self.referent_of(base) else {
                    unreachable!("checked: `&>`'s receiver references an array")
                };
                let (stride, elem, count) = self.array_parts(id);
                self.bounds_check(index, count, line);
                let addr = self.elem_addr(base, index, stride);
                self.push_reference(addr, elem);
            }
            "^" => {
                let base = self.stack.pop().expect("&^: cell reference");
                let IrType::OwnedCell(id) = self.referent_of(base) else {
                    unreachable!("checked: `&^`'s receiver references an owning cell")
                };
                let payload = self.cells.payload[id.index()];
                // The place holds the cell's heap pointer; the payload lives
                // at that pointer, so the projection reads it out.
                let cell_ptr = self.fresh_value(IrType::Ptr);
                self.push_instr(Instr::Load(cell_ptr, base));
                self.push_reference(cell_ptr, payload);
            }
            _ => {
                if let Some(&StructWord::Get(id, fi)) = self.structs.words.get(rest) {
                    let base = self.stack.pop().expect("field projection: receiver");
                    let field = self.structs.layouts[id.index()].fields[fi];
                    let addr = self.field_ptr(base, field.offset);
                    self.push_reference(addr, field.ty);
                    return;
                }
                let value = self
                    .locals
                    .iter()
                    .find(|(n, _)| n == rest)
                    .map(|(_, v)| *v)
                    .expect("checked: a borrow's operand is a local");
                self.lower_borrow(value);
            }
        }
    }

    /// Borrow a local. An aggregate local's own value *is* a pointer to
    /// its storage, so the borrow is that pointer retyped as an opaque handle.
    /// A cell local's value is the heap pointer itself, an SSA temporary with
    /// no address of its own; `&^`/`&!^` reads a cell reference by loading the
    /// pointer out of the place holding it, so borrowing a cell local first
    /// gives it a place.
    fn lower_borrow(&mut self, value: Value) {
        let referent = self.value_type(value);
        let ptr = match referent {
            IrType::OwnedCell(_) => {
                let slot = self.fresh_value(IrType::Ptr);
                self.push_alloc(Instr::Alloc(slot, WORD_WIDTH, WORD_WIDTH));
                self.push_instr(Instr::Store(slot, value));
                slot
            }
            _ => {
                let p = self.fresh_value(IrType::Ptr);
                self.push_instr(Instr::PtrOffset(p, value, 0));
                p
            }
        };
        self.push_reference(ptr, referent);
    }

    /// `@` fetches through a reference, `!` stores, `+!` adds in place.
    /// The referent is checker-guaranteed `Copy`; a Copy *aggregate* is a real
    /// case, taking the `Alloc`+`Blit` / `Blit` path `dup` already uses for
    /// the same shape of copy.
    fn lower_access_word(&mut self, name: &str) {
        match name {
            "@" => {
                let ptr = self.stack.pop().expect("@: reference");
                let referent = self.referent_of(ptr);
                match referent {
                    IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                        let dst = self.alloc_aggregate(referent);
                        let size = self.value_size(referent);
                        if size > 0 {
                            self.push_instr(Instr::Blit(ptr, dst, size));
                        }
                        self.stack.push(dst);
                    }
                    _ => {
                        let v = self.fresh_value(referent);
                        self.push_instr(Instr::FieldLoad(v, ptr));
                        self.stack.push(v);
                    }
                }
            }
            "!" => {
                let val = self.stack.pop().expect("!: value");
                let ptr = self.stack.pop().expect("!: reference");
                let referent = self.referent_of(ptr);
                match referent {
                    IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                        let size = self.value_size(referent);
                        if size > 0 {
                            self.push_instr(Instr::Blit(val, ptr, size));
                        }
                    }
                    _ => self.push_instr(Instr::FieldStore(ptr, val)),
                }
            }
            "+!" => {
                let val = self.stack.pop().expect("+!: addend");
                let ptr = self.stack.pop().expect("+!: reference");
                let referent = self.referent_of(ptr);
                let cur = self.fresh_value(referent);
                self.push_instr(Instr::FieldLoad(cur, ptr));
                let sum = self.fresh_value(referent);
                self.push_instr(Instr::Bin(sum, BinOp::Add, cur, val));
                self.push_instr(Instr::FieldStore(ptr, sum));
            }
            _ => unreachable!("lower_access_word only handles @/!/+!"),
        }
    }

    /// Alloc a fresh frame slot for struct `id`'s aggregate and yield it as a
    /// `Struct`-typed value (a pointer to the storage).
    fn alloc_struct(&mut self, id: StructId) -> Value {
        let (size, align) = {
            let l = &self.structs.layouts[id.index()];
            (l.size, l.align)
        };
        let v = self.fresh_value(IrType::Struct(id));
        self.push_alloc(Instr::Alloc(v, size, align));
        v
    }

    /// Alloc a fresh frame slot for enum `id`'s tagged aggregate and yield it
    /// as an `Enum`-typed value (a pointer to the storage), mirroring
    /// `alloc_struct`.
    fn alloc_enum(&mut self, id: EnumId) -> Value {
        let (size, align) = {
            let l = &self.enums.layouts[id.index()];
            (l.size, l.align)
        };
        let v = self.fresh_value(IrType::Enum(id));
        self.push_alloc(Instr::Alloc(v, size, align));
        v
    }

    /// Alloc a fresh frame slot for array `id`'s inline aggregate and yield it
    /// as an `Array`-typed value (a pointer to the storage), mirroring
    /// `alloc_struct`/`alloc_enum`.
    fn alloc_array(&mut self, id: ArrayId) -> Value {
        let (size, align) = {
            let l = &self.arrays.layouts[id.index()];
            (l.size, l.align)
        };
        let v = self.fresh_value(IrType::Array(id));
        self.push_alloc(Instr::Alloc(v, size, align));
        v
    }

    /// The `(stride, element type, count)` of array `id`, copied out of the
    /// layout registry so the caller can then emit against `&mut self`.
    fn array_parts(&self, id: ArrayId) -> (u32, IrType, u32) {
        let l = &self.arrays.layouts[id.index()];
        (l.stride, l.elem, l.count)
    }

    /// The `ArrayId` whose layout has element `elem` and `count`: `fill`'s
    /// target shape, already interned by the checker (R10), found by structural
    /// match on the combined registry.
    fn array_id_of(&self, elem: IrType, count: u32) -> ArrayId {
        let idx = self
            .arrays
            .layouts
            .iter()
            .position(|l| l.elem == elem && l.count == count)
            .expect("fill's array shape is interned by the checker");
        ArrayId::from_index(idx)
    }

    /// The exact byte size of a value of `ty` (an aggregate's whole size, a
    /// scalar's width) — the blit length for a `fill` aggregate element.
    fn value_size(&self, ty: IrType) -> u32 {
        match ty {
            IrType::Struct(id) => self.structs.layouts[id.index()].size,
            IrType::Enum(id) => self.enums.layouts[id.index()].size,
            IrType::Array(id) => self.arrays.layouts[id.index()].size,
            other => scalar_size_align(other).0,
        }
    }

    /// `dst = base + index*stride`, a `Ptr` (R17): every caller is a
    /// reference projection, so a `FieldLoad`/`FieldStore` through `dst`
    /// always follows.
    fn elem_addr(&mut self, base: Value, index: Value, stride: u32) -> Value {
        let dst = self.fresh_value(IrType::Ptr);
        self.push_instr(Instr::ElemAddr(dst, base, index, stride as i64));
        dst
    }

    /// Store `val` (of element type `elem`) at element place `fptr`: a
    /// width-exact scalar `FieldStore`, or an aggregate `Blit` of the whole
    /// element. `fill`'s unrolled stores are the only caller.
    fn store_elem(&mut self, fptr: Value, val: Value, elem: IrType) {
        match elem {
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                let size = self.value_size(elem);
                if size > 0 {
                    self.push_instr(Instr::Blit(val, fptr, size));
                }
            }
            _ => self.push_instr(Instr::FieldStore(fptr, val)),
        }
    }

    /// Lower an array word inline: `fill` = alloc + N unrolled stores;
    /// `len` = a constant `usize` from the layout, non-consuming.
    fn lower_array_word(&mut self, name: &str) {
        match name {
            "fill" => {
                let count_v = self.stack.pop().expect("fill: count");
                let n = *self
                    .const_vals
                    .get(&count_v)
                    .expect("fill's count is a checked literal") as u32;
                let elem_v = self.stack.pop().expect("fill: element");
                let elem = self.value_type(elem_v);
                let id = self.array_id_of(elem, n);
                let (stride, _, _) = self.array_parts(id);
                let dst = self.alloc_array(id);
                for i in 0..n {
                    let fptr = self.field_ptr(dst, i * stride);
                    self.store_elem(fptr, elem_v, elem);
                }
                self.stack.push(dst);
            }
            "len" => {
                // Non-consuming (R10): the array stays; the constant folds in.
                let array = *self.stack.last().expect("len: array");
                let id = match self.value_type(array) {
                    IrType::Array(id) => id,
                    _ => unreachable!("checked: len's operand is an array"),
                };
                let (_, _, count) = self.array_parts(id);
                let v = self.fresh_value(IrType::Usize);
                self.push_instr(Instr::Const(v, count as i64));
                self.stack.push(v);
            }
            _ => unreachable!("lower_array_word only handles fill/len"),
        }
    }

    /// The `OwnedCellId` whose payload shape is `payload`: `^`'s target shape,
    /// already interned by the checker, found by structural match on the
    /// combined registry, mirroring `array_id_of`.
    fn cell_id_of(&self, payload: IrType) -> OwnedCellId {
        let idx = self
            .cells
            .payload
            .iter()
            .position(|&p| p == payload)
            .expect("^'s payload shape is interned by the checker");
        OwnedCellId::from_index(idx)
    }

    /// Alloc a fresh frame slot for aggregate `ty` (a `Struct`/`Enum`/`Array`),
    /// dispatching to the matching per-kind helper. Shared by a cell's
    /// unwrap/peek, which must never alias the cell's own storage.
    fn alloc_aggregate(&mut self, ty: IrType) -> Value {
        match ty {
            IrType::Struct(id) => self.alloc_struct(id),
            IrType::Enum(id) => self.alloc_enum(id),
            IrType::Array(id) => self.alloc_array(id),
            _ => unreachable!("alloc_aggregate: not an aggregate IrType"),
        }
    }

    /// Never alias the cell: an aggregate payload gets a fresh frame slot and
    /// a `Blit` out, so a later `free` never leaves the caller holding a
    /// dangling interior pointer.
    fn load_owned_payload(&mut self, cell_ptr: Value, payload_ty: IrType) -> Value {
        match payload_ty {
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                let dst = self.alloc_aggregate(payload_ty);
                let size = self.value_size(payload_ty);
                if size > 0 {
                    self.push_instr(Instr::Blit(cell_ptr, dst, size));
                }
                dst
            }
            _ => {
                let v = self.fresh_value(payload_ty);
                self.push_instr(Instr::FieldLoad(v, cell_ptr));
                v
            }
        }
    }

    /// Drop every linear field of one aggregate level (a struct's own fields,
    /// or an enum variant's, offsets already adjusted) except `skip`, the
    /// field the disposal path continues through. The continuing field
    /// is read after every other read of this level, so it is skipped here
    /// rather than dropped in place.
    fn drop_level_fields(&mut self, base: Value, fields: &[FieldLayout], skip: Option<usize>) {
        for (fi, field) in fields.iter().enumerate() {
            if Some(fi) != skip && field_is_linear(field.ty, self.structs, self.enums, self.arrays)
            {
                let v = self.field_value(base, *field);
                self.emit_drop(v);
            }
        }
    }

    /// One `Unwrap` step: copy the cell's payload out and free the cell.
    ///
    /// **Every read of data held in the payload's frame slot must already be
    /// emitted.** `push_alloc` hoists the copy-out's `Alloc` into the entry
    /// block, so one slot per step site is reused by every iteration and the
    /// copy-out blits the next value over the memory the current one occupies.
    /// A field load, tag read or sibling drop emitted after this call
    /// would read the wrong value, and would do so with the alloc/free trace
    /// still perfectly balanced. A scalar payload (the inner step of `^^Self`)
    /// takes `load_owned_payload`'s plain-`FieldLoad` branch, has no slot, and
    /// so has no ordering hazard of its own.
    fn emit_unwrap(&mut self, cell_ptr: Value, cell: OwnedCellId) -> Value {
        let payload_ty = self.cells.payload[cell.index()];
        let next = self.load_owned_payload(cell_ptr, payload_ty);
        let size = self.value_size(payload_ty);
        let size_v = self.fresh_value(IrType::I64);
        self.push_instr(Instr::Const(size_v, size as i64));
        self.push_instr(Instr::Call(
            None,
            FREE_SYMBOL.to_string(),
            vec![cell_ptr, size_v],
        ));
        next
    }

    /// Emit the rest of a fused destructor loop's iteration from `cur`, whose
    /// own `IrType` names the level the next step reads. An empty `steps` is
    /// the end of one full trip around the path: `cur` is a fresh value
    /// of the loop's own type, so it back-edges to the header.
    fn emit_path_steps(&mut self, cur: Value, steps: &[PathStep]) {
        let Some(first) = steps.first() else {
            self.back_edges.push((self.cur_id, vec![cur]));
            self.seal_block(Terminator::Jmp(self.header.expect("loop header")));
            self.terminated = true;
            return;
        };
        match *first {
            // `cur` is itself the cell (the inner step of `^^Self`).
            PathStep::Unwrap { field: None, cell } => {
                let next = self.emit_unwrap(cur, cell);
                self.emit_path_steps(next, &steps[1..]);
            }
            PathStep::Branch {
                enum_id,
                ref variants,
            } => self.emit_branch(cur, enum_id, variants),
            PathStep::Project { .. } | PathStep::Unwrap { field: Some(_), .. } => {
                // Only a struct level is reached with a field step still
                // pending: an enum expands into a `Branch`, and a cell into a
                // fieldless `Unwrap`.
                let IrType::Struct(id) = self.value_type(cur) else {
                    unreachable!("a field step reads a struct level")
                };
                let fields = self.structs.layouts[id.index()].fields.clone();
                self.emit_field_level(cur, &fields, steps);
            }
        }
    }

    /// Emit `steps` from the aggregate level (`base`, `fields`) their first
    /// step reads: drop that level's other fields, then take the
    /// continuing field byval (`Project`) or through its cell (`Unwrap`).
    fn emit_field_level(&mut self, base: Value, fields: &[FieldLayout], steps: &[PathStep]) {
        let (first, rest) = steps.split_first().expect("a level's path is non-empty");
        let (fi, cell) = match *first {
            PathStep::Project { field } => (field, None),
            PathStep::Unwrap {
                field: Some(field),
                cell,
            } => (field, Some(cell)),
            _ => unreachable!("a level's path starts at one of its own fields"),
        };
        self.drop_level_fields(base, fields, Some(fi));
        let field = self.field_value(base, fields[fi]);
        let next = match cell {
            Some(cell) => self.emit_unwrap(field, cell),
            None => field,
        };
        self.emit_path_steps(next, rest);
    }

    /// Dispatch on `node`'s tag and emit each variant's own continuation: a
    /// variant that does not continue toward `Self` drops its fields and
    /// leaves the loop, one that does walks its own steps and back-edges.
    /// More than one variant may continue, and each back-edges
    /// independently.
    ///
    /// Every variant block resets `terminated` right after `start_block`, so
    /// the trailing seal fires for a base case and is skipped for a block a
    /// back-edge or a nested dispatch already sealed. All arms end
    /// sealed and nothing follows a dispatch in the same sequence, so the
    /// whole `Branch` reports itself terminated to its own caller.
    fn emit_branch(&mut self, node: Value, id: EnumId, variants: &[Option<Vec<PathStep>>]) {
        let payload_offset = self.enums.layouts[id.index()].payload_offset;
        let layouts = self.enums.layouts[id.index()].variants.clone();
        let blocks = self.dispatch_on_tag(node, id);
        for (vi, &block) in blocks.iter().enumerate() {
            self.start_block(block);
            self.terminated = false;
            let fields: Vec<FieldLayout> = layouts[vi]
                .fields
                .iter()
                .map(|field| FieldLayout {
                    offset: payload_offset + field.offset,
                    ..*field
                })
                .collect();
            match &variants[vi] {
                Some(steps) => self.emit_field_level(node, &fields, steps),
                None => self.drop_level_fields(node, &fields, None),
            }
            if !self.terminated {
                self.seal_block(Terminator::Ret(None));
            }
        }
        self.terminated = true;
    }

    /// Store `val` (of `payload_ty`) into the cell at `cell_ptr`: the mirror
    /// of `load_owned_payload`. A scalar payload is a width-exact
    /// `FieldStore`; an aggregate is a `Blit` from its frame slot; a
    /// zero-sized payload writes nothing.
    fn store_owned_payload(&mut self, cell_ptr: Value, val: Value, payload_ty: IrType) {
        match payload_ty {
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                let size = self.value_size(payload_ty);
                if size > 0 {
                    self.push_instr(Instr::Blit(val, cell_ptr, size));
                }
            }
            _ => self.push_instr(Instr::FieldStore(cell_ptr, val)),
        }
    }

    /// `^>` materialises the payload before freeing the cell, so the freed
    /// pointer is never handed to the stack.
    fn lower_owned_cell_word(&mut self, name: &str) {
        match name {
            "^" => {
                let payload_val = self.stack.pop().expect("^: payload");
                let payload_ty = self.value_type(payload_val);
                let id = self.cell_id_of(payload_ty);
                let size = self.value_size(payload_ty);
                let size_v = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(size_v, size as i64));
                let ptr = self.fresh_value(IrType::OwnedCell(id));
                self.push_instr(Instr::Call(
                    Some(ptr),
                    ALLOC_SYMBOL.to_string(),
                    vec![size_v],
                ));
                self.store_owned_payload(ptr, payload_val, payload_ty);
                self.stack.push(ptr);
            }
            "^>" => {
                let cell = self.stack.pop().expect("^>: cell");
                let id = match self.value_type(cell) {
                    IrType::OwnedCell(id) => id,
                    _ => unreachable!("checked: ^>'s operand is a cell"),
                };
                let payload_ty = self.cells.payload[id.index()];
                let val = self.load_owned_payload(cell, payload_ty);
                let size = self.value_size(payload_ty);
                let size_v = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(size_v, size as i64));
                self.push_instr(Instr::Call(
                    None,
                    FREE_SYMBOL.to_string(),
                    vec![cell, size_v],
                ));
                self.stack.push(val);
            }
            "^|>" => {
                // Non-consuming: the cell stays on the stack, the payload
                // copy is pushed atop it.
                let cell = *self.stack.last().expect("^|>: cell");
                let id = match self.value_type(cell) {
                    IrType::OwnedCell(id) => id,
                    _ => unreachable!("checked: ^|>'s operand is a cell"),
                };
                let payload_ty = self.cells.payload[id.index()];
                let val = self.load_owned_payload(cell, payload_ty);
                self.stack.push(val);
            }
            _ => unreachable!("lower_owned_cell_word only handles ^/^>/^|>"),
        }
    }

    /// Emit the runtime bounds guard for a dynamic array index (R19/D6): an
    /// `index < N` compare jumps to the continuation, otherwise a trap block
    /// calls the out-of-bounds helper (a located len+index message to stderr,
    /// then a nonzero exit) so an out-of-range access aborts rather than
    /// corrupting. A checked compile-time literal index (X4, R11) already had
    /// its bounds verified, so it skips the guard entirely and stays
    /// trap-free.
    fn bounds_check(&mut self, index: Value, count: u32, line: u32) {
        if self.const_vals.contains_key(&index) {
            return;
        }
        let n = self.fresh_value(IrType::Usize);
        self.push_instr(Instr::Const(n, i64::from(count)));
        let cond = self.fresh_value(IrType::Bool);
        self.push_instr(Instr::Cmp(cond, CmpOp::Lt, index, n));
        let ok = self.fresh_block();
        let trap = self.fresh_block();
        self.seal_block(Terminator::Jnz(cond, ok, trap));

        // The trap block never falls through: the helper exits, so the `Jmp`
        // to `ok` is an unreachable CFG edge that keeps the block validly
        // terminated regardless of the enclosing word's return type.
        self.start_block(trap);
        let line_v = self.fresh_value(IrType::Usize);
        self.push_instr(Instr::Const(line_v, i64::from(line)));
        let len_v = self.fresh_value(IrType::Usize);
        self.push_instr(Instr::Const(len_v, i64::from(count)));
        self.push_instr(Instr::Call(
            None,
            OOB_TRAP_SYMBOL.to_string(),
            vec![line_v, index, len_v],
        ));
        self.seal_block(Terminator::Jmp(ok));

        self.start_block(ok);
    }

    /// A `Ptr`-typed value for `base + offset` (a scalar field's address).
    fn field_ptr(&mut self, base: Value, offset: u32) -> Value {
        let p = self.fresh_value(IrType::Ptr);
        self.push_instr(Instr::PtrOffset(p, base, offset as i64));
        p
    }

    /// A nested-aggregate field's value: its interior address, typed as the
    /// inner struct/enum. No copy — the owning aggregate is consumed by the
    /// getter/destructure/clause, so aliasing its storage is sound; a later
    /// `dup` or word-return copies the bytes.
    fn field_aggregate_value(&mut self, base: Value, offset: u32, inner: IrType) -> Value {
        let v = self.fresh_value(inner);
        self.push_instr(Instr::PtrOffset(v, base, offset as i64));
        v
    }

    /// Store `val` into field `field` at `fptr`: a width-exact scalar store, or
    /// an aggregate blit for a nested struct/enum field.
    fn store_field(&mut self, fptr: Value, val: Value, field: FieldLayout) {
        match field.ty {
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                if field.size > 0 {
                    self.push_instr(Instr::Blit(val, fptr, field.size));
                }
            }
            _ => self.push_instr(Instr::FieldStore(fptr, val)),
        }
    }

    /// Field `field` of aggregate `base` as a value: a width-exact scalar load,
    /// or the interior pointer as a nested struct/enum value.
    fn field_value(&mut self, base: Value, field: FieldLayout) -> Value {
        match field.ty {
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                self.field_aggregate_value(base, field.offset, field.ty)
            }
            _ => {
                let fptr = self.field_ptr(base, field.offset);
                let v = self.fresh_value(field.ty);
                self.push_instr(Instr::FieldLoad(v, fptr));
                v
            }
        }
    }

    fn load_field_onto_stack(&mut self, base: Value, field: FieldLayout) {
        let v = self.field_value(base, field);
        self.stack.push(v);
    }

    /// Dispatch on `scrutinee`'s runtime tag (enum `id`): seal a compare chain
    /// (`n == 1` short-circuits to a bare `Jmp`; otherwise load the tag once
    /// and `Cmp`/`Jnz` variant-by-variant, the last compare's false edge
    /// falling straight through to the final variant with no default/trap
    /// block) and return one freshly allocated, not-yet-started block per
    /// variant in declaration order. Shared by `lower_clauses` (a clause
    /// word's scrutinee) and `synthesize_enum_destructor` (the same shape,
    /// only what each variant block does next differs).
    fn dispatch_on_tag(&mut self, scrutinee: Value, id: EnumId) -> Vec<BlockId> {
        let (tag_ty, tag_offset, n) = {
            let l = &self.enums.layouts[id.index()];
            (l.tag_ty, l.tag_offset, l.variants.len())
        };
        let variant_ids: Vec<BlockId> = (0..n).map(|_| self.fresh_block()).collect();
        if n == 1 {
            self.seal_block(Terminator::Jmp(variant_ids[0]));
        } else {
            let tag = self.fresh_value(tag_ty);
            let tag_ptr = self.field_ptr(scrutinee, tag_offset);
            self.push_instr(Instr::FieldLoad(tag, tag_ptr));
            for vi in 0..n - 1 {
                let idx_val = self.fresh_value(tag_ty);
                self.push_instr(Instr::Const(idx_val, vi as i64));
                let c = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(c, CmpOp::Eq, tag, idx_val));
                let false_target = if vi == n - 2 {
                    variant_ids[n - 1]
                } else {
                    self.fresh_block()
                };
                self.seal_block(Terminator::Jnz(c, variant_ids[vi], false_target));
                if vi < n - 2 {
                    self.start_block(false_target);
                }
            }
        }
        variant_ids
    }

    /// R5/R12/R16: the universal disposal primitive. On a linear value (a
    /// struct/enum whose `is_linear` is set, or an owning cell) this is a
    /// plain `Call` to the (builtin or synthesized) destructor; a `Copy`
    /// value is discarded with no runtime effect. Shared by `drop`, `S>fi`'s
    /// drop-the-rest, `S<fi`'s drop-on-overwrite, and the synthesized
    /// struct/enum destructors themselves, so "how a value is disposed" lives
    /// in one place.
    fn emit_drop(&mut self, v: Value) {
        match self.value_type(v) {
            // A cell always frees on drop, regardless of its payload's own
            // linearity: the synthesized destructor drops a linear payload
            // first.
            IrType::OwnedCell(id) => {
                let symbol = cell_drop_symbol(id, self.cells.drop_generations[id.index()]);
                self.push_instr(Instr::Call(None, symbol, vec![v]));
            }
            IrType::Struct(id) if self.structs.layouts[id.index()].is_linear => {
                let symbol =
                    struct_drop_symbol(id, self.structs.layouts[id.index()].drop_generation);
                self.push_instr(Instr::Call(None, symbol, vec![v]));
            }
            IrType::Enum(id) if self.enums.layouts[id.index()].is_linear => {
                let symbol = enum_drop_symbol(id, self.enums.layouts[id.index()].drop_generation);
                self.push_instr(Instr::Call(None, symbol, vec![v]));
            }
            IrType::Array(id) if self.arrays.layouts[id.index()].is_linear => unreachable!(
                "checked: a linear array element is rejected wherever an array type is named"
            ),
            _ => {}
        }
    }

    /// R10, callee side: pop the top `n` stack values into a fresh bundle of
    /// `id` (deepest output first, matching the field order the checker
    /// interned) and yield it as the word's single returned value. Literally
    /// the struct constructor, which is the point: the bundle is the struct
    /// users hand-wrote before this ABI existed.
    fn pack_bundle(&mut self, id: StructId) -> Value {
        self.lower_struct_word(StructWord::Construct(id));
        self.stack.pop().expect("pack: the bundle just constructed")
    }

    /// R11, caller side: replace the returned bundle on the stack with its
    /// fields, deepest first — the exact reverse of `pack_bundle`, through the
    /// same destructure a generated `S>` uses, so a linear field is moved out
    /// of the shell exactly as `S>` moves one.
    fn unpack_bundle(&mut self, id: StructId) {
        self.lower_struct_word(StructWord::Destructure(id));
    }

    /// R14/R11: lower a call to a polymorphic word through its per-call-site
    /// `CallInst`. The mangled symbol (not `(self.resolve)(name)`), the
    /// per-θ output arity, and the bundle come straight from the table, so
    /// two instantiations of one word reach two distinct symbols and two
    /// distinct return shapes even though `env`/`resolve` are name-keyed. The
    /// input arity is name-constant across θ and read from `poly_arities`; the
    /// row prefix, if any, stays on the stack below the popped args (S2). The
    /// bundle unpack is the same pack/unpack path a monomorphic multi-output
    /// call takes (R10/R11), so a row-variable-expanded count lowers
    /// identically to a fixed multi-output word — D4's one mechanism.
    fn lower_poly_call(&mut self, inst: &CallInst) {
        let in_arity = self.poly_arities[&inst.callee];
        let split = self.stack.len() - in_arity;
        let args = self.stack.split_off(split);
        let ret = if inst.out_arity == 1 || inst.bundle.is_some() {
            let ret_ty = match inst.bundle {
                Some(id) => IrType::Struct(id),
                None => ir_type_of(
                    *inst
                        .output_types
                        .first()
                        .expect("out_arity == 1 guarantees a single output type"),
                ),
            };
            Some(self.fresh_value(ret_ty))
        } else {
            None
        };
        self.push_instr(Instr::Call(ret, inst.symbol.clone(), args));
        if let Some(v) = ret {
            self.stack.push(v);
        }
        if let Some(id) = inst.bundle {
            self.unpack_bundle(id);
        }
    }

    /// Lower a generated struct word inline, first field deepest.
    fn lower_struct_word(&mut self, sw: StructWord) {
        match sw {
            StructWord::Construct(id) => {
                let n = self.structs.layouts[id.index()].fields.len();
                let split = self.stack.len() - n;
                let args = self.stack.split_off(split);
                let dst = self.alloc_struct(id);
                for (fi, arg) in args.into_iter().enumerate() {
                    let field = self.structs.layouts[id.index()].fields[fi];
                    let fptr = self.field_ptr(dst, field.offset);
                    self.store_field(fptr, arg, field);
                }
                self.stack.push(dst);
            }
            StructWord::Get(id, fi) => {
                let s = self.stack.pop().expect("getter: struct operand");
                let fields = self.structs.layouts[id.index()].fields.clone();
                self.load_field_onto_stack(s, fields[fi]);
                // R9: on a linear receiver, `S>fi` still consumes the whole
                // aggregate, so every non-extracted linear field is dropped
                // here (a no-op drop-the-rest when every other field is
                // Copy, unchanged from before this slice).
                for (j, field) in fields.iter().enumerate() {
                    if j != fi && field_is_linear(field.ty, self.structs, self.enums, self.arrays) {
                        let v = self.field_value(s, *field);
                        self.emit_drop(v);
                    }
                }
            }
            StructWord::Set(id, fi) => {
                let newval = self.stack.pop().expect("setter: new field value");
                let s = self.stack.pop().expect("setter: struct operand");
                let dst = self.alloc_struct(id);
                let size = self.structs.layouts[id.index()].size;
                if size > 0 {
                    self.push_instr(Instr::Blit(s, dst, size));
                }
                let field = self.structs.layouts[id.index()].fields[fi];
                // R11: the old shell's other fields transfer via the blit
                // above (consumed, never dropped); only the field being
                // overwritten is read back out and dropped, before the store,
                // so the order is deterministic.
                if field_is_linear(field.ty, self.structs, self.enums, self.arrays) {
                    let old = self.field_value(dst, field);
                    self.emit_drop(old);
                }
                let fptr = self.field_ptr(dst, field.offset);
                self.store_field(fptr, newval, field);
                self.stack.push(dst);
            }
            StructWord::Destructure(id) => {
                let s = self.stack.pop().expect("destructure: struct operand");
                let n = self.structs.layouts[id.index()].fields.len();
                for fi in 0..n {
                    let field = self.structs.layouts[id.index()].fields[fi];
                    self.load_field_onto_stack(s, field);
                }
            }
            StructWord::Peek(id, fi) => {
                // R10: non-consuming, so the aggregate stays on the stack;
                // only the field's value is pushed on top of it. The checker
                // already rejected a linear field, so there is no drop glue
                // to consider here (unlike `Get`).
                let s = *self.stack.last().expect("peek: struct operand");
                let field = self.structs.layouts[id.index()].fields[fi];
                self.load_field_onto_stack(s, field);
            }
        }
    }

    /// Lower a variant constructor inline (R15): alloc the enum's tagged
    /// aggregate, store the discriminant (the variant's declaration index) as
    /// an `i32` at `tag_offset`, then store each field at `payload_offset +
    /// field.offset` (first field deepest, reusing `store_field`).
    fn lower_enum_word(&mut self, ew: EnumWord) {
        match ew {
            EnumWord::Construct(id, variant_idx) => {
                let (tag_ty, tag_offset, payload_offset, fields) = {
                    let layout = &self.enums.layouts[id.index()];
                    (
                        layout.tag_ty,
                        layout.tag_offset,
                        layout.payload_offset,
                        layout.variants[variant_idx].fields.clone(),
                    )
                };
                let split = self.stack.len() - fields.len();
                let args = self.stack.split_off(split);
                let dst = self.alloc_enum(id);
                let tag = self.fresh_value(tag_ty);
                self.push_instr(Instr::Const(tag, variant_idx as i64));
                let tag_ptr = self.field_ptr(dst, tag_offset);
                self.push_instr(Instr::FieldStore(tag_ptr, tag));
                for (arg, field) in args.into_iter().zip(fields) {
                    let fptr = self.field_ptr(dst, payload_offset + field.offset);
                    self.store_field(fptr, arg, field);
                }
                self.stack.push(dst);
            }
        }
    }

    /// A two-block compare-and-select (`max`/`max-total`'s shared shape,
    /// R12/R13): branch on `cond`, run each closure in its own block to
    /// produce that arm's value, and join with one `Phi`. Simpler than
    /// `lower_if`/`seal_arm` because a select's arms never back-edge (they
    /// lower no user terms, just a handful of value-producing instructions),
    /// so both predecessors always reach the join.
    fn emit_select(
        &mut self,
        cond: Value,
        then_fn: impl FnOnce(&mut Self) -> Value,
        else_fn: impl FnOnce(&mut Self) -> Value,
    ) -> Value {
        let then_id = self.fresh_block();
        let else_id = self.fresh_block();
        let join_id = self.fresh_block();
        self.seal_block(Terminator::Jnz(cond, then_id, else_id));

        self.start_block(then_id);
        self.terminated = false;
        let then_val = then_fn(self);
        let then_pred = self.cur_id;
        self.seal_block(Terminator::Jmp(join_id));

        self.start_block(else_id);
        self.terminated = false;
        let else_val = else_fn(self);
        let else_pred = self.cur_id;
        self.seal_block(Terminator::Jmp(join_id));

        self.start_block(join_id);
        self.terminated = false;
        let ty = self.value_type(then_val);
        let v = self.fresh_value(ty);
        self.push_instr(Instr::Phi(
            v,
            vec![(then_pred, then_val), (else_pred, else_val)],
        ));
        v
    }

    /// R13: the `total_cmp` bit-pattern key for one `max-total` operand.
    /// Reinterprets `operand`'s IEEE bits as an unsigned integer (an 8-byte
    /// scratch slot, stored/reloaded at the operand's own width — `Store`/
    /// `Load` already dispatch on the value's declared `IrType`, R20), then
    /// maps the bits to a monotone key: flip every bit if the sign bit is
    /// set, else flip only the sign bit. Comparing two keys as unsigned
    /// integers then reproduces the total order without ever comparing the
    /// floats themselves.
    fn total_order_key(&mut self, operand: Value, bits: u8) -> Value {
        let uty = IrType::Int {
            bits,
            signed: false,
        };
        let slot = self.fresh_value(IrType::Ptr);
        self.push_alloc(Instr::Alloc(slot, 8, 8));
        self.push_instr(Instr::Store(slot, operand));
        let raw = self.fresh_value(uty);
        self.push_instr(Instr::Load(raw, slot));

        let sign_mask: i64 = 1i64 << (bits - 1);
        let mask_v = self.fresh_value(uty);
        self.push_instr(Instr::Const(mask_v, sign_mask));
        let masked = self.fresh_value(uty);
        self.push_instr(Instr::Bin(masked, BinOp::And, raw, mask_v));
        let zero_u = self.fresh_value(uty);
        self.push_instr(Instr::Const(zero_u, 0));
        let is_neg = self.fresh_value(IrType::Bool);
        self.push_instr(Instr::Cmp(is_neg, CmpOp::Ne, masked, zero_u));

        self.emit_select(
            is_neg,
            |b| {
                let all_ones = b.fresh_value(uty);
                b.push_instr(Instr::Const(all_ones, -1));
                let key = b.fresh_value(uty);
                b.push_instr(Instr::Bin(key, BinOp::Xor, raw, all_ones));
                key
            },
            |b| {
                let key = b.fresh_value(uty);
                b.push_instr(Instr::Bin(key, BinOp::Xor, raw, mask_v));
                key
            },
        )
    }

    /// `tail` (R1) is true when this `if` is itself in tail position; it then
    /// hands tail position to the last term of both arms, so a self-call at the
    /// end of either arm back-edges (R7). An arm that back-edges leaves the
    /// builder `terminated` and contributes no predecessor to the join; the
    /// join is elided entirely when both arms back-edge (R8, both-arms-tail).
    fn lower_if(&mut self, then_branch: &[Term], else_branch: &[Term], tail: bool) {
        let test = self.stack.pop().expect("if: test value");
        let then_id = self.fresh_block();
        let else_id = self.fresh_block();
        let join_id = self.fresh_block();

        let post_pop = self.stack.clone();
        // R2: each arm is a block, so a name it binds is out of scope at its
        // terminator; the checker has already rejected any use past there.
        let locals_depth = self.locals.len();
        self.seal_block(Terminator::Jnz(test, then_id, else_id));

        self.start_block(then_id);
        self.terminated = false;
        self.stack = post_pop.clone();
        self.lower_terms(then_branch, tail);
        let then_arm = self.seal_arm(join_id);
        self.locals.truncate(locals_depth);

        self.start_block(else_id);
        self.terminated = false;
        self.stack = post_pop;
        self.lower_terms(else_branch, tail);
        let else_arm = self.seal_arm(join_id);
        self.locals.truncate(locals_depth);

        match (then_arm, else_arm) {
            (None, None) => {
                // Both arms back-edged to the loop header; the join is
                // unreachable and the enclosing body is terminated.
                self.terminated = true;
            }
            (Some((_, s)), None) | (None, Some((_, s))) => {
                // A single fall-through predecessor: values flow directly, no
                // phi needed.
                self.start_block(join_id);
                self.terminated = false;
                self.stack = s;
            }
            (Some((then_pred, then_stack)), Some((else_pred, else_stack))) => {
                self.start_block(join_id);
                self.terminated = false;
                let mut join_stack = Vec::with_capacity(then_stack.len());
                for (t, e) in then_stack.into_iter().zip(else_stack) {
                    if t == e {
                        join_stack.push(t);
                    } else {
                        let ty = self.value_type(t);
                        let v = self.fresh_value(ty);
                        self.push_instr(Instr::Phi(v, vec![(then_pred, t), (else_pred, e)]));
                        // A merged reference is still `Ptr`, which says
                        // nothing about its referent; carry the shape across
                        // the join so a projection past it still resolves.
                        if let Some(&referent) = self.ref_inner.get(&t) {
                            self.ref_inner.insert(v, referent);
                        }
                        join_stack.push(v);
                    }
                }
                self.stack = join_stack;
            }
        }
    }

    /// Seal a just-lowered `if` arm: if it back-edged (terminated) it jumps
    /// nowhere here and yields no join predecessor; otherwise it jumps to the
    /// join, yielding `(pred, stack)`.
    fn seal_arm(&mut self, join_id: BlockId) -> Option<(BlockId, Vec<Value>)> {
        if self.terminated {
            None
        } else {
            let s = self.stack.clone();
            let pred = self.cur_id;
            self.seal_block(Terminator::Jmp(join_id));
            Some((pred, s))
        }
    }

    /// Lower a clause-style word (R16): load the scrutinee's discriminant into
    /// a temp, dispatch N-way (a `Cmp(Eq)`-tag compare-chain to each variant's
    /// clause block, the last variant the terminal fall-through since coverage
    /// is exhaustive), and merge every clause's outputs at a single join block
    /// with one `Phi` per declared output over all N clause predecessors.
    ///
    /// This is deliberately *not* the 2-predecessor `lower_if` shape: the join
    /// has N predecessors and M outputs.
    fn lower_clauses(&mut self, clauses: &[Clause], params: &[Value], scrutinee_ty: Type) {
        // A clause word is self-tail-recursive iff a header was opened (R6);
        // its clause bodies then carry tail position (D7).
        let tail = self.header.is_some();
        let scrutinee = *params.last().expect("clause word has a scrutinee input");
        let stack_below: Vec<Value> = params[..params.len() - 1].to_vec();
        // Threaded from the already-checked frontend `Type` rather than
        // re-derived from the lowered scrutinee's `IrType` — because a
        // `&!Enum` scrutinee lowers to the opaque `IrType::Ptr`, not
        // `IrType::Enum(id)`, so reading `self.value_type(scrutinee)` here
        // would make the enum arm below a reachable panic in reference mode.
        let (scrut_id, ref_mutable) = match scrutinee_ty {
            Type::Enum(id, _) => (id, None),
            Type::Ref(rid, mutable, _) => match self.refs.referent[rid.index()] {
                IrType::Enum(id) => (id, Some(mutable)),
                _ => unreachable!("checked: reference-mode clause scrutinee's referent is an enum"),
            },
            _ => unreachable!("checked: a clause word's top input is an enum"),
        };
        let payload_offset = self.enums.layouts[scrut_id.index()].payload_offset;
        let n = self.enums.layouts[scrut_id.index()].variants.len();

        // Map each variant index to the clause handling it (checker-guaranteed
        // exact coverage), so dispatch on tag == variant_index lands correctly
        // regardless of clause source order.
        let clause_ids = self.dispatch_on_tag(scrutinee, scrut_id);
        let join_id = self.fresh_block();
        let mut clause_for_variant: Vec<Option<&Clause>> = vec![None; n];
        for clause in clauses {
            let EnumWord::Construct(_, vi) = self.enums.words[&clause.variant];
            clause_for_variant[vi] = Some(clause);
        }

        let mut clause_ends: Vec<(BlockId, Vec<Value>)> = Vec::with_capacity(n);
        for vi in 0..n {
            let clause = clause_for_variant[vi].expect("checked: exhaustive coverage");
            self.start_block(clause_ids[vi]);
            self.locals.clear();
            self.stack = stack_below.clone();
            // Push the variant's payload first-deepest, loading each field from
            // `payload_offset + field.offset`. In reference mode every
            // field is pushed as a reference to its own storage inside the
            // scrutinee (its address, never its value), registered in
            // `ref_inner` so a later access/projection through it resolves the
            // right shape — the same `IrType::Ptr` any other reference lowers
            // to.
            let fields = self.enums.layouts[scrut_id.index()].variants[vi]
                .fields
                .clone();
            for field in &fields {
                let adjusted = FieldLayout {
                    offset: payload_offset + field.offset,
                    ..*field
                };
                match ref_mutable {
                    Some(_) => {
                        let fptr = self.field_ptr(scrutinee, adjusted.offset);
                        self.push_reference(fptr, adjusted.ty);
                    }
                    None => self.load_field_onto_stack(scrutinee, adjusted),
                }
            }
            // Bind clause-body `| names |` locals (top N, leftmost deepest).
            let take = clause.locals.len();
            let bound = self.stack.split_off(self.stack.len() - take);
            for (name, value) in clause.locals.iter().zip(bound) {
                self.locals.push((name.clone(), value));
            }
            // R7/R9: a clause whose body ends in a tail self-call back-edges to
            // the shared loop header and contributes no join predecessor;
            // `tail` is true iff this word is self-tail-recursive. The header
            // phi preds (entry + tail clause ends) and the dispatch-join phi
            // preds (non-tail clause ends) therefore stay disjoint.
            self.terminated = false;
            self.lower_terms(&clause.body, tail);
            if !self.terminated {
                let result = self.stack.clone();
                let pred = self.cur_id;
                self.seal_block(Terminator::Jmp(join_id));
                clause_ends.push((pred, result));
            }
        }

        // Every clause back-edged: the join is unreachable and the word is
        // terminated (no fall-through Ret).
        if clause_ends.is_empty() {
            self.terminated = true;
            return;
        }

        // Single join block: one phi per declared output, merging the
        // fall-through clause predecessors.
        self.start_block(join_id);
        self.terminated = false;
        let m = clause_ends[0].1.len();
        let mut join_stack = Vec::with_capacity(m);
        for out_i in 0..m {
            let arms: Vec<(BlockId, Value)> = clause_ends
                .iter()
                .map(|(pred, st)| (*pred, st[out_i]))
                .collect();
            let ty = self.value_type(arms[0].1);
            let v = self.fresh_value(ty);
            self.push_instr(Instr::Phi(v, arms));
            join_stack.push(v);
        }
        self.stack = join_stack;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Line;
    use crate::check::check;
    use crate::lexer::lex;
    use crate::parser::{parse, parse_line};

    fn lower_src(src: &str) -> IrModule {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        lower(&module).unwrap()
    }

    /// A scalar-only resource with a `drop` overload whose body has one
    /// observable effect (a `Print` no synthesized glue ever emits), so "the
    /// override is the destructor" is assertable on instructions.
    const FILE_RESOURCE: &str = "type: File fd i64 ; : drop ( File -- ) | f | f File>fd . ;";

    /// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
    /// primitive in Slice 8c: an ordinary one-field struct with a `drop`
    /// overload, so it is linear for the same reason any resource is (R3 of
    /// slice 8b), not by any compiler-known bit. Always the first struct in a
    /// source string that uses it, so every other struct's `StructId` shifts
    /// up by one relative to a spy-free program.
    const SPY_DEF: &str =
        "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy>tag . ;\n";

    /// Every symbol an `IrFunc` calls, in emission order: what "the override
    /// ran instead of the glue" is asserted on, rather than a substring of the
    /// emitted text.
    fn call_symbols(func: &IrFunc) -> Vec<&str> {
        instrs(func)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(_, sym, _) => Some(sym.as_str()),
                _ => None,
            })
            .collect()
    }

    fn func<'a>(module: &'a IrModule, name: &str) -> &'a IrFunc {
        module
            .funcs
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "no emitted func `{name}`: {:?}",
                    module.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
                )
            })
    }

    #[test]
    fn two_drop_overloads_for_different_structs_do_not_collide() {
        // Criterion 16: neither override lands in the generic per-word
        // lowering pass (which would emit two QBE functions literally named
        // `drop`, the second colliding with the first), and each instead fills
        // its own struct's destructor symbol with its own body.
        let module = lower_src(
            "type: A x i64 ; type: B y i64 ; \
             : drop ( A -- ) | a | a A>x . ; : drop ( B -- ) | b | b B>y drop ; \
             : main ( -- ) 1 A drop 2 B drop ;",
        );
        assert!(
            module.funcs.iter().all(|f| f.name != "drop"),
            "an emitted IrFunc was literally named `drop`: {:?}",
            module.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let a = func(&module, &struct_drop_symbol(StructId::from_index(0), None));
        let b = func(&module, &struct_drop_symbol(StructId::from_index(1), None));
        // `A`'s body prints its field, `B`'s discards it: two distinct bodies
        // under two distinct symbols, not one shared or one clobbered.
        assert_eq!(count(a, |i| matches!(i, Instr::Print(_))), 1);
        assert_eq!(count(b, |i| matches!(i, Instr::Print(_))), 0);
    }

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
    fn lower_forces_drop_overload_linearity_even_when_check_never_ran() {
        // R1/R2 code-review fix: `lower` used to trust
        // `StructDecl::has_drop_overload`, a bit only `check::check` sets. A
        // module that reaches `lower` without having gone through `check`
        // (this test skips it, unlike `lower_src`) must still layout `File`
        // as linear and substitute the override, not silently emit nothing.
        let src = format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;");
        let tokens = lex(&src).unwrap();
        let module = parse(&tokens).unwrap();
        let ir_module = lower(&module).unwrap();
        let file = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(call_symbols(func(&ir_module, "main")), vec![file.as_str()]);
        let dtor = func(&ir_module, &file);
        assert_eq!(count(dtor, |i| matches!(i, Instr::Print(_))), 1);
    }

    #[test]
    fn drop_of_an_overridden_struct_calls_its_destructor_symbol() {
        // R2: the whole of dispatch. `lower_call`'s `"drop"` arm is unchanged
        // and still symbol-based; forcing `is_linear` is what makes
        // `emit_drop`'s guard pass, and the substituted body is what the
        // symbol now resolves to.
        let module = lower_src(&format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;"));
        let file = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(call_symbols(func(&module, "main")), vec![file.as_str()]);
        // The destructor is the user's body (one `.` of the field), not the
        // generic glue (which for an all-`Copy` struct emits nothing at all).
        let dtor = func(&module, &file);
        assert_eq!(count(dtor, |i| matches!(i, Instr::Print(_))), 1);
    }

    #[test]
    fn synthesize_destructor_of_resource_with_a_linear_field_uses_user_body_not_field_glue() {
        // Criterion 15/R5: the override runs *instead of* the field glue, not
        // before or alongside it. `Res`'s only field is linear, so the glue
        // would call `Inner`'s destructor symbol directly; the body hands the
        // field to `dispose` instead, so that call is the only one emitted.
        let module = lower_src(&format!(
            "{SPY_DEF}type: Inner s Spy ; type: Res i Inner ; \
             : dispose ( Inner -- ) drop ; \
             : drop ( Res -- ) | r | r Res> dispose ; \
             : main ( -- ) 1 Spy Inner Res drop ;"
        ));
        let inner = struct_drop_symbol(StructId::from_index(1), None);
        let res = struct_drop_symbol(StructId::from_index(2), None);
        assert_eq!(call_symbols(func(&module, &res)), vec!["dispose"]);
        // The glue that would have run is still emitted for `Inner` itself,
        // which has no override: `dispose`'s own `drop` calls it.
        assert_eq!(call_symbols(func(&module, "dispose")), vec![inner.as_str()]);
    }

    #[test]
    fn resource_field_disposed_via_its_own_drop_symbol() {
        // Criterion 13/R7 (ordinary composition): an enclosing struct's
        // per-field disposal calls each linear field's destructor rather than
        // inlining its fields, so a resource field is disposed through the
        // user's body with no new mechanism -- `Holder`'s glue prints nothing
        // itself, it calls `File`'s destructor, which prints.
        let module = lower_src(&format!(
            "{FILE_RESOURCE} type: Holder h File n i64 ; \
             : main ( -- ) 1 File 2 Holder drop ;"
        ));
        let file = struct_drop_symbol(StructId::from_index(0), None);
        let holder = func(&module, &struct_drop_symbol(StructId::from_index(1), None));
        assert_eq!(call_symbols(holder), vec![file.as_str()]);
        assert_eq!(count(holder, |i| matches!(i, Instr::Print(_))), 0);
    }

    #[test]
    fn synthesize_destructor_excludes_override_structs_from_a_fused_disposal_path() {
        // Criterion 14/R7 (the disposal-cycle case): `Chain`'s cycle runs back
        // to itself *through* `Res`. The fused loop inlines every intermediate
        // type's field projection instead of calling its destructor, so
        // fusing this cycle would bypass `Res`'s override and leak its
        // resource silently. With `Res` overridden the search stops there, so
        // `Chain` falls back to per-field disposal and reaches the override
        // through its own symbol.
        let src = "type: Res fd i64 next ^Chain ; type: Chain r Res ; : main ( -- ) ;";
        let plain = Probe::new(src);
        assert!(
            plain.path(plain.struct_ty("Chain")).is_some(),
            "without an override, `Chain` fuses its cycle into one loop"
        );

        let p = Probe::with_overrides(src, &["Res"]);
        assert_eq!(p.path(p.struct_ty("Chain")), None);
        // The search's own root is unaffected: whether `Res` is on a cycle is
        // moot, since its destructor is its override either way (R2).
        assert!(p.path(p.struct_ty("Res")).is_some());

        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let chain = synthesize_struct_destructor(p.struct_id("Chain"), &env, &resolve, p.regs());
        assert_eq!(
            call_symbols(&chain),
            vec![struct_drop_symbol(p.struct_id("Res"), None).as_str()]
        );
    }

    fn structs_of(src: &str) -> Structs {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        Structs::from_structs(&module.structs)
    }

    fn enums_of(src: &str) -> Enums {
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
        .1
    }

    /// A probe program's four registries, owned so `recursive_disposal_path`
    /// can be called on any of its types by name.
    struct Probe {
        structs: Structs,
        enums: Enums,
        arrays: Arrays,
        cells: Cells,
        refs: Refs,
    }

    impl Probe {
        fn new(src: &str) -> Probe {
            Probe::with_overrides(src, &[])
        }

        /// A `Probe` whose named structs each carry a `drop` overload, set the
        /// way `check` sets it but without a `: drop` word in the source.
        /// Deliberately not written as a program: an override body on a
        /// disposal cycle must dispose something that leads back to its own
        /// receiver, which R6's self-recursion rejection refuses, so R7's
        /// cycle boundary is reachable from the registries but not from a
        /// module that type-checks.
        fn with_overrides(src: &str, overridden: &[&str]) -> Probe {
            let tokens = lex(src).unwrap();
            let mut module = parse(&tokens).unwrap();
            check(&mut module).unwrap();
            for name in overridden {
                let decl = module
                    .structs
                    .iter_mut()
                    .find(|s| s.name == *name)
                    .expect("declared struct");
                decl.has_drop_overload = true;
            }
            let (structs, enums, arrays, cells, refs) = build_registries(
                &module.structs,
                &module.enums,
                &module.arrays,
                &module.owned_cells,
                &module.refs,
            );
            Probe {
                structs,
                enums,
                arrays,
                cells,
                refs,
            }
        }

        fn regs(&self) -> Registries<'_> {
            Registries {
                structs: &self.structs,
                enums: &self.enums,
                arrays: &self.arrays,
                cells: &self.cells,
                refs: &self.refs,
            }
        }

        fn struct_id(&self, name: &str) -> StructId {
            match self.struct_ty(name) {
                IrType::Struct(id) => id,
                other => unreachable!("{other:?}"),
            }
        }

        fn struct_ty(&self, name: &str) -> IrType {
            let idx = self
                .structs
                .layouts
                .iter()
                .position(|l| l.name == name)
                .expect("declared struct");
            IrType::Struct(StructId::from_index(idx))
        }

        fn enum_ty(&self, name: &str) -> IrType {
            let idx = self
                .enums
                .layouts
                .iter()
                .position(|l| l.name == name)
                .expect("declared enum");
            IrType::Enum(EnumId::from_index(idx))
        }

        /// The interned cell holding `payload`, so an expected `Unwrap` names
        /// its cell by what it points at rather than by a guessed index.
        fn cell(&self, payload: IrType) -> OwnedCellId {
            let idx = self
                .cells
                .payload
                .iter()
                .position(|&p| p == payload)
                .expect("interned cell");
            OwnedCellId::from_index(idx)
        }

        fn path(&self, ty: IrType) -> Option<Vec<PathStep>> {
            recursive_disposal_path(ty, self.regs())
        }
    }

    fn layout<'a>(s: &'a Structs, name: &str) -> &'a StructLayout {
        s.layouts.iter().find(|l| l.name == name).expect("layout")
    }

    fn enum_layout<'a>(e: &'a Enums, name: &str) -> &'a EnumLayout {
        e.layouts.iter().find(|l| l.name == name).expect("layout")
    }

    fn instrs(func: &IrFunc) -> Vec<&Instr> {
        func.blocks.iter().flat_map(|b| b.instrs.iter()).collect()
    }

    fn line_terms(src: &str) -> Vec<Term> {
        let tokens = lex(src).unwrap();
        match parse_line(&tokens).unwrap() {
            Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        }
    }

    fn count(func: &IrFunc, pred: impl Fn(&Instr) -> bool) -> usize {
        func.blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter(|i| pred(i))
            .count()
    }

    fn empty_builder<'a>(
        env: &'a HashMap<String, Arity>,
        resolve: Resolver<'a>,
        regs: Registries<'a>,
    ) -> FuncBuilder<'a> {
        FuncBuilder::new(env, resolve, regs, String::new())
    }

    #[test]
    fn quotation_literal_emits_no_instr_and_records_body() {
        // R12u: `lower_term`'s `TermKind::Quotation` arm mints a phantom
        // `Value` that defines no `Instr`, records `Value -> QuotId`, and
        // pushes it; the body is interned, not emitted.
        let env: HashMap<String, Arity> = HashMap::new();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let mut b = empty_builder(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
        );
        let term = &line_terms("[ + ]")[0];
        assert!(matches!(term.kind, TermKind::Quotation(_)));
        b.lower_term(term, false);
        assert!(
            b.cur_instrs.is_empty(),
            "a quotation literal emits no instruction: {:?}",
            b.cur_instrs
        );
        assert_eq!(b.stack.len(), 1);
        let v = b.stack[0];
        assert!(
            b.quot_bodies.contains_key(&v),
            "the phantom value is recorded in quot_bodies"
        );
        assert_eq!(b.quot_defs.len(), 1, "the body is interned once");
    }

    #[test]
    fn call_of_literal_emits_no_call_instr() {
        // Criterion 6b (R13): `[ + ] call` fuses in place, so lowered `main`
        // contains no `Instr::Call`; the phantom quotation never becomes a
        // runtime code value.
        let module = lower_src(": main ( -- ) 1 2 [ + ] call . ;");
        let main = func(&module, "main");
        assert_eq!(count(main, |i| matches!(i, Instr::Call(..))), 0);
        assert_eq!(
            count(main, |i| matches!(i, Instr::Bin(_, BinOp::Add, ..))),
            1
        );
    }

    #[test]
    fn times_lowers_to_a_loop_header_not_a_per_iteration_call() {
        // Criterion 6 (R14/R17): `times` builds a header `Block` carrying the
        // index `Phi`, sealed with a `Terminator::Jnz`, reached by a back-edge
        // `Terminator::Jmp`, with no per-iteration `Instr::Call`. The index
        // `Phi` + header `Jnz` are pinned because "header + back-edge `Jmp` + no
        // `Call`" alone also describes a one-trip or infinite loop.
        let simple = lower_src(": main ( -- ) 0 1000000 [ + ] times . ;");
        let main = func(&simple, "main");
        let header = loop_header(main);
        let hblock = header_block(main, header);
        assert!(
            !header_phis(hblock).is_empty(),
            "the header carries the index phi"
        );
        assert!(
            matches!(hblock.term, Terminator::Jnz(..)),
            "the header is sealed with a Jnz (index < count), got {:?}",
            hblock.term
        );
        let entry_id = main.blocks[0].id;
        assert!(
            main.blocks
                .iter()
                .any(|b| b.id != entry_id && matches!(b.term, Terminator::Jmp(h) if h == header)),
            "a non-entry body block back-edges to the header"
        );
        assert_eq!(
            count(main, |i| matches!(i, Instr::Call(..))),
            0,
            "no per-iteration Instr::Call"
        );

        // On 5a's source (a `Vec2` constructed each iteration): every `Alloc`
        // hoists into the entry block, none into the body block (R17). This is
        // the deterministic R17 witness, not the coarse `ulimit` run.
        let agg = lower_src(
            "type: Vec2 x i64 y i64 ;\n\
             : main ( -- ) 0 1000000 [ | i | i i Vec2 Vec2>x + ] times . ;",
        );
        let main = func(&agg, "main");
        let header = loop_header(main);
        let entry = &main.blocks[0];
        let body = main
            .blocks
            .iter()
            .find(|b| b.id != entry.id && matches!(b.term, Terminator::Jmp(h) if h == header))
            .expect("a body block back-edging to the header");
        assert!(
            entry.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
            "the per-iteration Vec2 Alloc hoists into the entry block"
        );
        assert!(
            !body.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
            "no Alloc in the loop body block (R17)"
        );
    }

    #[test]
    fn times_saves_and_restores_loop_state() {
        // R15u: after the `times` arm returns, `header`/`entry_block`/
        // `carried_slots`/`back_edges` are all back to their pre-`times` values.
        // `finalize_loop` clears only two of the four, so the arm's explicit
        // save/restore is what lets a later `Alloc` (or a second sequential
        // `times`) not hoist into the dead `times` entry block.
        let env: HashMap<String, Arity> = HashMap::new();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let mut b = empty_builder(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
        );
        // A `times` over an empty row: push the count, then intern a body that
        // consumes just the synthesized index (`[ drop ]`) so the row stays
        // empty and the back-edge arity matches the single index slot.
        let count = b.fresh_value(IrType::I64);
        b.push_instr(Instr::Const(count, 3));
        b.const_vals.insert(count, 3);
        b.stack.push(count);
        let quot_term = &line_terms("[ drop ]")[0];
        b.lower_term(quot_term, false);
        assert_eq!(b.stack.len(), 2, "count beneath the quotation phantom");

        let saved_header = b.header;
        let saved_entry = b.entry_block;
        b.lower_call("times", Span { line: 1, col: 1 }, false);

        assert_eq!(b.header, saved_header, "header restored");
        assert_eq!(b.entry_block, saved_entry, "entry_block restored");
        assert!(b.carried_slots.is_empty(), "carried_slots restored");
        assert!(b.back_edges.is_empty(), "back_edges restored");
    }

    #[test]
    fn lower_max_emits_a_compare_and_select_no_call() {
        // R12: `max` lowers inline to `Cmp(Gt)` plus a `Phi`-joined select, no
        // `Instr::Call` and no monomorphization.
        let ir = lower_src(": main ( -- ) 3 5 max . ;");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        assert_eq!(
            count(main, |i| matches!(i, Instr::Cmp(_, CmpOp::Gt, ..))),
            1
        );
        assert_eq!(count(main, |i| matches!(i, Instr::Phi(..))), 1);
        assert_eq!(count(main, |i| matches!(i, Instr::Call(..))), 0);
    }

    #[test]
    fn lower_max_total_emits_no_float_compare() {
        // R13: `max-total` orders by the bit-pattern rule, so the emitted
        // `Cmp`s are all over the unsigned integer key, never `Instr::Cmp`
        // with a float operand.
        let ir = lower_src(": main ( -- ) 1.5 2.5 max-total . ;");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        let float_cmps = instrs(main)
            .iter()
            .filter(|i| match i {
                Instr::Cmp(_, _, a, _) => {
                    matches!(main.value_types[a.0 as usize], IrType::Float { .. })
                }
                _ => false,
            })
            .count();
        assert_eq!(float_cmps, 0);
        assert_eq!(count(main, |i| matches!(i, Instr::Call(..))), 0);
    }

    #[test]
    fn lower_two_output_word_returns_one_bundle_holding_both() {
        // Criterion 9 (R10): a two-output word's body ends in one `Ret` of the
        // synthesized bundle, with both outputs stored into it -- not a single
        // value returned and the other silently dropped.
        let ir = lower_src(": pair ( i64 -- i64 i64 ) dup ; : main ( -- ) 5 pair . . ;");
        let pair = ir.funcs.iter().find(|f| f.name == "pair").unwrap();
        let IrType::Struct(bundle) = pair.ret.expect("a two-output word returns its bundle") else {
            panic!("expected a struct return, got {:?}", pair.ret);
        };
        assert!(ir.structs[bundle.index()].bundle);
        assert_eq!(ir.structs[bundle.index()].fields.len(), 2);

        let last = pair.blocks.last().unwrap();
        let Terminator::Ret(Some(returned)) = last.term else {
            panic!("expected a value return, got {:?}", last.term);
        };
        assert_eq!(
            pair.value_types[returned.0 as usize],
            IrType::Struct(bundle)
        );
        assert_eq!(count(pair, |i| matches!(i, Instr::FieldStore(..))), 2);
    }

    #[test]
    fn lower_call_of_two_output_word_unpacks_the_bundle_onto_the_stack() {
        // R11: the caller reads both outputs back out of the returned bundle
        // (two field loads), so its lowering stack matches the stack the
        // checker verified -- the recon-3 desync that used to panic.
        let ir = lower_src(": pair ( i64 -- i64 i64 ) dup ; : main ( -- ) 5 pair . . ;");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        assert_eq!(count(main, |i| matches!(i, Instr::Call(Some(_), ..))), 1);
        assert_eq!(count(main, |i| matches!(i, Instr::FieldLoad(..))), 2);
        assert_eq!(count(main, |i| matches!(i, Instr::Print(_))), 2);
    }

    #[test]
    fn monomorphization_emits_one_mangled_func_per_instantiation() {
        // R9/R14: a polymorphic word is never emitted under its plain name;
        // instead one mangled `IrFunc` is emitted per distinct ground θ, and
        // each call site targets its own instantiation's symbol through the
        // R14 table, not `dupit`.
        let ir = lower_src(
            ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n\
             : main ( -- ) 5 dupit . . true dupit . . ;",
        );
        assert!(
            ir.funcs.iter().all(|f| f.name != "dupit"),
            "the polymorphic word must not lower under its plain name"
        );
        let mono: Vec<&str> = ir
            .funcs
            .iter()
            .map(|f| f.name.as_str())
            .filter(|n| n.starts_with("sooth_mono_dupit"))
            .collect();
        assert_eq!(mono.len(), 2, "one IrFunc per θ (i64 and bool)");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        let calls = call_symbols(main);
        for sym in &mono {
            assert!(calls.contains(sym), "main should call `{sym}` directly");
        }
    }

    #[test]
    fn lower_single_output_word_keeps_its_scalar_return() {
        // R2/R15: nothing about the bundle path reaches a word with one
        // output; it returns its scalar directly, as before the slice.
        let ir = lower_src(": inc ( i64 -- i64 ) 1 + ;");
        let inc = ir.funcs.iter().find(|f| f.name == "inc").unwrap();
        assert_eq!(inc.ret, Some(IrType::I64));
        assert!(ir.structs.is_empty());
    }

    #[test]
    fn lower_bundle_with_a_linear_field_gets_no_destructor() {
        // Criterion 10 (R10/R11, key risk 1): the bundle for `( -- ^i64 i64 )`
        // folds linear (its first field is an owning cell), yet no drop glue is
        // synthesized for it -- the glue would free the cell the caller's
        // unpack has already moved out.
        let ir =
            lower_src(": cell-and-tag ( -- ^i64 i64 ) 7 ^ 3 ; : main ( -- ) cell-and-tag . ^> . ;");
        let (idx, layout) = ir
            .structs
            .iter()
            .enumerate()
            .find(|(_, l)| l.bundle)
            .expect("the two-output word interned a bundle");
        assert!(
            layout.is_linear,
            "an owning-cell field folds the bundle linear"
        );
        let glue = format!("sooth_struct_drop_{idx}");
        assert!(
            !ir.funcs.iter().any(|f| f.name == glue),
            "a bundle must carry no destructor, found `{glue}`"
        );
    }

    #[test]
    fn lower_two_words_with_one_output_shape_share_one_bundle() {
        // R8: bundles are interned by output tuple, deduped structurally like
        // an array shape, so two words of the same shape share one struct and
        // a third shape gets its own.
        let ir = lower_src(
            ": pair ( i64 -- i64 i64 ) dup ;\n\
             : twice ( i64 -- i64 i64 ) dup ;\n\
             : flags ( -- bool bool ) true false ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(ir.structs.iter().filter(|l| l.bundle).count(), 2);
    }

    #[test]
    fn func_builder_new_threads_current_word_name() {
        // R5: FuncBuilder carries the word being lowered, set from `word.name`
        // in `lower_word`; the REPL path calls the same `lower_word` (no
        // REPL-specific plumbing), so this covers both callers.
        let env: HashMap<String, Arity> = HashMap::new();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let b = FuncBuilder::new(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
            "loop-word".to_string(),
        );
        assert_eq!(b.cur_word_name, "loop-word");
    }

    #[test]
    fn lower_borrow_of_cell_local_gives_the_pointer_a_place() {
        // `&^`/`&!^` project by *loading* the cell pointer out of the
        // place holding it, but a cell local's value already *is* that pointer
        // (an SSA temporary with no address), so borrowing one has to give it a
        // slot first. The load then reads that slot back.
        let ir = lower_src(": w ( -- i64 ) 7 ^ | c | &c &^ @ c ^> drop ;");
        let w = &ir.funcs[0];
        let alloc = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Alloc(v, size, _) if *size == WORD_WIDTH => Some(*v),
                _ => None,
            })
            .expect("borrowing a cell local allocs a one-word place");
        assert!(
            instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::Store(dst, _) if *dst == alloc)),
            "the cell pointer is stored into its new place: {:?}",
            instrs(w)
        );
        assert!(
            instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::Load(_, src) if *src == alloc)),
            "the projection loads the pointer back out: {:?}",
            instrs(w)
        );
    }

    #[test]
    fn lower_reference_through_a_branch_join_keeps_its_referent() {
        // A merged reference is still the opaque `Ptr`, which says nothing
        // about what it points at, so the join has to carry the referent shape
        // across or the projection past it has no field offset to use.
        let ir = lower_src(
            "type: V x i64 y i64 ;\n             : w ( bool -- i64 ) | c | 1 2 V | v | c if &v else &v end &V>x @ ;",
        );
        let w = &ir.funcs[0];
        let phi = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Phi(v, _) => Some(*v),
                _ => None,
            })
            .expect("the two arms merge their references in a phi");
        assert!(
            instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::PtrOffset(_, base, _) if *base == phi)),
            "the projection past the join offsets from the merged value: {:?}",
            instrs(w)
        );
    }

    #[test]
    fn lower_square_has_one_mul() {
        let ir = lower_src(": sq ( i64 -- i64 ) | n | n n * ;");
        let sq = &ir.funcs[0];
        let mul_count = instrs(sq)
            .iter()
            .filter(|i| matches!(i, Instr::Bin(_, BinOp::Mul, _, _)))
            .count();
        assert_eq!(mul_count, 1);
        let last = sq.blocks.last().unwrap();
        assert!(matches!(last.term, Terminator::Ret(Some(_))));
    }

    #[test]
    fn lower_dup_reuses_value_id() {
        // `dup +` squares: both operands must be the same SSA value, dup emits nothing.
        let ir = lower_src(": w ( i64 -- i64 ) dup + ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        assert!(is.iter().all(|i| !matches!(i, Instr::Const(..))));
        let bin = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(_, BinOp::Add, a, b) => Some((*a, *b)),
                _ => None,
            })
            .unwrap();
        assert_eq!(bin.0, bin.1);
    }

    #[test]
    fn lower_binding_emits_no_new_instr() {
        // R10: a binding is a compile-time rebinding of SSA values, so binding
        // the operands and mentioning them lowers to the same instructions as
        // leaving them on the stack. No `Instr` variant was added.
        let bound = lower_src(": w ( -- i64 ) 1 2 | a b | a b - ;");
        let plain = lower_src(": w ( -- i64 ) 1 2 - ;");
        assert_eq!(
            format!("{:?}", instrs(&bound.funcs[0])),
            format!("{:?}", instrs(&plain.funcs[0]))
        );
    }

    #[test]
    fn lower_swap_reorders_without_instr() {
        // `swap -` computes b - a instead of a - b, and swap itself emits no instr.
        let swapped = lower_src(": w ( i64 i64 -- i64 ) swap - ;");
        let plain = lower_src(": w ( i64 i64 -- i64 ) - ;");
        let operands = |ir: &IrModule| {
            instrs(&ir.funcs[0])
                .iter()
                .find_map(|i| match i {
                    Instr::Bin(_, BinOp::Sub, a, b) => Some((*a, *b)),
                    _ => None,
                })
                .unwrap()
        };
        let (sa, sb) = operands(&swapped);
        let (pa, pb) = operands(&plain);
        assert_eq!((sa, sb), (pb, pa));
        assert_eq!(instrs(&swapped.funcs[0]).len(), 1);
    }

    #[test]
    fn lower_drop_pops_without_instr() {
        let ir = lower_src(": w ( i64 i64 -- i64 ) drop ;");
        let w = &ir.funcs[0];
        assert!(instrs(w).is_empty());
        let last = w.blocks.last().unwrap();
        assert!(matches!(last.term, Terminator::Ret(Some(_))));
    }

    #[test]
    fn lower_if_emits_phi_at_join() {
        let ir = lower_src(": w ( bool -- i64 ) if 1 else 2 end ;");
        let w = &ir.funcs[0];
        let has_phi = instrs(w).iter().any(|i| matches!(i, Instr::Phi(..)));
        assert!(has_phi);
        assert!(w
            .blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))));
    }

    #[test]
    fn lower_line_marshals_all_inputs_and_outputs() {
        // `+` from a carried depth of 2 loads both slots and stores the single
        // result: D=2 loads, M=1 store.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let (func, m, _) = lower_line(
            0,
            &line_terms("+"),
            2,
            &[Type::I64, Type::I64],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_poly_arities(),
        );
        assert_eq!(m, 1);
        assert_eq!(count(&func, |i| matches!(i, Instr::Load(..))), 2);
        assert_eq!(count(&func, |i| matches!(i, Instr::Store(..))), 1);
    }

    #[test]
    fn lower_line_returns_advanced_top() {
        // `2 3 +` from D=0 nets +1, so new_top = top + 8.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let (func, m, _) = lower_line(
            0,
            &line_terms("2 3 +"),
            0,
            &[],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_poly_arities(),
        );
        assert_eq!(m, 1);
        let last = func.blocks.last().unwrap();
        let ret = match last.term {
            Terminator::Ret(Some(v)) => v,
            ref other => panic!("expected Ret(Some), got {other:?}"),
        };
        // The returned value is `top (%v1) + delta` with delta = 8.
        let is = instrs(&func);
        let (add_lhs, add_rhs) = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(d, BinOp::Add, a, b) if *d == ret => Some((*a, *b)),
                _ => None,
            })
            .expect("a top-advancing add");
        assert_eq!(add_lhs, Value(1), "add should read the `top` param %v1");
        let delta = is
            .iter()
            .find_map(|i| match i {
                Instr::Const(v, n) if *v == add_rhs => Some(*n),
                _ => None,
            })
            .expect("a delta const");
        assert_eq!(delta, 8);
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

    fn arrays_of(src: &str) -> Arrays {
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
        .2
    }

    fn module_of(src: &str) -> Module {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        module
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
    fn ir_type_of_array_and_usize_map() {
        let m = module_of(": w ( [i64 4] usize -- ) drop drop ;");
        let arr = m.resolve_type_name("usize").unwrap();
        assert_eq!(ir_type_of(arr), IrType::Usize);
        // The `[i64 4]` shape is interned as ArrayId 0.
        assert_eq!(
            ir_type_of(Type::Array(ArrayId::from_index(0), "[i64 4]")),
            IrType::Array(ArrayId::from_index(0))
        );
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

    #[test]
    fn lower_fill_allocs_and_unrolls_n_stores() {
        // R18/M6: `fill` allocs one array slot and unrolls N FieldStores of the
        // element (no loop, no blit for a scalar element).
        let ir = lower_src(": w ( -- ) 7 4 fill drop ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldStore(..))), 4);
        assert_eq!(count(w, |i| matches!(i, Instr::Blit(..))), 0);
    }

    #[test]
    fn lower_reference_element_read_is_elem_addr_and_load() {
        // `&>` addresses the element (`ElemAddr`); `@` loads it
        // (`FieldLoad`); neither allocs, since the array is never rebuilt.
        let ir = lower_src(": w ( [i64 4] -- i64 ) | a | &a 0 &> @ ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldLoad(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 0);
    }

    #[test]
    fn lower_reference_element_store_is_elem_addr_and_store_no_rebuild() {
        // `&!>` addresses the element; `!` stores directly, with no alloc and
        // no blit: replacing `set`'s whole-array rebuild is the point.
        let ir = lower_src(": w ( [i64 4] usize i64 -- ) | a i x | &!a i &!> x ! ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldStore(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::Blit(..))), 0);
    }

    #[test]
    fn lower_reference_element_runtime_index_emits_bounds_guard_and_trap_call() {
        // A runtime (non-literal) index guards the access with `index < N`
        // and jumps to a trap block that calls the OOB helper.
        let ir = lower_src(": w ( [i64 4] usize -- i64 ) | a i | &a i &> @ ;");
        let w = &ir.funcs[0];
        assert!(w
            .blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))));
        assert_eq!(
            count(
                w,
                |i| matches!(i, Instr::Call(None, sym, _) if sym == OOB_TRAP_SYMBOL)
            ),
            1
        );
    }

    #[test]
    fn lower_reference_element_constant_index_has_no_runtime_guard() {
        // A checked literal index is bounds-verified at compile time, so it
        // skips the runtime guard entirely — no branch, no trap call.
        let ir = lower_src(": w ( [i64 4] -- i64 ) | a | &a 0 &> @ ;");
        let w = &ir.funcs[0];
        assert!(!w
            .blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))));
        assert_eq!(
            count(
                w,
                |i| matches!(i, Instr::Call(None, sym, _) if sym == OOB_TRAP_SYMBOL)
            ),
            0
        );
    }

    #[test]
    fn lower_len_is_a_constant_with_no_memory_access() {
        // R18: `len` folds to a constant `usize` (the count) with no load and
        // no element addressing.
        let ir = lower_src(": w ( [i64 4] -- usize ) len swap drop ;");
        let w = &ir.funcs[0];
        assert!(instrs(w).iter().any(|i| matches!(i, Instr::Const(_, 4))));
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldLoad(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::Load(..))), 0);
    }

    #[test]
    fn str_literal_lowers_to_a_static_data_reference() {
        // R6: a `str` literal is exactly one `Instr::StrLit`, the backend's
        // hook to emit the static descriptor and take its address.
        let ir = lower_src(": w ( -- str ) \"hi\" ;");
        let w = &ir.funcs[0];
        assert_eq!(
            count(w, |i| matches!(i, Instr::StrLit(_, s) if s == "hi")),
            1
        );
    }

    #[test]
    fn len_of_str_lowers_to_str_len_with_no_call() {
        // R8: `len` on a `str` lowers to the dedicated `StrLen`
        // instruction, not a call and not a hand-written byte offset.
        let ir = lower_src(": w ( -- usize ) \"hi\" len ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::StrLen(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::Call(..))), 0);
    }

    #[test]
    fn cstr_conversion_lowers_to_str_ptr() {
        // R7: `cstr` lowers to the dedicated `StrPtr` instruction.
        let ir = lower_src(": w ( -- cstr ) \"hi\" cstr ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::StrPtr(..))), 1);
    }

    #[test]
    fn len_and_cstr_of_str_emit_no_byte_offset_instruction() {
        // Neither `len` nor `cstr` reads the descriptor via a hand-written
        // `field_ptr` offset (`PtrOffset` + `FieldLoad`) any more; both state
        // their intent through a dedicated instruction instead, keeping the
        // descriptor's layout a backend-only concern.
        let ir = lower_src(": w ( -- ) \"hi\" len drop \"hi\" cstr drop ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::PtrOffset(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldLoad(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::StrLen(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::StrPtr(..))), 1);
    }

    #[test]
    fn extern_call_lowers_to_a_call_with_the_declared_symbol() {
        // R1: an `extern:` declaration's C symbol, not its Sooth word name,
        // is what the emitted call names; binding a name that differs from
        // its symbol (`clen` bound to `strlen`) would not catch a lowering
        // bug that emitted `call $<word-name>` instead.
        let ir = lower_src(
            "extern: clen ( cstr -- usize ) \"strlen\" ;\n\
             : w ( -- usize ) \"hi\" cstr clen ;",
        );
        let w = &ir.funcs[0];
        let calls: Vec<&str> = w
            .blocks
            .iter()
            .flat_map(|b| &b.instrs)
            .filter_map(|i| match i {
                Instr::Call(_, sym, _) => Some(sym.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(calls, vec!["strlen"]);
    }

    #[test]
    fn lower_line_struct_slot_blits_in_and_out() {
        // A carried struct slot is copied out of the buffer on entry and back
        // on exit by aggregate blits, and the returned top advances by the
        // struct's aligned carried size. An empty line carries the one
        // Vec2 straight through: one prologue blit, one epilogue blit.
        let s = structs_of("type: Vec2 x i64 y i64 ;");
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let vec2 = Type::Struct(StructId::from_index(0), "Vec2");
        let (func, m, out_bytes) = lower_line(
            0,
            &line_terms(""),
            1,
            &[vec2],
            &env,
            &resolve,
            Registries {
                structs: &s,
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_poly_arities(),
        );
        assert_eq!(m, 1);
        assert_eq!(out_bytes, 16);
        assert_eq!(count(&func, |i| matches!(i, Instr::Blit(..))), 2);
        // No scalar 8-byte-cell Load/Store touches a struct slot.
        assert_eq!(count(&func, |i| matches!(i, Instr::Load(..))), 0);
        assert_eq!(count(&func, |i| matches!(i, Instr::Store(..))), 0);
    }

    #[test]
    fn lower_line_carried_str_slot_keeps_its_own_ir_type() {
        // The carried-slot prologue's match used to fall through a `_` arm
        // for `str` (and other non-aggregate types), loading it as a bare
        // `IrType::I64` and losing the type a later `len`/`.`/`cstr` in the
        // line dispatches on. An empty line carries one `str` straight
        // through: the loaded value must keep `IrType::Str`.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let (func, m, out_bytes) = lower_line(
            0,
            &line_terms(""),
            1,
            &[Type::Str],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_poly_arities(),
        );
        assert_eq!(m, 1);
        assert_eq!(out_bytes, 8);
        let loaded = instrs(&func)
            .iter()
            .find_map(|i| match i {
                Instr::Load(dst, _) => Some(*dst),
                _ => None,
            })
            .expect("a load of the carried str slot");
        assert_eq!(func.value_types[loaded.0 as usize], IrType::Str);
    }

    #[test]
    fn lower_line_scalar_only_uses_eight_byte_cells_and_no_blit() {
        // R16/NF3: a scalar-only line marshals exactly as before — 8-byte-cell
        // stores, `PtrOffset`s at multiples of 8, and never an aggregate
        // `Blit`. `+` from a carried depth of 2 reads cells 0/8 and writes the
        // single result at 0.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let (func, m, out_bytes) = lower_line(
            0,
            &line_terms("+"),
            2,
            &[Type::I64, Type::I64],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_poly_arities(),
        );
        assert_eq!(m, 1);
        assert_eq!(out_bytes, 8);
        assert_eq!(count(&func, |i| matches!(i, Instr::Blit(..))), 0);
        let offsets: Vec<i64> = instrs(&func)
            .iter()
            .filter_map(|i| match i {
                Instr::PtrOffset(_, _, off) => Some(*off),
                _ => None,
            })
            .collect();
        assert_eq!(
            offsets,
            vec![0, 8, 0],
            "two input cells at 0/8, one output cell at 0"
        );
    }

    #[test]
    fn lower_line_carried_narrow_slot_relabels_after_load() {
        // Q2/R16: a `u8` carried slot loads as `l`-width `i64` from the buffer
        // (canonicalization keeps its low bits authoritative), then must be
        // relabeled to `IrType::Int { bits: 8, signed: false }` via `Conv` so a
        // later homogeneous op in the same line sees the real operand type.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let u8_ty = Type::from_name("u8").unwrap();
        let (func, _m, _) = lower_line(
            0,
            &line_terms("1 >u8 +"),
            1,
            &[u8_ty],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_poly_arities(),
        );
        let conv_dst = instrs(&func)
            .iter()
            .find_map(|i| match i {
                Instr::Conv(dst, _) => Some(*dst),
                _ => None,
            })
            .expect("a Conv relabeling the loaded slot");
        assert_eq!(
            func.value_types[conv_dst.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
    }

    #[test]
    fn lower_call_uses_resolved_generation_symbol() {
        let mut env = HashMap::new();
        env.insert("sq".to_string(), (1usize, 1usize, None));
        let resolve = |name: &str| format!("{name}__gen2");
        let (func, _m, _) = lower_line(
            0,
            &line_terms("5 sq"),
            0,
            &[],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_poly_arities(),
        );
        let calls: Vec<&str> = instrs(&func)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(_, sym, _) => Some(sym.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(calls, vec!["sq__gen2"]);
    }

    #[test]
    fn lower_bool_literal_is_bool_typed() {
        let ir = lower_src(": w ( -- bool ) true ;");
        let w = &ir.funcs[0];
        let v = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Const(v, 1) => Some(*v),
                _ => None,
            })
            .expect("a const 1 for `true`");
        assert_eq!(w.value_types[v.0 as usize], IrType::Bool);
    }

    #[test]
    fn lower_comparison_result_is_bool() {
        let ir = lower_src(": w ( i64 i64 -- bool ) > ;");
        let w = &ir.funcs[0];
        let v = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Cmp(v, CmpOp::Gt, _, _) => Some(*v),
                _ => None,
            })
            .expect("a Gt comparison");
        assert_eq!(w.value_types[v.0 as usize], IrType::Bool);
    }

    #[test]
    fn lower_print_emits_print_instr() {
        let ir = lower_src(": w ( i64 -- ) . ;");
        let w = &ir.funcs[0];
        assert!(instrs(w).iter().any(|i| matches!(i, Instr::Print(_))));
        let last = w.blocks.last().unwrap();
        assert!(matches!(last.term, Terminator::Ret(None)));
    }

    #[test]
    fn lower_print_on_bool_and_float_emits_same_print_instr() {
        // `.` lowers to one `Print` regardless of operand type: the IR stays
        // neutral and the backend dispatches on the value's own `IrType`.
        let bool_ir = lower_src(": w ( bool -- ) . ;");
        assert!(instrs(&bool_ir.funcs[0])
            .iter()
            .any(|i| matches!(i, Instr::Print(_))));
        let float_ir = lower_src(": w ( f64 -- ) . ;");
        assert!(instrs(&float_ir.funcs[0])
            .iter()
            .any(|i| matches!(i, Instr::Print(_))));
    }

    #[test]
    fn lower_line_carried_float_slot_loads_as_float() {
        // A carried `f64` slot loads at its float `IrType` (R20), so the value
        // re-enters as a true float rather than a stale `i64`; no `Conv`
        // relabel is needed (that path is integer-only).
        let terms = line_terms("dup");
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let f64_ty = Type::from_name("f64").unwrap();
        let (func, _m, _) = lower_line(
            0,
            &terms,
            1,
            &[f64_ty],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_poly_arities(),
        );
        let loaded = func
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .find_map(|i| match i {
                Instr::Load(v, _) => Some(*v),
                _ => None,
            });
        let v = loaded.expect("a load in the prologue");
        assert_eq!(func.value_types[v.0 as usize], IrType::Float { bits: 64 });
        assert!(!func
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .any(|i| matches!(i, Instr::Conv(..))));
    }

    #[test]
    fn ir_type_of_each_width_expected() {
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
            let ty = Type::from_name(name).unwrap();
            assert_eq!(
                ir_type_of(ty),
                IrType::Int {
                    bits: *bits,
                    signed: *signed
                },
                "mapping {name}"
            );
        }
        assert_eq!(ir_type_of(Type::Bool), IrType::Bool);
    }

    #[test]
    fn ir_type_of_float_widths_expected() {
        assert_eq!(
            ir_type_of(Type::from_name("f32").unwrap()),
            IrType::Float { bits: 32 }
        );
        assert_eq!(
            ir_type_of(Type::from_name("f64").unwrap()),
            IrType::Float { bits: 64 }
        );
    }

    #[test]
    fn lower_float_literal_is_constf_f64_typed() {
        let ir = lower_src(": w ( -- f64 ) 2.5 ;");
        let w = &ir.funcs[0];
        let v = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::ConstF(v, x) if *x == 2.5 => Some(*v),
                _ => None,
            })
            .expect("a ConstF for the float literal");
        assert_eq!(w.value_types[v.0 as usize], IrType::Float { bits: 64 });
    }

    #[test]
    fn lower_float_div_routes_to_div_op() {
        // `/` lowers to `BinOp::Div` whose result carries the float operand type.
        let ir = lower_src(": w ( -- f64 ) 1.0 2.0 / ;");
        let w = &ir.funcs[0];
        let v = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Div, _, _) => Some(*v),
                _ => None,
            })
            .expect("a Div bin op");
        assert_eq!(w.value_types[v.0 as usize], IrType::Float { bits: 64 });
    }

    #[test]
    fn lower_conv_pushes_target_typed_value() {
        // `5 >u8` lowers the literal, then a `Conv` whose dst carries the u8 type.
        let ir = lower_src(": w ( -- u8 ) 5 >u8 ;");
        let w = &ir.funcs[0];
        let dst = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Conv(dst, _) => Some(*dst),
                _ => None,
            })
            .expect("a Conv instr");
        assert_eq!(
            w.value_types[dst.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
    }

    #[test]
    fn lower_bitwise_and_or_xor_route_to_matching_binop() {
        let ir = lower_src(": w ( -- i32 ) 1 >i32 2 >i32 and 3 >i32 or 4 >i32 xor ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::And, _, _))));
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::Or, _, _))));
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::Xor, _, _))));
    }

    #[test]
    fn lower_not_emits_xor_with_neg1_const() {
        let ir = lower_src(": w ( -- u8 ) 5 >u8 not ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        let neg1 = is
            .iter()
            .find_map(|i| match i {
                Instr::Const(v, -1) => Some(*v),
                _ => None,
            })
            .expect("a -1 const");
        let xor = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Xor, _, b) if *b == neg1 => Some(*v),
                _ => None,
            })
            .expect("a xor against the -1 const");
        assert_eq!(
            w.value_types[xor.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
    }

    #[test]
    fn lower_not_on_bool_emits_xor_with_1_const_not_neg1() {
        // Type-directed `not`: on a `bool` it must flip the low bit
        // (`xor operand, 1`), not the integer-complement `xor operand, -1`,
        // since `-1`/`-2` are not valid canonical `bool` values.
        let ir = lower_src(": w ( -- bool ) true not ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        assert!(
            !is.iter().any(|i| matches!(i, Instr::Const(_, -1))),
            "bool `not` must not use a -1 mask"
        );
        let (xor_v, mask_operand) = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Xor, _, b) => Some((*v, *b)),
                _ => None,
            })
            .expect("a xor bin op");
        assert_eq!(w.value_types[xor_v.0 as usize], IrType::Bool);
        let mask_const = is.iter().find_map(|i| match i {
            Instr::Const(v, n) if *v == mask_operand => Some(*n),
            _ => None,
        });
        assert_eq!(mask_const, Some(1));
    }

    #[test]
    fn lower_bitwise_and_or_xor_accept_bool_operands() {
        let ir =
            lower_src(": w ( -- bool ) true false and true false or drop true false xor drop ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        for op in [BinOp::And, BinOp::Or, BinOp::Xor] {
            let v = is
                .iter()
                .find_map(|i| match i {
                    Instr::Bin(v, o, ..) if *o == op => Some(*v),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected a {op:?} bin op"));
            assert_eq!(w.value_types[v.0 as usize], IrType::Bool);
        }
    }

    #[test]
    fn lower_le_ge_ne_route_to_matching_cmpop() {
        let ir = lower_src(": w ( -- bool bool bool ) 1 2 <= 1 2 >= 1 2 <> ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        for op in [CmpOp::Le, CmpOp::Ge, CmpOp::Ne] {
            assert!(
                is.iter()
                    .any(|i| matches!(i, Instr::Cmp(_, o, _, _) if *o == op)),
                "expected a {op:?} comparison"
            );
        }
    }

    #[test]
    fn lower_shl_shr_route_to_matching_binop_with_lhs_type() {
        let ir = lower_src(": w ( -- u8 ) 200 >u8 3 shl 3 shr ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        let shl_ty = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Shl, _, _) => Some(*v),
                _ => None,
            })
            .expect("a Shl bin op");
        assert_eq!(
            w.value_types[shl_ty.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::Shr, _, _))));
    }

    #[test]
    fn lower_add_u8_result_is_u8_typed() {
        // Drive `lower_call`'s arithmetic arm with hand-typed u8 operands
        // directly, isolating the arm from parsing/checking, and assert the
        // result carries the operand type through to its `IrType`.
        let u8 = IrType::Int {
            bits: 8,
            signed: false,
        };
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let mut b = FuncBuilder::new(
            &env,
            &resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
            "w".to_string(),
        );
        let x = b.fresh_value(u8);
        let y = b.fresh_value(u8);
        b.stack = vec![x, y];
        b.lower_call("+", Span::default(), false);
        let top = *b.stack.last().unwrap();
        assert_eq!(b.value_type(top), u8);
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
    fn lower_constructor_allocs_and_stores_each_field() {
        // The constructor allocs one aggregate slot and width-exact-stores both
        // fields; no aggregate copy for a flat struct.
        let ir = lower_src("type: Vec2 x i64 y i64 ; : mk ( i64 i64 -- Vec2 ) Vec2 ;");
        let mk = ir.funcs.iter().find(|f| f.name == "mk").unwrap();
        assert_eq!(count(mk, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(mk, |i| matches!(i, Instr::FieldStore(..))), 2);
    }

    #[test]
    fn lower_getter_is_single_field_load_no_copy() {
        let ir = lower_src("type: Vec2 x i64 y i64 ; : gx ( Vec2 -- i64 ) Vec2>x ;");
        let gx = ir.funcs.iter().find(|f| f.name == "gx").unwrap();
        assert_eq!(count(gx, |i| matches!(i, Instr::FieldLoad(..))), 1);
        assert_eq!(count(gx, |i| matches!(i, Instr::Blit(..))), 0);
        assert_eq!(count(gx, |i| matches!(i, Instr::Alloc(..))), 0);
    }

    #[test]
    fn lower_setter_allocs_new_blits_all_and_overwrites_one_field() {
        // Functional update: alloc a fresh aggregate, blit all bytes, then a
        // single width-exact store of the replaced field.
        let ir = lower_src("type: Vec2 x i64 y i64 ; : sx ( Vec2 i64 -- Vec2 ) Vec2<x ;");
        let sx = ir.funcs.iter().find(|f| f.name == "sx").unwrap();
        assert_eq!(count(sx, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(sx, |i| matches!(i, Instr::Blit(..))), 1);
        assert_eq!(count(sx, |i| matches!(i, Instr::FieldStore(..))), 1);
    }

    #[test]
    fn lower_dup_of_struct_allocs_and_blits() {
        // R14: `dup` of a struct copies the aggregate bytes (fresh alloc +
        // blit), unlike a scalar `dup` which reuses the value id. Single
        // output plus a `drop` of the extra copy, so this measures only
        // `dup`'s own copy, not the multi-output bundle-pack path.
        let ir = lower_src("type: Vec2 x i64 y i64 ; : d ( Vec2 -- Vec2 ) dup drop ;");
        let d = ir.funcs.iter().find(|f| f.name == "d").unwrap();
        assert_eq!(count(d, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(d, |i| matches!(i, Instr::Blit(..))), 1);
    }

    #[test]
    fn lower_destructure_loads_every_field() {
        let ir = lower_src("type: Vec2 x i64 y i64 ; : ex ( Vec2 -- i64 i64 ) Vec2> ;");
        let ex = ir.funcs.iter().find(|f| f.name == "ex").unwrap();
        assert_eq!(count(ex, |i| matches!(i, Instr::FieldLoad(..))), 2);
    }

    #[test]
    fn lower_zero_field_constructor_allocs_destructure_emits_nothing() {
        let ir = lower_src("type: Unit ; : u ( -- ) Unit Unit> ;");
        let u = ir.funcs.iter().find(|f| f.name == "u").unwrap();
        assert_eq!(count(u, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(u, |i| matches!(i, Instr::FieldLoad(..))), 0);
        assert_eq!(count(u, |i| matches!(i, Instr::Blit(..))), 0);
    }

    #[test]
    fn ir_type_of_struct_maps_to_struct_irtype() {
        let tokens = lex("type: Vec2 x i64 y i64 ;").unwrap();
        let module = parse(&tokens).unwrap();
        let ty = module.resolve_type_name("Vec2").unwrap();
        assert!(matches!(ir_type_of(ty), IrType::Struct(_)));
    }

    #[test]
    fn ir_type_of_enum_maps_to_enum_irtype() {
        let tokens = lex("type: Shape | Circle r f64 | Rect w f64 h f64 ;").unwrap();
        let module = parse(&tokens).unwrap();
        let ty = module.resolve_type_name("Shape").unwrap();
        assert!(matches!(ir_type_of(ty), IrType::Enum(_)));
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
    fn enum_layout_all_zero_field_variants_is_tag_only() {
        // Every variant zero-field: payload align 1, payload_offset 4, no
        // payload, so size = 4, align = 4 (the tag's).
        let e = enums_of("type: Dir | N | E | S | W ;");
        let d = enum_layout(&e, "Dir");
        assert_eq!(d.payload_offset, 4);
        assert_eq!((d.size, d.align), (4, 4));
        assert_eq!(d.variants.len(), 4);
        assert!(d.variants.iter().all(|v| v.fields.is_empty()));
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
    fn lower_constructor_allocs_stores_tag_and_each_field() {
        // R15: a variant constructor allocs the tagged aggregate, stores the
        // discriminant as a `Const`, then width-exact-stores each field. Rect
        // has two fields, so: one Alloc, one tag Const, three FieldStores
        // (tag + two fields).
        let ir = lower_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ; : mk ( f64 f64 -- Shape ) Rect ;",
        );
        let mk = ir.funcs.iter().find(|f| f.name == "mk").unwrap();
        assert_eq!(count(mk, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(mk, |i| matches!(i, Instr::FieldStore(..))), 3);
        // The tag store writes the variant index (Rect = 1).
        assert!(instrs(mk).iter().any(|i| matches!(i, Instr::Const(_, 1))));
    }

    #[test]
    fn lower_zero_field_constructor_stores_only_the_tag() {
        // A zero-field variant constructs with just the tag store: one Alloc,
        // one FieldStore (the tag), no payload store.
        let ir = lower_src("type: MaybeInt | None | Some v i64 ; : n ( -- MaybeInt ) None ;");
        let n = ir.funcs.iter().find(|f| f.name == "n").unwrap();
        assert_eq!(count(n, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(n, |i| matches!(i, Instr::FieldStore(..))), 1);
        // None is variant index 0.
        assert!(instrs(n).iter().any(|i| matches!(i, Instr::Const(_, 0))));
    }

    #[test]
    fn lower_dup_of_enum_allocs_and_blits() {
        // R15: `dup` of an enum copies the aggregate bytes (fresh alloc +
        // blit), like a struct and unlike a scalar. Single output plus a
        // `drop` of the extra copy, so this measures only `dup`'s own copy,
        // not the multi-output bundle-pack path.
        let ir = lower_src(
            "type: MaybeInt | None | Some v i64 ; : d ( MaybeInt -- MaybeInt ) dup drop ;",
        );
        let d = ir.funcs.iter().find(|f| f.name == "d").unwrap();
        assert_eq!(count(d, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(d, |i| matches!(i, Instr::Blit(..))), 1);
    }

    #[test]
    fn carried_slot_bytes_enum_is_aligned_aggregate() {
        // R17: a carried enum slot occupies its size rounded up to a multiple
        // of 8. Shape is 24 bytes (already a multiple of 8); a tag-only enum
        // (4 bytes) rounds up to one 8-byte cell.
        let e = enums_of("type: Shape | Circle r f64 | Rect w f64 h f64 ; type: Dir | N | S ;");
        assert_eq!(
            carried_slot_bytes(
                IrType::Enum(EnumId::from_index(0)),
                &Structs::default(),
                &e,
                &Arrays::default()
            ),
            24
        );
        assert_eq!(
            carried_slot_bytes(
                IrType::Enum(EnumId::from_index(1)),
                &Structs::default(),
                &e,
                &Arrays::default()
            ),
            8
        );
    }

    #[test]
    fn lower_line_enum_slot_blits_in_and_out() {
        // R17: a carried enum slot is copied out of the buffer on entry and
        // back on exit by aggregate blits, and the returned top advances by
        // the enum's aligned carried size. An empty line carries the one Shape
        // straight through: one prologue blit, one epilogue blit.
        let src = "type: Shape | Circle r f64 | Rect w f64 h f64 ;";
        let (structs, enums, arrays, cells, refs) = {
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
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let shape = Type::Enum(EnumId::from_index(0), "Shape");
        let (func, m, out_bytes) = lower_line(
            0,
            &line_terms(""),
            1,
            &[shape],
            &env,
            &resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
            empty_instantiations(),
            empty_poly_arities(),
        );
        assert_eq!(m, 1);
        assert_eq!(out_bytes, 24);
        assert_eq!(count(&func, |i| matches!(i, Instr::Blit(..))), 2);
        assert_eq!(count(&func, |i| matches!(i, Instr::Load(..))), 0);
        assert_eq!(count(&func, |i| matches!(i, Instr::Store(..))), 0);
    }

    #[test]
    fn lower_clause_word_builds_nway_dispatch_and_join_phi() {
        // R16: a clause word loads the discriminant (one FieldLoad on the
        // scrutinee tag), builds an N-way `Cmp(Eq)` compare-chain (N-1
        // compares for N variants, the last variant a fall-through), and
        // merges the clauses at a single join with one Phi per declared
        // output. A 4-variant enum: 3 Cmp(Eq), one Phi.
        let ir = lower_src(
            "type: Cmd | Halt | Push v i64 | Add | Dbl ;
             : run ( i64 Cmd -- i64 ) | Halt drop 0 | Push swap drop | Add 1 + | Dbl 2 * ;",
        );
        let run = ir.funcs.iter().find(|f| f.name == "run").unwrap();
        // Three `Cmp(Eq)` compares for four variants (the last falls through).
        assert_eq!(
            count(run, |i| matches!(i, Instr::Cmp(_, CmpOp::Eq, _, _))),
            3
        );
        // Exactly one Phi (single declared output) merging all four clauses.
        let phi_arms: Vec<usize> = run
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::Phi(_, arms) => Some(arms.len()),
                _ => None,
            })
            .collect();
        assert_eq!(phi_arms, vec![4]);
    }

    #[test]
    fn lower_single_variant_clause_word_jumps_without_compare() {
        // R16: a single-variant (newtype) enum needs no compare — the sole
        // clause is the terminal fall-through, reached by a direct jump.
        let ir = lower_src("type: Id | Wrap v i64 ; : unwrap ( Id -- i64 ) | Wrap ;");
        let unwrap = ir.funcs.iter().find(|f| f.name == "unwrap").unwrap();
        assert_eq!(count(unwrap, |i| matches!(i, Instr::Cmp(..))), 0);
    }

    /// The loop header of a self-tail-recursive word: the entry block (block 0)
    /// jumps to it (R6), so its id is the entry's `Jmp` target.
    fn loop_header(func: &IrFunc) -> BlockId {
        match func.blocks[0].term {
            Terminator::Jmp(h) => h,
            ref t => panic!("entry block should Jmp to the loop header, got {t:?}"),
        }
    }

    fn header_block(func: &IrFunc, header: BlockId) -> &Block {
        func.blocks.iter().find(|b| b.id == header).expect("header")
    }

    fn header_phis(block: &Block) -> Vec<&Vec<(BlockId, Value)>> {
        block
            .instrs
            .iter()
            .filter_map(|i| match i {
                Instr::Phi(_, arms) => Some(arms),
                _ => None,
            })
            .collect()
    }

    fn jmps_to(func: &IrFunc, target: BlockId) -> usize {
        func.blocks
            .iter()
            .filter(|b| matches!(b.term, Terminator::Jmp(h) if h == target))
            .count()
    }

    #[test]
    fn tail_self_call_lowers_to_back_edge_not_call() {
        // Criterion 2 (R6/R7/R8): a self-tail-recursive word lowers to a header
        // carrying one phi per loop-carried (input-arity) slot, and the tail
        // self-call is a `Jmp` back to that header with no `Instr::Call` to
        // self. `go` has input arity 2, so the header has two phis.
        let ir = lower_src(": go ( i64 i64 -- i64 ) dup 0 > if 1 - go else drop end ;");
        let f = &ir.funcs[0];
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        assert_eq!(phis.len(), 2, "one header phi per loop-carried slot");
        // Each phi has the entry arm plus the single back-edge arm.
        assert!(phis.iter().all(|arms| arms.len() == 2));
        // Entry + one back-edge both target the header.
        assert_eq!(jmps_to(f, header), 2);
        assert_eq!(
            count(f, |i| matches!(i, Instr::Call(..))),
            0,
            "tail self-call is a back-edge, not a Call"
        );
    }

    /// The header phi structure that matters for R11: how many phis, how many
    /// arms each has, and how many jumps target the header. Deliberately
    /// ignores the carried `Value`s themselves, since those differ between
    /// two independently-lowered programs even when the shape is identical.
    fn header_phi_shape(func: &IrFunc, header: BlockId) -> (usize, Vec<usize>, usize) {
        let phis = header_phis(header_block(func, header));
        let phi_count = phis.len();
        let arm_counts = phis.iter().map(|arms| arms.len()).collect();
        (phi_count, arm_counts, jmps_to(func, header))
    }

    #[test]
    fn lower_mid_body_binding_adds_no_header_phi() {
        // Criterion 22 (R11): a mid-body binding inside a self-tail-recursive
        // arm has its extent end at the arm's terminator, where the back-edge
        // sits, so no name is live across it and the header still carries
        // exactly one phi per loop-carried (input-arity) slot, unaffected by
        // the binding. Proved by comparing against a binding-free equivalent:
        // if a bound name ever leaked a phi onto the header, this source's
        // shape would diverge from the one below instead of both trivially
        // satisfying the same hard-coded numbers.
        let with_binding =
            lower_src(": go ( i64 i64 -- i64 ) dup 0 > if | x | 1 - x go else drop end ;");
        let without_binding =
            lower_src(": go ( i64 i64 -- i64 ) dup 0 > if 1 - go else drop end ;");
        let f1 = &with_binding.funcs[0];
        let f2 = &without_binding.funcs[0];
        let header1 = loop_header(f1);
        let header2 = loop_header(f2);
        let shape1 = header_phi_shape(f1, header1);
        let shape2 = header_phi_shape(f2, header2);
        assert_eq!(
            shape1, shape2,
            "a mid-body binding must not change the header's phi structure"
        );
        assert_eq!(shape1.0, 2, "one header phi per loop-carried slot");
    }

    #[test]
    fn non_tail_self_call_stays_a_call() {
        // R10: a self-call followed by more work (`fact *`) is not in tail
        // position, so it stays a real `Instr::Call` and no loop is built.
        let ir = lower_src(": fact ( i64 -- i64 ) dup 0 = if drop 1 else dup 1 - fact * end ;");
        let f = &ir.funcs[0];
        assert_eq!(
            count(f, |i| matches!(i, Instr::Call(..))),
            1,
            "non-tail self-call stays a real Call"
        );
        assert!(
            !matches!(f.blocks[0].term, Terminator::Jmp(_)),
            "a non-tail-recursive word builds no loop header"
        );
    }

    #[test]
    fn self_call_in_non_terminal_if_stays_a_call() {
        // R10 over-eager boundary: the `if` is followed by more terms
        // (`drop 5`), so it is non-terminal and its arms are not in tail
        // position; the self-call stays a real `Instr::Call`.
        let ir = lower_src(": w ( i64 -- i64 ) dup 0 > if w else drop 0 end drop 5 ;");
        let f = &ir.funcs[0];
        assert_eq!(count(f, |i| matches!(i, Instr::Call(..))), 1);
        assert!(!matches!(f.blocks[0].term, Terminator::Jmp(_)));
    }

    #[test]
    fn both_if_arms_tail_produce_two_back_edges() {
        // R8 multi-arm back-patch through `lower_if`: a self-tail-call in each
        // arm of a terminal `if` back-edges, so the single header phi gains two
        // back-edge arms on top of the entry arm (three total).
        let ir = lower_src(": go ( i64 -- i64 ) dup 0 > if 1 - go else 1 + go end ;");
        let f = &ir.funcs[0];
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        assert_eq!(phis.len(), 1);
        assert_eq!(phis[0].len(), 3, "entry arm + two back-edge arms");
        assert_eq!(jmps_to(f, header), 3);
        assert_eq!(count(f, |i| matches!(i, Instr::Call(..))), 0);
    }

    #[test]
    fn clause_tails_share_one_header() {
        // R9: a `|`-clause self-tail-recursive word gets a single header; each
        // clause's terminal self-call is one back-edge into it. Both clauses
        // here tail-recurse, so each of the two header phis has three arms
        // (entry + two back-edges) and no `Instr::Call` to self remains.
        let ir = lower_src(
            "type: Flag | Go | Stop ; \
             : loop2 ( i64 Flag -- i64 ) | Go 1 - Go loop2 | Stop 1 + Stop loop2 ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "loop2").unwrap();
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        // R2: the `Flag` (enum) slot loses its header phi under the aggregate-
        // staging transform, leaving only the `i64` scalar phi (was 2).
        assert_eq!(phis.len(), 1, "only the i64 scalar slot keeps a header phi");
        assert!(phis.iter().all(|arms| arms.len() == 3));
        assert_eq!(jmps_to(f, header), 3, "entry + two clause back-edges");
        assert_eq!(count(f, |i| matches!(i, Instr::Call(..))), 0);
    }

    #[test]
    fn mixed_clause_header_and_join_predecessors_stay_disjoint() {
        // R9 / risk 5: some clauses back-edge and one is a base case that
        // `Ret`s. The loop header phi (preds = entry + tail clause ends) and
        // the Slice-4 dispatch-join phi (preds = non-tail clause ends) must
        // keep disjoint predecessor sets.
        let ir = lower_src(
            "type: Flag | Go | Stop ; \
             : run ( i64 Flag -- i64 ) | Go 1 - Stop run | Stop ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "run").unwrap();
        let header = loop_header(f);
        let hb = header_block(f, header);
        let hphis = header_phis(hb);
        // R2: the `Flag` (enum) slot loses its header phi, leaving the i64 one.
        assert_eq!(hphis.len(), 1);
        // header preds: entry arm + the one Go back-edge.
        assert!(hphis.iter().all(|arms| arms.len() == 2));
        assert!(
            f.blocks
                .iter()
                .any(|b| matches!(b.term, Terminator::Ret(_))),
            "the Stop base case still Rets"
        );
        // Every phi that is not a header phi is a dispatch/join phi; its
        // predecessors must not overlap the header phi's predecessors.
        let header_preds: std::collections::HashSet<u32> = hphis
            .iter()
            .flat_map(|arms| arms.iter().map(|(p, _)| p.0))
            .collect();
        for block in &f.blocks {
            if block.id == header {
                continue;
            }
            for instr in &block.instrs {
                if let Instr::Phi(_, arms) = instr {
                    for (p, _) in arms {
                        assert!(
                            !header_preds.contains(&p.0),
                            "join phi pred {p:?} collides with a header phi pred"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn clause_tail_call_alloc_is_hoisted_to_entry_not_loop_body() {
        // A clause self-tail-call rebuilds its enum scrutinee on every
        // back-edge (`Go`/`Stop` above are payload-free, but the tag store
        // still needs a slot). If that `Alloc` stayed in the loop body, QBE's
        // `alloc*` would bump the frame pointer every iteration and blow the
        // stack well before Phase 4's N >= 1_000_000 golden. It must land in
        // the entry block instead, so the loop body has none.
        let ir = lower_src(
            "type: Flag | Go | Stop ; \
             : run ( i64 Flag -- i64 ) | Go 1 - Stop run | Stop ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "run").unwrap();
        let header = loop_header(f);
        let entry = &f.blocks[0];
        assert!(
            entry.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
            "the Stop scrutinee's alloc should be hoisted into the entry block"
        );
        let entry_id = entry.id;
        for block in &f.blocks {
            if block.id == entry_id || block.id == header {
                continue;
            }
            assert!(
                !block.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
                "block {:?} in the loop body must not alloc",
                block.id
            );
        }
    }

    // Phase 4 Slice 3: the aggregate-staging loop transform (R1-R4, R1a).
    // Structural coverage beside the changed `begin_loop`/`finalize_loop`; the
    // runtime witnesses are the `tests/phase4_generics.rs` goldens.

    /// A self-tail loop carrying an i64 (scalar) and a re-produced `Box`
    /// (aggregate), so the aggregate slot stages rather than forwards.
    const STAGED_LOOP: &str = "type: Box n i64 ;\n\
         : mk ( i64 -- Box ) | n | n Box ;\n\
         : loop ( i64 Box -- Box ) | n b | n 0 = if b else n 1 - n mk loop end ;";

    #[test]
    fn aggregate_carried_slot_gets_no_header_phi_but_scalar_does() {
        // R2: the aggregate (`Box`) slot contributes no header phi (it reads
        // its entry-hoisted stable slot); the scalar (i64) slot keeps one.
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        assert_eq!(
            phis.len(),
            1,
            "only the i64 scalar slot carries a header phi"
        );
        // `len() == 1` alone would also pass a transform that kept the `Box`
        // slot's phi and dropped the scalar's; pin that the survivor carries
        // the i64 counter, not a `Box` pointer, so "but scalar does" is checked.
        let (_, incoming) = phis[0][0];
        assert_eq!(
            f.value_types[incoming.0 as usize],
            IrType::I64,
            "the surviving header phi carries the scalar slot, not the aggregate"
        );
    }

    #[test]
    fn aggregate_stable_slot_and_temp_are_entry_hoisted_not_in_the_body() {
        // R1/R9: the stable slot and staging temp are `alloc`ed in the entry
        // block, not per-iteration in the body (which would bump the frame
        // every iteration and break the constant-stack guarantee). `instrs`
        // flattens across blocks, so this iterates `func.blocks` directly.
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let entry = &f.blocks[0];
        let entry_allocs = entry
            .instrs
            .iter()
            .filter(|i| matches!(i, Instr::Alloc(..)))
            .count();
        assert!(
            entry_allocs >= 2,
            "the stable slot and temp allocs should be hoisted into the entry block, saw {entry_allocs}"
        );
        let entry_id = entry.id;
        for block in &f.blocks {
            if block.id == entry_id || block.id == header {
                continue;
            }
            assert!(
                !block.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
                "block {:?} in the loop body must not alloc",
                block.id
            );
        }
    }

    #[test]
    fn aggregate_init_blit_lands_in_the_entry_block() {
        // R3: `begin_loop` seeds the stable slot with the incoming param once,
        // in the entry block, so iteration 1 reads an initialised value. It is
        // the only Blit routed to the entry block (the back-edge staging blits
        // go to predecessor blocks).
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let entry = &f.blocks[0];
        assert!(
            entry.instrs.iter().any(|i| matches!(i, Instr::Blit(..))),
            "the entry-arm init blit should land in the entry block"
        );
    }

    /// The back-edge predecessor block of a self-tail loop: the non-entry block
    /// that jumps to the header.
    fn back_edge_pred(f: &IrFunc, header: BlockId) -> &Block {
        let entry_id = f.blocks[0].id;
        f.blocks
            .iter()
            .find(|b| b.id != entry_id && matches!(b.term, Terminator::Jmp(h) if h == header))
            .expect("a back-edge predecessor block")
    }

    #[test]
    fn back_edge_stages_reads_before_writes() {
        // R4: on a staged back-edge, every read-phase blit (a snapshot into a
        // temp) precedes every write-phase blit (a store into the stable slot).
        // A blit is write-phase when its source is an earlier blit's dest in
        // the same predecessor block. `instrs` flattens across blocks, so this
        // inspects the predecessor block directly.
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let pred = back_edge_pred(f, header);
        let mut written: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let mut seen_write = false;
        let mut blits = 0;
        for instr in &pred.instrs {
            if let Instr::Blit(src, dst, _) = instr {
                blits += 1;
                if written.contains(&src.0) {
                    seen_write = true;
                } else {
                    assert!(!seen_write, "a read-phase blit follows a write-phase blit");
                }
                written.insert(dst.0);
            }
        }
        assert!(
            blits >= 2,
            "the staged Box back-edge should emit a read and a write blit, saw {blits}"
        );
    }

    #[test]
    fn forwarded_in_place_aggregate_slot_emits_zero_back_edge_blits() {
        // R4: an aggregate carried unchanged (`prev`, its back-edge arg is
        // exactly its own stable slot) is forwarded in place and stages
        // nothing.
        let ir = lower_src(
            "type: Box n i64 ;\n\
             : mk ( i64 -- Box ) | n | n Box ;\n\
             : loop ( i64 Box -- Box ) | n prev | n 0 = if prev else n 1 - prev loop end ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let pred = back_edge_pred(f, header);
        assert_eq!(
            pred.instrs
                .iter()
                .filter(|i| matches!(i, Instr::Blit(..)))
                .count(),
            0,
            "a forwarded-in-place slot emits zero back-edge blits"
        );
    }

    #[test]
    fn recursive_type_destructor_is_not_transformed() {
        // R1a: the fused iterative destructor's `begin_loop` is gated OFF, so a
        // recursive type's synthesized destructor keeps its one header phi for
        // the carried node (R2 would drop it to zero) and gains no entry-block
        // init Blit (R3's blit is the only Blit the transform routes to the
        // entry block; the destructor's own copy-out lands in a body block).
        // This is the check that is red when the gate is missing.
        let ir = lower_src(
            "type: Res n i64 ;\n\
             : drop ( Res -- ) | r | r Res>n 5000 + . ;\n\
             : mkres ( i64 -- Res ) | n | n Res ;\n\
             type: List | Nil | Cons v Res next ^List ;\n\
             : w ( -- ) ;",
        );
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_enum_drop_0")
            .expect("a fused destructor was synthesized for the recursive enum");
        let header = loop_header(dtor);
        let phis = header_phis(header_block(dtor, header));
        assert_eq!(
            phis.len(),
            1,
            "the ungated-off destructor keeps its one carried-node header phi"
        );
        let entry = &dtor.blocks[0];
        assert!(
            !entry.instrs.iter().any(|i| matches!(i, Instr::Blit(..))),
            "the destructor gains no entry-block init blit (R1a gate holds)"
        );
    }

    // Phase 3 Slice 1: the drop-spy's lowering (R5/R6/R16).

    #[test]
    fn lower_struct_constructor_emits_no_call_only_alloc_and_store() {
        // Constructing a linear struct value is inlined alloc + field
        // stores, not a runtime call: only `drop`'s own destructor call is
        // emitted.
        let ir = lower_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy drop ;"));
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let is = instrs(w);
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(
            count(
                w,
                |i| matches!(i, Instr::Call(_, sym, _) if sym != &spy_drop)
            ),
            0,
            "the constructor emits no call: {is:?}"
        );
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 1, "{is:?}");
        assert_eq!(
            count(w, |i| matches!(i, Instr::FieldStore(..))),
            1,
            "{is:?}"
        );
    }

    #[test]
    fn lower_drop_of_linear_value_calls_the_destructor() {
        let ir = lower_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy drop ;"));
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let calls: Vec<&String> = instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, args) if args.len() == 1 => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(
            calls,
            vec![spy_drop.as_str()],
            "expected one destructor call"
        );
    }

    #[test]
    fn lower_drop_of_copy_value_emits_no_destructor_call() {
        // R2: `drop` on a Copy value keeps its no-runtime-effect discard.
        let ir = lower_src(": w ( -- ) 7 drop ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::Call(..))), 0);
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
    fn lower_owned_cell_unwrap_scalar_loads_before_freeing() {
        // R13: `^>` must materialise the payload before calling `sooth_free`,
        // so the freed pointer is never handed to the stack.
        let ir = lower_src(": w ( -- i64 ) 5 ^ ^> ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        let load_at = is
            .iter()
            .position(|i| matches!(i, Instr::FieldLoad(..)))
            .expect("a FieldLoad");
        let free_at = is
            .iter()
            .position(|i| matches!(i, Instr::Call(None, sym, _) if sym == FREE_SYMBOL))
            .expect("a free call");
        assert!(
            load_at < free_at,
            "scalar payload must load before the cell frees: load at {load_at}, free at {free_at}"
        );
    }

    #[test]
    fn lower_owned_cell_unwrap_aggregate_blits_before_freeing() {
        // The aggregate counterpart of the scalar case above (R13): the copy-out
        // `Blit` must precede `sooth_free`, never aliasing the freed cell.
        let ir = lower_src("type: Point x i64 y i64 ; : w ( -- Point ) 1 2 Point ^ ^> ;");
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let is = instrs(w);
        let blit_at = is
            .iter()
            .position(|i| matches!(i, Instr::Blit(..)))
            .expect("a Blit");
        let free_at = is
            .iter()
            .position(|i| matches!(i, Instr::Call(None, sym, _) if sym == FREE_SYMBOL))
            .expect("a free call");
        assert!(
            blit_at < free_at,
            "aggregate payload must blit out before the cell frees: blit at {blit_at}, free at {free_at}"
        );
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

    #[test]
    fn lower_appends_one_destructor_func_per_linear_struct_only() {
        // R12: a synthesized destructor exists for every linear struct type,
        // and only those (a Copy struct needs no glue, so gets no function).
        // `Plain` (index 1, Copy) gets no destructor; `Holds` (index 2,
        // linear) does.
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Plain x i64 y i64 ; \
             type: Holds a Spy b i64 ; \
             : w ( -- ) ;"
        ));
        assert!(ir.funcs.iter().any(|f| f.name == "sooth_struct_drop_2"));
        assert!(!ir.funcs.iter().any(|f| f.name == "sooth_struct_drop_1"));
    }

    #[test]
    fn lower_drop_of_whole_linear_struct_calls_its_synthesized_destructor() {
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Holds a Spy b i64 ; : w ( -- ) 1 Spy 2 Holds drop ;"
        ));
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let calls: Vec<&String> = instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, args) if args.len() == 1 => Some(sym),
                _ => None,
            })
            .collect();
        let holds_drop = struct_drop_symbol(StructId::from_index(1), None);
        assert_eq!(calls, vec![holds_drop.as_str()]);
    }

    #[test]
    fn synthesized_struct_destructor_drops_linear_fields_in_declaration_order() {
        // R12: struct -> drop its linear fields in declaration order. `Holds`
        // has a linear field (`a`) then a Copy one (`b`), so the destructor
        // calls `Spy`'s destructor exactly once, for `a`.
        let ir = lower_src(&format!("{SPY_DEF}type: Holds a Spy b i64 ; : w ( -- ) ;"));
        let holds_drop = struct_drop_symbol(StructId::from_index(1), None);
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == holds_drop)
            .expect("a destructor was synthesized for the linear struct");
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(calls, vec![spy_drop.as_str()]);
    }

    #[test]
    fn lower_appends_a_destructor_func_for_every_cell_even_a_copy_payload() {
        // R8: unlike the struct/enum filters above, *every* cell gets a
        // destructor, because `drop` on a cell must free it whatever its
        // payload is. `^i64`'s payload is Copy and it still gets one.
        let ir = lower_src(": w ( -- ) 5 ^ drop ;");
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_cell_drop_0")
            .expect("a Copy-payload cell still gets a destructor");
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            vec![FREE_SYMBOL],
            "a Copy payload frees and nothing else"
        );
    }

    #[test]
    fn synthesized_cell_destructor_frees_before_dropping_a_linear_aggregate_payload() {
        // An aggregate payload is copied out of the cell (a Blit), then
        // the block is freed, and only then does the copy's own drop
        // glue run. The `^Spy` golden covers the scalar payload at
        // runtime; this pins the aggregate path, where the copy-out must
        // still complete before anything else touches the block or the copy.
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Holds a Spy b i64 ; : w ( -- ) 1 Spy 2 Holds ^ drop ;"
        ));
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_cell_drop_0")
            .expect("a destructor was synthesized for the cell");
        let is = instrs(dtor);
        let blit_at = is
            .iter()
            .position(|i| matches!(i, Instr::Blit(..)))
            .expect("a copy-out Blit");
        let calls: Vec<(usize, &String)> = is
            .iter()
            .enumerate()
            .filter_map(|(at, i)| match i {
                Instr::Call(None, sym, _) => Some((at, sym)),
                _ => None,
            })
            .collect();
        let holds_drop = struct_drop_symbol(StructId::from_index(1), None);
        assert_eq!(
            calls
                .iter()
                .map(|(_, sym)| sym.as_str())
                .collect::<Vec<_>>(),
            vec![FREE_SYMBOL, holds_drop.as_str()],
            "the cell frees, then the payload's own destructor runs"
        );
        assert!(
            blit_at < calls[0].0,
            "the payload must be copied out before the block is freed: blit at {blit_at}, free at {}",
            calls[0].0
        );
    }

    // Phase 3 Slice 1, Phase 4: the synthesized enum destructor's own tag
    // dispatch (structural, not full-stdout: `tests/phase0.rs` covers the
    // 2-variant runtime behavior; these pin the shapes it doesn't reach).

    #[test]
    fn synthesized_enum_destructor_newtype_skips_the_tag_compare() {
        // R7/R12: a single-variant enum (n == 1) has nothing to dispatch on,
        // so the destructor jumps straight to the one variant block instead
        // of loading a tag and comparing it (the `n == 1` branch of
        // `dispatch_on_tag`, otherwise unreached by the 2-variant goldens).
        let ir = lower_src(&format!("{SPY_DEF}type: Box | Full v Spy ; : w ( -- ) ;"));
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_enum_drop_0")
            .expect("a destructor was synthesized for the linear enum");
        assert_eq!(count(dtor, |i| matches!(i, Instr::Cmp(..))), 0);
        assert_eq!(
            dtor.blocks.len(),
            2,
            "a bare `Jmp` to the one variant block, no compare block"
        );
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(calls, vec![spy_drop.as_str()]);
    }

    #[test]
    fn synthesized_enum_destructor_three_variants_chains_through_a_middle_block() {
        // R7/R12: with 3 variants the compare chain has an intermediate block
        // between the first and last compare (`vi < n - 2` in
        // `dispatch_on_tag`), never built by the 2-variant goldens. Each of
        // the 3 variants gets its own block; only `Full`'s carries a drop.
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Item | Empty | Full v Spy | Named n i64 ; : w ( -- ) ;"
        ));
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_enum_drop_0")
            .expect("a destructor was synthesized for the linear enum");
        assert_eq!(dtor.blocks.len(), 5, "2 compares + 3 variant blocks");
        assert_eq!(count(dtor, |i| matches!(i, Instr::Cmp(..))), 2);
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(calls, vec![spy_drop.as_str()]);
    }

    // Unit-level coverage of `recursive_disposal_path`'s path-finding: which
    // steps it finds for a shape, distinct from the runtime goldens in
    // tests/phase0.rs that prove those shapes actually dispose correctly.

    #[test]
    fn recursive_disposal_path_finds_indirect_nested_mutual_and_composed_cycles() {
        // The wrapper-struct list: the cell is one byval struct hop away from
        // the enum that owns it, so the path is a tag dispatch, a projection
        // into `Wrap`, then the unwrap.
        let p = Probe::new(
            "type: Wrap v i64 next ^List ;\n\
             type: List | Nil | Cons w Wrap ;\n\
             : main ( -- ) ;",
        );
        let list = p.enum_ty("List");
        assert_eq!(
            p.path(list),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(0),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Project { field: 0 },
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(list),
                        },
                    ]),
                ],
            }])
        );
        // The same cycle rooted at `Wrap` instead: one rotation of it, the
        // dispatch now mid-path (every type on the cycle gets its own
        // loop, entered from its own shape).
        assert_eq!(
            p.path(p.struct_ty("Wrap")),
            Some(vec![
                PathStep::Unwrap {
                    field: Some(1),
                    cell: p.cell(list),
                },
                PathStep::Branch {
                    enum_id: EnumId::from_index(0),
                    variants: vec![None, Some(vec![PathStep::Project { field: 0 }])],
                },
            ])
        );

        // `^^Self`: the outer unwrap names the field, the inner one cannot
        // (the current type *is* the cell at that point).
        let p = Probe::new(
            "type: L | Nil | Cons n i64 next ^^L ;\n\
             : main ( -- ) ;",
        );
        let l = p.enum_ty("L");
        let inner = p.cell(l);
        assert_eq!(
            p.path(l),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(0),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(IrType::OwnedCell(inner)),
                        },
                        PathStep::Unwrap {
                            field: None,
                            cell: inner,
                        },
                    ]),
                ],
            }])
        );

        // The mutual A/B chain, from both directions: `A` dispatches at entry,
        // `B` (a plain struct, no tag of its own) dispatches mid-path.
        let p = Probe::new(
            "type: A | ANil | ACons x i64 next ^B ;\n\
             type: B y i64 z ^A ;\n\
             : main ( -- ) ;",
        );
        let (a, b) = (p.enum_ty("A"), p.struct_ty("B"));
        assert_eq!(
            p.path(a),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(0),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(b),
                        },
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(a),
                        },
                    ]),
                ],
            }])
        );
        assert_eq!(
            p.path(b),
            Some(vec![
                PathStep::Unwrap {
                    field: Some(1),
                    cell: p.cell(a),
                },
                PathStep::Branch {
                    enum_id: EnumId::from_index(0),
                    variants: vec![
                        None,
                        Some(vec![PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(b),
                        }]),
                    ],
                },
            ])
        );

        // Composition: a wrapper struct sitting inside a two-type cycle, so
        // one path threads three unwraps through three distinct types.
        let p = Probe::new(
            "type: P q ^W ;\n\
             type: W m i64 next ^Q ;\n\
             type: Q r ^P ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(
            p.path(p.struct_ty("P")),
            Some(vec![
                PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(p.struct_ty("W")),
                },
                PathStep::Unwrap {
                    field: Some(1),
                    cell: p.cell(p.struct_ty("Q")),
                },
                PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(p.struct_ty("P")),
                },
            ])
        );
    }

    #[test]
    fn recursive_disposal_path_finds_multi_variant_and_enum_enum_mutual_cycles() {
        // Two independently recursive variants: both continue, because an
        // enum's variants are mutually exclusive at runtime and so are not
        // the simultaneously-live branching case a struct's own field choice
        // must narrow. Collapsing to one would regress a program that
        // already disposes in constant stack.
        let p = Probe::new(
            "type: T | Nil | X n i64 next ^T | Y m i64 next ^T ;\n\
             : main ( -- ) ;",
        );
        let t = p.enum_ty("T");
        let step = vec![PathStep::Unwrap {
            field: Some(1),
            cell: p.cell(t),
        }];
        assert_eq!(
            p.path(t),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(0),
                variants: vec![None, Some(step.clone()), Some(step)],
            }])
        );

        // The enum/enum mutual pair: two nested `Branch` steps, the inner one
        // dispatched partway along the path rather than at the entry.
        let p = Probe::new(
            "type: A | ANil | ACons x i64 next ^B ;\n\
             type: B | BNil | BCons y i64 next ^A ;\n\
             : main ( -- ) ;",
        );
        let (a, b) = (p.enum_ty("A"), p.enum_ty("B"));
        assert_eq!(
            p.path(a),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(0),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(b),
                        },
                        PathStep::Branch {
                            enum_id: EnumId::from_index(1),
                            variants: vec![
                                None,
                                Some(vec![PathStep::Unwrap {
                                    field: Some(1),
                                    cell: p.cell(a),
                                }]),
                            ],
                        },
                    ]),
                ],
            }])
        );
    }

    #[test]
    fn recursive_disposal_path_rejects_non_cyclic_and_misleading_shapes() {
        // No cell at all: nothing to walk.
        let p = Probe::new(&format!(
            "{SPY_DEF}type: Plain x i64 y Spy ;\n: main ( -- ) ;"
        ));
        assert_eq!(p.path(p.struct_ty("Plain")), None);

        // The bait is the *last* field, which is where the reverse-order scan
        // starts, and the genuine edge is indirect, so the direct-field tier
        // cannot short-circuit past it: the scan must try `bait`, walk into
        // `Bait` and `Leafy`, fail, and back up to `good`. A greedy search
        // that committed to the first cell field it saw would return `None`.
        let p = Probe::new(
            "type: Leafy v i64 ;\n\
             type: Bait c ^Leafy ;\n\
             type: Hop n ^Node ;\n\
             type: Node good Hop bait ^Bait ;\n\
             : main ( -- ) ;",
        );
        let node = p.struct_ty("Node");
        assert_eq!(
            p.path(node),
            Some(vec![
                PathStep::Project { field: 0 },
                PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(node),
                },
            ])
        );

        // `^^Other`: the walk does step through the inner cell (that is how
        // `^^Self` is found at all), and still bottoms out in a dead end.
        let p = Probe::new(
            "type: Other v i64 ;\n\
             type: Twice c ^^Other ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(p.path(p.struct_ty("Twice")), None);

        // Two unrelated self-recursive types: each finds its own edge and
        // neither path wanders into the other type.
        let p = Probe::new(
            "type: R1 n ^R1 ;\n\
             type: R2 n ^R2 ;\n\
             : main ( -- ) ;",
        );
        for name in ["R1", "R2"] {
            let ty = p.struct_ty(name);
            assert_eq!(
                p.path(ty),
                Some(vec![PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(ty),
                }])
            );
        }
    }

    #[test]
    fn recursive_disposal_path_prefers_direct_field_over_later_indirect_one() {
        // `a` is a direct `^Self` field; `b` is declared after it and also
        // reaches `Self`, but only by way of `Wrap`'s own cell field. Without
        // a preferred tier for direct fields, the reverse scan tries `b`
        // first and finds it succeeds, silently swapping in the longer route
        // and lengthening every iteration of the fused loop.
        let p = Probe::new(
            "type: Wrap v i64 n ^List ;\n\
             type: List a ^List b Wrap ;\n\
             : main ( -- ) ;",
        );
        let list = p.struct_ty("List");
        assert_eq!(
            p.path(list),
            Some(vec![PathStep::Unwrap {
                field: Some(0),
                cell: p.cell(list),
            }])
        );

        // The same trap one level up, between an enum's variants: each
        // variant picks its own edge independently, `Direct`'s direct one and
        // `Indirect`'s route through `Wrap`.
        let p = Probe::new(
            "type: Wrap v i64 n ^E ;\n\
             type: E | Nil | Direct d ^E | Indirect w Wrap ;\n\
             : main ( -- ) ;",
        );
        let e = p.enum_ty("E");
        assert_eq!(
            p.path(e),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(0),
                variants: vec![
                    None,
                    Some(vec![PathStep::Unwrap {
                        field: Some(0),
                        cell: p.cell(e),
                    }]),
                    Some(vec![
                        PathStep::Project { field: 0 },
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(e),
                        },
                    ]),
                ],
            }])
        );
    }

    #[test]
    fn quotation_taking_word_emits_no_call_and_no_irfunc() {
        // Criterion 3b/R20: a monomorphic quotation-taking word is inlined, so
        // it mints no `IrFunc` and its caller emits no `Instr::Call`. The
        // lowered `main` is just `1 +` (the spliced literal over `3`), a pure
        // arithmetic body. Deleting the `combinator_indices` filter would put
        // an `apply` func back, and deleting the `lower_call` inline branch
        // would leave an `Instr::Call apply` in `main`.
        let ir = lower_src(
            ": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
             : main ( -- ) 3 [ 1 + ] apply . ;\n",
        );
        assert!(
            ir.funcs.iter().all(|f| f.name != "apply"),
            "a combinator mints no `IrFunc`, but one named `apply` was emitted"
        );
        let main = ir
            .funcs
            .iter()
            .find(|f| f.name == "main")
            .expect("`main` is emitted");
        assert!(
            call_symbols(main).is_empty(),
            "the inlined caller emits no `Instr::Call`, got: {:?}",
            call_symbols(main)
        );
    }

    #[test]
    fn abstract_forward_inlines_transitively_with_no_call() {
        // Criterion 10b (R21): transitive inlining. `outer` forwards its own
        // abstract quotation parameter to `inner`, so splicing `outer` into
        // `main` must in turn splice `inner` -- two levels, outermost-first.
        // The spec names this `map`-over-`each`, but `map`/`fold` cannot be
        // built on `each` inside slice 6a (each's `[ 'T -- ]` element quotation
        // hands neither the array nor the index, so a write-back/accumulator
        // needs either a captured mutable borrow (D3-forbidden) or a row
        // variable in the effect (R28, out of scope)). This two-combinator
        // chain exercises the same load-bearing property the criterion guards:
        // both combinators mint no `IrFunc` and `main` emits no `Instr::Call`.
        // Breaking the transitive splice (the `lower_call` combinator branch,
        // or the checker's abstract-forward accept) leaves an `Instr::Call`
        // for `inner` behind.
        let ir = lower_src(
            ": inner ( i64 [ i64 -- ] -- ) call ;\n\
             : outer ( i64 [ i64 -- ] -- ) inner ;\n\
             : main ( -- ) 7 [ 1 + . ] outer ;\n",
        );
        assert!(
            ir.funcs
                .iter()
                .all(|f| f.name != "inner" && f.name != "outer"),
            "both combinators are inlined and mint no `IrFunc`, got: {:?}",
            ir.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let main = ir
            .funcs
            .iter()
            .find(|f| f.name == "main")
            .expect("`main` is emitted");
        assert!(
            call_symbols(main).is_empty(),
            "transitive inlining leaves no `Instr::Call` in `main`, got: {:?}",
            call_symbols(main)
        );
    }

    #[test]
    fn quotation_type_never_reaches_mangling_or_irtype() {
        // Criterion 2d: R7's `unreachable!` arms are only sound because R7a's
        // audit and R20's lowering filter keep a quotation type away from
        // `ir_type_of` (layout) and `subst_polytype` (mangling). This asserts
        // the arms *are* the guard: each panics on a quotation, so replacing
        // an `unreachable!` with a real mapping (a silent accept) flips the
        // corresponding half of this test from panic to value and fails it.
        use crate::ast::{quotation_type, PolyType};
        let quot = quotation_type(vec![Type::I64], vec![Type::I64]);
        assert!(
            std::panic::catch_unwind(|| ir_type_of(quot)).is_err(),
            "`ir_type_of` on a quotation must hit the R7 `unreachable!` arm"
        );
        let poly_quot = PolyType::Quotation(vec![PolyType::Concrete(Type::I64)], Vec::new());
        let subst = Subst::default();
        assert!(
            std::panic::catch_unwind(|| subst_polytype(&poly_quot, &subst, &[])).is_err(),
            "`subst_polytype` on a quotation must hit the R7 `unreachable!` arm"
        );
    }
}
