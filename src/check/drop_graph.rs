use super::*;

/// R6/R11: the REPL's own whole-session call to `check_drop_overload_recursion`,
/// asked over every override currently live in the session (the new one
/// already included) and each one's *cached* `drop` call sites
/// (`check_def_collecting_drop_sites`, recorded once per override, at the
/// line that defined it) rather than a re-check of every body.
pub(crate) fn check_drop_overload_reachability(
    overrides: &[(StructId, &WordDef, &[Type])],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
) -> Result<(), String> {
    let words: Vec<&WordDef> = overrides.iter().map(|&(_, word, _)| word).collect();
    let overloads: HashMap<StructId, usize> = overrides
        .iter()
        .enumerate()
        .map(|(i, &(id, _, _))| (id, i))
        .collect();
    let dropped: Vec<Vec<Type>> = overrides
        .iter()
        .map(|&(_, _, sites)| sites.to_vec())
        .collect();
    check_drop_overload_recursion(&words, structs, enums, arrays, cells, &overloads, &dropped)
}

/// `main` is the program's entry point: nothing in the program calls it, so
/// a linear value in its declared effect either leaks past the program
/// boundary unnoticed (an output) or runs a destructor over an
/// uninitialised ABI register (an input). A non-empty Copy-typed effect on
/// `main` stays legal; only a non-Copy type in either side is rejected.
pub(super) fn check_main_effect(
    words: &[WordDef],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    let Some(main) = words.iter().find(|w| w.name == "main") else {
        return Ok(());
    };
    let offending = main
        .effect
        .inputs
        .iter()
        .chain(&main.effect.outputs)
        .map(|slot| slot.ty)
        .find(|ty| is_linear(*ty, structs, enums, arrays));
    let Some(ty) = offending else {
        return Ok(());
    };
    let span = word_span(main);
    Err(format!(
        "error: `main` (line {}) cannot declare a linear type `{}` in its stack effect\n  note: declared {}",
        span.line, ty, effect_str(&main.effect)
    ))
}

/// R1 (D2, D7): the callee names of every tail-position call in a word body.
///
/// Tail position is a purely *syntactic* property: a call is in tail position
/// iff it is the final term of a terms body or the final term of a clause
/// body. Any term after a call, arithmetic, a shuffle, a consumer, or another
/// call, breaks tail position. Output-equality with the declared outputs is a
/// *consequence* of this rule for a well-typed final call, not a second check.
///
/// Slice 10c (R-P1-1) adds the second way a term inherits tail position: a
/// splice. An always-spliced callee's body runs *in place of* the call, so its
/// own tail terms are the caller's, and a quotation literal the callee
/// `call`s in tail position is spliced there too. `[ ... ] call` at a tail is
/// the same thing one step shorter. A trailing `~[ t ] ~[ e ] if` hands tail
/// position to both arms through that rule rather than as a form of its own:
/// `if` splices, and `branch` tail-calls both quotation parameters. See
/// `TailWalk`.
///
/// Shared by the checker (R2 predicate, R3 tail-call graph); the lowerer
/// re-encodes the same syntactic rule via positional `tail` threading in
/// `lower_terms` (src/ir.rs), which a name list can't express. The two must
/// stay in lockstep if the tail rule changes.
pub(super) fn tail_position_calls<'a>(word: &'a WordDef, combs: &CombinatorIndex) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut walk = TailWalk::new(combs);
    match &word.body {
        WordBody::Terms { terms, .. } => {
            let binds = param_binds(terms, declared_input_count(word));
            walk.collect(terms, &binds, &mut out);
        }
        WordBody::Clauses(clauses) => {
            // A clause body's leading binds pop the dispatched variant's
            // payload, not the declared inputs, and a clause-bodied word is
            // never a combinator, so it has no parameter slots to forward.
            for clause in clauses {
                walk.collect(&clause.body, &HashMap::new(), &mut out);
            }
        }
    }
    out
}

/// Slice 10c (R-P1-5): the lowering-side entry point onto the same walk, for
/// the combinator splice, which holds a body and a name rather than a
/// `WordDef`.
///
/// R-P3-1b: it carries `has_self_tail_call`'s builtin-name refusal too. That
/// used to be omitted on the grounds that a builtin-named combinator could not
/// exist, since `check_operator`'s R11 guard rejects a quotation operand to any
/// builtin name before the env combinator lookup runs -- with a standing note
/// that nothing pinned it, so a narrowing of that guard would make the two
/// passes disagree about whether a splice is a loop. `branch` is exactly that
/// narrowing: it is the one builtin sanctioned to take quotation operands. So
/// the refusal is applied here as well, and neither pass may now read a
/// builtin name in tail position as a call to the enclosing word while the
/// other refuses to.
pub(crate) fn terms_tail_call_self(terms: &[Term], name: &str, combs: &CombinatorIndex) -> bool {
    if is_builtin_word_name(name) {
        return false;
    }
    let binds = match combs.get(name) {
        Some(entry) => param_binds(terms, entry.inputs),
        None => HashMap::new(),
    };
    let mut out = Vec::new();
    TailWalk::new(combs).collect(terms, &binds, &mut out);
    out.contains(&name)
}

/// Slice 10c (R-P1-1): which of always-spliced `name`'s declared parameter
/// slots hold a quotation it `call`s in tail position — the same set the tail
/// walk computes, for a consumer that wants the set itself rather than the
/// callee names a walk reaches. Empty for a name that is not an always-spliced
/// word, and for one whose provenance the walk declines to follow.
///
/// The argument-site literal check reads it to decide, per parameter, whether
/// the literal it is about to walk really occupies the caller's tail position:
/// `if`'s two arms do when the `if` does, `times`' body never does.
pub(crate) fn tail_called_param_slots(name: &str, combs: &CombinatorIndex) -> Vec<usize> {
    TailWalk::new(combs)
        .tail_called_params(name)
        .map(|(_, slots)| slots)
        .unwrap_or_default()
}

fn declared_input_count(word: &WordDef) -> usize {
    match word.poly.as_ref() {
        Some(sig) => sig.inputs.len(),
        None => word.effect.inputs.len(),
    }
}

/// What a tail position reaches: a callee name, or a declared parameter slot
/// of the word being walked (that slot's quotation runs in tail position, so a
/// caller's literal passed into it does too).
enum TailHit<'t> {
    Name(&'t str),
    Param(usize),
}

/// What a call site statically hands to one declared input slot.
enum Arg<'t> {
    /// A quotation literal written at the call site: its body is visible here.
    Literal(&'t [Term]),
    /// The walked word's own declared input slot, reached through the local
    /// its leading `| ... |` bound it to.
    Param(usize),
}

