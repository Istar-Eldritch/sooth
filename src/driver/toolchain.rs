//! The native toolchain half of the driver: turning emitted QBE IL into a
//! binary or a shared object (`qbe`, `cc`), running one, and the `build` /
//! `run` / `test` subcommands over that plumbing. Its input is
//! `super::emit_ssa_with_manifest`'s output -- nothing here knows how a source
//! file becomes a checked `Module`.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};

use super::emit_ssa_with_manifest;
use crate::packages;

const C_SHIM: &str = "extern void sooth_main(void);\nint main(void) { sooth_main(); return 0; }\n";

/// The native binary path for a source file: alongside the source, named after its
/// stem (`examples/gcd.sth` -> `examples/gcd`).
fn binary_path(source: &Path) -> PathBuf {
    source.with_extension("")
}

/// Compile a source file to a native binary. Returns the binary's path.
pub fn build(path: &Path) -> Result<PathBuf, String> {
    build_with_manifest(path, None)
}

/// `build` with a `--manifest` override; see `emit_ssa_with_manifest`.
pub fn build_with_manifest(path: &Path, manifest: Option<&Path>) -> Result<PathBuf, String> {
    let out_path = binary_path(path);
    build_into(path, &out_path, manifest)?;
    Ok(out_path)
}

/// `build_with_manifest`, with the output binary's path taken explicitly rather
/// than fixed at `binary_path(source)` (R4.3): `build`/`run` still land beside
/// the source (their callers keep passing `binary_path(source)`), while
/// `test` targets a temp path so no binary ever lands in the tree.
fn build_into(path: &Path, out: &Path, manifest: Option<&Path>) -> Result<(), String> {
    let ssa = emit_ssa_with_manifest(path, manifest)?;
    link_shimmed_binary(&ssa, out)
}

