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

use crate::ast::{Module, Span, Term, TermKind};
use std::collections::{BTreeMap, HashMap, HashSet};

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
///
/// P8 S2 (R3): these two are the whole exemption list. The third one covered a
/// prelude the compiler injected into every closure, and went with it: `if` and
/// the comparisons are ordinary `core` words now, mangled per module and
/// reached by `import:` like any other name.
pub(crate) fn mangle(name: &str, module: u32) -> String {
    if name == "main" || name == "drop" {
        return name.to_string();
    }
    format!("{name}__m{module}")
}

/// R2: a static's own mangle, with no exemptions. Every exemption `mangle`
/// makes exists so a *word* stays reachable by bare name from a module that
/// did not declare it (`main` from the C shim, a `drop` overload from
/// `find_drop_overloads`). A static is
/// reachable no such way: only `&NAME` inside its declaring module names one,
/// and that site mangles identically. Routing statics through `mangle` instead
/// gave `static: drop` and `static: main` a raw data symbol, so two modules
/// declaring one leaked a bare "symbol already defined" out of the assembler,
/// and `Ctx::static_type`, which matches on the mangled name alone, silently
/// borrowed whichever of the two it found first.
pub(crate) fn mangle_static(name: &str, module: u32) -> String {
    format!("{name}__m{module}")
}

/// `check::builtin_table`'s keys, mirrored by hand (that table's own
/// `check_operator::is_operator` list carries the same warning): the
/// arithmetic/comparison/`max`/`max-total`/`.` names a bare call must reach
/// `check_operator`'s operand-type dispatch for, never a static rewrite. Keep
/// in sync when a table operator is added.
///
/// P8 S2 (R3a): the six comparison *surface* names are deliberately absent.
/// They are not `BUILTIN_TABLE` keys (their rows moved to the `u`-prefixed
/// primitives listed below in slice 10c), `check_operator::is_operator` does
/// not list them, and `scoped_operator_overloads` early-returns for any name
/// the table has no row for -- so no operator-overload dispatch path ever
/// reached them through here. Listing them bought only the coincidence that a
/// bare *call* stayed unrewritten to match a prelude *declaration* that was
/// also unmangled; with the prelude deleted the decl mangles, so the call must
/// mangle with it or resolve to nothing.
fn is_operator_dispatch_name(name: &str) -> bool {
    matches!(
        name,
        "add"
            | "sub"
            | "mul"
            | "div"
            | "mod"
            | "and"
            | "or"
            | "xor"
            | "not"
            | "shl"
            | "shr"
            | "ueq"
            | "ult"
            | "ugt"
            | "ulte"
            | "ugte"
            | "une"
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
    /// the closure's `Visibility` tables, and `span` for a located diagnostic.
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
        imports: &HashMap<String, u32>,
        selective: &HashMap<String, u32>,
        scope: &HashSet<String>,
        vis: &Visibility,
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
                if !is_exported(&vis.exports[target as usize], type_part) {
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
                if !is_exported(&vis.exports[target as usize], rest) {
                    return Err(not_exported_error(rest, qualifier, span));
                }
                return Ok(Some(format!("{sigil}{}", mangle(rest, target))));
            }
            // P8 S2 (R4): the target may promise a name it does not declare.
            // `export:` accepts an imported name as readily as a declared one,
            // so a hub's export list is resolved to the module that actually
            // declares each name (`Visibility::origin`) and the call mangles
            // against *that* module. No separate export gate: an
            // `exported_origin` entry exists only for a name on the target's
            // own `export:` list, so the entry is the promise.
            if word_ok {
                if let Some(origin) = vis.origin(target, type_part) {
                    if self.names_a_type(origin, type_part, suffix) {
                        return Ok(Some(format!(
                            "{sigil}{}{}",
                            mangle(type_part, origin),
                            suffix
                        )));
                    }
                }
                if let Some(origin) = vis.origin(target, rest) {
                    if self.words[origin as usize].contains(rest) {
                        return Ok(Some(format!("{sigil}{}", mangle(rest, origin))));
                    }
                }
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
        // summing two `i64` fields with `add`, say). Left bare, `check_operator`
        // still runs its own per-call-site, operand-type-directed dispatch
        // between the builtin and the overload, reaching the decl through
        // `scoped_operator_overloads`' mangled-key lookup rather than by name.
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
                // R4: the selective (or wildcard-desugared) entry may name a
                // type the target re-exports rather than declares.
                if let Some(origin) = vis.origin(target, type_part) {
                    if self.names_a_type(origin, type_part, suffix) {
                        return Ok(Some(format!(
                            "{sigil}{}{}",
                            mangle(type_part, origin),
                            suffix
                        )));
                    }
                }
            }
        }
        // A selectively imported operator name stays unrewritten here for the
        // same reason the own-module branch above does: mangling it would force
        // *every* bare use in the importing module onto the imported overload's
        // signature, including ones plainly meant for the builtin (a plain
        // `1 2 add` in a module that selectively imports `Vec2 add`). Left bare,
        // `scoped_operator_overloads` (R12, `check/word_families.rs`) assembles
        // the imported overload as a candidate alongside the builtin and any own
        // overload, so `check_operator`'s per-call-site operand-type dispatch
        // still finds it -- this only defers the rewrite to that dispatch.
        if word_ok && !is_operator_dispatch_name(core) {
            if let Some(&target) = selective.get(core) {
                if self.words[target as usize].contains(core) {
                    return Ok(Some(format!("{sigil}{}", mangle(core, target))));
                }
                // R4: `import: hub | lw | ;` (or `import: hub * ;`) where the
                // hub re-exports `lw` -- the bare name resolves to the origin
                // module's decl, which is the whole point of a hub.
                if let Some(origin) = vis.origin(target, core) {
                    if self.words[origin as usize].contains(core) {
                        return Ok(Some(format!("{sigil}{}", mangle(core, origin))));
                    }
                }
            }
        }
        Ok(None)
    }
}

