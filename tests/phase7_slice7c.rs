//! P7.S7c phase 1: `core::show` -- `StrBuf`, the `Show['T]` trait and its
//! four scalar impls, and the restoring-division digit helper. There is no
//! Rust stage file for a `.sth` module (`tests/phase7_slice7b.rs:1-3` is the
//! precedent statement), so coverage is build-and-run goldens: a scratch
//! program renders a value into a `StrBuf` and reads the bytes/`len` back
//! directly through the still-live `.` intrinsic (no sink yet -- that is
//! Phase 2's `Write for Stdout`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s7c-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
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

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn checkout() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

/// A `core`-layer manifest: `core::show` has no `hosted` dependency (R8).
fn manifest() -> String {
    format!(
        "package: s7c ;\nlayer: core ;\ndepends: core path \"{}/lib/core\" ;\n",
        checkout()
    )
}

fn build_and_run(t: &Tree, main: &str) -> String {
    t.write("sooth.pkg", &manifest());
    let entry = t.write("main.sth", main);
    let binary = driver::build(&entry).expect("the fixture should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("the binary should run");
    std::fs::remove_file(&binary).ok();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

/// `42` rendered through `Show for i64` writes the ASCII digits `4` `2` at
/// `len` `2`, starting from an incoming `len` of `0` (append semantics, R3).
#[test]
fn show_i64_renders_positive_digits() {
    let t = Tree::new("i64-positive");
    let stdout = build_and_run(
        &t,
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: core::show | StrBuf render | ;\n\
         : main ( -- )\n\
         0 >u8 64 fill 0 >usize StrBuf | buf |\n\
         42 &!buf render\n\
         &buf &len @ .\n\
         &buf &data 0 >usize &> @ .\n\
         &buf &data 1 >usize &> @ .\n\
         buf drop ;\n",
    );
    assert_eq!(stdout, "2\n52\n50\n", "len 2, then bytes '4' (52) '2' (50)");
}

/// A negative `i64` prepends `-` (ASCII `45`) ahead of the magnitude's
/// digits (Ruling 6).
#[test]
fn show_i64_negative_prepends_minus() {
    let t = Tree::new("i64-negative");
    let stdout = build_and_run(
        &t,
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: core::show | StrBuf render | ;\n\
         : main ( -- )\n\
         0 >u8 64 fill 0 >usize StrBuf | buf |\n\
         -7 &!buf render\n\
         &buf &len @ .\n\
         &buf &data 0 >usize &> @ .\n\
         &buf &data 1 >usize &> @ .\n\
         buf drop ;\n",
    );
    assert_eq!(stdout, "2\n45\n55\n", "len 2, then '-' (45) '7' (55)");
}

/// `i64::MIN`'s magnitude does not fit back into an `i64`; Ruling 6's
/// `>u64 not 1 >u64 add` two's-complement-negation-over-the-bit-pattern is
/// exact for it regardless: 19 magnitude digits plus the sign is 20 bytes.
#[test]
fn show_i64_min_magnitude_is_exact() {
    let t = Tree::new("i64-min");
    let stdout = build_and_run(
        &t,
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: core::show | StrBuf render | ;\n\
         : main ( -- )\n\
         0 >u8 64 fill 0 >usize StrBuf | buf |\n\
         -9223372036854775808 &!buf render\n\
         &buf &len @ .\n\
         buf drop ;\n",
    );
    assert_eq!(stdout, "20\n", "1 sign byte + 19 magnitude digits");
}

/// `Bool` renders `true`/`false` at their respective lengths, both arms.
#[test]
fn show_bool_renders_both_arms() {
    let t = Tree::new("bool-both");
    let stdout = build_and_run(
        &t,
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: core::show | StrBuf render | ;\n\
         : main ( -- )\n\
         0 >u8 64 fill 0 >usize StrBuf | t |\n\
         True &!t render\n\
         &t &len @ .\n\
         0 >u8 64 fill 0 >usize StrBuf | f |\n\
         False &!f render\n\
         &f &len @ .\n\
         t drop f drop ;\n",
    );
    assert_eq!(stdout, "4\n5\n", "\"true\" is 4 bytes, \"false\" is 5");
}

/// `usize`/`isize` share the same digit path as `i64`/its magnitude.
#[test]
fn show_usize_and_isize_render_digits() {
    let t = Tree::new("usize-isize");
    let stdout = build_and_run(
        &t,
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: core::show | StrBuf render | ;\n\
         : main ( -- )\n\
         0 >u8 64 fill 0 >usize StrBuf | u |\n\
         123 >usize &!u render\n\
         &u &len @ .\n\
         0 >u8 64 fill 0 >usize StrBuf | i |\n\
         -123 >isize &!i render\n\
         &i &len @ .\n\
         u drop i drop ;\n",
    );
    assert_eq!(stdout, "3\n4\n", "\"123\" is 3 bytes, \"-123\" is 4");
}

/// R11: several renders appended into one buffer eventually drive the store
/// index past capacity; the overflow is discarded and `len` clamps at 64,
/// never above it, with no out-of-bounds store.
#[test]
fn show_overflow_clamps_len_at_capacity() {
    let t = Tree::new("overflow-clamp");
    let stdout = build_and_run(
        &t,
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: core::show | StrBuf render | ;\n\
         : main ( -- )\n\
         0 >u8 64 fill 0 >usize StrBuf | buf |\n\
         -9223372036854775808 &!buf render\n\
         -9223372036854775808 &!buf render\n\
         -9223372036854775808 &!buf render\n\
         -9223372036854775808 &!buf render\n\
         &buf &len @ .\n\
         buf drop ;\n",
    );
    assert_eq!(
        stdout, "64\n",
        "4 * 20 = 80 magnitude+sign bytes clamp to the 64-byte capacity"
    );
}

/// A `hosted`-layer manifest: `hosted::libc`'s `Stdout` sink depends on
/// `core::show`, so the Phase 2 dogfood needs both layers on the
/// dependency path.
fn hosted_manifest() -> String {
    format!(
        "package: s7c ;\nlayer: hosted ;\ndepends: core path \"{0}/lib/core\" ;\ndepends: hosted path \"{0}/lib/hosted\" ;\n",
        checkout()
    )
}

fn build_and_run_hosted(t: &Tree, main: &str) -> String {
    t.write("sooth.pkg", &hosted_manifest());
    let entry = t.write("main.sth", main);
    let binary = driver::build(&entry).expect("the fixture should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("the binary should run");
    std::fs::remove_file(&binary).ok();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

/// P7.S7c Phase 2 dogfood (R7): renders `Show` at two instantiations (`i64`
/// and `Bool`), flushes each through `Stdout`'s `write(2)` sink, and pins
/// the exact flushed bytes. Ownership is explicit: each buffer and the sink
/// are constructed, used, and dropped (the linear spine made visible). Only
/// one output channel (`write(2)`) appears, so there is no `.`-vs-write(2)
/// interleaving to assert (R9).
#[test]
fn stdout_flush_renders_two_instantiations() {
    let t = Tree::new("stdout-flush");
    let stdout = build_and_run_hosted(
        &t,
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: core::show | StrBuf render flush | ;\n\
         import: hosted::libc | Stdout | ;\n\
         : main ( -- )\n\
         0 >u8 64 fill 0 >usize StrBuf | n |\n\
         42 &!n render\n\
         Stdout | s1 |\n\
         &!s1 &!n flush\n\
         s1 drop n drop\n\
         0 >u8 64 fill 0 >usize StrBuf | b |\n\
         True &!b render\n\
         Stdout | s2 |\n\
         &!s2 &!b flush\n\
         s2 drop b drop ;\n",
    );
    assert_eq!(
        stdout, "42true",
        "42's digits flush first, then Bool's true, both via write(2)"
    );
}
