use super::*;
/// P7.S11 (R2.1): the word-scoped registry set a standalone combinator
/// check runs against: copies of the live registries, extended with whatever
/// the signature grounding minted. Dropped when the check returns (R2 step
/// 5, R6): nothing here ever reaches `module.structs`/`module.enums`/
/// `module.arrays`/`module.owned_cells`/`module.refs`/`module.slices`.
pub(in crate::check) struct WordScopedRegistries {
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
    pub arrays: Vec<ArrayDecl>,
    pub cells: Vec<OwnedCellDecl>,
    pub refs: Vec<RefDecl>,
    pub slices: Vec<SliceDecl>,
}

/// P7.S11 (R2/R2.1/R6): rebase a scratch clone of `generics` onto the live
/// registries' current lengths, run `ground` against word-scoped copies of
/// the shape registries, then flush the batch `ground` minted into
/// word-scoped copies of `structs`/`enums`. Returns `ground`'s value and the
/// extended registries; on `Err`, everything is dropped and nothing reaches a
/// live registry.
///
/// The rebase (P0-A) is what a fresh mint through the scratch clone needs:
/// without it, a scratch id can collide with whatever an *earlier* word's
/// check-time mint already flushed into the live registries this call was
/// handed. The flush into `local.enums`/`local.structs` (P0-B) is what makes
/// the body walk's unconditional `enums[id.index()]` lookups (`is_copy`,
/// `contains_reference`, the drop graph, `tag`) safe for an id this call just
/// minted. `arrays`/`cells`/`refs`/`slices` have no base to rebase (R6.2:
/// they intern by linear structural scan, with the id fixed at `vec.len()`
/// the moment a clone is taken) -- they are copied and handed to `ground`
/// and, via the caller, to the body walk, then simply dropped; keeping a
/// scratch-minted shape out of the live registries is P0-C's fix.
#[allow(clippy::too_many_arguments)]
pub(in crate::check) fn ground_into_word_scoped_registries<T>(
    generics: Option<&RefCell<GenericTypes>>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    slices: &[SliceDecl],
    ground: impl FnOnce(
        Option<&RefCell<GenericTypes>>,
        &mut Vec<ArrayDecl>,
        &mut Vec<OwnedCellDecl>,
        &mut Vec<RefDecl>,
    ) -> Result<T, String>,
) -> Result<(T, WordScopedRegistries), String> {
    let scratch = generics.map(|c| {
        let mut g = c.borrow().clone();
        g.rebase(structs.len(), enums.len());
        RefCell::new(g)
    });
    let mut local = WordScopedRegistries {
        structs: structs.to_vec(),
        enums: enums.to_vec(),
        arrays: arrays.to_vec(),
        cells: cells.to_vec(),
        refs: refs.to_vec(),
        slices: slices.to_vec(),
    };
    let value = ground(
        scratch.as_ref(),
        &mut local.arrays,
        &mut local.cells,
        &mut local.refs,
    )?;
    if let Some(s) = &scratch {
        let mut g = s.borrow_mut();
        g.flush_structs_into(&mut local.structs);
        g.flush_enums_into(&mut local.enums);
    }
    Ok((value, local))
}

