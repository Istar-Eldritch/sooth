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
    ArrayDecl, ArrayId, Clause, EnumDecl, EnumId, Module, StructDecl, StructId, Term, TermKind,
    Type, WordBody, WordDef,
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
    /// Opaque handle (backend-neutral-IR invariant): a native pointer under QBE,
    /// a linear-memory offset under a future WASM lowering. Used by the line
    /// wrapper's `%stack` parameter.
    Ptr,
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
        Type::Usize => IrType::Usize,
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
}

/// The IR's view of a program's structs: the per-`StructId` layout registry and
/// the generated-word name map (`S`/`S>`/`S>fi`/`S<fi` → `StructWord`). Built
/// once from the module and threaded into lowering; empty for a struct-free
/// program (the scalar paths never consult it).
#[derive(Debug, Default)]
pub struct Structs {
    pub layouts: Vec<StructLayout>,
    pub words: HashMap<String, StructWord>,
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
}

/// The IR's view of a program's arrays: the per-`ArrayId` layout registry.
/// Unlike `Structs`/`Enums` there is no generated-word name map: the array
/// words (`fill`/`get`/`set`/`len`) are generic and dispatched by name +
/// operand type in `lower_call`, not by a per-array symbol. Empty for an
/// array-free program.
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
        IrType::Ptr => 8,
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
        _ => 8,
    }
}

impl Structs {
    /// Build just the struct registry (no enums). A thin wrapper over
    /// `build_registries` for struct-only callers; a struct with an enum field
    /// needs the full `build_registries` (its enums must be present to size
    /// the field, D9).
    pub fn from_structs(structs: &[StructDecl]) -> Structs {
        build_registries(structs, &[], &[]).0
    }
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
) -> (Structs, Enums, Arrays) {
    build_registries_ww(structs, enums, arrays, WORD_WIDTH)
}

