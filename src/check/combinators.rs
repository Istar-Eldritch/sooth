use super::*;

/// R18/R6a: every quotation-taking word registered under one name. A name
/// can carry more than one candidate exactly as an ordinary overloaded word
/// can (R1); resolving which one a call splices needs the live stack's
/// operand types, the same shape as `Overload`/poly-candidate resolution --
/// a single-value map here would silently shadow a second combinator
/// overload exactly as env's `Sig` did before B1.
///
/// Slice 10c (R-P1-5): it also carries the `CombinatorIndex` the shared
/// tail-splice predicate reads, so every checker site that already threads a
/// `CombinatorEnv` gets the predicate's view of the same words with no second
/// channel to keep in step.
#[derive(Default)]
pub(crate) struct CombinatorEnv<'a> {
    candidates: HashMap<String, Vec<Combinator<'a>>>,
    tail: CombinatorIndex,
}

impl<'a> CombinatorEnv<'a> {
    pub(super) fn get(&self, name: &str) -> Option<&Vec<Combinator<'a>>> {
        self.candidates.get(name)
    }

    pub(super) fn contains_key(&self, name: &str) -> bool {
        self.candidates.contains_key(name)
    }

    /// The shared tail-splice view (`has_self_tail_call`, `terms_tail_call_self`).
    pub(crate) fn tail(&self) -> &CombinatorIndex {
        &self.tail
    }

    /// Register (or replace) one name's candidates, as the REPL does when it
    /// adds the word being defined to the view it checks the body against.
    pub(crate) fn insert(&mut self, name: String, candidates: Vec<Combinator<'a>>) {
        self.candidates.insert(name, candidates);
        self.tail = combinator_index(self.candidates.values().flatten().map(|c| c.word));
    }

    fn members(&self) -> impl Iterator<Item = &Combinator<'a>> {
        self.candidates.values().flatten()
    }
}

impl<'a> FromIterator<(String, Vec<Combinator<'a>>)> for CombinatorEnv<'a> {
    fn from_iter<T: IntoIterator<Item = (String, Vec<Combinator<'a>>)>>(iter: T) -> Self {
        let candidates: HashMap<String, Vec<Combinator<'a>>> = iter.into_iter().collect();
        let tail = combinator_index(candidates.values().flatten().map(|c| c.word));
        Self { candidates, tail }
    }
}

/// Slice 10c (R-P1-1): one always-spliced word as the shared tail-splice
/// predicate and both splice sites need it. Owned rather than borrowed so the
/// checker, the native lowering driver and the REPL can each build one from
/// whatever they hold (`&[WordDef]`, a `CombinatorEnv`, a session store)
/// without a lifetime running through the whole lowering stack.
#[derive(Clone)]
pub struct CombinatorEntry {
    /// The body spliced at every call site.
    pub terms: Vec<Term>,
    /// How many declared input slots the body's leading `| ... |` binds pop
    /// from. A row (`..s`) is not a slot, so this is the `sig.inputs.len()` /
    /// `effect.inputs.len()` `inline_combinator` itself counts.
    pub inputs: usize,
    /// A second always-spliced word shares this name. Which one a bare name
    /// reaches cannot be decided syntactically, so the tail walk declines
    /// (R-P1-4) rather than guessing -- the same conservatism
    /// `has_self_tail_call` already applies to a builtin name.
    pub ambiguous: bool,
}

/// Slice 10c: the always-spliced words, by name, as the tail walk reads them.
pub type CombinatorIndex = HashMap<String, CombinatorEntry>;

/// Build a `CombinatorIndex` over whichever words the caller holds. The
/// `is_combinator` filter is the same one `collect_combinators` and
/// `ir::lower`'s splice env apply, so the three views name the same words.
pub fn combinator_index<'w>(words: impl IntoIterator<Item = &'w WordDef>) -> CombinatorIndex {
    let mut index = CombinatorIndex::new();
    for word in words {
        if !is_combinator(word) {
            continue;
        }
        let WordBody::Terms { terms } = &word.body;
        let inputs = match word.poly.as_ref() {
            Some(sig) => sig.inputs.len(),
            None => word.effect.inputs.len(),
        };
        match index.entry(word.name.clone()) {
            std::collections::hash_map::Entry::Occupied(mut e) => e.get_mut().ambiguous = true,
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(CombinatorEntry {
                    terms: terms.clone(),
                    inputs,
                    ambiguous: false,
                });
            }
        }
    }
    index
}

