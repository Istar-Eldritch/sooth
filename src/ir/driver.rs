//! Lowering driver: the top-level `lower` entry point, the REPL per-line
//! `lower_line`, and word/instantiation lowering. The shared word-body
//! lowering (`lower_word_parts`) lives in `func_builder`, the common
//! dependency root shared with `destructors`. Touches all four other `ir`
//! submodules (`types`, `layout`, `destructors`, `func_builder`); imports
//! them via `super`.

use super::*;

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
    // Slice 8a (R1/R7): keyed by each word's distinct lowering symbol rather
    // than its surface name, so an overload set does not collapse to whichever
    // candidate was inserted last. A name with one candidate keys under itself,
    // which is every corpus word, so lookups by name are unchanged; an
    // overloaded call reaches its candidate through the checker's per-span
    // record, which carries the same symbol.
    let symbols = crate::ast::overload_symbols(&module.words);
    // Slice 9 phase 2 (R3/R18): `bool_print_word_def` is injected into every
    // assembled module regardless of whether the program ever prints a
    // `bool` (R6/R7 need it resolvable everywhere `.` is called), but R3
    // demands the *unused* case stay byte-for-byte QBE -- an always-emitted
    // `IrFunc` for it would add a function and two string constants to every
    // build that never touches it. Recognized by its unmistakable synthetic
    // span (no real source token ever parses to `Span::default()`, R2's
    // `bool_enum_decl` uses the same tell); excluded from both `env` and
    // `funcs` below unless some call site's `builtin_overloads` entry
    // actually names its lowering symbol.
    // Matched by its synthetic span alone, not by name: a multi-file closure
    // mangles every module-0 word (`resolve::resolve_modules`) to `{name}__m0`,
    // so `.` becomes `.__m0` there -- the span survives that rewrite unchanged.
    let unused_bool_print_idx = module
        .words
        .iter()
        .position(|w| w.span == Span::default() && w.name.starts_with('.'))
        .filter(|&idx| {
            !module
                .builtin_overloads
                .values()
                .any(|s| s == &symbols[idx])
        });
    let mut env: HashMap<String, Arity> = module
        .words
        .iter()
        .enumerate()
        .filter(|(idx, _)| {
            !drop_overload_indices.contains(idx)
                && !poly_indices.contains(idx)
                && !combinator_indices.contains(idx)
                && Some(*idx) != unused_bool_print_idx
        })
        .map(|(idx, w)| {
            let ret_ty = word_ret_ty(&w.effect.outputs, &structs);
            (
                symbols[idx].clone(),
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
                && Some(*idx) != unused_bool_print_idx
        })
        .flat_map(|(idx, w)| {
            // A word sharing its name with another candidate is not self-tail
            // recursive on a bare name match: the same name in its body may
            // resolve to the other candidate, the same reasoning that excludes
            // builtin-named words in `has_self_tail_call`.
            let self_tail = crate::check::has_self_tail_call(w) && symbols[idx] == w.name;
            // R9: a word plus every quotation literal it materialized (element
            // 0 is the word itself).
            lower_word_parts(
                &symbols[idx],
                &w.effect,
                &w.body,
                self_tail,
                &env,
                &resolve,
                regs,
                &module.instantiations,
                &module.builtin_overloads,
                &poly_arities,
                &combinator_bodies,
                EnvPlan::None,
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
        funcs.extend(lower_word_parts(
            &symbol,
            &effect,
            &word.body,
            false,
            &env,
            &resolve,
            regs,
            &module.instantiations,
            &module.builtin_overloads,
            &poly_arities,
            &combinator_bodies,
            EnvPlan::None,
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
        &env,
        &resolve,
        regs,
        &overrides,
        &combinator_bodies,
    ));

    // Slice 7a (R1/Q2): the module's distinct quotation signatures, scanned
    // out of every function and every aggregate layout once all funcs (words,
    // instantiations, materialized quotations, destructors) exist. Decoupled
    // from the materialization worklist: any `IrType::Quotation` the backend
    // could spell -- a param, a return, a value, or an aggregate field -- must
    // name an interned effect, so the scan is the single source of truth.
    let quot_sigs = collect_quot_sigs(&funcs, &structs.layouts, &enums.layouts, &arrays.layouts);

    Ok(IrModule {
        funcs,
        structs: structs.layouts,
        enums: enums.layouts,
        arrays: arrays.layouts,
        quot_sigs,
    })
}

/// Slice 7a (R1/Q2): every distinct quotation effect the backend could need a
/// `:Q{n}` symbol for, deduped by structural equality (two equal effects share
/// one entry, matching `qbe::quot_index`'s lookup). Scans each function's
/// params/return/value types and each aggregate layout's field/element types.
/// A quotation-value-free module yields an empty table, unchanged from before.
pub(crate) fn collect_quot_sigs(
    funcs: &[IrFunc],
    structs: &[StructLayout],
    enums: &[EnumLayout],
    arrays: &[ArrayLayout],
) -> Vec<QuotSigLayout> {
    let mut out: Vec<QuotSigLayout> = Vec::new();
    let add = |ty: IrType, out: &mut Vec<QuotSigLayout>| {
        if let IrType::Quotation(sig) = ty {
            if !out.iter().any(|q| q.effect == sig.0) {
                out.push(QuotSigLayout { effect: sig.0 });
            }
        }
    };
    for f in funcs {
        for &ty in &f.params {
            add(ty, &mut out);
        }
        if let Some(ty) = f.ret {
            add(ty, &mut out);
        }
        for &ty in &f.value_types {
            add(ty, &mut out);
        }
    }
    for s in structs {
        for field in &s.fields {
            add(field.ty, &mut out);
        }
    }
    for e in enums {
        for v in &e.variants {
            for field in &v.fields {
                add(field.ty, &mut out);
            }
        }
    }
    for a in arrays {
        add(a.elem, &mut out);
    }
    out
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
    builtin_overloads: &HashMap<Span, String>,
    poly_arities: &HashMap<String, usize>,
    combinators: &HashMap<String, Vec<Term>>,
) -> (IrFunc, Vec<IrFunc>, usize, usize) {
    debug_assert_eq!(entry_types.len(), entry_depth);
    // A REPL line has no word name to self-tail-call against.
    let mut b = FuncBuilder::new(env, resolve, regs, String::new());
    // R7: a line calling an overloaded word dispatches through the checker's
    // per-span record, exactly as a compiled body does; without it the call
    // falls into the name-directed builtin arm and miscompiles.
    b.builtin_overloads = builtin_overloads;
    // R7 (Slice 2): a call to a retained polymorphic word resolves through the
    // instantiation table keyed by its call-site span, not the name-keyed env.
    b.instantiations = instantiations;
    b.poly_arities = poly_arities;
    // R5 (Slice 6c): a bare line's call to a retained combinator is spliced in
    // place (the fifth threading site), rather than lowered to an `Instr::Call`
    // to a symbol never minted.
    b.combinators = combinators;

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
            // Slice 9 (R1): a zero-payload enum's carried slot loads as a
            // scalar, exactly like the retired `Bool` arm below.
            IrType::Enum(id) if b.enums.layouts[id.index()].is_scalar => {
                let v = b.fresh_value(slot_ty);
                b.push_instr(Instr::Load(v, ptr));
                stack.push(v);
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
            // A bare quotation on a REPL residual is likewise rejected (7a
            // keeps that rejection), and a `Code` handle is never a slot type.
            IrType::Ptr => unreachable!("a reference can never be a carried slot"),
            IrType::Code | IrType::Quotation(_) => {
                unreachable!("a bare quotation/code is never a REPL carried slot")
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
            // Slice 9 (R1): a zero-payload enum's carried slot stores as a
            // scalar; fall to the `_` scalar arm below.
            IrType::Enum(id) if b.enums.layouts[id.index()].is_scalar => {
                b.push_instr(Instr::Store(ptr, *v));
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

    // R9: a bare line can materialize a quotation too (e.g. constructing a
    // struct that holds one), so its callee funcs are surfaced alongside the
    // line function, which stays element separate for its distinguished name.
    let mats = std::mem::take(&mut b.materialized);
    let func = IrFunc {
        name: format!("sooth_line_{seq}"),
        params: vec![IrType::Ptr, IrType::I64],
        ret: Some(IrType::I64),
        blocks: b.blocks,
        value_types: b.value_types,
    };
    let extra = lower_materialized(
        mats,
        env,
        resolve,
        regs,
        instantiations,
        builtin_overloads,
        poly_arities,
        combinators,
    );
    (func, extra, m, out_bytes as usize)
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
pub(super) fn subst_polytype(pt: &PolyType, subst: &Subst, arrays: &[ArrayDecl]) -> Type {
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_word(
    word: &WordDef,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    instantiations: &HashMap<Span, CallInst>,
    builtin_overloads: &HashMap<Span, String>,
    poly_arities: &HashMap<String, usize>,
    combinators: &HashMap<String, Vec<Term>>,
) -> Vec<IrFunc> {
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
        builtin_overloads,
        poly_arities,
        combinators,
        EnvPlan::None,
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
    builtin_overloads: &HashMap<Span, String>,
    subst: &Subst,
    body: &WordBody,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    arrays: &[ArrayDecl],
    combinators: &HashMap<String, Vec<Term>>,
) -> Vec<IrFunc> {
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
        builtin_overloads,
        empty_poly_arities(),
        combinators,
        EnvPlan::None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Line, BOOL_ENUM_ID};
    use crate::check::check;
    use crate::ir::test_helpers::*;
    use crate::lexer::lex;
    use crate::parser::{parse, parse_line};

    #[test]
    fn lower_line_marshals_all_inputs_and_outputs() {
        // `+` from a carried depth of 2 loads both slots and stores the single
        // result: D=2 loads, M=1 store.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let (func, _q, m, _) = lower_line(
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
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
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
        let (func, _q, m, _) = lower_line(
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
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
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
        let (func, _q, m, out_bytes) = lower_line(
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
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
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
        let (func, _q, m, out_bytes) = lower_line(
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
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
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
        let (func, _q, m, out_bytes) = lower_line(
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
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
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
        let (func, _q, _m, _) = lower_line(
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
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
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
        let (func, _q, _m, _) = lower_line(
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
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
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
    fn lower_line_carried_float_slot_loads_as_float() {
        // A carried `f64` slot loads at its float `IrType` (R20), so the value
        // re-enters as a true float rather than a stale `i64`; no `Conv`
        // relabel is needed (that path is integer-only).
        let terms = line_terms("dup");
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
            empty_instantiations(),
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
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
        // `bool` occupies the reserved index 0 (Slice 9, R2), so `Shape` is 1.
        let shape = Type::Enum(EnumId::from_index(1), "Shape");
        let (func, _q, m, out_bytes) = lower_line(
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
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
        );
        assert_eq!(m, 1);
        assert_eq!(out_bytes, 24);
        assert_eq!(count(&func, |i| matches!(i, Instr::Blit(..))), 2);
        assert_eq!(count(&func, |i| matches!(i, Instr::Load(..))), 0);
        assert_eq!(count(&func, |i| matches!(i, Instr::Store(..))), 0);
    }

    #[test]
    fn quotation_type_never_reaches_mangling() {
        // R7's `subst_polytype` `unreachable!` arm is only sound because R20's
        // lowering filter keeps a quotation type away from mangling. This
        // asserts the arm *is* the guard: it panics on a quotation, so
        // replacing the `unreachable!` with a real mapping (a silent accept)
        // flips this test from panic to value and fails it. (Slice 7a lifts
        // the sibling `ir_type_of` guard, which now maps a quotation to a
        // runtime value; see `ir_type_of_quotation_is_two_slot_aggregate`.)
        use crate::ast::PolyType;
        let poly_quot = PolyType::Quotation(
            vec![PolyType::Concrete(Type::I64)],
            Vec::new(),
            false,
            None,
            None,
        );
        let subst = Subst::default();
        assert!(
            std::panic::catch_unwind(|| subst_polytype(&poly_quot, &subst, &[])).is_err(),
            "`subst_polytype` on a quotation must hit the R7 `unreachable!` arm"
        );
    }
}
