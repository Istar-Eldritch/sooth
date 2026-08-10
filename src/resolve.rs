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
fn mangle(name: &str, module: u32) -> String {
    if name == "main" || name == "drop" {
        return name.to_string();
    }
    format!("{name}__m{module}")
}

/// `check::builtin_table`'s keys, mirrored by hand (that table's own
/// `check_operator::is_operator` list carries the same warning): the
/// arithmetic/comparison/`max`/`max-total`/`.` names a bare call must reach
/// `check_operator`'s operand-type dispatch for, never a static rewrite. Keep
/// in sync when a table operator is added.
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

/// `demangle_word` for a *call* name, which may carry an accessor suffix. A
/// generated accessor mangles as `P__m0>x`, so the `__m0` sits mid-string and
/// the trailing-suffix strip above cannot see it; the type prefix has to be
/// demangled and the `>x` put back.
pub(crate) fn demangle_call(name: &str) -> std::borrow::Cow<'_, str> {
    let (head, accessor) = split_accessor(name);
    if accessor.is_empty() {
        return std::borrow::Cow::Borrowed(demangle_word(head));
    }
    let demangled = demangle_word(head);
    if demangled.len() == head.len() {
        return std::borrow::Cow::Borrowed(name);
    }
    std::borrow::Cow::Owned(format!("{demangled}{accessor}"))
}