/// P7b.S3 (S3-6): the `Type::CtorImage` stand-in for an Arrow-kinded variable
/// `v` -- the constructor target of the **first** `impl:` of `v`'s user bound,
/// in declaration order. `None` when there is nothing to stand in for: `v`
/// carries no `Bound::User` (its bound is `Copy`, or it has none), the bound's
/// trait has no `impl:` in this program, or no impl of it targets a
/// constructor. A concrete-target impl is skipped rather than accepted: an
/// Arrow-kinded variable can only ever be satisfied by a constructor, so a
/// concrete target is not a candidate stand-in for it.
///
/// Declaration order is the whole determinism claim: `impls` is the
/// whole-program registry in source order, and the first matching entry is
/// pinned by a unit test, so which representative instance a body is checked
/// against does not drift with unrelated edits.
///
/// P7b.S8 (phase 3 review, finding 2a): the match below is on `i.target.
/// pattern`'s shape alone (`PolyType::Generic{is_enum, idx, module, ..}`),
/// never on whether `args` are all concrete. Before Delta B, a fully-applied
/// ctor target (`Box[i64]`) folded to `PolyType::Concrete` at parse time
/// (`is_concrete()` true) and so never matched this arm; Delta B keeps such
/// a target's `Generic` pattern instead (`is_concrete()` now false for it,
/// `parse_impl_target_fully_applied_concrete_ctor_keeps_its_ctor_pattern`,
/// `src/parser.rs`), so it now falls into this arm too, alongside the
/// applied-var targets (`Box['ctor1]`) this doc's "concrete-target impl is
/// skipped" line describes. Measured safe, not fenced: `ctor_image_type`
/// carries only `gid` (the constructor *header*'s identity), never the
/// candidate impl's own argument spelling, so which impl of the *same*
/// header wins declaration order cannot change the grounded shape a
/// standalone check runs against -- only which *header* wins can, and
/// picking a different header's ctor image only ever changes which impl's
/// body the standalone precheck opportunistically dispatches into; the real
/// bound member call at every actual splice site re-resolves against the
/// caller's own concrete argument, independent of this choice. Of
/// `check_poly_combinator_standalone`'s two rescues, the first is blanket
/// but the second is tag-gated (`strip_stand_in_tag`): a stand-in-caused
/// body failure that arrives untagged stays a hard error. The admit is
/// nonetheless safe for pre-existing programs: every member of an
/// Arrow-kinded trait must App-head on the trait variable (a bare
/// higher-kinded variable is rejected outright, and a member with no
/// dispatchable input is rejected -- both measured), so before Delta B every
/// lifted-target impl of an Arrow-kinded trait was rejected by the S2-6
/// fence -- the new admit is unreachable for any program that compiled
/// before.
pub(super) fn arrow_stand_in(
    v: u32,
    sig: &PolySig,
    impls: &[ImplDecl],
    generics: Option<&RefCell<GenericTypes>>,
) -> Option<Type> {
    let tid = sig.bounds.iter().find_map(|(bv, b)| match (bv, b) {
        (bv, Bound::User(tid)) if *bv == v => Some(*tid),
        _ => None,
    })?;
    // The variable's declared kind fixes the constructor arity a stand-in
    // must have: applying a 2-parameter constructor at a 1-argument slot is
    // not a `Result` failure downstream, it is an index panic inside field
    // substitution -- so an arity-mismatched impl (legal in itself: S2's
    // `impl: Functor for Res` dogfood) is not a candidate representative,
    // and the *first fitting* impl in declaration order is.
    let (want_ty, want_len) = match sig.ty_kinds.get(v as usize) {
        Some(crate::ast::Kind::Arrow { domains, .. }) => (
            domains
                .iter()
                .filter(|d| !matches!(d, crate::ast::Kind::Len))
                .count(),
            domains
                .iter()
                .filter(|d| matches!(d, crate::ast::Kind::Len))
                .count(),
        ),
        _ => return None,
    };
    let generics = &generics?.borrow();
    let gid = impls
        .iter()
        .filter(|i| i.trait_id == tid)
        .find_map(|i| match &i.target.pattern {
            PolyType::Generic {
                is_enum,
                idx,
                module,
                ..
            } => {
                let gid = GenericId {
                    is_enum: *is_enum,
                    idx: *idx,
                    module: *module,
                };
                let (tys, lens) = if *is_enum {
                    let d = &generics.enums[*idx as usize];
                    (d.ty_var_names.len(), d.len_var_names.len())
                } else {
                    let d = &generics.structs[*idx as usize];
                    (d.ty_var_names.len(), d.len_var_names.len())
                };
                (tys == want_ty && lens == want_len).then_some(gid)
            }
            _ => None,
        })?;
    Some(crate::ast::ctor_image_type(generics, gid))
}

/// P7.S3k (R4/N3): the monomorphs reachable only *through* a generic body's
/// call to another generic word, discovered once every concrete call site has
/// been recorded.
///
/// A concrete caller's `CallInst` says that `(w, θ_w)` exists; `w`'s own body
/// may call another generic word, and that call was recorded only symbolically
/// (`Module::poly_cross_calls`) because `w`'s variables were still rigid when
/// its body was walked. Composing a record's mapping with a caller θ is what
/// grounds it, so discovery belongs here, where the concrete θs live and where
/// `apply_subst`'s registry interning is still reachable -- not at lowering,
/// which only ever looks an already-interned shape up.
///
/// Terminates with no depth cap (N3): R6 rejects a compound image over a
/// caller variable at the call site, so a composed θ assigns each callee
/// variable either a fixed concrete type or the caller θ's image of one caller
/// variable -- never a constructor applied to one. Every reachable θ therefore
/// draws its types from the finite pool the seed instantiations introduced, the
/// reachable `(word, θ)` set is finite, and the symbol dedup below reaches a
/// fixpoint. A mutual `g <-> h` cycle revisits `(g, θ)` at the *same* θ and
/// stops.
///
/// `word_symbols` and `trait_obligations` (P7.S3s R2) are what `compose`
/// needs to resolve a composed callee's own `Bound::User` obligations against
/// a concrete θ, mirroring `check_poly_call`'s bound loop -- passed in rather
/// than read off `module`, since the `TraitResolveCtx` built from them below
/// must not borrow `module` while the destructure above still needs `&mut`
/// access to it.
#[allow(clippy::too_many_arguments)]
pub(in crate::check) fn discover_transitive_instantiations(
    module: &mut Module,
    insts: &mut HashMap<Span, CallInst>,
    splice_records: &mut HashMap<(u32, Span), CallInst>,
    word_symbols: &[String],
    trait_obligations: &[WordObligations],
    word_enum_sites: &[WordEnumSites],
    word_cell_sites: &[WordCellSites],
    impl_monos: Vec<(String, Subst)>,
    generics: Option<&RefCell<GenericTypes>>,
) -> Result<Vec<CallInst>, String> {
    // P7.S4 (R6): build CallInsts for generic-impl member-word monomorphs
    // before the fixpoint, so their bodies' poly cross-calls get composed.
    // Even when poly_cross_calls is empty (no generic-calls-generic), the
    // member-word monomorphs themselves still need to be emitted by lowering.
    let Module {
        words,
        structs,
        enums,
        arrays,
        refs,
        owned_cells,
        statics,
        modules,
        poly_cross_calls,
        traits,
        impls,
        ..
    } = module;
    // Built here rather than accepted from the caller (R2): a caller-side
    // `TraitResolveCtx` would borrow `module` immutably for the whole call,
    // and this function still needs `&mut module` through the destructure
    // above.
    let tr = TraitResolveCtx {
        traits,
        impls,
        word_symbols,
        words,
        recorded: trait_obligations,
        enum_sites_recorded: word_enum_sites,
        cell_sites_recorded: word_cell_sites,
    };
    let ground = CrossGround {
        words,
        structs,
        enums,
        statics,
        modules: Some(modules),
        records: poly_cross_calls,
        tr,
        generics,
    };
    // P7.S4 (R6): build CallInsts for each generic-impl member-word monomorph.
    let mut extra_seeds: Vec<CallInst> = Vec::new();
    let mut seed_discovered: Vec<(String, Subst)> = Vec::new();
    for (word_name, subst) in &impl_monos {
        let symbol = instantiation_symbol(word_name, subst);
        if extra_seeds.iter().any(|s| s.symbol == symbol) {
            continue;
        }
        if let Some(seed) = ground.impl_mono_seed(
            word_name,
            subst,
            arrays,
            refs,
            owned_cells,
            &mut seed_discovered,
        )? {
            extra_seeds.push(seed);
        }
    }
    if poly_cross_calls.is_empty() && extra_seeds.is_empty() {
        // Still need to intern bundles for splice_records before returning.
        for inst in splice_records.values_mut() {
            if inst.out_arity >= 2 {
                inst.bundle = Some(intern_bundle_struct(structs, &inst.output_types));
            }
        }
        return Ok(Vec::new());
    }
    let mut transitive = ground.fixpoint(
        insts,
        splice_records,
        extra_seeds,
        arrays,
        refs,
        owned_cells,
        seed_discovered,
    )?;
    // R8/R14, the composed twin of `check`'s own `out_arity >= 2` loop: a
    // composed callee returning a bundle is laid out like any other. Run as a
    // post-pass because interning needs `&mut structs`, which the grounding
    // above holds immutably through `Ctx`. `intern_bundle_struct` dedups by
    // output tuple, so the flat entry and the routing copy of one `(h, θ_h)`
    // land on the same `StructId`.
    for inst in transitive.iter_mut() {
        intern_composed_bundles(structs, inst);
    }
    for inst in insts.values_mut() {
        intern_composed_bundles(structs, inst);
    }
    for inst in splice_records.values_mut() {
        intern_composed_bundles(structs, inst);
    }
    // P7.S12 (R1.2a): flush whatever `compose`/`impl_mono_seed` minted while
    // grounding `enum_words`, after the fixpoint (so a mint mid-fixpoint
    // never needs to be visible to a sibling grounding step within it, which
    // this slice does not attempt) and before layout reads `module.enums`.
    if let Some(cell) = generics {
        let mut g = cell.borrow_mut();
        g.flush_structs_into(structs);
        g.flush_enums_into(enums);
    }
    Ok(transitive)
}

