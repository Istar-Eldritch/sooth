//! The IR data model: `IrModule`/`IrFunc`, `IrType`, SSA `Value`/`Block`/`Instr`,
//! the quotation signature/layout types, and the backend symbol-name consts.

use super::*;

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

/// P7 slice 3c (R10.3): `subslice`'s own runtime range trap. It does not
/// reuse `OOB_TRAP_SYMBOL` because a sub-range failure has no index to report:
/// its three numbers are the requested start, the requested length, and the
/// length of the view being cut, and reporting them as "index N out of bounds
/// for length M" names quantities the source never wrote.
pub const SUBSLICE_TRAP_SYMBOL: &str = "sooth_subslice_trap";

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
    /// Slice 7a (R1/Q2): the module's distinct quotation signatures, interned
    /// by structural effect equality. The backend emits a `type :Q{n} = { l,
    /// l }` per entry and spells `:Q{n}` for each `IrType::Quotation` naming
    /// that effect; empty for a quotation-value-free module (every module
    /// until a materialization boundary produces one).
    pub quot_sigs: Vec<QuotSigLayout>,
    /// Phase 7 slice 2 (D1): the module's `static:` storage, one entry per
    /// declaration in source order. The backend lays each one down as a data
    /// symbol in the preamble; `Instr::StaticAddr` names the same symbol.
    /// Empty for a static-free module (every program until this slice).
    pub statics: Vec<StaticData>,
}

/// One `static:`'s emitted storage. Backend-neutral: the slot's byte width
/// and the constant to lay down, never a QBE data class.
#[derive(Debug)]
pub struct StaticData {
    /// The module-mangled static name, which is also its data symbol and the
    /// string an `Instr::StaticAddr` carries.
    pub symbol: String,
    pub size: u32,
    pub init: StaticValue,
}

/// D3: a static's constant initialiser, already reduced from `ast::StaticInit`
/// (an elided one having become its type's zero).
#[derive(Debug)]
pub enum StaticValue {
    /// An integer constant occupying the slot's full width; a `bool` is `0`/`1`.
    Int(i64),
    /// A `str`: the slot holds the address of this content's `{ptr, len}`
    /// descriptor, interned in the same literal pool as an `Instr::StrLit`.
    Str(String),
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
    /// Slice 7a (R1/Q2): a code handle, the identity of a function symbol,
    /// distinct from a data `Ptr`. Produced only by `Instr::FuncAddr` and
    /// consumed only by `Instr::CallIndirect` (as the callee) or by an
    /// ordinary aggregate store/load of a quotation's `code` slot: no
    /// arithmetic, no dereference, no cast to/from `Ptr` or an integer. On QBE
    /// it classifies identically to `Ptr` (`l` in a register, `l` in an ABI
    /// position), so a future table-based backend (WASM) is free to realize it
    /// as a table index instead of an address, exactly the opacity `Ptr`
    /// already keeps for data pointers.
    Code,
    /// Slice 7a (R1/Q2): a quotation value, a pointer to a fixed two-slot
    /// aggregate `{ code: Code@0, env: Ptr@WORD_WIDTH }` (`quotation_layout`).
    /// `QuotSigId` carries the declared effect directly, so `IrType` stays
    /// `Copy` and two structurally different effects are distinct `IrType`s,
    /// with no interning table threaded through `ir_type_of` (the
    /// `Type::Quotation` side made the same deliberate choice, ast.rs).
    /// Spelled `:Q{id}` in ABI positions and `l` in a register, like
    /// `Struct`/`Enum`/`Array`.
    Quotation(QuotSigId),
    /// P7.S3h: an *owning* quotation value. Byte-for-byte the same two-slot
    /// `{ code, env }` aggregate `IrType::Quotation` is, and it shares the
    /// `:Q{n}` signature symbol, so nothing about its representation differs.
    /// The variant exists because lowering has exactly one decision to make
    /// off it and no other channel to make it from: a materialization
    /// boundary reads the *declared* `IrType` to decide what to build, and an
    /// owning closure's env is a heap block its body copies out and frees,
    /// where a plain closure's is an inline word or a frame bundle. Erasing
    /// the two together would silently build a frame env for a closure that
    /// outlives the frame.
    OwningQuotation(QuotSigId),
    /// P7 slice 3c (R2.1): a borrowed view `Slice[T]`, keyed by the `SliceId`
    /// of its interned `(element, mutable)` shape so `IrType` stays `Copy`,
    /// like `Struct`/`Enum`/`Array`. At runtime a genuine **two-word
    /// aggregate** `{ ptr, len }` (`slice_layout`): an opaque element pointer
    /// plus a target-width length computed at runtime. `Str`'s single-word
    /// shape deliberately does not carry -- a `str` is the address of a
    /// *statically built* descriptor, and a slice has no static descriptor to
    /// point at -- so a slice is spelled as an aggregate in ABI positions and
    /// `l` (a pointer to its storage) in a register, and is blit-copied rather
    /// than scalar-loaded.
    Slice(SliceId),
}

