//! QBE backend: emit QBE IL text from the neutral IR.
//!
//! Driver then pipes this through `qbe` (-> assembly) and `cc` (-> native binary).
//! QBE gives arm64/x86_64/riscv64 and C-ABI struct classification for free; costs
//! accepted are i128 synthesised in the frontend and atomics via C11 FFI.

use std::fmt::Write;

use crate::ast::BOOL_ENUM_ID;
use crate::ir::{
    ArrayLayout, BinOp, BlockId, CmpOp, EnumLayout, Instr, IrFunc, IrModule, IrType, QuotSigId,
    QuotSigLayout, StructLayout, Terminator, Value, ALLOC_SYMBOL, FREE_SYMBOL, OOB_TRAP_SYMBOL,
    TRACE_ALLOC_ENV, WORD_WIDTH,
};

/// Reached only from the allocator shim's NULL branch, so unlike
/// `OOB_TRAP_SYMBOL` the IR never names it and it stays backend-private.
const OOM_TRAP_SYMBOL: &str = "sooth_oom_trap";

/// Shared by both halves of the allocator so the gate is decided in exactly
/// one place. Backend-private for the same reason as `OOM_TRAP_SYMBOL`.
const TRACE_EVENT_SYMBOL: &str = "sooth_trace_event";

/// The backend's view of a program's aggregate layouts: the struct and enum
/// registries, threaded as one `Copy` handle so a nested-aggregate member/ABI
/// spelling can consult either (a struct field of enum type, D9). The
/// registries stay separate (D10); this only co-locates two `&[...]`.
#[derive(Clone, Copy)]
struct Layouts<'a> {
    structs: &'a [StructLayout],
    enums: &'a [EnumLayout],
    /// Slice 7a: the module's distinct quotation signatures, so a quotation
    /// value's ABI/member spelling resolves to its `:Q{n}` symbol.
    quot_sigs: &'a [QuotSigLayout],
}

/// Slice 7a: the `:Q{n}` symbol index for a quotation signature, found by
/// structural effect equality (two equal effects share one `type :Q{n}`).
fn quot_index(layouts: Layouts, sig: QuotSigId) -> usize {
    layouts
        .quot_sigs
        .iter()
        .position(|q| q.effect == sig.0)
        .expect("a quotation IrType names an effect interned in `quot_sigs`")
}

pub fn emit(ir: &IrModule) -> Result<String, String> {
    let mut out = String::new();
    let layouts = Layouts {
        structs: &ir.structs,
        enums: &ir.enums,
        quot_sigs: &ir.quot_sigs,
    };
    out.push_str("data $fmt = { b \"%ld\\n\", b 0 }\n");
    out.push_str("data $ufmt = { b \"%lu\\n\", b 0 }\n");
    out.push_str("data $ffmt = { b \"%g\\n\", b 0 }\n");
    // Bool prints via a 2-entry pointer table indexed by the canonical 0/1
    // value (no branch needed): `$boolstrs[v]` selects `$true_str`/`$false_str`,
    // printed through `%s` (`$sfmt`).
    out.push_str("data $sfmt = { b \"%s\", b 0 }\n");
    out.push_str("data $true_str = { b \"true\\n\", b 0 }\n");
    out.push_str("data $false_str = { b \"false\\n\", b 0 }\n");
    out.push_str("data $boolstrs = { l $false_str, l $true_str }\n");
    // The runtime out-of-bounds trap message: a located line + the offending
    // index + the array length, printed to stderr before a nonzero exit.
    out.push_str(
        "data $oobfmt = { b \"sooth: array index out of range (line %ld)\\n  index %ld is out of bounds for length %ld\\n\", b 0 }\n",
    );
    // Both trace lines go through the same `printf` path as `.`, so program
    // order equals transcript order in a golden.
    out.push_str("data $allocfmt = { b \"alloc %ld\\n\", b 0 }\n");
    out.push_str("data $freefmt = { b \"free %ld\\n\", b 0 }\n");
    writeln!(out, "data $tracenv = {{ b \"{TRACE_ALLOC_ENV}\", b 0 }}").unwrap();
    // The failed-allocation message: stderr, then a nonzero exit, exactly
    // like the out-of-bounds trap.
    out.push_str(
        "data $oomfmt = { b \"sooth: out of memory (allocation of %ld bytes failed)\\n\", b 0 }\n",
    );
    // `str`'s print format; see `Instr::Print`'s `IrType::Str` arm for why `%.*s`.
    out.push_str("data $strfmt = { b \"%.*s\", b 0 }\n");
    let str_lits = collect_str_literals(&ir.funcs);
    // Slice 9: emitted in `idx` order, not `HashMap` iteration order -- a
    // module with two or more distinct string literals (previously never
    // exercised by the corpus; the injected `bool` print overload's
    // `"false\n"`/`"true\n"` literals are the first) produced nondeterministic
    // `$strb{N}` declaration order across process runs otherwise, since each
    // content's `idx` binding was already fixed but the printing loop wasn't.
    let mut ordered_lits: Vec<(&String, usize)> = str_lits
        .iter()
        .map(|(content, idx)| (content, *idx))
        .collect();
    ordered_lits.sort_by_key(|(_, idx)| *idx);
    for (content, idx) in ordered_lits {
        emit_str_literal(&mut out, idx, content);
    }
    // Enum and array aggregates are self-contained opaque byte blobs (they name
    // no member types), so they are emitted first: a struct member of enum or
    // array type then references an already-declared `:E`/`:arr_N`. Structs are
    // the only aggregates whose QBE type spells its members, so a struct nested
    // by value inside another must already be defined when QBE reaches the
    // outer struct's `type` line (QBE resolves aggregate names in one pass,
    // no forward references); `topo_sorted_structs` emits them in containment
    // order rather than assuming source-declaration order already is one.
    // Slice 9 (R1/R3): a zero-payload enum (`Bool`, generally) never needs
    // its own aggregate type -- it is a bare scalar, nowhere spelled as `:name`
    // (`qbe_abi_ty`/`member_ty` route it to a register width instead), so
    // emitting its declaration here would be a dead, byte-for-byte-breaking
    // addition to every program's QBE text.
    for layout in &ir.enums {
        if !layout.is_scalar {
            emit_enum_type(&mut out, layout);
        }
    }
    for (idx, layout) in ir.arrays.iter().enumerate() {
        emit_array_type(&mut out, idx, layout);
    }
    // Slice 7a: a quotation value is a fixed two-slot `{ code, env }`
    // aggregate; every distinct effect gets its own `:Q{n}` (like arrays,
    // self-contained, so a struct field of quotation type sees it already
    // declared). All slots classify `l`, so the members are `{ l, l }`.
    for idx in 0..ir.quot_sigs.len() {
        writeln!(out, "type :Q{idx} = {{ l, l }}").unwrap();
    }
    for idx in topo_sorted_structs(&ir.structs) {
        emit_struct_type(&mut out, &ir.structs[idx], layouts);
    }
    for func in &ir.funcs {
        out.push('\n');
        emit_func(&mut out, func, layouts, &str_lits);
    }
    emit_oob_trap(&mut out);
    emit_alloc_shim(&mut out);
    emit_free_shim(&mut out);
    emit_oom_trap(&mut out);
    emit_trace_event(&mut out);
    Ok(out)
}

/// Emit a struct's QBE aggregate type `type :Name = { members }`, one
/// member per field in layout order. QBE re-derives offsets from the member
/// list with natural alignment, which agrees with the hand-computed layout,
/// the load-bearing ABI-agreement property. A zero-field struct emits an
/// empty aggregate `{ }`.
fn emit_struct_type(out: &mut String, layout: &StructLayout, layouts: Layouts) {
    let members: Vec<String> = layout
        .fields
        .iter()
        .map(|f| member_ty(f.ty, layouts))
        .collect();
    writeln!(
        out,
        "type :{} = {{ {} }}",
        qbe_name(layout.name),
        members.join(", ")
    )
    .unwrap();
}

/// Order struct indices (into `ir.structs`, keyed by `StructId`) so a struct
/// nested by value inside another is always emitted before it: a DFS
/// postorder over the by-value containment edges (`IrType::Struct` fields).
/// `check.rs` rejects illegal by-value cycles at check time, so this graph is
/// guaranteed acyclic and the recursion always terminates; `emitted` also
/// guards the ordinary (non-cyclic) diamond case where two structs share a
/// common nested struct, so that struct is emitted once, not once per user.
fn topo_sorted_structs(structs: &[StructLayout]) -> Vec<usize> {
    fn visit(idx: usize, structs: &[StructLayout], emitted: &mut [bool], order: &mut Vec<usize>) {
        if emitted[idx] {
            return;
        }
        emitted[idx] = true;
        for field in &structs[idx].fields {
            if let IrType::Struct(id) = field.ty {
                visit(id.index(), structs, emitted, order);
            }
        }
        order.push(idx);
    }
    let mut order = Vec::with_capacity(structs.len());
    let mut emitted = vec![false; structs.len()];
    for idx in 0..structs.len() {
        visit(idx, structs, &mut emitted, &mut order);
    }
    order
}
/// blob `type :Name = align A { b N }` (R15): the payload has no single member
/// layout across variants, so the aggregate is sized/aligned only, and every
/// access is offset-driven (explicit `PtrOffset` + width-exact field ops +
/// `Blit`). Caller and callee agree because they share `:Name`. A zero-size
/// enum still emits at least one byte so the type is non-empty.
fn emit_enum_type(out: &mut String, layout: &EnumLayout) {
    writeln!(
        out,
        "type :{} = align {} {{ b {} }}",
        qbe_name(layout.name),
        layout.align,
        layout.size.max(1)
    )
    .unwrap();
}

/// Emit an array's QBE aggregate type as an alignment-annotated opaque byte
/// blob `type :Name = align A { b N }` (R20), like the enum aggregate: the
/// backend never reasons about element structure except through the
/// element-addressing op + width-exact field ops + `Blit`. Caller and callee
/// agree because they share `:Name`.
fn emit_array_type(out: &mut String, idx: usize, layout: &ArrayLayout) {
    writeln!(
        out,
        "type :{} = align {} {{ b {} }}",
        array_type_symbol(idx),
        layout.align,
        layout.size.max(1)
    )
    .unwrap();
}

/// The QBE aggregate symbol for array `idx`: the `[T N]` spelling is not a
/// valid QBE identifier (it contains `[`, spaces, `]`), so an array's `:A`
/// name is derived from its `ArrayId` index instead, which is unique per
/// compilation unit. A struct/enum aggregate keeps its declared spelling but
/// goes through `qbe_name` (injectively) at every emission site, since a
/// hyphenated user type name -- and a monomorphized generic instantiation's
/// `Box[i64]` registry name -- are no more valid QBE identifiers than `[T N]`.
fn array_type_symbol(idx: usize) -> String {
    format!("arr_{idx}")
}

/// The Sooth `main` word is emitted as `sooth_main`; the C shim owns `main`.
/// Sooth word names may contain characters (`-`, `<`, `>`, etc., identifier-
/// continuation characters in the lexer) that are not valid in a QBE global
/// symbol. Every character outside `[A-Za-z0-9_]` -- including a literal
/// `.`, since `.` is the escape lead-in below and must not become ambiguous
/// with one -- becomes `.{hex}.`, its codepoint in lowercase hex. This is
/// injective: scanning the output left to right, `.` is never a passthrough
/// character, so it can only ever open or close an escape, which means every
/// output string has exactly one valid parse back into the source characters
/// that produced it. Two distinct names can therefore never collide on the
/// same symbol (`+` and `-` used to both sanitize to the bare symbol `_`;
/// they now produce `.2b.` and `.2d.`). `.` is deliberately the escape
/// character rather than `_`: every compiler-generated symbol this function
/// also has to leave alone (`sooth_line_{seq}`'s dlopen lookup in `repl.rs`,
/// the resolver's `__m{module}`/`__import{epoch}` mangle suffix, the alloc/
/// free/OOB-trap shims below, all `_`-only) would otherwise need to move in
/// lockstep with a change here; none of them contain `.`, so none of them
/// are affected by fixing this. Applied identically at both the function
/// definition and every call site, so a lookup by name always finds the
/// symbol that source name actually owns.
fn qbe_name(name: &str) -> std::borrow::Cow<'_, str> {
    if name == "main" {
        return std::borrow::Cow::Borrowed("sooth_main");
    }
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        std::borrow::Cow::Borrowed(name)
    } else {
        let mut out = String::with_capacity(name.len());
        for c in name.chars() {
            if c.is_ascii_alphanumeric() || c == '_' {
                out.push(c);
            } else {
                out.push('.');
                out.push_str(&format!("{:x}", c as u32));
                out.push('.');
            }
        }
        std::borrow::Cow::Owned(out)
    }
}

fn val(v: Value) -> String {
    format!("%v{}", v.0)
}

fn label(id: BlockId) -> String {
    if id.0 == 0 {
        "@start".to_string()
    } else {
        format!("@blk{}", id.0)
    }
}