/// **INV-INLINE-COMBINATOR.** A quotation-taking word is always inlined
/// (spliced) at each call site and mints no `IrFunc`; it has no opaque call
/// form. Its declared output row is discovered by forward checking of the
/// spliced terms, never solved for by row unification.
///
/// This walk rests on that invariant twice over: it reads a callee's body
/// because there is only ever one, spliced, form of it, and it treats that
/// body's tail terms as the caller's because the splice really is in place.
/// Slice 7b (first-class runtime quotations) is where the invariant breaks;
/// the walk must be revisited there, together with the combinator splice in
/// `ir::func_builder::calls`.
///
/// The walk is conservative in one direction only: it declines (reports no
/// tail call) wherever provenance is not syntactically visible -- an ambiguous
/// name, a forwarding cycle, a quotation reached through anything but a
/// literal or a declared parameter. Declining costs a loop transform, never
/// correctness.
struct TailWalk<'a> {
    combs: &'a CombinatorIndex,
    /// The combinators whose tail-called-parameter sets are being computed.
    /// The inline-always invariant proves *lowering* terminates; it does not
    /// prove this *static* closure does, because the closure follows edges
    /// between distinct combinators and two of them mutually forwarding a
    /// tail-called parameter would loop `C -> D -> C`.
    visiting: Vec<&'a str>,
}

impl<'a> TailWalk<'a> {
    fn new(combs: &'a CombinatorIndex) -> Self {
        Self {
            combs,
            visiting: Vec::new(),
        }
    }

    fn collect<'t>(
        &mut self,
        terms: &'t [Term],
        binds: &HashMap<&'t str, usize>,
        out: &mut Vec<&'t str>,
    ) {
        let mut hits = Vec::new();
        self.walk(terms, binds, &mut hits);
        out.extend(hits.into_iter().filter_map(|hit| match hit {
            TailHit::Name(name) => Some(name),
            TailHit::Param(_) => None,
        }));
    }

    fn walk<'t>(
        &mut self,
        terms: &'t [Term],
        binds: &HashMap<&'t str, usize>,
        out: &mut Vec<TailHit<'t>>,
    ) {
        let Some(last) = terms.last() else {
            return;
        };
        let before = &terms[..terms.len() - 1];
        if let TermKind::Call(name) = &last.kind {
            out.push(TailHit::Name(name.as_str()));
            // Which of the callee's argument slots inherit this tail
            // position: `call`'s single quotation operand, `branch`'s two,
            // or an always-spliced callee's tail-`call`ed parameter slots.
            //
            // Slice 10c (R-P3-5a): `branch` is *seeded*, taking over the
            // role the deleted `TermKind::If` descent played. It is a
            // primitive with no walkable body, so the closure below cannot
            // compute its tail-called-parameter set by inspection; without
            // the seed `if`'s own set computes empty and every caller that
            // recurses through a branch arm silently loses its loop.
            let inherits = match name.as_str() {
                "call" => Some((1, vec![0])),
                "branch" => Some((3, vec![1, 2])),
                _ => self.tail_called_params(name),
            };
            let Some((inputs, slots)) = inherits else {
                return;
            };
            let args = visible_args(before, binds);
            for slot in slots {
                match args.get(inputs - 1 - slot) {
                    Some(Arg::Literal(body)) => self.walk(body, binds, out),
                    Some(Arg::Param(param)) => out.push(TailHit::Param(*param)),
                    None => {}
                }
            }
        }
    }

    /// R-P1-1: `name`'s declared input count, and which of its quotation
    /// parameter slots it `call`s in tail position (directly, or transitively
    /// by forwarding into another combinator's tail-called slot). `None`
    /// declines: not an always-spliced word, an ambiguous name (R-P1-4), or a
    /// forwarding cycle.
    fn tail_called_params(&mut self, name: &str) -> Option<(usize, Vec<usize>)> {
        let combs = self.combs;
        let (key, entry) = combs.get_key_value(name)?;
        if entry.ambiguous || self.visiting.contains(&key.as_str()) {
            return None;
        }
        self.visiting.push(key.as_str());
        let binds = param_binds(&entry.terms, entry.inputs);
        let mut hits = Vec::new();
        self.walk(&entry.terms, &binds, &mut hits);
        self.visiting.pop();
        let slots = hits
            .into_iter()
            .filter_map(|hit| match hit {
                TailHit::Param(slot) => Some(slot),
                TailHit::Name(_) => None,
            })
            .collect();
        Some((entry.inputs, slots))
    }
}

/// The arguments a call receives, top of stack first, as far as they are
/// statically visible. The scan stops at the first term that does not push
/// exactly one value of known provenance, so a slot deeper than the returned
/// run is undecidable and its caller declines (R-P1-3): a quotation reached
/// through a computed value has no body to walk, and lowering sends it to
/// `lower_indirect_call` rather than the splice branch.
fn visible_args<'t>(before: &'t [Term], binds: &HashMap<&'t str, usize>) -> Vec<Arg<'t>> {
    let mut out = Vec::new();
    for term in before.iter().rev() {
        match &term.kind {
            TermKind::Quotation(body, _, _) => out.push(Arg::Literal(body)),
            TermKind::Call(name) => match binds.get(name.as_str()) {
                Some(&slot) => out.push(Arg::Param(slot)),
                None => break,
            },
            _ => break,
        }
    }
    out
}

/// The leading `| ... |` binds of a body, mapping each bound name back to the
/// declared input slot it took. Only the leading run is read: a combinator
/// names its quotation parameters before doing anything else (every one in
/// `lib/` does), and a bind after any other term pops a computed value whose
/// provenance this syntactic pass cannot follow.
fn param_binds(terms: &[Term], inputs: usize) -> HashMap<&str, usize> {
    let mut map = HashMap::new();
    let mut remaining = inputs;
    for term in terms {
        let TermKind::Bind(names) = &term.kind else {
            break;
        };
        if names.len() > remaining {
            break;
        }
        // Leftmost name takes the deepest of the popped values.
        remaining -= names.len();
        for (i, name) in names.iter().enumerate() {
            map.insert(name.as_str(), remaining + i);
        }
    }
    map
}

/// R2 (M1): whether a word contains at least one tail-position call to itself.
/// The lowerer uses this to decide whether to build the loop shape at all.
///
/// A word whose own name is a builtin's never self-tail-calls on a bare name
/// match. A builtin name in tail position resolves against the builtin table
/// first, so it need not mean the enclosing word: `: drop ( T -- )`'s trailing
/// `drop` disposes whatever is on top (the dogfood's own
/// `| f | f File> close drop ;` closes the fd rather than looping), and since
/// slice 8a made every builtin name overloadable the same applies throughout,
/// e.g. `: < ( Vec2 Vec2 -- bool ) | a b | a Vec2>x b Vec2>x < ;` ends in the
/// *builtin* `<` on two `i64`s. Treating either as a back-edge opens loop
/// machinery whose phi operands never arrive, and lowering then panics on the
/// missing header.
///
/// A genuine self-recursive overload keeps its meaning but loses its loop: it
/// lowers as an ordinary recursive `Instr::Call`, so it computes the right
/// answer and then overflows the stack once driven deep (measured: a segfault
/// around 1e6 frames, where the identical body under a non-builtin name loops
/// in constant space). Renaming a word to a builtin name therefore changes its
/// depth behaviour silently, which is a poor fit for a language whose point is
/// turning Forth's silent failures into errors. Telling the two apart needs
/// the call site's resolved operand types: the checker does have those in
/// `check_term`, so a diagnostic (or a resolution-aware self-call test) is
/// reachable future work; this syntactic pass, which runs over a `WordDef`
/// alone, is simply the wrong place for it.
///
/// Slice 10c (R-P1-5): the one predicate every consumer shares -- the two
/// syntactic passes, the per-word build gate, the REPL and destructor lowering
/// paths, and the checker's `splice_tail` -- so check and lowering agree on
/// whether a splice is a loop by construction rather than by luck.
pub(crate) fn has_self_tail_call(word: &WordDef, combs: &CombinatorIndex) -> bool {
    !is_builtin_word_name(&word.name)
        && tail_position_calls(word, combs)
            .iter()
            .any(|&callee| callee == word.name)
}

