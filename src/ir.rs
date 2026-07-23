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

use crate::ast::{Module, StructDecl, StructId, Term, TermKind, Type, WordDef};

#[derive(Debug, Default)]
pub struct IrModule {
    pub funcs: Vec<IrFunc>,
    /// Per-struct memory layout (R11), indexed by `StructId`. The backend emits
    /// a `type :S = { … }` per entry and reads field offsets/widths from it;
    /// empty for a struct-free module (or a single-func REPL emit).
    pub structs: Vec<StructLayout>,
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
    /// A user-declared struct (R11), keyed by a small `Copy` `StructId` into the
    /// module's `StructLayout` registry; the layout (offsets/size/align) lives
    /// there, not inlined, so `IrType` stays `Copy`. At runtime a struct value
    /// is a pointer to its aggregate storage; the backend spells it `:S` in
    /// ABI positions (params/returns/call args) and `l` (a pointer) in a
    /// register.
    Struct(StructId),
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
        // The layout lives in the module's `StructLayout` registry (R11); the
        // `IrType` carries only the `StructId` so it stays `Copy`.
        Type::Struct(id, _) => IrType::Struct(id),
    }
}

/// The computed memory layout of one struct (R11), word-width-neutral: every
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

/// How a generated struct-word name lowers (R13): the four kinds keyed off the
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

fn round_up(offset: u32, align: u32) -> u32 {
    offset.div_ceil(align) * align
}

/// The size/align of a scalar `IrType` (R11): `i8`/`u8`/`bool` = 1, `i16`/`u16`
/// = 2, `i32`/`u32`/`f32` = 4, `i64`/`u64`/`f64` = 8. A `Ptr` is 8 (unused as a
/// field this slice). Never called on a `Struct` (nested fields resolve through
/// the layout registry).
fn scalar_size_align(ty: IrType) -> (u32, u32) {
    let bytes = match ty {
        IrType::Bool => 1,
        IrType::Int { bits, .. } => (bits / 8) as u32,
        IrType::Float { bits } => (bits / 8) as u32,
        IrType::Ptr => 8,
        IrType::Struct(_) => unreachable!("a struct field resolves via the layout registry"),
    };
    (bytes, bytes)
}

/// The carried-stack bytes a slot of `ty` occupies (R16). A scalar stays a
/// byte-identical 8-byte cell, so every scalar-only line marshals exactly as
/// before; a struct occupies its aggregate size rounded up to a multiple of 8
/// so the next slot stays 8-aligned (struct alignment is at most 8 this slice).
/// Cumulative sums give each carried slot's byte offset in the buffer.
pub fn carried_slot_bytes(ty: IrType, structs: &Structs) -> u32 {
    match ty {
        IrType::Struct(id) => round_up(structs.layouts[id.index()].size, 8),
        _ => 8,
    }
}

impl Structs {
    /// Build the layout + generated-word registry from a program's struct
    /// declarations (the build path passes `&module.structs`, the REPL passes
    /// its accumulated registry). Recursion is already rejected by the checker
    /// (X3), so the memoized layout recursion terminates.
    pub fn from_structs(structs: &[StructDecl]) -> Structs {
        let mut memo: Vec<Option<StructLayout>> = vec![None; structs.len()];
        for i in 0..structs.len() {
            compute_layout(structs, i, &mut memo);
        }
        let layouts: Vec<StructLayout> = memo.into_iter().map(|l| l.expect("layout")).collect();

        let mut words = HashMap::new();
        for (idx, decl) in structs.iter().enumerate() {
            let id = StructId::from_index(idx);
            words.insert(decl.name.clone(), StructWord::Construct(id));
            words.insert(format!("{}>", decl.name), StructWord::Destructure(id));
            for (fi, (fname, _)) in decl.fields.iter().enumerate() {
                words.insert(format!("{}>{}", decl.name, fname), StructWord::Get(id, fi));
                words.insert(format!("{}<{}", decl.name, fname), StructWord::Set(id, fi));
            }
        }
        Structs { layouts, words }
    }
}

