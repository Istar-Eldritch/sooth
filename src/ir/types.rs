//! The IR data model: `IrModule`/`IrFunc`, `IrType`, SSA `Value`/`Block`/`Instr`,
//! the quotation signature/layout types, and the backend symbol-name consts.

use super::*;

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
    /// Slice 7a (R1/Q2): the module's distinct quotation signatures, interned
    /// by structural effect equality. The backend emits a `type :Q{n} = { l,
    /// l }` per entry and spells `:Q{n}` for each `IrType::Quotation` naming
    /// that effect; empty for a quotation-value-free module (every module
    /// until a materialization boundary produces one).
    pub quot_sigs: Vec<QuotSigLayout>,
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
        // `IrType` carries only the `EnumId` so it stays `Copy`. `Bool` is
        // `Type::Enum(BOOL_ENUM_ID, "bool")` and flows through this arm like
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Line, BOOL_ENUM_ID};
    use crate::check::check;
    use crate::ir::test_helpers::*;
    use crate::lexer::lex;
    use crate::parser::{parse, parse_line};

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
        // Slice 9 (R1/R2): `Bool` is `Type::Enum(BOOL_ENUM_ID, "bool")`, and
        // flows through the general enum arm above like any other enum --
        // whether its value ends up scalar or a memory aggregate is decided
        // by `EnumLayout::is_scalar`, not by a hard-coded arm here (which has
        // no registry access to consult).
        assert_eq!(ir_type_of(Type::BOOL), IrType::Enum(BOOL_ENUM_ID));
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
            ir_type_of(quotation_type(vec![Type::I64], vec![Type::BOOL])),
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
