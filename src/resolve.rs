//! Phase 4 slice 5a (R10/R11/R22): the module-resolution pass that runs between
//! parse and check on a multi-file import closure assembled into one `Module`.
//!
//! The merged registry holds every file's decls in one set, each tagged with an
//! owning module id. Two files may each declare a `Point` or a `push`; left
//! alone they would collide in the checker's word environment and in the
//! emitted symbols. This pass mangles every decl name to a module-unique form
//! (`push__m1`) and rewrites every body reference to match, so the existing
//! single-module checker and backend need no notion of modules: a qualified
//! `q::word` is resolved to a concrete decl here (D8: no `::` reaches the
//! symbol minter) and a same-named word in another module already carries a
//! distinct name by the time a symbol is spelled (R22).
//!
//! A single-module closure (every single-file program, every REPL session) is
//! left byte-for-byte untouched (R22): the pass is a no-op below two modules,
//! so today's symbols and output are unchanged.

use crate::ast::{Clause, Module, Span, Term, TermKind, WordBody};
use std::collections::HashSet;

/// The surface `main` is never mangled: it must stay the symbol the C shim
/// links against (`sooth_main`, via the backend's `qbe_name`). A `drop`
/// overload is never mangled either (R19): `find_drop_overloads`
/// (`check.rs`) dispatches it by the literal name `drop` plus the struct id
/// its one input names, never through the ordinary word environment a
/// mangled name would otherwise need to be looked up in, so mangling it would
/// only break that lookup for no benefit -- two modules' `drop` overloads for
/// two different structs never collide on the name, since neither is ever
/// registered under it. Every other name gains a `__m{module}` component,
/// minted so no punctuation reaches a symbol sanitizer (D8).
pub(crate) fn mangle(name: &str, module: u32) -> String {
    if name == "main" || name == "drop" || is_prelude_word_name(name) {
        return name.to_string();
    }
    format!("{name}__m{module}")
}

/// R2: a static's own mangle, with no exemptions. Every exemption `mangle`
/// makes exists so a *word* stays reachable by bare name from a module that
/// did not declare it (`main` from the C shim, a `drop` overload from
/// `find_drop_overloads`, `if` from every module at once). A static is
/// reachable no such way: only `&NAME` inside its declaring module names one,
/// and that site mangles identically. Routing statics through `mangle` instead
/// gave `static: drop` and `static: main` a raw data symbol, so two modules
/// declaring one leaked a bare "symbol already defined" out of the assembler,
/// and `Ctx::static_type`, which matches on the mangled name alone, silently
/// borrowed whichever of the two it found first.
pub(crate) fn mangle_static(name: &str, module: u32) -> String {
    format!("{name}__m{module}")
}

/// Slice 10c (R-P3-4): the `lib/core.sth` words injected into every closure.
/// They are declared once but reached by bare name from every module, so
/// mangling them per module would bind each module's `if` to a spelling only
/// the injected copy's own module could resolve, leaving every other module's
/// bare `if` unresolvable. Same reasoning as `main` and `drop`.
///
/// Read off `lib/core.sth` rather than listed here: the file is meant to grow,
/// and a hand-mirrored list would leave each word added to it unresolvable
/// from every module but the entry one.
pub(crate) fn is_prelude_word_name(name: &str) -> bool {
    static NAMES: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
    NAMES
        .get_or_init(|| {
            crate::parser::prelude_words()
                .into_iter()
                .map(|w| w.name)
                .collect()
        })
        .contains(name)
}

/// `check::builtin_table`'s keys, mirrored by hand (that table's own
/// `check_operator::is_operator` list carries the same warning): the
/// arithmetic/comparison/`max`/`max-total`/`.` names a bare call must reach
/// `check_operator`'s operand-type dispatch for, never a static rewrite. Keep
/// in sync when a table operator is added.
///
/// Slice 10c (R-P3-3): the six comparison *surface* names stay listed even
/// though they are no longer table rows. They are `lib/` words now, reached by
/// bare name from every module, so mangling one per module would bind every
/// use inside a module to that module's own spelling and leave every other
/// module's bare `=` unresolvable. The `u`-prefixed primitives that took over
/// their rows are listed beside them.
fn is_operator_dispatch_name(name: &str) -> bool {
    matches!(
        name,
        "+" | "-"
            | "*"
            | "/"
            | "mod"
            | "and"
            | "or"
            | "xor"
            | "not"
            | "shl"
            | "shr"
            | "="
            | "<"
            | ">"
            | "<="
            | ">="
            | "<>"
            | "u="
            | "u<"
            | "u>"
            | "u<="
            | "u>="
            | "u<>"
            | "max"
            | "max-total"
            | "."
    )
}

/// Recover a word's source spelling for a *diagnostic*: strip the single
/// trailing `__m{digits}` group `mangle` appended (`w__m0` -> `w`). A user
/// diagnostic must never show the compiler-internal mangled spelling; it shows
/// what the author wrote. Lookups keep using the mangled name, only the
/// rendered string is stripped. `main`/`drop` are never mangled and pass
/// through unchanged, as does any name `mangle` never touched. Kept beside
/// `mangle` so the two stay in step.
pub(crate) fn demangle_word(name: &str) -> &str {
    let Some(idx) = name.rfind("__m") else {
        return name;
    };
    let digits = &name[idx + "__m".len()..];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return name;
    }
    &name[..idx]
}