/// Slice 6a (R18): one monomorphic quotation-taking word available to inline.
/// Both fields are shared references into the module, so a `Combinator` is a
/// pair of pointers (`Copy`), which lets a call site copy it out of the
/// borrowed map and then reborrow `PolyCtx` mutably for the splice.
#[derive(Clone, Copy)]
pub(crate) struct Combinator<'a> {
    pub(super) word: &'a WordDef,
    terms: &'a [Term],
}

/// R18: gather the quotation-taking `WordBody::Terms` words, mono and poly
/// alike (`is_combinator` does not filter on `word.poly`), keyed by name, so a
/// call to one is intercepted and its body spliced (the inliner) rather than
/// lowered to a call to a word that mints no `IrFunc` (R20). `inline_combinator`
/// branches on `word.poly` internally to pick the mono or poly splice path.
pub(super) fn collect_combinators(words: &[WordDef]) -> CombinatorEnv<'_> {
    let mut map: HashMap<String, Vec<Combinator<'_>>> = HashMap::new();
    for word in words {
        if !is_combinator(word) {
            continue;
        }
        let WordBody::Terms { terms } = &word.body;
        map.entry(word.name.clone())
            .or_default()
            .push(Combinator { word, terms });
    }
    map.into_iter().collect()
}

/// R2 (Slice 6c): the checker's inline view for one retained combinator, the
/// per-`WordDef` analogue of `collect_combinators`, so the REPL can project its
/// session store into the `HashMap<String, Combinator>` the inline path reads
/// without reaching into `Combinator`'s private fields.
pub(crate) fn combinator_of(word: &WordDef) -> Combinator<'_> {
    let WordBody::Terms { terms } = &word.body;
    Combinator { word, terms }
}

/// R18/R20: a combinator is a word declaring `inline` (recognition is
/// **declared, not inferred**, R-A1). The checker inlines a call to one
/// (splicing its body) and lowering mints no `IrFunc` for it, so `check` and
/// `ir::lower` must agree on the predicate exactly; it lives here as the
/// single source.
///
/// A word that takes a `~[ ... ]` (`Type::InlineQuotation`) parameter cannot
/// be anything *but* a combinator (a `~` quotation has no runtime
/// representation), so it must declare `inline` too
/// (`check_inline_declaration`'s R-B1 neighbour rejects it otherwise); a word
/// taking only an ordinary `[ ... ]` (`Type::Quotation`) parameter and no
/// `inline` is an ordinary real call (part D lowers that shape).
pub fn is_combinator(word: &WordDef) -> bool {
    word.declares_inline
}

/// R23 (D7): whether a word's declared effect names a quotation parameter,
/// regardless of flavour. This is *not* the
/// "does this word splice?" question -- `is_combinator` answers that, from the
/// declaration alone -- but the coarser "is this slot a quotation?" one its
/// callers across `check/{poly,terms,audits,captures}.rs` ask, and the one the
/// REPL's own boundary asks to decline the ordinary `[ ... ]` parameter shape
/// it does not support.
pub(crate) fn word_declares_quotation_parameter(word: &WordDef) -> bool {
    match &word.poly {
        None => word
            .effect
            .inputs
            .iter()
            .any(|s| crate::ast::is_quotation_type(s.ty).is_some()),
        Some(sig) => sig.inputs.iter().any(poly_input_is_quotation),
    }
}

/// A polymorphic input slot that declares a quotation parameter: either a
/// variable-bearing effect (`[ 'T -- ]`) or a fully-concrete one that folded
/// to `Concrete(Type::Quotation)`.
pub(super) fn poly_input_is_quotation(p: &PolyType) -> bool {
    match p {
        PolyType::Quotation(..) => true,
        // Slice 10a (R1): a fully-concrete `~` folds to `Concrete(~)` on the
        // same footing as a fully-concrete ordinary quotation, so the accessor
        // recognizes both. Failing to recognize a `~` here makes the word not a
        // combinator, so it is lowered as an ordinary call and reaches
        // `ir_type_of`'s `unreachable!` -- the ICE this predicate guards.
        PolyType::Concrete(t) => crate::ast::is_quotation_type(*t).is_some(),
        _ => false,
    }
}