/// Split a call name into its leading identifier and the accessor suffix a
/// generated word carries: `Point>x` -> (`Point`, `>x`), `Point|>x` ->
/// (`Point`, `|>x`), `Point` -> (`Point`, ``). The generated-word spellings are
/// `Type>field` / `Type<field` / `Type|>field` (`check.rs`, `ir.rs`), and a
/// type name never contains `<`, `>`, or `|`, so the first of those characters
/// is the boundary.
pub(crate) fn split_accessor(name: &str) -> (&str, &str) {
    match name.find(['>', '<', '|']) {
        Some(i) => name.split_at(i),
        None => (name, ""),
    }
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

/// The per-module name tables the rewrite consults: which bare names are this
/// module's types (so a constructor/accessor call rewrites) and which are its
/// words/externs (so a plain call rewrites). Builtins and genuinely unknown
/// names appear in neither and are left raw for the checker to resolve or
/// reject.
struct NameTables {
    types: Vec<HashSet<String>>,
    words: Vec<HashSet<String>>,
}

impl NameTables {
    fn build(module: &Module) -> NameTables {
        let n = module.modules.len();
        let mut types = vec![HashSet::new(); n];
        let mut words = vec![HashSet::new(); n];
        for s in &module.structs {
            types[s.module as usize].insert(s.name.clone());
        }
        for e in &module.enums {
            types[e.module as usize].insert(e.name.clone());
        }
        for w in &module.words {
            words[w.module as usize].insert(w.name.clone());
        }
        for x in &module.externs {
            words[x.module as usize].insert(x.name.clone());
        }
        NameTables { types, words }
    }

    /// Rewrite one call name occurring in a body owned by `module`, given that
    /// module's qualifier->module import map, the locals currently in scope,
    /// every module's `export:` list, and `span` for a located diagnostic.
    /// Returns `Ok(None)` to leave the name unchanged (unqualified: an own-
    /// module reference, which is never gated by export; qualified: absent
    /// from the target module, left for `check.rs`'s unknown-word error).
    /// Returns `Err` when the name resolves to a real decl in the target
    /// module that is not on its export list (R14/R16): a qualified
    /// `Type>field`/`Type<field`/`Type|>field`/`Type>` accessor is gated by
    /// its type's export status, one unit per R15.
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
        // A `&`/`&!` borrow prefix and a qualifier/accessor compose on one
        // token (`&!q::Point>x`), so the sigil is peeled off first and the
        // rest resolved against the same tables an unprefixed name would use.
        // But a sigil is only ever meaningful ahead of the one accessor form
        // `check.rs`'s struct-projection branch parses back out itself
        // (`rest.split_once('>')`, i.e. a suffix starting with `>`): a bare
        // word or a bare/other-accessor type name can never be borrowed, so
        // under a sigil those must stay un-mangled and fall through to
        // `Ok(None)`, or a downstream "not a local" diagnostic would name a
        // mangled decl the surface program never wrote.
        let (sigil, core) = strip_ref_sigil(name);
        if scope.contains(core) {
            return Ok(None);
        }
        let type_ok = |suffix: &str| sigil.is_empty() || suffix.starts_with('>');
        let word_ok = sigil.is_empty();
        if let Some((qualifier, rest)) = core.split_once("::") {
            let target = match imports.get(qualifier) {
                Some(&t) => t,
                None => return Ok(None),
            };
            let (type_part, suffix) = split_accessor(rest);
            if type_ok(suffix) && self.types[target as usize].contains(type_part) {
                if !is_exported(&exports[target as usize], type_part) {
                    return Err(not_exported_error(type_part, qualifier, span));
                }
                return Ok(Some(format!(
                    "{sigil}{}{}",
                    mangle(type_part, target),
                    suffix
                )));
            }
            if word_ok && suffix.is_empty() && self.words[target as usize].contains(rest) {
                if !is_exported(&exports[target as usize], rest) {
                    return Err(not_exported_error(rest, qualifier, span));
                }
                return Ok(Some(format!("{sigil}{}", mangle(rest, target))));
            }
            return Ok(None);
        }
        let (type_part, suffix) = split_accessor(core);
        if type_ok(suffix) && self.types[module as usize].contains(type_part) {
            return Ok(Some(format!(
                "{sigil}{}{}",
                mangle(type_part, module),
                suffix
            )));
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
        if word_ok
            && suffix.is_empty()
            && !is_operator_dispatch_name(core)
            && self.words[module as usize].contains(core)
        {
            return Ok(Some(format!("{sigil}{}", mangle(core, module))));
        }
        // R20/R15c: own module first, then a selectively imported name. The
        // map (validated by `check::check_selective_imports`) exposes a bare
        // `Type` together with its generated words as one unit, so a
        // `Type>field` call whose `type_part` is selectively imported rewrites
        // against the target module just like a plain word does.
        if type_ok(suffix) {
            if let Some(&target) = selective.get(type_part) {
                if self.types[target as usize].contains(type_part) {
                    return Ok(Some(format!(
                        "{sigil}{}{}",
                        mangle(type_part, target),
                        suffix
                    )));
                }
            }
        }
        if word_ok && suffix.is_empty() {
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
            TermKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                rewrite_terms(
                    then_branch,
                    module,
                    imports,
                    selective,
                    tables,
                    scope,
                    exports,
                )?;
                rewrite_terms(
                    else_branch,
                    module,
                    imports,
                    selective,
                    tables,
                    scope,
                    exports,
                )?;
            }
            TermKind::Quotation(inner) => {
                rewrite_terms(inner, module, imports, selective, tables, scope, exports)?;
            }
            TermKind::IntLit(_) | TermKind::FloatLit(_) | TermKind::StrLit(_) => {}
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
    fn split_accessor_finds_the_type_prefix() {
        assert_eq!(split_accessor("Point"), ("Point", ""));
        assert_eq!(split_accessor("Point>x"), ("Point", ">x"));
        assert_eq!(split_accessor("Point<x"), ("Point", "<x"));
        assert_eq!(split_accessor("Point|>x"), ("Point", "|>x"));
        assert_eq!(split_accessor(">"), ("", ">"));
    }

    #[test]
    fn mangle_keeps_main_and_suffixes_others() {
        assert_eq!(mangle("main", 0), "main");
        assert_eq!(mangle("push", 1), "push__m1");
        assert_eq!(mangle("Point", 0), "Point__m0");
    }

    /// U5 (R16): a name that exists in the target module but is not on its
    /// `export:` list is a located `Err`; a name absent from the target
    /// module entirely is `Ok(None)`, left for `check.rs`'s unknown-word
    /// error. The two must never collide on the same result shape.
    #[test]
    fn visibility_lookup_distinguishes_unexported_from_absent() {
        let mut words = vec![HashSet::new(), HashSet::new()];
        words[1].insert("grow".to_string());
        let tables = NameTables {
            types: vec![HashSet::new(), HashSet::new()],
            words,
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
    }

    /// U6 (R15): naming a type in `export:` exports it and its five generated
    /// words (constructor, destructure, getter, setter, peek) as one unit,
    /// gated by the type's own single export entry rather than five separate
    /// ones.
    #[test]
    fn export_of_type_includes_all_five_generated_words() {
        let mut types = vec![HashSet::new(), HashSet::new()];
        types[1].insert("Point".to_string());
        let tables = NameTables {
            types,
            words: vec![HashSet::new(), HashSet::new()],
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
            "geo::Point",    // constructor
            "geo::Point>",   // destructure
            "geo::Point>x",  // getter
            "geo::Point<x",  // setter
            "geo::Point|>x", // peek
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

    /// A `&`/`&!` borrow prefix and a struct field accessor compose on one
    /// token (`&!Acc>arr`), so the mangling rewrite must see past the sigil
    /// to the type name underneath it: with only a *second*, unrelated module
    /// present, an unqualified `&!Acc>arr` in module 0 must still mangle to
    /// `Acc`'s module-0 name, exactly as the unprefixed `Acc>arr` already
    /// does.
    #[test]
    fn sigiled_struct_field_accessor_mangles_past_the_borrow_prefix() {
        let mut module = assemble_two_modules(
            "type: Acc arr [i64 4] ;\n\
             : main ( -- )\n\
             0 4 fill Acc | acc |\n\
             &!acc &!Acc>arr | a |\n\
             a Acc>arr drop\n\
             acc drop ;",
            ": noop ( -- ) ;",
        );
        resolve_modules(&mut module, false).unwrap();
        let main = module.words.iter().find(|w| w.name == "main").unwrap();
        let calls = call_names(&main.body);
        assert!(
            calls.contains(&"&!Acc__m0>arr".to_string()),
            "sigiled accessor mangled: {calls:?}"
        );
        assert!(
            calls.contains(&"Acc__m0>arr".to_string()),
            "unprefixed accessor mangled the same way: {calls:?}"
        );
    }

    /// The converse: a sigil is only ever meaningful ahead of a `Type>field`
    /// accessor (the one form `check.rs`'s struct-projection branch parses
    /// back out), never ahead of a plain word or a bare type name. Those must
    /// stay unmangled so a "not a local" diagnostic downstream names the
    /// spelling the author wrote, not a decl the sigil was never resolved
    /// against.
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

    #[test]
    fn single_module_closure_is_left_unchanged() {
        let tokens = lex(": p ( -- i64 ) 1 ; : main ( -- ) p drop ;").unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        resolve_modules(&mut module, false).unwrap();
        let names: Vec<&str> = module.words.iter().map(|w| w.name.as_str()).collect();
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
                TermKind::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    collect_calls(then_branch, out);
                    collect_calls(else_branch, out);
                }
                TermKind::Quotation(inner) => collect_calls(inner, out),
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
        )
        .unwrap();
        let mut words = entry_bodies.words;
        words.extend(lib_bodies.words);
        Module {
            words,
            structs,
            enums,
            arrays,
            owned_cells,
            refs,
            externs: Vec::new(),
            instantiations: HashMap::new(),
            builtin_overloads: HashMap::new(),
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
        }
    }
}
