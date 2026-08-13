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
/// iff it is the final term of a terms body, the final term of a clause body,
/// or the final term of an arm of a *terminal* `if` (an `if` that is itself
/// the final term hands tail position to the last term of both arms,
/// recursively). Any term after a call, arithmetic, a shuffle, a consumer, or
/// another call, breaks tail position, and a call inside a non-terminal `if`
/// is not tail. Output-equality with the declared outputs is a *consequence*
/// of this rule for a well-typed final call, not a second check.
///
/// Shared by the checker (R2 predicate, R3 tail-call graph); the lowerer
/// re-encodes the same syntactic rule via positional `tail` threading in
/// `lower_terms` (src/ir.rs), which a name list can't express. The two must
/// stay in lockstep if the tail rule changes.
pub(super) fn tail_position_calls(body: &WordBody) -> Vec<&str> {
    let mut out = Vec::new();
    match body {
        WordBody::Terms { terms, .. } => collect_tail_calls(terms, &mut out),
        WordBody::Clauses(clauses) => {
            for clause in clauses {
                collect_tail_calls(&clause.body, &mut out);
            }
        }
    }
    out
}

fn collect_tail_calls<'a>(terms: &'a [Term], out: &mut Vec<&'a str>) {
    let Some(last) = terms.last() else {
        return;
    };
    match &last.kind {
        TermKind::Call(name) => out.push(name.as_str()),
        TermKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            collect_tail_calls(then_branch, out);
            collect_tail_calls(else_branch, out);
        }
        _ => {}
    }
}

/// R2 (M1): whether a word contains at least one tail-position call to itself.
/// The lowerer uses this to decide whether to build the loop shape at all.
///
/// A word whose own name is a builtin's never self-tail-calls on a bare name
/// match. A builtin name in tail position resolves against the builtin table
/// first, so it need not mean the enclosing word: `: drop ( T -- )`'s trailing
/// `drop` disposes whatever is on top (the dogfood's own
/// `| f | f File>fd close drop ;` closes the fd rather than looping), and since
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
pub(crate) fn has_self_tail_call(word: &WordDef) -> bool {
    !is_builtin_word_name(&word.name)
        && tail_position_calls(&word.body)
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
        for callee in tail_position_calls(&word.body) {
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
            TermKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                collect_all_calls(then_branch, out);
                collect_all_calls(else_branch, out);
            }
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
    const SPY_DEF: &str =
        "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy>tag . ;\n";
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
                   : drop ( File -- ) | f | true if f shut else f shut end 1 . ; \
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
                   : drop ( File -- ) | f | f File>fd drop ; \
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
                   : drop ( File -- ) | f | f File>fd show ; \
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
        assert_eq!(tail_position_calls(&w.body), vec!["rec"]);
        assert!(has_self_tail_call(&w));
    }
    #[test]
    fn tail_position_trailing_arithmetic_is_not_tail() {
        // `rec *`: the final term is `*`, so the self-call is not in tail
        // position (classic non-tail recursion).
        let w = first_word(": rec ( i64 -- i64 ) rec * ;");
        assert_eq!(tail_position_calls(&w.body), vec!["*"]);
        assert!(!has_self_tail_call(&w));
    }
    #[test]
    fn tail_position_trailing_swap_is_not_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec swap ;");
        assert_eq!(tail_position_calls(&w.body), vec!["swap"]);
        assert!(!has_self_tail_call(&w));
    }
    #[test]
    fn tail_position_trailing_drop_is_not_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec drop ;");
        assert_eq!(tail_position_calls(&w.body), vec!["drop"]);
        assert!(!has_self_tail_call(&w));
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
        assert_eq!(tail_position_calls(&w.body), vec!["<"]);
        assert!(!has_self_tail_call(&w));
    }
    #[test]
    fn tail_position_both_terminal_if_arms_are_tail() {
        // A terminal `if` hands tail position to the last term of both arms.
        let w = first_word(": rec ( i64 -- i64 ) dup 0 > if rec else rec end ;");
        assert_eq!(tail_position_calls(&w.body), vec!["rec", "rec"]);
        assert!(has_self_tail_call(&w));
    }
    #[test]
    fn tail_position_non_terminal_if_self_call_is_not_tail() {
        // The `if` is followed by more terms, so it is non-terminal and its
        // arms are not in tail position.
        let w = first_word(": rec ( i64 -- i64 ) dup 0 > if rec else 0 end drop 5 ;");
        assert!(!has_self_tail_call(&w));
        assert!(!tail_position_calls(&w.body).contains(&"rec"));
    }
    #[test]
    fn tail_position_clause_body_final_self_call_is_tail() {
        let w = first_word("type: E | A | B ; : w ( E -- E ) | A w | B w ;");
        assert_eq!(tail_position_calls(&w.body), vec!["w", "w"]);
        assert!(has_self_tail_call(&w));
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
            ": a ( i64 -- i64 ) dup 0 > if b 1 + else drop 0 end ; \
             : b ( i64 -- i64 ) dup 0 > if a 1 + else drop 0 end ;",
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