/// `build_registries` with an explicit target word width (R15). Production
/// callers use `build_registries`; criterion 2's structural test flips
/// `word_width` here to prove a `usize`-embedding aggregate resizes with the
/// parameter (no stray literal `8`).
pub fn build_registries_ww(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    word_width: u32,
) -> (Structs, Enums, Arrays) {
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
    for (idx, decl) in structs.iter().enumerate() {
        let id = StructId::from_index(idx);
        swords.insert(decl.name.clone(), StructWord::Construct(id));
        swords.insert(format!("{}>", decl.name), StructWord::Destructure(id));
        for (fi, (fname, _)) in decl.fields.iter().enumerate() {
            swords.insert(format!("{}>{}", decl.name, fname), StructWord::Get(id, fi));
            swords.insert(format!("{}<{}", decl.name, fname), StructWord::Set(id, fi));
        }
    }

    let mut ewords = HashMap::new();
    for (idx, decl) in enums.iter().enumerate() {
        let id = EnumId::from_index(idx);
        for (vi, variant) in decl.variants.iter().enumerate() {
            ewords.insert(variant.name.clone(), EnumWord::Construct(id, vi));
        }
    }

    (
        Structs {
            layouts: struct_layouts,
            words: swords,
        },
        Enums {
            layouts: enum_layouts,
            words: ewords,
        },
        Arrays {
            layouts: array_layouts,
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
        self.struct_memo[idx] = Some(StructLayout {
            name: structs[idx].name_static,
            size,
            align,
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
        self.enum_memo[idx] = Some(EnumLayout {
            name: enums[idx].name_static,
            tag_offset: 0,
            tag_ty,
            payload_offset,
            size,
            align,
            variants,
        });
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
        self.array_memo[idx] = Some(ArrayLayout {
            name: self.arrays[idx].name_static,
            elem,
            count,
            stride,
            size: stride * count,
            align: elem_align.max(1),
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value(pub u32);

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

/// Declared signature of a user word: (input count, output count, output
/// `IrType` if any). The build path derives this from declared slot types; the
/// REPL derives it from the checker's typed env. A `None` output type (e.g. a
/// word with no output) is treated as `IrType::Int` by callers.
pub type Arity = (usize, usize, Option<IrType>);

/// Maps a called user-word name to the symbol it is emitted/linked as. The build
/// path uses identity; the REPL supplies generation-mangled symbols so a unit
/// links against the words it was compiled against.
pub type Resolver<'a> = &'a dyn Fn(&str) -> String;

pub fn lower(module: &Module) -> Result<IrModule, String> {
    let (structs, enums, arrays) = build_registries(&module.structs, &module.enums, &module.arrays);
    let env: HashMap<String, Arity> = module
        .words
        .iter()
        .map(|w| {
            let ret_ty = w.effect.outputs.first().map(|slot| ir_type_of(slot.ty));
            (
                w.name.clone(),
                (w.effect.inputs.len(), w.effect.outputs.len(), ret_ty),
            )
        })
        .collect();
    let resolve = |name: &str| name.to_string();

    let funcs = module
        .words
        .iter()
        .map(|w| lower_word(w, &env, &resolve, &structs, &enums, &arrays))
        .collect();

    Ok(IrModule {
        funcs,
        structs: structs.layouts,
        enums: enums.layouts,
        arrays: arrays.layouts,
    })
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
#[allow(clippy::too_many_arguments)] // one wrapper's marshalling inputs; a bundle would obscure them
pub fn lower_line(
    seq: u64,
    terms: &[Term],
    entry_depth: usize,
    entry_types: &[Type],
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    structs: &Structs,
    enums: &Enums,
    arrays: &Arrays,
) -> (IrFunc, usize, usize) {
    debug_assert_eq!(entry_types.len(), entry_depth);
    // A REPL line has no word name to self-tail-call against.
    let mut b = FuncBuilder::new(env, resolve, structs, enums, arrays, String::new());

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
            _ => {
                let v = b.fresh_value(IrType::I64);
                b.push_instr(Instr::Load(v, ptr));
                stack.push(v);
            }
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

/// Lower a single word body against an external env/resolver. The REPL uses
/// this directly (renaming the returned `IrFunc.name` to a mangled symbol)
/// so a definition compiles against previously-loaded words.
pub(crate) fn lower_word(
    word: &WordDef,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    structs: &Structs,
    enums: &Enums,
    arrays: &Arrays,
) -> IrFunc {
    let params: Vec<IrType> = word
        .effect
        .inputs
        .iter()
        .map(|s| ir_type_of(s.ty))
        .collect();
    let ret = word.effect.outputs.first().map(|s| ir_type_of(s.ty));

    let mut b = FuncBuilder::new(env, resolve, structs, enums, arrays, word.name.clone());

    // Params occupy the first N value ids; leftmost input is deepest.
    // (b.cur_word_name is set above for R7's self-tail-call detection.)
    let params_values: Vec<Value> = params.iter().map(|ty| b.fresh_value(*ty)).collect();

    // R6: a self-tail-recursive word lowers to a loop. The entry block binds
    // the params and jumps to a header carrying one phi per loop-carried slot;
    // the body reads the phi outputs so each iteration rebinds them. A word
    // with no tail self-call lowers exactly as before (no header, no phi).
    let self_tail = crate::check::has_self_tail_call(word);
    let entry_values = if self_tail {
        b.begin_loop(&params_values)
    } else {
        params_values
    };

    match &word.body {
        WordBody::Terms { locals, terms } => {
            let mut stack = entry_values;
            // Bind `| ... |` locals: pop the top N (D6: from the header phi
            // outputs when looping), leftmost local = deepest.
            let take = locals.len();
            let bound = stack.split_off(stack.len() - take);
            for (name, value) in locals.iter().zip(bound) {
                b.locals.insert(name.clone(), value);
            }
            b.stack = stack;
            b.lower_terms(terms, self_tail);
        }
        WordBody::Clauses(clauses) => b.lower_clauses(clauses, &entry_values),
    }

    // R8: back-patch the header phis with the collected back-edge operands.
    if self_tail {
        b.finalize_loop();
    }

    // The fall-through (base-case) block returns; a body that ended entirely in
    // back-edges is already terminated and needs no Ret.
    if !b.terminated {
        let result = if ret.is_some() { b.stack.pop() } else { None };
        b.seal_block(Terminator::Ret(result));
    }

    IrFunc {
        name: word.name.clone(),
        params,
        ret,
        blocks: b.blocks,
        value_types: b.value_types,
    }
}

struct FuncBuilder<'a> {
    env: &'a HashMap<String, Arity>,
    resolve: Resolver<'a>,
    structs: &'a Structs,
    enums: &'a Enums,
    arrays: &'a Arrays,
    /// Name of the word currently being lowered, used by the tail-call ->
    /// back-edge transform (R7) to recognize a self-call.
    cur_word_name: String,
    /// The loop header block (R6), `Some` iff this word is self-tail-recursive
    /// and is being lowered as a loop. Tail self-calls back-edge to it (R7).
    header: Option<BlockId>,
    /// One phi output value per loop-carried slot (input arity many), in slot
    /// order (R6). The body reads these, not the raw params.
    header_phis: Vec<Value>,
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
    /// static alloc site, reused (overwritten) every iteration instead.
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
    locals: HashMap<String, Value>,
    value_types: Vec<IrType>,
    /// Compile-time integer value of each `Const`-defined `Value`, for the
    /// `fill` count (M1: the count is a checker-guaranteed literal) and the
    /// element/array-shape lookup. A shuffle reuses a value id, so a duped
    /// literal keeps its recorded value.
    const_vals: HashMap<Value, i64>,
}

impl<'a> FuncBuilder<'a> {
    fn new(
        env: &'a HashMap<String, Arity>,
        resolve: Resolver<'a>,
        structs: &'a Structs,
        enums: &'a Enums,
        arrays: &'a Arrays,
        cur_word_name: String,
    ) -> Self {
        FuncBuilder {
            env,
            resolve,
            structs,
            enums,
            arrays,
            cur_word_name,
            header: None,
            header_phis: Vec::new(),
            back_edges: Vec::new(),
            entry_block: None,
            terminated: false,
            blocks: Vec::new(),
            cur_id: BlockId(0),
            cur_instrs: Vec::new(),
            next_value: 0,
            next_block: 1, // block 0 is the entry, already current
            stack: Vec::new(),
            locals: HashMap::new(),
            value_types: Vec::new(),
            const_vals: HashMap::new(),
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

    /// Emit an `Alloc` into the current block, unless looping (`entry_block`
    /// is `Some`), in which case it goes into the entry block instead: see
    /// `entry_block`'s doc comment for why a loop body must never alloc.
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

    /// R6: open the loop shape. The current (entry) block binds `params`,
    /// jumps to a fresh header, and the header carries one phi per carried
    /// slot, each seeded with the entry arm `(entry, param)`. Returns the phi
    /// outputs, which the body reads instead of the raw params. An input arity
    /// of 0 yields a header with zero phis (just a back-edge target), handled
    /// without special-casing.
    fn begin_loop(&mut self, params: &[Value]) -> Vec<Value> {
        let entry = self.cur_id;
        let header = self.fresh_block();
        self.seal_block(Terminator::Jmp(header));
        self.start_block(header);
        self.header = Some(header);
        self.entry_block = Some(entry);
        let mut outs = Vec::with_capacity(params.len());
        for &p in params {
            let out = self.fresh_value(self.value_type(p));
            self.push_instr(Instr::Phi(out, vec![(entry, p)]));
            self.header_phis.push(out);
            outs.push(out);
        }
        outs
    }

    /// R8: after the body lowers, append each collected back-edge's per-slot
    /// operand to the matching header phi. The back-edge arms cannot be known
    /// when the header is emitted (they are produced on the back-edges), so
    /// they are finalized here in a second step.
    fn finalize_loop(&mut self) {
        let header = self.header.expect("finalize_loop: loop mode");
        let phis = mem::take(&mut self.header_phis);
        let back_edges = mem::take(&mut self.back_edges);
        let block = self
            .blocks
            .iter_mut()
            .find(|b| b.id == header)
            .expect("header block");
        for instr in &mut block.instrs {
            if let Instr::Phi(v, arms) = instr {
                if let Some(slot) = phis.iter().position(|&p| p == *v) {
                    for (pred, vals) in &back_edges {
                        arms.push((*pred, vals[slot]));
                    }
                }
            }
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
            TermKind::Call(name) => self.lower_call(name, term.span.line, tail),
            TermKind::If {
                then_branch,
                else_branch,
            } => self.lower_if(then_branch, else_branch, tail),
        }
    }

    fn lower_call(&mut self, name: &str, line: u32, tail: bool) {
        if let Some(&value) = self.locals.get(name) {
            self.stack.push(value); // i64 is Copy; reuse the value id.
            return;
        }
        match name {
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
                self.stack.pop().expect("drop: non-empty stack");
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
            "." => {
                let v = self.stack.pop().expect("print: value");
                self.push_instr(Instr::Print(v));
            }
            "fill" | "get" | "set" | "len" => self.lower_array_word(name, line),
            _ => {
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
                // A generated struct word (`S`/`S>`/`S>fi`/`S<fi`) lowers to
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
                let ret = if out_arity == 1 {
                    Some(self.fresh_value(ret_ty.unwrap_or(IrType::I64)))
                } else {
                    None
                };
                let sym = (self.resolve)(name);
                self.push_instr(Instr::Call(ret, sym, args));
                if let Some(v) = ret {
                    self.stack.push(v);
                }
            }
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
    /// scalar's width) — the blit length for a `fill`/`set` aggregate element.
    fn value_size(&self, ty: IrType) -> u32 {
        match ty {
            IrType::Struct(id) => self.structs.layouts[id.index()].size,
            IrType::Enum(id) => self.enums.layouts[id.index()].size,
            IrType::Array(id) => self.arrays.layouts[id.index()].size,
            other => scalar_size_align(other).0,
        }
    }

    /// `dst = base + index*stride`, typed `ty` (R17). Scalar element paths pass
    /// `Ptr` (a `FieldLoad`/`FieldStore` follows); an aggregate element path
    /// passes the element's own aggregate type so the address doubles as the
    /// element value.
    fn elem_addr(&mut self, base: Value, index: Value, stride: u32, ty: IrType) -> Value {
        let dst = self.fresh_value(ty);
        self.push_instr(Instr::ElemAddr(dst, base, index, stride as i64));
        dst
    }

    /// Store `val` (of element type `elem`) at element place `fptr`: a
    /// width-exact scalar `FieldStore`, or an aggregate `Blit` of the whole
    /// element. Shared by `fill`'s unrolled stores and `set`'s single store.
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

    /// Lower an array word inline (R18): `fill` = alloc + N unrolled stores
    /// (M6); `get` = element-addr + load, non-consuming (R12); `set` = alloc +
    /// whole-array blit + element-addr + store, yielding a fresh array; `len`
    /// = a constant `usize` from the layout, non-consuming.
    fn lower_array_word(&mut self, name: &str, line: u32) {
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
            "get" => {
                let index = self.stack.pop().expect("get: index");
                // Non-consuming (R12/M4): the array stays on the stack.
                let array = *self.stack.last().expect("get: array");
                let id = match self.value_type(array) {
                    IrType::Array(id) => id,
                    _ => unreachable!("checked: get's second operand is an array"),
                };
                let (stride, elem, count) = self.array_parts(id);
                self.bounds_check(index, count, line);
                match elem {
                    IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                        // The element address is itself the aggregate value.
                        let v = self.elem_addr(array, index, stride, elem);
                        self.stack.push(v);
                    }
                    _ => {
                        let addr = self.elem_addr(array, index, stride, IrType::Ptr);
                        let v = self.fresh_value(elem);
                        self.push_instr(Instr::FieldLoad(v, addr));
                        self.stack.push(v);
                    }
                }
            }
            "set" => {
                let val = self.stack.pop().expect("set: value");
                let index = self.stack.pop().expect("set: index");
                let array = self.stack.pop().expect("set: array");
                let id = match self.value_type(array) {
                    IrType::Array(id) => id,
                    _ => unreachable!("checked: set's first operand is an array"),
                };
                let (stride, elem, count) = self.array_parts(id);
                self.bounds_check(index, count, line);
                let size = self.arrays.layouts[id.index()].size;
                let dst = self.alloc_array(id);
                if size > 0 {
                    self.push_instr(Instr::Blit(array, dst, size));
                }
                let addr = self.elem_addr(dst, index, stride, IrType::Ptr);
                self.store_elem(addr, val, elem);
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
            _ => unreachable!("lower_array_word only handles fill/get/set/len"),
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

    /// Load field `field` at `fptr` onto the stack: a width-exact scalar load,
    /// or the interior pointer as a nested struct/enum value.
    fn load_field_onto_stack(&mut self, base: Value, field: FieldLayout) {
        let v = match field.ty {
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                self.field_aggregate_value(base, field.offset, field.ty)
            }
            _ => {
                let fptr = self.field_ptr(base, field.offset);
                let v = self.fresh_value(field.ty);
                self.push_instr(Instr::FieldLoad(v, fptr));
                v
            }
        };
        self.stack.push(v);
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
                let field = self.structs.layouts[id.index()].fields[fi];
                self.load_field_onto_stack(s, field);
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
        self.seal_block(Terminator::Jnz(test, then_id, else_id));

        self.start_block(then_id);
        self.terminated = false;
        self.stack = post_pop.clone();
        self.lower_terms(then_branch, tail);
        let then_arm = self.seal_arm(join_id);

        self.start_block(else_id);
        self.terminated = false;
        self.stack = post_pop;
        self.lower_terms(else_branch, tail);
        let else_arm = self.seal_arm(join_id);

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
    fn lower_clauses(&mut self, clauses: &[Clause], params: &[Value]) {
        // A clause word is self-tail-recursive iff a header was opened (R6);
        // its clause bodies then carry tail position (D7).
        let tail = self.header.is_some();
        let scrutinee = *params.last().expect("clause word has a scrutinee input");
        let stack_below: Vec<Value> = params[..params.len() - 1].to_vec();
        let scrut_id = match self.value_type(scrutinee) {
            IrType::Enum(id) => id,
            _ => unreachable!("checked: a clause word's top input is an enum"),
        };
        let (tag_ty, tag_offset, payload_offset, n) = {
            let l = &self.enums.layouts[scrut_id.index()];
            (l.tag_ty, l.tag_offset, l.payload_offset, l.variants.len())
        };

        // Map each variant index to the clause handling it (checker-guaranteed
        // exact coverage), so dispatch on tag == variant_index lands correctly
        // regardless of clause source order.
        let clause_ids: Vec<BlockId> = (0..n).map(|_| self.fresh_block()).collect();
        let join_id = self.fresh_block();
        let mut clause_for_variant: Vec<Option<&Clause>> = vec![None; n];
        for clause in clauses {
            let EnumWord::Construct(_, vi) = self.enums.words[&clause.variant];
            clause_for_variant[vi] = Some(clause);
        }

        if n == 1 {
            self.seal_block(Terminator::Jmp(clause_ids[0]));
        } else {
            // The discriminant is a temp used only for the compare-chain, never
            // pushed onto the virtual stack. A newtype (n == 1) skips it.
            let tag = self.fresh_value(tag_ty);
            let tag_ptr = self.field_ptr(scrutinee, tag_offset);
            self.push_instr(Instr::FieldLoad(tag, tag_ptr));
            for vi in 0..n - 1 {
                let idx_val = self.fresh_value(tag_ty);
                self.push_instr(Instr::Const(idx_val, vi as i64));
                let c = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(c, CmpOp::Eq, tag, idx_val));
                // The last compare's false edge falls straight through to the
                // final variant; no default/trap block (exhaustive coverage).
                let false_target = if vi == n - 2 {
                    clause_ids[n - 1]
                } else {
                    self.fresh_block()
                };
                self.seal_block(Terminator::Jnz(c, clause_ids[vi], false_target));
                if vi < n - 2 {
                    self.start_block(false_target);
                }
            }
        }

        let mut clause_ends: Vec<(BlockId, Vec<Value>)> = Vec::with_capacity(n);
        for vi in 0..n {
            let clause = clause_for_variant[vi].expect("checked: exhaustive coverage");
            self.start_block(clause_ids[vi]);
            self.locals = HashMap::new();
            self.stack = stack_below.clone();
            // Push the variant's payload first-deepest, loading each field from
            // `payload_offset + field.offset`.
            let fields = self.enums.layouts[scrut_id.index()].variants[vi]
                .fields
                .clone();
            for field in &fields {
                let adjusted = FieldLayout {
                    offset: payload_offset + field.offset,
                    ..*field
                };
                self.load_field_onto_stack(scrutinee, adjusted);
            }
            // Bind clause-body `| names |` locals (top N, leftmost deepest).
            let take = clause.locals.len();
            let bound = self.stack.split_off(self.stack.len() - take);
            for (name, value) in clause.locals.iter().zip(bound) {
                self.locals.insert(name.clone(), value);
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
        build_registries(&module.structs, &module.enums, &module.arrays).1
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

    #[test]
    fn func_builder_new_threads_current_word_name() {
        // R5: FuncBuilder carries the word being lowered, set from `word.name`
        // in `lower_word`; the REPL path calls the same `lower_word` (no
        // REPL-specific plumbing), so this covers both callers.
        let env: HashMap<String, Arity> = HashMap::new();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let b = FuncBuilder::new(
            &env,
            resolve,
            &structs,
            &enums,
            &arrays,
            "loop-word".to_string(),
        );
        assert_eq!(b.cur_word_name, "loop-word");
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
            &Structs::default(),
            &Enums::default(),
            &Arrays::default(),
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
            &Structs::default(),
            &Enums::default(),
            &Arrays::default(),
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
        build_registries(&module.structs, &module.enums, &module.arrays).2
    }

    fn module_of(src: &str) -> Module {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        module
    }

    #[test]
    fn word_width_parameter_sizes_usize_not_a_literal_eight() {
        // Criterion 2 (structural): `usize` size/align derives from the word
        // width parameter, not a hardcoded `8`. At the default width it is 8;
        // flipping the parameter to 4 changes the derived size of both a bare
        // `usize` and an aggregate that embeds one, proving no stray literal.
        assert_eq!(scalar_size_align(IrType::Usize), (8, 8));
        assert_eq!(scalar_size_align_ww(IrType::Usize, 8), (8, 8));
        assert_eq!(scalar_size_align_ww(IrType::Usize, 4), (4, 4));

        // A struct with two `usize` fields and an array of `usize`: both resize
        // with the parameter.
        let m = module_of(": w ( [usize 4] -- ) drop ;\ntype: Cursor a usize b usize ;");
        let (s8, _, a8) = build_registries_ww(&m.structs, &m.enums, &m.arrays, 8);
        let (s4, _, a4) = build_registries_ww(&m.structs, &m.enums, &m.arrays, 4);
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
    fn lower_get_is_non_consuming_elem_addr_and_load() {
        // R18/R17: `get` addresses the element (`ElemAddr`) and loads it
        // (`FieldLoad`); it allocs nothing (non-consuming, R12).
        let ir = lower_src(": w ( [i64 4] -- i64 ) 0 get swap drop ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldLoad(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 0);
    }

    #[test]
    fn lower_set_allocs_blits_addresses_and_stores() {
        // R18: `set` allocs a fresh array, blits the whole original into it,
        // addresses the element, and stores the new value — yielding a new
        // array while the original is untouched (value semantics, D5).
        let ir = lower_src(": w ( [i64 4] -- [i64 4] ) 0 9 set ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::Blit(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldStore(..))), 1);
    }

    #[test]
    fn lower_get_runtime_index_emits_bounds_guard_and_trap_call() {
        // R19/D6: a runtime (non-literal) index guards the access with
        // `index < N` and jumps to a trap block that calls the OOB helper.
        let ir = lower_src(": w ( [i64 4] usize -- i64 ) get swap drop ;");
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
    fn lower_get_constant_index_has_no_runtime_guard() {
        // R11/X4: a checked literal index is bounds-verified at compile time,
        // so it skips the runtime guard entirely — no branch, no trap call.
        let ir = lower_src(": w ( [i64 4] -- i64 ) 0 get swap drop ;");
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
            &s,
            &Enums::default(),
            &Arrays::default(),
        );
        assert_eq!(m, 1);
        assert_eq!(out_bytes, 16);
        assert_eq!(count(&func, |i| matches!(i, Instr::Blit(..))), 2);
        // No scalar 8-byte-cell Load/Store touches a struct slot.
        assert_eq!(count(&func, |i| matches!(i, Instr::Load(..))), 0);
        assert_eq!(count(&func, |i| matches!(i, Instr::Store(..))), 0);
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
            &Structs::default(),
            &Enums::default(),
            &Arrays::default(),
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
            &Structs::default(),
            &Enums::default(),
            &Arrays::default(),
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
            &Structs::default(),
            &Enums::default(),
            &Arrays::default(),
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
            &Structs::default(),
            &Enums::default(),
            &Arrays::default(),
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
        let mut b = FuncBuilder::new(&env, &resolve, &structs, &enums, &arrays, "w".to_string());
        let x = b.fresh_value(u8);
        let y = b.fresh_value(u8);
        b.stack = vec![x, y];
        b.lower_call("+", 0, false);
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
        // blit), unlike a scalar `dup` which reuses the value id.
        let ir = lower_src("type: Vec2 x i64 y i64 ; : d ( Vec2 -- Vec2 Vec2 ) dup ;");
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
        let (structs, enums, _arrays) = {
            let src = "type: Vec2 x f64 y f64 ; type: Shape | Dot p Vec2 | Unit ;";
            let tokens = lex(src).unwrap();
            let mut module = parse(&tokens).unwrap();
            check(&mut module).unwrap();
            build_registries(&module.structs, &module.enums, &module.arrays)
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
        let (structs, _enums, _arrays) = {
            let src =
                "type: Shape | Circle r f64 | Rect w f64 h f64 ; type: Tagged k Shape n i64 ;";
            let tokens = lex(src).unwrap();
            let mut module = parse(&tokens).unwrap();
            check(&mut module).unwrap();
            build_registries(&module.structs, &module.enums, &module.arrays)
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
        // blit), like a struct and unlike a scalar.
        let ir = lower_src(
            "type: MaybeInt | None | Some v i64 ; : d ( MaybeInt -- MaybeInt MaybeInt ) dup ;",
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
        let (structs, enums, arrays) = {
            let tokens = lex(src).unwrap();
            let mut module = parse(&tokens).unwrap();
            check(&mut module).unwrap();
            build_registries(&module.structs, &module.enums, &module.arrays)
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
            &structs,
            &enums,
            &arrays,
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

    fn header_block<'a>(func: &'a IrFunc, header: BlockId) -> &'a Block {
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
        assert_eq!(phis.len(), 2, "one header phi per input slot (i64, Flag)");
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
        assert_eq!(hphis.len(), 2);
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
}
