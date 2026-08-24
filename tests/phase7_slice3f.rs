//! Phase 7 Slice 3f goldens: a ground `Type::Quotation` value crossing the
//! polymorphism boundary -- the argument boundary (R1/R2), the body boundary
//! (R3), and the two composing. Negatives land alongside whichever phase's
//! fix they exercise.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3d.rs`'s
/// own pattern). Carries its own `sooth.pkg` naming `core` at this checkout's
/// `lib/` (P7.S3i: `core::bool` is an ordinary package import now, not a
/// compiler injection, so a fixture using `Bool`/`True`/`False` needs a real
/// package tree to resolve it, same as `tests/phase7_slice3i.rs`'s `Tree`).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3f-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sooth.pkg"), common::fixture_package("p7s3f")).unwrap();
        let path = dir.join("prog.sth");
        std::fs::write(&path, contents).unwrap();
        Scratch(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

fn build_and_run(src: &Path) -> (PathBuf, String, i32) {
    let binary = driver::build(src).expect("program should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    (
        binary,
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

/// R1/R2 behavioural: a poly word declaring both a real type variable and a
/// ground `Type::Quotation` parameter, called from a concrete body with a
/// literal quotation argument, run at two distinct instantiations of the
/// variable so it is carried rigidly rather than coincidentally matching.
#[test]
fn argument_boundary_materializes_ground_quotation_param() {
    let src = "import: intrinsics * ;\n\
               import: core::bool * ;\n\
               : run_it ( 'T: Copy [ i64 -- i64 ] -- 'T ) drop ;\n\
               : main ( -- )\n\
                 7 [ 1 add ] run_it .\n\
                 True [ 1 add ] run_it .\n\
               ;\n";
    let prog = Scratch::write("argument-boundary-behavioural", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "7\nTrue\n",
        "each instantiation of `'T` must carry the materialized quotation argument independently"
    );
}

/// R3 behavioural: a poly word declaring a ground `Type::Quotation` parameter
/// and `call`ing it inside its own body -- a real `(code, env)` value, so the
/// body honours the declared effect instead of splicing a literal. Run at two
/// instantiations of its unrelated `'T`, which the quotation never touches.
#[test]
fn body_boundary_calls_ground_quotation_param() {
    let src = "import: intrinsics * ;\n\
               import: core::bool * ;\n\
               : call_it ( 'T: Copy [ i64 -- i64 ] -- 'T i64 )\n\
                 1 swap call\n\
               ;\n\
               : main ( -- )\n\
                 9 [ 1 add ] call_it . .\n\
                 True [ 1 add ] call_it . .\n\
               ;\n";
    let prog = Scratch::write("body-boundary-behavioural", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "2\n9\n2\nTrue\n",
        "the called quotation must run in each instantiation, beside the untouched `'T`"
    );
}

/// R1/R2 and R3 composing: one poly word that both receives a ground
/// `Type::Quotation` argument across the call boundary and `call`s it in its
/// own body. The declared effect takes two inputs, so the body boundary pops
/// more than the single slot the goldens above exercise.
#[test]
fn argument_and_body_boundary_together() {
    let src = "import: intrinsics * ;\n\
               import: core::bool * ;\n\
               : apply_it ( 'T: Copy [ i64 i64 -- i64 ] i64 -- 'T i64 )\n\
                 3 rot call\n\
               ;\n\
               : main ( -- )\n\
                 9 [ add ] 4 apply_it . .\n\
                 True [ add ] 4 apply_it . .\n\
               ;\n";
    let prog = Scratch::write("round-trip-behavioural", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "7\n9\n7\nTrue\n",
        "a two-input declared effect must pop both operands at the body boundary"
    );
}

/// R3's *ordering*: a declared effect whose inputs are heterogeneous, so each
/// operand is pinned to its own declared position rather than merely counted.
/// A golden rather than a unit test because it pins two facts at once -- the
/// checker pops deepest-first, *and* the backend passes the operands in that
/// same order, which a checker-only test cannot see (reverse both and they
/// would agree with each other while silently miscompiling).
#[test]
fn body_boundary_pops_declared_inputs_deepest_first() {
    let src = "import: intrinsics * ;\n\
               import: core::bool * ;\n\
               : call_it ( 'T: Copy [ i64 Bool -- ] -- 'T )\n\
                 1 True rot call\n\
               ;\n\
               : main ( -- )\n\
                 9 [ swap . . ] call_it .\n\
               ;\n";
    let prog = Scratch::write("body-boundary-input-order", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "1\nTrue\n9\n",
        "the deepest operand must satisfy the first declared input, at check time and at run time"
    );
}

/// L1 at the golden level, flipped by P7.S3l (R1/R2): a declared quotation
/// parameter that still carries a free variable inside its brackets is now
/// accepted at the body boundary and lowers end-to-end, mirroring the
/// headline `apply` shape (`: apply ( 'T [ 'T -- 'T ] -- 'T ) call ;`), run
/// at two distinct instantiations of `'T` so it is carried rigidly rather
/// than coincidentally matching. The quotation argument is forwarded through
/// a helper word (`mk_i64`/`mk_bool`) rather than passed as a literal at the
/// `apply` call site: a *literal* quotation at that position hits an
/// unrelated, still-pinned rejection (`check_poly_call`'s R9p guard, which
/// only materializes a literal against a **ground** `Concrete(Type::
/// Quotation)` slot, not this abstract one) -- closing that gap, if it is
/// done at all, is phase 2's concern (recorded as a phase 1 recon finding).
/// Phase 1 also confirmed and closed a second, lower-level gap this golden
/// depends on: `subst_polytype`'s `PolyType::Quotation` lowering arm
/// (`src/ir/driver.rs`) previously asserted this shape could never reach
/// monomorphized lowering and panicked when it did; it now grounds the row
/// through `θ`, mirroring check-side `apply_subst`'s existing arm.
#[test]
fn body_boundary_calls_an_abstract_quotation_param() {
    let src = "import: intrinsics * ;\n\
               import: core::bool * ;\n\
               : mk_i64 ( -- [ i64 -- i64 ] ) [ 1 add ] ;\n\
               : mk_bool ( -- [ Bool -- Bool ] ) [ ] ;\n\
               : apply ( 'T [ 'T -- 'T ] -- 'T ) call ;\n\
               : main ( -- )\n\
                 4 mk_i64 apply .\n\
                 True mk_bool apply .\n\
               ;\n";
    let prog = Scratch::write("body-boundary-abstract-quotation", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "5\nTrue\n",
        "each instantiation of `'T` must call its own forwarded quotation independently"
    );
}

/// P7.S3l phase 2 (R9p closure): the headline shape, with the quotation
/// argument as a *literal* at `apply`'s own call site rather than forwarded
/// out of a helper word's return value -- the gap `body_boundary_calls_an_
/// abstract_quotation_param` (above) recorded as still open at phase 1 exit.
/// `check_poly_call`'s R9p guard now grounds an abstract declared quotation
/// slot through the `subst` already bound by an earlier plain input before
/// materializing the literal, exactly as it already did for a ground
/// declared slot.
#[test]
fn headline_apply_accepts_a_literal_quotation_argument() {
    let src = "import: intrinsics * ;\n\
               : apply ( 'T [ 'T -- 'T ] -- 'T ) call ;\n\
               : main ( -- ) 4 [ 1 add ] apply . ;\n";
    let prog = Scratch::write("headline-apply-literal-argument", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(stdout, "5\n");
}
