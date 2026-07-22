//! Pipeline orchestration: the one place that wires the stages together.

use std::path::{Path, PathBuf};
use std::process::Command;

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

    run_command(
        Command::new("/usr/bin/qbe")
            .arg(&ssa_path)
            .arg("-o")
            .arg(&asm_path),
    )?;

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

pub fn run(path: &Path) -> Result<(), String> {
    let binary = build(path)?;
    let status = Command::new(&binary)
        .status()
        .map_err(|e| format!("running {binary:?}: {e}"))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

pub fn repl() -> Result<(), String> {
    Err("repl: not implemented (Phase 1)".into())
}

fn tempfile_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("sooth-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating temp dir {dir:?}: {e}"))?;
    Ok(dir)
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

    #[test]
    fn driver_binary_path_from_source_stem() {
        assert_eq!(
            binary_path(Path::new("examples/gcd.sth")),
            PathBuf::from("examples/gcd")
        );
    }
}
