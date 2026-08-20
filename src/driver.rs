//! Pipeline orchestration: the one place that wires the stages together.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ast::{Import, Module, ModuleInfo, Span};
use crate::lexer::Token;
use crate::packages::{ManifestCache, ResolutionConfig, UnresolvedImport};
use crate::{backend, check, ir, lexer, packages, parser, resolve};

const C_SHIM: &str = "extern void sooth_main(void);\nint main(void) { sooth_main(); return 0; }\n";

/// The native binary path for a source file: alongside the source, named after its
/// stem (`examples/gcd.sth` -> `examples/gcd`).
fn binary_path(source: &Path) -> PathBuf {
    source.with_extension("")
}

/// One file in the import closure (R2). `import_targets` is aligned with
/// `imports`: the module id the qualifier at that position binds to, or `None`
/// for an import that adds no closure edge (the reserved `intrinsics` name, and
/// a cross-package import recorded as unresolved).
struct FileNode {
    canon: PathBuf,
    dir: PathBuf,
    tokens: Vec<(Token, Span)>,
    imports: Vec<Import>,
    import_targets: Vec<Option<u32>>,
}

/// The whole import closure, one `FileNode` per file, indexed by module id
/// (entry is id 0, R10).
pub(crate) struct Closure {
    nodes: Vec<FileNode>,
    /// Cross-package imports resolution declined to turn into an edge, audited
    /// by `packages::check_package_graph` against the `depends:` and
    /// `module:` tables before `discover_closure` returns.
    unresolved_imports: Vec<UnresolvedImport>,
}

/// Read, lex, and scan one file's imports into a `FileNode` (R2). The path is
/// already canonical, so its parent is a stable directory to resolve this
/// file's own relative imports against. `module` is this file's permanent id
/// in the closure being built (the caller already knows it, since `id_of`
/// assigns it before calling); every token's span is stamped with it here, so
/// two files' tokens landing on the identical (line, col) by coincidence
/// never collide once merged into one `Module` (`Span`'s doc comment).
fn make_node(canon: PathBuf, module: u32) -> Result<FileNode, String> {
    let src = std::fs::read_to_string(&canon).map_err(|e| format!("reading {canon:?}: {e}"))?;
    let mut tokens = lexer::lex(&src)?;
    for (_, span) in &mut tokens {
        span.module = module;
    }
    let imports = parser::scan_imports(&tokens)?;
    let dir = canon.parent().unwrap_or(Path::new(".")).to_path_buf();
    Ok(FileNode {
        canon,
        dir,
        tokens,
        imports,
        import_targets: Vec::new(),
    })
}

impl ResolutionConfig {
    /// `manifest_override` empty (the flag is threaded in by the caller);
    /// `user_manifest` populated from `$XDG_CONFIG_HOME/sooth/global_sooth.pkg`,
    /// falling back to `$HOME/.config/sooth/global_sooth.pkg` (R6), but only
    /// when that file actually exists — a missing file is tier 4, not tier 3.
    /// Kept here rather than beside the struct in `packages.rs`: reading the
    /// process environment is `driver.rs`'s concern, not `packages.rs`'s
    /// (CLAUDE.md growth structure).
    pub(crate) fn from_env() -> Self {
        ResolutionConfig {
            manifest_override: None,
            user_manifest: user_manifest_path(
                std::env::var_os("XDG_CONFIG_HOME"),
                std::env::var_os("HOME"),
            )
            .filter(|p| p.is_file()),
        }
    }
}

/// Where the user-level manifest lives (tier 3, R6), as a function of the two
/// environment values that decide it rather than of `std::env` itself: the
/// branches are then testable without mutating process-wide state, which R6
/// forbids. An empty `XDG_CONFIG_HOME` counts as unset (the XDG spec).
fn user_manifest_path(config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let config_home = config_home
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".config")))?;
    Some(config_home.join("sooth").join("global_sooth.pkg"))
}

/// R2/R3: discover the whole import closure from the entry file. A quoted-path
/// import (manifest-less files only) resolves relative to the importing file's
/// directory; a module name resolves against its package, by its anchor. Both
/// canonicalize and dedupe by canonical path (a diamond imports a file once).
/// Rejects a cycle and a self-import with a located both-files error (R4) and a
/// missing file with a located error (R5).
pub(crate) fn discover_closure(entry: &Path) -> Result<Closure, String> {
    discover_closure_audited(entry, &ResolutionConfig::from_env())
}

/// `discover_closure_configured` over a fresh `ManifestCache`, followed by the
/// package-graph audit — the shared body of `discover_closure` and
/// `emit_ssa_with_manifest`, so the two entry points can't drift apart.
fn discover_closure_audited(entry: &Path, config: &ResolutionConfig) -> Result<Closure, String> {
    let mut manifests = ManifestCache::default();
    let closure = discover_closure_configured(entry, config, &mut manifests)?;
    packages::check_package_graph(&mut manifests, &closure.unresolved_imports)?;
    Ok(closure)
}

/// `discover_closure` over a caller-supplied manifest cache and resolution
/// config, so a test can see how many manifests the walk actually parsed and
/// point the fallback tiers at a fixture (R6).
pub(crate) fn discover_closure_configured(
    entry: &Path,
    config: &ResolutionConfig,
    manifests: &mut ManifestCache,
) -> Result<Closure, String> {
    let entry_canon =
        std::fs::canonicalize(entry).map_err(|e| format!("reading {}: {e}", entry.display()))?;
    let mut id_of: HashMap<PathBuf, u32> = HashMap::new();
    id_of.insert(entry_canon.clone(), 0);
    let mut nodes: Vec<FileNode> = vec![make_node(entry_canon.clone(), 0)?];
    let mut unresolved_imports = Vec::new();

    let mut i = 0;
    while i < nodes.len() {
        let dir = nodes[i].dir.clone();
        let canon = nodes[i].canon.clone();
        let imports = nodes[i].imports.clone();
        // Every file's own site, from its canonical path: the `--manifest`
        // override for the entry file (R3), else the fallback chain from its
        // nearest ancestor manifest down (R1). Manifests are parsed once each
        // however many files they own.
        let site = packages::select_site(&canon, &entry_canon, config, manifests)?;
        let mut targets = Vec::with_capacity(imports.len());
        for imp in &imports {
            let resolved = packages::resolve_import(
                &canon,
                &dir,
                imp,
                site.as_ref(),
                manifests,
                &mut unresolved_imports,
            )?;
            let Some(file) = resolved else {
                targets.push(None);
                continue;
            };
            let id = match id_of.get(&file) {
                Some(&id) => id,
                None => {
                    let id = nodes.len() as u32;
                    id_of.insert(file.clone(), id);
                    nodes.push(make_node(file, id)?);
                    id
                }
            };
            targets.push(Some(id));
        }
        nodes[i].import_targets = targets;
        i += 1;
    }

    let closure = Closure {
        nodes,
        unresolved_imports,
    };
    closure.reject_cycles()?;
    Ok(closure)
}

impl Closure {
    /// R14 (slice 5b): the canonical file path a module id in an assembled
    /// closure came from, for a located error naming the file (e.g. an
    /// imported `main`).
    pub(crate) fn path_of(&self, module: u32) -> &Path {
        &self.nodes[module as usize].canon
    }

    /// R4: reject any import cycle (a self-import is the degenerate case), with
    /// a located error naming both files at the edge that closes the cycle.
    fn reject_cycles(&self) -> Result<(), String> {
        let n = self.nodes.len();
        // 0 = unvisited, 1 = on the current DFS stack, 2 = done.
        let mut state = vec![0u8; n];
        for start in 0..n {
            if state[start] == 0 {
                self.dfs_detect(start, &mut state)?;
            }
        }
        Ok(())
    }

    fn dfs_detect(&self, u: usize, state: &mut [u8]) -> Result<(), String> {
        state[u] = 1;
        for &v in self.nodes[u].import_targets.iter().flatten() {
            let v = v as usize;
            if state[v] == 1 {
                let importer = self.nodes[u].canon.display();
                let target = self.nodes[v].canon.display();
                return Err(if u == v {
                    format!("error: import cycle: `{importer}` imports itself")
                } else {
                    format!(
                        "error: import cycle: `{importer}` imports `{target}`, which (directly or transitively) imports `{importer}`"
                    )
                });
            }
            if state[v] == 0 {
                self.dfs_detect(v, state)?;
            }
        }
        state[u] = 2;
        Ok(())
    }
}

