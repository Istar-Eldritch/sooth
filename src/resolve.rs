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
        if scope.contains(name) {
            return Ok(None);
        }
        if let Some((qualifier, rest)) = name.split_once("::") {
            let target = match imports.get(qualifier) {
                Some(&t) => t,
                None => return Ok(None),
            };
            let (type_part, suffix) = split_accessor(rest);
            if self.types[target as usize].contains(type_part) {
                if !is_exported(&exports[target as usize], type_part) {
                    return Err(not_exported_error(type_part, qualifier, span));
                }
                return Ok(Some(format!("{}{}", mangle(type_part, target), suffix)));
            }
            if suffix.is_empty() && self.words[target as usize].contains(rest) {
                if !is_exported(&exports[target as usize], rest) {
                    return Err(not_exported_error(rest, qualifier, span));
                }
                return Ok(Some(mangle(rest, target)));
            }
            return Ok(None);
        }
        let (type_part, suffix) = split_accessor(name);
        if self.types[module as usize].contains(type_part) {
            return Ok(Some(format!("{}{}", mangle(type_part, module), suffix)));
        }
        if suffix.is_empty() && self.words[module as usize].contains(name) {
            return Ok(Some(mangle(name, module)));
        }
        // R20/R15c: own module first, then a selectively imported name. The
        // map (validated by `check::check_selective_imports`) exposes a bare
        // `Type` together with its generated words as one unit, so a
        // `Type>field` call whose `type_part` is selectively imported rewrites
        // against the target module just like a plain word does.
        if let Some(&target) = selective.get(type_part) {
            if self.types[target as usize].contains(type_part) {
                return Ok(Some(format!("{}{}", mangle(type_part, target), suffix)));
            }
        }
        if suffix.is_empty() {
            if let Some(&target) = selective.get(name) {
                if self.words[target as usize].contains(name) {
                    return Ok(Some(mangle(name, target)));
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
/// match. A no-op below two modules, so a single-file program is unchanged
/// (R22).
pub fn resolve_modules(module: &mut Module) -> Result<(), String> {
    if module.modules.len() < 2 {
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
    for w in &mut module.words {
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
            TermKind::IntLit(_)
            | TermKind::FloatLit(_)
            | TermKind::BoolLit(_)
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
        let span = Span { line: 1, col: 1 };

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
            vec![("Point".to_string(), Span { line: 1, col: 1 })],
        ];
        let scope = HashSet::new();
        let span = Span { line: 2, col: 3 };

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
        resolve_modules(&mut module).unwrap();
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

    #[test]
    fn single_module_closure_is_left_unchanged() {
        let tokens = lex(": p ( -- i64 ) 1 ; : main ( -- ) p drop ;").unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        resolve_modules(&mut module).unwrap();
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