/// `demangle_word` for a *call* name, which may carry one remaining
/// generated-word suffix: a destructure mangles as `P__m0>` and an eliminator
/// as `Shape__m0?` (Phase 6 slice 3), so in either case the `__m0` sits
/// mid-string and the trailing-suffix strip above cannot see it; the type
/// prefix has to be demangled and the trailing sigil put back. A constructor
/// call (`P__m0`) has no suffix and falls straight through to `demangle_word`.
pub(crate) fn demangle_call(name: &str) -> std::borrow::Cow<'_, str> {
    let Some(head) = name.strip_suffix('>').or_else(|| name.strip_suffix('?')) else {
        return std::borrow::Cow::Borrowed(demangle_word(name));
    };
    let suffix = &name[head.len()..];
    let demangled = demangle_word(head);
    if demangled.len() == head.len() {
        return std::borrow::Cow::Borrowed(name);
    }
    std::borrow::Cow::Owned(format!("{demangled}{suffix}"))
}

/// Split a leading reference sigil (`&!`/`&`) off a call name, so `rewrite`
/// resolves the same core spelling a same-module word or accessor does
/// (`&!Acc>arr` and `Acc>arr` name the same type). The sigil is reattached
/// verbatim to whatever `rewrite` returns for the core name; a name with no
/// such prefix is untouched.
fn strip_ref_sigil(name: &str) -> (&str, &str) {
    if let Some(rest) = name.strip_prefix("&!") {
        ("&!", rest)
    } else if let Some(rest) = name.strip_prefix('&') {
        ("&", rest)
    } else {
        ("", name)
    }
}

/// Split a call name into its leading type identifier and the one remaining
/// generated-word suffix: `Point>` -> (`Point`, `>`), `Point` -> (`Point`,
/// ``). Phase 6 slice 3: an eliminator call (`Shape?`) is a second such
/// suffix, `Shape?` -> (`Shape`, `?`). A type name never contains `>` or `?`,
/// so a trailing one is unambiguous *as a split*; whether the head plus that
/// suffix is a generated name at all is `NameTables::names_a_type`'s call, and
/// depends on which kind of type the head names.
pub(crate) fn split_destructure_suffix(name: &str) -> (&str, &str) {
    if let Some(head) = name.strip_suffix('>') {
        return (head, ">");
    }
    if let Some(head) = name.strip_suffix('?') {
        return (head, "?");
    }
    (name, "")
}

/// The per-module name tables the rewrite consults: which bare names are this
/// module's struct and enum types (so a constructor/accessor call rewrites),
/// which are its words/externs (so a plain call rewrites), and (R2) which are
/// its `static:` declarations (so a `&NAME`/`&!NAME` borrow of one rewrites).
/// Builtins and genuinely unknown names appear in none of them and are left
/// raw for the checker to resolve or reject. Structs and enums are kept apart
/// because a generated suffix belongs to one kind or the other, never both
/// (`names_a_type`).
struct NameTables {
    structs: Vec<HashSet<String>>,
    enums: Vec<HashSet<String>>,
    words: Vec<HashSet<String>>,
    statics: Vec<HashSet<String>>,
}

impl NameTables {
    fn build(module: &Module) -> NameTables {
        let n = module.modules.len();
        let mut structs = vec![HashSet::new(); n];
        let mut enums = vec![HashSet::new(); n];
        let mut words = vec![HashSet::new(); n];
        let mut statics = vec![HashSet::new(); n];
        for s in &module.statics {
            statics[s.module as usize].insert(s.name.clone());
        }
        for s in &module.structs {
            structs[s.module as usize].insert(s.name.clone());
        }
        for e in &module.enums {
            enums[e.module as usize].insert(e.name.clone());
        }
        for w in &module.words {
            words[w.module as usize].insert(w.name.clone());
        }
        for x in &module.externs {
            words[x.module as usize].insert(x.name.clone());
        }
        NameTables {
            structs,
            enums,
            words,
            statics,
        }
    }

    /// Whether `head` + `suffix` (as split by `split_destructure_suffix`) is a
    /// name `module` generates from a type declaration. Each generated suffix
    /// comes from exactly one kind of type -- `>` is the struct destructure,
    /// `?` (slice 3) the enum eliminator -- so eligibility is per suffix, not
    /// per "names a type at all": a plain word called `P?` where `P` is a
    /// *struct* (or `E>` where `E` is an *enum*) is a word, and must fall
    /// through to the word branches rather than be mangled into a generated
    /// name nothing ever generated.
    fn names_a_type(&self, module: u32, head: &str, suffix: &str) -> bool {
        let m = module as usize;
        match suffix {
            ">" => self.structs[m].contains(head),
            "?" => self.enums[m].contains(head),
            _ => self.structs[m].contains(head) || self.enums[m].contains(head),
        }
    }