/// P8 slice 1a: two imports in one file binding the same qualifier is a located
/// error at the second, naming where the first bound it. No shadowing and no
/// precedence: the second binding is always the error.
fn duplicate_qualifier_error(file: &Path, qualifier: &str, at: Span, first: Span) -> String {
    format!(
        "error: duplicate import qualifier `{qualifier}` at line {}, col {} in {}:\n  qualifier `{qualifier}` was first bound at line {}, col {}",
        at.line,
        at.col,
        file.display(),
        first.line,
        first.col
    )
}

/// P8 slice 1a: a wildcard import binds no qualifier, and nothing here gives
/// it a visibility effect to splice in -- a compiled build rejects it
/// outright, exactly as the REPL does, rather than silently binding nothing.
fn wildcard_import_is_error(file: &Path, at: Span) -> String {
    format!(
        "error: wildcard import at line {}, col {} in {}:\n  a wildcard import binds no names in this build\n  use a qualified import instead",
        at.line,
        at.col,
        file.display()
    )
}

/// R3/R11: assemble the discovered closure into one `Module`. Runs the shared
/// type pre-pass across every file into one merged registry, parses each file's
/// bodies module-aware against that shared registry, then hands the merged
/// module to the resolver to mangle same-named decls apart.
///
/// `always_mangle` forces the resolver to mangle even a single-file closure: the
/// native build path sets it so a user word named like a libc symbol (`close`)
/// or a runtime shim's callee (`free`) cannot be emitted as that bare symbol and
/// hijack it at link time. The REPL import path leaves it unset, keeping the R22
/// single-file no-op (it renames imported words to epoch symbols itself).
pub(crate) fn assemble_module(closure: &Closure, always_mangle: bool) -> Result<Module, String> {
    let mut structs = Vec::new();
    // R2 (slice 9): the builtin `bool` enum occupies the reserved head of the
    // merged registry (`BOOL_ENUM_ID`) ahead of every file's user enums, so
    // each module's `enum_base` offset already accounts for it.
    let mut enums = vec![crate::ast::bool_enum_decl()];
    let mut struct_base = Vec::with_capacity(closure.nodes.len());
    let mut enum_base = Vec::with_capacity(closure.nodes.len());
    for (m, node) in closure.nodes.iter().enumerate() {
        struct_base.push(structs.len());
        enum_base.push(enums.len());
        parser::prepass_and_register(&node.tokens, m as u32, &mut structs, &mut enums)?;
    }

    // R14/R16: every module's `export:` list, scanned upfront (independent
    // of body-parse order, which is discovery order, not dependency order) so
    // an importer's effect can visibility-check a cross-module type even
    // before the exporting file's own body has parsed.
    let exports_by_module: Vec<Vec<(String, Span)>> = closure
        .nodes
        .iter()
        .map(|node| parser::scan_exports(&node.tokens))
        .collect::<Result<_, _>>()?;

    // Each module's qualifier -> target module map, and its unqualified
    // selective names -> target module map (R20/R15c). Built for the whole
    // closure upfront because both parse passes below need them; the selective
    // maps are unvalidated here, so body parsing can resolve a bare selective
    // `Type` and `check::check_selective_imports` rejects a private or
    // colliding name before any codegen (own-module-first resolution means a
    // collision shadows to the local decl at parse, never miscompiles).
    let mut import_by_module: Vec<HashMap<String, u32>> = Vec::with_capacity(closure.nodes.len());
    let mut selective_maps: Vec<HashMap<String, u32>> = Vec::with_capacity(closure.nodes.len());
    // R20/R21: every module's selective-import entries, kept with their source
    // qualifier and span for the post-assembly validation (`check::check_selective_imports`).
    let mut selective_by_module: Vec<Vec<check::SelectiveName>> =
        Vec::with_capacity(closure.nodes.len());
    for node in closure.nodes.iter() {
        let mut import_map: HashMap<String, u32> = HashMap::new();
        let mut selective_map: HashMap<String, u32> = HashMap::new();
        let mut selective_entries: Vec<check::SelectiveName> = Vec::new();
        // P8 slice 1a: where each qualifier was first bound, so a second
        // import binding the same one is a located error at that second import
        // rather than a silent shadow. Both an explicit qualifier and a
        // defaulted last segment bind concretely, so both collide.
        let mut bound_at: HashMap<&str, Span> = HashMap::new();
        for (imp, target) in node.imports.iter().zip(&node.import_targets) {
            let qualifier = match imp.qualifier() {
                Some(q) => q,
                // The reserved `intrinsics` wildcard (F6) is the one wildcard
                // shape that adds no closure edge and needs no qualifier; any
                // other wildcard binds no names and is rejected outright.
                None if target.is_none() => continue,
                None => return Err(wildcard_import_is_error(&node.canon, imp.span)),
            };
            if let Some(first) = bound_at.insert(qualifier, imp.span) {
                return Err(duplicate_qualifier_error(
                    &node.canon,
                    qualifier,
                    imp.span,
                    first,
                ));
            }
            // The reserved `intrinsics` name and an import recorded as
            // unresolved add no closure edge, so they bind no qualifier to a
            // module id here.
            let Some(&target) = target.as_ref() else {
                continue;
            };
            import_map.insert(qualifier.to_string(), target);
            for (name, span) in imp.selective() {
                selective_map.insert(name.clone(), target);
                selective_entries.push(check::SelectiveName {
                    name: name.clone(),
                    qualifier: qualifier.to_string(),
                    target,
                    span: *span,
                });
            }
        }
        import_by_module.push(import_map);
        selective_maps.push(selective_map);
        selective_by_module.push(selective_entries);
    }

    let mut arrays = Vec::new();
    let mut owned_cells = Vec::new();
    let mut refs = Vec::new();
    // Slice 9 phase 2 (R6): the library `.` overload for `bool`, injected
    // ahead of every file's own words exactly as `bool_enum_decl` injects
    // the enum it dispatches over. Slice 10c (R-P3-4) puts `if`/`unless` and
    // the six comparison words beside it, from `lib/core.sth`.
    let mut words = vec![crate::ast::bool_print_word_def()];
    let mut externs = Vec::new();
    let mut statics = Vec::new();
    // Phase 5 slice 1 (D5): shared across the closure, its instantiation ids
    // computed against the concrete registries as the pre-pass left them --
    // every file's `type:` names are registered above before any body parses,
    // so nothing lands between those entries and the appended instantiations.
    let mut generics = crate::ast::GenericTypes::with_bases(structs.len(), enums.len());
    // Phase 5 slice 2 (OQ1): every file's generic `type:` headers registered
    // across the whole closure before any body parses, the generic twin of
    // `prepass_and_register` above -- so a qualified application
    // (`q::Box[i64]`) resolves whether or not the declaring file has been
    // body-parsed yet. Shares the `arrays`/`owned_cells`/`refs` registries the
    // body loop below uses, keeping interned ids in sync across both passes,
    // and the same name environment, so a generic field naming an imported
    // concrete type resolves here exactly as it does in the body pass.
    for (m, node) in closure.nodes.iter().enumerate() {
        parser::prepass_generic_typedefs(
            &node.tokens,
            &structs,
            &enums,
            m as u32,
            &import_by_module[m],
            &exports_by_module,
            &selective_maps[m],
            &mut arrays,
            &mut owned_cells,
            &mut refs,
            &mut generics,
        )?;
    }
    let mut modules = Vec::with_capacity(closure.nodes.len());
    for (m, node) in closure.nodes.iter().enumerate() {
        let import_map = import_by_module[m].clone();
        let selective_map = &selective_maps[m];
        let bodies = parser::parse_bodies(
            &node.tokens,
            &structs,
            &enums,
            m as u32,
            &import_map,
            &exports_by_module,
            selective_map,
            &mut arrays,
            &mut owned_cells,
            &mut refs,
            &mut generics,
        )?;
        for (k, fields) in bodies.struct_fields_by_decl.into_iter().enumerate() {
            structs[struct_base[m] + k].fields = fields;
        }
        for (k, variant_fields) in bodies.enum_fields_by_decl.into_iter().enumerate() {
            for (vidx, fields) in variant_fields.into_iter().enumerate() {
                enums[enum_base[m] + k].variants[vidx].fields = fields;
            }
        }
        words.extend(bodies.words);
        externs.extend(bodies.externs);
        statics.extend(bodies.statics);
        modules.push(ModuleInfo {
            imports: import_map,
            exports: exports_by_module[m].clone(),
            selective: selective_map.clone(),
        });
    }

    // Slice 10c (R-P3-4): `lib/core.sth`'s words — `if`, `unless` and the six
    // comparison words — appended once for the whole closure, the multi-file
    // twin of `parser::parse`'s own injection.
    words.extend(parser::prelude_words());

    // R4/D5: the minted instantiations join the ordinary registries, after
    // every pre-pass entry their ids were computed against.
    //
    // P7 slice 3a phase 2 (R2): `generics` is flushed onto the live
    // registries, then *rebased* to their new length, rather than consumed
    // and dropped -- check and lowering keep it alive and mutable so a poly
    // word's own construction can mint a monomorph on demand. The naive
    // `structs.extend(generics.inst_structs)` alone (no rebase) is the
    // id-collision trap: a later downstream mint would count from the same
    // stale base and land on an id a parse-time instance already occupies.
    generics.flush_structs_into(&mut structs);
    generics.flush_enums_into(&mut enums);
    generics.rebase(structs.len(), enums.len());

    let mut module = Module {
        words,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        slices: Vec::new(),
        generic_structs: generics.structs.clone(),
        generic_enums: generics.enums.clone(),
        externs,
        instantiations: HashMap::new(),
        builtin_overloads: HashMap::new(),
        resolved_fields: HashMap::new(),
        resolved_variant_fields: HashMap::new(),
        modules,
        statics,
        generics,
    };
    // R18: checked on the raw, pre-mangle module -- a word's name and its
    // module's `export:` list are both still their raw source spellings here,
    // which `resolve::resolve_modules` would otherwise mangle apart.
    check::check_exported_signatures(&module)?;
    // R20/R21: validate selective imports (each name exported by its source,
    // no collision) on the raw, pre-mangle module.
    check::check_selective_imports(&module, &selective_by_module)?;
    // Phase 7 slice 2 (R6): the `static:` declarations and then the per-word
    // global sets, both pre-mangle for the same reason -- a static's name, a
    // word's name and a module's `export:` list all still agree here.
    check::check_static_decls(&module)?;
    check::check_globals(&module)?;
    resolve::resolve_modules(&mut module, always_mangle)?;
    Ok(module)
}