/// R22 (D5)/R4 (D5 relaxed): reject a cycle in the quotation-taking-word call
/// subgraph. Edge `A -> B` iff combinator `A`'s body names combinator `B`
/// (any position; a call to a quotation-taking word necessarily passes it a
/// quotation). Since the inliner splices `B`'s body into `A`'s, a cycle would
/// inline forever, so unlike `check_tail_call_cycles` a self-edge is normally
/// the error.
///
/// R4 relaxes this for one shape only: a **self-tail** combinator, whose every
/// self-occurrence is in tail position, gets no self-edge, because the loop
/// transform lowers that self-call to a back-edge (a finite loop) rather than
/// re-splicing forever. A self-name in *any* non-tail position (`all_calls`
/// count exceeds `tail_position_calls` count) keeps its self-edge and stays a
/// cycle error, and every cycle of length >= 2 (a mutual cycle) is untouched.
/// Reuses `check_tail_call_cycles`'s 3-colour DFS shape (recon 8).
pub(crate) fn check_combinator_cycles(combinators: &CombinatorEnv) -> Result<(), String> {
    let members: Vec<&Combinator> = combinators.members().collect();
    // Slice 8a: two combinators may now share a name (an overload set, R1),
    // so a bare callee name can name more than one node. Unlike
    // `check_tail_call_cycles`'s diagnostic -- where treating an ambiguous
    // name as no edge at all merely costs a runtime optimization on the rare
    // program that hits it -- a missed edge here is not a missed diagnostic,
    // it is the inliner splicing a real cycle forever. So this pass
    // over-approximates: an ambiguous name is an edge to *every* candidate
    // that shares it, never to none, which can only reject a cycle-free
    // program that happens to share a combinator name (rare, and equivalent
    // to renaming one of the two), never miss a real one.
    let mut idx: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, c) in members.iter().enumerate() {
        idx.entry(c.word.name.as_str()).or_default().push(i);
    }
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); members.len()];
    for (i, c) in members.iter().enumerate() {
        let self_name = c.word.name.as_str();
        let self_all = all_calls(&c.word.body)
            .iter()
            .filter(|&&n| n == self_name)
            .count();
        let self_tail = tail_position_calls(c.word, combinators.tail())
            .iter()
            .filter(|&&n| n == self_name)
            .count();
        // R4: a tail-only self-edge (every self-occurrence in tail position,
        // and at least one) is permitted -- the loop transform makes it finite.
        let tail_only_self = self_all > 0 && self_all == self_tail;
        for callee in all_calls(&c.word.body) {
            let Some(targets) = idx.get(callee) else {
                continue;
            };
            for &j in targets {
                if i == j && tail_only_self {
                    continue;
                }
                if !adj[i].contains(&j) {
                    adj[i].push(j);
                }
            }
        }
    }
    let mut color = vec![0u8; members.len()];
    let mut path: Vec<usize> = Vec::new();
    for start in 0..members.len() {
        if color[start] == 0 {
            if let Some(cycle) = find_combinator_cycle(start, &adj, &mut color, &mut path) {
                return Err(combinator_cycle_error(&members, &cycle));
            }
        }
    }
    Ok(())
}

/// 3-colour DFS returning the members of the first cycle reached. Unlike
/// `find_tail_cycle`, a self-edge (`v == u`) is a cycle, not skipped.
fn find_combinator_cycle(
    u: usize,
    adj: &[Vec<usize>],
    color: &mut [u8],
    path: &mut Vec<usize>,
) -> Option<Vec<usize>> {
    color[u] = 1;
    path.push(u);
    for &v in &adj[u] {
        if color[v] == 1 {
            let start = path.iter().position(|&x| x == v).unwrap();
            return Some(path[start..].to_vec());
        }
        if color[v] == 0 {
            if let Some(cycle) = find_combinator_cycle(v, adj, color, path) {
                return Some(cycle);
            }
        }
    }
    path.pop();
    color[u] = 2;
    None
}

/// R22: a located cycle rejection naming the members in order and closing the
/// loop back to the first (`` `rec` -> `rec` `` for the self-recursive case).
fn combinator_cycle_error(members: &[&Combinator], cycle: &[usize]) -> String {
    let mut chain: Vec<&str> = cycle
        .iter()
        .map(|&i| crate::resolve::demangle_word(members[i].word.name.as_str()))
        .collect();
    chain.push(chain[0]);
    let rendered = chain
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(" -> ");
    let span = word_span(members[cycle[0]].word);
    format!(
        "error: an always-spliced word cannot be recursive (the inliner would splice it forever): {} (line {}, col {})",
        rendered, span.line, span.col
    )
}