/// The QBE base-type letter for an `IrType`, derived here (not in the IR, R15):
/// `Bool` is a 4-byte `w` (0/1); an integer is `w` for `bits <= 32` and `l` for
/// `64`; a float is `s` (32) or `d` (64); `Ptr` is the 8-byte `l` used by the
/// buffer and C ABI. This is the only place the `s`/`d` register class is
/// spelled (NF2).
fn width(ty: IrType, layouts: Layouts) -> &'static str {
    match ty {
        IrType::Bool => "w",
        IrType::Int { bits, .. } => {
            if bits <= 32 {
                "w"
            } else {
                "l"
            }
        }
        IrType::Float { bits } => {
            if bits == 32 {
                "s"
            } else {
                "d"
            }
        }
        // `usize` is a target-width unsigned integer; on the 8-byte QBE target
        // it fills the `l` register (its width flows from `WORD_WIDTH`, R15).
        IrType::Usize => "l",
        IrType::Isize => "l",
        IrType::Ptr => "l",
        // An owning cell is its heap pointer in a register; only the frontend
        // distinguishes it (to emit the free on `drop`).
        IrType::OwnedCell(_) => "l",
        // Slice 9 (R1/R3): a zero-payload enum (`Bool`, generally) is a bare
        // scalar discriminant, not a memory aggregate -- it runs at the same
        // `w` register class the retired primitive `Bool` used, so `Cmp`/
        // `Jnz`/bitwise codegen stays byte-for-byte.
        IrType::Enum(id) if layouts.enums[id.index()].is_scalar => "w",
        // A struct/enum/array value is a pointer in a register (`l`); its
        // aggregate `:S`/`:E`/`:A` type is only spelled in ABI positions.
        IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => "l",
        // `str`'s descriptor address and `cstr`'s bytes pointer are each one
        // opaque pointer, exactly like `Ptr`.
        IrType::Str | IrType::Cstr => "l",
        // A code handle and a quotation value are each one pointer in a
        // register; the quotation's `:Q{n}` aggregate type is only spelled in
        // ABI/member positions.
        IrType::Code | IrType::Quotation(_) => "l",
    }
}

/// The QBE type spelled in an ABI position (a function param/return or a call
/// argument): a struct is its aggregate `:Name` (so QBE applies its C-ABI
/// by-value classification); every scalar is its register `width`, including a
/// zero-payload enum (Slice 9, R1/R8): it is never passed/returned as an
/// aggregate, so a returned `Bool` stays a scalar ABI value.
fn qbe_abi_ty(ty: IrType, layouts: Layouts) -> String {
    match ty {
        IrType::Struct(id) => format!(":{}", qbe_name(layouts.structs[id.index()].name)),
        IrType::Enum(id) if layouts.enums[id.index()].is_scalar => width(ty, layouts).to_string(),
        IrType::Enum(id) => format!(":{}", qbe_name(layouts.enums[id.index()].name)),
        IrType::Array(id) => format!(":{}", array_type_symbol(id.index())),
        IrType::Quotation(sig) => format!(":Q{}", quot_index(layouts, sig)),
        _ => width(ty, layouts).to_string(),
    }
}

/// The QBE member letter for a struct field: `b`/`h`/`w`/`l` by integer width,
/// `s`/`d` by float width, `:Inner` for a nested struct.
fn member_ty(ty: IrType, layouts: Layouts) -> String {
    match ty {
        IrType::Bool => "b".to_string(),
        IrType::Int { bits: 8, .. } => "b".to_string(),
        IrType::Int { bits: 16, .. } => "h".to_string(),
        IrType::Int { bits: 32, .. } => "w".to_string(),
        IrType::Int { .. } => "l".to_string(),
        IrType::Float { bits: 32 } => "s".to_string(),
        IrType::Float { .. } => "d".to_string(),
        IrType::Usize => "l".to_string(),
        IrType::Isize => "l".to_string(),
        IrType::Ptr => "l".to_string(),
        IrType::OwnedCell(_) => "l".to_string(),
        IrType::Str | IrType::Cstr => "l".to_string(),
        // A code slot is one opaque pointer, `l` like `Ptr`.
        IrType::Code => "l".to_string(),
        // Slice 9 (R1): a zero-payload-enum field is a scalar byte, matching
        // the retired primitive `Bool`'s own member spelling.
        IrType::Enum(id) if layouts.enums[id.index()].is_scalar => "b".to_string(),
        IrType::Struct(id) => format!(":{}", qbe_name(layouts.structs[id.index()].name)),
        IrType::Enum(id) => format!(":{}", qbe_name(layouts.enums[id.index()].name)),
        IrType::Array(id) => format!(":{}", array_type_symbol(id.index())),
        IrType::Quotation(sig) => format!(":Q{}", quot_index(layouts, sig)),
    }
}

fn field_load_op(ty: IrType, layouts: Layouts) -> (&'static str, &'static str) {
    match ty {
        IrType::Bool => ("w", "loadub"),
        IrType::Int {
            bits: 8,
            signed: true,
        } => ("w", "loadsb"),
        IrType::Int { bits: 8, .. } => ("w", "loadub"),
        IrType::Int {
            bits: 16,
            signed: true,
        } => ("w", "loadsh"),
        IrType::Int { bits: 16, .. } => ("w", "loaduh"),
        IrType::Int { bits: 32, .. } => ("w", "loadw"),
        IrType::Int { .. } => ("l", "loadl"),
        IrType::Float { bits: 32 } => ("s", "loads"),
        IrType::Float { .. } => ("d", "loadd"),
        IrType::Usize => ("l", "loadl"),
        IrType::Isize => ("l", "loadl"),
        IrType::Ptr => ("l", "loadl"),
        IrType::OwnedCell(_) => ("l", "loadl"),
        IrType::Str | IrType::Cstr => ("l", "loadl"),
        // A quotation's `code` slot is one opaque pointer, loaded at `l`.
        IrType::Code => ("l", "loadl"),
        // Slice 9 (R1): a zero-payload enum loads exactly like `Bool` did.
        IrType::Enum(id) if layouts.enums[id.index()].is_scalar => ("w", "loadub"),
        IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) | IrType::Quotation(_) => {
            unreachable!("an aggregate field is copied by blit, not scalar-loaded")
        }
    }
}

fn field_store_op(ty: IrType, layouts: Layouts) -> &'static str {
    match ty {
        IrType::Bool => "storeb",
        IrType::Int { bits: 8, .. } => "storeb",
        IrType::Int { bits: 16, .. } => "storeh",
        IrType::Int { bits: 32, .. } => "storew",
        IrType::Int { .. } => "storel",
        IrType::Float { bits: 32 } => "stores",
        IrType::Float { .. } => "stored",
        IrType::Usize => "storel",
        IrType::Isize => "storel",
        IrType::Ptr => "storel",
        IrType::OwnedCell(_) => "storel",
        IrType::Str | IrType::Cstr => "storel",
        // A quotation's `code` slot is one opaque pointer, stored at `l`.
        IrType::Code => "storel",
        // Slice 9 (R1): a zero-payload enum stores exactly like `Bool` did.
        IrType::Enum(id) if layouts.enums[id.index()].is_scalar => "storeb",
        IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) | IrType::Quotation(_) => {
            unreachable!("an aggregate field is copied by blit, not scalar-stored")
        }
    }
}

/// The `alloc` mnemonic for a struct alignment: QBE offers `alloc4`/`alloc8`/
/// `alloc16`; an align of 1/2 rounds up to `alloc4` (over-alignment is sound).
fn alloc_op(align: u32) -> &'static str {
    match align {
        a if a <= 4 => "alloc4",
        8 => "alloc8",
        _ => "alloc16",
    }
}

/// A sub-word integer type (`bits < 32`, i.e. `i8`/`i16`/`u8`/`u16`) whose value
/// can carry dirty high bits in its `w` register after a width-overflowing op.
/// `i32`/`u32` fill the `w` register exactly and need no canonicalization.
fn sub_word(ty: IrType) -> Option<(u8, bool)> {
    match ty {
        IrType::Int { bits, signed } if bits < 32 => Some((bits, signed)),
        _ => None,
    }
}

/// The single sub-word canonicalization point (R15): normalize `src`'s
/// out-of-width bits into `dst` at register width `w`. A signed type
/// sign-extends from its low `bits` (`extsb`/`extsh`); an unsigned type masks
/// off everything above `bits`. Every dirtying op (sub-word arithmetic here,
/// narrowing conversion in the conversion lowering) routes through this so no
/// two code paths disagree on a value's high bits.
fn emit_canonicalize(
    out: &mut String,
    dst: &str,
    src: &str,
    w: &str,
    bits: u8,
    signed: bool,
) -> std::fmt::Result {
    if signed {
        let ext = match bits {
            8 => "extsb",
            16 => "extsh",
            _ => unreachable!("sub_word only yields bits 8/16"),
        };
        writeln!(out, "\t{dst} ={w} {ext} {src}")
    } else {
        let mask = (1u32 << bits) - 1;
        writeln!(out, "\t{dst} ={w} and {src}, {mask}")
    }
}

/// Lower a numeric conversion `dst = convert(src)` (R18), dispatching on the
/// source/target `IrType` classes: int->int (the Slice-2 path, unchanged),
/// int->float, float->float, float->int. The frontend never spells the QBE op;
/// the register class (`s`/`d`) is derived here (NF2).
fn emit_conv(
    out: &mut String,
    dst: Value,
    src: Value,
    value_types: &[IrType],
    layouts: Layouts,
    ext_id: &mut u32,
) -> std::fmt::Result {
    let src_ty = norm_scalar(ty_of(value_types, src));
    let dst_ty = norm_scalar(ty_of(value_types, dst));
    match (src_ty, dst_ty) {
        (IrType::Int { .. }, IrType::Int { .. }) => {
            emit_conv_int(out, dst, src, value_types, layouts, ext_id)
        }
        (
            IrType::Int {
                bits: sb,
                signed: ss,
            },
            IrType::Float { .. },
        ) => {
            // int -> float: the mnemonic picks source width (`w` for bits <= 32,
            // `l` for 64) and source signedness; the result letter (`s`/`d`)
            // selects the target float width. A sub-word source is already
            // canonical in its `w` carrier (R15), so `swtof`/`uwtof` read it
            // directly. Exact when representable, else round to nearest.
            let dw = width(dst_ty, layouts);
            let op = match (sb <= 32, ss) {
                (true, true) => "swtof",
                (true, false) => "uwtof",
                (false, true) => "sltof",
                (false, false) => "ultof",
            };
            writeln!(out, "\t{} ={dw} {op} {}", val(dst), val(src))
        }
        (IrType::Float { bits: sb }, IrType::Float { bits: db }) => {
            // float -> float: widen is exact (`exts`), narrow rounds to nearest
            // (`truncd`); a same-width `>fN` on its own type is a bit relabel.
            let dw = width(dst_ty, layouts);
            let m = if db > sb {
                "exts"
            } else if db < sb {
                "truncd"
            } else {
                "copy"
            };
            writeln!(out, "\t{} ={dw} {m} {}", val(dst), val(src))
        }
        (IrType::Float { bits: sb }, IrType::Int { .. }) => {
            // float -> int: truncate toward zero to the 32/64 integer carrier
            // (`stosi`/`dtosi` signed, `stoui`/`dtoui` unsigned), then the
            // shared canonicalization point (R15) for a sub-word target.
            // Out-of-range/NaN is unspecified this slice (D7).
            let ds = matches!(dst_ty, IrType::Int { signed: true, .. });
            let op = match (sb == 32, ds) {
                (true, true) => "stosi",
                (true, false) => "stoui",
                (false, true) => "dtosi",
                (false, false) => "dtoui",
            };
            match sub_word(dst_ty) {
                Some((bits, signed)) => {
                    let tmp = format!("%conv{ext_id}");
                    *ext_id += 1;
                    writeln!(out, "\t{tmp} =w {op} {}", val(src))?;
                    emit_canonicalize(out, &val(dst), &tmp, "w", bits, signed)
                }
                None => {
                    let dw = width(dst_ty, layouts);
                    writeln!(out, "\t{} ={dw} {op} {}", val(dst), val(src))
                }
            }
        }
        (s, d) => unreachable!("conversion endpoints are numeric, got {s:?} -> {d:?}"),
    }
}

/// Lower an integer conversion `dst = convert(src)` (R6), the Slice-2 path
/// unchanged. Widening extends by the *source* signedness (`exts*` signed,
/// `extu*` unsigned) from the source width; if the *target* is sub-word, that
/// extend is re-canonicalized to the target's own convention (R15), because the
/// source-signed extend is only accidentally canonical for the target: a signed
/// source widened to an unsigned sub-word target (e.g. `i8 >u16`) sign-extends
/// into bits the target requires to be zero, which a later in-register unsigned
/// compare would read as dirty. Narrowing keeps the low `dst` bits: for a
/// sub-word target that routes through the shared canonicalization point (R15),
/// otherwise a `w`-width `copy` truncates a `64 -> 32` step. Same-width is a
/// relabel: a plain `copy` when the target fills its register, but a sub-word
/// signedness flip (`u8 >i8`, `i8 >u8`) still re-canonicalizes to the new
/// convention so a later widen/compare reads the right high bits (Q5).
fn emit_conv_int(
    out: &mut String,
    dst: Value,
    src: Value,
    value_types: &[IrType],
    layouts: Layouts,
    ext_id: &mut u32,
) -> std::fmt::Result {
    let dst_ty = norm_scalar(ty_of(value_types, dst));
    let src_ty = norm_scalar(ty_of(value_types, src));
    let db = match dst_ty {
        IrType::Int { bits, .. } => bits,
        other => unreachable!("conversion target is always an integer, got {other:?}"),
    };
    let (sb, ss) = match src_ty {
        IrType::Int { bits, signed } => (bits, signed),
        other => unreachable!("conversion source is always an integer, got {other:?}"),
    };
    let dw = width(dst_ty, layouts);
    if db > sb {
        // Widen: sign-/zero-extend from the source width by the source sign.
        let ext = match (sb, ss) {
            (8, true) => "extsb",
            (8, false) => "extub",
            (16, true) => "extsh",
            (16, false) => "extuh",
            (32, true) => "extsw",
            (32, false) => "extuw",
            _ => unreachable!("widening source is 8/16/32 bits, got {sb}"),
        };
        match sub_word(dst_ty) {
            Some((bits, signed)) => {
                let tmp = format!("%widen{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{tmp} ={dw} {ext} {}", val(src))?;
                emit_canonicalize(out, &val(dst), &tmp, dw, bits, signed)
            }
            None => writeln!(out, "\t{} ={dw} {ext} {}", val(dst), val(src)),
        }
    } else {
        // Narrow or same-width: the value already sits in `src`'s low `db` bits.
        // Canonicalize a sub-word target; otherwise a `copy` fills (and, for a
        // `64 -> 32` narrowing, truncates) the register.
        match sub_word(dst_ty) {
            Some((bits, signed)) => emit_canonicalize(out, &val(dst), &val(src), dw, bits, signed),
            None => writeln!(out, "\t{} ={dw} copy {}", val(dst), val(src)),
        }
    }
}

