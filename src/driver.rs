//! Pipeline orchestration: the one place that wires the stages together.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{backend, check, ir, lexer, parser};

const C_SHIM: &str = "extern void sooth_main(void);\nint main(void) { sooth_main(); return 0; }\n";

/// The native binary path for a source file: alongside the source, named after its
/// stem (`examples/gcd.sth` -> `examples/gcd`).
fn binary_path(source: &Path) -> PathBuf {
    source.with_extension("")
}

/// Compile a source file to a native binary. Returns the binary's path.
pub fn build(path: &Path) -> Result<PathBuf, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("reading {path:?}: {e}"))?;
    let tokens = lexer::lex(&src)?;
    let module = parser::parse(&tokens)?;
    check::check(&module)?;
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

pub(crate) fn tempfile_dir() -> Result<PathBuf, String> {
    // Each build gets its own scratch dir so concurrent in-process builds (e.g.
    // parallel goldens) don't clobber each other's fixed-name intermediates.
    static N: AtomicU64 = AtomicU64::new(0);
    let seq = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("sooth-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating temp dir {dir:?}: {e}"))?;
    Ok(dir)
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

    #[test]
    fn driver_binary_path_from_source_stem() {
        assert_eq!(
            binary_path(Path::new("examples/gcd.sth")),
            PathBuf::from("examples/gcd")
        );
    }

    #[test]
    fn compile_so_produces_loadable_object() {
        let src = ": sq ( int -- int ) | n | n n * ;";
        let tokens = lexer::lex(src).unwrap();
        let module = parser::parse(&tokens).unwrap();
        check::check(&module).unwrap();
        let ir = ir::lower(&module).unwrap();
        let ssa = backend::qbe::emit(&ir).unwrap();

        let dir = tempfile_dir().unwrap();
        let so = dir.join("libsq.so");
        compile_so(&ssa, &so).expect("compile_so should succeed");
        assert!(so.exists(), "shared object should exist at {so:?}");
    }
}