/// P8 S2 (R4): the cross-module visibility tables `rewrite` consults -- every
/// module's `export:` list, plus each exported name's *origin* module (the one
/// that declares it, which for a re-exported name is not the exporting module).
/// Bundled into one parameter because `rewrite` and `rewrite_terms` already sit
/// at clippy's argument ceiling.
struct Visibility {
    exports: Vec<Vec<(String, Span)>>,
    /// Per module: each name on its `export:` list -> the module id that
    /// declares it. A module that declares what it exports maps the name to
    /// itself, so a present entry never implies a re-export; an absent one
    /// means the name is not exported at all.
    exported_origin: Vec<HashMap<String, u32>>,
}

impl Visibility {
    /// The module that declares `name`, if `target` exports it and is not
    /// itself the declarer. `None` for a name `target` does not export, and
    /// for one it declares itself (the caller's own branches, which run
    /// first, already resolve that case).
    fn origin(&self, target: u32, name: &str) -> Option<u32> {
        let origin = *self.exported_origin[target as usize].get(name)?;
        (origin != target).then_some(origin)
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

/// P8 S2 (R5/R6c): an `export:` name that is neither declared by the exporting
/// module nor imported into it, so it has no origin to re-export. Until this
/// slice `export:` did no existence check at all and such a name built clean.
/// The exporting module is identified by the located `export:` site rather than
/// by name: a module carries no name in the resolved closure (only importers
/// spell one, as a qualifier).
fn export_unknown_name_error(name: &str, span: Span) -> String {
    format!(
        "error: `{name}` in `export:` names nothing declared or imported in this module (line {}, col {})",
        span.line, span.col
    )
}

/// P8 S2 (R4/R6c): a re-export chain that revisits a `(module, name)` pair
/// before reaching a declaration. The one way the origin walk can fail to
/// converge, so it is rejected rather than looped on.
fn re_export_cycle_error(name: &str, span: Span) -> String {
    format!(
        "error: `{name}` re-exports itself through a cycle of `export:` chains (line {}, col {})",
        span.line, span.col
    )
}

/// P8 S2 (R4/R6c): a bare `export:` name declared by two or more of the
/// modules this one imports *qualified*, with no local decl and no
/// selective/wildcard entry to pick between them. `export:` is a flat name
/// list (there is no `export: dep1::lw ;`), so there is nothing to disambiguate
/// with and no defensible tiebreak: taking the first import would silently
/// privilege declaration order.
fn ambiguous_re_export_error(name: &str, origins: &[&str], span: Span) -> String {
    let origins = origins
        .iter()
        .map(|q| format!("`{q}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "error: `{name}` in `export:` is declared by more than one qualified-imported module ({origins}) and cannot be re-exported without disambiguation (line {}, col {})",
        span.line, span.col
    )
}

/// Every name a module may legitimately promise in `export:`: its words,
/// externs, structs, enums, generic `type:` headers, and the variant
/// constructors of both kinds of enum. Wider than `NameTables`, which holds
/// only what a *call site* can be mangled against: a generic header and a
/// variant constructor are resolved by other machinery (monomorphization, the
/// variant registry) and never mangled here, but `lib/result.sth` exports all
/// three shapes, so an existence check keyed on the mangling tables alone would
/// reject them. `static:` declarations are deliberately absent: a static is
/// module-private and reachable only by `&NAME` inside its own module, so
/// naming one in `export:` promises something no importer could ever reach.
fn exportable_names(module: &Module, m: u32) -> HashSet<&str> {
    let mut names = HashSet::new();
    for w in module.words.iter().filter(|w| w.module == m) {
        names.insert(w.name.as_str());
    }
    for x in module.externs.iter().filter(|x| x.module == m) {
        names.insert(x.name.as_str());
    }
    for s in module.structs.iter().filter(|s| s.module == m) {
        names.insert(s.name.as_str());
    }
    for s in module.generic_structs.iter().filter(|s| s.module == m) {
        names.insert(s.name.as_str());
    }
    for e in module.enums.iter().filter(|e| e.module == m) {
        names.insert(e.name.as_str());
        names.extend(e.variants.iter().map(|v| v.name.as_str()));
    }
    for e in module.generic_enums.iter().filter(|e| e.module == m) {
        names.insert(e.name.as_str());
        names.extend(e.variants.iter().map(|v| v.name.as_str()));
    }
    // P7.S3e (R1/decision 4): a `trait:` declaration is exportable exactly
    // like `type:`/`extern:` -- without this, `export: TraitName ;` fails
    // with a no-origin error before reaching `not_exported_error` downstream.
    for t in module.traits.iter().filter(|t| t.module == m) {
        names.insert(t.name.as_str());
    }
    names
}

/// P8 S2 (R4/R5): resolve every module's `export:` list to a name -> origin
/// module map. A name the exporting module declares maps to itself; a name it
/// imported maps to the module that declares it, following re-export chains
/// (hub of hubs) to their end.
fn build_exported_origin(
    module: &Module,
    exports: &[Vec<(String, Span)>],
    import_maps: &[HashMap<String, u32>],
    selectives: &[HashMap<String, u32>],
) -> Result<Vec<HashMap<String, u32>>, String> {
    let declared: Vec<HashSet<&str>> = (0..exports.len())
        .map(|m| exportable_names(module, m as u32))
        .collect();
    resolve_export_origins(&declared, exports, import_maps, selectives)
}

/// `build_exported_origin` over already-collected declaration sets: the whole
/// origin resolution, independent of how a `Module` spells its decls.
fn resolve_export_origins(
    declared: &[HashSet<&str>],
    exports: &[Vec<(String, Span)>],
    import_maps: &[HashMap<String, u32>],
    selectives: &[HashMap<String, u32>],
) -> Result<Vec<HashMap<String, u32>>, String> {
    let n = exports.len();
    // The *immediate* source of each exported name: one hop, which for a
    // re-exported name may itself be a re-export. Resolved to the declaring
    // module below, once every module's hop is known -- an export list may
    // name a hub whose own list has not been walked yet.
    let mut immediate: Vec<HashMap<String, u32>> = vec![HashMap::new(); n];
    for (m, list) in exports.iter().enumerate() {
        for (name, span) in list {
            let source = if declared[m].contains(name.as_str()) {
                m as u32
            } else if let Some(&target) = selectives[m].get(name) {
                target
            } else {
                // A dependency imported *qualified only* is reachable as
                // `dep::name` and appears in neither table above, so its own
                // declarations are scanned: `import_maps` is keyed by
                // qualifier, never by word name, so looking `name` up in it
                // would always miss. Keyed by module id so two qualifiers
                // bound to one module are one origin, not an ambiguity.
                let mut origins: BTreeMap<u32, &str> = BTreeMap::new();
                for (qualifier, dep) in import_maps[m]
                    .iter()
                    .filter(|(_, dep)| declared[**dep as usize].contains(name.as_str()))
                {
                    // One module bound under two qualifiers is still one
                    // origin; the lower spelling wins so the diagnostic does
                    // not depend on hash order.
                    let shown = origins.entry(*dep).or_insert(qualifier);
                    *shown = (*shown).min(qualifier.as_str());
                }
                let mut ids = origins.keys();
                match (ids.next(), ids.next()) {
                    (None, _) => return Err(export_unknown_name_error(name, *span)),
                    (Some(&dep), None) => dep,
                    _ => {
                        let qualifiers: Vec<&str> = origins.values().copied().collect();
                        return Err(ambiguous_re_export_error(name, &qualifiers, *span));
                    }
                }
            };
            immediate[m].insert(name.clone(), source);
        }
    }

    let mut exported_origin: Vec<HashMap<String, u32>> = vec![HashMap::new(); n];
    for (m, list) in exports.iter().enumerate() {
        for (name, span) in list {
            let origin = walk_to_origin(m as u32, name, *span, &immediate)?;
            exported_origin[m].insert(name.clone(), origin);
        }
    }
    Ok(exported_origin)
}

/// Follow one export entry's chain of immediate sources to the module that
/// declares the name. The visited set belongs to this one resolution and is
/// keyed on the `(module, name)` pair: a hub re-exporting two names that both
/// route through one downstream hub is a diamond, not a cycle, so a set shared
/// across resolutions -- or keyed on the module alone -- would reject the
/// second name.
fn walk_to_origin(
    module: u32,
    name: &str,
    span: Span,
    immediate: &[HashMap<String, u32>],
) -> Result<u32, String> {
    let mut visited: HashSet<(u32, &str)> = HashSet::new();
    let mut current = module;
    loop {
        if !visited.insert((current, name)) {
            return Err(re_export_cycle_error(name, span));
        }
        match immediate[current as usize].get(name) {
            // A hop that leaves the name's own export list ends the chain:
            // the source declares it (it is not re-exporting it onward).
            Some(&next) if next != current => current = next,
            _ => return Ok(current),
        }
    }
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
    let import_maps: Vec<HashMap<String, u32>> =
        module.modules.iter().map(|m| m.imports.clone()).collect();
    let selectives: Vec<HashMap<String, u32>> =
        module.modules.iter().map(|m| m.selective.clone()).collect();
    let exports: Vec<Vec<(String, Span)>> =
        module.modules.iter().map(|m| m.exports.clone()).collect();
    // P8 S2 (R4/R5): resolved before any body is rewritten, since a body may
    // reference a name through a hub whose own export list has not been
    // reached yet. This is also where an `export:` name with no origin, and an
    // ambiguous or cyclic re-export, are rejected.
    let vis = Visibility {
        exported_origin: build_exported_origin(module, &exports, &import_maps, &selectives)?,
        exports,
    };
    for word in &mut module.words {
        let imports = &import_maps[word.module as usize];
        let selective = &selectives[word.module as usize];
        let mut scope = HashSet::new();
        let terms = &mut word.body;
        rewrite_terms(
            terms,
            word.module,
            imports,
            selective,
            &tables,
            &mut scope,
            &vis,
        )?;
    }

    for s in &mut module.structs {
        s.name = mangle(&s.name, s.module);
    }
    for e in &mut module.enums {
        e.name = mangle(&e.name, e.module);
    }
    // An operator decl is mangled like any other, in a single-module closure as
    // much as a multi-module one. The bare *call* the rewrite above leaves
    // unmangled still finds it: `scoped_operator_overloads` assembles the
    // caller-visible candidates under `mangle(name, m)` rather than the bare
    // name, so `check_operator`'s operand-type dispatch sees the overload
    // without the decl having to own the bare symbol. Owning it is not
    // harmless: the operators-as-words rename made these names alphabetic, and
    // a bare `div` decl is a strong definition of libc's `div` in the
    // executable, interposing it for every shared library linked in.
    for w in &mut module.words {
        w.name = mangle(&w.name, w.module);
    }
    for x in &mut module.externs {
        x.name = mangle(&x.name, x.module);
    }
    // R2: a static's data symbol is module-scoped exactly as a word's is, so
    // two modules may each declare `COUNT` without colliding at codegen --
    // and, via `mangle_static`, without colliding on `drop`/`main` either.
    // No operator-name carve-out applies: a static is never a
    // dispatch name.
    for s in &mut module.statics {
        s.name = mangle_static(&s.name, s.module);
    }
    Ok(())
}

/// Rewrite a block of terms left to right. A `Bind` extends the scope for the
/// rest of this block (and any nested block that follows); the added names are
/// removed when the block ends, so a sibling block does not see them.
#[allow(clippy::too_many_arguments)]
fn rewrite_terms(
    terms: &mut [Term],
    module: u32,
    imports: &HashMap<String, u32>,
    selective: &HashMap<String, u32>,
    tables: &NameTables,
    scope: &mut HashSet<String>,
    vis: &Visibility,
) -> Result<(), String> {
    let base = scope.len();
    let mut added: Vec<String> = Vec::new();
    for term in terms.iter_mut() {
        match &mut term.kind {
            TermKind::Call(name) => {
                if let Some(new) =
                    tables.rewrite(name, module, imports, selective, scope, vis, term.span)?
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
                rewrite_terms(inner, module, imports, selective, tables, scope, vis)?;
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
        for name in ["main", "drop"] {
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
        let vis = declaring_visibility(vec![Vec::new(), Vec::new()]); // module 1 exports nothing
        let scope = HashSet::new();
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };

        let no_selective = std::collections::HashMap::new();
        let unexported = tables.rewrite("q::grow", 0, &imports, &no_selective, &scope, &vis, span);
        assert!(
            matches!(unexported, Err(ref e) if e.contains("not exported")),
            "existing-but-private name is a located error: {unexported:?}"
        );

        let absent = tables.rewrite("q::missing", 0, &imports, &no_selective, &scope, &vis, span);
        assert_eq!(absent, Ok(None), "absent name defers to unknown-word");

        // An enum is a type for an *unsuffixed* reference too, which is what
        // makes a qualified one answer R16 here rather than falling through to
        // unknown-word. (`?` eligibility is the enum set's other reader, see
        // `names_a_type`.)
        let private_enum =
            tables.rewrite("q::Hidden", 0, &imports, &no_selective, &scope, &vis, span);
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
        let vis = declaring_visibility(vec![
            Vec::new(),
            vec![(
                "Point".to_string(),
                Span {
                    line: 1,
                    col: 1,
                    module: 0,
                },
            )],
        ]);
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
            let result = tables.rewrite(spelling, 0, &imports, &no_selective, &scope, &vis, span);
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

    /// P8 S2 (R3a): the six surface comparisons left `is_operator_dispatch_name`
    /// with the prelude, so a bare call to one now *mangles* like any other word
    /// -- which is the only way it can match its own, now-mangled declaration.
    /// A real table operator (`add`) still stays bare for `check_operator`'s
    /// operand dispatch, so this pins the split rather than "nothing is
    /// carved out any more".
    #[test]
    fn a_comparison_call_mangles_while_a_table_operator_stays_bare() {
        let mut words = vec![HashSet::new(), HashSet::new()];
        for name in ["lt", "add"] {
            words[0].insert(name.to_string());
            words[1].insert(name.to_string());
        }
        let tables = NameTables {
            structs: vec![HashSet::new(), HashSet::new()],
            enums: vec![HashSet::new(), HashSet::new()],
            words,
            statics: vec![HashSet::new(), HashSet::new()],
        };
        let imports = std::collections::HashMap::new();
        let selective: HashMap<String, u32> =
            [("lt".to_string(), 1u32), ("add".to_string(), 1u32)].into();
        let vis = declaring_visibility(vec![
            Vec::new(),
            vec![
                ("lt".to_string(), Span::default()),
                ("add".to_string(), Span::default()),
            ],
        ]);
        let scope = HashSet::new();
        let span = Span::default();
        let rewrite = |name: &str| {
            tables
                .rewrite(name, 0, &imports, &selective, &scope, &vis, span)
                .unwrap()
        };
        assert_eq!(
            rewrite("lt"),
            Some("lt__m0".to_string()),
            "a bare comparison resolves own-module-first and mangles"
        );
        assert_eq!(
            rewrite("add"),
            None,
            "a table operator is still left bare for operand dispatch"
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
        let vis = declaring_visibility(vec![Vec::new(), Vec::new()]);
        let scope = HashSet::new();
        let span = Span::default();
        let rewrite = |name: &str, module: u32| {
            tables
                .rewrite(name, module, &imports, &selective, &scope, &vis, span)
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
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( Rect ) Rect> mul ] Shape? ;\n\
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
        let vis =
            declaring_visibility(vec![Vec::new(), vec![("ok?".to_string(), Span::default())]]);
        let scope = HashSet::new();
        let span = Span::default();
        let none = std::collections::HashMap::new();
        let mut selective = std::collections::HashMap::new();
        selective.insert("ok?".to_string(), 1u32);
        let at = |name: &str, sel: &std::collections::HashMap<String, u32>| {
            tables.rewrite(name, 0, &imports, sel, &scope, &vis, span)
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
             : P? ( i64 -- i64 ) 1 add ;\n\
             : E> ( i64 -- i64 ) 1 add ;\n\
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

    /// An operator-named decl carries no exemption from mangling. It used to,
    /// in a single-module closure, so `check_operator`'s candidate scan could
    /// find it under the bare name -- which handed the word the bare symbol.
    /// Harmless while every dispatch name was symbolic (`qbe_name` escapes `+`
    /// to `.2b.`), but the operators-as-words rename spells four of them
    /// `add`/`sub`/`mul`/`div`, and `div` is a libc function: the decl became a
    /// strong definition of `div` in the executable, interposing libc's for
    /// every shared library linked in. The call site stays bare (asserted here
    /// too, since that is what the operand dispatch keys on); only the decl
    /// moves.
    #[test]
    fn operator_named_decl_mangles_in_a_forced_single_module_build() {
        let tokens = lex("type: V x f64 ;\n\
             : div ( V V -- V ) drop ;\n\
             : main ( -- ) 1.0 V 2.0 V div &x @ swap drop . 9.0 3.0 div . ;\n")
        .unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        resolve_modules(&mut module, true).unwrap();
        let names: Vec<&str> = module.words.iter().map(|w| w.name.as_str()).collect();
        assert!(
            names.contains(&"div__m0") && !names.contains(&"div"),
            "the `div` overload owns a module-scoped symbol, not libc's: {names:?}"
        );
        let main = module.words.iter().find(|w| w.name == "main").unwrap();
        assert!(
            call_names(&main.body)
                .iter()
                .filter(|n| *n == "div")
                .count()
                == 2,
            "both call sites stay bare for `check_operator`'s operand dispatch: {:?}",
            call_names(&main.body)
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

    /// P8 S2 (R4): a hub-of-hubs chain resolves to the module that actually
    /// declares the name -- module 2 declares `lw`, module 1 re-exports it,
    /// module 0 re-exports module 1's re-export -- so a consumer of any link
    /// in the chain mangles against module 2.
    #[test]
    fn export_origin_follows_a_re_export_chain_to_the_declaring_module() {
        let declared = vec![HashSet::new(), HashSet::new(), HashSet::from(["lw"])];
        let origins = resolve_export_origins(
            &declared,
            &[exports(&["lw"]), exports(&["lw"]), exports(&["lw"])],
            &[HashMap::new(), HashMap::new(), HashMap::new()],
            &[selective("lw", 1), selective("lw", 2), HashMap::new()],
        )
        .expect("a two-hop re-export chain resolves");
        assert_eq!(origins[0]["lw"], 2, "the hub of hubs reaches the declarer");
        assert_eq!(origins[1]["lw"], 2);
        assert_eq!(origins[2]["lw"], 2, "the declarer exports its own name");
    }

    /// R4: the visited set is per resolution and keyed on the `(module, name)`
    /// pair. Two names re-exported by one hub and both routed through the same
    /// downstream hub is a diamond, not a cycle: one visited set shared across
    /// resolutions and keyed on the module alone rejects the second name here.
    #[test]
    fn export_origin_accepts_two_names_routed_through_one_hub() {
        let declared = vec![HashSet::new(), HashSet::new(), HashSet::from(["a", "b"])];
        let mut hub = selective("a", 2);
        hub.insert("b".to_string(), 2);
        let mut top = selective("a", 1);
        top.insert("b".to_string(), 1);
        let both = exports(&["a", "b"]);
        let origins = resolve_export_origins(
            &declared,
            &[both.clone(), both.clone(), both],
            &[HashMap::new(), HashMap::new(), HashMap::new()],
            &[top, hub, HashMap::new()],
        )
        .expect("two names through one hub is a diamond, not a cycle");
        assert_eq!((origins[0]["a"], origins[0]["b"]), (2, 2));
    }

    /// R4/R6c: two modules re-exporting each other's `lw` is a located error,
    /// and the walk terminates rather than looping (this test would hang if it
    /// did not).
    #[test]
    fn export_re_export_cycle_is_a_located_error() {
        let err = resolve_export_origins(
            &[HashSet::new(), HashSet::new()],
            &[vec![("lw".to_string(), at(4, 1))], exports(&["lw"])],
            &[HashMap::new(), HashMap::new()],
            &[selective("lw", 1), selective("lw", 0)],
        )
        .expect_err("a re-export cycle never reaches a declaration");
        assert_eq!(
            err,
            "error: `lw` re-exports itself through a cycle of `export:` chains (line 4, col 1)"
        );
    }

    /// R5/R6c: an `export:` name that is neither declared locally nor imported
    /// has no origin. Silently accepted before this slice.
    #[test]
    fn export_of_an_undeclared_unimported_name_is_a_located_error() {
        let err = resolve_export_origins(
            &[HashSet::from(["other"])],
            &[vec![("nonexistent".to_string(), at(2, 9))]],
            &[HashMap::new()],
            &[HashMap::new()],
        )
        .expect_err("an export naming nothing is an error");
        assert_eq!(
            err,
            "error: `nonexistent` in `export:` names nothing declared or imported in this module (line 2, col 9)"
        );
    }

    /// R4/R6c: a bare `export:` name declared by two *qualified*-imported
    /// modules has no spelling that could pick between them, so it is a
    /// located error naming both -- never a first-import-wins pick. The
    /// qualifiers are listed in module-id order, so the message does not
    /// depend on the import map's hash order.
    #[test]
    fn export_name_declared_by_two_qualified_imports_is_ambiguous() {
        let declared = vec![HashSet::new(), HashSet::from(["lw"]), HashSet::from(["lw"])];
        let mut imports = HashMap::new();
        imports.insert("dep2".to_string(), 2u32);
        imports.insert("dep1".to_string(), 1u32);
        let err = resolve_export_origins(
            &declared,
            &[
                vec![("lw".to_string(), at(3, 1))],
                exports(&["lw"]),
                exports(&["lw"]),
            ],
            &[imports, HashMap::new(), HashMap::new()],
            &[HashMap::new(), HashMap::new(), HashMap::new()],
        )
        .expect_err("two qualified origins cannot be disambiguated");
        assert_eq!(
            err,
            "error: `lw` in `export:` is declared by more than one qualified-imported module (`dep1`, `dep2`) and cannot be re-exported without disambiguation (line 3, col 1)"
        );
    }

    /// R4: a hub that imports its dependency *qualified only* reaches the name
    /// as `dep::lw` alone -- neither a local decl nor a selective entry -- so
    /// the origin comes from scanning the qualified-imported modules' own
    /// declarations. Without that scan this is a spurious existence error.
    #[test]
    fn export_origin_scans_qualified_imported_declarations() {
        let declared = vec![HashSet::new(), HashSet::from(["lw"])];
        let mut imports = HashMap::new();
        imports.insert("dep".to_string(), 1u32);
        let origins = resolve_export_origins(
            &declared,
            &[exports(&["lw"]), exports(&["lw"])],
            &[imports, HashMap::new()],
            &[HashMap::new(), HashMap::new()],
        )
        .expect("a qualified-only re-export has an origin");
        assert_eq!(origins[0]["lw"], 1);
    }

    /// R4: with the origin table in hand, both of `rewrite`'s cross-module
    /// branches reach a re-exported name -- a qualified `hub::lw` and the bare
    /// `lw` a selective (or wildcard-desugared) import of the hub binds -- and
    /// both mangle against the *declaring* module, not the hub.
    #[test]
    fn rewrite_resolves_a_re_exported_name_to_its_origin_module() {
        let mut words = vec![HashSet::new(), HashSet::new(), HashSet::new()];
        words[2].insert("lw".to_string());
        let tables = NameTables {
            structs: vec![HashSet::new(); 3],
            enums: vec![HashSet::new(); 3],
            words,
            statics: vec![HashSet::new(); 3],
        };
        let mut imports = HashMap::new();
        imports.insert("hub".to_string(), 1u32);
        let vis = Visibility {
            exports: vec![Vec::new(), exports(&["lw"]), exports(&["lw"])],
            exported_origin: vec![
                HashMap::new(),
                HashMap::from([("lw".to_string(), 2u32)]),
                HashMap::from([("lw".to_string(), 2u32)]),
            ],
        };
        let scope = HashSet::new();
        let span = Span::default();
        assert_eq!(
            tables.rewrite("hub::lw", 0, &imports, &HashMap::new(), &scope, &vis, span),
            Ok(Some("lw__m2".to_string())),
            "qualified through the hub"
        );
        assert_eq!(
            tables.rewrite("lw", 0, &imports, &selective("lw", 1), &scope, &vis, span),
            Ok(Some("lw__m2".to_string())),
            "bare, through a selective or wildcard import of the hub"
        );
    }

    /// One `export:` list, every entry at the default span (the origin table
    /// is keyed on names; a span only surfaces in a diagnostic).
    fn exports(names: &[&str]) -> Vec<(String, Span)> {
        names
            .iter()
            .map(|n| (n.to_string(), Span::default()))
            .collect()
    }

    fn at(line: u32, col: u32) -> Span {
        Span {
            line,
            col,
            module: 0,
        }
    }

    /// A one-entry selective-import map: `name` resolves to module `target`.
    fn selective(name: &str, target: u32) -> HashMap<String, u32> {
        HashMap::from([(name.to_string(), target)])
    }

    /// The `Visibility` of a closure where every module declares what it
    /// exports: each name's origin is the module exporting it, so no lookup
    /// reaches a re-export. What every test predating P8 S2 assumed.
    fn declaring_visibility(exports: Vec<Vec<(String, Span)>>) -> Visibility {
        let exported_origin = exports
            .iter()
            .enumerate()
            .map(|(m, list)| {
                list.iter()
                    .map(|(name, _)| (name.clone(), m as u32))
                    .collect()
            })
            .collect();
        Visibility {
            exports,
            exported_origin,
        }
    }

    fn call_names(body: &[Term]) -> Vec<String> {
        let mut out = Vec::new();
        collect_calls(body, &mut out);
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
        let mut slices = Vec::new();
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
            &[],
            &mut arrays,
            &mut owned_cells,
            &mut refs,
            &mut slices,
            &mut generics,
            &[],
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
            &[],
            &mut arrays,
            &mut owned_cells,
            &mut refs,
            &mut slices,
            &mut generics,
            &[],
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
            slices: Vec::new(),
            generic_structs: Vec::new(),
            generic_enums: Vec::new(),
            generics: crate::ast::GenericTypes::default(),
            externs: Vec::new(),
            instantiations: HashMap::new(),
            builtin_overloads: HashMap::new(),
            resolved_fields: HashMap::new(),
            resolved_variant_fields: HashMap::new(),
            modules: vec![
                ModuleInfo {
                    imports: imports0,
                    exports: entry_bodies.exports,
                    ..ModuleInfo::default()
                },
                ModuleInfo {
                    imports: no_imports,
                    exports: lib_bodies.exports,
                    ..ModuleInfo::default()
                },
            ],
            statics,
            traits: Vec::new(),
            impls: Vec::new(),
        }
    }
}