fn ty_of(value_types: &[IrType], v: Value) -> IrType {
    value_types[v.0 as usize]
}

/// Normalize a scalar for the bits/signedness-inspecting arms (conversion,
/// `Rem`, `Cmp`, `Shl`/`Shr`) at the default target word width. Thin wrapper
/// over `norm_scalar_ww`, mirroring `scalar_size_align`/`scalar_size_align_ww`
/// (ir.rs).
fn norm_scalar(ty: IrType) -> IrType {
    norm_scalar_ww(ty, WORD_WIDTH)
}

/// `usize`/`isize` are target-width integers, so on a `word_width`-byte QBE
/// target they behave exactly like a `u64`/`i64` of that width; the bits
/// come from the parameter, never a hardcoded `64`. Every other type is
/// unchanged; the `width` register class already agrees (`l`).
fn norm_scalar_ww(ty: IrType, word_width: u32) -> IrType {
    match ty {
        IrType::Usize => IrType::Int {
            bits: (word_width * 8) as u8,
            signed: false,
        },
        IrType::Isize => IrType::Int {
            bits: (word_width * 8) as u8,
            signed: true,
        },
        other => other,
    }
}

/// Walk every function's instructions in order and assign each distinct
/// string literal content a stable index, deduping repeats of the same
/// content. Called once, before any function body is emitted, so `emit_func`
/// only ever looks an index up rather than assigning one (assigning per-func
/// would let the same content get two indices in two functions).
fn collect_str_literals(funcs: &[IrFunc]) -> std::collections::HashMap<String, usize> {
    let mut lits = std::collections::HashMap::new();
    for func in funcs {
        for block in &func.blocks {
            for instr in &block.instrs {
                if let Instr::StrLit(_, content) = instr {
                    if !lits.contains_key(content) {
                        let idx = lits.len();
                        lits.insert(content.clone(), idx);
                    }
                }
            }
        }
    }
    lits
}

/// The byte offset of a `str` descriptor's length word, matching the
/// `{ l $strb{idx}, l <len> }` shape `emit_str_literal` writes below. Read by
/// `Instr::StrLen`'s emission, the descriptor's only other consumer.
const STR_LEN_OFFSET: u32 = 8;

/// Emit one string literal's static data (R6): the byte content plus a
/// trailing NUL the descriptor's `len` does **not** count, which is what makes
/// R7's `cstr` conversion sound for a literal-rooted `str` (R11) without R4
/// promising anything, then the `{ptr, len}` descriptor itself. Every
/// byte is spelled as its own `b <decimal>` component rather than a quoted
/// string, so arbitrary content (embedded quotes, backslashes, control bytes)
/// never needs its own escaping pass.
fn emit_str_literal(out: &mut String, idx: usize, content: &str) {
    let mut bytes: Vec<String> = content
        .as_bytes()
        .iter()
        .map(|b| format!("b {b}"))
        .collect();
    bytes.push("b 0".to_string());
    writeln!(out, "data $strb{idx} = {{ {} }}", bytes.join(", ")).unwrap();
    writeln!(
        out,
        "data $strd{idx} = {{ l $strb{idx}, l {} }}",
        content.len()
    )
    .unwrap();
}

fn emit_func(
    out: &mut String,
    func: &IrFunc,
    layouts: Layouts,
    str_lits: &std::collections::HashMap<String, usize>,
) {
    let ret_ty = match func.ret {
        Some(ty) => format!("{} ", qbe_abi_ty(ty, layouts)),
        None => String::new(),
    };
    let params: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .map(|(i, ty)| format!("{} %v{i}", qbe_abi_ty(*ty, layouts)))
        .collect();
    writeln!(
        out,
        "export function {ret_ty}${}({}) {{",
        qbe_name(&func.name),
        params.join(", ")
    )
    .unwrap();
    let mut ext_id = 0u32;
    for block in &func.blocks {
        writeln!(out, "{}", label(block.id)).unwrap();
        for instr in &block.instrs {
            emit_instr(
                out,
                instr,
                &func.value_types,
                layouts,
                &mut ext_id,
                str_lits,
            );
        }
        emit_term(out, &block.term);
    }
    out.push_str("}\n");
}

/// Emit the runtime out-of-bounds trap helper (R19/D6): Sooth's first runtime
/// failure path. It prints the located len+index message to stderr via
/// `dprintf(2, …)` (the hosted print path, fd 2 = stderr, no new runtime
/// dependency) then `exit(1)`s. It must abort, not fall through, so the block
/// ends in `hlt`: after `exit` the program is gone, and `hlt` marks the edge
/// unreachable rather than returning into corrupt state.
fn emit_oob_trap(out: &mut String) {
    writeln!(
        out,
        "\nfunction ${OOB_TRAP_SYMBOL}(l %line, l %idx, l %len) {{"
    )
    .unwrap();
    out.push_str("@start\n");
    out.push_str("\tcall $dprintf(w 2, l $oobfmt, l %line, l %idx, l %len, ...)\n");
    out.push_str("\tcall $exit(w 1)\n");
    out.push_str("\thlt\n");
    out.push_str("}\n");
}

/// `max(n, 1)` because `malloc(0)` may return NULL.
fn emit_size_adjust(out: &mut String) {
    out.push_str("\t%zero =l ceql %n, 0\n");
    out.push_str("\t%adj =l add %n, %zero\n");
}

fn emit_alloc_shim(out: &mut String) {
    writeln!(out, "\nfunction l ${ALLOC_SYMBOL}(l %n) {{").unwrap();
    out.push_str("@start\n");
    emit_size_adjust(out);
    out.push_str("\t%p =l call $malloc(l %adj)\n");
    out.push_str("\t%isnull =w ceql %p, 0\n");
    out.push_str("\tjnz %isnull, @oom, @ok\n");
    out.push_str("@oom\n");
    writeln!(out, "\tcall ${OOM_TRAP_SYMBOL}(l %adj)").unwrap();
    out.push_str("\thlt\n");
    out.push_str("@ok\n");
    writeln!(out, "\tcall ${TRACE_EVENT_SYMBOL}(l $allocfmt, l %adj)").unwrap();
    out.push_str("\tret %p\n");
    out.push_str("}\n");
}

/// `malloc`'s `free` needs no size, but the interface carries one because
/// the trace reports it.
fn emit_free_shim(out: &mut String) {
    writeln!(out, "\nfunction ${FREE_SYMBOL}(l %p, l %n) {{").unwrap();
    out.push_str("@start\n");
    emit_size_adjust(out);
    out.push_str("\tcall $free(l %p)\n");
    writeln!(out, "\tcall ${TRACE_EVENT_SYMBOL}(l $freefmt, l %adj)").unwrap();
    out.push_str("\tret\n");
    out.push_str("}\n");
}

/// `exit(1)` rather than `abort`, so a test observes `Some(1)` instead of
/// death by signal.
fn emit_oom_trap(out: &mut String) {
    writeln!(out, "\nfunction ${OOM_TRAP_SYMBOL}(l %n) {{").unwrap();
    out.push_str("@start\n");
    out.push_str("\tcall $dprintf(w 2, l $oomfmt, l %n, ...)\n");
    out.push_str("\tcall $exit(w 1)\n");
    out.push_str("\thlt\n");
    out.push_str("}\n");
}

/// `getenv` per event, not cached: caching would need a mutable global data
/// symbol with no precedent in the emitter. Unset or empty prints nothing.
fn emit_trace_event(out: &mut String) {
    writeln!(out, "\nfunction ${TRACE_EVENT_SYMBOL}(l %fmt, l %n) {{").unwrap();
    out.push_str("@start\n");
    out.push_str("\t%e =l call $getenv(l $tracenv)\n");
    out.push_str("\t%unset =w ceql %e, 0\n");
    out.push_str("\tjnz %unset, @off, @set\n");
    out.push_str("@set\n");
    out.push_str("\t%c =w loadub %e\n");
    out.push_str("\tjnz %c, @on, @off\n");
    out.push_str("@on\n");
    out.push_str("\tcall $printf(l %fmt, l %n, ...)\n");
    out.push_str("\tret\n");
    out.push_str("@off\n");
    out.push_str("\tret\n");
    out.push_str("}\n");
}