/// The bundle of a composed `CallInst` and of every cross-call its body
/// routes. The nested copies get one too: `lower_poly_call` reads `bundle`
/// off the *call site's* record to know what shape came back.
fn intern_composed_bundles(structs: &mut Vec<StructDecl>, inst: &mut CallInst) {
    if inst.out_arity >= 2 {
        inst.bundle = Some(intern_bundle_struct(structs, &inst.output_types));
    }
    for callee in inst.poly_calls.values_mut() {
        if callee.out_arity >= 2 {
            callee.bundle = Some(intern_bundle_struct(structs, &callee.output_types));
        }
    }
}

/// P7.S3k (R4): everything the composition step reads. Bundled because it is
/// threaded through a worklist loop that also carries the two registries
/// grounding *writes* to (`arrays`/`refs`), which cannot ride here.
struct CrossGround<'a> {
    words: &'a [WordDef],
    structs: &'a [StructDecl],
    enums: &'a [EnumDecl],
    statics: &'a [StaticDecl],
    modules: Option<&'a [ModuleInfo]>,
    records: &'a HashMap<String, Vec<PolyCrossCall>>,
    tr: TraitResolveCtx<'a>,
    /// P7.S12 (R1.2a): the live instantiator, threaded here rather than left
    /// `None` as `word_ctx`'s two call sites below used to. `compose` grounds
    /// a cross-called generic word's own body spans (`enum_words`), which
    /// needs the same find-or-mint door `check_poly_call` uses -- there is no
    /// find-only alternative (`apply_subst`'s `Generic` arm).
    generics: Option<&'a RefCell<GenericTypes>>,
}