/// R14/D4 (slice 5b), generalized: reject a closure where a module *other
/// than* `allowed_module` declares a word named `main`, naming the declaring
/// file and the word, before any codegen. `mangle` (`src/resolve.rs`) never
/// renames `main` regardless of module, so a plain name scan over every file
/// in the closure finds it, whichever file it came from.
///
/// `allowed_module` distinguishes the two callers: `eval_import` passes
/// `None`, since every file in an *imported* closure is a library file and
/// none may declare `main`; `build` passes `Some(0)`, since module 0 there is
/// the program's own entry file and is the one place `main` is expected.
pub(crate) fn check_no_main_in_closure(
    module: &Module,
    closure: &Closure,
    allowed_module: Option<u32>,
) -> Result<(), String> {
    let Some(main) = module
        .words
        .iter()
        .find(|w| w.name == "main" && Some(w.module) != allowed_module)
    else {
        return Ok(());
    };
    let path = closure.path_of(main.module);
    let span = check::word_span(main);
    Err(format!(
        "error: `{}` declares a word named `main` (line {}, col {}); a library file may not declare `main`",
        path.display(),
        span.line,
        span.col
    ))
}

/// Compile a source file's whole import closure to emitted QBE IL text: the
/// exact bytes `build` hands to `qbe`, produced without shelling out. The R9
/// baseline golden asserts this stays byte-identical across the slice-8a
/// builtin-table refactor.
pub fn emit_ssa(path: &Path) -> Result<String, String> {
    emit_ssa_with_manifest(path, None)
}

/// `emit_ssa` with a `--manifest` override (tier 1, R1): resolves the entry
/// file's dependency-anchored imports against `manifest` instead of an
/// ancestor manifest (R3), the other fallback tiers unaffected.
pub fn emit_ssa_with_manifest(path: &Path, manifest: Option<&Path>) -> Result<String, String> {
    let mut config = ResolutionConfig::from_env();
    config.manifest_override = manifest.map(PathBuf::from);
    let closure = discover_closure_audited(path, &config)?;
    let mut module = assemble_module(&closure, true)?;
    check::check(&mut module)?;
    // R14/D4 (native-build fix): only the entry file (module 0) may declare
    // `main`; an imported file that also declares one is rejected here,
    // mirroring the REPL import path's own scan.
    check_no_main_in_closure(&module, &closure, Some(0))?;
    let ir = ir::lower(&module)?;
    backend::qbe::emit(&ir)
}

/// Compile a source file to a native binary. Returns the binary's path.
pub fn build(path: &Path) -> Result<PathBuf, String> {
    build_with_manifest(path, None)
}

/// `build` with a `--manifest` override; see `emit_ssa_with_manifest`.
pub fn build_with_manifest(path: &Path, manifest: Option<&Path>) -> Result<PathBuf, String> {
    let ssa = emit_ssa_with_manifest(path, manifest)?;

    let dir = tempfile_dir()?;
    let ssa_path = dir.join("out.ssa");
    let asm_path = dir.join("out.s");
    let shim_path = dir.join("shim.c");
    std::fs::write(&ssa_path, &ssa).map_err(|e| format!("writing {ssa_path:?}: {e}"))?;
    std::fs::write(&shim_path, C_SHIM).map_err(|e| format!("writing {shim_path:?}: {e}"))?;

    run_command(Command::new("qbe").arg(&ssa_path).arg("-o").arg(&asm_path))?;

    let out_path = binary_path(path);
    run_command(
        Command::new("cc")
            .arg(&asm_path)
            .arg(&shim_path)
            .arg("-o")
            .arg(&out_path),
    )?;

    Ok(out_path)
}

/// Compile and run a source file, returning the child's exit status. The caller
/// decides how to propagate it (`main` mirrors it as its own exit code).
pub fn run(path: &Path) -> Result<ExitStatus, String> {
    run_with_manifest(path, None)
}

/// `run` with a `--manifest` override; see `emit_ssa_with_manifest`.
pub fn run_with_manifest(path: &Path, manifest: Option<&Path>) -> Result<ExitStatus, String> {
    let binary = build_with_manifest(path, manifest)?;
    Command::new(&binary)
        .status()
        .map_err(|e| format!("running {binary:?}: {e}"))
}

pub fn repl() -> Result<(), String> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    crate::repl::run(stdin.lock(), stdout.lock())
}

/// Compile QBE IL text to a shared object at `out`. Mirrors `build`'s qbe/cc
/// plumbing but targets a `.so`: no C shim, since a shared object has no `main`.
pub fn compile_so(ssa: &str, out: &Path) -> Result<(), String> {
    let dir = tempfile_dir()?;
    let ssa_path = dir.join("out.ssa");
    let asm_path = dir.join("out.s");
    std::fs::write(&ssa_path, ssa).map_err(|e| format!("writing {ssa_path:?}: {e}"))?;

    run_command(Command::new("qbe").arg(&ssa_path).arg("-o").arg(&asm_path))?;

    let mut cc = Command::new("cc");
    cc.arg("-shared")
        .arg("-fPIC")
        .arg(&asm_path)
        .arg("-o")
        .arg(out);
    // macOS's two-level namespace rejects undefined symbols at link time; allow
    // them (earlier generations, printf) to resolve at load under RTLD_GLOBAL.
    if cfg!(target_os = "macos") {
        cc.arg("-Wl,-undefined,dynamic_lookup");
    }
    run_command(&mut cc)?;
    Ok(())
}

/// A build's scratch directory, removed when dropped. Every current caller writes
/// into it and is done (qbe/cc have already produced their output elsewhere, or
/// `dlopen` has already read the `.so`) by the time its function returns, so
/// scope-end `Drop` is always ordered after last use, not a race against it.
pub(crate) struct TempDir(PathBuf);

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best-effort: a compile error earlier in the same function already
        // reports the real failure, and a second one here on cleanup would
        // only obscure it.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn tempfile_dir() -> Result<TempDir, String> {
    // Each build gets its own scratch dir so concurrent in-process builds (e.g.
    // parallel goldens) don't clobber each other's fixed-name intermediates.
    static N: AtomicU64 = AtomicU64::new(0);
    let seq = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("sooth-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating temp dir {dir:?}: {e}"))?;
    Ok(TempDir(dir))
}