    /// Rewrite one call name occurring in a body owned by `module`, given that
    /// module's qualifier->module import map, the locals currently in scope,
    /// every module's `export:` list, and `span` for a located diagnostic.
    /// Returns `Ok(None)` to leave the name unchanged (unqualified: an own-
    /// module reference, which is never gated by export; qualified: absent
    /// from the target module, left for `check.rs`'s unknown-word error).
    /// Returns `Err` when the name resolves to a real decl in the target
    /// module that is not on its export list (R14/R16): a qualified `Type>`
    /// destructure is gated by its type's export status, one unit per R15.
    #[allow(clippy::too_many_arguments)]
    fn rewrite(
        &self,
        name: &str,
        module: u32,
        imports: &std::collections::HashMap<String, u32>,
        selective: &std::collections::HashMap<String, u32>,
        scope: &HashSet<String>,
        exports: &[Vec<(String, Span)>],
        span: Span,
    ) -> Result<Option<String>, String> {
        // A `&`/`&!` borrow prefix and a qualifier compose on one token
        // (`&!q::COUNT`), so the sigil is peeled off first and the rest
        // resolved against the same tables an unprefixed name would use. A
        // sigil is only ever meaningful ahead of a bare place name (a local,
        // caught above, or a static, below): a type name or a plain word can
        // never be borrowed, so under a sigil those must stay un-mangled and
        // fall through to `Ok(None)`, or a downstream "not a local"
        // diagnostic would name a mangled decl the surface program never
        // wrote. A field projection (`&f`) carries no type name at all, so it
        // never reaches the type/word branches below regardless of sigil.
        let (sigil, core) = strip_ref_sigil(name);
        if scope.contains(core) {
            return Ok(None);
        }
        let word_ok = sigil.is_empty();
        if let Some((qualifier, rest)) = core.split_once("::") {
            let target = match imports.get(qualifier) {
                Some(&t) => t,
                None => return Ok(None),
            };
            let (type_part, suffix) = split_destructure_suffix(rest);
            if word_ok && self.names_a_type(target, type_part, suffix) {
                if !is_exported(&exports[target as usize], type_part) {
                    return Err(not_exported_error(type_part, qualifier, span));
                }
                return Ok(Some(format!(
                    "{sigil}{}{}",
                    mangle(type_part, target),
                    suffix
                )));
            }
            // Review fix: `suffix` only means "the type branch above already
            // handled this" if `type_part` actually named a real type, which
            // that branch's own `return` already guarantees by the time
            // control reaches here -- gating on `suffix.is_empty()` too
            // silently dropped a plain word whose bare name happens to end in
            // `>`/`?` (Phase 6 slice 3's `Shape?`, or any pre-existing `Foo>`)
            // when no type of that head name exists, which is a real word
            // this table holds, not a destructure/eliminator collision.
            if word_ok && self.words[target as usize].contains(rest) {
                if !is_exported(&exports[target as usize], rest) {
                    return Err(not_exported_error(rest, qualifier, span));
                }
                return Ok(Some(format!("{sigil}{}", mangle(rest, target))));
            }
            return Ok(None);
        }
        let (type_part, suffix) = split_destructure_suffix(core);
        if word_ok && self.names_a_type(module, type_part, suffix) {
            return Ok(Some(format!(
                "{sigil}{}{}",
                mangle(type_part, module),
                suffix
            )));
        }
        // R2: a module static is the third name category the sigil can name.
        // It is module-private -- never exported, never imported -- so only an
        // accessor-free, unqualified name declared by the *accessing* module
        // resolves, and only under a sigil: a static is reachable no other
        // way, so a bare `COUNT` stays whatever it means today.
        if !sigil.is_empty() && self.statics[module as usize].contains(core) {
            return Ok(Some(format!("{sigil}{}", mangle_static(core, module))));
        }
        // A bare call to one of `check::builtin_table`'s operator names stays
        // unrewritten even when this module declares a same-named overload:
        // mangling it here would force *every* bare use inside the declaring
        // module onto that overload's declared signature, including ones
        // whose operands plainly mean the builtin (the overload's own body
        // summing two `i64` fields with `+`, say). Left bare, `check_operator`
        // still runs its own per-call-site, operand-type-directed dispatch
        // between the builtin and the overload, exactly as it does in a
        // single-module build (where this pass is a no-op and the name is
        // never touched at all).
        // Review fix: as in the qualified branch above, `suffix` non-empty
        // no longer means "skip -- already a destructure/eliminator"; the
        // type branch's own early `return` already owns that case.
        if word_ok && !is_operator_dispatch_name(core) && self.words[module as usize].contains(core)
        {
            return Ok(Some(format!("{sigil}{}", mangle(core, module))));
        }
        // R20/R15c: own module first, then a selectively imported name. The
        // map (validated by `check::check_selective_imports`) exposes a bare
        // `Type` together with its generated words as one unit, so a `Type>`
        // call whose `type_part` is selectively imported rewrites against the
        // target module just like a plain word does.
        if word_ok {
            if let Some(&target) = selective.get(type_part) {
                if self.names_a_type(target, type_part, suffix) {
                    return Ok(Some(format!(
                        "{sigil}{}{}",
                        mangle(type_part, target),
                        suffix
                    )));
                }
            }
        }
        // A selectively imported operator name stays unrewritten here for the
        // same reason the own-module branch above does: mangling it would force
        // *every* bare use in the importing module onto the imported overload's
        // signature, including ones plainly meant for the builtin (a plain
        // `1 2 +` in a module that selectively imports `Vec2 +`). Left bare,
        // `scoped_operator_overloads` (R12, `check/word_families.rs`) assembles
        // the imported overload as a candidate alongside the builtin and any own
        // overload, so `check_operator`'s per-call-site operand-type dispatch
        // still finds it -- this only defers the rewrite to that dispatch.
        if word_ok && !is_operator_dispatch_name(core) {
            if let Some(&target) = selective.get(core) {
                if self.words[target as usize].contains(core) {
                    return Ok(Some(format!("{sigil}{}", mangle(core, target))));
                }
            }
        }
        Ok(None)
    }
}

/// Whether `name` is on an `export:` list (R14/R16's positive check).
fn is_exported(exports: &[(String, Span)], name: &str) -> bool {
    exports.iter().any(|(n, _)| n == name)
}

/// R16: a qualified reference to a name that exists in the target module but
/// is not on its `export:` list. Distinct wording from an unknown name
/// (`check.rs`'s `unknown_word_error`), so the two cases are never conflated.
pub(crate) fn not_exported_error(name: &str, qualifier: &str, span: Span) -> String {
    format!(
        "error: `{name}` is not exported from module `{qualifier}` at line {}, col {}",
        span.line, span.col
    )
}