/// R3/R4 (D3, X1): build the whole-module tail-call graph (an edge `A -> B`
/// iff `A` has a tail-position call to user word `B`) and reject any cycle of
/// length >= 2. A self-loop (`A -> A`) is tier-1 self-tail-recursion and
/// allowed; only mutual cycles are the error. Builtins, generated words, and
/// non-tail calls contribute no edge, so a pair of words that mutually call
/// each other in non-tail position never false-positives.
///
/// Slice 8b (D1): a recognized `drop` overload's exclusion here is unrelated
/// to D1's own drop-import-visibility gate (`check_drop_import_visibility`,
/// run later, per call site, inside `check_shuffle`'s `"drop"` arm) -- this
/// pass only keeps a scalar `drop` inside an override's own body from being
/// misread as a tail call to the override itself.
pub(super) fn check_tail_call_cycles(
    words: &[WordDef],
    drop_overload_indices: &HashSet<usize>,
    combs: &CombinatorIndex,
) -> Result<(), String> {
    // A recognized `drop` overload is not callable by name (`check_shuffle`'s
    // `"drop"` arm intercepts every call site first), so it contributes no
    // edge in either direction: a body's trailing `drop` of a scalar would
    // otherwise register a tail call *to* the overload and fabricate a cycle.
    // Keyed by registry membership, not the literal name, matching every
    // other exclusion in this pass.
    //
    // Slice 8a generalizes that to every builtin name, for the same reason
    // `drop` needed it: this pass runs before any body is checked, so it has
    // only names, and a tail-position `<` is far more often the builtin on two
    // scalars than a call to a `Vec2 <` overload that happens to share the
    // name. Crediting it as an edge rejects valid programs outright -- a word
    // ending in `<` beside any `<` overload was reported as `mutual tail
    // recursion`. The cost is that a real mutual cycle between two
    // builtin-named overloads is no longer rejected here; it compiles as
    // ordinary mutual recursion (correct, but without the tier-1 loop shape,
    // so it will overflow the stack when driven deep), which is the same
    // trade `has_self_tail_call` documents. Deciding these apart needs each
    // call site's resolved candidate, which exists only after the body walk
    // this pass precedes.
    //
    // The identical hazard exists for an *ordinary* overloaded name once R1
    // admits two candidates sharing it: `name_to_idx` maps a name to one word
    // index, so building it via a plain `.collect()` over every non-builtin
    // word silently kept only the last-indexed candidate, and a tail call
    // meant for the other candidate was credited as an edge to it instead --
    //   : show ( i64 -- ) . ;
    //   : p ( Vec2 -- ) | v | v Vec2>x show ;   -- tail-calls show(i64), a leaf
    //   : show ( Vec2 -- ) | v | v p ;          -- tail-calls p
    // has no real cycle (`show(i64)` calls nothing further), but the fabricated
    // edge `p -> show(Vec2) -> p` reported `mutual tail recursion`. Any name
    // with more than one non-drop-overload candidate is excluded from
    // `name_to_idx` for the same reason a builtin name is: which candidate a
    // bare tail call reaches cannot be decided here. The same accepted cost
    // applies -- a genuine mutual cycle purely between two candidates of one
    // overloaded name is no longer caught by this pass -- and for the same
    // reason: the call site's resolved candidate exists only after the body
    // walk this pass precedes.
    let mut name_counts: HashMap<&str, usize> = HashMap::new();
    for (i, w) in words.iter().enumerate() {
        if !drop_overload_indices.contains(&i) {
            *name_counts.entry(w.name.as_str()).or_insert(0) += 1;
        }
    }
    let name_to_idx: HashMap<&str, usize> = words
        .iter()
        .enumerate()
        .filter(|(i, w)| {
            !drop_overload_indices.contains(i)
                && !is_builtin_word_name(&w.name)
                && name_counts.get(w.name.as_str()) == Some(&1)
        })
        .map(|(i, w)| (w.name.as_str(), i))
        .collect();

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
    for (i, word) in words.iter().enumerate() {
        for callee in tail_position_calls(word, combs) {
            if let Some(&j) = name_to_idx.get(callee) {
                if !adj[i].contains(&j) {
                    adj[i].push(j);
                }
            }
        }
    }

    let mut color = vec![0u8; words.len()];
    let mut path: Vec<usize> = Vec::new();
    for start in 0..words.len() {
        if color[start] == 0 {
            if let Some(cycle) = find_tail_cycle(start, &adj, &mut color, &mut path) {
                return Err(mutual_tail_recursion_error(words, &cycle));
            }
        }
    }
    Ok(())
}

/// DFS from `u` over the tail-call graph, returning the members (in order) of
/// the first cycle of length >= 2 reached. A self-edge (`v == u`) is skipped:
/// tier-1 self-tail-recursion is allowed. `color`: 0 unvisited, 1 on the
/// current path, 2 finished.
fn find_tail_cycle(
    u: usize,
    adj: &[Vec<usize>],
    color: &mut [u8],
    path: &mut Vec<usize>,
) -> Option<Vec<usize>> {
    color[u] = 1;
    path.push(u);
    for &v in &adj[u] {
        if v == u {
            continue;
        }
        if color[v] == 1 {
            let start = path.iter().position(|&x| x == v).unwrap();
            return Some(path[start..].to_vec());
        }
        if color[v] == 0 {
            if let Some(cycle) = find_tail_cycle(v, adj, color, path) {
                return Some(cycle);
            }
        }
    }
    path.pop();
    color[u] = 2;
    None
}

/// X1: a located mutual-tail-recursion error naming the cycle members in
/// order, closing the loop back to the first (e.g. `` `a` -> `b` -> `a` ``).
fn mutual_tail_recursion_error(words: &[WordDef], cycle: &[usize]) -> String {
    let mut chain: Vec<&str> = cycle
        .iter()
        .map(|&i| crate::resolve::demangle_word(words[i].name.as_str()))
        .collect();
    chain.push(chain[0]);
    let rendered = chain
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(" -> ");
    let span = word_span(&words[cycle[0]]);
    format!(
        "error: mutual tail recursion {} (line {}, col {})",
        rendered, span.line, span.col
    )
}