fn emit_instr(
    out: &mut String,
    instr: &Instr,
    value_types: &[IrType],
    layouts: Layouts,
    ext_id: &mut u32,
    str_lits: &std::collections::HashMap<String, usize>,
) {
    match instr {
        Instr::Const(v, n) => {
            let w = width(ty_of(value_types, *v), layouts);
            writeln!(out, "\t{} ={w} copy {n}", val(*v))
        }
        Instr::StrLit(v, content) => {
            let idx = str_lits[content];
            writeln!(out, "\t{} =l copy $strd{idx}", val(*v))
        }
        Instr::ConstF(v, x) => {
            // QBE float constants carry an `s_`/`d_` prefix; Rust's `f64`
            // `Display` renders round-trippable text QBE parses (R14).
            let ty = ty_of(value_types, *v);
            let w = width(ty, layouts);
            let prefix = if matches!(ty, IrType::Float { bits: 32 }) {
                "s_"
            } else {
                "d_"
            };
            writeln!(out, "\t{} ={w} copy {prefix}{x}", val(*v))
        }
        Instr::Bin(v, op @ (BinOp::Shl | BinOp::Shr), a, b) => {
            // Type-directed like the comparison codegen: the result's own
            // signedness picks logical (`shr`) vs arithmetic (`sar`) right
            // shift; `shl` has one form. The hardware already masks the shift
            // count mod the register width (32 for `w`, 64 for `l`), which
            // only matches the *type* width for `w`/`l`-filling types (32/64
            // bits); a sub-word type (bits < 32) needs the count explicitly
            // masked to its own width first, or an over-shift would wrap at 32
            // instead of at the type's bit width (Rust `wrapping_shl`/`shr`
            // semantics for both literal and runtime counts).
            let ty = ty_of(value_types, *v);
            let w = width(ty, layouts);
            let signed = matches!(norm_scalar(ty), IrType::Int { signed: true, .. });
            let m = match op {
                BinOp::Shl => "shl",
                BinOp::Shr if signed => "sar",
                BinOp::Shr => "shr",
                _ => unreachable!("matched only Shl/Shr"),
            };
            let sub = sub_word(ty);
            let count = match sub {
                Some((bits, _)) => {
                    let masked = format!("%shamt{ext_id}");
                    *ext_id += 1;
                    // Mask the count mod the type's bit width, not the value
                    // mask used for canonicalization: e.g. a `u8`'s count
                    // masks to `bits - 1 = 7` (mod 8), not `255`.
                    writeln!(out, "\t{masked} =l and {}, {}", val(*b), bits - 1).unwrap();
                    masked
                }
                None => val(*b),
            };
            if let Some((bits, signed)) = sub {
                let tmp = format!("%bin{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{tmp} ={w} {m} {}, {count}", val(*a)).unwrap();
                emit_canonicalize(out, &val(*v), &tmp, w, bits, signed)
            } else {
                writeln!(out, "\t{} ={w} {m} {}, {count}", val(*v), val(*a))
            }
        }
        Instr::Bin(v, op, a, b) => {
            // The op runs at the result's register width; a sub-word result can
            // overflow its width, so canonicalize it (R15) via the shared point.
            let ty = ty_of(value_types, *v);
            let w = width(ty, layouts);
            let m = match op {
                BinOp::Add => "add",
                BinOp::Sub => "sub",
                BinOp::Mul => "mul",
                // `div` is emitted only for floats (no integer `/`, R16); it
                // runs at the operand's `s`/`d` width like the other arms.
                BinOp::Div => "div",
                BinOp::Rem if matches!(norm_scalar(ty), IrType::Int { signed: false, .. }) => {
                    "urem"
                }
                BinOp::Rem => "rem",
                BinOp::And => "and",
                BinOp::Or => "or",
                BinOp::Xor => "xor",
                BinOp::Shl | BinOp::Shr => unreachable!("handled in the arm above"),
            };
            if let Some((bits, signed)) = sub_word(ty) {
                let tmp = format!("%bin{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{tmp} ={w} {m} {}, {}", val(*a), val(*b)).unwrap();
                emit_canonicalize(out, &val(*v), &tmp, w, bits, signed)
            } else {
                writeln!(out, "\t{} ={w} {m} {}, {}", val(*v), val(*a), val(*b))
            }
        }
        Instr::Cmp(v, op, a, b) => {
            // Signedness and operand width come from the operand type (R10),
            // not the result (always `Bool`/`w`): `<`/`>` pick signed
            // (`cslt`/`csgt`) vs unsigned (`cult`/`cugt`); `=` is
            // signedness-agnostic (`ceq`). The mnemonic's width suffix is the
            // operand width.
            let operand = ty_of(value_types, *a);
            let ow = width(operand, layouts);
            let is_float = matches!(operand, IrType::Float { .. });
            let signed = matches!(norm_scalar(operand), IrType::Int { signed: true, .. });
            let w = width(ty_of(value_types, *v), layouts);
            // QBE's amd64 backend lowers `ceq{s,d}`/`clt{s,d}`/`cle{s,d}`/
            // `cne{s,d}` straight to `comis{s,d}` + `sete`/`setb`/`setbe`/
            // `setne`, but x86's unordered (NaN) result sets both ZF and CF,
            // so `eq`/`lt`/`le` report *true* for a NaN operand (wrong, an
            // ordered compare must be false on NaN) while `ne` reports
            // *false* (also wrong, `!=` must be true on NaN) instead of the
            // required IEEE result (`cgt{s,d}`/`cge{s,d}` use `seta`/`setae`,
            // which x86 clears on unordered, so those two stay correct as-is;
            // verified against emitted assembly). Work around it here rather
            // than in QBE: `a < b` and `a <= b` swap operands and reuse the
            // correct `cgt`/`cge` forms (`b > a`, `b >= a`); `a = b` ANDs
            // `ceq` with the ordered predicate `cod`/`cos` (false on NaN) to
            // mask the false positive; `a <> b` ORs `cne` with the unordered
            // predicate `cuo{s,d}` (true on NaN) to add the missing true
            // (R17, R21, RISK 1).
            if is_float && matches!(op, CmpOp::Lt) {
                return writeln!(out, "\t{} ={w} cgt{ow} {}, {}", val(*v), val(*b), val(*a))
                    .unwrap();
            }
            if is_float && matches!(op, CmpOp::Le) {
                return writeln!(out, "\t{} ={w} cge{ow} {}, {}", val(*v), val(*b), val(*a))
                    .unwrap();
            }
            if is_float && matches!(op, CmpOp::Eq) {
                let eq = format!("%cmp{ext_id}");
                *ext_id += 1;
                let ord = format!("%cmp{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{eq} ={w} ceq{ow} {}, {}", val(*a), val(*b)).unwrap();
                writeln!(out, "\t{ord} ={w} co{ow} {}, {}", val(*a), val(*b)).unwrap();
                return writeln!(out, "\t{} ={w} and {eq}, {ord}", val(*v)).unwrap();
            }
            if is_float && matches!(op, CmpOp::Ne) {
                let ne = format!("%cmp{ext_id}");
                *ext_id += 1;
                let uo = format!("%cmp{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{ne} ={w} cne{ow} {}, {}", val(*a), val(*b)).unwrap();
                writeln!(out, "\t{uo} ={w} cuo{ow} {}, {}", val(*a), val(*b)).unwrap();
                return writeln!(out, "\t{} ={w} or {ne}, {uo}", val(*v)).unwrap();
            }
            let m = match op {
                CmpOp::Eq => "ceq",
                CmpOp::Ne => "cne",
                CmpOp::Lt if signed => "cslt",
                CmpOp::Lt => "cult",
                CmpOp::Gt if is_float => "cgt",
                CmpOp::Gt if signed => "csgt",
                CmpOp::Gt => "cugt",
                CmpOp::Le if signed => "csle",
                CmpOp::Le => "cule",
                CmpOp::Ge if is_float => "cge",
                CmpOp::Ge if signed => "csge",
                CmpOp::Ge => "cuge",
            };
            writeln!(out, "\t{} ={w} {m}{ow} {}, {}", val(*v), val(*a), val(*b))
        }
        Instr::Call(ret, f, args) => {
            // A struct argument/return is spelled `:S` so QBE applies its
            // by-value C-ABI classification; the temporary is a pointer to
            // the aggregate on both sides.
            let a: Vec<String> = args
                .iter()
                .map(|x| {
                    format!(
                        "{} {}",
                        qbe_abi_ty(ty_of(value_types, *x), layouts),
                        val(*x)
                    )
                })
                .collect();
            match ret {
                Some(r) => {
                    let w = qbe_abi_ty(ty_of(value_types, *r), layouts);
                    writeln!(
                        out,
                        "\t{} ={w} call ${}({})",
                        val(*r),
                        qbe_name(f),
                        a.join(", ")
                    )
                }
                None => writeln!(out, "\tcall ${}({})", qbe_name(f), a.join(", ")),
            }
        }
        // Slice 7a (R4): materialize a function symbol as a `Code` handle; the
        // symbol is sanitized identically to a direct call site.
        Instr::FuncAddr(dst, sym) => {
            writeln!(out, "\t{} =l copy ${}", val(*dst), qbe_name(sym))
        }
        // Slice 7a (R4): an indirect call through a code-handle value. Mirrors
        // `Call` but the callee is `%fp`, not a `$sym`; `env` is not passed in
        // 7a (a non-capturing callee has no env parameter).
        Instr::CallIndirect(ret, fp, args) => {
            let a: Vec<String> = args
                .iter()
                .map(|x| {
                    format!(
                        "{} {}",
                        qbe_abi_ty(ty_of(value_types, *x), layouts),
                        val(*x)
                    )
                })
                .collect();
            match ret {
                Some(r) => {
                    let w = qbe_abi_ty(ty_of(value_types, *r), layouts);
                    writeln!(
                        out,
                        "\t{} ={w} call {}({})",
                        val(*r),
                        val(*fp),
                        a.join(", ")
                    )
                }
                None => writeln!(out, "\tcall {}({})", val(*fp), a.join(", ")),
            }
        }
        // `.` is type-directed on the operand's own `IrType` (same dispatch
        // shape as `Cmp`/`Shr`): signed decimal, unsigned decimal, `%g` float,
        // or `true`/`false` for `Bool`.
        //
        // Every `$printf` call below writes `...` last, which in QBE means "all
        // preceding arguments are fixed, zero variadic ones" -- the marker is
        // positional, `call $printf(l $fmt, ..., w %v)` is the form that says the
        // value is variadic. Wrong per C, but currently unobservable: on
        // amd64_sysv, arm64 and rv64 QBE emits identical code either way, and
        // `driver.rs` invokes `qbe` with no `-t`, so only the default
        // amd64_sysv target is ever built. It becomes a real wrong-output bug on
        // `arm64_apple`, whose ABI passes variadic arguments on the stack while
        // fixed ones go in registers: `printf` would read the stack while these
        // calls left the value in a register. Fix the marker position before
        // adding target selection. Same shape in `sooth_oom_trap`'s `dprintf`
        // and `sooth_trace_event`'s `printf`.
        Instr::Print(v) => match ty_of(value_types, *v) {
            // Slice 9: `Bool` is `IrType::Enum(BOOL_ENUM_ID)` now, still
            // routed to the same `$boolstrs` table -- `.`'s primitive `bool`
            // printable row is deleted in P2 (R6), not this phase, so `.` on
            // `Bool` must keep working identically through P1.
            IrType::Bool | IrType::Enum(BOOL_ENUM_ID) => {
                // No branch needed: widen the canonical 0/1 to an index into
                // the 2-entry `$boolstrs` pointer table and print the
                // selected string via `%s`.
                let idx = format!("%pidx{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{idx} =l extuw {}", val(*v)).unwrap();
                let off = format!("%poff{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{off} =l mul {idx}, 8").unwrap();
                let addr = format!("%paddr{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{addr} =l add $boolstrs, {off}").unwrap();
                let ptr = format!("%pptr{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{ptr} =l loadl {addr}").unwrap();
                writeln!(out, "\tcall $printf(l $sfmt, l {ptr}, ...)")
            }
            // A float always prints as a `d`: an `f32` widens first (`exts`)
            // since a variadic C call needs the explicit promotion QBE never
            // does implicitly.
            IrType::Float { bits: 32 } => {
                let d = format!("%pf{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{d} =d exts {}", val(*v)).unwrap();
                writeln!(out, "\tcall $printf(l $ffmt, d {d}, ...)")
            }
            IrType::Float { .. } => writeln!(out, "\tcall $printf(l $ffmt, d {}, ...)", val(*v)),
            // Signed prints `%ld`, unsigned prints `%lu` (the unsigned-decimal
            // fix: a high-bit `u64` must render as its unsigned value, not
            // reinterpreted negative). `printf`'s variadic ABI expects a full
            // 8-byte slot regardless of the value's own width, so a sub-64-bit
            // operand widens first, by its own signedness (the sign-extend
            // reads its already-canonical `w` bits, R15; the zero-extend is
            // exact since sub-word unsigned canonicalization already zeroed
            // the high bits).
            IrType::Int { bits: 64, signed } => {
                let fmt = if signed { "$fmt" } else { "$ufmt" };
                writeln!(out, "\tcall $printf(l {fmt}, l {}, ...)", val(*v))
            }
            IrType::Int { signed, .. } => {
                let fmt = if signed { "$fmt" } else { "$ufmt" };
                let ext = if signed { "extsw" } else { "extuw" };
                let w64 = format!("%pw{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{w64} =l {ext} {}", val(*v)).unwrap();
                writeln!(out, "\tcall $printf(l {fmt}, l {w64}, ...)")
            }
            // `usize` prints unsigned decimal (`%lu`), like a `u64`: the value
            // fills the `l` register on this target, so no widening is needed.
            IrType::Usize => writeln!(out, "\tcall $printf(l $ufmt, l {}, ...)", val(*v)),
            // `isize` prints signed decimal (`%ld`), like an `i64`: same
            // no-widening reasoning as `usize`, but routed to `$fmt`.
            IrType::Isize => writeln!(out, "\tcall $printf(l $fmt, l {}, ...)", val(*v)),
            IrType::Ptr => unreachable!("Ptr is not a printable scalar; checker rejects it"),
            // R9: `%.*s` with the carried length, not `%s`: R4 promises no
            // terminator, so the carried length is the only safe bound.
            // `cstr` below must rely on a terminator, having no length.
            IrType::Str => {
                let ptr = format!("%sptr{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{ptr} =l loadl {}", val(*v)).unwrap();
                let len_addr = format!("%slena{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{len_addr} =l add {}, {STR_LEN_OFFSET}", val(*v)).unwrap();
                let len = format!("%slen{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{len} =l loadl {len_addr}").unwrap();
                writeln!(out, "\tcall $printf(l $strfmt, l {len}, l {ptr}, ...)")
            }
            // `cstr` is already a bare NUL-terminated byte pointer, so it
            // prints exactly like a bool's selected string does: `%s`.
            IrType::Cstr => writeln!(out, "\tcall $printf(l $sfmt, l {}, ...)", val(*v)),
            IrType::OwnedCell(_) => {
                unreachable!("a cell is not a printable scalar; checker rejects it")
            }
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                unreachable!("an aggregate is not a printable scalar; checker rejects it (X6/M2)")
            }
            IrType::Code | IrType::Quotation(_) => {
                unreachable!("a quotation/code is not a printable scalar; checker rejects it")
            }
        },
        Instr::PtrOffset(dst, base, bytes) => {
            writeln!(out, "\t{} =l add {}, {bytes}", val(*dst), val(*base))
        }
        // `dst = base + index*stride` (R17): a `mul` of the runtime index by
        // the compile-time stride, then an `add` onto the aggregate base. Both
        // run at `l` width; `Ptr` stays opaque (no pointer-as-`u64` reasoning
        // leaks into the IR, only into this backend arm).
        Instr::ElemAddr(dst, base, index, stride) => {
            let off = format!("%eoff{ext_id}");
            *ext_id += 1;
            writeln!(out, "\t{off} =l mul {}, {stride}", val(*index)).unwrap();
            writeln!(out, "\t{} =l add {}, {off}", val(*dst), val(*base))
        }
        Instr::Alloc(dst, size, align) => {
            // A frame-local aggregate slot; QBE only offers alloc4/8/16, so a
            // size-0 (zero-field) struct still allocs a minimal slot.
            let op = alloc_op(*align);
            writeln!(out, "\t{} =l {op} {}", val(*dst), (*size).max(1))
        }
        Instr::Blit(src, dst, size) => {
            // QBE `blit src, dst, n` copies n bytes src -> dst (verified). A
            // zero-field struct never emits a blit (guarded in the frontend).
            writeln!(out, "\tblit {}, {}, {size}", val(*src), val(*dst))
        }
        Instr::FieldLoad(dst, ptr) => {
            let (w, op) = field_load_op(ty_of(value_types, *dst), layouts);
            writeln!(out, "\t{} ={w} {op} {}", val(*dst), val(*ptr))
        }
        Instr::FieldStore(ptr, v) => {
            let op = field_store_op(ty_of(value_types, *v), layouts);
            writeln!(out, "\t{op} {}, {}", val(*v), val(*ptr))
        }
        // `src`'s carried length is the descriptor's second word: this file
        // owns the offset (`STR_LEN_OFFSET`) because `emit_str_literal` is
        // what wrote the descriptor's `{ptr, len}` shape in the first place
        // (the IR states intent, not a byte offset).
        Instr::StrLen(dst, src) => {
            let addr = format!("%straddr{ext_id}");
            *ext_id += 1;
            writeln!(out, "\t{addr} =l add {}, {STR_LEN_OFFSET}", val(*src)).unwrap();
            writeln!(out, "\t{} =l loadl {addr}", val(*dst))
        }
        // `src`'s bytes pointer is the descriptor's first word, so no offset
        // is needed: `src`'s own address already points at it.
        Instr::StrPtr(dst, src) => {
            writeln!(out, "\t{} =l loadl {}", val(*dst), val(*src))
        }
        Instr::Load(dst, ptr) => {
            // The load width follows the destination's `IrType` (R20): a float
            // slot loads at its `s`/`d` width so its bits re-enter as a true
            // float; every other slot loads the full 8-byte `l`.
            let (w, op) = match ty_of(value_types, *dst) {
                IrType::Float { bits: 32 } => ("s", "loads"),
                IrType::Float { .. } => ("d", "loadd"),
                _ => ("l", "loadl"),
            };
            writeln!(out, "\t{} ={w} {op} {}", val(*dst), val(*ptr))
        }
        Instr::Store(ptr, v) => {
            // A float slot stores at its `s`/`d` width (R20), symmetric with the
            // float load; an `f32` writes 4 of the 8 slot bytes (Q2). Otherwise
            // the 8-byte buffer slot is an `l` sink (R4): any `w`-width value
            // (`Bool`, or an integer with `bits <= 32`) is widened to `l` first.
            // A signed integer sign-extends (its `w` register already holds
            // canonical bits, R15); `Bool`/unsigned zero-extend.
            let ty = ty_of(value_types, *v);
            match ty {
                IrType::Float { bits: 32 } => writeln!(out, "\tstores {}, {}", val(*v), val(*ptr)),
                IrType::Float { .. } => writeln!(out, "\tstored {}, {}", val(*v), val(*ptr)),
                _ if width(ty, layouts) == "w" => {
                    let signed = matches!(ty, IrType::Int { signed: true, .. });
                    let ext_op = if signed { "extsw" } else { "extuw" };
                    let ext = format!("%ext{ext_id}");
                    *ext_id += 1;
                    writeln!(out, "\t{ext} =l {ext_op} {}", val(*v)).unwrap();
                    writeln!(out, "\tstorel {}, {}", ext, val(*ptr))
                }
                _ => writeln!(out, "\tstorel {}, {}", val(*v), val(*ptr)),
            }
        }
        // Slice 10c (R-P3-2): a scalar enum's value already *is* its
        // discriminant, in the same `w` register a `u32` occupies, so reading
        // the tag is a relabel. QBE coalesces the copy, so the emitted machine
        // code is the same as if the enum value had been used directly.
        Instr::Tag(dst, src) => {
            writeln!(out, "\t{} =w copy {}", val(*dst), val(*src))
        }
        Instr::Conv(dst, src) => emit_conv(out, *dst, *src, value_types, layouts, ext_id),
        Instr::Phi(r, arms) => {
            let a: Vec<String> = arms
                .iter()
                .map(|(b, v)| format!("{} {}", label(*b), val(*v)))
                .collect();
            let w = width(ty_of(value_types, *r), layouts);
            writeln!(out, "\t{} ={w} phi {}", val(*r), a.join(", "))
        }
    }
    .unwrap();
}