/// Slice 10c (R-P3-1a): what a quotation operand on the live stack actually
/// is. There are exactly two forms and both are load-bearing, so the
/// classification lives here once rather than being hand-rolled per consumer.
pub(super) enum QuotOperand {
    /// A quotation literal written at the call site; its body is interned in
    /// `prov.quotations` and can be spliced.
    Literal(QuotId),
    /// R21: an abstract quotation *parameter* of the enclosing always-spliced
    /// word, forwarded onward. It carries a declared effect and no body: this
    /// is what the standalone def-site check of a combinator sees, and at a
    /// real call site the substitution has already replaced it with the
    /// caller's literal.
    Forwarded(&'static crate::ast::QuotEffect),
}

/// Classify one quotation operand. `None` means the slot is not a quotation at
/// all, which every caller reports with its own diagnostic.
///
/// Shared by `inline_combinator`'s declared-parameter loops and the `branch`
/// primitive's checker arm (R-P3-1a): `branch` is the single builtin that
/// takes quotation operands, and its operands arrive in both forms -- literals
/// at a real splice, abstract parameters while checking `if`'s own body
/// (`| e | | t | | c | c tag t e branch`) standalone. A second hand-rolled
/// copy of this would silently handle only the literal form, which every
/// caller of `if` still exercises, so nothing but `if`'s own definition would
/// ever notice the omission.
pub(super) fn resolve_quotation_operand(found: Slot) -> Option<QuotOperand> {
    if let Some(QuotRef::Known(id)) = found.quot {
        return Some(QuotOperand::Literal(id));
    }
    crate::ast::is_quotation_type(found.ty).map(QuotOperand::Forwarded)
}

/// R18: inline a call to a monomorphic quotation-taking word. Validate each
/// declared input against the caller's live slot (a quotation parameter takes
/// a `Known` literal, checked directionally with the D3 capture check, R11/R12;
/// every other parameter is matched as usual), then splice the callee body
/// against the live stack (bracketed like a `call`), so the callee's own
/// `call`/`times` fuse against the caller's literals. R22 guarantees
/// termination. `tail` is the call site's own tail position, threaded into the
/// splice (R-P1-6).
#[allow(clippy::too_many_arguments)]
pub(super) fn inline_combinator(
    comb: &Combinator,
    span: Span,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
    granted: &HashSet<String>,
    tail: bool,
) -> Result<Vec<Slot>, String> {
    let name = comb.word.name.as_str();
    // A polymorphic combinator (`each`/`map`/`fold`, or any `'T`-carrying
    // quotation-taking word) keeps its signature in `word.poly`, not
    // `word.effect` (which is empty), so the monomorphic argument loop below
    // would run zero checks and skip R11/R12 entirely (item 3). Route it
    // through the poly-argument check, which resolves the parameter's declared
    // effect against the live stack and runs the *same* directional + D3
    // check, so the two paths agree.
    let poly_subst = if let Some(sig) = comb.word.poly.as_ref() {
        Some(check_poly_combinator_args(
            sig, span, &stack, name, ctx, env, arrays, cells, refs, prov, scope, poly, granted,
            tail,
        )?)
    } else {
        let tail_slots = tail_called_param_slots(name, poly.combinators.tail());
        let inputs: Vec<Type> = comb.word.effect.inputs.iter().map(|s| s.ty).collect();
        let n = inputs.len();
        if stack.len() < n {
            return Err(underflow_error(ctx, span, name, n, stack.len()));
        }
        let base = stack.len() - n;
        for (i, want) in inputs.iter().enumerate() {
            let found = stack[base + i];
            if let Some(eff) = crate::ast::is_quotation_type(*want) {
                match resolve_quotation_operand(found) {
                    // Slice 10a (R9, context 4): a monomorphic word's declared
                    // quotation parameter is a `Type::Quotation`/`InlineQuotation`
                    // whose `QuotEffect` carries no row, so the row grounds to
                    // the empty region. (Unreachable for a `~`: `inline_combinator`
                    // routes any poly word here to `check_poly_combinator_args`.)
                    Some(QuotOperand::Literal(id)) => {
                        check_literal_against_declared_effect(
                            id,
                            eff,
                            false,
                            &[],
                            name,
                            span,
                            ctx,
                            env,
                            arrays,
                            cells,
                            refs,
                            prov,
                            scope,
                            poly,
                            granted,
                            LiteralBoundary {
                                shape_changing: false,
                                is_arm: tail_slots.contains(&i),
                                caller_tail: tail,
                                finalize: false,
                            },
                            None,
                        )?;
                    }
                    // R21: forwarding an abstract quotation parameter. `found`
                    // is itself a declared quotation parameter of the enclosing
                    // combinator, reached only while checking that enclosing
                    // combinator standalone. At a real call site the
                    // substitution has already bound it to the caller's
                    // literal, so it carries a `Known` marker and splices
                    // there; here, at the def site, accept it when its declared
                    // effect matches the callee parameter, so `outer` may pass
                    // its own `f` to `inner`. The spliced callee body's own
                    // `f call`/`f times` then check the forwarded parameter
                    // against its declared effect (R8/R9).
                    Some(QuotOperand::Forwarded(_)) => {
                        if found.ty != *want {
                            return Err(quotation_argument_required_error(
                                ctx, span, name, *want, found.ty,
                            ));
                        }
                    }
                    None => {
                        return Err(quotation_argument_required_error(
                            ctx, span, name, *want, found.ty,
                        ));
                    }
                }
            } else if found.quot.is_some() {
                return Err(reject_quotation_argument(ctx, span, name));
            } else {
                match match_slot(found, *want) {
                    SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
                    SlotMatch::NeedsSizeConversion => {
                        return Err(size_conversion_needed_error(ctx, span, name, *want));
                    }
                    SlotMatch::NeedsStrToCstrConversion => {
                        return Err(str_needs_cstr_conversion_error(ctx, span, name));
                    }
                    SlotMatch::Mismatch => {
                        return Err(type_mismatch_error(ctx, span, name, *want, found.ty));
                    }
                }
            }
        }
        None
    };
    // R6: a self-tail combinator opens a splice-time loop. Its body is spliced
    // with `tail = true` so its own tail-position self-call is recognized as
    // the back-edge (above). 6d/R6: the nested-loop rejection is retired --
    // lowering's hoist-target split keeps a nested loop constant-stack -- so
    // opening this loop inside another is now legal and `splice_tail` is just
    // whether this is a self-tail combinator.
    let self_tail = crate::check::is_combinator(comb.word)
        && has_self_tail_call(comb.word, poly.combinators.tail());
    // Slice 10c (R-P1-6): the splice is *in place of* the call, so the callee
    // body's tail terms are the caller's. Threading `tail` here is what carries
    // tail position into a quotation literal the callee `call`s at its own tail
    // (`sum-to`'s recursive branch through a hand-written `if`), and is the
    // checker's half of the same threading `lower_call` does at the lowering
    // splice: the two must stay identical or the linear-spine guards run over a
    // back-edge lowering did not build.
    let splice_tail = self_tail || tail;
    let input_count = match comb.word.poly.as_ref() {
        Some(sig) => sig.inputs.len(),
        None => comb.word.effect.inputs.len(),
    };
    // Slice 10a (R11): a self-tail combinator's back-edge needs its ground
    // declared shape (inputs for R12's argument check, outputs and the
    // bottom-aligned index map for the arm's result), which only this set
    // site can compute -- the arm deep in the splice has no `sig`/`Subst`.
    let (ground_inputs, ground_outputs, index_map) = if self_tail {
        back_edge_declared_shape(
            comb.word,
            poly_subst.as_ref(),
            name,
            span,
            ctx,
            arrays,
            refs,
        )?
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    // R18/R21: splice the callee body, alpha-renamed so its `| ... |` locals
    // cannot collide with a caller local or, under transitive inlining, an
    // outer combinator's locals already in scope. Lowering renames identically
    // (`ir`), so a passed-down literal's captured name stays lexical.
    let uid = prov.inline_uid;
    prov.inline_uid += 1;
    let renamed = crate::ast::alpha_rename_locals(comb.terms, uid);
    let depth = scope.depth();
    let saved_marker = if self_tail {
        let saved = prov.self_tail_combinator.take();
        prov.self_tail_combinator = Some(SelfTailMarker {
            name: name.to_string(),
            input_count,
            ground_inputs,
            ground_outputs,
            index_map,
        });
        Some(saved)
    } else {
        None
    };
    // D1 fix (slice 8b, bug 3): the spliced body is `comb.word`'s own, so a
    // module-scoped visibility gate inside it (D1's drop-import check, 8a's
    // operator scoping) must resolve against the module that declares *it*,
    // not `ctx.module()` -- otherwise a library combinator disposing its own
    // resource gets attributed to whichever module happened to call it.
    let spliced_ctx = ctx.with_module(comb.word.module);
    let result = check_terms_relaxed(
        &renamed,
        stack,
        &spliced_ctx,
        env,
        arrays,
        cells,
        refs,
        prov,
        scope,
        splice_tail,
        poly,
        granted,
        true,
    );
    if let Some(saved) = saved_marker {
        prov.self_tail_combinator = saved;
    }
    stack = result?;
    leave_block(
        ctx,
        scope,
        depth,
        BlockEnd::Arm {
            token: "inline",
            span,
        },
    )?;
    Ok(stack)
}

/// R11/R12 (poly, item 3): the polymorphic twin of `inline_combinator`'s
/// monomorphic argument loop. A poly combinator's declared inputs live in
/// `sig.inputs`, not `word.effect`, so without this the directional (R11) and
/// D3 capture (R12) checks never ran on the poly argument path -- a caller
/// literal borrowing an enclosing place was silently accepted, a mono/poly
/// divergence in the premise D3 rests on. Resolve the parameter's declared
/// effect against the live stack (`unify_poly_input` binds any variable a
/// non-quotation input carries, e.g. `'T` in `['T ...] [ 'T -- &i64 ]`), then
/// ground the quotation effect and run the *same* `check_literal_against_
/// declared_effect` the monomorphic path uses, so the two agree.
#[allow(clippy::too_many_arguments)]
fn check_poly_combinator_args(
    sig: &PolySig,
    span: Span,
    stack: &[Slot],
    name: &str,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
    granted: &HashSet<String>,
    tail: bool,
) -> Result<Subst, String> {
    let tail_slots = tail_called_param_slots(name, poly.combinators.tail());
    let n = sig.inputs.len();
    if stack.len() < n {
        return Err(underflow_error(ctx, span, name, n, stack.len()));
    }
    let base = stack.len() - n;
    // Pass 1: unify the non-quotation inputs to resolve theta first, so a
    // variable a quotation effect mentions (`'T` in `[ 'T -- &i64 ]`) is
    // already bound when the effect is grounded in pass 2, whatever the
    // parameter order.
    //
    // Slice 10c: a *fresh integer literal* filling a bare type variable is
    // held back and unified last, against whatever the variable resolved to.
    // This is D8's literal coercion, which the comparison operators had for
    // free as builtin rows (`5 3 >usize <` typed through `unify_pair`'s
    // `LiteralSizeType`) and would otherwise lose on becoming `'T: Copy Ord`
    // library words: a bare `5` carries `i64`, so unifying it first pins `'T`
    // to `i64` and the `usize` operand then reads as a conflict.
    let mut subst = Subst::default();
    let mut deferred_literals: Vec<(usize, &PolyType)> = Vec::new();
    for (i, pin) in sig.inputs.iter().enumerate() {
        if poly_input_is_quotation(pin) {
            continue;
        }
        let found = stack[base + i];
        if found.quot.is_some() {
            return Err(reject_quotation_argument(ctx, span, name));
        }
        if found.literal && found.ty == Type::I64 && matches!(pin, PolyType::Var(_)) {
            deferred_literals.push((i, pin));
            continue;
        }
        unify_poly_input(
            sig, pin, found.ty, name, span, ctx, arrays, refs, &mut subst,
        )?;
    }
    for (i, pin) in deferred_literals {
        let PolyType::Var(v) = pin else {
            unreachable!("only a `Var` parameter is deferred")
        };
        let ty = match subst.ty_of(*v) {
            // Exactly D8's domain, no wider: a fresh literal fills a `usize`
            // or `isize` position without an explicit conversion, and nothing
            // else. Widening this to every numeric type would accept
            // `1 >i32 2 <>`, which the builtin rows always rejected.
            Some(resolved @ (Type::Usize | Type::Isize)) => resolved,
            _ => stack[base + i].ty,
        };
        unify_poly_input(sig, pin, ty, name, span, ctx, arrays, refs, &mut subst)?;
    }
    // Pass 2: ground each quotation parameter and run the directional + D3
    // check on its caller literal.
    //
    // Slice 10c (R-P2-3): sibling parameters sharing one declared *output*
    // row (`..o` in `~[ ..i -- ..o ]` on more than one parameter, e.g. `if`'s
    // two branch quotations) have no fixed `..o` to check a literal against
    // (R-P2-4). The first such literal checked for a given row id sets the
    // baseline; every later one sharing that row id is compared against it
    // and, on a contradiction, rejected here -- at the argument site -- with
    // both literals' actual shapes named, rather than left to surface later
    // at the splice site under a generic message (recon 8).
    let mut shape_baseline: HashMap<u32, Vec<Type>> = HashMap::new();
    for (i, pin) in sig.inputs.iter().enumerate() {
        if !poly_input_is_quotation(pin) {
            continue;
        }
        let found = stack[base + i];
        let concrete = apply_subst(sig, pin, &subst, name, span, ctx, arrays, refs)?;
        // Slice 10a (R1): `apply_subst` grounds an ordinary quotation parameter
        // to `Type::Quotation` and (phase 2) a `~` parameter to
        // `Type::InlineQuotation`; the accessor accepts both, so this let-else
        // never becomes a spurious `unreachable!` once `~` grounding lands.
        let Some(eff) = crate::ast::is_quotation_type(concrete) else {
            unreachable!("a quotation input grounds to a quotation type (apply_subst)")
        };
        // Slice 10a (R9, context 1): a row-bearing declared quotation parameter
        // grounds its row to the concrete caller-stack region below the
        // combinator's fixed inputs (`stack[..base]`). Per R4 that row is the
        // signature's own top-level row, so it grounds to the same region the
        // top-level row does. A parameter that declared no row grounds against
        // the empty region. `apply_subst` deliberately left the row off the
        // interned `eff` (splicing it would mint an effect no literal equals),
        // so it is reconstructed here, at the callee, and only type-only
        // (`Slot::computed`, dropping provenance, R16).
        let row: Vec<Slot> = match pin {
            PolyType::Quotation(_, _, _, Some(_), _) => stack[..base].to_vec(),
            _ => Vec::new(),
        };
        // Slice 10c (R-P2-2/R-P2-3): a declared quotation whose input and
        // output rows differ has no fixed exit-row check (R-P2-4).
        let row_out_id = match pin {
            PolyType::Quotation(_, _, _, a, Some(b)) if *a != Some(*b) => Some(*b),
            _ => None,
        };
        let shape_changing = row_out_id.is_some();
        let operand = resolve_quotation_operand(found);
        if let Some(QuotOperand::Literal(id)) = operand {
            let is_inline = matches!(concrete, Type::InlineQuotation(_));
            let literal_span = prov.quotations[id.0].span;
            let actual: Vec<Type> = check_literal_against_declared_effect(
                id,
                eff,
                is_inline,
                &row,
                name,
                span,
                ctx,
                env,
                arrays,
                cells,
                refs,
                prov,
                scope,
                poly,
                granted,
                LiteralBoundary {
                    shape_changing,
                    is_arm: tail_slots.contains(&i),
                    caller_tail: tail,
                    finalize: false,
                },
                None,
            )?
            .iter()
            .map(|s| s.ty)
            .collect();
            if let Some(rid) = row_out_id {
                if let Some(expected) = shape_baseline.get(&rid) {
                    let matches = actual.len() == expected.len()
                        && actual.iter().zip(expected).all(|(f, w)| {
                            matches!(
                                match_slot(Slot::computed(*f), *w),
                                SlotMatch::Exact | SlotMatch::LiteralSizeType
                            )
                        });
                    if !matches {
                        return Err(combinator_branch_output_mismatch_error(
                            ctx,
                            literal_span,
                            name,
                            expected,
                            &actual,
                        ));
                    }
                } else {
                    shape_baseline.insert(rid, actual);
                }
            }
        } else if operand.is_some() {
            // R21 (poly): a forwarded abstract quotation parameter, accepted
            // when its declared effect matches (the spliced body's own
            // `call`/`times` re-checks it, R8/R9).
            if found.ty != concrete {
                return Err(quotation_argument_required_error(
                    ctx, span, name, concrete, found.ty,
                ));
            }
        } else {
            return Err(quotation_argument_required_error(
                ctx, span, name, concrete, found.ty,
            ));
        }
    }
    // Slice 10a (R11): the resolved `θ` is no longer discarded -- the back-edge
    // marker grounds the declared outputs through it (`inline_combinator`).
    Ok(subst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    /// Slice 10a (R1): a monomorphic word whose input is a `~` still counts as
    /// declaring a quotation parameter (accessor-routed), so it is inlined
    /// rather than lowered to a call.
    #[test]
    fn word_declares_quotation_parameter_recognizes_inline() {
        use crate::ast::TypedSlot;
        let inl = crate::ast::inline_quotation_type(vec![Type::I64], Vec::new());
        let w = WordDef {
            name: "w".to_string(),
            effect: StackEffect {
                inputs: vec![TypedSlot {
                    name: None,
                    ty: inl,
                }],
                outputs: Vec::new(),
            },
            body: WordBody::Terms { terms: Vec::new() },
            poly: None,
            declares_inline: false,
            module: 0,
            span: Span::default(),
            declared_globals: None,
        };
        assert!(word_declares_quotation_parameter(&w));
    }
    #[test]
    fn quotation_taking_word_mints_no_symbol() {
        // Slice 12 (R-A1, M-A): recognition is declared, not inferred. An
        // ordinary `[ ... ]` parameter no longer makes a word a combinator by
        // itself -- `apply` here declares no `inline`, so it is an ordinary
        // word (a real call, part D's territory), same as `plain`.
        // Constructed directly (not via an end-to-end build) so the test
        // discriminates the retired inference leg from an end-to-end "it
        // still builds" placebo: re-adding the `word_declares_quotation_parameter`
        // disjunct to `is_combinator` flips `apply` to `true` and this must fail.
        let src = ": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
                   : apply-inline inline ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
                   : plain ( i64 -- i64 ) 1 + ;\n";
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        let apply = module.words.iter().find(|w| w.name == "apply").unwrap();
        let apply_inline = module
            .words
            .iter()
            .find(|w| w.name == "apply-inline")
            .unwrap();
        let plain = module.words.iter().find(|w| w.name == "plain").unwrap();
        assert!(
            !is_combinator(apply),
            "`apply` declares no `inline`, so an ordinary `[ ... ]` parameter alone is not a combinator"
        );
        assert!(
            is_combinator(apply_inline),
            "the identical shape with `inline` declared is a combinator"
        );
        assert!(!is_combinator(plain), "`plain` is an ordinary word");
    }

    /// Slice 11 (R2): the declared flag alone makes a word always-spliced, with
    /// no quotation parameter anywhere in its effect. Constructed directly (an
    /// e2e build cannot discriminate the flag from a quotation parameter) and
    /// asserted both ways round, so the `|| word.declares_inline` disjunct is
    /// what the `true` rests on.
    #[test]
    fn is_combinator_true_for_inline_non_quotation_word() {
        use crate::ast::TypedSlot;
        let mut w = WordDef {
            name: "ClkDiv".to_string(),
            effect: StackEffect {
                inputs: Vec::new(),
                outputs: vec![TypedSlot {
                    name: None,
                    ty: Type::I64,
                }],
            },
            body: WordBody::Terms { terms: Vec::new() },
            poly: None,
            declares_inline: true,
            module: 0,
            span: Span::default(),
            declared_globals: None,
        };
        assert!(!word_declares_quotation_parameter(&w));
        assert!(is_combinator(&w), "`inline` alone makes a word spliced");
        w.declares_inline = false;
        assert!(
            !is_combinator(&w),
            "the same word without `inline` is an ordinary word"
        );
    }

    /// Slice 11 (R4): an `inline` word inherits the cycle rejection verbatim,
    /// under the reworded umbrella term (it need not take a quotation), and
    /// inherits R4's self-*tail* allowance too -- that shape lowers to a
    /// back-edge, so it is finite rather than a splice-forever cycle.
    #[test]
    fn check_inline_self_nontail_cycle_is_error() {
        let src = ": loopy inline ( i64 -- i64 ) 1 + loopy 2 * ;";
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        let err = check(&mut module).unwrap_err();
        assert_eq!(
            err,
            "error: an always-spliced word cannot be recursive (the inliner would splice it forever): `loopy` -> `loopy` (line 1, col 3)"
        );

        let tail_src = ": down inline ( i64 -- i64 ) dup 0 > ~[ 1 - down ] ~[ ] if ;";
        let tokens = lex(tail_src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module)
            .expect("a self-tail `inline` word is the R4-relaxed shape, not a cycle error");
    }
}
