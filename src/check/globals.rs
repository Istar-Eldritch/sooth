//! Phase 7 slice 2 (R4/R5/R6): the per-word **global set** — which module
//! statics a word touches, and in what mode — inferred over the intra-module
//! call graph and checked against the `global:` clause an exported word must
//! declare.
//!
//! Runs on the raw, **pre-mangle** module the driver assembles
//! (`driver::assemble_module`), beside `check_exported_signatures`: a word's
//! name, a static's name and a module's `export:` list are all still their
//! source spellings there, which `resolve::resolve_modules` would otherwise
//! rewrite apart.
//!
//! Soundness scope (R4/R5): the direct-set walk approximates combinator
//! inlining by traversing quotation *literals* textually, so it is sound over
//! the non-escaping subset DESIGN.md already restricts closures to, and it
//! attributes a literal's accesses to the word that textually contains it.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::*;
use crate::ast::{GlobalEntry, GlobalMode};

/// A word's global set: static name -> access mode, joined under `r ⊔ w = w`.
/// Ordered so a diagnostic listing a whole set reads the same on every run.
type GlobalSet = BTreeMap<String, GlobalMode>;

fn mode_str(mode: GlobalMode) -> &'static str {
    match mode {
        GlobalMode::R => "r",
        GlobalMode::W => "w",
    }
}

/// Join one access into a set, reporting whether the set grew — the monotone
/// step the R5 fixpoint iterates on. `w` absorbs `r`; `r` never demotes a `w`.
fn join_into(set: &mut GlobalSet, name: &str, mode: GlobalMode) -> bool {
    match set.get_mut(name) {
        Some(existing) => {
            if *existing == GlobalMode::R && mode == GlobalMode::W {
                *existing = GlobalMode::W;
                return true;
            }
            false
        }
        None => {
            set.insert(name.to_string(), mode);
            true
        }
    }
}

/// R4: the static a `&NAME`/`&!NAME` term names, and the mode it implies, or
/// `None` for every term that names no static of this module.
///
/// The sigil strip plus the exact-name table lookup is the whole filter: a
/// qualified or accessor-suffixed name never equals a bare static name, so
/// `statics.contains(rest)` already rejects it with no separate shape check
/// needed.
fn static_access<'a>(
    name: &'a str,
    shadowed: &HashSet<String>,
    statics: &HashSet<&str>,
) -> Option<(&'a str, GlobalMode)> {
    let (mode, rest) = if let Some(rest) = name.strip_prefix("&!") {
        (GlobalMode::W, rest)
    } else if let Some(rest) = name.strip_prefix('&') {
        (GlobalMode::R, rest)
    } else {
        return None;
    };
    if shadowed.contains(rest) || !statics.contains(rest) {
        return None;
    }
    Some((rest, mode))
}

/// One word's own accesses: the statics it names directly (R4) and the bare
/// names it calls, the latter resolved to intra-module callees by `infer_sets`.
#[derive(Default)]
struct Direct {
    set: GlobalSet,
    calls: HashSet<String>,
}

fn direct_of(word: &WordDef, statics: &HashSet<&str>) -> Direct {
    let mut out = Direct::default();
    let terms = &word.body;
    walk(terms, statics, &mut HashSet::new(), &mut out);
    out
}

/// The direct-set traversal: every term at any depth, recursing into
/// quotation literals (which is also how an `if` arm is reached, `if` being a
/// library word over two quotations). `shadowed` follows the language's own
/// scoping — a bind's extent is the rest of its block, and a nested quotation
/// inherits the outer binds by value — so a local shadowing a static keeps the
/// static out of the set, and a call to a local is not read as a word call.
fn walk(terms: &[Term], statics: &HashSet<&str>, shadowed: &mut HashSet<String>, out: &mut Direct) {
    for term in terms {
        match &term.kind {
            TermKind::Bind(names) => shadowed.extend(names.iter().cloned()),
            TermKind::Call(name, _) => {
                if let Some((static_name, mode)) = static_access(name, shadowed, statics) {
                    join_into(&mut out.set, static_name, mode);
                } else if !shadowed.contains(name.as_str()) {
                    out.calls.insert(name.clone());
                }
            }
            TermKind::Quotation(inner, _, _) => walk(inner, statics, &mut shadowed.clone(), out),
            _ => {}
        }
    }
}

