//! Phase 7 Slice 1 goldens: field access as a mode-carrying projection word
//! (`&hp` / `&!hp`) resolved against the receiver's type, source in -> program
//! output out.
//!
//! Phase 1 introduces `&f` alongside the fused `Type>f` spelling; the fused one
//! goes in phase 5, so nothing here asserts its absence.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let seq = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("sooth-p7s1-{}-{tag}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating the scratch dir should succeed");
    dir
}

fn build_and_run(entry: &Path) -> String {
    let binary = sooth::driver::build_with_manifest(entry, common::manifest_for(entry).as_deref())
        .expect("build should succeed");
    let out = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    let dir = entry.parent().expect("the entry sits in a scratch dir");
    std::fs::remove_dir_all(dir).ok();
    assert!(
        out.status.success(),
        "program exited with {:?}; stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout should be utf8")
}

fn run_program(tag: &str, src: &str) -> String {
    let dir = scratch(tag);
    let entry = dir.join("main.sth");
    common::write_fixture(&entry, src).expect("writing the entry should succeed");
    build_and_run(&entry)
}

/// D2/R6: reading and writing fields through projections, on both receiver
/// shapes. The owned receiver stays on the stack across two projections; the
/// reference receiver is consumed by each.
#[test]
fn projections_read_and_write_struct_fields() {
    let out = run_program(
        "read-write",
        "type: Point x i64 y i64 ;\n\
         : main ( -- )\n  \
           1 2 Point\n  \
           &x @ .\n  \
           &y @ .\n  \
           &!x 30 !\n  \
           &!y 40 !\n  \
           &x @ .\n  \
           &y @ .\n  \
           | p |\n  \
           &p &x @ .\n  \
           &!p &!y 50 !\n  \
           &p &y @ .\n  \
           p drop ;\n",
    );
    assert_eq!(out, "1\n2\n30\n40\n30\n50\n");
}

/// D2's chaining case: a projection out of a projection. Each step consumes
/// its reference, so the chain leaves exactly one value.
#[test]
fn nested_projection_chain_runs() {
    let out = run_program(
        "nested-chain",
        "type: Stats hp i64 mp i64 ;\n\
         type: Unit tag i64 stats Stats ;\n\
         : main ( -- )\n  \
           7 100 20 Stats Unit | u |\n  \
           &u &stats &hp @ .\n  \
           &u &stats &mp @ .\n  \
           &!u &!stats &!hp 99 !\n  \
           &u &stats &hp @ .\n  \
           &u &tag @ .\n  \
           u drop ;\n",
    );
    assert_eq!(out, "100\n20\n99\n7\n");
}

/// R1/R2: one spelling, two receivers. `&tag` resolves against the receiver's
/// own instantiation, so the two generic applications read two different
/// layouts. Deliberately asymmetric: the two type arguments have *different
/// sizes*, so `tag` sits at a different offset in each. A symmetric pair
/// (`Box[i64]` against `Box[Bool]`) lays both instantiations out identically
/// and could not tell a correct resolution from a swapped one.
#[test]
fn projection_resolves_per_instantiation() {
    let out = run_program(
        "per-instantiation",
        "type: S1 a i64 ;\n\
         type: S2 a i64 b i64 ;\n\
         type: Box 'T val 'T tag i64 ;\n\
         : show1 ( Box[S1] -- ) &tag @ . drop ;\n\
         : show2 ( Box[S2] -- ) &tag @ . drop ;\n\
         : main ( -- )\n  \
           1 S1 11 Box show1\n  \
           2 3 S2 22 Box show2 ;\n",
    );
    assert_eq!(out, "11\n22\n");
}

/// A projection written inside a callee's body, against a `&`/`&!` parameter,
/// resolves the same field as one written inline in the caller: `getx`/`bump`
/// and `main`'s own `&p &y @` agree on `Point`'s layout.
///
/// Migrated off the REPL's bare-line/word-definition split (P7.S1's R5, "the
/// REPL path", which goes vacuous when the REPL does). It does not witness
/// "every lowering path": with the REPL gone there is one, `build`, and the
/// inline half is already covered by `projections_read_and_write_struct_fields`
/// above.
#[test]
fn projection_in_a_called_word_body_matches_an_inline_one() {
    let out = run_program(
        "callee-body-projection",
        "type: Point x i64 y i64 ;\n\
         : getx ( &Point -- i64 ) &x @ ;\n\
         : bump ( &!Point -- ) &!x 1 +! ;\n\
         : main ( -- )\n  \
           1 2 Point | p |\n  \
           &p &y @ .\n  \
           &!p &!y 9 !\n  \
           &p &y @ .\n  \
           &p getx .\n  \
           &!p bump\n  \
           &p getx .\n  \
           p drop ;\n",
    );
    assert_eq!(out, "2\n9\n1\n2\n");
}

/// Review fix: the owned-receiver arm's output has a region (`Slot.alias`)
/// but no `Deriv`, so nothing protected its receiver from being consumed
/// while the projection was still live -- reachable even through a chain of
/// reference-narrowing words (here `&^`, unwrapping the owning cell `data`
/// projects into) that don't otherwise touch region tracking. Left
/// unguarded, this compiles and reads through a dangling reference into a
/// freed heap block.
#[test]
fn drop_of_owned_receiver_through_a_reference_chain_while_projected_is_diagnostic() {
    let src = "type: Buf data ^[u8 4] len usize ;\n\
               : mk ( -- Buf ) 0 >u8 4 fill ^ 0 >usize Buf ;\n\
               : main ( -- ) mk &data &^ swap drop 0 >usize &> @ >i64 . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-p7s1-uaf-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let err = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail its check");
    std::fs::remove_file(&path).ok();
    assert!(
        err.contains("`drop` consumes a value while a reference derived from it is still live"),
        "unexpected message: {err}"
    );
}