/// The qbe/cc plumbing shared by every native-binary output: compile the SSA
/// to assembly, link it with the C shim (`sooth_main` -> `main`) into `out`.
fn link_shimmed_binary(ssa: &str, out: &Path) -> Result<(), String> {
    let dir = tempfile_dir()?;
    let ssa_path = dir.join("out.ssa");
    let asm_path = dir.join("out.s");
    let shim_path = dir.join("shim.c");
    std::fs::write(&ssa_path, ssa).map_err(|e| format!("writing {ssa_path:?}: {e}"))?;
    std::fs::write(&shim_path, C_SHIM).map_err(|e| format!("writing {shim_path:?}: {e}"))?;

    run_command(Command::new("qbe").arg(&ssa_path).arg("-o").arg(&asm_path))?;

    run_command(
        Command::new("cc")
            .arg(&asm_path)
            .arg(&shim_path)
            .arg("-o")
            .arg(out),
    )?;

    Ok(())
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

/// Every `*.sth` directly under `dir` (non-recursive), sorted for determinism.
fn collect_sth_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("reading {dir:?}: {e}"))?;
    let mut out = Vec::new();
    for entry in read {
        let entry = entry.map_err(|e| format!("reading {dir:?}: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("sth") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Resolve `sooth test`'s entry set (R3). No `paths` resolves the package
/// containing `cwd` and takes every `*.sth` under its `tests/` directory
/// (R3.1): `find_package_root` walks from a *file's* parent, so `cwd` is
/// joined with a nonexistent sentinel component first, making the walk's
/// first step `cwd` itself rather than skipping past it. Given `paths`, each
/// named file is an entry and each named directory contributes every
/// `*.sth` directly under it (R3.2, non-recursive).
pub fn discover_test_entries(cwd: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() {
        let root = packages::find_package_root(&cwd.join("_"))
            .ok_or_else(|| format!("no sooth.pkg found at or above {}", cwd.display()))?;
        let tests_dir = root.join("tests");
        if !tests_dir.is_dir() {
            return Err(format!("no tests directory at {}", tests_dir.display()));
        }
        let entries = collect_sth_files(&tests_dir)?;
        if entries.is_empty() {
            return Err(format!("{} contains no *.sth files", tests_dir.display()));
        }
        Ok(entries)
    } else {
        let mut entries = Vec::new();
        for p in paths {
            if p.is_dir() {
                entries.extend(collect_sth_files(p)?);
            } else {
                entries.push(p.clone());
            }
        }
        entries.sort();
        Ok(entries)
    }
}

/// Count R1 protocol lines in a captured stdout: `(ok, not_ok)`. `not ok` is
/// classified before `ok` so a `not ok -- ...` line is never miscounted as a
/// pass by a substring match on `"ok"`.
fn count_protocol(stdout: &str) -> (usize, usize) {
    let mut ok = 0usize;
    let mut not_ok = 0usize;
    for line in stdout.lines() {
        if line.starts_with("not ok -- ") {
            not_ok += 1;
        } else if line.starts_with("ok -- ") {
            ok += 1;
        }
    }
    (ok, not_ok)
}

/// `sooth test`: discover entries (R3), build each into its own temp dir and
/// run it (R4), count R1 protocol lines, and write a summary. Returns the
/// process exit code: 0 iff every entry passed. An entry fails if any `not
/// ok` line appears, the child exits non-zero, or its build fails (R1).
///
/// The per-entry verdicts and the summary go to `report`; the *diagnostic*
/// text behind a failure -- a build's compiler errors, a failing entry's own
/// `not ok -- <label>` lines, a failed child's own stderr -- goes to
/// `diagnostics`, keeping R1.1's build errors and R1's failing labels on
/// their own channel and out of the run report. The CLI passes stdout and
/// stderr.
pub fn test(
    cwd: &Path,
    paths: &[PathBuf],
    report: &mut dyn Write,
    diagnostics: &mut dyn Write,
) -> Result<i32, String> {
    let entries = discover_test_entries(cwd, paths)?;

    let mut total_ok = 0usize;
    let mut total_not_ok = 0usize;
    let mut failed = 0usize;

    for entry in &entries {
        let dir = tempfile_dir()?;
        let binary = dir.join("test");
        let outcome = match build_into(entry, &binary, None) {
            Err(e) => {
                write_diagnostic(diagnostics, &format!("{e}\n"))?;
                Err("build failed".to_string())
            }
            Ok(()) => match Command::new(&binary).output() {
                Err(e) => Err(format!("running {binary:?}: {e}")),
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let (ok, not_ok) = count_protocol(&stdout);
                    total_ok += ok;
                    total_not_ok += not_ok;
                    if not_ok > 0 || !output.status.success() {
                        for line in stdout.lines().filter(|l| l.starts_with("not ok -- ")) {
                            write_diagnostic(diagnostics, &format!("{line}\n"))?;
                        }
                        write_diagnostic(diagnostics, &String::from_utf8_lossy(&output.stderr))?;
                        Err(format!("{ok} ok, {not_ok} not ok, {}", output.status))
                    } else {
                        Ok(())
                    }
                }
            },
        };
        let line = match outcome {
            Ok(()) => format!("ok   {}", entry.display()),
            Err(e) => {
                failed += 1;
                format!("FAIL {} -- {e}", entry.display())
            }
        };
        write_report(report, &line)?;
    }

    write_report(
        report,
        &format!(
            "{} entries, {failed} failed ({total_ok} ok, {total_not_ok} not ok assertions)",
            entries.len(),
        ),
    )?;

    Ok(if failed == 0 { 0 } else { 1 })
}

fn write_report(report: &mut dyn Write, line: &str) -> Result<(), String> {
    writeln!(report, "{line}").map_err(|e| format!("writing the test report: {e}"))
}

/// Diagnostic text is forwarded verbatim, newlines and all: a compiler error
/// and a trapping child's own message are already formatted for a terminal.
fn write_diagnostic(diagnostics: &mut dyn Write, text: &str) -> Result<(), String> {
    write!(diagnostics, "{text}").map_err(|e| format!("writing a test diagnostic: {e}"))
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
    // them (printf and friends) to resolve at load under RTLD_GLOBAL.
    if cfg!(target_os = "macos") {
        cc.arg("-Wl,-undefined,dynamic_lookup");
    }
    run_command(&mut cc)?;
    Ok(())
}

// RTLD_NOW is 2 on both Linux and macOS; RTLD_GLOBAL's value differs.
const RTLD_NOW: c_int = 2;
#[cfg(target_os = "linux")]
const RTLD_GLOBAL: c_int = 0x100;
#[cfg(target_os = "macos")]
const RTLD_GLOBAL: c_int = 0x8;

extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
}

/// A loaded shared object, kept resident (never `dlclose`) so its exports stay
/// callable for as long as the caller holds it.
pub struct Library {
    handle: *mut c_void,
}

impl Library {
    /// Open a shared object with global visibility, so its exports resolve for
    /// objects loaded later.
    pub fn open(path: &Path) -> Result<Library, String> {
        let cpath = CString::new(path.as_os_str().as_bytes())
            .map_err(|e| format!("path has interior nul: {e}"))?;
        // SAFETY: cpath is a valid nul-terminated C string for the call's duration.
        let handle = unsafe {
            dlerror(); // clear any stale error
            dlopen(cpath.as_ptr(), RTLD_NOW | RTLD_GLOBAL)
        };
        if handle.is_null() {
            return Err(format!("dlopen {path:?} failed: {}", last_dlerror()));
        }
        Ok(Library { handle })
    }

    /// Resolve an exported symbol to a raw pointer (caller transmutes to a fn).
    pub fn symbol(&self, name: &str) -> Result<*mut c_void, String> {
        let cname = CString::new(name).map_err(|e| format!("symbol has interior nul: {e}"))?;
        // SAFETY: handle came from a successful dlopen; cname is nul-terminated.
        let sym = unsafe {
            dlerror();
            dlsym(self.handle, cname.as_ptr())
        };
        if sym.is_null() {
            return Err(format!("dlsym {name:?} failed: {}", last_dlerror()));
        }
        Ok(sym)
    }
}

fn last_dlerror() -> String {
    // SAFETY: dlerror returns either null or a valid C string owned by libdl.
    unsafe {
        let p = dlerror();
        if p.is_null() {
            "unknown error".to_string()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// A build's scratch directory, removed when dropped. Every current caller writes
/// into it and is done (qbe/cc have already produced their output elsewhere, or
/// `dlopen` has already read the `.so`) by the time its function returns, so
/// scope-end `Drop` is always ordered after last use, not a race against it.
struct TempDir(PathBuf);

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

fn tempfile_dir() -> Result<TempDir, String> {
    // Each build gets its own scratch dir so concurrent in-process builds (e.g.
    // parallel goldens) don't clobber each other's fixed-name intermediates.
    static N: AtomicU64 = AtomicU64::new(0);
    let seq = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("sooth-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating temp dir {dir:?}: {e}"))?;
    Ok(TempDir(dir))
}

fn run_command(cmd: &mut Command) -> Result<(), String> {
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

    use crate::{backend, check, ir, lexer, parser};

    #[test]
    fn driver_binary_path_from_source_stem() {
        assert_eq!(
            binary_path(Path::new("examples/gcd.sth")),
            PathBuf::from("examples/gcd")
        );
    }

    #[test]
    fn compile_so_produces_loadable_object() {
        let src = ": sq ( i64 -- i64 ) | n | n n mul ;\nimport: intrinsics * ;\n";
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

    #[test]
    fn library_opens_and_resolves_a_compiled_symbol() {
        let src = ": sq ( i64 -- i64 ) | n | n n mul ;\nimport: intrinsics * ;\n";
        let tokens = lexer::lex(src).unwrap();
        let mut module = parser::parse(&tokens).unwrap();
        check::check(&mut module).unwrap();
        let ir = ir::lower(&module).unwrap();
        let ssa = backend::qbe::emit(&ir).unwrap();

        let dir = tempfile_dir().unwrap();
        let so = dir.join("libsq.so");
        compile_so(&ssa, &so).expect("compile_so should succeed");

        let lib = Library::open(&so).expect("dlopen should succeed");
        let sym = lib.symbol("sq").expect("exported symbol should resolve");
        // SAFETY: `sq` was emitted as `export function l $sq(l %v0)`, i.e. a
        // C-ABI `l`-taking, `l`-returning function on this 64-bit target.
        let sq: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(sym) };
        assert_eq!(sq(5), 25);
        assert!(
            lib.symbol("no_such_symbol").is_err(),
            "a bad symbol name should error"
        );
    }

    #[test]
    fn count_protocol_counts_ok_and_not_ok_separately() {
        let stdout = "ok -- a\nnot ok -- b\nok -- c\n";
        assert_eq!(count_protocol(stdout), (2, 1));
    }

    /// R1's ordering hazard: a naive substring match on `"ok"` (rather than a
    /// `not ok -- ` / `ok -- ` prefix match) would count a `not ok` line as a
    /// pass too.
    #[test]
    fn count_protocol_does_not_miscount_not_ok_as_ok() {
        assert_eq!(count_protocol("not ok -- x\n"), (0, 1));
    }

    #[test]
    fn count_protocol_ignores_non_protocol_lines() {
        let stdout = "ok -- a\nsome other line\nnot ok -- b\n\n";
        assert_eq!(count_protocol(stdout), (1, 1));
    }

    /// A scratch package tree for the discovery unit tests, removed on drop.
    struct PkgTree(PathBuf);
    impl PkgTree {
        fn new(tag: &str) -> PkgTree {
            static N: AtomicU64 = AtomicU64::new(0);
            let seq = N.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "sooth-test-discovery-{}-{tag}-{seq}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            PkgTree(dir)
        }
        fn write(&self, rel: &str, contents: &str) -> PathBuf {
            let path = self.0.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, contents).unwrap();
            path
        }
    }
    impl Drop for PkgTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discover_test_entries_no_path_reads_pkgroot_tests_dir() {
        let t = PkgTree::new("no-path");
        t.write("sooth.pkg", "package: p ; layer: hosted ;");
        t.write("tests/a.sth", "");
        t.write("tests/b.sth", "");
        let entries = discover_test_entries(&t.0, &[]).expect("discovery should succeed");
        assert_eq!(
            entries,
            vec![t.0.join("tests/a.sth"), t.0.join("tests/b.sth")]
        );
    }

    #[test]
    fn discover_test_entries_explicit_file_and_dir() {
        let t = PkgTree::new("explicit");
        t.write("sooth.pkg", "package: p ; layer: hosted ;");
        let file = t.write("tests/a.sth", "");
        t.write("other/b.sth", "");
        t.write("other/c.sth", "");
        let entries = discover_test_entries(&t.0, &[file.clone(), t.0.join("other")])
            .expect("discovery should succeed");
        assert_eq!(
            entries,
            vec![t.0.join("other/b.sth"), t.0.join("other/c.sth"), file]
        );
    }

    #[test]
    fn discover_test_entries_no_ancestor_pkg_is_error() {
        let t = PkgTree::new("no-pkg");
        let err = discover_test_entries(&t.0, &[]).expect_err("should be an error");
        assert!(err.contains("no sooth.pkg found"), "got: {err}");
    }

    #[test]
    fn discover_test_entries_missing_tests_dir_is_error() {
        let t = PkgTree::new("missing-tests");
        t.write("sooth.pkg", "package: p ; layer: hosted ;");
        let err = discover_test_entries(&t.0, &[]).expect_err("should be an error");
        assert!(err.contains("no tests directory"), "got: {err}");
    }

    #[test]
    fn discover_test_entries_empty_tests_dir_is_error() {
        let t = PkgTree::new("empty-tests");
        t.write("sooth.pkg", "package: p ; layer: hosted ;");
        std::fs::create_dir_all(t.0.join("tests")).unwrap();
        let err = discover_test_entries(&t.0, &[]).expect_err("should be an error");
        assert!(err.contains("contains no *.sth files"), "got: {err}");
    }

    /// R3.1: `find_package_root`'s ancestor walk must resolve the package
    /// root from a *subdirectory* of it, not only from the root itself --
    /// every other discovery test puts `sooth.pkg` directly at `cwd`.
    #[test]
    fn discover_test_entries_from_subdirectory_walks_up_to_pkgroot() {
        let t = PkgTree::new("subdir");
        t.write("sooth.pkg", "package: p ; layer: hosted ;");
        t.write("tests/a.sth", "");
        let sub = t.0.join("src");
        std::fs::create_dir_all(&sub).unwrap();
        let entries = discover_test_entries(&sub, &[]).expect("discovery should succeed");
        assert_eq!(entries, vec![t.0.join("tests/a.sth")]);
    }
}