impl CrossGround<'_> {
    /// Seed from the concrete instantiations, then compose until no new
    /// `(word, θ)` appears. Each taken instantiation's routing map is written
    /// back onto it, so a seed learns what its own body calls and a composed
    /// entry carries the map its body will be lowered against.
    #[allow(clippy::too_many_arguments)]
    fn fixpoint(
        &self,
        insts: &mut HashMap<Span, CallInst>,
        splice_records: &mut HashMap<(u32, Span), CallInst>,
        extra_seeds: Vec<CallInst>,
        arrays: &mut Vec<ArrayDecl>,
        refs: &mut Vec<RefDecl>,
        cells: &mut Vec<OwnedCellDecl>,
        initial_discovered: Vec<(String, Subst)>,
    ) -> Result<Vec<CallInst>, String> {
        let mut seen: HashSet<String> = insts.values().map(|i| i.symbol.clone()).collect();
        let mut frontier: Vec<CallInst> = Vec::new();
        // P7.S4 (R6, review fix): a composed cross-call's own `Bound::User`
        // obligation can resolve to a *generic*-impl member word, discovering
        // a monomorph mid-composition rather than at a top-level call site
        // (e.g. `outer` calls `inner` calls `show`, and only `inner`'s own
        // bound resolves the generic `impl:`). `cross_calls_of` drains every
        // such discovery into this accumulator so the loop below can seed it
        // too -- otherwise the member word's body is never emitted and
        // lowering panics on the dangling symbol.
        let mut discovered: Vec<(String, Subst)> = initial_discovered;
        for inst in insts.values_mut() {
            inst.poly_calls = self.cross_calls_of(inst, arrays, refs, cells, &mut discovered)?;
            enqueue_new(&inst.poly_calls, &mut seen, &mut frontier);
        }
        // P7.S3o (R1/R2): splice-derived CallInsts are concrete instantiations
        // of poly words called from combinator bodies. They seed the fixpoint
        // just like ordinary instantiations, so a poly chain (combinator →
        // p1 → p2) discovers p2 through p1's cross-calls.
        for inst in splice_records.values_mut() {
            inst.poly_calls = self.cross_calls_of(inst, arrays, refs, cells, &mut discovered)?;
            enqueue_new(&inst.poly_calls, &mut seen, &mut frontier);
        }
        // P7.S4 (R6): seed the generic-impl member-word monomorphs into the
        // fixpoint, so their bodies' poly cross-calls get composed and their
        // own `poly_calls` routing maps are filled.
        for mut inst in extra_seeds {
            if !seen.insert(inst.symbol.clone()) {
                continue;
            }
            inst.poly_calls = self.cross_calls_of(&inst, arrays, refs, cells, &mut discovered)?;
            enqueue_new(&inst.poly_calls, &mut seen, &mut frontier);
            frontier.push(inst);
        }
        let mut transitive: Vec<CallInst> = Vec::new();
        loop {
            while !frontier.is_empty() {
                let mut next = Vec::new();
                for mut inst in frontier {
                    inst.poly_calls =
                        self.cross_calls_of(&inst, arrays, refs, cells, &mut discovered)?;
                    enqueue_new(&inst.poly_calls, &mut seen, &mut next);
                    transitive.push(inst);
                }
                frontier = next;
            }
            // Seed anything composition discovered along the way and loop
            // again if that introduces new work.
            for (word_name, subst) in std::mem::take(&mut discovered) {
                let symbol = instantiation_symbol(&word_name, &subst);
                if !seen.insert(symbol) {
                    continue;
                }
                if let Some(seed) =
                    self.impl_mono_seed(&word_name, &subst, arrays, refs, cells, &mut discovered)?
                {
                    frontier.push(seed);
                }
            }
            if frontier.is_empty() {
                break;
            }
        }
        // `insts` iterates a `HashMap`, so the discovery order is randomized;
        // sort so the recorded set is a deterministic sequence and not merely
        // a deterministic set. Lowering sorts by symbol too, but a test
        // reading this field should not have to.
        transitive.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        Ok(transitive)
    }

    /// Build the seed `CallInst` for one generic-impl member-word monomorph
    /// `(word_name, subst)` discovered by resolving a `Bound::User`
    /// obligation -- either a top-level call's or a composed cross-call's.
    /// `None` when the name resolves to no polymorphic word, which should not
    /// happen: `resolve_user_bound` only ever pushes a resolved impl's own
    /// member word.
    fn impl_mono_seed(
        &self,
        word_name: &str,
        subst: &Subst,
        arrays: &mut Vec<ArrayDecl>,
        refs: &mut Vec<RefDecl>,
        cells: &mut Vec<OwnedCellDecl>,
        impl_monos: &mut Vec<(String, Subst)>,
    ) -> Result<Option<CallInst>, String> {
        let Some(word) = self.words.iter().find(|w| w.name == word_name) else {
            return Ok(None);
        };
        let Some(sig) = &word.poly else {
            return Ok(None);
        };
        let combs = CombinatorIndex::new();
        // P7.S12 (R1.2a): `self.generics` rather than `None` -- this member
        // word's own body may hold a generated enum word call site
        // (`enum_words`), and grounding one needs the live instantiator, not
        // just a declared output shape.
        let ctx = word_ctx(
            word,
            self.structs,
            self.enums,
            self.statics,
            self.modules,
            &combs,
            self.generics,
        );
        let mut outputs = Vec::with_capacity(sig.outputs.len());
        for pty in &sig.outputs {
            outputs.push(apply_subst(
                sig, pty, subst, word_name, word.span, &ctx, arrays, cells, refs,
            )?);
        }
        // P7.S4b (R6): resolve the member word's own `where`-clause bounds
        // at this concrete instantiation, filling `trait_calls` the same
        // way `compose` does for cross-calls. A member word whose body calls
        // a trait member on its own bounded variable (e.g. `print` on `'T`
        // where `'T: Print`) records an obligation during `check_poly_body`;
        // that obligation is resolved here against the concrete θ, and the
        // dispatched symbol rides the `CallInst` so lowering finds it. Without
        // this step, the seed's `trait_calls` stays empty and lowering panics
        // on the unresolved trait-member call.
        let mut trait_calls: HashMap<Span, String> = HashMap::new();
        for (v, bound) in &sig.bounds {
            let Bound::User(trait_id) = bound else {
                continue;
            };
            let Some(ty) = subst.ty_of(*v) else {
                continue;
            };
            resolve_user_bound(
                *trait_id,
                *v,
                ty,
                sig,
                word_name,
                word.span,
                &ctx,
                &self.tr,
                arrays,
                cells,
                refs,
                &mut trait_calls,
                impl_monos,
                subst,
            )?;
        }
        // P7.S12 (R1.2/R1.2a): the same grounding `check_poly_call` does for
        // an ordinary instantiation, run here for the member word's own
        // monomorph.
        let mut enum_words: HashMap<Span, EnumId> = HashMap::new();
        for (site_span, site_pty) in self.tr.enum_sites_of(word_name, sig) {
            let grounded = apply_subst(
                sig, site_pty, subst, word_name, word.span, &ctx, arrays, cells, refs,
            )?;
            if let Type::Enum(found, _) = grounded {
                enum_words.insert(*site_span, found);
            }
        }
        // P7b.S6 (review fix): ground and intern this member word's own
        // body-internal cell-construction sites, mirroring `enum_words`
        // above -- the result is discarded, since interning (the
        // `apply_subst` `OwnedCell` arm's side effect) is the only thing a
        // body-internal temporary needs from grounding.
        for (_, site_pty) in self.tr.cell_sites_of(word_name, sig) {
            apply_subst(
                sig, site_pty, subst, word_name, word.span, &ctx, arrays, cells, refs,
            )?;
        }
        let symbol = instantiation_symbol(word_name, subst);
        Ok(Some(CallInst {
            callee: word_name.to_string(),
            subst: subst.clone(),
            symbol,
            out_arity: outputs.len(),
            output_types: outputs,
            bundle: None,
            quot_inputs: Vec::new(),
            trait_calls,
            poly_calls: HashMap::new(),
            enum_words,
        }))
    }

    /// Ground every cross-call recorded in `caller`'s body against `caller`'s
    /// own θ, keyed by the body span each one sits at -- the routing map
    /// lowering consults while lowering that one instantiation.
    fn cross_calls_of(
        &self,
        caller: &CallInst,
        arrays: &mut Vec<ArrayDecl>,
        refs: &mut Vec<RefDecl>,
        cells: &mut Vec<OwnedCellDecl>,
        impl_monos: &mut Vec<(String, Subst)>,
    ) -> Result<HashMap<Span, CallInst>, String> {
        let Some(records) = self.records.get(&caller.callee) else {
            return Ok(HashMap::new());
        };
        let caller_word = self.sole_poly_word(&caller.callee).ok_or_else(|| {
            overloaded_cross_call_error(
                &caller.callee,
                &records[0].callee,
                &caller.callee,
                records[0].span,
            )
        })?;
        // Every diagnostic a grounding failure raises names the *caller's*
        // body, since that is where the call site is; `word_ctx`'s combinator
        // view is only read by the back-edge guards, which nothing here
        // reaches. P7.S12 (R1.2a): `self.generics` rather than `None` --
        // `compose` used to ground only a callee's declared *output*, but now
        // also grounds the callee body's own `enum_words` sites, which needs
        // the live instantiator (there is no find-only door).
        let ctx = word_ctx(
            caller_word,
            self.structs,
            self.enums,
            self.statics,
            self.modules,
            &CombinatorIndex::new(),
            self.generics,
        );
        let mut routed = HashMap::new();
        for record in records {
            let Some(callee_word) = self.sole_poly_word(&record.callee) else {
                return Err(overloaded_cross_call_error(
                    &caller.callee,
                    &record.callee,
                    &record.callee,
                    record.span,
                ));
            };
            // An `inline` callee with a `Bound::User` is composed like any
            // other cross-call: its body was walked by `check_poly_body`
            // (the pre-pass runs for combinator words with user bounds), so
            // its trait obligations are recorded and `compose` resolves them
            // against the caller's concrete θ. Lowering finds the composed
            // `CallInst` in `poly_calls` (checked before the combinator
            // splice), so the call routes to a real function and the splice
            // is never reached.
            //
            // An `inline` callee *without* a `Bound::User` keeps the old
            // behavior: its body was not walked by `check_poly_body` (it may
            // use builtins `poly_call_term` does not dispatch, like `if`'s
            // `tag`/`branch`), so its own cross-calls are unrecorded and the
            // fixpoint cannot compose them. Splicing is safe only if the
            // body calls no polymorphic word; otherwise it is a located
            // error rather than a silent miscompile.
            if is_combinator(callee_word) {
                let callee_sig = callee_word
                    .poly
                    .as_ref()
                    .expect("sole_poly_word yields a polymorphic word");
                if !callee_sig
                    .bounds
                    .iter()
                    .any(|(_, b)| matches!(b, Bound::User(_)))
                {
                    if body_calls_a_poly_word(&callee_word.body, self.words) {
                        return Err(inline_callee_cross_call_error(
                            &caller.callee,
                            &record.callee,
                            record.span,
                        ));
                    }
                    continue;
                }
            }
            let sig = callee_word
                .poly
                .as_ref()
                .expect("sole_poly_word yields a polymorphic word");
            routed.insert(
                record.span,
                self.compose(record, caller, sig, &ctx, arrays, refs, cells, impl_monos)?,
            );
        }
        Ok(routed)
    }

    /// The single polymorphic word declared under `name`, or `None` when the
    /// name is a polymorphic overload set. An overload set cannot be routed:
    /// `Module::poly_cross_calls` merges both candidates' records under the
    /// one name while each record's mapping indexes its *own* candidate's
    /// variables, and nothing on a `CallInst` says which candidate it came
    /// from -- so composing would silently pick the wrong signature.
    fn sole_poly_word(&self, name: &str) -> Option<&WordDef> {
        let mut candidates = self
            .words
            .iter()
            .filter(|w| w.name == name && w.poly.is_some());
        let first = candidates.next()?;
        candidates.next().is_none().then_some(first)
    }

    /// θ_h = θ_w ∘ mapping: each callee variable takes either the concrete
    /// type the caller supplied outright or `θ_w`'s image of the caller
    /// variable it was matched against. Entry order follows the mapping's,
    /// which is the order the callee's declared inputs first mention each
    /// variable -- and since an id *is* an index handed out in first-mention
    /// order, and `poly_cross_signature_supported` keeps a quotation
    /// parameter (the one shape whose grounding is deferred to a second pass)
    /// off this path entirely, that order is ascending by id. So it agrees
    /// with the sort `check_poly_call` applies to its own θ, and a
    /// `(callee, θ)` reached both ways mints one symbol and one `IrFunc`.
    ///
    /// The callee's declared *inputs* are deliberately not ground here. The
    /// only reason to would be `apply_subst`'s interning, and a cross-call's
    /// input shapes mirror the caller's operand slots (`poly_cross_match`
    /// decomposes structurally, and R6 rejects a compound the caller built
    /// itself), which the caller's own instantiation already interned.
    #[allow(clippy::too_many_arguments)]
    fn compose(
        &self,
        record: &PolyCrossCall,
        caller: &CallInst,
        sig: &PolySig,
        ctx: &Ctx,
        arrays: &mut Vec<ArrayDecl>,
        refs: &mut Vec<RefDecl>,
        cells: &mut Vec<OwnedCellDecl>,
        impl_monos: &mut Vec<(String, Subst)>,
    ) -> Result<CallInst, String> {
        let mut subst = Subst::default();
        for (v, image) in &record.mapping {
            let ty = match image {
                Image::Concrete(t) => *t,
                // A caller variable no input mentions is rejected at the
                // caller's own declaration (its body could not produce one),
                // so θ_w binds every variable an image can name.
                Image::CallerVar(u) => caller.subst.ty_of(*u).expect(
                    "a caller type variable an operand carried is mentioned by the caller's inputs, so its own instantiation bound it",
                ),
            };
            subst.ty.push((*v, ty));
        }
        let mut outputs = Vec::with_capacity(sig.outputs.len());
        for pty in &sig.outputs {
            outputs.push(apply_subst(
                sig,
                pty,
                &subst,
                &record.callee,
                record.span,
                ctx,
                arrays,
                cells,
                refs,
            )?);
        }
        // R2: the same bound loop `check_poly_call` runs against a concrete
        // caller, run here once `subst` is grounded -- a `Bound::User` on a
        // composed callee's own variable is resolved against `self.tr`'s
        // whole-program tables, the identical diagnostic and the identical
        // span (`record.span`) a rejection would have used either way.
        let mut trait_calls: HashMap<Span, String> = HashMap::new();
        for (v, bound) in &sig.bounds {
            let Bound::User(trait_id) = bound else {
                continue;
            };
            // Ungrounded variables skip resolution for the same reason they do
            // in `check_poly_call`: no obligation can name a variable the body
            // could not have dispatched on.
            let Some(ty) = subst.ty_of(*v) else { continue };
            resolve_user_bound(
                *trait_id,
                *v,
                ty,
                sig,
                &record.callee,
                record.span,
                ctx,
                &self.tr,
                arrays,
                cells,
                refs,
                &mut trait_calls,
                impl_monos,
                &subst,
            )?;
        }
        // P7.S12 (R1.2/R1.2a): the same grounding `check_poly_call` does for
        // an ordinary instantiation, run here for the composed callee.
        let mut enum_words: HashMap<Span, EnumId> = HashMap::new();
        for (site_span, site_pty) in self.tr.enum_sites_of(&record.callee, sig) {
            let grounded = apply_subst(
                sig,
                site_pty,
                &subst,
                &record.callee,
                record.span,
                ctx,
                arrays,
                cells,
                refs,
            )?;
            if let Type::Enum(found, _) = grounded {
                enum_words.insert(*site_span, found);
            }
        }
        // P7b.S6 (review fix): the composed twin of `impl_mono_seed`'s own
        // cell-site grounding above -- discarded, interning-only.
        for (_, site_pty) in self.tr.cell_sites_of(&record.callee, sig) {
            apply_subst(
                sig,
                site_pty,
                &subst,
                &record.callee,
                record.span,
                ctx,
                arrays,
                cells,
                refs,
            )?;
        }
        let symbol = instantiation_symbol(&record.callee, &subst);
        Ok(CallInst {
            callee: record.callee.clone(),
            subst,
            symbol,
            out_arity: outputs.len(),
            output_types: outputs,
            bundle: None,
            // A quotation parameter is a located rejection on the cross-call
            // path (`poly_cross_signature_supported`), so a composed callee
            // has none to record.
            quot_inputs: Vec::new(),
            trait_calls,
            poly_calls: HashMap::new(),
            enum_words,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    /// A source checked the way `driver::assemble_module` checks one: the
    /// declaration checks first (P7.S3e -- `check_impl_decls` is what resolves
    /// each `impl:` binding to the word it names, and no call site can resolve
    /// an obligation without it, which `parse_with_core` now runs for every
    /// caller), then the body/call-site pass, returning the checked module
    /// alongside what R17's pre-pass recorded.
    fn checked_like_a_build(src: &str) -> Result<(Module, Vec<WordObligations>), String> {
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        let recorded = crate::check::check_module(&mut module)?;
        Ok((module, recorded))
    }

    /// P7b.S3 Phase 2 fixtures: a Functor trait over two structurally
    /// identical single-parameter enums, `Opt` implemented first and `Opt2`
    /// second, in that source order.
    fn functor_two_impls_src(extra: &str) -> String {
        format!(
            "type: Opt['T] | None | Some 'T ;\n\
             type: Opt2['T] | None2 | Some2 'T ;\n\
             trait: Functor['F: * -> *] :\n\
             map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             impl: Functor for Opt\n\
             : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Opt? ;\n\
             ;\n\
             impl: Functor for Opt2\n\
             : map swap ~[ ( Some2 ) Some2> swap call Some2 ] ~[ ( None2 ) drop drop None2 ] Opt2? ;\n\
             ;\n\
             {extra}\n\
             : main ( -- ) ;\n"
        )
    }

    /// R2.1's seam, called directly: a stale `enum_base` (forced through the
    /// public `rebase`, exactly the state a preceding check-time flush
    /// leaves behind) must not throw off the mint's id -- the rebase inside
    /// `ground_into_word_scoped_registries` is what re-points it at the live
    /// registry's *current* length. Kills mutation 4 (dropped rebase) and
    /// mutation 5 (dropped flush).
    #[test]
    fn standalone_grounded_output_id_matches_its_extended_slice_position() {
        let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             type: Color | Red | Green | Blue ;\n\
             type: Status | Active | Done ;\n";
        let tokens = lex(src).unwrap();
        let module = crate::test_support::parse_with_core(&tokens).unwrap();
        let result_idx = module
            .generics
            .enums
            .iter()
            .position(|e| e.name == "Result")
            .expect("the Result header is registered");
        let header_module = module.generics.enums[result_idx].module;
        let live_enum_len = module.enums.len();
        assert!(
            live_enum_len >= 2,
            "the fixture must declare at least two ordinary concrete enums"
        );
        let mut generics = module.generics.clone();
        // Force a stale `enum_base`, exactly the state a preceding check-time
        // flush (`src/check.rs:1012-1013`) leaves behind: one short of
        // `module.enums.len()`.
        generics.rebase(module.structs.len(), live_enum_len - 1);
        let cell = RefCell::new(generics);
        let (ty, local) = ground_into_word_scoped_registries(
            Some(&cell),
            &module.structs,
            &module.enums,
            &module.arrays,
            &module.owned_cells,
            &module.refs,
            &module.slices,
            |scratch, arrays, cells, refs| {
                let regs = crate::ast::MutRegistries {
                    structs: &module.structs,
                    enums: &module.enums,
                    arrays,
                    cells,
                    refs,
                };
                Ok(scratch
                    .expect("R1 threads the live cell in")
                    .borrow_mut()
                    .instantiate_enum(
                        result_idx,
                        &[Type::I64, Type::I64],
                        &[],
                        header_module,
                        regs,
                    ))
            },
        )
        .expect("grounding through the free function succeeds");
        let Type::Enum(id, name) = ty else {
            panic!("instantiate_enum must mint a Type::Enum: {ty:?}")
        };
        assert_eq!(
            id.index(),
            live_enum_len,
            "the rebase (P0-A) must land the mint at the live registry's current \
             length, not the stale pre-rebase base"
        );
        assert_eq!(
            local.enums.len(),
            live_enum_len + 1,
            "the flush (P0-B) must extend the word-scoped copy by exactly the \
             minted batch"
        );
        assert_eq!(
            local.enums[id.index()].name_static,
            name,
            "the id/index/name correspondence, read back off the Type itself"
        );
        assert_eq!(
            module.enums.len(),
            live_enum_len,
            "the live registry is untouched"
        );
    }

    /// R6's unit guard (P0-C): a combinator whose declared output is an array
    /// of its own grounded monomorph must intern that array into the
    /// word-scoped `local.arrays`, never into the live `arrays` registry --
    /// checked directly through `ground_into_word_scoped_registries` (the
    /// only route that can observe `local.arrays` at all, since
    /// `check_poly_combinator_standalone` returns only `Result<(), String>`)
    /// and, for the live-registry half, through an actual
    /// `check_poly_combinator_standalone` call (`hold` is a combinator, so
    /// checking the whole module runs it), so both call routes are covered.
    #[test]
    fn standalone_grounding_an_array_of_a_monomorph_leaves_the_live_arrays_untouched() {
        let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : hold inline ( 'T ~[ 'T -- Result['T i64] ] -- array[Result['T i64] 4] )\n\
               call 4 fill ;\n";
        let tokens = lex(src).unwrap();
        let module = crate::test_support::parse_with_core(&tokens).unwrap();
        let word = module
            .words
            .iter()
            .find(|w| w.name == "hold")
            .expect("hold parses");
        let sig = word.poly.as_ref().expect("hold is polymorphic");
        let cell = RefCell::new(module.generics.clone());
        let mut subst = Subst::default();
        for v in 0..sig.ty_var_names.len() as u32 {
            subst.ty.push((v, Type::I64));
        }
        for ln in 0..sig.len_var_names.len() as u32 {
            subst.len.push((ln, 4));
        }
        let span = word_span(word);
        let live_array_len = module.arrays.len();
        let (ty, local) = ground_into_word_scoped_registries(
            Some(&cell),
            &module.structs,
            &module.enums,
            &module.arrays,
            &module.owned_cells,
            &module.refs,
            &module.slices,
            |scratch, arrays, cells, refs| {
                let ctx = word_ctx(
                    word,
                    &module.structs,
                    &module.enums,
                    &[],
                    None,
                    &CombinatorIndex::new(),
                    scratch,
                );
                apply_subst(
                    sig,
                    &sig.outputs[0],
                    &subst,
                    &word.name,
                    span,
                    &ctx,
                    arrays,
                    cells,
                    refs,
                )
            },
        )
        .expect("grounding the array output succeeds");
        let Type::Array(id, _) = ty else {
            panic!("the declared output must ground to a Type::Array: {ty:?}")
        };
        let elem = local.arrays[id.index()].element;
        let Type::Enum(enum_id, _) = elem else {
            panic!("the array's element must ground to the scratch monomorph: {elem:?}")
        };
        assert_eq!(
            enum_id.index(),
            module.enums.len(),
            "the array points at the scratch monomorph, inside the local enum registry"
        );
        assert_eq!(
            module.arrays.len(),
            live_array_len,
            "the live arrays registry is untouched by the grounding"
        );

        // The same property through the production entry point:
        // `check_poly_combinator_standalone` itself must not leak the array
        // shape into the live registry either.
        let (checked, _) =
            checked_like_a_build(src).expect("hold checks standalone and the module compiles");
        assert_eq!(
            checked.arrays.len(),
            live_array_len,
            "check_poly_combinator_standalone must not intern the array into the live registry"
        );
    }

    /// P7b.S3 (S3-6): the Arrow-kinded standalone stand-in is the `CtorImage`
    /// of the *first* impl of the variable's user bound, in declaration
    /// order -- pinned, so which representative a body is checked against
    /// does not drift with unrelated edits. `Opt` is declared and implemented
    /// before `Opt2`, so `Opt` it is.
    #[test]
    fn standalone_stand_in_binds_arrow_var_to_first_impls_ctor_image() {
        let (module, _) = checked_like_a_build(&functor_two_impls_src(
            ": apply['F: Functor 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) map ;",
        ))
        .expect("the fixture checks");
        let apply = module
            .words
            .iter()
            .find(|w| w.name == "apply")
            .expect("apply exists");
        let sig = apply.poly.as_deref().expect("apply is polymorphic");
        let f = sig
            .ty_var_names
            .iter()
            .position(|n| n == "'F")
            .expect("'F declared") as u32;
        assert!(
            matches!(
                sig.ty_kinds.get(f as usize),
                Some(crate::ast::Kind::Arrow { .. })
            ),
            "sanity: 'F is Arrow-kinded"
        );
        let generics = RefCell::new(module.generics.clone());
        let stand_in = arrow_stand_in(f, sig, &module.impls, Some(&generics));
        match stand_in {
            Some(Type::CtorImage(_, name)) => assert_eq!(
                name, "Opt",
                "the stand-in is the FIRST impl's constructor in declaration order"
            ),
            other => panic!("expected a CtorImage stand-in, got {other:?}"),
        }
    }

    /// P7b.S3 (S3-6): a first-in-declaration-order impl whose constructor
    /// arity does not match the variable's declared kind (`Res['T 'E]` under
    /// `'F: * -> *` -- legal in itself, S2's shared-bound dogfood) is not
    /// chosen as the stand-in; the first *fitting* impl (`Opt`) is. Applying
    /// the mismatched constructor would not even be a rescuable `Err`: it is
    /// an index panic inside field substitution, so the fit check is
    /// structural and up front.
    #[test]
    fn arity_mismatched_first_impl_is_not_the_stand_in() {
        let (module, _) = checked_like_a_build(
            "type: Res['T 'E] | Ok 'T | Err 'E ;\n\
             type: Opt['T] | None | Some 'T ;\n\
             trait: Functor['F: * -> *] :\n\
             map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             impl: Functor for Res\n\
             : map swap ~[ ( Ok ) Ok> swap call Ok ] ~[ ( Err ) Err> swap drop Err ] Res? ;\n\
             ;\n\
             impl: Functor for Opt\n\
             : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Opt? ;\n\
             ;\n\
             : apply['F: Functor 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) map ;\n\
             : main ( -- ) ;\n",
        )
        .expect("the fixture checks");
        let apply = module
            .words
            .iter()
            .find(|w| w.name == "apply")
            .expect("apply exists");
        let sig = apply.poly.as_deref().expect("apply is polymorphic");
        let f = sig
            .ty_var_names
            .iter()
            .position(|n| n == "'F")
            .expect("'F declared") as u32;
        let generics = RefCell::new(module.generics.clone());
        match arrow_stand_in(f, sig, &module.impls, Some(&generics)) {
            Some(Type::CtorImage(_, name)) => assert_eq!(
                name, "Opt",
                "the 2-parameter `Res` is skipped; the first FITTING impl is the stand-in"
            ),
            other => panic!("expected a CtorImage stand-in, got {other:?}"),
        }
    }
}