/// Slice 7a (R1/Q2): the identity of a quotation's declared effect. A `Copy`
/// handle carrying the leaked `&'static QuotEffect`, so two structurally-equal
/// effects compare equal through the reference (the value equality unification
/// already relies on for `Type::Quotation`). It is not an index into a table:
/// carrying the effect keeps `ir_type_of` pure and threads no mutable
/// interning table, matching the no-table choice on the `Type` side. The
/// backend assigns each distinct effect a `:Q{n}` aggregate symbol from
/// `IrModule::quot_sigs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotSigId(pub &'static QuotEffect);

/// Slice 7a (R1/Q2): one entry of the module-level quotation signature table
/// (`IrModule::quot_sigs`), interned by structural effect equality. The
/// backend emits a `type :Q{n} = { l, l }` per entry and spells `:Q{n}` for
/// each `IrType::Quotation` naming this effect; the effect is what a
/// materialization boundary reads to mint the callee `IrFunc`'s signature.
#[derive(Debug, Clone)]
pub struct QuotSigLayout {
    pub effect: &'static QuotEffect,
}

/// Slice 7a (R2/D5): the fixed two-slot layout every quotation value shares,
/// every figure word-width-derived (backend-neutral invariant): `code` at
/// offset 0, `env` at offset `word_width`, size `2 * word_width`, align
/// `word_width`. The `env` slot is always the null pointer in 7a (7b fills
/// it); it is not elided, so widening to a capturing closure stays additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotLayout {
    pub code_offset: u32,
    pub env_offset: u32,
    pub size: u32,
    pub align: u32,
}

pub fn quotation_layout(word_width: u32) -> QuotLayout {
    QuotLayout {
        code_offset: 0,
        env_offset: word_width,
        size: 2 * word_width,
        align: word_width,
    }
}

/// P7 slice 3c (R2.1): the fixed two-slot layout every slice value shares,
/// every figure word-width-derived (backend-neutral invariant): the element
/// `ptr` at offset 0, the runtime `len` at offset `word_width`, size `2 *
/// word_width`, align `word_width`. Shaped like `QuotLayout` rather than like
/// `str`'s static descriptor: both slots are live, runtime-computed values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliceLayout {
    pub ptr_offset: u32,
    pub len_offset: u32,
    pub size: u32,
    pub align: u32,
}