fn emit_term(out: &mut String, term: &Terminator) {
    match term {
        Terminator::Ret(Some(v)) => writeln!(out, "\tret {}", val(*v)),
        Terminator::Ret(None) => writeln!(out, "\tret"),
        Terminator::Jnz(c, t, e) => {
            writeln!(out, "\tjnz {}, {}, {}", val(*c), label(*t), label(*e))
        }
        Terminator::Jmp(b) => writeln!(out, "\tjmp {}", label(*b)),
    }
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Line;
    use crate::ast::Type;
    use crate::check::check;
    use crate::ir::{lower, lower_line, Arrays, Cells, Enums, IrModule, Refs, Registries, Structs};
    use crate::lexer::lex;
    use crate::parser::{parse, parse_line};
    use std::collections::HashMap;

    fn empty_layouts() -> Layouts<'static> {
        Layouts {
            structs: &[],
            enums: &[],
            quot_sigs: &[],
        }
    }

    fn emit_src(src: &str) -> String {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        let ir = lower(&module).unwrap();
        emit(&ir).unwrap()
    }

    fn emit_line(src: &str, entry_depth: usize) -> String {
        let tokens = lex(src).unwrap();
        let terms = match parse_line(&tokens).unwrap() {
            Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let entry_types = vec![Type::I64; entry_depth];
        let (func, _q, _m, _) = lower_line(
            0,
            &terms,
            entry_depth,
            &entry_types,
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        );
        emit(&IrModule {
            funcs: vec![func],
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn emit_word_name_with_hyphen_sanitizes_qbe_symbol() {
        // A word name containing `-` (a legal identifier-continuation
        // character in the lexer, e.g. the S8 dogfood's `shift-x`) is not a
        // valid QBE global symbol; it is sanitized identically at the
        // function definition and its call site. `-` is codepoint 0x2d, so
        // it escapes to `.2d.` (see `qbe_name`), not a bare `_` -- a bare `_`
        // would collide with any other name whose own non-alphanumeric
        // characters also collapsed to one underscore.
        let il = emit_src(
            ": shift-x ( i64 -- i64 ) | n | n 1 + ;
            : main ( -- ) 5 shift-x . ;",
        );
        assert!(!il.contains("shift-x"), "raw hyphenated name leaked: {il}");
        assert!(
            il.contains("$shift.2d.x"),
            "expected the injective sanitized symbol: {il}"
        );
    }

    #[test]
    fn qbe_name_distinct_operator_names_never_collide() {
        // Regression: qbe_name used to replace every non-alphanumeric
        // character with a bare `_`, so any two names built entirely of
        // non-alphanumeric characters of the same length collapsed onto the
        // identical symbol -- `+` and `-` both sanitized to `_`. This is the
        // exact set 8a's overload table is about to make dispatchable, so
        // every pair in it must be checked, not just one.
        let ops = [
            "+",
            "-",
            "*",
            "/",
            "mod",
            "and",
            "or",
            "xor",
            "not",
            "shl",
            "shr",
            "=",
            "<",
            ">",
            "<=",
            ">=",
            "<>",
            "max",
            "max-total",
            ".",
        ];
        let sanitized: Vec<String> = ops.iter().map(|op| qbe_name(op).into_owned()).collect();
        for i in 0..ops.len() {
            for j in (i + 1)..ops.len() {
                assert_ne!(
                    sanitized[i], sanitized[j],
                    "`{}` and `{}` collide on the symbol `{}`",
                    ops[i], ops[j], sanitized[i]
                );
            }
        }
    }

    #[test]
    fn qbe_name_plus_and_minus_no_longer_collide() {
        // The exact reproduction that broke before the fix: `~` and `?`
        // (ordinary symbolic word names, nothing to do with operator
        // overloading) both sanitized to the bare symbol `_`.
        assert_ne!(qbe_name("~"), qbe_name("?"));
        assert_ne!(qbe_name("+"), qbe_name("-"));
    }

    #[test]
    fn emit_square_contains_mul_and_ret() {
        let il = emit_src(": sq ( i64 -- i64 ) | n | n n * ;");
        assert!(il.contains("mul"));
        assert!(il.contains("ret "));
    }

    #[test]
    fn emit_print_uses_printf_and_fmt() {
        let il = emit_src(": w ( i64 -- ) . ;");
        assert!(il.contains("data $fmt = { b \"%ld\\n\", b 0 }"));
        assert!(il.contains("call $printf(l $fmt,"));
        assert!(il.contains(", ...)"));
    }

    #[test]
    fn emit_print_on_float_uses_ffmt_and_d_arg() {
        // `.` on an `f64` prints via the `%g` float format, passing the value
        // as a `d`.
        let il = emit_src(": w ( f64 -- ) . ;");
        assert!(
            il.contains("data $ffmt = { b \"%g\\n\", b 0 }"),
            "unexpected IL: {il}"
        );
        assert!(
            il.contains("call $printf(l $ffmt, d "),
            "unexpected IL: {il}"
        );
    }

    #[test]
    fn emit_print_on_f32_widens_before_calling_printf() {
        // `.` on an `f32` widens to `d` (`exts`) before the call, since a
        // variadic C call needs the explicit promotion QBE never does
        // implicitly.
        let il = emit_src(": w ( -- ) 1.5 >f32 . ;");
        assert!(il.contains("=d exts"), "unexpected IL: {il}");
        assert!(
            il.contains("call $printf(l $ffmt, d "),
            "unexpected IL: {il}"
        );
    }

    #[test]
    fn emit_print_on_unsigned_uses_ufmt() {
        // `.` on a `u64` prints unsigned decimal (`$ufmt`/`%lu`), the fix for
        // the high-bit-set misprint-as-negative gap.
        let il = emit_src(": w ( -- ) 1 >u64 . ;");
        assert!(
            il.contains("data $ufmt = { b \"%lu\\n\", b 0 }"),
            "unexpected IL: {il}"
        );
        assert!(
            il.contains("call $printf(l $ufmt, l "),
            "unexpected IL: {il}"
        );
    }

    #[test]
    fn norm_scalar_ww_follows_word_width_for_both_size_types() {
        // Neither size type carries a literal 64; both derive bits from the
        // `word_width` parameter.
        assert_eq!(
            norm_scalar_ww(IrType::Usize, 4),
            IrType::Int {
                bits: 32,
                signed: false
            }
        );
        assert_eq!(
            norm_scalar_ww(IrType::Isize, 4),
            IrType::Int {
                bits: 32,
                signed: true
            }
        );
        assert_eq!(
            norm_scalar_ww(IrType::Usize, 8),
            IrType::Int {
                bits: 64,
                signed: false
            }
        );
        assert_eq!(
            norm_scalar_ww(IrType::Isize, 8),
            IrType::Int {
                bits: 64,
                signed: true
            }
        );
    }

    #[test]
    fn emit_print_on_isize_uses_fmt_signed() {
        // `.` on an `isize` prints signed decimal (`$fmt`/`%ld`), unlike
        // `usize`'s `$ufmt`.
        let il = emit_src(": w ( -- ) 1 >isize . ;");
        assert!(
            il.contains("call $printf(l $fmt, l "),
            "unexpected IL: {il}"
        );
    }

    #[test]
    fn emit_print_on_subword_unsigned_widens_via_extuw() {
        // A sub-64-bit unsigned operand (`u8`) must zero-extend to a full
        // 8-byte slot before the variadic call, reusing `$ufmt`.
        let il = emit_src(": w ( -- ) 200 >u8 . ;");
        assert!(il.contains("=l extuw"), "unexpected IL: {il}");
        assert!(
            il.contains("call $printf(l $ufmt, l "),
            "unexpected IL: {il}"
        );
    }

    #[test]
    fn emit_print_on_subword_signed_widens_via_extsw() {
        let il = emit_src(": w ( -- ) 5 >i32 . ;");
        assert!(il.contains("=l extsw"), "unexpected IL: {il}");
        assert!(
            il.contains("call $printf(l $fmt, l "),
            "unexpected IL: {il}"
        );
    }

    #[test]
    fn emit_print_on_bool_indexes_boolstrs_via_sfmt() {
        // `.` on a `bool` selects `$true_str`/`$false_str` through the
        // 2-entry `$boolstrs` pointer table, no branch, printed via `%s`.
        let il = emit_src(": w ( -- ) true . ;");
        assert!(
            il.contains("data $boolstrs = { l $false_str, l $true_str }"),
            "unexpected IL: {il}"
        );
        assert!(
            il.contains("data $true_str = { b \"true\\n\", b 0 }"),
            "unexpected IL: {il}"
        );
        assert!(
            il.contains("data $false_str = { b \"false\\n\", b 0 }"),
            "unexpected IL: {il}"
        );
        assert!(il.contains("add $boolstrs,"), "unexpected IL: {il}");
        assert!(
            il.contains("call $printf(l $sfmt, l "),
            "unexpected IL: {il}"
        );
    }

    #[test]
    fn emit_print_of_str_uses_precision_format() {
        // R9/criterion 8: `.` on a `str` prints via `%.*s`; see `Instr::Print`'s
        // `IrType::Str` arm for why.
        let il = emit_src(": w ( -- ) \"hi\" . ;");
        assert!(
            il.contains("data $strfmt = { b \"%.*s\", b 0 }"),
            "unexpected IL: {il}"
        );
        assert!(
            il.contains("call $printf(l $strfmt, l "),
            "unexpected IL: {il}"
        );
    }

    #[test]
    fn emit_print_of_cstr_uses_string_format() {
        // R9/criterion 9: `.` on a `cstr` prints via plain `%s` (`$sfmt`),
        // since it has no carried length to prefer.
        let il = emit_src(": w ( -- ) \"hi\" cstr . ;");
        assert!(
            il.contains("call $printf(l $sfmt, l "),
            "unexpected IL: {il}"
        );
        assert!(
            !il.contains("call $printf(l $strfmt"),
            "unexpected IL: {il}"
        );
    }

    #[test]
    fn emit_str_literal_writes_uncounted_terminator_and_descriptor_shape() {
        // R6: the static byte data ends with a NUL the descriptor's length
        // word does not count (`STR_LEN_OFFSET` reads that word); "hi" is
        // b 104, b 105 plus the uncounted b 0, and the descriptor's length is
        // 2, not 3.
        let il = emit_src(": w ( -- str ) \"hi\" ;");
        assert!(
            il.contains("data $strb0 = { b 104, b 105, b 0 }"),
            "unexpected IL: {il}"
        );
        assert!(
            il.contains("data $strd0 = { l $strb0, l 2 }"),
            "unexpected IL: {il}"
        );
    }

    #[test]
    fn emit_float_slot_round_trips_with_float_load_store() {
        // A carried `f64` slot loads/stores with the float ops (R20), so its
        // bits re-enter as a true float rather than a stale `i64`.
        let tokens = lex("dup").unwrap();
        let terms = match parse_line(&tokens).unwrap() {
            Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let f64_ty = Type::from_name("f64").unwrap();
        let (func, _q, _m, _) = lower_line(
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
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        );
        let il = emit(&IrModule {
            funcs: vec![func],
            ..Default::default()
        })
        .unwrap();
        assert!(il.contains("loadd "), "expected a float load: {il}");
        assert!(il.contains("stored "), "expected a float store: {il}");
        assert!(
            !il.contains("loadl "),
            "a float slot never uses loadl: {il}"
        );
    }

    #[test]
    fn emit_if_has_jnz_and_phi() {
        let il = emit_src(": w ( bool -- i64 ) ~[ 1 ] ~[ 2 ] if ;");
        assert!(il.contains("jnz "));
        assert!(il.contains("phi "));
    }

    #[test]
    fn emit_bounds_trap_helper_prints_and_exits() {
        // The module always emits the OOB trap helper, which writes the
        // located message to stderr (`dprintf` fd 2 + `$oobfmt`) and `exit`s
        // nonzero, ending in `hlt` so it aborts rather than falls through.
        let il = emit_src(": w ( [i64 4] usize -- i64 ) | a i | &a i &> @ ;");
        assert!(il.contains("$sooth_oob_trap("), "missing trap helper: {il}");
        assert!(
            il.contains("data $oobfmt"),
            "missing trap message data: {il}"
        );
        assert!(
            il.contains("$dprintf(w 2,"),
            "trap must write to stderr: {il}"
        );
        assert!(il.contains("$exit(w 1)"), "trap must exit nonzero: {il}");
        assert!(il.contains("hlt"), "trap must abort (hlt): {il}");
        // The runtime element access guards it with a branch to the trap symbol.
        assert!(il.contains("jnz "), "runtime index must be guarded: {il}");
        assert!(
            il.contains("call $sooth_oob_trap("),
            "guard must call the trap helper: {il}"
        );
    }

    #[test]
    fn emit_main_becomes_sooth_main() {
        let il = emit_src(": main ( -- ) 5 . ;");
        assert!(il.contains("$sooth_main"));
        assert!(!il.contains("$main("));
    }

    #[test]
    fn emit_bool_value_uses_w_width() {
        let il = emit_src(": w ( -- bool ) true ;");
        assert!(il.contains("=w copy 1"), "unexpected IL: {il}");
    }

    #[test]
    fn emit_comparison_line_stores_bool_via_extension() {
        // `5 3 u>` from D=0 leaves a 32-bit flag on top; the line-wrapper
        // epilogue must widen it (`extuw`) before the fixed 8-byte `storel`
        // (R4/RK1). Slice 10c: the *primitive*, since `>` is a `lib/` word now
        // and this helper lowers a bare line with no word environment.
        let il = emit_line("5 3 u>", 0);
        assert!(il.contains("=w csgtl"), "unexpected IL: {il}");
        assert!(il.contains("extuw"), "expected a w->l extension: {il}");
        assert!(il.contains("storel"), "expected a storel: {il}");
    }

    #[test]
    fn emit_wrapper_signature_takes_stack_and_top() {
        let il = emit_line("2 3 +", 0);
        assert!(
            il.contains("export function l $sooth_line_0(l %v0, l %v1)"),
            "unexpected signature: {il}"
        );
    }

    #[test]
    fn emit_line_wrapper_has_load_and_store() {
        // `+` from a carried depth of 2 loads the two slots and stores the result.
        let il = emit_line("+", 2);
        assert!(il.contains("loadl "), "expected a load: {il}");
        assert!(il.contains("storel "), "expected a store: {il}");
    }

    fn int(bits: u8, signed: bool) -> IrType {
        IrType::Int { bits, signed }
    }

    /// Emit a single-block function over hand-built value types and instrs,
    /// returning `v2` (the result of a binary/compare op). Hand-built types
    /// isolate the bare sub-word/unsigned codegen path per operand pairing.
    fn emit_binary(operand: IrType, result: IrType, instr: Instr) -> String {
        let func = IrFunc {
            name: "t".to_string(),
            params: vec![],
            ret: Some(result),
            blocks: vec![crate::ir::Block {
                id: crate::ir::BlockId(0),
                instrs: vec![Instr::Const(Value(0), 5), Instr::Const(Value(1), 3), instr],
                term: Terminator::Ret(Some(Value(2))),
            }],
            value_types: vec![operand, operand, result],
        };
        emit(&IrModule {
            funcs: vec![func],
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn qbe_width_u8_is_w_expected() {
        assert_eq!(width(int(8, false), empty_layouts()), "w");
        assert_eq!(width(int(16, true), empty_layouts()), "w");
        assert_eq!(width(int(32, false), empty_layouts()), "w");
    }

    #[test]
    fn qbe_width_i64_is_l_expected() {
        assert_eq!(width(int(64, true), empty_layouts()), "l");
        assert_eq!(width(int(64, false), empty_layouts()), "l");
    }

    #[test]
    fn qbe_width_float_is_s_and_d_expected() {
        assert_eq!(width(IrType::Float { bits: 32 }, empty_layouts()), "s");
        assert_eq!(width(IrType::Float { bits: 64 }, empty_layouts()), "d");
    }

    #[test]
    fn emit_float_literal_uses_d_prefix() {
        let il = emit_src(": w ( -- f64 ) 3.14 ;");
        assert!(il.contains("=d copy d_3.14"), "unexpected IL: {il}");
    }

    #[test]
    fn emit_float_add_runs_at_d_width() {
        let f64_ty = IrType::Float { bits: 64 };
        let il = emit_binary(
            f64_ty,
            f64_ty,
            Instr::Bin(Value(2), BinOp::Add, Value(0), Value(1)),
        );
        assert!(il.contains("=d add"), "expected a d-width add: {il}");
        assert!(!il.contains("and"), "floats never canonicalize: {il}");
    }

    #[test]
    fn emit_float_div_emits_div() {
        let f32_ty = IrType::Float { bits: 32 };
        let il = emit_binary(
            f32_ty,
            f32_ty,
            Instr::Bin(Value(2), BinOp::Div, Value(0), Value(1)),
        );
        assert!(il.contains("=s div"), "expected an s-width div: {il}");
    }

    #[test]
    fn emit_float_lt_swaps_to_ordered_gt() {
        // `<` on `f64` operands does NOT emit `cltd`: QBE's amd64 backend
        // lowers `cltd` to `comisd`+`setb`, which x86 sets on an unordered
        // (NaN) operand too, so it would report `NaN < x` as true. Swapping
        // operands and reusing `cgtd` (`comisd`+`seta`, which x86 clears on
        // unordered) keeps the compare false against NaN (R17, RISK 1).
        let il = emit_binary(
            IrType::Float { bits: 64 },
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Lt, Value(0), Value(1)),
        );
        assert!(
            il.contains("=w cgtd %v1, %v0"),
            "expected a swapped ordered compare: {il}"
        );
    }

    #[test]
    fn emit_float_eq_masks_unordered_with_cod() {
        // `=` on floats does NOT rely on a bare `ceqd`: QBE's amd64 backend
        // lowers `ceqd` to `comisd`+`sete`, which x86 also sets on an
        // unordered (NaN) operand, so it would report `NaN = NaN` as true.
        // ANDing with `cod` (ordered, false on NaN) masks that false positive
        // so `x = x` is a valid NaN test (R17/D3, RISK 1).
        let il = emit_binary(
            IrType::Float { bits: 64 },
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Eq, Value(0), Value(1)),
        );
        assert!(il.contains("ceqd"), "expected an eq compare: {il}");
        assert!(il.contains("cod"), "expected an ordered mask: {il}");
        assert!(il.contains("and"), "expected the eq/ordered AND: {il}");
    }

    #[test]
    fn emit_float_le_swaps_to_ordered_ge() {
        // `<=` on floats reuses the already-NaN-correct `cge` form (`a <= b`
        // === `b >= a`), the same swap trick as `<` reusing `cgt` (R21).
        let il = emit_binary(
            IrType::Float { bits: 64 },
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Le, Value(0), Value(1)),
        );
        assert!(
            il.contains("=w cged %v1, %v0"),
            "expected a swapped ordered compare: {il}"
        );
    }

    #[test]
    fn emit_float_ge_uses_direct_cge_no_fix_needed() {
        // `>=` needs no NaN workaround: x86's `setae` (used by `cge{s,d}`)
        // already clears on an unordered operand (R21).
        let il = emit_binary(
            IrType::Float { bits: 64 },
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Ge, Value(0), Value(1)),
        );
        assert!(
            il.contains("=w cged %v0, %v1"),
            "expected a direct compare: {il}"
        );
    }

    #[test]
    fn emit_float_ne_ors_unordered_with_cuo() {
        // `<>` on floats does NOT rely on a bare `cned`: x86's `setne` (used
        // by `cne{s,d}`) is *false* on an unordered operand, but IEEE `!=`
        // must be *true* when either operand is NaN. ORing with `cuod`
        // (unordered, true on NaN) adds the missing true (R21, RISK 1).
        let il = emit_binary(
            IrType::Float { bits: 64 },
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Ne, Value(0), Value(1)),
        );
        assert!(il.contains("cned"), "expected a ne compare: {il}");
        assert!(il.contains("cuod"), "expected an unordered mask: {il}");
        assert!(il.contains(" or "), "expected the ne/unordered OR: {il}");
    }

    #[test]
    fn emit_cmp_le_signed_uses_csle() {
        let il = emit_binary(
            int(32, true),
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Le, Value(0), Value(1)),
        );
        assert!(il.contains("cslew"), "expected a signed compare: {il}");
    }

    #[test]
    fn emit_cmp_le_unsigned_uses_cule() {
        let il = emit_binary(
            int(32, false),
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Le, Value(0), Value(1)),
        );
        assert!(il.contains("culew"), "expected an unsigned compare: {il}");
    }

    #[test]
    fn emit_cmp_ge_signed_uses_csge() {
        let il = emit_binary(
            int(32, true),
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Ge, Value(0), Value(1)),
        );
        assert!(il.contains("csgew"), "expected a signed compare: {il}");
    }

    #[test]
    fn emit_cmp_ge_unsigned_uses_cuge() {
        let il = emit_binary(
            int(32, false),
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Ge, Value(0), Value(1)),
        );
        assert!(il.contains("cugew"), "expected an unsigned compare: {il}");
    }

    #[test]
    fn emit_cmp_ne_is_sign_agnostic() {
        // `<>` on integers is sign-agnostic, same as `=` (R21).
        let il = emit_binary(
            int(32, true),
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Ne, Value(0), Value(1)),
        );
        assert!(il.contains("=w cnew"), "expected a ne compare: {il}");
    }

    #[test]
    fn emit_cmp_signed_uses_cslt() {
        let il = emit_binary(
            int(32, true),
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Lt, Value(0), Value(1)),
        );
        assert!(il.contains("csltw"), "expected a signed compare: {il}");
    }

    #[test]
    fn emit_cmp_unsigned_uses_cult() {
        let il = emit_binary(
            int(32, false),
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Lt, Value(0), Value(1)),
        );
        assert!(il.contains("cultw"), "expected an unsigned compare: {il}");
    }

    #[test]
    fn emit_unsigned_mod_uses_urem() {
        let u32_ty = int(32, false);
        let il = emit_binary(
            u32_ty,
            u32_ty,
            Instr::Bin(Value(2), BinOp::Rem, Value(0), Value(1)),
        );
        assert!(il.contains("urem"), "expected an unsigned rem: {il}");
    }

    #[test]
    fn emit_signed_mod_uses_rem() {
        let i32_ty = int(32, true);
        let il = emit_binary(
            i32_ty,
            i32_ty,
            Instr::Bin(Value(2), BinOp::Rem, Value(0), Value(1)),
        );
        assert!(il.contains(" rem "), "expected a signed rem: {il}");
        assert!(!il.contains("urem"), "unexpected urem: {il}");
    }

    /// Emit a single-block function `src (v0) -> Conv -> v1`, returning the IL.
    /// Hand-built types isolate the bare conversion codegen path per cell,
    /// rather than needing a matching Sooth program for every width/sign pair.
    fn emit_conv_il(src_ty: IrType, dst_ty: IrType) -> String {
        let src_const = if matches!(src_ty, IrType::Float { .. }) {
            Instr::ConstF(Value(0), 5.0)
        } else {
            Instr::Const(Value(0), 5)
        };
        let func = IrFunc {
            name: "t".to_string(),
            params: vec![],
            ret: Some(dst_ty),
            blocks: vec![crate::ir::Block {
                id: crate::ir::BlockId(0),
                instrs: vec![src_const, Instr::Conv(Value(1), Value(0))],
                term: Terminator::Ret(Some(Value(1))),
            }],
            value_types: vec![src_ty, dst_ty],
        };
        emit(&IrModule {
            funcs: vec![func],
            ..Default::default()
        })
        .unwrap()
    }

    fn f32() -> IrType {
        IrType::Float { bits: 32 }
    }

    fn f64() -> IrType {
        IrType::Float { bits: 64 }
    }

    #[test]
    fn emit_conv_signed_int_to_float_uses_swtof_sltof() {
        // i32 -> f64 reads the `w` source as signed; i64 -> f32 reads `l`.
        assert!(
            emit_conv_il(int(32, true), f64()).contains("=d swtof"),
            "expected swtof to double"
        );
        assert!(
            emit_conv_il(int(64, true), f32()).contains("=s sltof"),
            "expected sltof to single"
        );
    }

    #[test]
    fn emit_conv_unsigned_int_to_float_uses_uwtof_ultof() {
        // A sub-word unsigned source rides its canonical `w` carrier (uwtof).
        assert!(
            emit_conv_il(int(8, false), f64()).contains("=d uwtof"),
            "expected uwtof to double"
        );
        assert!(
            emit_conv_il(int(64, false), f32()).contains("=s ultof"),
            "expected ultof to single"
        );
    }

    #[test]
    fn emit_conv_float_widen_is_exts() {
        // f32 >f64 is the exact single->double extend.
        let il = emit_conv_il(f32(), f64());
        assert!(il.contains("=d exts"), "expected an exts: {il}");
    }

    #[test]
    fn emit_conv_float_narrow_is_truncd() {
        // f64 >f32 rounds to nearest via truncd.
        let il = emit_conv_il(f64(), f32());
        assert!(il.contains("=s truncd"), "expected a truncd: {il}");
    }

    #[test]
    fn emit_conv_float_to_int_truncates_toward_zero() {
        // f64 >i64 truncates toward zero (dtosi to the `l` carrier); f32 >i32
        // uses stosi to the `w` carrier.
        assert!(
            emit_conv_il(f64(), int(64, true)).contains("=l dtosi"),
            "expected dtosi to long"
        );
        assert!(
            emit_conv_il(f32(), int(32, true)).contains("=w stosi"),
            "expected stosi to word"
        );
    }

    #[test]
    fn emit_conv_float_to_unsigned_int_uses_toui() {
        // An unsigned int target selects the `*toui` mnemonic.
        let il = emit_conv_il(f64(), int(64, false));
        assert!(il.contains("=l dtoui"), "expected dtoui: {il}");
    }

    #[test]
    fn emit_conv_float_to_subword_int_canonicalizes() {
        // f64 >u8 truncates to the `w` carrier then masks to the low byte (R15).
        let il = emit_conv_il(f64(), int(8, false));
        assert!(
            il.contains("dtoui") || il.contains("dtosi"),
            "expected a float->int trunc: {il}"
        );
        assert!(
            il.contains("and") && il.contains("255"),
            "expected a u8 mask after the trunc: {il}"
        );
    }

    #[test]
    fn emit_conv_narrow_truncates_and_canonicalizes() {
        // i64 -> u8 keeps the low byte via the unsigned canonicalization mask.
        let il = emit_conv_il(int(64, true), int(8, false));
        assert!(
            il.contains("and") && il.contains("255"),
            "expected a low-byte mask: {il}"
        );
    }

    #[test]
    fn emit_conv_signed_widen_sign_extends() {
        // i16 -> i64 sign-extends from the source width.
        let il = emit_conv_il(int(16, true), int(64, true));
        assert!(il.contains("=l extsh"), "expected a sign-extend: {il}");
    }

    #[test]
    fn emit_conv_unsigned_widen_zero_extends() {
        // u8 -> u32 zero-extends by the (unsigned) source signedness.
        let il = emit_conv_il(int(8, false), int(32, false));
        assert!(il.contains("=w extub"), "expected a zero-extend: {il}");
    }

    #[test]
    fn emit_conv_signed_widen_to_unsigned_subword_canonicalizes() {
        // i8 -> u16: extsb sign-extends into bits the target (u16) requires to
        // be zero, so the widen must be re-canonicalized to an unsigned mask
        // rather than trusted as-is (this is the dirty-high-bits cell).
        let il = emit_conv_il(int(8, true), int(16, false));
        assert!(
            il.contains("extsb"),
            "expected the source-signed extend: {il}"
        );
        assert!(
            il.contains("and") && il.contains("65535"),
            "expected a u16 mask after the extend: {il}"
        );
    }

    #[test]
    fn emit_conv_same_width_is_relabel() {
        // i32 >u32 fills its register either way: a pure bit relabel (`copy`).
        let il = emit_conv_il(int(32, true), int(32, false));
        assert!(il.contains("=w copy"), "expected a copy relabel: {il}");
        assert!(
            !il.contains("ext"),
            "a same-width relabel extends nothing: {il}"
        );
    }

    #[test]
    fn emit_bitwise_and_or_xor_use_matching_qbe_mnemonics() {
        let i32_ty = int(32, true);
        let il = emit_binary(
            i32_ty,
            i32_ty,
            Instr::Bin(Value(2), BinOp::And, Value(0), Value(1)),
        );
        assert!(il.contains("=w and"), "expected an and: {il}");

        let il = emit_binary(
            i32_ty,
            i32_ty,
            Instr::Bin(Value(2), BinOp::Or, Value(0), Value(1)),
        );
        assert!(il.contains("=w or"), "expected an or: {il}");

        let il = emit_binary(
            i32_ty,
            i32_ty,
            Instr::Bin(Value(2), BinOp::Xor, Value(0), Value(1)),
        );
        assert!(il.contains("=w xor"), "expected a xor: {il}");
    }

    #[test]
    fn emit_bitwise_subword_and_or_xor_do_not_dirty_canonical_form() {
        // Two already-canonical sub-word operands stay canonical through
        // `and`/`or`/`xor` (bitwise ops preserve consistent high bits); the
        // shared canonicalization mask is a no-op here but still applied
        // uniformly through the same single point as every other sub-word Bin.
        let u8 = int(8, false);
        let il = emit_binary(u8, u8, Instr::Bin(Value(2), BinOp::And, Value(0), Value(1)));
        assert!(il.contains("and"), "expected the and: {il}");
        assert!(
            il.contains("255"),
            "expected the shared u8 canonicalization mask: {il}"
        );
    }

    #[test]
    fn emit_not_xors_with_neg1_and_canonicalizes_unsigned() {
        let u8 = int(8, false);
        let func = IrFunc {
            name: "t".to_string(),
            params: vec![],
            ret: Some(u8),
            blocks: vec![crate::ir::Block {
                id: crate::ir::BlockId(0),
                instrs: vec![
                    Instr::Const(Value(0), 5),
                    Instr::Const(Value(1), -1),
                    Instr::Bin(Value(2), BinOp::Xor, Value(0), Value(1)),
                ],
                term: Terminator::Ret(Some(Value(2))),
            }],
            value_types: vec![u8, u8, u8],
        };
        let il = emit(&IrModule {
            funcs: vec![func],
            ..Default::default()
        })
        .unwrap();
        assert!(il.contains("copy -1"), "expected the -1 const: {il}");
        assert!(il.contains("xor"), "expected the xor: {il}");
        assert!(
            il.contains("and") && il.contains("255"),
            "expected the u8 canonicalization mask after xor-with-neg1: {il}"
        );
    }

    #[test]
    fn emit_shr_signed_uses_sar_unsigned_uses_shr() {
        let il = emit_binary(
            int(32, true),
            int(32, true),
            Instr::Bin(Value(2), BinOp::Shr, Value(0), Value(1)),
        );
        assert!(il.contains("=w sar"), "expected sar for signed: {il}");

        let il = emit_binary(
            int(32, false),
            int(32, false),
            Instr::Bin(Value(2), BinOp::Shr, Value(0), Value(1)),
        );
        assert!(il.contains("=w shr"), "expected shr for unsigned: {il}");
    }

    #[test]
    fn emit_shl_uses_shl_mnemonic() {
        let il = emit_binary(
            int(32, true),
            int(32, true),
            Instr::Bin(Value(2), BinOp::Shl, Value(0), Value(1)),
        );
        assert!(il.contains("=w shl"), "expected shl: {il}");
    }

    #[test]
    fn emit_subword_shift_masks_count_to_type_width() {
        // A u8 shift must mask its (always-i64) count to mod 8, not rely on
        // the hardware's mod-32 masking of the `w` register, or an over-shift
        // (e.g. by 10) would diverge from Rust's `wrapping_shl` semantics.
        let u8 = int(8, false);
        let il = emit_binary(u8, u8, Instr::Bin(Value(2), BinOp::Shl, Value(0), Value(1)));
        assert!(
            il.contains("and") && il.contains(" 7"),
            "expected a mod-8 count mask: {il}"
        );
        assert!(il.contains("shl"), "expected the shl: {il}");
        assert!(
            il.contains("255"),
            "expected the u8 canonicalization mask on the result: {il}"
        );
    }

    #[test]
    fn emit_word_width_shift_does_not_mask_count() {
        // i32/u32 fill the `w` register exactly, so the hardware's mod-32
        // masking already matches the type width; no explicit count mask.
        let i32_ty = int(32, true);
        let il = emit_binary(
            i32_ty,
            i32_ty,
            Instr::Bin(Value(2), BinOp::Shl, Value(0), Value(1)),
        );
        assert!(
            !il.contains("shamt"),
            "a word-width shift should not mask its count: {il}"
        );
    }

    #[test]
    fn emit_subword_arith_canonicalizes() {
        // An unsigned sub-word add masks its result to the low `bits`.
        let u8 = int(8, false);
        let il = emit_binary(u8, u8, Instr::Bin(Value(2), BinOp::Add, Value(0), Value(1)));
        assert!(il.contains("add"), "expected the add: {il}");
        assert!(
            il.contains("and") && il.contains("255"),
            "expected a mask: {il}"
        );

        // A signed sub-word add sign-extends its result from `bits`.
        let i8 = int(8, true);
        let il = emit_binary(i8, i8, Instr::Bin(Value(2), BinOp::Add, Value(0), Value(1)));
        assert!(il.contains("extsb"), "expected a sign-extend: {il}");
    }

    #[test]
    fn emit_struct_declares_aggregate_type() {
        // R12: a `type :Vec2 = { l, l }` is emitted for the struct.
        let il = emit_src("type: Vec2 x i64 y i64 ; : mk ( i64 i64 -- Vec2 ) Vec2 ;");
        assert!(
            il.contains("type :Vec2 = { l, l }"),
            "expected the aggregate type decl: {il}"
        );
    }

    #[test]
    fn emit_struct_return_uses_aggregate_abi() {
        // A struct-returning word declares its return as `:Vec2`, not `l`, so
        // QBE copies the aggregate by value across the boundary.
        let il = emit_src("type: Vec2 x i64 y i64 ; : mk ( i64 i64 -- Vec2 ) Vec2 ;");
        assert!(
            il.contains("export function :Vec2 $mk("),
            "expected an aggregate return type: {il}"
        );
        assert!(il.contains("alloc8 16"), "expected a 16-byte alloc: {il}");
    }

    #[test]
    fn emit_packed_subword_fields_use_width_exact_stores() {
        // R15/RISK 3: adjacent `i8` fields store 1 byte each (`storeb`), never
        // the 8-byte marshalling `storel`, so neither clobbers its neighbour.
        let il = emit_src("type: P p i8 q i8 r i64 ; : mk ( i8 i8 i64 -- P ) P ;");
        assert!(il.contains("type :P = { b, b, l }"), "unexpected IL: {il}");
        assert_eq!(
            il.matches("storeb").count(),
            2,
            "two 1-byte field stores: {il}"
        );
        assert!(il.contains("storel"), "the i64 field stores 8 bytes: {il}");
    }

    #[test]
    fn emit_getter_i8_field_sign_extends_via_loadsb() {
        let il = emit_src("type: P p i8 q i8 r i64 ; : g ( P -- i8 ) P>p ;");
        assert!(
            il.contains("loadsb"),
            "expected a width-exact i8 load: {il}"
        );
    }

    #[test]
    fn emit_nested_struct_member_references_inner_aggregate() {
        let il = emit_src("type: Vec2 x i64 y i64 ; type: Segment from Vec2 to Vec2 ;");
        assert!(
            il.contains("type :Segment = { :Vec2, :Vec2 }"),
            "expected nested aggregate members: {il}"
        );
    }

    #[test]
    fn emit_struct_declared_before_its_nested_member_still_orders_member_first() {
        let il = emit_src("type: Outer v Inner ; type: Inner x i64 ;");
        let inner_pos = il.find("type :Inner").expect("missing Inner decl");
        let outer_pos = il.find("type :Outer").expect("missing Outer decl");
        assert!(inner_pos < outer_pos, "Inner must be emitted before Outer");
    }

    #[test]
    fn emit_struct_shared_nested_member_emits_it_exactly_once() {
        let il = emit_src("type: Outer1 v Inner ; type: Outer2 v Inner ; type: Inner x i64 ;");
        assert_eq!(
            il.matches("type :Inner").count(),
            1,
            "Inner emitted more than once"
        );
    }

    #[test]
    fn emit_enum_declares_opaque_aligned_blob() {
        // R15: the enum aggregate is an alignment-annotated opaque byte blob
        // sized to the whole enum (tag + max payload), not a member list.
        // Shape = i32 tag (padded to 8) + max payload 16 = 24 bytes, align 8.
        let il = emit_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ; : mk ( f64 -- Shape ) Circle ;",
        );
        assert!(
            il.contains("type :Shape = align 8 { b 24 }"),
            "expected an opaque aligned blob: {il}"
        );
    }

    #[test]
    fn emit_enum_param_and_return_use_aggregate_abi() {
        // A word taking/returning an enum spells `:Shape` in ABI positions, so
        // QBE copies the tagged aggregate by value across the boundary.
        let il =
            emit_src("type: Shape | Circle r f64 | Rect w f64 h f64 ; : id ( Shape -- Shape ) ;");
        assert!(
            il.contains("export function :Shape $id(:Shape"),
            "expected an aggregate param + return: {il}"
        );
    }

    #[test]
    fn emit_struct_field_of_enum_references_enum_aggregate() {
        // D9: a struct member of enum type references the enum's `:E`
        // aggregate, and the enum type is declared before the struct that
        // uses it.
        let il = emit_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ; type: Tagged k Shape n i64 ; : mk ( Shape i64 -- Tagged ) Tagged ;",
        );
        assert!(
            il.contains("type :Tagged = { :Shape, l }"),
            "expected a struct member of enum type: {il}"
        );
        let enum_pos = il.find("type :Shape").expect("enum type emitted");
        let struct_pos = il.find("type :Tagged").expect("struct type emitted");
        assert!(
            enum_pos < struct_pos,
            "the enum type must be declared before the struct that references it: {il}"
        );
    }

    #[test]
    fn emit_self_tail_call_loop_renders_phi_and_back_edge_jmp() {
        // R12: no codegen change is expected for the self-tail-call -> loop
        // transform (D5); QBE already renders `Phi`/back-edge `Jmp` natively.
        // This structural test verifies the loop IL (a header `phi` with a
        // back-edge predecessor, plus the back-edge `jmp`) is valid QBE text.
        let il = emit_src(
            ": sum-to ( i64 i64 -- i64 ) | acc n | n 0 = ~[ acc ] ~[ acc n + n 1 - sum-to ] if ;",
        );
        assert!(
            il.contains("phi"),
            "expected a header phi in the loop IL: {il}"
        );
        assert!(
            !il.contains("call $sum_to"),
            "a tail self-call must not render as a QBE call: {il}"
        );
        let jmp_targets: Vec<&str> = il
            .lines()
            .filter_map(|l| l.trim().strip_prefix("jmp "))
            .collect();
        assert!(
            jmp_targets.len() >= 2,
            "expected at least two jmps (entry forward jump + back-edge): {il}"
        );
        let target = jmp_targets[0];
        assert!(
            jmp_targets.iter().filter(|t| **t == target).count() >= 2,
            "expected the entry jump and the back-edge jump to target the same header label: {il}"
        );
    }

    /// The text of the emitted `function` whose header line starts with `header`,
    /// up to its closing brace. Every module carries several runtime helpers, and
    /// more than one of them `exit(1)`s, so an assertion about one helper has to
    /// be pinned to that helper rather than to the whole module.
    fn func_body<'a>(il: &'a str, header: &str) -> &'a str {
        let start = il
            .find(header)
            .unwrap_or_else(|| panic!("no `{header}` in IL: {il}"));
        let rest = &il[start..];
        let end = rest.find("\n}").expect("a function body ends in `}`");
        &rest[..end]
    }

    #[test]
    fn emitted_alloc_shim_has_null_trap() {
        // Criterion 14 (R9), deliberately emitter-level: a runtime golden would
        // need a memory limit low enough to fail a small `malloc` but high enough
        // to exec, which does not exist reliably. The shim checks `malloc`'s
        // return against NULL and branches to a trap that exits nonzero, so no
        // NULL ever reaches a dereference. `exit(1)`, not `abort`, so a future
        // test observes `Some(1)` rather than death by signal.
        let il = emit_src(": main ( -- ) 5 . ;");
        let alloc = func_body(&il, "function l $sooth_alloc(l %n)");
        assert!(
            alloc.contains("%isnull =w ceql %p, 0"),
            "expected a NULL check on malloc's return: {alloc}"
        );
        assert!(
            alloc.contains("jnz %isnull, @oom, @ok"),
            "expected the NULL branch to reach the trap: {alloc}"
        );
        assert!(
            alloc.contains("call $sooth_oom_trap(l %adj)"),
            "expected the trap call: {alloc}"
        );
        let trap = func_body(&il, "function $sooth_oom_trap(l %n)");
        assert!(
            trap.contains("$dprintf(w 2, l $oomfmt"),
            "the trap message goes to stderr: {trap}"
        );
        assert!(
            trap.contains("call $exit(w 1)"),
            "the trap exits nonzero rather than aborting: {trap}"
        );
        assert!(
            trap.contains("hlt"),
            "the trap must not fall through: {trap}"
        );
    }

    #[test]
    fn emitted_alloc_trace_is_gated_on_the_env_var() {
        // R10: the trace is gated on `SOOTH_TRACE_ALLOC`, read per event (caching
        // it would need a mutable global, which has no precedent here), with an
        // empty value counting as unset so a real program using `^` stays silent.
        // It prints through `printf` to stdout, never `dprintf` to stderr: one
        // stdio stream is what makes program order equal transcript order.
        let il = emit_src(": main ( -- ) 5 . ;");
        assert!(
            il.contains("data $tracenv = { b \"SOOTH_TRACE_ALLOC\", b 0 }"),
            "expected the gating variable's name: {il}"
        );
        let trace = func_body(&il, "function $sooth_trace_event(l %fmt, l %n)");
        assert!(
            trace.contains("call $getenv(l $tracenv)"),
            "expected a per-event `getenv`: {trace}"
        );
        assert!(
            trace.contains("jnz %unset, @off, @set"),
            "an unset variable prints nothing: {trace}"
        );
        assert!(
            trace.contains("%c =w loadub %e") && trace.contains("jnz %c, @on, @off"),
            "an empty value prints nothing either: {trace}"
        );
        assert!(
            trace.contains("call $printf(l %fmt, l %n, ...)"),
            "the trace prints one line per event to stdout: {trace}"
        );
        assert!(
            !trace.contains("dprintf"),
            "the trace must not go to stderr: {trace}"
        );
    }

    #[test]
    fn emitted_alloc_and_free_shims_agree_on_the_adjusted_size() {
        // R6/R7/R15: one global allocator behind `allocate(n)`/`free(ptr, n)`,
        // emitted as a shim over libc (no user-facing FFI). Both halves apply
        // `max(n, 1)` as `n + (n == 0)`, so `free` reports the size `allocate`
        // requested and a zero-sized payload never reaches `malloc(0)`, which may
        // return NULL and would fire the trap on a correct program. Each traces
        // its own event, giving the transcript its `alloc`/`free` lines.
        let il = emit_src(": main ( -- ) 5 . ;");
        assert!(
            il.contains("data $allocfmt = { b \"alloc %ld\\n\", b 0 }")
                && il.contains("data $freefmt = { b \"free %ld\\n\", b 0 }"),
            "expected one size-only line format per event: {il}"
        );
        let alloc = func_body(&il, "function l $sooth_alloc(l %n)");
        let free = func_body(&il, "function $sooth_free(l %p, l %n)");
        for body in [alloc, free] {
            assert!(
                body.contains("%zero =l ceql %n, 0") && body.contains("%adj =l add %n, %zero"),
                "expected the max(n, 1) adjustment: {body}"
            );
        }
        assert!(
            alloc.contains("call $malloc(l %adj)") && alloc.contains("ret %p"),
            "expected a malloc of the adjusted size: {alloc}"
        );
        assert!(
            alloc.contains("call $sooth_trace_event(l $allocfmt, l %adj)"),
            "expected the alloc event: {alloc}"
        );
        assert!(
            free.contains("call $free(l %p)"),
            "expected the libc free: {free}"
        );
        assert!(
            free.contains("call $sooth_trace_event(l $freefmt, l %adj)"),
            "expected the free event: {free}"
        );
    }

    /// The buffer dogfood plus a rebuild-style control word in the same
    /// module, so the two structural criteria read the same emitted IL.
    const MUTATION_PROBE: &str = "\
type: Buf  data ^[u8 64]  len usize ;
type: Counter n i64 ;