/// R5: every word's inferred set — its direct set unioned with every
/// intra-module callee's inferred set, mode-joined — plus the number of
/// relaxation passes it took to converge.
///
/// Worklist form rather than recursion: the lattice (subsets of the module's
/// statics × `{r, w}`) is finite and the update monotone, so relaxing every
/// word until a full pass changes nothing converges with no cycle-breaking
/// special case, and mutual recursion needs no visited-set guard. The pass
/// count is returned so a test can pin convergence to a small bound; the
/// assertion below turns a regression that stopped converging into a failure
/// rather than a hang.
fn infer_sets(words: &[WordDef], statics: &[StaticDecl]) -> (Vec<GlobalSet>, usize) {
    let mut by_module: HashMap<u32, HashSet<&str>> = HashMap::new();
    for decl in statics {
        by_module
            .entry(decl.module)
            .or_default()
            .insert(decl.name.as_str());
    }
    let empty: HashSet<&str> = HashSet::new();

    // A name can carry several same-module words (an overload set), so an edge
    // goes to every candidate: which one a call site picks is a type-directed
    // question this pass has no answer for, and the union is the safe answer.
    let mut by_name: HashMap<(u32, &str), Vec<usize>> = HashMap::new();
    for (i, word) in words.iter().enumerate() {
        by_name
            .entry((word.module, word.name.as_str()))
            .or_default()
            .push(i);
    }

    let direct: Vec<Direct> = words
        .iter()
        .map(|w| direct_of(w, by_module.get(&w.module).unwrap_or(&empty)))
        .collect();
    let callees: Vec<Vec<usize>> = words
        .iter()
        .zip(&direct)
        .map(|(word, d)| {
            let mut out: Vec<usize> = d
                .calls
                .iter()
                .filter_map(|name| by_name.get(&(word.module, name.as_str())))
                .flatten()
                .copied()
                .collect();
            out.sort_unstable();
            out.dedup();
            out
        })
        .collect();

    let mut sets: Vec<GlobalSet> = direct.into_iter().map(|d| d.set).collect();
    // A monotone relaxation propagates at least one call-graph level per pass,
    // so a converging run needs at most one pass per word plus the final
    // no-change one.
    let bound = words.len() + 2;
    let mut passes = 0;
    loop {
        passes += 1;
        let mut changed = false;
        for i in 0..sets.len() {
            for &j in &callees[i] {
                if j == i {
                    continue;
                }
                let callee: Vec<(String, GlobalMode)> =
                    sets[j].iter().map(|(n, m)| (n.clone(), *m)).collect();
                for (name, mode) in callee {
                    changed |= join_into(&mut sets[i], &name, mode);
                }
            }
        }
        if !changed {
            break;
        }
        assert!(
            passes < bound,
            "global-set inference did not converge in {bound} passes"
        );
    }
    (sets, passes)
}

/// R6: every word's `global:` clause against its inferred set — mandatory on
/// an exported word that touches anything, optional on a private one, and
/// exact (not superset) wherever it is written.
pub(crate) fn check_globals(module: &Module) -> Result<(), String> {
    let (sets, _) = infer_sets(&module.words, &module.statics);
    let mut by_module: HashMap<u32, HashSet<&str>> = HashMap::new();
    for decl in &module.statics {
        by_module
            .entry(decl.module)
            .or_default()
            .insert(decl.name.as_str());
    }
    let empty: HashSet<&str> = HashSet::new();
    for (word, inferred) in module.words.iter().zip(&sets) {
        let info = module.modules.get(word.module as usize).expect(
            "assemble_module pushes one ModuleInfo per closure node, indexed by word.module",
        );
        let exported = info.exports.iter().any(|(n, _)| n == &word.name);
        let statics = by_module.get(&word.module).unwrap_or(&empty);
        check_clause(word, inferred, exported, statics)?;
    }
    Ok(())
}