/// Fill `memo[idx]` with the natural-alignment layout of struct `idx`, recursing
/// into nested-struct fields first (D9). Each field is placed at the next offset
/// aligned to its own alignment; struct align = max field align (min 1); struct
/// size = final offset rounded up to struct align (R11).
fn compute_layout(structs: &[StructDecl], idx: usize, memo: &mut Vec<Option<StructLayout>>) {
    if memo[idx].is_some() {
        return;
    }
    let decl = &structs[idx];
    let mut offset = 0u32;
    let mut align = 1u32;
    let mut fields = Vec::with_capacity(decl.fields.len());
    for (_, field_ty) in &decl.fields {
        let ir_ty = ir_type_of(*field_ty);
        let (size, falign) = match ir_ty {
            IrType::Struct(id) => {
                compute_layout(structs, id.index(), memo);
                let inner = memo[id.index()].as_ref().expect("inner layout computed");
                (inner.size, inner.align)
            }
            _ => scalar_size_align(ir_ty),
        };
        let off = round_up(offset, falign);
        fields.push(FieldLayout {
            offset: off,
            ty: ir_ty,
            size,
            align: falign,
        });
        offset = off + size;
        align = align.max(falign);
    }
    let size = round_up(offset, align);
    memo[idx] = Some(StructLayout {
        name: decl.name_static,
        size,
        align,
        fields,
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// A float constant carrying its `f64` value (R14). Distinct from `Const`
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
    /// `dst: Int = *ptr`.
    Load(Value, Value),
    /// `*ptr = val` (Int).
    Store(Value, Value),
    /// `dst: Struct = alloc(size, align)`: a frame-local aggregate slot (R13).
    /// The two operands are the whole-struct byte size and alignment from the
    /// layout registry.
    Alloc(Value, u32, u32),
    /// `blit src -> dst, size`: copy `size` bytes between two aggregate
    /// pointers (R14/R13) — the byte-copy `dup`, the setter's copy-all, and a
    /// nested-struct field store.
    Blit(Value, Value, u32),
    /// `dst = *ptr` at the field's exact width (R15), the load op picked from
    /// `dst`'s scalar `IrType` (`loadsb`/`loadub`/`loadsh`/…). Distinct from the
    /// 8-byte-slot `Load` so a field read never over-reads its neighbour.
    FieldLoad(Value, Value),
    /// `*ptr = val` at `val`'s exact width (R15), the store op picked from
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
    let structs = Structs::from_structs(&module.structs);
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
        .map(|w| lower_word(w, &env, &resolve, &structs))
        .collect();

    Ok(IrModule {
        funcs,
        structs: structs.layouts,
    })
}

/// Lower a bare REPL line to a uniform-signature wrapper `sooth_line_{seq}`
/// `(Ptr stack, Int top) -> Int`. The prologue loads the whole carried stack
/// (`entry_depth` slots) from the buffer, the body runs in registers exactly
/// like a word, the epilogue stores the resulting output slots back, and it
/// returns the advanced top `top + (out_bytes - in_bytes)`.
///
/// Carried slots are size-aware per slot (R16, D5): a scalar occupies a
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
pub fn lower_line(
    seq: u64,
    terms: &[Term],
    entry_depth: usize,
    entry_types: &[Type],
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    structs: &Structs,
) -> (IrFunc, usize, usize) {
    debug_assert_eq!(entry_types.len(), entry_depth);
    let mut b = FuncBuilder::new(env, resolve, structs);

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
        in_bytes += carried_slot_bytes(slot_ty, b.structs);
    }
    b.stack = stack;

    b.lower_terms(terms);

    // Epilogue: store each result slot back to the buffer at its cumulative
    // byte offset. A scalar 8-byte cell is written at the value's own width
    // (R20): a float via `stores`/`stored`, an integer or `Bool` via `storel`
    // (a `Bool` widening to `l`, its stored 0/1 valid `l`-content). A struct
    // is copied back into the buffer by an aggregate `blit` (R16).
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
            _ => b.push_instr(Instr::Store(ptr, *v)),
        }
        out_bytes += carried_slot_bytes(vty, b.structs);
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
) -> IrFunc {
    let params: Vec<IrType> = word
        .effect
        .inputs
        .iter()
        .map(|s| ir_type_of(s.ty))
        .collect();
    let ret = word.effect.outputs.first().map(|s| ir_type_of(s.ty));

    let mut b = FuncBuilder::new(env, resolve, structs);

    // Params occupy the first N value ids; leftmost input is deepest.
    let mut stack: Vec<Value> = params.iter().map(|ty| b.fresh_value(*ty)).collect();

    // Bind `| ... |` locals: pop the top N params, leftmost local = deepest.
    let take = word.locals.len();
    let bound = stack.split_off(stack.len() - take);
    for (name, value) in word.locals.iter().zip(bound) {
        b.locals.insert(name.clone(), value);
    }
    b.stack = stack;

    b.lower_terms(&word.body);

    let result = if ret.is_some() { b.stack.pop() } else { None };
    b.seal_block(Terminator::Ret(result));

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
    blocks: Vec<Block>,
    cur_id: BlockId,
    cur_instrs: Vec<Instr>,
    next_value: u32,
    next_block: u32,
    stack: Vec<Value>,
    locals: HashMap<String, Value>,
    value_types: Vec<IrType>,
}

