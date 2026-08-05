//! Pipeline orchestration: the one place that wires the stages together.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ast::{Import, Module, ModuleInfo, Span};
use crate::lexer::Token;
use crate::{backend, check, ir, lexer, parser, resolve};

const C_SHIM: &str = "extern void sooth_main(void);\nint main(void) { sooth_main(); return 0; }\n";

/// The native binary path for a source file: alongside the source, named after its
/// stem (`examples/gcd.sth` -> `examples/gcd`).
fn binary_path(source: &Path) -> PathBuf {
    source.with_extension("")
}

/// One file in the import closure (R2). `import_targets` is aligned with
/// `imports`: the module id the qualifier at that position binds to.
struct FileNode {
    canon: PathBuf,
    dir: PathBuf,
    tokens: Vec<(Token, Span)>,
    imports: Vec<Import>,
    import_targets: Vec<u32>,
}

/// The whole import closure, one `FileNode` per file, indexed by module id
/// (entry is id 0, R10).
struct Closure {
    nodes: Vec<FileNode>,
}

/// Read, lex, and scan one file's imports into a `FileNode` (R2). The path is
/// already canonical, so its parent is a stable directory to resolve this
/// file's own relative imports against.
fn make_node(canon: PathBuf) -> Result<FileNode, String> {
    let src = std::fs::read_to_string(&canon).map_err(|e| format!("reading {canon:?}: {e}"))?;
    let tokens = lexer::lex(&src)?;
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

/// R5: a resolved import path that does not exist or cannot be read is an error
/// naming the importing site (the `import:` line/col) and the path, distinct
/// from a lex/parse error on the target file.
fn missing_import_error(importer: &Path, imp: &Import) -> String {
    format!(
        "error: cannot read import `{}` at line {}, col {} (imported by {})",
        imp.path,
        imp.span.line,
        imp.span.col,
        importer.display()
    )
}

/// R2/R3: discover the whole import closure from the entry file. Resolves each
/// import relative to the importing file's directory, canonicalizes, and
/// dedupes by canonical path (a diamond imports a file once). Rejects a cycle
/// and a self-import with a located both-files error (R4) and a missing file
/// with a located error (R5).
fn discover_closure(entry: &Path) -> Result<Closure, String> {
    let entry_canon =
        std::fs::canonicalize(entry).map_err(|e| format!("reading {}: {e}", entry.display()))?;
    let mut id_of: HashMap<PathBuf, u32> = HashMap::new();
    id_of.insert(entry_canon.clone(), 0);
    let mut nodes: Vec<FileNode> = vec![make_node(entry_canon)?];

    let mut i = 0;
    while i < nodes.len() {
        let dir = nodes[i].dir.clone();
        let imports = nodes[i].imports.clone();
        let mut targets = Vec::with_capacity(imports.len());
        for imp in &imports {
            let raw = dir.join(&imp.path);
            let canon = std::fs::canonicalize(&raw)
                .map_err(|_| missing_import_error(&nodes[i].canon, imp))?;
            let id = match id_of.get(&canon) {
                Some(&id) => id,
                None => {
                    let id = nodes.len() as u32;
                    id_of.insert(canon.clone(), id);
                    nodes.push(make_node(canon)?);
                    id
                }
            };
            targets.push(id);
        }
        nodes[i].import_targets = targets;
        i += 1;
    }

    let closure = Closure { nodes };
    closure.reject_cycles()?;
    Ok(closure)
}

impl Closure {
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
        for &v in &self.nodes[u].import_targets {
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

/// R3/R11: assemble the discovered closure into one `Module`. Runs the shared
/// type pre-pass across every file into one merged registry, parses each file's
/// bodies module-aware against that shared registry, then hands the merged
/// module to the resolver to mangle same-named decls apart (a no-op for a
/// single-file closure, R22).
fn assemble_module(closure: &Closure) -> Result<Module, String> {
    let mut structs = Vec::new();
    let mut enums = Vec::new();
    let mut struct_base = Vec::with_capacity(closure.nodes.len());
    let mut enum_base = Vec::with_capacity(closure.nodes.len());
    for (m, node) in closure.nodes.iter().enumerate() {
        struct_base.push(structs.len());
        enum_base.push(enums.len());
        parser::prepass_and_register(&node.tokens, m as u32, &mut structs, &mut enums)?;
    }

    let mut arrays = Vec::new();
    let mut owned_cells = Vec::new();
    let mut refs = Vec::new();
    let mut words = Vec::new();
    let mut externs = Vec::new();
    let mut modules = Vec::with_capacity(closure.nodes.len());
    for (m, node) in closure.nodes.iter().enumerate() {
        let mut import_map: HashMap<String, u32> = HashMap::new();
        for (imp, &target) in node.imports.iter().zip(&node.import_targets) {
            import_map.insert(imp.qualifier.clone(), target);
        }
        let bodies = parser::parse_bodies(
            &node.tokens,
            &structs,
            &enums,
            m as u32,
            &import_map,
            &mut arrays,
            &mut owned_cells,
            &mut refs,
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
        modules.push(ModuleInfo {
            imports: import_map,
            exports: bodies.exports,
        });
    }

    let mut module = Module {
        words,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        externs,
        instantiations: HashMap::new(),
        modules,
    };
    resolve::resolve_modules(&mut module);
    Ok(module)
}

/// Compile a source file to a native binary. Returns the binary's path.
pub fn build(path: &Path) -> Result<PathBuf, String> {
    let closure = discover_closure(path)?;
    let mut module = assemble_module(&closure)?;
    check::check(&mut module)?;
    let ir = ir::lower(&module)?;
    let ssa = backend::qbe::emit(&ir)?;

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
    let binary = build(path)?;
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
            "import: base \"base.sth\" ;\n: lf ( -- i64 ) base::b ;\n",
        );
        s.write(
            "right.sth",
            "import: base \"base.sth\" ;\n: rt ( -- i64 ) base::b ;\n",
        );
        let entry = s.write(
            "main.sth",
            "import: l \"left.sth\" ;\nimport: r \"right.sth\" ;\n: main ( -- ) 0 . ;\n",
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
        s.write("a.sth", "import: b \"b.sth\" ;\n: main ( -- ) 0 . ;\n");
        s.write("b.sth", "import: a \"a.sth\" ;\n: q ( -- i64 ) 1 ;\n");
        let entry = s.0.join("a.sth");
        let err = match discover_closure(&entry) {
            Ok(_) => panic!("a cycle must be rejected"),
            Err(e) => e,
        };
        assert!(err.contains("cycle"), "names the failure: {err}");
        assert!(err.contains("a.sth"), "names the first file: {err}");
        assert!(err.contains("b.sth"), "names the second file: {err}");
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
        let src = ": sq ( i64 -- i64 ) | n | n n * ;";
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
}