/// R6 (D4, slice 8b): reject a `drop` overload that can reach itself. Per
/// override, the question is only whether `drop@T`'s own word is reachable
/// from itself through any sequence of calls, direct or indirect -- a bare
/// self-call is the cycle of length one, a chain through helpers the general
/// case, and a `drop` of some *other* aggregate merely containing a `T` is
/// the same question again, since disposing that aggregate runs `T`'s
/// override through its own generic field glue.
///
/// This cannot be a sibling of `check_tail_call_cycles`, run before body
/// checking: resolving *which* override a `drop` call site dispatches to
/// needs the operand's static type, and nothing computes that before
/// `check_word`'s per-term stack simulation. A purely syntactic pass over
/// callee names (`check_tail_call_cycles`'s own shape) could not tell
/// `drop@File` from the `drop` of the `i64` that `close` returns, and so
/// would reject the dogfood outright.
///
/// **Known, accepted limitation:** reachability is not data-flow, so it is
/// context-insensitive. A helper called from `drop@T` that is *separately*
/// reachable back to `drop@T` only down a branch never taken from there
/// still reads as a cycle -- the same false positive the tail-cycle pass
/// already accepts, with the same remedy: factor out a distinct helper.
pub(super) fn check_drop_overload_recursion(
    words: &[&WordDef],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    overloads: &HashMap<StructId, usize>,
    dropped: &[Vec<Type>],
) -> Result<(), String> {
    if overloads.is_empty() {
        return Ok(());
    }
    let adj = drop_reachability_graph(words, structs, enums, arrays, cells, overloads, dropped);
    // Sorted by struct id, so a program with two offending overloads always
    // reports the same one.
    let mut targets: Vec<(StructId, usize)> = overloads.iter().map(|(&id, &i)| (id, i)).collect();
    targets.sort_by_key(|(id, _)| id.index());
    for (id, idx) in targets {
        let mut visited = vec![false; words.len()];
        visited[idx] = true;
        let mut chain = vec![idx];
        if reaches_start(idx, &adj, &mut visited, &mut chain) {
            return Err(recursive_drop_overload_error(
                words, structs, overloads, id, &chain,
            ));
        }
    }
    Ok(())
}

/// R6: the whole-program graph the reachability question is asked over. Two
/// kinds of edge out of a word `A`:
///
/// - an ordinary call anywhere in `A`'s body resolving to a user word `B`
///   (**any** position, unlike `tail_position_calls`, which only ever reads
///   `terms.last()`);
/// - `A -> drop@T` for a `drop` call site in `A` whose recorded operand type
///   either *is* the overridden struct `T`, or is an aggregate with no
///   override of its own whose linear fields reach `T` through ordinary,
///   non-overridden composition.
///
/// Every edge is resolved through the `StructId`-keyed override table, never
/// through a name-keyed map: the literal name `"drop"` is shared by every
/// override and says nothing about which one a site dispatches to.
fn drop_reachability_graph(
    words: &[&WordDef],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    overloads: &HashMap<StructId, usize>,
    dropped: &[Vec<Type>],
) -> Vec<Vec<usize>> {
    // An override is not callable by name (every `drop` call site is
    // intercepted before name resolution reaches `env`), so it contributes no
    // name edge in either direction: its only incoming edges are `drop` sites.
    let overload_words: HashSet<usize> = overloads.values().copied().collect();
    let name_to_idx: HashMap<&str, usize> = words
        .iter()
        .enumerate()
        .filter(|(i, _)| !overload_words.contains(i))
        .map(|(i, w)| (w.name.as_str(), i))
        .collect();

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
    for (i, word) in words.iter().enumerate() {
        for callee in all_calls(&word.body) {
            if let Some(&j) = name_to_idx.get(callee) {
                if !adj[i].contains(&j) {
                    adj[i].push(j);
                }
            }
        }
        for &ty in &dropped[i] {
            let mut targets = Vec::new();
            collect_drop_targets(
                ty,
                structs,
                enums,
                arrays,
                cells,
                overloads,
                &mut Vec::new(),
                &mut targets,
            );
            for j in targets {
                if !adj[i].contains(&j) {
                    adj[i].push(j);
                }
            }
        }
    }
    adj
}

/// R6: the override bodies one `drop` call site can run, given its operand
/// type. A check-side fold over `StructDecl` fields, shaped like `is_copy`'s,
/// because there is no `StructLayout` to walk yet -- `build_registries` runs
/// inside `ir::lower`, after `check` entirely.
///
/// An overridden struct is where the walk stops, the same boundary R7 applies
/// to the fused-loop search in `ir::expand_path`, and that one stop covers
/// both of R6's cases:
///
/// - at the root, it *is* case (a): dropping an overridden `B` runs `B`'s own
///   body, so the edge goes there and reachability continues from `B`'s own
///   recorded call sites during the DFS. Descending into `B`'s fields as well
///   would inspect field glue that never runs, and fabricate an edge.
/// - below the root, it is case (b)'s boundary: a non-overridden aggregate is
///   disposed by generic field glue, which calls each linear field's own
///   destructor, so every override reachable through that composition really
///   does run -- but the composition stops at the first override, for the
///   same reason.
///
/// A `Copy` type is a dead end because nothing disposes it at all. `seen` is
/// monotone (never popped) since the answer is a *set* of reachable
/// overrides, not a path, and a `^T` payload may close a type cycle the
/// struct and enum registries cannot.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_drop_targets(
    ty: Type,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    overloads: &HashMap<StructId, usize>,
    seen: &mut Vec<Type>,
    found: &mut Vec<usize>,
) {
    if is_copy(ty, structs, enums, arrays) || seen.contains(&ty) {
        return;
    }
    seen.push(ty);
    let descend = |field: Type, seen: &mut Vec<Type>, found: &mut Vec<usize>| {
        collect_drop_targets(field, structs, enums, arrays, cells, overloads, seen, found)
    };
    match ty {
        Type::Struct(id, _) => {
            if let Some(&idx) = overloads.get(&id) {
                if !found.contains(&idx) {
                    found.push(idx);
                }
                return;
            }
            for (_, field_ty) in &structs[id.index()].fields {
                descend(*field_ty, seen, found);
            }
        }
        Type::Enum(id, _) => {
            for variant in &enums[id.index()].variants {
                for (_, field_ty) in &variant.fields {
                    descend(*field_ty, seen, found);
                }
            }
        }
        Type::Array(id, _) => descend(arrays[id.index()].element, seen, found),
        Type::OwnedCell(id, _) => descend(cells[id.index()].payload, seen, found),
        _ => {}
    }
}

/// R6: every callee name a body mentions, in any position -- the whole-body
/// sibling of `tail_position_calls`, which only ever reads `terms.last()`.
/// Both `if` arms and every clause body are visited.
///
/// A local's own name reads as a `Call` term too, so a local sharing a word's
/// name contributes an edge that no call justifies. That over-approximation
/// can only add edges, never lose one, and is the same one
/// `check_tail_call_cycles` already lives with.
pub(super) fn all_calls(body: &WordBody) -> Vec<&str> {
    let mut out = Vec::new();
    match body {
        WordBody::Terms { terms } => collect_all_calls(terms, &mut out),
        WordBody::Clauses(clauses) => {
            for clause in clauses {
                collect_all_calls(&clause.body, &mut out);
            }
        }
    }
    out
}