pub fn slice_layout(word_width: u32) -> SliceLayout {
    SliceLayout {
        ptr_offset: 0,
        len_offset: word_width,
        size: 2 * word_width,
        align: word_width,
    }
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
        // The layout lives in the module's `StructLayout` registry; the
        // `IrType` carries only the `StructId` so it stays `Copy`.
        Type::Struct(id, _) => IrType::Struct(id),
        // The tagged layout lives in the module's `EnumLayout` registry; the
        // `IrType` carries only the `EnumId` so it stays `Copy`. `bool` is
        // `core::bool`'s enum and flows through this arm like
        // any other enum (Slice 9, D-A): whether its *value* ends up
        // register-resident or a memory aggregate is the general
        // zero-payload-enum rule in `EnumLayout`/`ensure_enum`, not a
        // hard-coded arm here (which cannot consult the registry anyway).
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
        // P7 slice 3c (R1.3/R2.1): a slice is two words (an element pointer
        // and a runtime length), so it is emphatically *not* `IrType::Ptr` --
        // every existing `Ptr` site assumes one word, which is why the type is
        // its own variant rather than a fat `Type::Ref`. The element shape a
        // lowering needs stays behind the `SliceId`, so `IrType` remains
        // `Copy`; the mutability is frontend-only, exactly as it is for a
        // `Type::Ref` mapping to `Ptr`.
        Type::Slice(id, _, _) => IrType::Slice(id),
        Type::Usize => IrType::Usize,
        Type::Isize => IrType::Isize,
        Type::Str => IrType::Str,
        Type::Cstr => IrType::Cstr,
        // Slice 7a (R3): a quotation now has a runtime `(code, env)` value.
        // `QuotSigId` carries the effect directly, so this arm stays pure --
        // no interning table threaded (the `Type::Quotation` side chose the
        // same, ast.rs). The backend assigns each distinct effect a `:Q{n}`
        // symbol from `IrModule::quot_sigs`.
        Type::Quotation(eff) => IrType::Quotation(QuotSigId(eff)),
        // Slice 10a (R1): a `~` cannot be materialized, so it never reaches the
        // backend. Every materialization boundary rejects it by type inequality
        // upstream (it is not `Type::Quotation`), and it is never a field,
        // output, referent, or captured value, so lowering cannot construct a
        // value of this type. Reaching here is a checker bug, not a legal input.
        Type::InlineQuotation(_) => {
            unreachable!(
                "a `~` inline quotation never reaches the backend (it cannot be materialized)"
            )
        }
        // P7.S3h (phase 2): an `owning` quotation type-checks but has no
        // representation yet. Unlike the `~` arm above this is not permanent:
        // phase 3 gives it a real `IrType`. Until then
        // `reject_owning_quotation_declarations` rejects every declaration
        // position lowering can read, which is what keeps this arm unreached
        // -- a declared `owning` parameter reaches here through signature
        // lowering without ever crossing a materialization boundary.
        // P7.S3h (phase 3): an owning quotation is represented exactly as a
        // plain one -- the same two-word `(code, env)` aggregate under the
        // same `:Q{n}` symbol. The distinct `IrType` carries only the env
        // *storage* decision into lowering (a heap block the body frees), not
        // a distinct shape.
        Type::OwningQuotation(eff) => IrType::OwningQuotation(QuotSigId(eff)),
        // Phase 6 slice 3 (R6): a variant is represented identically to its
        // enum at the backend -- only the frontend distinguishes them, so a
        // `Type::Variant` erases to the same `IrType::Enum(id)` its parent
        // enum already gets. Load-bearing since decision 6: an eliminator's
        // reference-mode arm can declare `&Shape.Circle`, which interns a real
        // `RefDecl` and forces `ir_type_of` over it at build time.
        Type::Variant(id, _, _) => IrType::Enum(id),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value(pub u32);

/// R12: an index into `FuncBuilder::quot_defs`, the per-function table of
/// quotation-literal bodies. A quotation lowers to a phantom `Value` (no
/// defining `Instr`) mapped to its `QuotId`; `call`/`times` splice the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct QuotId(pub(super) usize);

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
    /// Slice 7a (R4/Q3): the address of a (materialized) function symbol as an
    /// `IrType::Code` value (a distinct opaque handle, not `Ptr`). Emitted at
    /// a materialization boundary to fill a quotation's `code` slot; realized
    /// on QBE as `%dst =l copy $sym`.
    FuncAddr(Value, String),
    /// Phase 7 slice 2 (R1): the address of a module static's data symbol as an
    /// `IrType::Ptr`. The referent shape is not in the type (references never
    /// carry it); `push_reference` records it, so `@`/`!`/`+!` through the
    /// resulting borrow dispatch exactly as they do for a struct-field place.
    StaticAddr(Value, String),
    /// Slice 7a (R4/Q3): an indirect call through a code-handle `Value` (the
    /// quotation's `code` slot, already `Load`ed, `IrType::Code`). Mirrors
    /// `Call` but the callee is a value, not a symbol. `env` is not passed in
    /// 7a (a non-capturing callee has no env parameter); 7b adds the env
    /// argument here.
    CallIndirect(Option<Value>, Value, Vec<Value>),
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
    /// Slice 10c (R-P3-2): `dst: Int{32, unsigned} = src`'s enum discriminant,
    /// for a scalar (all-variants-payload-free) enum only. Such a value *is*
    /// its discriminant in a 32-bit register, so this reads no memory and
    /// converts no width: it exists solely to give the discriminant an integer
    /// `IrType`, which the source value (an `IrType::Enum`) cannot carry. The
    /// backend relabels the register; the machine code is unchanged.
    Tag(Value, Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    /// Float division (`div`); present only for float operands (there is no
    /// integer `div`, checker-guaranteed, R16).
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

/// Declared signature of a user word or `extern:` declaration. The build path
/// derives this from declared slot types; the REPL derives it from the
/// checker's typed env. A `None` `ret_ty` (e.g. a word with no output) is
/// treated as `IrType::Int` by callers.
#[derive(Debug, Clone)]
pub struct Arity {
    pub in_arity: usize,
    pub out_arity: usize,
    pub ret_ty: Option<IrType>,
    /// The callee's ordinary `[ ... ]` quotation parameters (R-D1). A call
    /// site materializes the phantom argument at each of these slots before it
    /// enters `Instr::Call`; the name-keyed env is the only thing a call site
    /// holds about its callee, so the shape has to travel here rather than be
    /// re-read from the callee's `WordDef` (which lowering never has, and the
    /// REPL has no module to read one from).
    pub quot_inputs: Vec<(usize, IrType)>,
}

/// The ordinary `[ ... ]` quotation slots of a declared input row, as
/// `(index, IrType::Quotation)` pairs. A `~[ ... ]` slot never appears: a word
/// declaring one is a combinator, spliced at every call site and absent from
/// every lowering env.
pub fn quot_input_slots(inputs: impl IntoIterator<Item = Type>) -> Vec<(usize, IrType)> {
    inputs
        .into_iter()
        .enumerate()
        .filter(|(_, ty)| matches!(ty, Type::Quotation(_) | Type::OwningQuotation(_)))
        .map(|(i, ty)| (i, ir_type_of(ty)))
        .collect()
}

/// Maps a called user-word name to the symbol it is emitted/linked as. The build
/// path uses identity; the REPL supplies generation-mangled symbols so a unit
/// links against the words it was compiled against.
pub type Resolver<'a> = &'a dyn Fn(&str) -> String;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::test_helpers::*;
    use crate::lexer::lex;

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

    /// P7 slice 3c (R1.3/R2.1): the `Type::Slice` -> `IrType::Slice` mapping,
    /// carrying the `SliceId` through, and the two-word layout it lowers to.
    /// The failure this guards is the tempting one: mapping a slice to a
    /// one-word `Ptr`/`Str`, which compiles everywhere and truncates the
    /// length.
    #[test]
    fn ir_type_of_slice_is_a_two_word_aggregate_not_a_pointer() {
        let mut slices = Vec::new();
        let shared = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        let mutable = crate::ast::intern_slice_type(&mut slices, Type::F64, true);
        let ir = ir_type_of(shared);
        assert!(matches!(ir, IrType::Slice(_)), "got {ir:?}");
        assert_ne!(ir, IrType::Ptr);
        assert_ne!(ir, IrType::Str);
        // The element/mutability distinction survives into the IR through the
        // `SliceId`, so two different views are two different `IrType`s.
        assert_ne!(ir, ir_type_of(mutable));

        // Two words, the length at the second: `{ ptr, len }`, every figure
        // derived from the word-width parameter rather than a literal.
        let l = slice_layout(WORD_WIDTH);
        assert_eq!((l.ptr_offset, l.len_offset, l.size, l.align), (0, 8, 16, 8));
        let narrow = slice_layout(4);
        assert_eq!(
            (narrow.len_offset, narrow.size, narrow.align),
            (4, 8, 4),
            "the layout derives from the word width, not a hardcoded 8"
        );
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
        // P7 slice 3i: `bool` is `core::bool`'s enum at whatever registry
        // position the build resolved, and flows through the general enum arm
        // above like any other enum -- whether its value ends up scalar or a
        // memory aggregate is decided by `EnumLayout::is_scalar`, not by a
        // hard-coded arm here (which has no registry access to consult).
        let bool_ty = crate::ast::resolve_bool_type(&crate::test_support::core_bool_enums())
            .expect("`core::bool` declares `Bool`");
        let Type::Enum(bool_id, _) = bool_ty else {
            panic!("`Bool` is an enum");
        };
        assert_eq!(ir_type_of(bool_ty), IrType::Enum(bool_id));
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
    fn ir_type_of_variant_erases_to_its_parent_enum() {
        // Phase 6 slice 3 (R6): a `Type::Variant` erases to the same
        // `IrType::Enum(id)` a plain `Type::Enum(id, _)` of the same id does
        // -- a positive equality assertion, not merely "does not panic".
        let id = EnumId::from_index(0);
        assert_eq!(
            ir_type_of(Type::Variant(id, 0, "Shape.Circle")),
            IrType::Enum(id)
        );
        assert_eq!(
            ir_type_of(Type::Variant(id, 0, "Shape.Circle")),
            ir_type_of(Type::Enum(id, "Shape"))
        );
    }

    #[test]
    fn ir_type_of_struct_maps_to_struct_irtype() {
        let tokens = lex("type: Vec2 x i64 y i64 ;").unwrap();
        let module = crate::test_support::parse_with_core(&tokens).unwrap();
        let ty = module.resolve_type_name("Vec2").unwrap();
        assert!(matches!(ir_type_of(ty), IrType::Struct(_)));
    }

    #[test]
    fn ir_type_of_enum_maps_to_enum_irtype() {
        let tokens = lex("type: Shape | Circle r f64 | Rect w f64 h f64 ;").unwrap();
        let module = crate::test_support::parse_with_core(&tokens).unwrap();
        let ty = module.resolve_type_name("Shape").unwrap();
        assert!(matches!(ir_type_of(ty), IrType::Enum(_)));
    }

    #[test]
    fn ir_type_of_quotation_is_two_slot_aggregate() {
        // T-irtype (R2/R3): a quotation type maps to a runtime value ---
        // `IrType::Quotation` naming its effect --- with a fixed two-slot
        // `{ code@0, env@WORD_WIDTH }` layout: size `2*WORD_WIDTH`, align
        // `WORD_WIDTH`, every figure word-width-derived, not a hardcoded
        // 16/8. The carried effect gives value equality, so two structurally
        // equal effects share one `IrType`.
        use crate::ast::quotation_type;
        let ir = ir_type_of(quotation_type(vec![Type::I64], vec![Type::I64]));
        assert!(
            matches!(ir, IrType::Quotation(_)),
            "a quotation type maps to `IrType::Quotation`, got {ir:?}"
        );
        assert_eq!(
            ir,
            ir_type_of(quotation_type(vec![Type::I64], vec![Type::I64])),
            "structurally equal effects are one `IrType`"
        );
        assert_ne!(
            ir,
            ir_type_of(quotation_type(vec![Type::I64], vec![Type::U32])),
            "structurally different effects are distinct `IrType`s"
        );
        let layout = quotation_layout(WORD_WIDTH);
        assert_eq!(layout.code_offset, 0, "code slot at offset 0");
        assert_eq!(
            layout.env_offset, WORD_WIDTH,
            "env slot at offset WORD_WIDTH"
        );
        assert_eq!(layout.size, 2 * WORD_WIDTH, "two word-width slots");
        assert_eq!(layout.align, WORD_WIDTH);
    }
}
