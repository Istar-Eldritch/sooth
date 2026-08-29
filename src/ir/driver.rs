//! Lowering driver: the top-level `lower` entry point and word/instantiation
//! lowering. The shared word-body lowering (`lower_word_parts`) lives in
//! `func_builder`, the common dependency root shared with `destructors`.
//! Touches all four other `ir` submodules (`types`, `layout`, `destructors`,
//! `func_builder`); imports them via `super`.

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
    let combinator_bodies = crate::check::combinator_index(module.words.iter());
    // P7.S8 (R2): each word's check-time `inline_uid` seed, by name. Built from
    // the same `module.words.iter().enumerate()` the per-word pass below and
    // `check.rs`'s own word loop walk, so the two sides agree by construction
    // rather than by copying. `lower_resolved_word_call` reads it to lower a
    // spliced trait member body under that member's own uid namespace.
    let member_uid_seeds: HashMap<String, u32> = module
        .words
        .iter()
        .enumerate()
        .map(|(idx, w)| (w.name.clone(), idx as u32 * crate::check::INLINE_UID_STRIDE))
        .collect();
    // P7.S10 (R3.3): each word's own declaration span, name-keyed exactly
    // like `member_uid_seeds` above -- read only by `lower_resolved_word_call`'s
    // splice-budget guard to locate the offending impl member's own `: name`
    // declaration rather than a call site (R3.3: no call-site span survives
    // past `lib/cmp.sth`'s `inline` splice).
    let member_spans: HashMap<String, Span> = module
        .words
        .iter()
        .map(|w| (w.name.clone(), w.span))
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
    // P7 slice 3i: an *operator* overload (a word declared under a builtin
    // operator's own name, e.g. `core::bool`'s `: . ( bool -- )`) that nothing
    // reaches is not emitted. Without this, importing a module that merely
    // *declares* one -- which every program using `core::bool` does -- would add
    // a function and its string constants to a build that never prints a bool.
    //
    // Two ways to reach one, so two ways to keep it. A bare `.` in a body is a
    // builtin name, so 8a dispatch records the chosen candidate per span
    // (`builtin_overloads`) rather than leaving the name in the term; a
    // *qualified* call (`b::.`) is rewritten to the declaration's own mangled
    // spelling and dispatched as an ordinary call, which does name it.
    //
    // The name is demangled for the operator test alone: a closure rewrites
    // every module-0 word to `{name}__m0`, and `.__m0` is the same overload.
    let called = called_names(&module.words);
    let uncalled_operator_overloads: std::collections::HashSet<usize> = module
        .words
        .iter()
        .enumerate()
        .filter(|(idx, w)| {
            crate::check::is_builtin_operator_name(crate::resolve::demangle_word(&w.name))
                && !called.contains(w.name.as_str())
                && !module
                    .builtin_overloads
                    .values()
                    .any(|s| s == &symbols[*idx])
        })
        .map(|(idx, _)| idx)
        .collect();
    let mut env: HashMap<String, Arity> = module
        .words
        .iter()
        .enumerate()
        .filter(|(idx, _)| {
            !drop_overload_indices.contains(idx)
                && !poly_indices.contains(idx)
                && !combinator_indices.contains(idx)
                && !uncalled_operator_overloads.contains(idx)
        })
        .map(|(idx, w)| {
            let ret_ty = word_ret_ty(&w.effect.outputs, &structs);
            (
                symbols[idx].clone(),
                Arity {
                    in_arity: w.effect.inputs.len(),
                    out_arity: w.effect.outputs.len(),
                    ret_ty,
                    quot_inputs: quot_input_slots(w.effect.inputs.iter().map(|s| s.ty)),
                },
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
            Arity {
                in_arity: decl.effect.inputs.len(),
                out_arity: decl.effect.outputs.len(),
                ret_ty,
                quot_inputs: quot_input_slots(decl.effect.inputs.iter().map(|s| s.ty)),
            },
        );
        extern_symbols.insert(decl.name.clone(), decl.symbol.clone());
    }
    let resolve = |name: &str| {
        extern_symbols
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    };
    let (statics, static_data) = build_statics(&module.statics, &enums);
    let slices = build_slices(&module.slices, &structs, &enums, &arrays);
    let regs = Registries {
        structs: &structs,
        enums: &enums,
        arrays: &arrays,
        cells: &cells,
        refs: &refs,
        slices: &slices,
        statics: &statics,
    };

    // R1: a recognized `drop` overload is excluded from this generic
    // per-word lowering pass -- unfiltered, it would compile to a QBE
    // function literally named `drop`, and a second override in the same
    // module would collide with it under the identical symbol. The override's
    // body is instead compiled by `synthesize_aggregate_destructors` (R2)
    // into the struct's own destructor symbol.
    let mut funcs: Vec<IrFunc> = Vec::new();
    for (idx, w) in module.words.iter().enumerate().filter(|(idx, _)| {
        !drop_overload_indices.contains(idx)
            && !poly_indices.contains(idx)
            && !combinator_indices.contains(idx)
            && !uncalled_operator_overloads.contains(idx)
    }) {
        // A word sharing its name with another candidate is not self-tail
        // recursive on a bare name match: the same name in its body may
        // resolve to the other candidate, the same reasoning that excludes
        // builtin-named words in `has_self_tail_call`.
        let self_tail =
            crate::check::has_self_tail_call(w, &combinator_bodies) && symbols[idx] == w.name;
        // R9: a word plus every quotation literal it materialized (element
        // 0 is the word itself).
        funcs.extend(lower_word_parts(
            &symbols[idx],
            &w.effect,
            &w.body,
            self_tail,
            None,
            &env,
            &resolve,
            regs,
            &module.instantiations,
            &module.builtin_overloads,
            // A monomorphic word declares no bounds (only a polymorphic
            // word's signature can), so it can never call through a
            // resolved trait obligation -- empty here, unlike the
            // per-instantiation loop below.
            empty_trait_calls(),
            // A monomorphic word is walked once, so its own polymorphic
            // call sites are all in the global `instantiations` table;
            // only a *generic* body's cross-call needs per-instantiation
            // routing.
            empty_poly_calls(),
            // A monomorphic word's own generated-enum-word call sites
            // resolve through `module.builtin_overloads` (the concrete
            // path), never through a per-instantiation map -- empty here,
            // as `poly_calls` is above.
            empty_enum_words(),
            &module.resolved_fields,
            &module.resolved_variant_fields,
            &poly_arities,
            &combinator_bodies,
            EnvPlan::None,
            &module.splice_records,
            &module.splice_trait_calls,
            &member_uid_seeds,
            // Mirrors `check.rs`'s `word_idx * INLINE_UID_STRIDE` seed:
            // both walk `module.words` in the same order (this loop
            // filters afterward, but `idx` still comes from the same
            // full enumerate), so a splice this word's body performs
            // resolves to the checker's own record for it, not another
            // word's that happens to share a `(0, span)` key.
            idx as u32 * crate::check::INLINE_UID_STRIDE,
            &member_spans,
        )?);
    }

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
    // P7.S4 (R6): add each monomorph's symbol to the lowering env so a
    // `trait_calls` dispatch to a generic-impl member-word monomorph can
    // resolve its arity. A poly word is excluded from the initial env (it
    // is never called by its bare name), but its monomorphs are called by
    // their instantiation symbols, which `lower_resolved_word_call` looks up.
    for inst in module
        .instantiations
        .values()
        .chain(&module.transitive_instantiations)
    {
        let symbol = crate::ast::instantiation_symbol(&inst.callee, &inst.subst);
        let word = poly_words[inst.callee.as_str()];
        let sig = word
            .poly
            .as_ref()
            .expect("a recorded callee is polymorphic");
        let effect = concrete_effect(
            sig,
            &inst.subst,
            &module.arrays,
            &module.owned_cells,
            &module.refs,
            &module.generics,
        );
        let ret_ty = word_ret_ty(&effect.outputs, &structs);
        env.insert(
            symbol,
            Arity {
                in_arity: effect.inputs.len(),
                out_arity: effect.outputs.len(),
                ret_ty,
                quot_inputs: quot_input_slots(effect.inputs.iter().map(|s| s.ty)),
            },
        );
    }
    // Dedup by symbol and sort, so the monomorphized funcs emit in a fixed
    // order regardless of `instantiations`' randomized HashMap iteration --
    // the rest of the module emits deterministically from `Vec`-ordered words,
    // and the IL should too.
    let mut distinct: Vec<(String, &CallInst)> = Vec::new();
    let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    // P7.S3k (R4): the monomorphs reached only through a generic body's call
    // to another generic word join the same dedup, so a `(callee, θ)` a
    // concrete caller *also* instantiates is emitted once. Chained rather
    // than merged: `transitive_instantiations` is empty for a program with no
    // generic-calls-generic call, so `distinct` is the identical list it was
    // and the IL is byte-for-byte (N2).
    for inst in module
        .instantiations
        .values()
        .chain(&module.transitive_instantiations)
        .chain(module.splice_records.values())
    {
        let symbol = crate::ast::instantiation_symbol(&inst.callee, &inst.subst);
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
        let effect = concrete_effect(
            sig,
            &inst.subst,
            &module.arrays,
            &module.owned_cells,
            &module.refs,
            &module.generics,
        );
        // R7/R14: a self-recursive polymorphic word is a nested polymorphic
        // call (the body calling the very word being instantiated), routed by
        // `inst.callee` below (P7 slice 3g, R2) rather than through the
        // `CallInst`/`env` paths, neither of which holds a poly self-name.
        // `has_self_tail_call` is the same syntactic predicate a monomorphic
        // word asks (it is already poly-aware: `declared_input_count` reads
        // `word.poly`), so a tail self-call in a generic body gets the loop
        // header and back-edge too, rather than the one-frame-per-level
        // ordinary recursion S3g's D3 deferred (P7 slice 3g-follow).
        funcs.extend(lower_word_parts(
            &symbol,
            &effect,
            &word.body,
            crate::check::has_self_tail_call(word, &combinator_bodies),
            Some(inst.callee.as_str()),
            &env,
            &resolve,
            regs,
            &module.instantiations,
            &module.builtin_overloads,
            // P7.S3e (R9): this instantiation's own bound-dispatch
            // resolutions -- a pure function of `(callee, θ)`, so this map is
            // identical to every other instantiation of the same `(callee,
            // θ)` pair (`CallInst::trait_calls`'s own doc comment).
            &inst.trait_calls,
            // P7.S3k (R4): and this instantiation's own cross-calls, composed
            // against the same θ -- likewise identical across two call sites
            // sharing a `(callee, θ)`.
            &inst.poly_calls,
            // P7.S12 (R1.2): this instantiation's own generated-enum-word
            // resolutions, identical across every call site sharing this
            // `(callee, θ)` pair for the same reason `trait_calls`/`poly_calls`
            // are.
            &inst.enum_words,
            &module.resolved_fields,
            &module.resolved_variant_fields,
            &poly_arities,
            &combinator_bodies,
            EnvPlan::None,
            &module.splice_records,
            &module.splice_trait_calls,
            // The real map, not an empty one: a composed instantiation's body
            // is exactly where the generic path reaches a member re-splice, so
            // an empty map here would leave P7.S8's R1 inert on that path.
            &member_uid_seeds,
            // A generic instantiation's own body is never checked through
            // `check_word`'s real (non-scratch) `PolyCtx` (`check_poly_body`'s
            // symbolic `poly_walk` pre-pass handles a poly body instead, and
            // never writes `splice_trait_calls`/`splice_records`), so no seed
            // here can collide with a real entry. P7.S8 (R3): safe because a
            // member body spliced out of this one no longer inherits this
            // counter at all -- it is reset to the member's own seed for the
            // duration of its splice, so the transitive collision that made
            // this `0` questionable cannot happen through that path.
            0,
            &member_spans,
        )?);
    }

    // R2: the override's body, by reference, keyed the way synthesis is keyed.
    let overrides: DropOverrides = drop_overloads
        .iter()
        .map(|(id, idx)| (*id, &module.words[*idx]))
        .collect();

    // R12: append a synthesized destructor for every linear struct/enum type
    // (the drop-glue home decided in Phase 4, used starting here): `drop`
    // calls it as a plain `Call` (R16).
    funcs.extend(synthesize_aggregate_destructors(
        &env,
        &resolve,
        regs,
        &overrides,
        &module.resolved_fields,
        &module.resolved_variant_fields,
        &combinator_bodies,
    )?);

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
        statics: static_data,
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
        // P7.S3h: an owning quotation is spelled with the same `:Q{n}` symbol,
        // so it must seed the same table. Omitting it emits a param or return
        // naming a type the module never declares.
        if let IrType::Quotation(sig) | IrType::OwningQuotation(sig) = ty {
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

/// R9: build the concrete `StackEffect` of one instantiation `(word, θ)`,
/// substituting the ground `θ` into the polymorphic signature's fixed inputs
/// and outputs. The row variable (`..s`) is not materialized: it is a
/// pass-through that stays on the caller's stack (S2), so it never enters the
/// monomorphized function's frame.
fn concrete_effect(
    sig: &PolySig,
    subst: &Subst,
    arrays: &[ArrayDecl],
    owned_cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    generics: &GenericTypes,
) -> StackEffect {
    let slot = |pt: &PolyType| TypedSlot {
        name: None,
        ty: subst_polytype(pt, subst, arrays, owned_cells, refs, generics),
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
pub(super) fn subst_polytype(
    pt: &PolyType,
    subst: &Subst,
    arrays: &[ArrayDecl],
    owned_cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    generics: &GenericTypes,
) -> Type {
    match pt {
        PolyType::Concrete(t) => *t,
        PolyType::Var(v) => subst
            .ty_of(*v)
            .expect("checked: unification bound every input type variable"),
        PolyType::Array(elem, len) => {
            let element = subst_polytype(elem, subst, arrays, owned_cells, refs, generics);
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
        // P7.S3l (phase 1, R4 recon finding): a plain (non-combinator) word
        // may declare an abstract quotation parameter and call it (S3l), so
        // it *does* reach standalone monomorphized lowering -- the R7/R20
        // premise this arm used to assert ("never monomorphized") was
        // already false before S3l for a word that only `drop`s such a
        // parameter; S3l's `call` arm just added a second, checker-visible
        // path to the same pre-existing gap. Mirrors check-side
        // `apply_subst`'s `Quotation` arm: substitute both rows, then ground
        // to `Type::Quotation`/`Type::InlineQuotation` exactly as that
        // function does (no interning needed here -- `quotation_type`/
        // `inline_quotation_type` mint a fresh leaked effect per call, so a
        // lookup-only style like the `Array`/`Ref` arms above does not apply).
        PolyType::Quotation(ins, outs, is_inline, _, _) => {
            let cins = ins
                .iter()
                .map(|p| subst_polytype(p, subst, arrays, owned_cells, refs, generics))
                .collect();
            let couts = outs
                .iter()
                .map(|p| subst_polytype(p, subst, arrays, owned_cells, refs, generics))
                .collect();
            if *is_inline {
                crate::ast::inline_quotation_type(cins, couts)
            } else {
                crate::ast::quotation_type(cins, couts)
            }
        }
        // P7 slice 3b: a body-only marker, never in a declared signature.
        PolyType::QuotLit => unreachable!("a quotation-literal marker never reaches a signature"),
        // Slice 13 (R-A8/D4): lowering only *looks up* an already-interned
        // shape, exactly as the array arm does -- check-side `apply_subst`
        // has interned every `Type::Ref` this word's instantiations can
        // produce by the time lowering runs, so a miss here is a gap in that
        // coverage, not a reason to intern from the lowering side.
        PolyType::Ref(referent, mutable) => {
            let referent = subst_polytype(referent, subst, arrays, owned_cells, refs, generics);
            let idx = refs
                .iter()
                .position(|d| d.referent == referent && d.mutable == *mutable)
                .expect("checked: the concrete reference shape was interned at the call site");
            Type::Ref(RefId::from_index(idx), *mutable, refs[idx].name_static)
        }
        // P7.S3n (R3): the cell twin of the `Ref` arm above -- a lookup, not
        // an intern, for the same reason: `apply_subst` has already interned
        // every `Type::OwnedCell` this word's instantiations can produce.
        PolyType::OwnedCell(payload) => {
            let payload = subst_polytype(payload, subst, arrays, owned_cells, refs, generics);
            let idx = owned_cells
                .iter()
                .position(|d| d.payload == payload)
                .expect("checked: the concrete cell shape was interned at the call site");
            Type::OwnedCell(
                crate::ast::OwnedCellId::from_index(idx),
                owned_cells[idx].name_static,
            )
        }
        // P7 slice 3a phase 2 (R2): lowering only *looks up* an
        // already-minted instantiation, exactly as the array/ref arms above
        // -- check's own `apply_subst` has minted every generic monomorph
        // this word's instantiations can produce by the time lowering runs
        // (the same registry, kept alive rather than dropped), so a miss
        // here is a gap in that coverage, not a reason to mint from the
        // lowering side.
        // P7.S6a (R8a lands length-aware lookup in a later phase):
        // `len_args` is not yet substituted or matched here -- phase 3's
        // scope is the mechanical compile-forced ripple only.
        PolyType::Generic {
            is_enum,
            idx,
            module,
            args,
            len_args: _,
            name: _,
        } => {
            let concrete_args: Vec<Type> = args
                .iter()
                .map(|a| subst_polytype(a, subst, arrays, owned_cells, refs, generics))
                .collect();
            // P7.S6a (R8a lands length-aware lookup in a later phase): an
            // empty placeholder matches every check-time mint's own
            // placeholder length list in this phase (R7's real value has
            // not landed yet), so the key still agrees with what
            // `apply_subst` minted.
            let found = if *is_enum {
                generics.lookup_enum(*idx as usize, *module, &concrete_args, &[])
            } else {
                generics.lookup_struct(*idx as usize, *module, &concrete_args, &[])
            };
            found.expect(
                "checked: apply_subst already minted this generic instantiation at check time",
            )
        }
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in a declared signature this walk grounds.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches a declared signature"
        ),
    }
}

/// Every word name called anywhere in `words`' bodies, quotation bodies
/// included. Read by the uncalled-operator-overload filter in `lower`: a term
/// naming a word is the one reach that leaves the name in the IR.
fn called_names(words: &[WordDef]) -> std::collections::HashSet<&str> {
    fn walk<'a>(terms: &'a [Term], out: &mut std::collections::HashSet<&'a str>) {
        for t in terms {
            match &t.kind {
                TermKind::Call(name, _) => {
                    out.insert(name.as_str());
                }
                TermKind::Quotation(body, ..) => walk(body, out),
                _ => {}
            }
        }
    }
    let mut out = std::collections::HashSet::new();
    for w in words {
        walk(&w.body, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::ir::test_helpers::*;
    use crate::lexer::lex;

    /// A user `impl: Ord` whose `cmp` body calls the `inline` library
    /// comparisons, so its own body splices and the checker records splice
    /// entries under the *member word's* uid namespace.
    const POINT_ORD: &str = "type: Point x i64 ;\n\
         impl: Ord for Point\n\
           : cmp\n\
             | a b |\n\
             &a &x @ | ax | &b &x @ | bx |\n\
             a drop b drop\n\
             ax bx lt ~[ Less ] ~[ ax bx gt ~[ Greater ] ~[ Equal ] if ] if ;\n\
         ;\n";

    /// P7.S8 (R1/R2): the seed `member_uid_seeds` hands `lower_resolved_word_call`
    /// is the uid the checker *actually* minted for that member's first splice,
    /// measured off `module.splice_trait_calls` rather than assumed from the
    /// formula. The member word is not at index 0, so `word_idx * STRIDE` is a
    /// non-zero value a constant-`0` seed could not have produced.
    #[test]
    fn a_member_words_check_time_seed_is_its_word_index_times_the_stride() {
        // A word ahead of the `impl:` block, so the member is not `words[0]`
        // and its seed is a value a constant-`0` formula could not produce.
        let src = format!(
            ": leading ( -- ) ;\n{POINT_ORD}: main ( -- ) 3 Point 7 Point lt ~[ 1 ] ~[ 0 ] if . ;\n"
        );
        let tokens = lex(&src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module).unwrap();

        let (idx, name) = module
            .words
            .iter()
            .enumerate()
            .find_map(|(i, w)| w.name.ends_with(";Point").then(|| (i, w.name.clone())))
            .expect("the `impl: Ord for Point` member word is a module word");
        let seed = idx as u32 * crate::check::INLINE_UID_STRIDE;
        assert_ne!(
            seed, 0,
            "the member is not `words[0]`, so the seed is not 0"
        );

        let uids: Vec<u32> = module
            .splice_trait_calls
            .keys()
            .map(|&(uid, _)| uid)
            .filter(|uid| *uid / crate::check::INLINE_UID_STRIDE == idx as u32)
            .collect();
        assert!(
            !uids.is_empty(),
            "`{name}`'s body splices `lt`/`gt`, whose bare `cmp` calls are recorded per splice"
        );
        assert_eq!(
            uids.iter().copied().min(),
            Some(seed),
            "the first uid minted inside the member body is its seed itself, not `seed + k`"
        );
    }

    /// P7.S10 (R4.4): error propagation, beside `lower`'s own driver code. A
    /// real recursive `impl: Ord` -- the same shape as the golden -- drives
    /// `lower_resolved_word_call`'s budget guard to `Err`, and this asserts
    /// that `lower()`'s own `Result` carries it unchanged, rather than a
    /// swallowing `.unwrap()`/`if let Ok(..)` dam anywhere in the 18-function
    /// closure discarding it first. `check_trait_decls`/`check_impl_decls` run
    /// inside `parse_with_core` (its own doc comment), so this needs no
    /// synthetic forced error: the real route works, per R4.4's mandate to
    /// attempt it first.
    #[test]
    fn a_recursive_impl_ord_error_propagates_unchanged_to_lowers_result() {
        let src = "type: Wrap v i64 ;\n\
             impl: Ord for Wrap\n\
               : cmp\n\
                 | a b |\n\
                 a b lt ~[ Less ] ~[ Equal ] if ;\n\
             ;\n\
             : main ( -- )\n\
               1 Wrap 2 Wrap lt ~[ 1 ] ~[ 0 ] if . ;\n";
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module).unwrap();

        let err = crate::ir::lower(&module)
            .expect_err("a recursive impl member's splice exceeds the budget");
        assert!(
            err.contains("exceeded the splice budget of"),
            "lower()'s Err should carry the budget guard's own message unchanged, got: {err}"
        );
        assert!(
            err.contains("`cmp` (member of trait `Ord` for `Wrap`)"),
            "unexpected diagnostic: {err}"
        );
    }

    /// P7.S8 (R1): the shape the fix exists for -- a spliced member body that
    /// itself splices a combinator, two levels below the caller. It reaches
    /// lowering through `lower_src`'s own `check` -> `lower` path, so a
    /// regression in the uid rule is a panic here and not only in the
    /// integration goldens.
    #[test]
    fn a_spliced_member_body_that_splices_a_combinator_lowers() {
        // The leading word shifts the member off `words[0]`, so its seed is
        // non-zero and a constant-`0` seed makes the nested lookup miss.
        let module = lower_src(&format!(
            ": leading ( -- ) ;\n{POINT_ORD}\
             : w ( -- i64 ) 3 Point 7 Point lt ~[ 1 ] ~[ 0 ] if ;\n\
             : main ( -- ) w . ;\n"
        ));
        let w = func(&module, "w");
        assert!(
            call_symbols(w).is_empty(),
            "every level splices: no call survives in `w`, got {:?}",
            call_symbols(w)
        );
    }

    /// Whether a function's block graph contains a real cycle (a block
    /// reachable from one of its own successors), rather than guessing from
    /// block-id ordering. `BlockId`s are allocated in construction order, not
    /// execution order: a branch's join block is minted before an else-arm
    /// that itself branches is descended into, so that inner branch's own
    /// join can end up with a *higher* id than the outer join it jumps
    /// forward into. A plain "does some block jump to an id <= its own" check
    /// mistakes that ordinary forward merge for a back-edge -- exactly the
    /// shape an inlined multi-arm eliminator (`eq`'s `Ordering?` dispatch)
    /// produces once spliced into a caller. Real DFS-based cycle detection is
    /// immune to id allocation order.
    fn block_graph_has_cycle(blocks: &[crate::ir::Block]) -> bool {
        use std::collections::HashMap;
        let succs: HashMap<u32, Vec<u32>> = blocks
            .iter()
            .map(|b| {
                let out = match b.term {
                    Terminator::Ret(_) => vec![],
                    Terminator::Jmp(to) => vec![to.0],
                    Terminator::Jnz(_, a, b) => vec![a.0, b.0],
                };
                (b.id.0, out)
            })
            .collect();
        #[derive(Clone, Copy, PartialEq)]
        enum Color {
            White,
            Gray,
            Black,
        }
        let mut color: HashMap<u32, Color> =
            blocks.iter().map(|b| (b.id.0, Color::White)).collect();
        fn visit(id: u32, succs: &HashMap<u32, Vec<u32>>, color: &mut HashMap<u32, Color>) -> bool {
            color.insert(id, Color::Gray);
            for &next in &succs[&id] {
                match color[&next] {
                    Color::Gray => return true,
                    Color::White => {
                        if visit(next, succs, color) {
                            return true;
                        }
                    }
                    Color::Black => {}
                }
            }
            color.insert(id, Color::Black);
            false
        }
        blocks
            .iter()
            .any(|b| color[&b.id.0] == Color::White && visit(b.id.0, &succs, &mut color))
    }

    /// E-P1-4 (slice 10c): the checker's tail-splice predicate and the loop
    /// lowering actually built must answer the same question. Asked across the
    /// two sites rather than of one function twice: the checker side is the
    /// predicate `inline_combinator`/`check_term` consult, the lowering side is
    /// the emitted back-edge, so a private rule on either side (or dropping the
    /// `tail` threading at one splice) shows up as a mismatch.
    #[test]
    fn tail_splice_check_and_lowering_agree_on_the_loop() {
        const BOOL_Q: &str = ": decide inline ( Bool ~[ -- i64 ] ~[ -- i64 ] -- i64 )\n\
             | e | | t | | c | c ~[ t call ] ~[ e call ] if ;\n";
        const BOOL_D: &str = ": decide! inline ( Bool ~[ -- i64 ] ~[ -- i64 ] -- i64 )\n\
             | e | | t | | c | c ~[ t call e drop ] ~[ e call t drop ] if ;\n";
        for (branch, callee, expected) in [(BOOL_Q, "decide", true), (BOOL_D, "decide!", false)] {
            let src = format!(
                "{branch}: sum-to ( i64 i64 -- i64 )\n\
                 | n | | acc | n 0 eq ~[ acc ] ~[ acc n add n 1 sub sum-to ] {callee} ;\n\
                 : main ( -- ) 0 10 sum-to . ;\n"
            );
            let tokens = lex(&src).unwrap();
            let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
            check(&mut module).unwrap();
            let combs = crate::check::combinator_index(module.words.iter());
            let word = module.words.iter().find(|w| w.name == "sum-to").unwrap();
            let checker = crate::check::has_self_tail_call(word, &combs);

            let ir = lower(&module).unwrap();
            let sum = ir.funcs.iter().find(|f| f.name == "sum-to").unwrap();
            let lowered_a_loop = block_graph_has_cycle(&sum.blocks);

            assert_eq!(checker, expected, "the checker's decision for {callee}");
            assert_eq!(
                checker, lowered_a_loop,
                "check and lowering must agree on whether the splice is a loop ({callee})"
            );
        }
    }

    /// P7 slice 3g (R2): a self-call inside a polymorphic body lowers to an
    /// ordinary `Instr::Call` targeting *this instantiation's own* symbol --
    /// never the shared poly name (absent from `env`, so the ordinary
    /// dispatch would panic on it) and never the sibling instantiation's
    /// symbol. Asserted at two instantiations, since one alone cannot tell
    /// "its own symbol" from "the only symbol there is".
    ///
    /// P7 slice 3g-follow: the fixture's self-call sits in *non*-tail
    /// position (`dup drop` follows it), which is what keeps this the
    /// no-loop-header case now that a tail one back-edges -- see
    /// `poly_self_tail_call_lowers_to_loop_back_edge` for the other side.
    #[test]
    fn poly_self_call_lowers_to_ordinary_recursive_call() {
        let src = ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T i64 -- 'T )\n\
               dup iszero ~[ drop ] ~[ 1 sub loopg dup drop ] if ;\n\
             : main ( -- ) 5 3 loopg . True 3 loopg drop ;\n";
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module).unwrap();
        let ir = lower(&module).unwrap();

        let all: Vec<&str> = ir.funcs.iter().map(|f| f.name.as_str()).collect();
        let insts: Vec<&IrFunc> = ir
            .funcs
            .iter()
            .filter(|f| f.name.contains("loopg"))
            .collect();
        assert_eq!(insts.len(), 2, "two instantiations, one per theta: {all:?}");
        for f in insts {
            let targets: Vec<&str> = f
                .blocks
                .iter()
                .flat_map(|b| &b.instrs)
                .filter_map(|i| match i {
                    Instr::Call(_, sym, _) => Some(sym.as_str()),
                    _ => None,
                })
                .collect();
            assert!(
                targets.contains(&f.name.as_str()),
                "{} must recurse into itself, called: {targets:?}",
                f.name
            );
            assert!(
                !targets.contains(&"loopg"),
                "{} must not target the bare poly name: {targets:?}",
                f.name
            );
            let header = match f.blocks[0].term {
                Terminator::Jmp(to) => f
                    .blocks
                    .iter()
                    .find(|b| b.id == to)
                    .is_some_and(|b| b.instrs.iter().any(|i| matches!(i, Instr::Phi(..)))),
                _ => false,
            };
            assert!(
                !header,
                "{} must not lower to a loop: its self-call is not in tail position",
                f.name
            );
        }
    }

    /// P7 slice 3g-follow: the same body with the self-call left in tail
    /// position lowers to a loop instead -- a phi-carrying header the entry
    /// block jumps into, and a backward `Jmp` where S3g emitted a recursive
    /// `Instr::Call`. Asserted on the IR because an instantiation cannot
    /// report its own theta at runtime; both instantiations are checked so
    /// the transform is not one theta's accident.
    #[test]
    fn poly_self_tail_call_lowers_to_loop_back_edge() {
        let src = ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T i64 -- 'T )\n\
               dup iszero ~[ drop ] ~[ 1 sub loopg ] if ;\n\
             : main ( -- ) 5 3 loopg . True 3 loopg drop ;\n";
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module).unwrap();
        let ir = lower(&module).unwrap();

        let all: Vec<&str> = ir.funcs.iter().map(|f| f.name.as_str()).collect();
        let insts: Vec<&IrFunc> = ir
            .funcs
            .iter()
            .filter(|f| f.name.contains("loopg"))
            .collect();
        assert_eq!(insts.len(), 2, "two instantiations, one per theta: {all:?}");
        for f in insts {
            let header = match f.blocks[0].term {
                Terminator::Jmp(to) => f
                    .blocks
                    .iter()
                    .find(|b| b.id == to)
                    .filter(|b| b.instrs.iter().any(|i| matches!(i, Instr::Phi(..))))
                    .map(|b| b.id),
                _ => None,
            };
            let header = header.unwrap_or_else(|| panic!("{} must open a loop header", f.name));
            assert!(
                f.blocks
                    .iter()
                    .any(|b| matches!(b.term, Terminator::Jmp(to) if to == header)
                        && b.id != f.blocks[0].id),
                "{} must back-edge to its header",
                f.name
            );
            let targets: Vec<&str> = f
                .blocks
                .iter()
                .flat_map(|b| &b.instrs)
                .filter_map(|i| match i {
                    Instr::Call(_, sym, _) => Some(sym.as_str()),
                    _ => None,
                })
                .collect();
            assert!(
                !targets.contains(&f.name.as_str()),
                "{} must not also recurse by call: {targets:?}",
                f.name
            );
        }
    }

    /// P7 slice 3g-follow: the back-edge is gated on tail position, not merely
    /// on the word having a loop header. This body holds both self-calls at
    /// once -- a trailing one that back-edges and an earlier one that cannot --
    /// so the earlier one must still lower to an ordinary recursive
    /// `Instr::Call` into the very header-carrying func it sits in.
    #[test]
    fn poly_non_tail_self_call_in_a_self_tail_body_stays_an_ordinary_call() {
        let src = ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T i64 -- 'T )\n\
               dup iszero ~[ drop ] ~[ 1 sub loopg 0 loopg ] if ;\n\
             : main ( -- ) 5 3 loopg . True 3 loopg drop ;\n";
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module).unwrap();
        let ir = lower(&module).unwrap();

        let insts: Vec<&IrFunc> = ir
            .funcs
            .iter()
            .filter(|f| f.name.contains("loopg"))
            .collect();
        assert_eq!(insts.len(), 2, "two instantiations, one per theta");
        for f in insts {
            let back_edges = f
                .blocks
                .iter()
                .filter(|b| matches!(b.term, Terminator::Jmp(to) if to.0 <= b.id.0))
                .count();
            assert_eq!(back_edges, 1, "{} must keep exactly one back-edge", f.name);
            let self_calls = f
                .blocks
                .iter()
                .flat_map(|b| &b.instrs)
                .filter(|i| matches!(i, Instr::Call(_, sym, _) if *sym == f.name))
                .count();
            assert_eq!(
                self_calls, 1,
                "{}'s non-tail self-call must stay a call",
                f.name
            );
        }
    }

    #[test]
    fn subst_polytype_grounds_a_poly_ref_slot_from_a_monomorphic_caller() {
        // Slice 13 (R-A8, review fix): `subst_polytype`'s `Ref` arm was
        // untested and the spec assumed it unreachable in Phase 1 -- but a
        // monomorphic caller borrowing a local into a generic word already
        // instantiates a poly ref slot, so lowering must ground it *now*,
        // not in a later phase. Stubbing the arm to `panic!` breaks this
        // build.
        let src = ": firstref ( &array['T 4] -- ) drop ;\n\
             : main ( -- ) 7 4 fill | a | &a firstref a drop ;\n";
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module).unwrap();
        lower(&module).expect("a monomorphic caller must ground the poly ref slot");
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
    fn word_str_slot_keeps_its_own_ir_type() {
        // A `str` slot enters a lowered frame as `IrType::Str`, not as a bare
        // 8-byte integer: it is the type a later `len`/`.`/`cstr` dispatches
        // on. The retired carried-slot form asserted this of a stack-buffer
        // load; the mapping itself is what survives, and `ir_type_of`'s own
        // width/float cases do not cover `Type::Str`.
        let ir = lower_src(": w ( str -- usize ) len ;");
        let w = func(&ir, "w");
        assert_eq!(w.params, vec![IrType::Str]);
        assert_eq!(w.value_types[0], IrType::Str);
    }

    #[test]
    fn quotation_type_grounds_a_still_abstract_row() {
        // P7.S3l (phase 1, R4): a plain word may now declare an abstract
        // quotation parameter and call it, so its declared row does reach
        // monomorphized lowering once the word's own type variable is bound
        // (the previous `unreachable!` here pinned an already-false "never
        // monomorphized" premise -- see the arm's own comment). This asserts
        // the substituted rows ground to the same `Type::Quotation` the
        // check side's `apply_subst` would produce for the same `θ`.
        use crate::ast::PolyType;
        let poly_quot = PolyType::Quotation(
            vec![PolyType::Var(0)],
            vec![PolyType::Var(0)],
            false,
            None,
            None,
        );
        let subst = Subst {
            ty: vec![(0, Type::I64)],
            len: Vec::new(),
        };
        let generics = GenericTypes::default();
        let grounded = subst_polytype(&poly_quot, &subst, &[], &[], &[], &generics);
        assert_eq!(
            grounded,
            crate::ast::quotation_type(vec![Type::I64], vec![Type::I64])
        );
    }

    /// P7.S3n (R3): `subst_polytype`'s owned-cell arm resolves `^'T` to the
    /// *right* registry entry, not merely to some cell.
    ///
    /// Asserted here rather than through a golden program because the result
    /// is unobservable at runtime: `^>` is rejected in a generic body, so a
    /// monomorphized poly body never loads through the cell, and forcing this
    /// lookup to index 0 leaves every program's output unchanged. Two entries,
    /// and the *second* requested, so returning the first (or ignoring the
    /// payload) fails.
    #[test]
    fn subst_polytype_owned_cell_resolves_the_payloads_own_registry_entry() {
        use crate::ast::{intern_owned_cell_type, PolyType};
        let mut cells = Vec::new();
        intern_owned_cell_type(&mut cells, Type::I64);
        let want = intern_owned_cell_type(&mut cells, Type::U32);
        let mut subst = Subst::default();
        subst.ty.push((0, Type::U32));
        let got = subst_polytype(
            &PolyType::OwnedCell(Box::new(PolyType::Var(0))),
            &subst,
            &[],
            &cells,
            &[],
            &GenericTypes::default(),
        );
        assert_eq!(got, want, "`^'T` at `'T = u32` is the `^u32` entry");
    }

    /// P7.S3r: `lower_src` is not enough for a bound-dispatch fixture, because
    /// it does not mangle -- a synthesized impl-body member's own call to a
    /// builtin-operator-named word (`max`) only resolves through
    /// `scoped_operator_overloads`, which reads the mangled candidate key;
    /// unmangled, the candidate set is always empty and every such call falls
    /// through to the builtin's own numeric-operand rejection. The
    /// trait/`impl:` pre-passes such a fixture also needs (without them
    /// `ImplDecl::resolved` is empty and every member call fails to resolve)
    /// now run inside `parse_with_core` itself, for every caller. Mirrors the
    /// real build's order.
    fn lower_with_resolve(src: &str) -> IrModule {
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        crate::resolve::resolve_modules(&mut module, true).unwrap();
        check(&mut module).unwrap();
        crate::ir::lower(&module).unwrap()
    }

    /// P7.S3e phase 4 (R9): the IR-level twin of the `sort` golden. Each
    /// monomorphization of one bounded word calls *its own* instantiation's
    /// `impl:` member, asserted on the emitted `Instr::Call` symbol rather
    /// than on program output -- which is what "the per-instantiation
    /// `CallInst::trait_calls` map reached the lowered call site" means. Two
    /// instantiations, because a single one cannot distinguish a
    /// per-instantiation map from one shared across all of them.
    #[test]
    fn bound_dispatch_lowers_each_instantiation_to_its_own_impl_member() {
        let m = lower_with_resolve(
            "trait: Getter['T] : get ( &'T -- i64 ) ; ;\n\
             type: Pt n i64 ;\n\
             type: Qt n i64 ;\n\
             impl: Getter for Pt\n\
               : get | p | p &n @ ;\n\
             ;\n\
             impl: Getter for Qt\n\
               : get | q | q &n @ ;\n\
             ;\n\
             : getval ['T: Getter] ( &'T -- i64 ) get ;\n\
             : main ( -- ) 7 Pt |p| &p getval . p drop\n\
                           9 Qt |q| &q getval . q drop ;\n",
        );
        assert_eq!(
            call_symbols(func(&m, "sooth_mono_getval__m0__t0_Pt")),
            vec!["get;Getter;0;Pt__m0"]
        );
        assert_eq!(
            call_symbols(func(&m, "sooth_mono_getval__m0__t0_Qt")),
            vec!["get;Getter;0;Qt__m0"]
        );
    }

    /// A synthesized impl-body member whose body calls a word named after a
    /// builtin operator (`max`): the call resolves to the local overload
    /// rather than the builtin, through `scoped_operator_overloads`' mangled
    /// candidate key, and that overload keeps its own body in the module. The
    /// overload's arity matches the builtin `max`'s, so the two agree.
    ///
    /// Pruning needs no help here: an impl member's body spells `max` as a
    /// literal term, so `called_names` alone spares the overload.
    #[test]
    fn an_impl_body_members_operator_named_call_resolves_to_the_local_overload() {
        let m = lower_with_resolve(
            "trait: Getter['T] : get ( &'T &'T -- i64 ) ; ;\n\
             type: Pt n i64 ;\n\
             : max ( &Pt &Pt -- i64 ) drop &n @ ;\n\
             impl: Getter for Pt\n\
               : get | a b | a b max ;\n\
             ;\n\
             : getval ['T: Getter] ( &'T &'T -- i64 ) get ;\n\
             : main ( -- ) 7 Pt |p| &p &p getval . p drop ;\n",
        );
        assert_eq!(
            call_symbols(func(&m, "sooth_mono_getval__m0__t0_Pt")),
            vec!["get;Getter;0;Pt__m0"]
        );
        assert_eq!(
            call_symbols(func(&m, "get;Getter;0;Pt__m0")),
            vec!["max__m0"]
        );
        assert!(
            m.funcs.iter().any(|f| f.name == "max__m0"),
            "the bound-reached operator overload keeps its body: {:?}",
            m.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }
}