fn collect_all_calls<'a>(terms: &'a [Term], out: &mut Vec<&'a str>) {
    for term in terms {
        match &term.kind {
            TermKind::Call(name) => out.push(name.as_str()),
            // Slice 10c: a branch arm is a quotation literal now, so a call
            // written inside one is an ordinary call of the enclosing body.
            // Without this descent `check_combinator_cycles` stops seeing a
            // combinator's own name inside its arms, and a *non-tail* self-call
            // there -- which the inliner would splice forever -- goes from a
            // located rejection to a compiler stack overflow.
            TermKind::Quotation(inner, _, _) => collect_all_calls(inner, out),
            _ => {}
        }
    }
}

/// Whether `start` is reachable from the last word on `chain`, growing
/// `chain` into the route that gets there. A node is marked on the way down
/// and never unmarked: if it could reach `start`, the search from it already
/// said so, so skipping it on a later branch cannot lose a cycle.
fn reaches_start(
    start: usize,
    adj: &[Vec<usize>],
    visited: &mut [bool],
    chain: &mut Vec<usize>,
) -> bool {
    let u = *chain.last().expect("reachability chain is never empty");
    for &v in &adj[u] {
        if v == start {
            return true;
        }
        if !visited[v] {
            visited[v] = true;
            chain.push(v);
            if reaches_start(start, adj, visited, chain) {
                return true;
            }
            chain.pop();
        }
    }
    false
}

