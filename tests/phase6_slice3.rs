//! Phase 6 Slice 3 goldens: the generated eliminator word (`Shape?`), source
//! in -> program output out.
//!
//! Both modes are witnesses, and neither subsumes the other: the owning golden
//! is the first program in the language to put a `Type::Variant` value on the
//! stack from surface syntax, while the reference golden is the one that forces
//! `ir_type_of(Type::Variant)` at build time (an arm's declared
//! `&Shape.Circle` interns a referent, and every interned referent is converted
//! whether or not it is ever executed) and the one that shows the call consumes
//! nothing.

mod common;

fn run_example(rel: &str) -> String {
    let binary = common::build_example(rel);
    let out = std::process::Command::new(&binary)
        .output()
        .unwrap_or_else(|e| panic!("running {rel}: {e}"));
    std::fs::remove_file(&binary).ok();
    assert!(
        out.status.success(),
        "{rel} exited {}; stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("program output is utf-8")
}

/// Owning mode: each arm receives the whole narrowed variant, reads a field
/// through a `&field` projection, and consumes it. Two variants with different
/// field counts, so an arm routed to the wrong variant reads the wrong offsets
/// rather than merely the wrong value.
#[test]
fn eliminator_owning_mode_dispatches_to_the_annotated_arm() {
    assert_eq!(run_example("examples/eliminator.sth"), "75\n12\n");
}

/// Reference mode: the arms are annotated `( &Circle )`/`( &!Rect )`, read and
/// write through the narrowed reference, and leave `main`'s own `Shape` intact
/// -- the second `area` reads the value `grow` mutated in place through it.
#[test]
fn eliminator_reference_mode_borrows_without_consuming_the_scrutinee() {
    assert_eq!(run_example("examples/eliminator_ref.sth"), "12\n16\n");
}