fn check_clause(
    word: &WordDef,
    inferred: &GlobalSet,
    exported: bool,
    statics: &HashSet<&str>,
) -> Result<(), String> {
    let Some(entries) = &word.declared_globals else {
        // An exported word touching nothing needs no clause: `Some(vec![])` is
        // not even representable (D4), so the empty set has no spelling to
        // demand.
        if exported && !inferred.is_empty() {
            return Err(missing_global_clause_error(word, inferred));
        }
        return Ok(());
    };
    for entry in entries {
        if !statics.contains(entry.name.as_str()) {
            return Err(no_such_static_error(word, entry));
        }
        match inferred.get(&entry.name) {
            None => return Err(extra_entry_error(word, entry)),
            Some(&mode) if mode != entry.mode => return Err(wrong_mode_error(word, entry, mode)),
            Some(_) => {}
        }
    }
    for (name, mode) in inferred {
        if !entries.iter().any(|e| &e.name == name) {
            return Err(missing_entry_error(word, name, *mode));
        }
    }
    Ok(())
}

/// The inferred set as it would be written as a clause body, e.g. `COUNT w,
/// LIMIT r`, so the "must declare" error can hand back the exact text to paste.
fn clause_text(set: &GlobalSet) -> String {
    set.iter()
        .map(|(name, mode)| format!("{name} {}", mode_str(*mode)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn touch_list(set: &GlobalSet) -> String {
    set.iter()
        .map(|(name, mode)| format!("`{name}` ({})", mode_str(*mode)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn missing_global_clause_error(word: &WordDef, inferred: &GlobalSet) -> String {
    let span = word_span(word);
    format!(
        "error: exported word `{}` (line {}, col {}) must declare its global set: it touches {}\n  write `global: {}` after the effect",
        word.name,
        span.line,
        span.col,
        touch_list(inferred),
        clause_text(inferred)
    )
}

fn missing_entry_error(word: &WordDef, name: &str, mode: GlobalMode) -> String {
    let span = word_span(word);
    format!(
        "error: word `{}` (line {}, col {}) touches static `{}` ({}), which its `global:` clause does not declare\n  the declared set must match the inferred one exactly",
        word.name,
        span.line,
        span.col,
        name,
        mode_str(mode)
    )
}

fn wrong_mode_error(word: &WordDef, entry: &GlobalEntry, inferred: GlobalMode) -> String {
    format!(
        "error: `global:` entry `{}` of word `{}` (line {}, col {}) declares mode `{}`, but the body infers `{}`\n  a mode is derived from the body, never authored: fix the entry or the body",
        entry.name,
        word.name,
        entry.span.line,
        entry.span.col,
        mode_str(entry.mode),
        mode_str(inferred)
    )
}

fn extra_entry_error(word: &WordDef, entry: &GlobalEntry) -> String {
    format!(
        "error: `global:` entry `{}` of word `{}` (line {}, col {}) declares a static the word never touches\n  the match is exact, not a superset: drop the entry",
        entry.name, word.name, entry.span.line, entry.span.col
    )
}

fn no_such_static_error(word: &WordDef, entry: &GlobalEntry) -> String {
    format!(
        "error: `global:` entry `{}` of word `{}` (line {}, col {}) names no static declared in this module\n  a static is module-private: it must be declared by the module that names it",
        entry.name, word.name, entry.span.line, entry.span.col
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::StaticInit;

    fn span(line: u32) -> Span {
        Span {
            line,
            col: 1,
            module: 0,
        }
    }

    fn call(name: &str) -> Term {
        Term {
            kind: TermKind::Call(name.to_string(), Vec::new()),
            span: span(1),
        }
    }

    fn bind(names: &[&str]) -> Term {
        Term {
            kind: TermKind::Bind(names.iter().map(|n| n.to_string()).collect()),
            span: span(1),
        }
    }

    fn quot(terms: Vec<Term>) -> Term {
        Term {
            kind: TermKind::Quotation(terms, false, None),
            span: span(1),
        }
    }

    fn word_in(name: &str, module: u32, terms: Vec<Term>) -> WordDef {
        WordDef {
            name: name.to_string(),
            effect: StackEffect::default(),
            body: terms,
            poly: None,
            declares_inline: false,
            module,
            span: span(1),
            declared_globals: None,
        }
    }

    fn word(name: &str, terms: Vec<Term>) -> WordDef {
        word_in(name, 0, terms)
    }

    fn static_in(name: &str, module: u32) -> StaticDecl {
        StaticDecl {
            name: name.to_string(),
            ty: Type::I64,
            init: StaticInit::Zero,
            module,
            span: span(1),
        }
    }

    fn statik(name: &str) -> StaticDecl {
        static_in(name, 0)
    }

    fn entry(name: &str, mode: GlobalMode) -> GlobalEntry {
        GlobalEntry {
            name: name.to_string(),
            mode,
            span: Span {
                line: 1,
                col: 20,
                module: 0,
            },
        }
    }

    fn set_of(pairs: &[(&str, GlobalMode)]) -> GlobalSet {
        pairs
            .iter()
            .map(|(n, m)| (n.to_string(), *m))
            .collect::<GlobalSet>()
    }

    /// A module of one file, its `export:` list the given names.
    fn module_of(statics: Vec<StaticDecl>, words: Vec<WordDef>, exports: &[&str]) -> Module {
        Module {
            words,
            statics,
            modules: vec![ModuleInfo {
                exports: exports.iter().map(|n| (n.to_string(), span(1))).collect(),
                ..ModuleInfo::default()
            }],
            ..Module::default()
        }
    }

    #[test]
    fn direct_set_counts_named_static_not_ref_parameter() {
        // Decision 6: only a term *naming* a static accrues. A word handed a
        // `&!` parameter and never naming a static accrues nothing, even
        // though it writes through the reference it was given.
        let statics = vec![statik("COUNT")];
        let words = vec![
            word("writer", vec![call("&!COUNT"), call("1"), call("+!")]),
            word(
                "via-param",
                vec![bind(&["c"]), call("c"), call("1"), call("+!")],
            ),
        ];
        let (sets, _) = infer_sets(&words, &statics);
        assert_eq!(sets[0], set_of(&[("COUNT", GlobalMode::W)]));
        assert_eq!(sets[1], GlobalSet::new());
    }

    #[test]
    fn mode_is_write_if_any_mutable_borrow() {
        // Both orders: `w` absorbs an `r` that came before it, and a later `r`
        // never demotes a `w`.
        let statics = vec![statik("COUNT")];
        let words = vec![
            word(
                "read-then-write",
                vec![
                    call("&COUNT"),
                    call("@"),
                    call("&!COUNT"),
                    call("1"),
                    call("+!"),
                ],
            ),
            word(
                "write-then-read",
                vec![
                    call("&!COUNT"),
                    call("1"),
                    call("+!"),
                    call("&COUNT"),
                    call("@"),
                ],
            ),
        ];
        let (sets, _) = infer_sets(&words, &statics);
        assert_eq!(sets[0], set_of(&[("COUNT", GlobalMode::W)]));
        assert_eq!(sets[1], set_of(&[("COUNT", GlobalMode::W)]));
    }

    #[test]
    fn static_lookup_is_scoped_to_the_accessing_module() {
        // A static is module-private (R2), so a same-named static of another
        // module is not this word's to accrue -- the borrow-typing lookup is
        // module-keyed and so is this one.
        let statics = vec![static_in("COUNT", 1)];
        let words = vec![word_in(
            "a",
            0,
            vec![call("&!COUNT"), call("1"), call("+!")],
        )];
        let (sets, _) = infer_sets(&words, &statics);
        assert_eq!(sets[0], GlobalSet::new());
    }

    #[test]
    fn local_shadowing_a_word_name_is_not_a_call_edge() {
        // `b` here is a bound local being pushed, not a call to the word `b`,
        // so `a` inherits nothing from that word's set.
        let statics = vec![statik("COUNT")];
        let words = vec![
            word("a", vec![bind(&["b"]), call("b")]),
            word("b", vec![call("&!COUNT"), call("1"), call("+!")]),
        ];
        let (sets, _) = infer_sets(&words, &statics);
        assert_eq!(sets[0], GlobalSet::new());
        assert_eq!(sets[1], set_of(&[("COUNT", GlobalMode::W)]));
    }

    #[test]
    fn direct_set_reaches_into_a_quotation_literal() {
        // R4: an `if` arm is a quotation literal, so the walk must see through
        // one or every conditional access would go uncounted.
        let statics = vec![statik("COUNT")];
        let words = vec![word(
            "cond",
            vec![
                quot(vec![call("&!COUNT"), call("1"), call("+!")]),
                call("if"),
            ],
        )];
        let (sets, _) = infer_sets(&words, &statics);
        assert_eq!(sets[0], set_of(&[("COUNT", GlobalMode::W)]));
    }

    #[test]
    fn local_shadowing_a_static_contributes_nothing() {
        // R1's resolution order at the borrow site is local-then-static, so a
        // `&COUNT` under a local named `COUNT` borrows the local and accrues
        // no global access.
        let statics = vec![statik("COUNT")];
        let words = vec![word("shadow", vec![bind(&["COUNT"]), call("&COUNT")])];
        let (sets, _) = infer_sets(&words, &statics);
        assert_eq!(sets[0], GlobalSet::new());
    }

    #[test]
    fn fixpoint_unions_callee_sets() {
        let statics = vec![statik("COUNT")];
        let words = vec![
            word("a", vec![call("b")]),
            word("b", vec![call("&!COUNT"), call("1"), call("+!")]),
        ];
        let (sets, _) = infer_sets(&words, &statics);
        assert_eq!(sets[0], set_of(&[("COUNT", GlobalMode::W)]));
    }

    #[test]
    fn direct_set_ignores_imported_callee() {
        // R4 is intra-module: module 1's `tick` touches module 1's `COUNT`,
        // and module 0's `a` calling a same-named word gains nothing from it.
        let statics = vec![static_in("COUNT", 1), static_in("LIMIT", 0)];
        let words = vec![
            word_in("a", 0, vec![call("&LIMIT"), call("@"), call("tick")]),
            word_in("tick", 1, vec![call("&!COUNT"), call("1"), call("+!")]),
        ];
        let (sets, _) = infer_sets(&words, &statics);
        assert_eq!(sets[0], set_of(&[("LIMIT", GlobalMode::R)]));
        assert_eq!(sets[1], set_of(&[("COUNT", GlobalMode::W)]));
    }

    #[test]
    fn fixpoint_converges_on_mutual_recursion() {
        // OQ3's witness. The bound is what makes it fail red: an unguarded
        // recursive formulation would hang here instead of over-counting
        // passes, so the pass counter is asserted, not just the sets.
        let statics = vec![statik("COUNT"), statik("LIMIT")];
        let words = vec![
            word("a", vec![call("&!COUNT"), call("b")]),
            word("b", vec![call("&LIMIT"), call("a")]),
        ];
        let (sets, passes) = infer_sets(&words, &statics);
        let both = set_of(&[("COUNT", GlobalMode::W), ("LIMIT", GlobalMode::R)]);
        assert_eq!(sets[0], both);
        assert_eq!(sets[1], both);
        assert!(passes <= 3, "a two-word cycle converged in {passes} passes");
    }

    #[test]
    fn exact_match_missing_entry_is_error() {
        let mut tick = word(
            "tick",
            vec![
                call("&!COUNT"),
                call("1"),
                call("+!"),
                call("&LIMIT"),
                call("@"),
            ],
        );
        tick.declared_globals = Some(vec![entry("LIMIT", GlobalMode::R)]);
        let module = module_of(
            vec![statik("COUNT"), statik("LIMIT")],
            vec![tick],
            &["tick"],
        );
        let err = check_globals(&module).unwrap_err();
        assert!(
            err.contains("word `tick` (line 1, col 1) touches static `COUNT` (w), which its `global:` clause does not declare"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn exact_match_wrong_mode_is_error() {
        let mut tick = word("tick", vec![call("&!COUNT"), call("1"), call("+!")]);
        tick.declared_globals = Some(vec![entry("COUNT", GlobalMode::R)]);
        let module = module_of(vec![statik("COUNT")], vec![tick], &["tick"]);
        let err = check_globals(&module).unwrap_err();
        assert!(
            err.contains(
                "`global:` entry `COUNT` of word `tick` (line 1, col 20) declares mode `r`, but the body infers `w`"
            ),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn exact_match_extra_entry_is_error() {
        // `COUNT` is a real static of the module; the word simply never
        // touches it. Exact, not superset.
        let mut tick = word("tick", vec![call("1")]);
        tick.declared_globals = Some(vec![entry("COUNT", GlobalMode::W)]);
        let module = module_of(vec![statik("COUNT")], vec![tick], &["tick"]);
        let err = check_globals(&module).unwrap_err();
        assert!(
            err.contains(
                "`global:` entry `COUNT` of word `tick` (line 1, col 20) declares a static the word never touches"
            ),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn no_such_static_entry_is_distinct_error() {
        // A name that resolves to nothing is not an over-declaration: it gets
        // its own unresolved-name message, never the extra-entry one.
        let mut tick = word("tick", vec![call("1")]);
        tick.declared_globals = Some(vec![entry("NOPE", GlobalMode::W)]);
        let module = module_of(vec![statik("COUNT")], vec![tick], &["tick"]);
        let err = check_globals(&module).unwrap_err();
        assert!(
            err.contains(
                "`global:` entry `NOPE` of word `tick` (line 1, col 20) names no static declared in this module"
            ),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains("never touches"),
            "a dangling name must not be reported as an over-declaration: {err}"
        );
    }

    #[test]
    fn exported_word_touching_a_static_needs_a_clause() {
        let module = module_of(
            vec![statik("COUNT")],
            vec![word("tick", vec![call("&!COUNT"), call("1"), call("+!")])],
            &["tick"],
        );
        let err = check_globals(&module).unwrap_err();
        assert!(
            err.contains("exported word `tick` (line 1, col 1) must declare its global set: it touches `COUNT` (w)")
                && err.contains("write `global: COUNT w` after the effect"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn exported_word_touching_nothing_needs_no_clause() {
        // The empty set has no spelling (`Some(vec![])` is unrepresentable),
        // so the mandatory rule fires only on a non-empty inferred set.
        let module = module_of(
            vec![statik("COUNT")],
            vec![word("pure", vec![call("1")])],
            &["pure"],
        );
        assert!(check_globals(&module).is_ok());
    }

    #[test]
    fn private_word_clause_optional_absent_ok() {
        let module = module_of(
            vec![statik("COUNT")],
            vec![word("tick", vec![call("&!COUNT"), call("1"), call("+!")])],
            &[],
        );
        assert!(check_globals(&module).is_ok());
    }

    #[test]
    fn private_word_clause_checked_when_present() {
        // Decision 7: written is checked, whether or not the word is exported.
        let mut tick = word("tick", vec![call("&!COUNT"), call("1"), call("+!")]);
        tick.declared_globals = Some(vec![entry("COUNT", GlobalMode::R)]);
        let module = module_of(vec![statik("COUNT")], vec![tick], &[]);
        let err = check_globals(&module).unwrap_err();
        assert!(err.contains("declares mode `r`"), "unexpected: {err}");
    }

    #[test]
    fn exported_word_with_the_right_clause_checks() {
        let mut tick = word(
            "tick",
            vec![
                call("&!COUNT"),
                call("1"),
                call("+!"),
                call("&LIMIT"),
                call("@"),
            ],
        );
        tick.declared_globals = Some(vec![
            entry("COUNT", GlobalMode::W),
            entry("LIMIT", GlobalMode::R),
        ]);
        let module = module_of(
            vec![statik("COUNT"), statik("LIMIT")],
            vec![tick],
            &["tick"],
        );
        assert!(check_globals(&module).is_ok());
    }
}