pub(crate) fn run_command(cmd: &mut Command) -> Result<(), String> {
    let output = cmd
        .output()
        .map_err(|e| format!("running {:?}: {e}", cmd.get_program()))?;
    if !output.status.success() {
        return Err(format!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory of `.sth` files for the closure-discovery unit
    /// tests, removed on drop.
    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new(tag: &str) -> Sandbox {
            static N: AtomicU64 = AtomicU64::new(0);
            let seq = N.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("sooth-closure-{}-{tag}-{seq}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Sandbox(dir)
        }
        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, contents).unwrap();
            path
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// U1: graph resolution canonicalizes, dedupes by canonical path, and puts
    /// the entry at id 0. A diamond (entry -> {left, right} -> base) discovers
    /// four files, base exactly once.
    #[test]
    fn resolve_import_graph_dedupes_and_orders() {
        let s = Sandbox::new("diamond");
        s.write("base.sth", ": b ( -- i64 ) 1 ;\n");
        s.write(
            "left.sth",
            "import: \"base.sth\" base ;\n: lf ( -- i64 ) base::b ;\n",
        );
        s.write(
            "right.sth",
            "import: \"base.sth\" base ;\n: rt ( -- i64 ) base::b ;\n",
        );
        let entry = s.write(
            "main.sth",
            "import: \"left.sth\" l ;\nimport: \"right.sth\" r ;\n: main ( -- ) 0 . ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        assert_eq!(closure.nodes.len(), 4, "base deduped: one node per file");
        assert!(
            closure.nodes[0].canon.ends_with("main.sth"),
            "entry is module 0"
        );
        // Exactly one node is base.sth (the diamond's shared dependency).
        let bases = closure
            .nodes
            .iter()
            .filter(|n| n.canon.ends_with("base.sth"))
            .count();
        assert_eq!(bases, 1, "base reached by two importers, parsed once");
    }

    /// U2: a mutual import is a located cycle error naming both files.
    #[test]
    fn import_cycle_detected_with_both_files() {
        let s = Sandbox::new("cycle");
        s.write("a.sth", "import: \"b.sth\" b ;\n: main ( -- ) 0 . ;\n");
        s.write("b.sth", "import: \"a.sth\" a ;\n: q ( -- i64 ) 1 ;\n");
        let entry = s.0.join("a.sth");
        let err = match discover_closure(&entry) {
            Ok(_) => panic!("a cycle must be rejected"),
            Err(e) => e,
        };
        assert!(err.contains("cycle"), "names the failure: {err}");
        assert!(err.contains("a.sth"), "names the first file: {err}");
        assert!(err.contains("b.sth"), "names the second file: {err}");
    }

    /// The native build path's entry file (module 0) is allowed to declare
    /// `main` -- the common case -- while an imported file that declares none
    /// checks cleanly.
    #[test]
    fn check_no_main_in_closure_allows_entry_module_main() {
        let s = Sandbox::new("entry-main-ok");
        s.write("lib.sth", ": helper ( -- i64 ) 1 ;\nexport: helper ;\n");
        let entry = s.write(
            "main.sth",
            "import: \"lib.sth\" l ;\n: main ( -- ) l::helper drop ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module).expect("checks");
        check_no_main_in_closure(&module, &closure, Some(0))
            .expect("module 0's own `main` is allowed");
    }

    /// P7 slice 2 review fix: an imported inline combinator splices its body
    /// under its own declaring module (`ctx.with_module`), including the
    /// caller's own `~[ ... ]` argument, so a caller borrowing its own static
    /// inside that argument used to fail to check -- `Ctx::static_type`
    /// filtered on `s.module == ctx.module()`, which the splice sets to the
    /// *callee's* module, not the caller's. This is exactly the shape every
    /// `lib/combinators.sth` user (`while`/`each`/`map`/`fold`/`filter`/
    /// `times`) reaches with a static in play.
    #[test]
    fn imported_inline_combinator_sees_callers_own_static() {
        let s = Sandbox::new("combinator-sees-static");
        s.write(
            "lib.sth",
            ": apply inline ( ~[ -- ] -- ) call ;\nexport: apply ;\n",
        );
        let entry = s.write(
            "main.sth",
            "import: \"lib.sth\" c ;\n\
             static: COUNT i64 = 0 ;\n\
             : main ( -- ) ~[ &!COUNT 1 +! ] c::apply &COUNT @ drop ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module)
            .expect("`main`'s own static is in scope inside the spliced quotation argument");
    }

    /// The native build path rejects an *imported* file that also declares
    /// `main`, naming that file and the word -- the bug this test guards
    /// against regressing (previously nothing rejected this).
    #[test]
    fn check_no_main_in_closure_rejects_imported_module_main() {
        let s = Sandbox::new("imported-main-bad");
        let lib = s.write(
            "lib.sth",
            ": helper ( -- i64 ) 1 ;\n: main ( -- ) ;\nexport: helper ;\n",
        );
        let entry = s.write(
            "main.sth",
            "import: \"lib.sth\" l ;\n: main ( -- ) l::helper drop ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module).expect("checks");
        let err = check_no_main_in_closure(&module, &closure, Some(0)).unwrap_err();
        assert!(err.contains("main"), "names the word: {err}");
        assert!(
            err.contains(lib.file_name().unwrap().to_str().unwrap()),
            "names the imported file, not the entry: {err}"
        );
    }

    /// End-to-end regression guard, through `build` itself rather than the
    /// helper directly: the bug this whole fix exists for was that
    /// `cargo run -- build` never called `check_no_main_in_closure` on the
    /// native path at all (only `Session::eval_import`, the REPL path, did),
    /// so an imported file's `main` reached codegen silently. Calling
    /// `check_no_main_in_closure` in isolation (the two tests above) cannot
    /// catch a missing call site; only driving `build` proves it is wired in.
    #[test]
    fn build_rejects_imported_module_declaring_main() {
        let s = Sandbox::new("build-imported-main-bad");
        let lib = s.write(
            "lib.sth",
            ": helper ( -- i64 ) 1 ;\n: main ( -- ) ;\nexport: helper ;\n",
        );
        let entry = s.write(
            "main.sth",
            "import: \"lib.sth\" l ;\n: main ( -- ) l::helper drop ;\n",
        );
        let err = build(&entry).expect_err("an imported `main` must reject the native build");
        assert!(err.contains("main"), "names the word: {err}");
        assert!(
            err.contains(lib.file_name().unwrap().to_str().unwrap()),
            "names the imported file, not the entry: {err}"
        );
    }

    /// Phase 5 slice 1 phase 2 (round-2 review fix, R4): two structs
    /// sharing a bare name across modules (a local `P` and an imported
    /// `o::P` with a different field count, so a wrong dedup gives `W` the
    /// wrong layout) must monomorphize `Box[...]` to two distinct
    /// `StructId`s *and* to two distinct names. Ids and names are separate
    /// claims: dedup is keyed on `Type` identity, so the ids stay distinct
    /// even with the naming tie-break gone -- what collapses then is the
    /// rendered name, and `check`'s duplicate-type rule rejects the program.
    #[test]
    fn instantiate_struct_distinct_across_modules_same_bare_name() {
        let s = Sandbox::new("generic-cross-module-name-collision");
        s.write("other.sth", "type: P a i64 b i64 ;\nexport: P ;\n");
        let entry = s.write(
            "main.sth",
            "import: \"other.sth\" o ;\ntype: P x i64 ;\ntype: Box 'T val 'T ;\ntype: W a Box[P] b Box[o::P] ;\n: main ( -- ) ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module)
            .expect("two instantiations of one generic are not a duplicate type");

        let w = module
            .structs
            .iter()
            .find(|d| d.name.starts_with("W"))
            .expect("W is registered");
        let (_, box_a) = &w.fields[0];
        let (_, box_b) = &w.fields[1];
        let crate::ast::Type::Struct(box_a_id, _) = box_a else {
            panic!("field a is a struct: {box_a:?}")
        };
        let crate::ast::Type::Struct(box_b_id, _) = box_b else {
            panic!("field b is a struct: {box_b:?}")
        };
        assert_ne!(
            box_a_id, box_b_id,
            "Box[P] and Box[o::P] must mint distinct StructIds"
        );
        assert_ne!(
            module.structs[box_a_id.index()].name,
            module.structs[box_b_id.index()].name,
            "Box[P] and Box[o::P] must also render distinct names"
        );

        let val_field_count = |box_id: &crate::ast::StructId| {
            let boxed = &module.structs[box_id.index()];
            let (_, val_ty) = &boxed.fields[0];
            let crate::ast::Type::Struct(p_id, _) = val_ty else {
                panic!("Box's val field is a struct: {val_ty:?}")
            };
            module.structs[p_id.index()].fields.len()
        };
        assert_eq!(
            val_field_count(box_a_id),
            1,
            "Box[P]'s val is the local one-field P"
        );
        assert_eq!(
            val_field_count(box_b_id),
            2,
            "Box[o::P]'s val is the imported two-field P, not the local one"
        );
    }

    /// The same collision one indirection down (round-3 review fix, R4).
    /// `^P` and `[P 2]` take their spellings from `intern_owned_cell_type`/
    /// `intern_array_type`, both built from the module-blind `Type::name()`,
    /// so `Box[^P]` and `Box[^o::P]` render identically unless the
    /// instantiation name recurses into the wrapper's registry entry. A
    /// legal program was rejected as a `duplicate type` before it did.
    #[test]
    fn instantiate_struct_distinct_for_wrapped_cross_module_args() {
        let s = Sandbox::new("generic-cross-module-wrapped-collision");
        s.write("other.sth", "type: P a i64 b i64 ;\nexport: P ;\n");
        let entry = s.write(
            "main.sth",
            "import: \"other.sth\" o ;\ntype: P x i64 ;\ntype: Box 'T val 'T ;\ntype: W a Box[^P] b Box[^o::P] c Box[[P 2]] d Box[[o::P 2]] ;\n: main ( -- ) ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module).expect("four distinct instantiations are not a duplicate type");

        let w = module
            .structs
            .iter()
            .find(|d| d.name.starts_with("W"))
            .expect("W is registered");
        let names: Vec<&str> = w
            .fields
            .iter()
            .map(|(_, ty)| {
                let crate::ast::Type::Struct(id, _) = ty else {
                    panic!("every field of W is a struct: {ty:?}")
                };
                module.structs[id.index()].name.as_str()
            })
            .collect();
        let unique: std::collections::HashSet<&&str> = names.iter().collect();
        assert_eq!(
            unique.len(),
            4,
            "each wrapped instantiation needs its own name: {names:?}"
        );
    }

    /// A two-file generic closure assembled through the real `assemble_module`
    /// path: `box.sth` declares `Box 'T`, `use.sth` applies it qualified as
    /// `b::Box[i64]`, and the entry file's `import:` order decides which of the
    /// two discovery reaches first.
    fn assemble_generic_cross_module_closure(tag: &str, entry_src: &str) -> Module {
        let s = Sandbox::new(tag);
        s.write("box.sth", "type: Box 'T val 'T ;\nexport: Box ;\n");
        s.write(
            "use.sth",
            "import: \"box.sth\" b ;\n: unwrap ( b::Box[i64] -- i64 ) Box> ;\n\
             : show ( i64 -- ) Box unwrap . ;\nexport: show ;\n",
        );
        let entry = s.write("main.sth", entry_src);
        let closure = discover_closure(&entry).expect("closure resolves");
        assemble_module(&closure, true).expect("assembles")
    }

    const APPLIER_FIRST: &str =
        "import: \"use.sth\" u ;\nimport: \"box.sth\" b ;\n: main ( -- ) 7 u::show ;\n";
    const OWNER_FIRST: &str =
        "import: \"box.sth\" b ;\nimport: \"use.sth\" u ;\n: main ( -- ) 7 u::show ;\n";

    /// Slice 2 (OQ1): a qualified cross-module generic application resolves and
    /// monomorphizes whichever order discovery reached the two files in. The
    /// applier-first arrangement is the one the whole-closure header pre-pass
    /// exists for: without it the applying module body-parses before the
    /// declaring module has registered its header, and a legal program fails
    /// with `unknown type` on nothing but its entry file's import order.
    #[test]
    fn generic_application_resolves_cross_module_in_either_discovery_order() {
        for (tag, entry_src) in [
            ("generic-xmod-applier-first", APPLIER_FIRST),
            ("generic-xmod-owner-first", OWNER_FIRST),
        ] {
            let mut module = assemble_generic_cross_module_closure(tag, entry_src);
            check::check(&mut module).unwrap_or_else(|e| panic!("{tag} checks: {e}"));
            assert!(
                module.structs.iter().any(|d| d.name_static == "Box[i64]"),
                "{tag}: the qualified application monomorphized"
            );
        }
    }

    /// Slice 2 (OQ1), the idempotency half: the declaring module's header is
    /// registered by the whole-closure pre-pass and not a second time by that
    /// module's own `parse_bodies`. This is the only path where both can fire
    /// over the same tokens -- the single-file and direct-`parse_bodies`
    /// callers have no pre-pass above them, so a count assertion there would
    /// pass with or without the guard.
    #[test]
    fn whole_closure_generic_pre_pass_registers_each_header_once() {
        let module = assemble_generic_cross_module_closure("generic-xmod-once", OWNER_FIRST);
        let declared = module
            .generic_structs
            .iter()
            .filter(|d| d.name == "Box")
            .count();
        assert_eq!(declared, 1, "`Box` is registered once, not per pass");
    }

    /// R15c: a selectively imported generic name applies bare, the rule a
    /// selectively imported concrete type already follows -- the spelling a
    /// `Result`/`Option` user reaches for before the qualified one.
    #[test]
    fn selectively_imported_generic_name_applies_bare() {
        let s = Sandbox::new("generic-xmod-selective");
        s.write("box.sth", "type: Box 'T val 'T ;\nexport: Box ;\n");
        let entry = s.write(
            "main.sth",
            "import: \"box.sth\" b | Box | ;\n: unwrap ( Box[i64] -- i64 ) Box> ;\n\
             : main ( -- ) 7 Box unwrap . ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module).expect("the bare selective application checks");
        assert!(
            module.structs.iter().any(|d| d.name_static == "Box[i64]"),
            "the bare selective application monomorphized"
        );
    }

    /// Own module first: a locally declared generic header wins over a
    /// selectively imported one of the same name, the order
    /// `resolve_type_name_in_module` already gives a concrete name. (For a
    /// concrete type that arrangement is an R21 collision error instead --
    /// `check_selective_imports` reads the instantiated `structs`/`enums`
    /// registries, where a generic header has no entry under its bare name.)
    #[test]
    fn local_generic_header_shadows_a_selectively_imported_one() {
        let s = Sandbox::new("generic-xmod-selective-shadow");
        s.write("box.sth", "type: Box 'T val 'T ;\nexport: Box ;\n");
        let entry = s.write(
            "main.sth",
            "import: \"box.sth\" b | Box | ;\ntype: Box 'T val 'T tag i64 ;\n\
             type: W f Box[i64] ;\n: main ( -- ) ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        let module = assemble_module(&closure, true).expect("assembles");
        let boxed = module
            .structs
            .iter()
            .find(|d| d.name_static == "Box[i64]")
            .expect("the application monomorphized");
        let names: Vec<&str> = boxed.fields.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["val", "tag"], "the local header was applied");
    }

    /// The whole-closure pre-pass parses a generic header against the same
    /// name environment `parse_bodies` gets, not a bare one: a field naming an
    /// imported concrete type (`o::P`) has to resolve through the declaring
    /// module's import map and its target's `export:` list in both passes.
    #[test]
    fn generic_header_field_naming_an_imported_type_resolves_in_the_pre_pass() {
        let s = Sandbox::new("generic-field-imported-type");
        s.write("other.sth", "type: P a i64 ;\nexport: P ;\n");
        let entry = s.write(
            "main.sth",
            "import: \"other.sth\" o ;\ntype: Box 'T val 'T p o::P ;\n\
             type: W b Box[i64] ;\n: main ( -- ) ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        let module = assemble_module(&closure, true).expect("assembles");
        let boxed = module
            .structs
            .iter()
            .find(|d| d.name_static == "Box[i64]")
            .expect("the application monomorphized");
        let (_, p_field) = &boxed.fields[1];
        let crate::ast::Type::Struct(id, _) = p_field else {
            panic!("the imported field is a struct: {p_field:?}")
        };
        assert_eq!(module.structs[id.index()].name_static, "P");
    }

    /// The located error a closure discovery is expected to fail with.
    /// `Closure` is not `Debug`, so `expect_err` is not available.
    /// `discover_closure` with both fallback tiers pinned off (R6): a bare
    /// `discover_closure(&entry)` reads the invoking machine's real
    /// `$XDG_CONFIG_HOME/sooth/global_sooth.pkg`, which can turn a
    /// manifest-less fixture's expected tier-4 error into a tier-3 resolution
    /// (or a different error) on any machine that has one. Every
    /// `discover_err` caller pins a located *error*, so an explicit,
    /// user-manifest-free config keeps them reproducible everywhere.
    fn discover_err(entry: &Path) -> String {
        let config = ResolutionConfig {
            manifest_override: None,
            user_manifest: None,
        };
        match discover_closure_audited(entry, &config) {
            Ok(_) => panic!("expected a located error from {}", entry.display()),
            Err(e) => e,
        }
    }

    /// A package fixture: `sooth.pkg` at the sandbox root, plus whatever files
    /// the caller writes under it.
    fn pkg(sb: &Sandbox, dir: &str, manifest: &str) {
        sb.write(&format!("{dir}sooth.pkg"), manifest);
    }

    /// P8 slice 1a: an intra-package `self::` import resolves by path-joining
    /// the package root, so a file naming a sibling BFS has not reached yet
    /// still lands on it -- and every edge is in the graph before
    /// `reject_cycles` runs.
    #[test]
    fn discover_closure_intra_package_forward_reference_resolves() {
        let s = Sandbox::new("selfimport");
        pkg(&s, "", "package: app ; layer: hosted ;");
        s.write("b.sth", ": bw ( -- i64 ) 2 ;\nexport: bw ;\n");
        s.write(
            "a.sth",
            "import: self::b ;\n: aw ( -- i64 ) b::bw ;\nexport: aw ;\n",
        );
        let entry = s.write(
            "main.sth",
            "import: self::a ;\nimport: self::b ;\n: main ( -- ) a::aw b::bw add . ;\n",
        );
        let closure = discover_closure(&entry).expect("the self-anchored imports resolve");
        assert_eq!(closure.nodes.len(), 3, "b is discovered once, not twice");
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module).expect("checks");
    }

    /// Two files of one package parse that package's manifest once, not once
    /// per file.
    #[test]
    fn discover_closure_manifest_cache_reads_once() {
        let s = Sandbox::new("manifest-cache");
        pkg(&s, "", "package: app ; layer: hosted ;");
        s.write("b.sth", ": bw ( -- i64 ) 2 ;\nexport: bw ;\n");
        let entry = s.write("main.sth", "import: self::b ;\n: main ( -- ) b::bw . ;\n");
        let mut manifests = ManifestCache::default();
        discover_closure_configured(&entry, &ResolutionConfig::from_env(), &mut manifests)
            .expect("closure resolves");
        assert_eq!(
            manifests.parses, 1,
            "one manifest owns both files, so it is parsed once"
        );
    }

    /// R3, end to end through the closure walk: `--manifest` resolves the
    /// entry file's dependency import, while the file that import pulls in
    /// resolves its own `self::` sibling against its *own* ancestor manifest.
    /// Mutation-test by applying the override to every file in the closure:
    /// `lib.sth`'s `self::helper` then joins the flag manifest's root, where
    /// nothing is.
    #[test]
    fn discover_closure_configured_flag_override_entry_only() {
        let s = Sandbox::new("flag-entry-only");
        pkg(
            &s,
            "flag/",
            "package: flagpkg ; layer: hosted ;\ndepends: dep path \"../dep\" ;",
        );
        pkg(&s, "dep/", "package: dep ; layer: hosted ;\nmodule: lib ;");
        s.write("dep/helper.sth", ": hw ( -- i64 ) 7 ;\nexport: hw ;\n");
        s.write(
            "dep/lib.sth",
            "import: self::helper h ;\n: lw ( -- i64 ) h::hw ;\nexport: lw ;\n",
        );
        let entry = s.write(
            "scratch/main.sth",
            "import: dep::lib l ;\n: main ( -- ) l::lw . ;\n",
        );
        let mut config = ResolutionConfig::from_env();
        config.manifest_override = Some(s.0.join("flag/sooth.pkg"));
        let mut manifests = ManifestCache::default();
        let closure = discover_closure_configured(&entry, &config, &mut manifests)
            .expect("the flag resolves the entry, the dependency resolves itself");
        assert_eq!(
            closure.nodes.len(),
            3,
            "entry, dep::lib, and dep's own self::helper"
        );
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module).expect("checks");
    }

    /// R1 tier 3: a manifest-less entry file with no `--manifest` resolves a
    /// dependency module against the user-level manifest, pointed at a fixture
    /// rather than at a real `$XDG_CONFIG_HOME` (R6). Mutation-test by making
    /// `select_site`'s tier-3 branch return `Ok(None)`: the import then has no
    /// `depends:` table to resolve against.
    #[test]
    fn discover_closure_configured_user_manifest_fallback() {
        let s = Sandbox::new("user-level-fallback");
        let user_manifest = s.write("cfg/global_sooth.pkg", "depends: dep path \"../dep\" ;");
        pkg(&s, "dep/", "package: dep ; layer: hosted ;\nmodule: lib ;");
        s.write("dep/lib.sth", ": lw ( -- i64 ) 7 ;\nexport: lw ;\n");
        let entry = s.write(
            "scratch/main.sth",
            "import: dep::lib l ;\n: main ( -- ) l::lw . ;\n",
        );
        let config = ResolutionConfig {
            manifest_override: None,
            user_manifest: Some(user_manifest),
        };
        let mut manifests = ManifestCache::default();
        let closure = discover_closure_configured(&entry, &config, &mut manifests)
            .expect("the user-level manifest resolves the dependency");
        assert_eq!(closure.nodes.len(), 2, "entry and dep::lib");
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module).expect("checks");
    }

    /// R1 tier 4: neither `--manifest` nor a user-level manifest is present,
    /// so a manifest-less entry falls all the way to the implicit anonymous
    /// package -- a quoted-path sibling still resolves there. Driven through
    /// an explicit `ResolutionConfig` with both fields `None`, never through
    /// `ResolutionConfig::from_env()`, so the assertion can't be at the mercy
    /// of the test machine's real `$XDG_CONFIG_HOME`.
    #[test]
    fn discover_closure_configured_anonymous_fallback() {
        let s = Sandbox::new("anonymous-fallback");
        s.write("b.sth", ": bw ( -- i64 ) 2 ;\nexport: bw ;\n");
        let entry = s.write(
            "main.sth",
            "import: \"b.sth\" b ;\n: main ( -- ) b::bw . ;\n",
        );
        let config = ResolutionConfig {
            manifest_override: None,
            user_manifest: None,
        };
        let mut manifests = ManifestCache::default();
        let closure = discover_closure_configured(&entry, &config, &mut manifests)
            .expect("a quoted-path sibling resolves under an anonymous package");
        assert_eq!(closure.nodes.len(), 2, "entry and b.sth");
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module).expect("checks");
    }

    fn os(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn user_manifest_path_prefers_xdg_config_home() {
        assert_eq!(
            user_manifest_path(os("/x/cfg"), os("/home/u")),
            Some(PathBuf::from("/x/cfg/sooth/global_sooth.pkg"))
        );
    }

    #[test]
    fn user_manifest_path_empty_xdg_falls_back_to_home() {
        assert_eq!(
            user_manifest_path(os(""), os("/home/u")),
            Some(PathBuf::from("/home/u/.config/sooth/global_sooth.pkg"))
        );
    }

    #[test]
    fn user_manifest_path_unset_xdg_falls_back_to_home() {
        assert_eq!(
            user_manifest_path(None, os("/home/u")),
            Some(PathBuf::from("/home/u/.config/sooth/global_sooth.pkg"))
        );
    }

    #[test]
    fn user_manifest_path_neither_set_is_none() {
        assert_eq!(user_manifest_path(None, None), None);
    }

    /// OQ2 manifest locality: a file's package is its *nearest* ancestor
    /// manifest, so an inner package's own `self::leaf` is the inner file even
    /// when the outer package holds a `leaf.sth` of its own.
    #[test]
    fn discover_closure_inner_manifest_wins() {
        let s = Sandbox::new("inner-wins");
        pkg(&s, "", "package: outer ; layer: hosted ;");
        pkg(&s, "inner/", "package: inner ; layer: hosted ;");
        s.write("leaf.sth", ": lw ( -- i64 ) 1 ;\nexport: lw ;\n");
        s.write("inner/leaf.sth", ": lw ( -- i64 ) 2 ;\nexport: lw ;\n");
        let entry = s.write(
            "inner/main.sth",
            "import: self::leaf ;\n: main ( -- ) leaf::lw . ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        assert!(
            closure.path_of(1).ends_with("inner/leaf.sth"),
            "the inner package's own leaf, not the outer one: {}",
            closure.path_of(1).display()
        );
    }

    /// A file inside a package names its imports by module name; the
    /// quoted-path form survives only outside a package.
    #[test]
    fn discover_closure_quoted_path_inside_package_is_error() {
        let s = Sandbox::new("quoted-in-package");
        pkg(&s, "", "package: app ; layer: hosted ;");
        s.write("b.sth", ": bw ( -- i64 ) 2 ;\nexport: bw ;\n");
        let entry = s.write(
            "main.sth",
            "import: \"b.sth\" b ;\n: main ( -- ) b::bw . ;\n",
        );
        let err = discover_err(&entry);
        assert!(
            err.contains("error: quoted-path import at line 1, col 1 in")
                && err.contains("file is in package `app`: use a module name"),
            "unexpected message: {err}"
        );
    }

    /// P8 slice 1a: a wildcard import binds no qualifier and gets no
    /// visibility effect until S2 -- so a compiled build rejects it outright,
    /// exactly as the REPL does, rather than silently binding no names.
    #[test]
    fn driver_wildcard_import_is_error() {
        let s = Sandbox::new("wildcard-build");
        s.write("lib.sth", ": lw ( -- i64 ) 1 ;\nexport: lw ;\n");
        let entry = s.write("main.sth", "import: \"lib.sth\" * ;\n: main ( -- ) 0 . ;\n");
        let closure = discover_closure(&entry).expect("a wildcard import still resolves a target");
        let err = assemble_module(&closure, true).expect_err("a wildcard import is rejected");
        assert!(
            err.contains("error: wildcard import at line 1, col 1 in")
                && err.contains("a wildcard import binds no names in this build")
                && err.contains("use a qualified import instead"),
            "unexpected message: {err}"
        );
    }

    /// F6: `intrinsics` is the one wildcard shape a compiled build accepts --
    /// it adds no closure edge and needs no qualifier, so it must reach
    /// `assemble_module` without tripping the wildcard rejection above.
    #[test]
    fn driver_intrinsics_wildcard_import_builds() {
        let s = Sandbox::new("intrinsics-wildcard-build");
        let entry = s.write(
            "main.sth",
            "import: intrinsics * ;\n: main ( -- ) 1 1 add . ;\n",
        );
        let closure = discover_closure(&entry).expect("the reserved name needs no depends:");
        assemble_module(&closure, true).expect("intrinsics wildcard import must still build");
    }

    /// P8 slice 1a: two imports binding the same qualifier -- here both
    /// defaulting to their last segment -- is a located error at the second,
    /// naming where the first bound it. No shadowing, no precedence.
    #[test]
    fn driver_duplicate_import_qualifier_is_error() {
        let s = Sandbox::new("dup-qualifier");
        pkg(&s, "", "package: app ; layer: hosted ;");
        s.write("text/ascii.sth", ": tw ( -- i64 ) 1 ;\nexport: tw ;\n");
        s.write("bin/ascii.sth", ": bw ( -- i64 ) 2 ;\nexport: bw ;\n");
        let entry = s.write(
            "main.sth",
            "import: self::text::ascii ;\nimport: self::bin::ascii ;\n: main ( -- ) 0 . ;\n",
        );
        let closure = discover_closure(&entry).expect("both targets resolve");
        let err = assemble_module(&closure, true).expect_err("the second binding is an error");
        assert!(
            err.contains("error: duplicate import qualifier `ascii` at line 2, col 1 in")
                && err.contains("qualifier `ascii` was first bound at line 1, col 1"),
            "unexpected message: {err}"
        );
    }

    /// OQ4 failure mode D2, `self::` side: path-joining lands on a real file
    /// that a nested inner manifest owns, so it is not a module of the
    /// importer's package at all. This re-check is what keeps `self::` from
    /// reaching into a nested package and around its layer check.
    #[test]
    fn self_anchored_import_into_nested_package_is_error() {
        let s = Sandbox::new("self-into-nested");
        pkg(&s, "", "package: app ; layer: hosted ;");
        pkg(&s, "inner/", "package: inner ; layer: hosted ;");
        s.write("inner/thing.sth", ": tw ( -- i64 ) 1 ;\nexport: tw ;\n");
        let entry = s.write(
            "main.sth",
            "import: self::inner::thing ;\n: main ( -- ) thing::tw . ;\n",
        );
        let err = discover_err(&entry);
        assert!(
            err.contains("error: import `self::inner::thing` at line 1, col 1 in")
                && err.contains("package `app` has no module `inner::thing`")
                && err.contains("belongs to the nested package rooted at")
                && err.contains("inner/sooth.pkg`, not `app`"),
            "unexpected message: {err}"
        );
    }

    /// OQ4 failure mode D2, dependency side: the dependency publishes the
    /// module, but its own nested manifest owns the file the name joins to.
    #[test]
    fn dependency_anchored_import_into_nested_package_is_error() {
        let s = Sandbox::new("dep-into-nested");
        pkg(
            &s,
            "app/",
            "package: app ; layer: hosted ;\ndepends: dep path \"../dep\" ;",
        );
        pkg(
            &s,
            "dep/",
            "package: dep ; layer: hosted ;\nmodule: inner::thing ;",
        );
        pkg(&s, "dep/inner/", "package: depinner ; layer: hosted ;");
        s.write("dep/inner/thing.sth", ": tw ( -- i64 ) 1 ;\nexport: tw ;\n");
        let entry = s.write(
            "app/main.sth",
            "import: dep::inner::thing ;\n: main ( -- ) thing::tw . ;\n",
        );
        let err = discover_err(&entry);
        assert!(
            err.contains("error: import `dep::inner::thing` at line 1, col 1 in")
                && err.contains("package `dep` has no module `inner::thing`")
                && err.contains("belongs to the nested package rooted at")
                && err.contains("dep/inner/sooth.pkg`, not `dep`"),
            "unexpected message: {err}"
        );
    }

    /// F6: `intrinsics` is reserved only as a bare Dependency-anchored name.
    /// `self::intrinsics` is an ordinary own-package module name, so with no
    /// `intrinsics.sth` in the package it is D1, not the reserved fast path.
    #[test]
    fn self_intrinsics_is_not_the_reserved_name() {
        let s = Sandbox::new("self-intrinsics");
        pkg(&s, "", "package: app ; layer: hosted ;");
        let entry = s.write(
            "main.sth",
            "import: self::intrinsics ;\n: main ( -- ) 0 . ;\n",
        );
        let err = discover_err(&entry);
        assert!(
            err.contains("error: import `self::intrinsics` at line 1, col 1 in")
                && err.contains("package `app` has no module `intrinsics` (looked for ")
                && err.contains("intrinsics.sth)"),
            "unexpected message: {err}"
        );
    }

    /// F6: the reserved name is matched ahead of the `depends:` lookup, so it
    /// resolves in a package with no `depends:` at all -- and adds no closure
    /// edge, having no file.
    #[test]
    fn resolve_intrinsics_precedes_depends_lookup() {
        let s = Sandbox::new("intrinsics-reserved");
        pkg(&s, "", "package: app ; layer: hosted ;");
        let entry = s.write("main.sth", "import: intrinsics * ;\n: main ( -- ) 0 . ;\n");
        let closure = discover_closure(&entry).expect("the reserved name needs no `depends:`");
        assert_eq!(closure.nodes.len(), 1, "the reserved name adds no edge");
    }

    /// OQ4: a Dependency-anchored target with no segments past the package
    /// name names a package, not a module -- checked ahead of the `depends:`
    /// lookup, so the message says what is actually wrong.
    #[test]
    fn resolve_bare_package_name_no_module_is_error() {
        let s = Sandbox::new("bare-package");
        pkg(&s, "", "package: app ; layer: hosted ;");
        let entry = s.write("main.sth", "import: core ;\n: main ( -- ) 0 . ;\n");
        let err = discover_err(&entry);
        assert!(
            err.contains("error: import `core` at line 1, col 1 in")
                && err.contains(
                    "`core` names a package, not a module -- import one of its `module:` entries"
                ),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains("depends:"),
            "the bare-name check precedes the `depends:` lookup: {err}"
        );
    }

    /// F2: `module:` visibility is never consulted for a `self::` anchor --
    /// every module of the importer's own package stays reachable. A
    /// regression fence against a future accidental coupling, not a
    /// killed-mutant guard: nothing on this path reads `module:` by design.
    #[test]
    fn self_import_of_non_public_module_is_ok() {
        let s = Sandbox::new("self-private");
        pkg(&s, "", "package: app ; layer: hosted ;\nmodule: other ;");
        s.write("secret.sth", ": sw ( -- i64 ) 1 ;\nexport: sw ;\n");
        let entry = s.write(
            "main.sth",
            "import: self::secret ;\n: main ( -- ) secret::sw . ;\n",
        );
        let closure = discover_closure(&entry).expect("`module:` does not gate a `self::` import");
        assert_eq!(closure.nodes.len(), 2);
    }

    /// OQ2: a segment that does not lex as a single `Token::Word` names no
    /// module. The file is written, so the rejection is the naming rule and
    /// not a not-found error standing in for it.
    #[test]
    fn import_target_non_word_segment_is_error() {
        let s = Sandbox::new("non-word-segment");
        pkg(&s, "", "package: app ; layer: hosted ;");
        s.write("42.sth", ": nw ( -- i64 ) 1 ;\nexport: nw ;\n");
        let entry = s.write("main.sth", "import: self::42 q ;\n: main ( -- ) 0 . ;\n");
        let err = discover_err(&entry);
        assert!(
            err.contains("error: import `self::42` at line 1, col 1 in")
                && err.contains("module-name segment `42` is not a single identifier"),
            "unexpected message: {err}"
        );
    }

    /// OQ2/OQ3: a bare `*` segment lexes as an ordinary word, so only the
    /// reserved-target rule keeps `*.sth` from being importable.
    #[test]
    fn import_target_star_segment_is_error() {
        let s = Sandbox::new("star-segment");
        pkg(&s, "", "package: app ; layer: hosted ;");
        s.write("*.sth", ": sw ( -- i64 ) 1 ;\nexport: sw ;\n");
        let entry = s.write("main.sth", "import: self::* q ;\n: main ( -- ) 0 . ;\n");
        let err = discover_err(&entry);
        assert!(
            err.contains("error: import `self::*` at line 1, col 1 in")
                && err.contains("module-name segment `*` is reserved for the wildcard import"),
            "unexpected message: {err}"
        );
    }

    /// OQ2: `..` is rejected at the segment-validity stage, so a `self::`
    /// import cannot spell the same file two different ways (here,
    /// `self::sub::..::sub::y` alongside `self::sub::y`).
    #[test]
    fn import_target_dotdot_segment_is_error() {
        let s = Sandbox::new("dotdot-segment");
        pkg(&s, "", "package: app ; layer: hosted ;");
        s.write("sub/y.sth", ": yw ( -- i64 ) 1 ;\nexport: yw ;\n");
        let entry = s.write(
            "main.sth",
            "import: self::sub::..::sub::y q ;\n: main ( -- ) 0 . ;\n",
        );
        let err = discover_err(&entry);
        assert!(
            err.contains("error: import `self::sub::..::sub::y` at line 1, col 1 in")
                && err.contains(
                    "module-name segment `..` is reserved for directory navigation, not a module name"
                ),
            "unexpected message: {err}"
        );
    }

    /// `existing_module_file` checks `is_file` before canonicalizing exactly so
    /// a directory cannot resolve as a module. `module_file` joins segments
    /// under directories and appends `.sth` only to the last one, so the path
    /// that can collide with a directory is one literally named `<segment>.sth`
    /// -- here a directory `text.sth/` (holding further modules), with no file
    /// `text.sth` beside it. `import: self::text ;` must fail D1, not resolve
    /// into the directory.
    #[test]
    fn import_target_directory_is_not_a_module() {
        let s = Sandbox::new("directory-as-module");
        pkg(&s, "", "package: app ; layer: hosted ;");
        s.write("text.sth/ascii.sth", ": tw ( -- i64 ) 1 ;\nexport: tw ;\n");
        let entry = s.write("main.sth", "import: self::text ;\n: main ( -- ) 0 . ;\n");
        let err = discover_err(&entry);
        assert!(
            err.contains("error: import `self::text` at line 1, col 1 in")
                && err.contains("package `app` has no module `text` (looked for ")
                && err.contains("text.sth)"),
            "unexpected message: {err}"
        );
    }

    /// A module name outside any package, with no user-level manifest either
    /// (S1b R2(c)), is an implicit anonymous package: `self::` names no
    /// package identity there, so it is the same located error as any other
    /// module name would get. (The quoted-path form still works there.)
    #[test]
    fn module_import_outside_a_package_is_error() {
        let s = Sandbox::new("no-manifest");
        s.write("b.sth", ": bw ( -- i64 ) 2 ;\nexport: bw ;\n");
        let entry = s.write("main.sth", "import: self::b ;\n: main ( -- ) b::bw . ;\n");
        let err = discover_err(&entry);
        assert!(
            err.contains("error: import `self::b` at line 1, col 1 in")
                && err.contains(
                    "has no ancestor `sooth.pkg` and no user-level manifest, so it is an implicit anonymous package"
                )
                && err.contains("`self` cannot be resolved"),
            "unexpected message: {err}"
        );
    }

    /// A `depends:` path pointing where no manifest is, located to the entry
    /// in the declaring manifest rather than to the import that tripped over
    /// it: the manifest is what is wrong.
    #[test]
    fn depends_entry_with_no_manifest_is_error() {
        let s = Sandbox::new("depends-no-manifest");
        pkg(
            &s,
            "app/",
            "package: app ; layer: hosted ;\ndepends: dep path \"../dep\" ;",
        );
        s.write("dep/cmp.sth", ": lt ( -- i64 ) 1 ;\nexport: lt ;\n");
        let entry = s.write(
            "app/main.sth",
            "import: dep::cmp c ;\n: main ( -- ) c::lt . ;\n",
        );
        let err = discover_err(&entry);
        assert!(
            err.contains("error: `depends:` entry `dep` at line 2, col 1 in")
                && err.contains("app/sooth.pkg:")
                && err.contains("no manifest at")
                && err.contains("dep/sooth.pkg"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn driver_binary_path_from_source_stem() {
        assert_eq!(
            binary_path(Path::new("examples/gcd.sth")),
            PathBuf::from("examples/gcd")
        );
    }

    #[test]
    fn compile_so_produces_loadable_object() {
        let src = ": sq ( i64 -- i64 ) | n | n n mul ;";
        let tokens = lexer::lex(src).unwrap();
        let mut module = parser::parse(&tokens).unwrap();
        check::check(&mut module).unwrap();
        let ir = ir::lower(&module).unwrap();
        let ssa = backend::qbe::emit(&ir).unwrap();

        let dir = tempfile_dir().unwrap();
        let so = dir.join("libsq.so");
        compile_so(&ssa, &so).expect("compile_so should succeed");
        assert!(so.exists(), "shared object should exist at {so:?}");
    }

    /// P7 slice 1 (R2): `resolved_fields` is keyed on the whole `Span`,
    /// `module` field included. Two files whose projections land on the
    /// identical (line, col) resolve to different structs and different
    /// fields; a bare (line, col) key would collapse them into one entry and
    /// silently lower one of the two against the other's layout.
    #[test]
    fn resolved_fields_key_includes_module() {
        let s = Sandbox::new("resolved-fields-span-module");
        // Both `&n` sites sit at the same line and column in their own file.
        s.write(
            "lib.sth",
            ": show ( -- ) 1 true B &n @ drop drop ;\nexport: show ;\ntype: B tag i64 n bool ;\nexport: B ;\n",
        );
        let entry = s.write(
            "main.sth",
            ": main ( -- ) 7 A      &n @ . drop ;\ntype: A n i64 ;\nimport: \"lib.sth\" l ;\n",
        );
        let closure = discover_closure(&entry).expect("closure resolves");
        let mut module = assemble_module(&closure, true).expect("assembles");
        check::check(&mut module).expect("checks");
        let sites: Vec<(u32, u32, u32, usize)> = module
            .resolved_fields
            .iter()
            .map(|(span, (_, fi))| (span.line, span.col, span.module, *fi))
            .collect();
        assert_eq!(sites.len(), 2, "one entry per file's site: {sites:?}");
        let (l0, c0, _, _) = sites[0];
        let (l1, c1, _, _) = sites[1];
        assert_eq!(
            (l0, c0),
            (l1, c1),
            "the two sites must collide on (line, col) or this proves nothing"
        );
        let mut fields: Vec<usize> = sites.iter().map(|&(_, _, _, fi)| fi).collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            vec![0, 1],
            "each file's `&n` resolved against its own receiver: {sites:?}"
        );
    }
}