/// R6: a located error naming the whole cycle in order, closing back to the
/// override it started from, and naming `T>` as the remedy -- modeled on
/// `mutual_tail_recursion_error`'s shape. An override has no callable name of
/// its own, so it is rendered as the declaration the user wrote.
fn recursive_drop_overload_error(
    words: &[&WordDef],
    structs: &[StructDecl],
    overloads: &HashMap<StructId, usize>,
    id: StructId,
    chain: &[usize],
) -> String {
    let render = |i: usize| match overloads.iter().find(|(_, &w)| w == i) {
        Some((sid, _)) => format!("`drop ( {} -- )`", structs[sid.index()].name),
        None => format!("`{}`", words[i].name),
    };
    let mut rendered: Vec<String> = chain.iter().map(|&i| render(i)).collect();
    rendered.push(render(chain[0]));
    let name = &structs[id.index()].name;
    let span = word_span(words[overloads[&id]]);
    format!(
        "error: recursive `drop` overload for `{}`: {} (line {}, col {})\n  a `drop` body cannot dispose its own receiver, directly or through any chain of calls; destructure it with `{}>` and dispose the fields instead",
        name,
        rendered.join(" -> "),
        span.line,
        span.col,
        name
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module)
    }
    /// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
    /// primitive in Slice 8c: an ordinary one-field struct with a `drop`
    /// overload, so it is linear for the same reason any resource is (R3),
    /// not by any compiler-known bit. Always the first struct in a source
    /// string that uses it, so every other struct's `StructId` shifts up by
    /// one relative to a spy-free program.
    const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy> . ;\n";
    fn first_word(src: &str) -> WordDef {
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        module.words.into_iter().next().unwrap()
    }
    #[test]
    fn check_drop_body_must_consume_linear_fields() {
        // Criterion 12/R5/R9: an override body is checked like any other word
        // body, so a resource holding a linear field is already forced to
        // account for it -- no scalar-only restriction, and no new check.
        let src = format!(
            "{SPY_DEF}type: Inner s Spy ; type: Res i Inner ; \
             : drop ( Res -- ) | r | r Res> drop ; \
             : main ( -- ) 1 Spy Inner Res drop ;"
        );
        check_src(&src).unwrap();

        let forgotten = format!(
            "{SPY_DEF}type: Inner s Spy ; type: Res i Inner ; \
             : drop ( Res -- ) | r | ; \
             : main ( -- ) 1 Spy Inner Res drop ;"
        );
        let err = check_src(&forgotten).unwrap_err();
        assert!(
            err.contains("linear value `r` is never consumed"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_drop_body_direct_self_recursion_is_error() {
        // Criterion 8/R6: a `drop` body that drops its own receiver is a
        // cycle of length one. The message names the chain and `File>` as the
        // remedy, since destructuring is what the user has to do instead.
        let src = "type: File fd i64 ; : drop ( File -- ) drop ; : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`File>`"), "unexpected message: {err}");
    }
    #[test]
    fn check_drop_body_indirect_self_recursion_through_helper_is_error() {
        // Criterion 9/R6: the same rejection through one helper word, which is
        // why this is reachability over the whole call graph rather than a
        // self-call test. The chain names the helper it goes through.
        let src = "type: File fd i64 ; \
                   : shut ( File -- ) drop ; \
                   : drop ( File -- ) shut ; \
                   : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`shut`"), "unexpected message: {err}");
        assert!(err.contains("`File>`"), "unexpected message: {err}");
    }
    #[test]
    fn check_drop_body_recursion_inside_an_if_arm_is_error() {
        // R6: the call graph is over calls in *any* position, so the walker
        // has to visit both `if` arms and every term after them --
        // `tail_position_calls` only ever reads `terms.last()`, and would see
        // neither of these.
        let src = "type: File fd i64 ; \
                   : shut ( File -- ) drop ; \
                   : drop ( File -- ) | f | true ~[ f shut ] ~[ f shut ] if 1 . ; \
                   : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`shut`"), "unexpected message: {err}");
    }
    #[test]
    fn check_drop_of_copy_scalar_inside_drop_body_is_not_a_cycle() {
        // Criterion 10/R6: the dogfood's own shape. Its body ends in a `drop`
        // of the `Copy` `i64` its extern call returns, which a name-keyed
        // graph would read as a call to the override itself and reject.
        let src = "type: File fd i64 ; \
                   : drop ( File -- ) | f | f File> drop ; \
                   : main ( -- ) 1 File drop ;";
        check_src(src).unwrap();
    }
    #[test]
    fn check_drop_of_different_resource_inside_another_drop_body_is_ok() {
        // Criterion 11/R6: dispatch is per struct id, so `drop@A` disposing a
        // `B` is an edge to `drop@B` and nothing more -- no cycle, since
        // `drop@B` reaches nothing back.
        let src = "type: A x i64 ; type: B y i64 ; \
                   : drop ( A -- ) | a | a A>x B drop ; \
                   : drop ( B -- ) | b | b B>y drop ; \
                   : main ( -- ) 1 A drop ;";
        check_src(src).unwrap();
    }
    #[test]
    fn check_drop_body_recursion_through_a_containing_aggregate_is_error() {
        // Criterion 21/R6 case (b): `Box` has no override, so dropping one
        // runs generic field glue that disposes its `File` field through
        // `File`'s own override -- unbounded recursion at runtime, invisible
        // to a graph that only looked at directly dropped types.
        let src = "type: File fd i64 ; type: Box f File ; \
                   : drop ( File -- ) | f | f Box drop ; \
                   : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`File>`"), "unexpected message: {err}");
    }
    #[test]
    fn check_drop_of_an_overridden_aggregate_disposing_its_overridden_field_is_not_a_cycle() {
        // R6: case (b) must not fire when the dropped type is *itself*
        // overridden -- `B`'s own body is its whole disposal, so the graph
        // must reflect only the `drop` calls that body actually makes, never
        // a synthesized walk of its fields. D3 requires `B`'s override to
        // dispose its drop-overloaded `a` field with a real `drop` call
        // (destructuring it apart from calling `drop` would itself be D3's
        // own rejection), forming exactly one edge, `B` -> `A`; since `A`'s
        // own override never calls back into `B`, this is not a cycle.
        let src = "type: A x i64 ; type: B a A ; \
                   : drop ( A -- ) | a | a A>x drop ; \
                   : drop ( B -- ) | b | b B>a drop ; \
                   : main ( -- ) 1 A B drop ;";
        check_src(src).unwrap();
    }
    #[test]
    fn collect_drop_targets_stops_descending_at_an_overridden_struct() {
        // R6 case (b), on `collect_drop_targets` directly. Post-D3, no legal
        // Sooth program can discriminate this rule any more (see the
        // `check_src`-based test above): disposing an overridden field always
        // requires a real `drop` call, which already contributes the same
        // edge a field-walk would synthesize, so a mutated (fields-walking)
        // version of this function passes every `check_src` test just as the
        // correct one does. Hand-build the registries instead: `B` overrides
        // `drop` and has a field of type `A`, which also overrides `drop`.
        // Walking `B`'s targets must add `B`'s own override and nothing else
        // -- never descend into the overridden field to add `A`'s too.
        let a = StructDecl {
            name: "A".to_string(),
            name_static: "A",
            fields: vec![("x".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: true,
            is_bundle: false,
            module: 0,
        };
        let b = StructDecl {
            name: "B".to_string(),
            name_static: "B",
            fields: vec![("a".to_string(), Type::Struct(StructId::from_index(0), "A"))],
            span: Span::default(),
            has_drop_overload: true,
            is_bundle: false,
            module: 0,
        };
        let structs = vec![a, b];
        let mut overloads = HashMap::new();
        overloads.insert(StructId::from_index(0), 0usize);
        overloads.insert(StructId::from_index(1), 1usize);

        let mut found = Vec::new();
        collect_drop_targets(
            Type::Struct(StructId::from_index(1), "B"),
            &structs,
            &[],
            &[],
            &[],
            &overloads,
            &mut Vec::new(),
            &mut found,
        );

        assert_eq!(
            found,
            vec![1],
            "walking B's targets must stop at B's own override, never also \
             descend into its overridden `A` field"
        );
    }
    #[test]
    fn check_drop_body_sharing_a_helper_with_another_word_is_not_a_cycle() {
        // R6: reachability is over the whole call graph, so a helper called
        // both from an override and from elsewhere must not read as a cycle
        // just for being reachable from two places.
        let src = "type: File fd i64 ; \
                   : show ( i64 -- ) . ; \
                   : drop ( File -- ) | f | f File> show ; \
                   : main ( -- ) 1 File drop 2 show ;";
        check_src(src).unwrap();
    }
    #[test]
    fn check_a_word_named_drop_contributes_no_tail_call_edge() {
        // A `drop` term never resolves to a user word (`check_shuffle`
        // intercepts it first), so the tail-call graph must not treat one as a
        // call to a `drop` overload: `helper`'s trailing `drop` of an `i64`
        // would otherwise close a fabricated mutual cycle with the override
        // that tail-calls `helper`.
        let src = "type: T x i64 ; \
                   : helper ( i64 -- ) drop ; \
                   : drop ( T -- ) | t | t T>x helper ; \
                   : main ( -- ) 1 T drop ;";
        check_src(src).unwrap();
    }
    #[test]
    fn check_main_linear_output_is_error() {
        let err = check_src(&format!("{SPY_DEF}: main ( -- Spy ) 7 Spy ;")).unwrap_err();
        assert!(
            err.contains("cannot declare a linear type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }
    #[test]
    fn check_main_linear_input_is_error() {
        let err = check_src(&format!("{SPY_DEF}: main ( Spy -- ) | s | s drop ;")).unwrap_err();
        assert!(
            err.contains("cannot declare a linear type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }
    #[test]
    fn check_main_copy_effect_is_ok() {
        check_src(": main ( i64 -- i64 ) 1 + ;").unwrap();
        // The misfire risk is `is_copy`'s recursive struct/enum arms, not the
        // scalar arm: a Copy struct in `main`'s effect must not be rejected.
        check_src("type: P a i64 b i64 ; : main ( P -- ) P> drop drop ;").unwrap();
    }
    #[test]
    fn tail_position_final_self_call_is_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec ;");
        assert_eq!(
            tail_position_calls(&w, &CombinatorIndex::new()),
            vec!["rec"]
        );
        assert!(has_self_tail_call(&w, &CombinatorIndex::new()));
    }
    #[test]
    fn tail_position_trailing_arithmetic_is_not_tail() {
        // `rec *`: the final term is `*`, so the self-call is not in tail
        // position (classic non-tail recursion).
        let w = first_word(": rec ( i64 -- i64 ) rec * ;");
        assert_eq!(tail_position_calls(&w, &CombinatorIndex::new()), vec!["*"]);
        assert!(!has_self_tail_call(&w, &CombinatorIndex::new()));
    }
    #[test]
    fn tail_position_trailing_swap_is_not_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec swap ;");
        assert_eq!(
            tail_position_calls(&w, &CombinatorIndex::new()),
            vec!["swap"]
        );
        assert!(!has_self_tail_call(&w, &CombinatorIndex::new()));
    }
    #[test]
    fn tail_position_trailing_drop_is_not_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec drop ;");
        assert_eq!(
            tail_position_calls(&w, &CombinatorIndex::new()),
            vec!["drop"]
        );
        assert!(!has_self_tail_call(&w, &CombinatorIndex::new()));
    }
    #[test]
    fn tail_position_builtin_named_word_trailing_its_own_name_is_not_self_tail() {
        // Slice 8a made every builtin name overloadable, so a builtin-named
        // word ending in that same name is resolving against the builtin
        // table, not recursing: `<` here compares the two extracted `i64`s.
        // `tail_position_calls` still reports the name (it is syntactic);
        // only the self-call conclusion changes.
        let w = first_word(
            "type: Vec2 x i64 y i64 ; : < ( Vec2 Vec2 -- bool ) | a b | a Vec2>x b Vec2>x < ;",
        );
        assert_eq!(tail_position_calls(&w, &CombinatorIndex::new()), vec!["<"]);
        assert!(!has_self_tail_call(&w, &CombinatorIndex::new()));
    }
    #[test]
    fn tail_position_both_terminal_if_arms_are_tail() {
        // A terminal `if` hands tail position to the last term of both arms.
        // Slice 10c (R-P3-5a): that is no longer a grammar rule but the
        // combinator walk -- `if` is a `lib/` word whose tail-called-parameter
        // set is both branch quotations, seeded from `branch` -- so the index
        // has to carry the real `if`, not be empty.
        let src = ": rec ( i64 -- i64 ) dup 0 > ~[ rec ] ~[ rec ] if ;";
        let module = parse(&lex(src).unwrap()).unwrap();
        let combs = combinator_index(module.words.iter());
        let w = module.words.iter().find(|w| w.name == "rec").unwrap();
        // The tail callees are `if` itself and, through both of its
        // tail-called parameter slots, the self-call in each arm.
        assert_eq!(tail_position_calls(w, &combs), vec!["if", "rec", "rec"]);
        assert!(has_self_tail_call(w, &combs));
    }
    #[test]
    fn tail_position_non_terminal_if_self_call_is_not_tail() {
        // The `if` is followed by more terms, so it is non-terminal and its
        // arms are not in tail position.
        let w = first_word(": rec ( i64 -- i64 ) dup 0 > ~[ rec ] ~[ 0 ] if drop 5 ;");
        assert!(!has_self_tail_call(&w, &CombinatorIndex::new()));
        assert!(!tail_position_calls(&w, &CombinatorIndex::new()).contains(&"rec"));
    }
    #[test]
    fn tail_position_clause_body_final_self_call_is_tail() {
        let w = first_word("type: E | A | B ; : w ( E -- E ) | A w | B w ;");
        assert_eq!(
            tail_position_calls(&w, &CombinatorIndex::new()),
            vec!["w", "w"]
        );
        assert!(has_self_tail_call(&w, &CombinatorIndex::new()));
    }

    // -- Slice 10c (R-P1): tail-splice recognition ---------------------------

    /// A hand-written two-way branch over the primitive `if`, whose two
    /// quotation parameters are each `call`ed in tail position: the shape
    /// whose tail-called-parameter set is `{1, 2}`.
    const BOOL_Q: &str = ": Bool? inline ( bool ~[ -- i64 ] ~[ -- i64 ] -- i64 )\n\
         | e | | t | | c | c ~[ t call ] ~[ e call ] if ;\n";
    /// Recon 4's negative: each arm `call`s one parameter and *then* drops the
    /// other, so the tail term is `drop` and neither parameter is tail-called.
    const BOOL_D: &str = ": Bool!? inline ( bool ~[ -- i64 ] ~[ -- i64 ] -- i64 )\n\
         | e | | t | | c | c ~[ t call e drop ] ~[ e call t drop ] if ;\n";

    fn words_of(src: &str) -> Vec<WordDef> {
        let tokens = lex(src).unwrap();
        parse(&tokens).unwrap().words
    }
    fn named<'w>(words: &'w [WordDef], name: &str) -> &'w WordDef {
        words.iter().find(|w| w.name == name).unwrap()
    }

    #[test]
    fn tail_splice_through_a_tail_called_parameter_is_self_tail() {
        // Recon 2: `sum-to`'s recursive call sits inside a quotation literal
        // handed to `Bool?`, which `call`s that parameter in tail position, so
        // the literal inherits `sum-to`'s tail position.
        let words = words_of(&format!(
            "{BOOL_Q}: sum-to ( i64 i64 -- i64 )\n\
             | n | | acc | n 0 = [ acc ] [ acc n + n 1 - sum-to ] Bool? ;\n"
        ));
        let combs = combinator_index(&words);
        assert!(has_self_tail_call(named(&words, "sum-to"), &combs));
        // Without the callee's body there is nothing to prove the splice hands
        // on tail position, so the walk declines rather than guessing.
        assert!(!has_self_tail_call(
            named(&words, "sum-to"),
            &CombinatorIndex::new()
        ));
    }

    #[test]
    fn tail_splice_through_a_discarded_parameter_is_not_self_tail() {
        // R-P1-2: `t call e drop` puts `drop` in tail position, so neither
        // parameter is tail-called and the identical caller stays ordinary
        // recursion.
        let words = words_of(&format!(
            "{BOOL_D}: sum-to ( i64 i64 -- i64 )\n\
             | n | | acc | n 0 = [ acc ] [ acc n + n 1 - sum-to ] Bool!? ;\n"
        ));
        assert!(!has_self_tail_call(
            named(&words, "sum-to"),
            &combinator_index(&words)
        ));
    }

    #[test]
    fn tail_splice_through_a_forwarded_quotation_declines() {
        // R-P1-3: the recursive literal is bound to a local before the call,
        // so the argument slot is not a syntactically visible literal and the
        // walk declines. Conservative by design: it costs the loop transform,
        // never correctness.
        let words = words_of(&format!(
            "{BOOL_Q}: sum-to ( i64 i64 -- i64 )\n\
             | n | | acc | [ acc n + n 1 - sum-to ] | rec | n 0 = [ acc ] rec Bool? ;\n"
        ));
        assert!(!has_self_tail_call(
            named(&words, "sum-to"),
            &combinator_index(&words)
        ));
    }

    #[test]
    fn tail_splice_through_an_ambiguous_combinator_name_declines() {
        // R-P1-4: two always-spliced words share the name, so which body the
        // call reaches cannot be decided syntactically.
        let words = words_of(&format!(
            "{BOOL_Q}: Bool? inline ( str ~[ -- i64 ] ~[ -- i64 ] -- i64 )\n\
             | e | | t | | c | c drop t call e drop ;\n\
             : sum-to ( i64 i64 -- i64 )\n\
             | n | | acc | n 0 = [ acc ] [ acc n + n 1 - sum-to ] Bool? ;\n"
        ));
        let combs = combinator_index(&words);
        assert!(combs["Bool?"].ambiguous);
        assert!(!has_self_tail_call(named(&words, "sum-to"), &combs));
    }

    #[test]
    fn tail_splice_both_predicate_wrappers_agree_for_a_combinator() {
        // E-P1-4, for the one shape its end-to-end twin
        // (`ir::driver::tests::tail_splice_check_and_lowering_agree_on_the_loop`)
        // cannot reach: here the self-tailing word is itself a *combinator*,
        // whose argument-site literal check splices forever (a pre-existing
        // hole, unrelated to this walk), so nothing can be compiled to compare
        // against. The two wrappers the two splice sites call are asked
        // directly instead: `has_self_tail_call` (the checker's `splice_tail`)
        // and `terms_tail_call_self` (the lowering splice gate).
        //
        // Because nothing routes through those two production call sites, nor
        // through `check_combinator_cycles`, all three keep passing the suite
        // with an empty `CombinatorIndex` substituted in. That is expected, not
        // a missing guard: see the witness map under E-P1-4 in
        // `docs/roadmap/P4/slice10c-spec.md` before "fixing" a survivor there.
        let recon2 = words_of(&format!(
            "{BOOL_Q}: walk inline ( i64 ~[ -- i64 ] -- i64 )\n\
             | f | | n | n 0 = [ f call ] [ n 1 - f walk ] Bool? ;\n"
        ));
        let recon4 = words_of(&format!(
            "{BOOL_D}: walk inline ( i64 ~[ -- i64 ] -- i64 )\n\
             | f | | n | n 0 = [ f call ] [ n 1 - f walk ] Bool!? ;\n"
        ));
        for (words, expected) in [(recon2, true), (recon4, false)] {
            let combs = combinator_index(&words);
            let word = named(&words, "walk");
            let WordBody::Terms { terms } = &word.body else {
                unreachable!("`walk` is a terms body")
            };
            let checker = is_combinator(word) && has_self_tail_call(word, &combs);
            let lowering = terms_tail_call_self(terms, &word.name, &combs);
            assert_eq!(checker, expected, "the checker's `splice_tail`");
            assert_eq!(
                checker, lowering,
                "check and lowering must decide a splice-is-a-loop identically"
            );
        }
    }

    #[test]
    fn tail_splice_a_builtin_named_word_is_self_tail_to_neither_pass() {
        // R-P3-1b, asked of the two predicates directly. Nothing routes a
        // builtin-named word through the production splice sites -- which is
        // exactly the problem: the refusal `terms_tail_call_self` carries is
        // the counterpart of `has_self_tail_call`'s, and with no source shape
        // reaching it the suite stays green if it is deleted, so it is pinned
        // here or not at all (the `collect_drop_targets` precedent).
        //
        // Both words end in their own builtin name, so a walk with no refusal
        // reports a self-tail. `<` resolves against the builtin table, and
        // `branch` is the narrowing that made this live: it is the one builtin
        // sanctioned to take quotation operands (R-P3-1a), so it is also the
        // one that can reach the env combinator lookup a builtin name used to
        // be kept away from.
        for src in [
            "type: Vec2 x i64 y i64 ;\n\
             : < ( Vec2 Vec2 -- bool ) | a b | a Vec2>x b Vec2>x < ;\n",
            ": branch inline ( u32 ~[ -- i64 ] ~[ -- i64 ] -- i64 )\n\
             | e | | t | | c | c t e branch ;\n",
        ] {
            let words = words_of(src);
            // The word under test is the source's own, at index 0: `words_of`
            // appends the `lib/core.sth` prelude, which now defines `<` too,
            // so both `last()` and a name lookup can find the wrong one.
            let word = words.first().expect("the builtin-named word");
            let WordBody::Terms { terms } = &word.body else {
                unreachable!("a terms body")
            };
            let combs = combinator_index(&words);
            assert!(
                !has_self_tail_call(word, &combs),
                "`{}`: the checker must not read a builtin name as a self-call",
                word.name
            );
            assert!(
                !terms_tail_call_self(terms, &word.name, &combs),
                "`{}`: lowering must refuse it too, or the two passes disagree \
                 about whether a splice is a loop",
                word.name
            );
        }
    }

    #[test]
    fn tail_splice_forwarding_cycle_declines() {
        // Two combinators each forwarding a tail-called parameter into the
        // other would loop the static closure `C -> D -> C`; the visited set
        // declines instead. (`check_combinator_cycles` rejects this program
        // separately -- the point here is that the walk terminates on it.)
        let words = words_of(
            ": ping inline ( ~[ -- i64 ] -- i64 ) | f | f pong ;\n\
             : pong inline ( ~[ -- i64 ] -- i64 ) | f | f ping ;\n\
             : go ( -- i64 ) [ 7 ] ping ;\n",
        );
        let combs = combinator_index(&words);
        assert!(!has_self_tail_call(named(&words, "ping"), &combs));
        assert!(!has_self_tail_call(named(&words, "go"), &combs));
    }

    #[test]
    fn tail_call_of_a_literal_is_tail() {
        // `[ ... ] call` is the one-step-shorter splice: the literal runs in
        // place of the `call`, so a self-call at its tail is the back-edge.
        // Lowering threads `tail` through the same splice, so the walk must
        // see it too or the two disagree.
        let words = words_of(": rec ( i64 -- i64 ) [ 1 - rec ] call ;\n");
        assert!(has_self_tail_call(
            named(&words, "rec"),
            &CombinatorIndex::new()
        ));
    }

    #[test]
    fn tail_splice_leading_binds_map_to_declared_slots() {
        // The mechanism the whole slice rests on: `| e | | t | | c |` binds the
        // *last* declared input first, so `t`/`e` resolve back to slots 1 and
        // 2 and their tail `call`s land there. Getting the direction wrong
        // silently empties the set (and costs every loop), so it is pinned
        // directly rather than only through a caller.
        //
        // Both spellings are checked. One-name binds cannot tell the two
        // directions apart (each pops the current top either way); only a
        // multi-name `| c t e |`, whose *leftmost* name takes the deepest
        // value, does.
        for (src, name) in [
            (BOOL_Q, "Bool?"),
            (
                ": Pick inline ( bool ~[ -- i64 ] ~[ -- i64 ] -- i64 )\n\
                 | c t e | c ~[ t call ] ~[ e call ] if ;\n",
                "Pick",
            ),
        ] {
            let words = words_of(src);
            let combs = combinator_index(&words);
            let mut walk = TailWalk::new(&combs);
            let (inputs, mut slots) = walk.tail_called_params(name).unwrap();
            slots.sort_unstable();
            assert_eq!((inputs, slots), (3, vec![1, 2]), "{name}");
        }
    }

    #[test]
    fn tail_splice_discarded_parameter_set_is_empty() {
        let words = words_of(BOOL_D);
        let combs = combinator_index(&words);
        let mut walk = TailWalk::new(&combs);
        assert_eq!(walk.tail_called_params("Bool!?"), Some((3, Vec::new())));
    }
    #[test]
    fn check_mutual_tail_recursion_is_error() {
        // X1: A tail-calls B, B tail-calls A -> located error naming the cycle.
        let err = check_src(": a ( i64 -- i64 ) b ; : b ( i64 -- i64 ) a ;").unwrap_err();
        assert!(
            err.contains("mutual tail recursion"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`a`"), "unexpected message: {err}");
        assert!(err.contains("`b`"), "unexpected message: {err}");
    }
    #[test]
    fn check_non_tail_mutual_recursion_is_ok() {
        // Both words call each other only in non-tail position (`x 1 +`), so no
        // tail-call edge exists and X1 must not fire (R4 no-false-positive).
        check_src(
            ": a ( i64 -- i64 ) dup 0 > ~[ b 1 + ] ~[ drop 0 ] if ; \
             : b ( i64 -- i64 ) dup 0 > ~[ a 1 + ] ~[ drop 0 ] if ;",
        )
        .unwrap();
    }
    #[test]
    fn check_self_tail_recursion_is_allowed() {
        // A self-loop (`gcd -> gcd`) is tier-1 and must not be flagged as a
        // mutual cycle.
        check_src(&std::fs::read_to_string("examples/gcd.sth").unwrap()).unwrap();
    }
}