impl<'a> FuncBuilder<'a> {
    fn new(env: &'a HashMap<String, Arity>, resolve: Resolver<'a>, structs: &'a Structs) -> Self {
        FuncBuilder {
            env,
            resolve,
            structs,
            blocks: Vec::new(),
            cur_id: BlockId(0),
            cur_instrs: Vec::new(),
            next_value: 0,
            next_block: 1, // block 0 is the entry, already current
            stack: Vec::new(),
            locals: HashMap::new(),
            value_types: Vec::new(),
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

    fn lower_terms(&mut self, terms: &[Term]) {
        for term in terms {
            self.lower_term(term);
        }
    }

    fn lower_term(&mut self, term: &Term) {
        match &term.kind {
            TermKind::IntLit(n) => {
                let v = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(v, *n));
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
            TermKind::Call(name) => self.lower_call(name),
            TermKind::If {
                then_branch,
                else_branch,
            } => self.lower_if(then_branch, else_branch),
        }
    }

    fn lower_call(&mut self, name: &str) {
        if let Some(&value) = self.locals.get(name) {
            self.stack.push(value); // i64 is Copy; reuse the value id.
            return;
        }
        match name {
            "dup" => {
                let top = *self.stack.last().expect("dup: non-empty stack");
                // A scalar is `Copy`: reuse the value id (dup emits nothing). A
                // struct is copied by value (R14): alloc a fresh slot and blit
                // the bytes, so a functional setter on the copy leaves the
                // original intact.
                if let IrType::Struct(id) = self.value_type(top) {
                    let copy = self.alloc_struct(id);
                    let size = self.structs.layouts[id.index()].size;
                    if size > 0 {
                        self.push_instr(Instr::Blit(top, copy, size));
                    }
                    self.stack.push(copy);
                } else {
                    self.stack.push(top);
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
                // alloc/blit/field-load-store inline (R13), not a normal call.
                if let Some(&sw) = self.structs.words.get(name) {
                    self.lower_struct_word(sw);
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
    /// `Struct`-typed value (a pointer to the storage) (R13).
    fn alloc_struct(&mut self, id: StructId) -> Value {
        let (size, align) = {
            let l = &self.structs.layouts[id.index()];
            (l.size, l.align)
        };
        let v = self.fresh_value(IrType::Struct(id));
        self.push_instr(Instr::Alloc(v, size, align));
        v
    }

    /// A `Ptr`-typed value for `base + offset` (a scalar field's address).
    fn field_ptr(&mut self, base: Value, offset: u32) -> Value {
        let p = self.fresh_value(IrType::Ptr);
        self.push_instr(Instr::PtrOffset(p, base, offset as i64));
        p
    }

    /// A nested-struct field's value: its interior address, typed as the inner
    /// struct (R13/R15). No copy — the owning struct is consumed by the
    /// getter/destructure, so aliasing its storage is sound; a later `dup` or
    /// word-return copies the bytes.
    fn field_struct_value(&mut self, base: Value, offset: u32, inner: StructId) -> Value {
        let v = self.fresh_value(IrType::Struct(inner));
        self.push_instr(Instr::PtrOffset(v, base, offset as i64));
        v
    }

    /// Store `val` into field `field` at `fptr`: a width-exact scalar store, or
    /// an aggregate blit for a nested-struct field (R15).
    fn store_field(&mut self, fptr: Value, val: Value, field: FieldLayout) {
        match field.ty {
            IrType::Struct(_) => {
                if field.size > 0 {
                    self.push_instr(Instr::Blit(val, fptr, field.size));
                }
            }
            _ => self.push_instr(Instr::FieldStore(fptr, val)),
        }
    }

    /// Load field `field` at `fptr` onto the stack: a width-exact scalar load,
    /// or the interior pointer as a nested-struct value (R13/R15).
    fn load_field_onto_stack(&mut self, base: Value, field: FieldLayout) {
        let v = match field.ty {
            IrType::Struct(inner) => self.field_struct_value(base, field.offset, inner),
            _ => {
                let fptr = self.field_ptr(base, field.offset);
                let v = self.fresh_value(field.ty);
                self.push_instr(Instr::FieldLoad(v, fptr));
                v
            }
        };
        self.stack.push(v);
    }

    /// Lower a generated struct word inline (R13, M1: first field deepest).
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

    fn lower_if(&mut self, then_branch: &[Term], else_branch: &[Term]) {
        let test = self.stack.pop().expect("if: test value");
        let then_id = self.fresh_block();
        let else_id = self.fresh_block();
        let join_id = self.fresh_block();

        let post_pop = self.stack.clone();
        self.seal_block(Terminator::Jnz(test, then_id, else_id));

        self.start_block(then_id);
        self.stack = post_pop.clone();
        self.lower_terms(then_branch);
        let then_stack = self.stack.clone();
        let then_pred = self.cur_id;
        self.seal_block(Terminator::Jmp(join_id));

        self.start_block(else_id);
        self.stack = post_pop;
        self.lower_terms(else_branch);
        let else_stack = self.stack.clone();
        let else_pred = self.cur_id;
        self.seal_block(Terminator::Jmp(join_id));

        self.start_block(join_id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Line;
    use crate::check::check;
    use crate::lexer::lex;
    use crate::parser::{parse, parse_line};

    fn lower_src(src: &str) -> IrModule {
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        check(&module).unwrap();
        lower(&module).unwrap()
    }

    fn structs_of(src: &str) -> Structs {
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        check(&module).unwrap();
        Structs::from_structs(&module.structs)
    }

    fn layout<'a>(s: &'a Structs, name: &str) -> &'a StructLayout {
        s.layouts.iter().find(|l| l.name == name).expect("layout")
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
        let ir = lower_src(": w ( bool -- i64 ) if 1 else 2 then ;");
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
        // aggregate size rounded up to a multiple of 8 (R16).
        let s = structs_of("type: Pair a i8 b i8 ;\ntype: Vec2 x i64 y i64 ;");
        assert_eq!(carried_slot_bytes(IrType::I64, &s), 8);
        assert_eq!(carried_slot_bytes(IrType::Bool, &s), 8);
        // Pair is two i8s = 2 bytes, rounded up to one 8-byte cell.
        assert_eq!(
            carried_slot_bytes(IrType::Struct(StructId::from_index(0)), &s),
            8
        );
        // Vec2 is two i64s = 16 bytes, already a multiple of 8.
        assert_eq!(
            carried_slot_bytes(IrType::Struct(StructId::from_index(1)), &s),
            16
        );
    }

    #[test]
    fn lower_line_struct_slot_blits_in_and_out() {
        // A carried struct slot is copied out of the buffer on entry and back
        // on exit by aggregate blits, and the returned top advances by the
        // struct's aligned carried size (R16). An empty line carries the one
        // Vec2 straight through: one prologue blit, one epilogue blit.
        let s = structs_of("type: Vec2 x i64 y i64 ;");
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let vec2 = Type::Struct(StructId::from_index(0), "Vec2");
        let (func, m, out_bytes) = lower_line(0, &line_terms(""), 1, &[vec2], &env, &resolve, &s);
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
        let (func, _m, _) =
            lower_line(0, &terms, 1, &[f64_ty], &env, &resolve, &Structs::default());
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
        let mut b = FuncBuilder::new(&env, &resolve, &structs);
        let x = b.fresh_value(u8);
        let y = b.fresh_value(u8);
        b.stack = vec![x, y];
        b.lower_call("+");
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
        // fields (R13); no aggregate copy for a flat struct.
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
        // single width-exact store of the replaced field (R13).
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
}