: new ( -- Buf )
  0 >u8 64 fill ^ 0 >usize Buf ;

: push-byte ( &!Buf u8 -- )
  | b x |
  b &!Buf>len @ | i |
  b &!Buf>data &!^ | arr |
  arr i &!> x !
  b &!Buf>len 1 +! ;

: bump-rebuild ( Counter -- Counter )
  | c |
  c c Counter>n 1 + Counter<n ;

: main ( -- )
  new | a |
  &!a 7 >u8 push-byte
  a drop
  0 Counter bump-rebuild Counter> . ;
";

    /// Instruction lines (tab-indented) in an emitted function body: the
    /// labels, the header and the closing brace are not instructions.
    fn instr_lines(body: &str) -> usize {
        body.lines().filter(|l| l.starts_with('\t')).count()
    }

    #[test]
    fn mutation_through_reference_emits_no_rebuild() {
        // Structural because a runtime golden cannot
        // distinguish "mutated in place" from "rebuilt correctly", and
        // eliminating the rebuild is the point of the slice. Pinned to the
        // mangled symbol: `qbe_name` escapes `-` to `.2d.`, so the literal
        // `push-byte` could never match.
        let il = emit_src(MUTATION_PROBE);
        let body = func_body(&il, "export function $push.2d.byte(");
        assert!(
            !body.contains("alloc"),
            "mutation through a reference must not allocate a rebuilt aggregate: {body}"
        );
        assert!(
            !body.contains("blit"),
            "mutation through a reference must not copy a rebuilt aggregate: {body}"
        );
        assert!(
            body.contains("storeb ") && body.contains("storel "),
            "expected the address-arithmetic-plus-store shape: {body}"
        );
        // The ceiling is set from this body's own measured shape: the two
        // projections and the element store, plus `bounds_check`'s guard
        // (a `Cmp`, a `Jnz`, a trap block and its `sooth_oob_trap` call) that
        // a *computed* index pays for, plus the fetch-add-store of `+!`.
        assert!(
            instr_lines(body) <= 24,
            "in-place mutation should stay near the address-arithmetic shape, found {} instructions: {body}",
            instr_lines(body)
        );
    }

    #[test]
    fn rebuild_style_equivalent_still_emits_alloc_and_blit() {
        // The control: the functional setter in the same module keeps the
        // whole-aggregate rebuild, so the no-rebuild test's assertion is measuring
        // `push-byte` rather than an emitter that stopped emitting `alloc`.
        let il = emit_src(MUTATION_PROBE);
        let body = func_body(&il, "export function :Counter $bump.2d.rebuild(");
        assert!(
            body.contains("alloc"),
            "a functional setter still allocates its new shell: {body}"
        );
        assert!(
            body.contains("blit"),
            "a functional setter still copies the old shell: {body}"
        );
    }

    #[test]
    fn qbe_emits_func_addr_as_copy_of_symbol() {
        // T-qbe-addr (R4): materializing a function symbol as a `Code` handle
        // is a plain `copy` of the (sanitized) global address, `l`-wide,
        // distinct from any pointer arithmetic.
        let func = IrFunc {
            name: "t".to_string(),
            params: vec![],
            ret: Some(IrType::Code),
            blocks: vec![crate::ir::Block {
                id: crate::ir::BlockId(0),
                instrs: vec![Instr::FuncAddr(Value(0), "f".to_string())],
                term: Terminator::Ret(Some(Value(0))),
            }],
            value_types: vec![IrType::Code],
        };
        let il = emit(&IrModule {
            funcs: vec![func],
            ..Default::default()
        })
        .unwrap();
        assert!(
            il.contains("=l copy $f"),
            "a `Code` handle is a copy of the symbol: {il}"
        );
    }

    #[test]
    fn qbe_emits_indirect_call_through_value() {
        // T-qbe-ind (R4): an indirect call goes through the callee *value*
        // (`%v1`, not a `$sym`); an aggregate quotation argument is spelled
        // with its `:Q{n}` ABI type (from the module's `quot_sigs`), and the
        // module emits the matching `type :Q0 = { l, l }`.
        let sig = match crate::ir::ir_type_of(crate::ast::quotation_type(
            vec![Type::I64],
            vec![Type::I64],
        )) {
            IrType::Quotation(sig) => sig,
            other => panic!("expected a quotation IrType, got {other:?}"),
        };
        let quot = IrType::Quotation(sig);
        let func = IrFunc {
            name: "t".to_string(),
            params: vec![quot, IrType::Code],
            ret: Some(IrType::I64),
            blocks: vec![crate::ir::Block {
                id: crate::ir::BlockId(0),
                instrs: vec![Instr::CallIndirect(
                    Some(Value(2)),
                    Value(1),
                    vec![Value(0)],
                )],
                term: Terminator::Ret(Some(Value(2))),
            }],
            value_types: vec![quot, IrType::Code, IrType::I64],
        };
        let il = emit(&IrModule {
            funcs: vec![func],
            quot_sigs: vec![QuotSigLayout { effect: sig.0 }],
            ..Default::default()
        })
        .unwrap();
        assert!(
            il.contains("type :Q0 = { l, l }"),
            "the module emits the quotation aggregate type: {il}"
        );
        assert!(
            il.contains("call %v1(:Q0 %v0)"),
            "the call goes through the value with a `:Q` aggregate arg: {il}"
        );
    }
}