/// Mangle every decl name for a multi-module closure and rewrite every body to
/// match. Below two modules the pass is a no-op *unless* `always_mangle` is set
/// (R22): the REPL import path leaves a single-file closure raw and does its own
/// epoch renaming, while the native build path forces mangling even for one
/// module so a user word whose bare name equals a libc symbol (`close`) or a
/// runtime shim's callee (`free`, called from `sooth_free`) becomes `close__m0`
/// / `free__m0` and can no longer hijack that symbol at link time.
pub fn resolve_modules(module: &mut Module, always_mangle: bool) -> Result<(), String> {
    if module.modules.len() < 2 && !always_mangle {
        return Ok(());
    }
    let tables = NameTables::build(module);

    // The word bodies are rewritten first, reading the still-raw decl names via
    // `tables`; only then are the decl names themselves mangled. `words` is
    // split out so a body's own module id and import map drive its rewrite.
    let import_maps: Vec<std::collections::HashMap<String, u32>> =
        module.modules.iter().map(|m| m.imports.clone()).collect();
    let selectives: Vec<std::collections::HashMap<String, u32>> =
        module.modules.iter().map(|m| m.selective.clone()).collect();
    let exports: Vec<Vec<(String, Span)>> =
        module.modules.iter().map(|m| m.exports.clone()).collect();
    for word in &mut module.words {
        let imports = &import_maps[word.module as usize];
        let selective = &selectives[word.module as usize];
        let mut scope = HashSet::new();
        match &mut word.body {
            WordBody::Terms { terms } => {
                rewrite_terms(
                    terms,
                    word.module,
                    imports,
                    selective,
                    &tables,
                    &mut scope,
                    &exports,
                )?;
            }
            WordBody::Clauses(clauses) => {
                for clause in clauses {
                    rewrite_clause(
                        clause,
                        word.module,
                        imports,
                        selective,
                        &tables,
                        &mut scope,
                        &exports,
                    )?;
                }
            }
        }
    }

    for s in &mut module.structs {
        s.name = mangle(&s.name, s.module);
    }
    for e in &mut module.enums {
        e.name = mangle(&e.name, e.module);
    }
    // A forced single-module closure (the native build path's hijack fix) has
    // no qualified calls, so an operator-named overload is only ever reached by
    // a bare call, which the rewrite above deliberately left unmangled for
    // `check_operator`'s operand-type dispatch (over the candidate set keyed by
    // the bare name). Its decl must therefore stay bare too, or that lookup
    // would miss it and fall through to the builtin, rejecting struct operands.
    // A genuine multi-module closure keeps mangling operator decls: a qualified
    // `v::+` *is* rewritten to `+__m1` (a cross-module use names one module's
    // overload directly), so the decl it targets must carry the same mangled
    // name. An operator symbol never collides with a libc name (`qbe_name`
    // escapes `+` to `.2b.`), so leaving it bare here reintroduces no hijack.
    let single = module.modules.len() < 2;
    for w in &mut module.words {
        if single && is_operator_dispatch_name(&w.name) {
            continue;
        }
        w.name = mangle(&w.name, w.module);
    }
    for x in &mut module.externs {
        x.name = mangle(&x.name, x.module);
    }
    // R2: a static's data symbol is module-scoped exactly as a word's is, so
    // two modules may each declare `COUNT` without colliding at codegen --
    // and, via `mangle_static`, without colliding on `drop`/`main`/a prelude
    // word either. No operator-name carve-out applies: a static is never a
    // dispatch name.
    for s in &mut module.statics {
        s.name = mangle_static(&s.name, s.module);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn rewrite_clause(
    clause: &mut Clause,
    module: u32,
    imports: &std::collections::HashMap<String, u32>,
    selective: &std::collections::HashMap<String, u32>,
    tables: &NameTables,
    scope: &mut HashSet<String>,
    exports: &[Vec<(String, Span)>],
) -> Result<(), String> {
    let base = scope.len();
    let added: Vec<String> = clause.locals.clone();
    for name in &added {
        scope.insert(name.clone());
    }
    rewrite_terms(
        &mut clause.body,
        module,
        imports,
        selective,
        tables,
        scope,
        exports,
    )?;
    truncate_scope(scope, base, &added);
    Ok(())
}

/// Rewrite a block of terms left to right. A `Bind` extends the scope for the
/// rest of this block (and any nested block that follows); the added names are
/// removed when the block ends, so a sibling block does not see them.
#[allow(clippy::too_many_arguments)]
fn rewrite_terms(
    terms: &mut [Term],
    module: u32,
    imports: &std::collections::HashMap<String, u32>,
    selective: &std::collections::HashMap<String, u32>,
    tables: &NameTables,
    scope: &mut HashSet<String>,
    exports: &[Vec<(String, Span)>],
) -> Result<(), String> {
    let base = scope.len();
    let mut added: Vec<String> = Vec::new();
    for term in terms.iter_mut() {
        match &mut term.kind {
            TermKind::Call(name) => {
                if let Some(new) =
                    tables.rewrite(name, module, imports, selective, scope, exports, term.span)?
                {
                    *name = new;
                }
            }
            TermKind::Bind(names) => {
                for name in names.iter() {
                    if scope.insert(name.clone()) {
                        added.push(name.clone());
                    }
                }
            }
            TermKind::Quotation(inner, _, _) => {
                rewrite_terms(inner, module, imports, selective, tables, scope, exports)?;
            }
            // Slice 6h: an array constructor carries an already-resolved
            // `Type::Array(id)`, not a name, so there is nothing to rewrite.
            TermKind::ArrayCtor(_)
            | TermKind::IntLit(_)
            | TermKind::FloatLit(_)
            | TermKind::StrLit(_) => {}
        }
    }
    truncate_scope(scope, base, &added);
    Ok(())
}

/// Remove the names this block added, restoring the scope to its entry size.
/// Only names newly inserted here are removed (a `Bind` that re-bound an outer
/// name added nothing), so an outer local survives an inner shadow.
fn truncate_scope(scope: &mut HashSet<String>, base: usize, added: &[String]) {
    if scope.len() == base {
        return;
    }
    for name in added {
        scope.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::{parse_bodies, scan_imports};

    #[test]
    fn split_destructure_suffix_finds_the_type_prefix() {
        assert_eq!(split_destructure_suffix("Point"), ("Point", ""));
        assert_eq!(split_destructure_suffix("Point>"), ("Point", ">"));
        assert_eq!(split_destructure_suffix(">"), ("", ">"));
        assert_eq!(split_destructure_suffix("Shape?"), ("Shape", "?"));
    }

    #[test]
    fn mangle_keeps_main_and_suffixes_others() {
        assert_eq!(mangle("main", 0), "main");
        assert_eq!(mangle("push", 1), "push__m1");
        assert_eq!(mangle("Point", 0), "Point__m0");
    }

    /// R2: a static inherits none of `mangle`'s exemptions. Each name below is
    /// one `mangle` returns raw, and each used to reach the backend as a raw
    /// data symbol that a second module's static -- or, for `main`, the entry
    /// word's own `sooth_main` -- then collided with at the assembler.
    #[test]
    fn mangle_static_suffixes_even_the_exempt_word_names() {
        for name in ["main", "drop", "if"] {
            assert_eq!(mangle(name, 1), name, "precondition: {name} is exempt");
            assert_eq!(mangle_static(name, 1), format!("{name}__m1"));
        }
        assert_eq!(mangle_static("COUNT", 2), "COUNT__m2");
    }

    /// U5 (R16): a name that exists in the target module but is not on its
    /// `export:` list is a located `Err`; a name absent from the target
    /// module entirely is `Ok(None)`, left for `check.rs`'s unknown-word
    /// error. The two must never collide on the same result shape.
    #[test]
    fn visibility_lookup_distinguishes_unexported_from_absent() {
        let mut words = vec![HashSet::new(), HashSet::new()];
        words[1].insert("grow".to_string());
        let mut enums = vec![HashSet::new(), HashSet::new()];
        enums[1].insert("Hidden".to_string());
        let tables = NameTables {
            structs: vec![HashSet::new(), HashSet::new()],
            enums,
            words,
            statics: vec![HashSet::new(), HashSet::new()],
        };
        let mut imports = std::collections::HashMap::new();
        imports.insert("q".to_string(), 1u32);
        let exports = vec![Vec::new(), Vec::new()]; // module 1 exports nothing
        let scope = HashSet::new();
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };

        let no_selective = std::collections::HashMap::new();
        let unexported = tables.rewrite(
            "q::grow",
            0,
            &imports,
            &no_selective,
            &scope,
            &exports,
            span,
        );
        assert!(
            matches!(unexported, Err(ref e) if e.contains("not exported")),
            "existing-but-private name is a located error: {unexported:?}"
        );

        let absent = tables.rewrite(
            "q::missing",
            0,
            &imports,
            &no_selective,
            &scope,
            &exports,
            span,
        );
        assert_eq!(absent, Ok(None), "absent name defers to unknown-word");

        // An enum is a type for an *unsuffixed* reference too, which is what
        // makes a qualified one answer R16 here rather than falling through to
        // unknown-word. (`?` eligibility is the enum set's other reader, see
        // `names_a_type`.)
        let private_enum = tables.rewrite(
            "q::Hidden",
            0,
            &imports,
            &no_selective,
            &scope,
            &exports,
            span,
        );
        assert!(
            matches!(private_enum, Err(ref e) if e.contains("`Hidden` is not exported")),
            "an unexported enum is a located error, not unknown-word: {private_enum:?}"
        );
    }

    /// U6 (R15): naming a type in `export:` exports it and its two generated
    /// words (constructor, destructure) as one unit, gated by the type's own
    /// single export entry rather than two separate ones. Per-field access is
    /// a receiver-directed projection (`&f`), which carries no type name and
    /// so has no export entry of its own to gate.
    #[test]
    fn export_of_type_includes_both_generated_words() {
        let mut structs = vec![HashSet::new(), HashSet::new()];
        structs[1].insert("Point".to_string());
        let tables = NameTables {
            structs,
            enums: vec![HashSet::new(), HashSet::new()],
            words: vec![HashSet::new(), HashSet::new()],
            statics: vec![HashSet::new(), HashSet::new()],
        };
        let mut imports = std::collections::HashMap::new();
        imports.insert("geo".to_string(), 1u32);
        let exports = vec![
            Vec::new(),
            vec![(
                "Point".to_string(),
                Span {
                    line: 1,
                    col: 1,
                    module: 0,
                },
            )],
        ];
        let scope = HashSet::new();
        let span = Span {
            line: 2,
            col: 3,
            module: 0,
        };

        for spelling in [
            "geo::Point",  // constructor
            "geo::Point>", // destructure
        ] {
            let no_selective = std::collections::HashMap::new();
            let result =
                tables.rewrite(spelling, 0, &imports, &no_selective, &scope, &exports, span);
            assert!(
                result.is_ok(),
                "{spelling} resolves once its type is exported: {result:?}"
            );
        }
    }

    /// U9 (ast/ir): two modules that each define a word `p` mint distinct
    /// names after resolution, so their symbols cannot collide; a single-module
    /// closure is left exactly as parsed.
    #[test]
    fn same_named_words_across_modules_get_distinct_symbols() {
        // Two modules, module 1 imported by module 0 under qualifier `lib`,
        // each with a word `p`. Assembled by hand into one module the way the
        // driver would, then resolved.
        let mut module = assemble_two_modules(
            ": p ( -- i64 ) 1 ; : main ( -- ) lib::p drop p drop ;",
            ": p ( -- i64 ) 2 ;\nexport: p ;",
        );
        resolve_modules(&mut module, false).unwrap();
        let names: Vec<&str> = module.words.iter().map(|w| w.name.as_str()).collect();
        assert!(names.contains(&"p__m0"), "module 0's p mangled: {names:?}");
        assert!(names.contains(&"p__m1"), "module 1's p mangled: {names:?}");
        assert!(names.contains(&"main"), "main is never mangled: {names:?}");
        // The body call to `lib::p` resolved to module 1's mangled name, and
        // the unqualified `p` to module 0's, so check/emit see two distinct
        // callees.
        let main = module.words.iter().find(|w| w.name == "main").unwrap();
        let calls = call_names(&main.body);
        assert!(calls.contains(&"p__m1".to_string()), "qualified: {calls:?}");
        assert!(
            calls.contains(&"p__m0".to_string()),
            "unqualified: {calls:?}"
        );
    }

    /// A sigil is only ever meaningful ahead of a bare place name (a local
    /// or a static); a field projection (`&f`) carries no type name at all,
    /// and a bare word or type name can never be borrowed. Those must stay
    /// unmangled so a "not a local" diagnostic downstream names the spelling
    /// the author wrote, not a decl the sigil was never resolved against.
    #[test]
    fn sigiled_plain_word_or_bare_type_is_left_unmangled() {
        let mut module = assemble_two_modules(
            "type: Acc arr [i64 4] ;\n\
             : dst ( -- i64 ) 5 ;\n\
             : main ( -- ) &!dst drop ;",
            ": noop ( -- ) ;",
        );
        resolve_modules(&mut module, false).unwrap();
        let main = module.words.iter().find(|w| w.name == "main").unwrap();
        let calls = call_names(&main.body);
        assert!(
            calls.contains(&"&!dst".to_string()),
            "a sigiled word call is never a borrow and stays raw: {calls:?}"
        );
    }

    /// R2: each module's `static:` decl is mangled per module, and a
    /// `&NAME`/`&!NAME` borrow rewrites against the *accessing* module's own
    /// table -- two modules may each declare `COUNT` and neither reaches the
    /// other's.
    #[test]
    fn static_borrow_resolves_to_the_accessing_modules_own_static() {
        let mut module = assemble_two_modules(
            "static: COUNT i64 = 0 ;\n\
             : main ( -- ) &!COUNT 1 +! ;",
            "static: COUNT i64 = 0 ;\n\
             : bump ( -- ) &COUNT drop ;",
        );
        resolve_modules(&mut module, false).unwrap();
        let names: Vec<&str> = module.statics.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["COUNT__m0", "COUNT__m1"]);
        let main = module.words.iter().find(|w| w.name == "main").unwrap();
        assert!(
            call_names(&main.body).contains(&"&!COUNT__m0".to_string()),
            "module 0 borrows its own static: {:?}",
            call_names(&main.body)
        );
        let bump = module.words.iter().find(|w| w.name == "bump__m1").unwrap();
        assert!(
            call_names(&bump.body).contains(&"&COUNT__m1".to_string()),
            "module 1 borrows its own static: {:?}",
            call_names(&bump.body)
        );
    }

    /// R2: a static is reachable only through the sigil, so an unsigilled
    /// `COUNT` is left raw -- it means whatever it means today (a word call,
    /// or the checker's unknown-word error naming the spelling the author
    /// wrote), not a silently mangled static.
    #[test]
    fn unsigiled_static_name_is_left_unmangled() {
        let mut statics = vec![HashSet::new(), HashSet::new()];
        statics[0].insert("COUNT".to_string());
        let tables = NameTables {
            structs: vec![HashSet::new(), HashSet::new()],
            enums: vec![HashSet::new(), HashSet::new()],
            words: vec![HashSet::new(), HashSet::new()],
            statics,
        };
        let imports = std::collections::HashMap::new();
        let selective = std::collections::HashMap::new();
        let exports = vec![Vec::new(), Vec::new()];
        let scope = HashSet::new();
        let span = Span::default();
        let rewrite = |name: &str, module: u32| {
            tables
                .rewrite(name, module, &imports, &selective, &scope, &exports, span)
                .unwrap()
        };
        assert_eq!(rewrite("COUNT", 0), None, "no sigil, no static rewrite");
        assert_eq!(rewrite("&COUNT", 0), Some("&COUNT__m0".to_string()));
        assert_eq!(
            rewrite("&COUNT", 1),
            None,
            "module 1 declares no `COUNT`: a static is module-private"
        );
    }

    /// The native build path forces mangling even for one module, so a user
    /// word whose bare name equals a libc symbol (`close`) can no longer be
    /// emitted as the bare symbol and hijack it: it becomes `close__m0`, while
    /// `main` (the C entry) and `drop` (dispatched by literal name) are left
    /// alone exactly as in a multi-module closure. The call site to `close` is
    /// rewritten in step so the definition and its callers still agree.
    #[test]
    fn single_module_forced_mangle_renames_libc_named_word() {
        let tokens = lex(": close ( -- ) ; : main ( -- ) close ;").unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        resolve_modules(&mut module, true).unwrap();
        let names: Vec<&str> = module.words.iter().map(|w| w.name.as_str()).collect();
        assert!(
            names.contains(&"close__m0"),
            "user close mangled: {names:?}"
        );
        assert!(
            !names.contains(&"close"),
            "bare close no longer emitted: {names:?}"
        );
        assert!(names.contains(&"main"), "main is never mangled: {names:?}");
        let main = module.words.iter().find(|w| w.name == "main").unwrap();
        assert_eq!(
            call_names(&main.body),
            vec!["close__m0".to_string()],
            "the call site is rewritten to match the mangled definition"
        );
    }

    /// Phase 6 slice 3 review fix (finding 4): a real build mangles the
    /// enum-based eliminator key (`Shape__m0?`), not the ordinary
    /// suffix-appended form (`Shape?__m0`) -- `check_src`-style tests never
    /// exercise this at all, since they skip `resolve_modules` entirely and
    /// leave every name bare, so the call site's mangled form has to be
    /// pinned here instead.
    #[test]
    fn eliminator_call_site_mangles_to_match_the_enum_based_key() {
        let tokens = lex("type: Shape | Circle r i64 | Rect w i64 h i64 ;\n\
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( Rect ) Rect> * ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n")
        .unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        resolve_modules(&mut module, true).unwrap();
        let shape_id = module
            .enums
            .iter()
            .position(|e| e.name == "Shape__m0")
            .expect("`Shape` is mangled by declaration order, unconditionally");
        assert_eq!(
            shape_id,
            module.enums.len() - 1,
            "Shape is the last-declared enum"
        );
        let area = module.words.iter().find(|w| w.name == "area__m0").unwrap();
        assert!(
            call_names(&area.body).contains(&"Shape__m0?".to_string()),
            "the eliminator call site mangles to the enum-based key, matching \
             `eliminator_registry`'s own `\"{{EnumName}}?\"` keying, not the \
             ordinary word-suffix form `Shape?__m0`: {:?}",
            call_names(&area.body)
        );
    }

    /// Phase 6 slice 3 review fix (cycle 2): teaching `split_destructure_suffix`
    /// about the eliminator's `?` made every *plain* word whose name ends in
    /// `?` (`ok?`, `zero?` -- an ordinary spelling) present a non-empty
    /// suffix to the four name-table branches below, each of which then
    /// skipped it and left the call unresolved (`unknown word` across
    /// modules). A generated word is recognized by its *type prefix* naming a
    /// real type, and that branch returns on its own, so the suffix alone
    /// must never gate the rest. Each assertion below fails if its branch's
    /// `suffix.is_empty()` gate comes back, and every one of them is
    /// reachable from real source (a qualified call, an own-module call, a
    /// borrowed `static:`, a selective import).
    #[test]
    fn word_named_with_a_generated_suffix_resolves_in_every_branch() {
        let mut enums = vec![HashSet::new(), HashSet::new()];
        enums[0].insert("Shape".to_string());
        let mut words = vec![HashSet::new(), HashSet::new()];
        words[0].insert("own?".to_string());
        words[1].insert("ok?".to_string());
        let mut statics = vec![HashSet::new(), HashSet::new()];
        statics[0].insert("FLAG?".to_string());
        let tables = NameTables {
            structs: vec![HashSet::new(), HashSet::new()],
            enums,
            words,
            statics,
        };
        let mut imports = std::collections::HashMap::new();
        imports.insert("lib".to_string(), 1u32);
        let exports = vec![Vec::new(), vec![("ok?".to_string(), Span::default())]];
        let scope = HashSet::new();
        let span = Span::default();
        let none = std::collections::HashMap::new();
        let mut selective = std::collections::HashMap::new();
        selective.insert("ok?".to_string(), 1u32);
        let at = |name: &str, sel: &std::collections::HashMap<String, u32>| {
            tables.rewrite(name, 0, &imports, sel, &scope, &exports, span)
        };

        assert_eq!(
            at("lib::ok?", &none),
            Ok(Some("ok?__m1".to_string())),
            "qualified call to another module's word"
        );
        assert_eq!(
            at("own?", &none),
            Ok(Some("own?__m0".to_string())),
            "unqualified call to this module's own word"
        );
        assert_eq!(
            at("&!FLAG?", &none),
            Ok(Some("&!FLAG?__m0".to_string())),
            "borrow of this module's own static"
        );
        assert_eq!(
            at("ok?", &selective),
            Ok(Some("ok?__m1".to_string())),
            "selectively imported word, bare"
        );
        assert_eq!(
            at("Shape?", &none),
            Ok(Some("Shape__m0?".to_string())),
            "the eliminator itself still resolves through the type branch, \
             which is what the suffix is actually for"
        );
    }

    /// Phase 6 slice 3 review fix (cycle 3): a generated suffix belongs to
    /// exactly one *kind* of type -- `>` is the struct destructure, `?` the
    /// enum eliminator -- so the type branch's eligibility is per suffix, not
    /// "the head names a type at all". Under the looser gate a plain word
    /// called `P?` beside a *struct* `P` was rewritten to `P__m0?`, a
    /// generated name nothing generates, and a call that resolved before this
    /// slice became `unknown word`; `E>` beside an *enum* `E` was the same
    /// hole in the other direction (that one predates this slice). Both plain
    /// words and both genuinely generated names are asserted, so "never
    /// mangle a suffixed name" does not pass either.
    #[test]
    fn word_named_for_another_kinds_generated_suffix_stays_a_word() {
        let tokens = lex("type: P x i64 ;\n\
             type: E | A y i64 ;\n\
             : P? ( i64 -- bool ) 0 > ;\n\
             : E> ( i64 -- bool ) 0 > ;\n\
             : main ( -- ) 3 P? drop 3 E> drop 1 P P> drop E? ;\n")
        .unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        resolve_modules(&mut module, true).unwrap();
        let main = module.words.iter().find(|w| w.name == "main").unwrap();
        assert_eq!(
            call_names(&main.body),
            vec![
                "P?__m0".to_string(),
                "drop".to_string(),
                "E>__m0".to_string(),
                "drop".to_string(),
                "P__m0".to_string(),
                "P__m0>".to_string(),
                "drop".to_string(),
                "E__m0?".to_string(),
            ],
            "`P?`/`E>` are words (mangled as words); `P`/`P>`/`E?` are the \
             generated names (mangled through the type branch)"
        );
        let names: Vec<&str> = module.words.iter().map(|w| w.name.as_str()).collect();
        assert!(
            names.contains(&"P?__m0") && names.contains(&"E>__m0"),
            "both declarations mangle as ordinary words, matching their call \
             sites: {names:?}"
        );
    }

    #[test]
    fn single_module_closure_is_left_unchanged() {
        let tokens = lex(": p ( -- i64 ) 1 ; : main ( -- ) p drop ;").unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        resolve_modules(&mut module, false).unwrap();
        // Slice 10c: `parse` appends `lib/core.sth`'s words, which are never
        // mangled (`is_prelude_word_name`), so only the file's own two are
        // asserted here.
        let names: Vec<&str> = module
            .words
            .iter()
            .map(|w| w.name.as_str())
            .filter(|n| !is_prelude_word_name(n))
            .collect();
        assert_eq!(names, vec!["p", "main"]);
        let main = module.words.iter().find(|w| w.name == "main").unwrap();
        assert_eq!(
            call_names(&main.body),
            vec!["p".to_string(), "drop".to_string()]
        );
    }

    fn call_names(body: &WordBody) -> Vec<String> {
        let mut out = Vec::new();
        if let WordBody::Terms { terms } = body {
            collect_calls(terms, &mut out);
        }
        out
    }

    fn collect_calls(terms: &[Term], out: &mut Vec<String>) {
        for t in terms {
            match &t.kind {
                TermKind::Call(n) => out.push(n.clone()),
                TermKind::Quotation(inner, _, _) => collect_calls(inner, out),
                _ => {}
            }
        }
    }

    /// Assemble module 0 (`entry`, importing module 1 as `lib`) and module 1
    /// (`lib_src`) into one merged module, mirroring the driver's two-file
    /// closure assembly closely enough to drive `resolve_modules`.
    fn assemble_two_modules(entry: &str, lib_src: &str) -> Module {
        use crate::ast::ModuleInfo;
        use std::collections::HashMap;
        let entry_tokens = lex(entry).unwrap();
        let lib_tokens = lex(lib_src).unwrap();
        let _ = scan_imports(&entry_tokens).unwrap();
        let mut structs = Vec::new();
        let mut enums = Vec::new();
        crate::parser::prepass_and_register(&entry_tokens, 0, &mut structs, &mut enums).unwrap();
        crate::parser::prepass_and_register(&lib_tokens, 1, &mut structs, &mut enums).unwrap();
        let mut arrays = Vec::new();
        let mut owned_cells = Vec::new();
        let mut refs = Vec::new();
        let mut generics = crate::ast::GenericTypes::with_bases(structs.len(), enums.len());
        let mut imports0: HashMap<String, u32> = HashMap::new();
        imports0.insert("lib".to_string(), 1);
        let no_imports: HashMap<String, u32> = HashMap::new();
        let exports_by_module = vec![
            crate::parser::scan_exports(&entry_tokens).unwrap(),
            crate::parser::scan_exports(&lib_tokens).unwrap(),
        ];
        let entry_bodies = parse_bodies(
            &entry_tokens,
            &structs,
            &enums,
            0,
            &imports0,
            &exports_by_module,
            &no_imports,
            &mut arrays,
            &mut owned_cells,
            &mut refs,
            &mut generics,
        )
        .unwrap();
        let lib_bodies = parse_bodies(
            &lib_tokens,
            &structs,
            &enums,
            1,
            &no_imports,
            &exports_by_module,
            &no_imports,
            &mut arrays,
            &mut owned_cells,
            &mut refs,
            &mut generics,
        )
        .unwrap();
        let mut words = entry_bodies.words;
        words.extend(lib_bodies.words);
        let mut statics = entry_bodies.statics;
        statics.extend(lib_bodies.statics);
        Module {
            words,
            structs,
            enums,
            arrays,
            owned_cells,
            refs,
            generic_structs: Vec::new(),
            generic_enums: Vec::new(),
            externs: Vec::new(),
            instantiations: HashMap::new(),
            builtin_overloads: HashMap::new(),
            resolved_fields: HashMap::new(),
            resolved_variant_fields: HashMap::new(),
            modules: vec![
                ModuleInfo {
                    imports: imports0,
                    exports: entry_bodies.exports,
                    selective: HashMap::new(),
                },
                ModuleInfo {
                    imports: no_imports,
                    exports: lib_bodies.exports,
                    selective: HashMap::new(),
                },
            ],
            statics,
        }
    }
}
