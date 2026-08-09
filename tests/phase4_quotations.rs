//! Phase 4 Slice 7a/7b goldens: a quotation literal reaching a materialization
//! boundary (a struct field, an array element, a word output, or a branch
//! join) becomes a runtime `(code, env)` value, and `call`/`times` on an
//! erased quotation whose identity the checker cannot resolve emits an indirect
//! call. 7b (capturing closures): a captured *scalar* is snapshotted into the
//! env and admissible at every boundary; an *outer-rooted* reference (a `&T`
//! parameter) is admitted; a *frame-rooted* capture escaping the frame is a
//! past-owning-frame error; a captured quotation-typed name and a 2+-capture
//! escaping closure are deferred. A capturing literal at a direct `call` still
//! splices, unaffected. The branch join materializes each arm against the
//! enclosing declared output row (R11); the same literal in both arms stays a
//! splice.

use sooth::ast::WordBody;
use sooth::ir::{Instr, IrType};
use sooth::{check, lexer, parser};

mod common;

/// Compile and run `src`, returning stdout and the exit code. `name`
/// distinguishes the temp source per test (the goldens run in parallel).
fn run_src(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

/// Whether the lowered module emits at least one indirect call: the witness
/// that a `call` resolved to a runtime dispatch through a materialized value,
/// not a compile-time body splice.
fn emits_call_indirect(src: &str) -> bool {
    count_call_indirect(src) > 0
}

/// The total number of indirect calls in the lowered module: `times` over an
/// erased quotation must emit exactly one (in the loop body, not once per
/// iteration).
fn count_call_indirect(src: &str) -> usize {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = sooth::ir::lower(&module).expect("lower should succeed");
    ir.funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::CallIndirect(..)))
        .count()
}

/// The param list of the sole materialized quotation func (its symbol carries
/// `__quot`), for the 7b env-parameter shape assertions.
fn materialized_quot_params(src: &str) -> Vec<IrType> {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = sooth::ir::lower(&module).expect("lower should succeed");
    let mut mats: Vec<&sooth::ir::IrFunc> = ir
        .funcs
        .iter()
        .filter(|f| f.name.contains("__quot"))
        .collect();
    assert_eq!(mats.len(), 1, "expected exactly one materialized quotation");
    mats.remove(0).params.clone()
}

// -- T-field: a quotation stored in a struct field, called back out ----------

#[test]
fn quotation_stored_in_struct_field_compiles_and_calls() {
    let src = "type: Holder q [ i64 -- i64 ] ;\n\
               : main ( -- ) [ 1 + ] Holder Holder>q 4 swap call . ;\n";
    let (stdout, code) = run_src("qfield", src);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "the store/getter path must lower `call` to an indirect call"
    );
}

// -- T-array: two quotations in an array, each indirect-called ---------------

#[test]
fn quotation_in_array_element_indirect_calls() {
    // The seed element comes from a word returning an *erased* quotation (a
    // declared-effect context `fill` can intern an element type from); the
    // second element is a literal stored through the array's `&!` referent,
    // which carries the declared `[ i64 -- i64 ]` the store materializes it
    // against. A bare literal to `fill` has no effect context and is rejected.
    let src = ": one ( -- [ i64 -- i64 ] ) [ 1 + ] ;\n\
               : main ( -- )\n\
               one 2 fill | a |\n\
               &!a 1 >usize &!> [ 2 + ] !\n\
               &a 0 &> @ 4 swap call .\n\
               &a 1 &> @ 4 swap call . ;\n";
    let (stdout, code) = run_src("qarray", src);
    assert_eq!(stdout, "5\n6\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "reading an element back and calling it must be an indirect call"
    );
}

// -- T-return: a word returning a quotation, indirect-called at the call site -

#[test]
fn quotation_returned_from_word_indirect_calls() {
    let src = ": mk ( -- [ i64 -- i64 ] ) [ 1 + ] ;\n\
               : main ( -- ) mk 4 swap call . ;\n";
    let (stdout, code) = run_src("qreturn", src);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "a returned quotation is erased at the call site, so `call` is indirect"
    );
}

// -- T-repoint (struct field): a captured scalar snapshots into the env -------

#[test]
fn capturing_scalar_stored_snapshots_into_env() {
    // 7b re-points 7a's `capturing_literal_stored_is_error_naming_7b`: `[ x + ]`
    // reads the enclosing scalar `x`, which is now snapshotted into the env
    // rather than rejected. Stored into a field, read back, and called with 4:
    // 4 + 10 = 14.
    let src = "type: Holder q [ i64 -- i64 ] ;\n\
               : main ( -- ) 10 | x | [ x + ] Holder Holder>q 4 swap call . ;\n";
    let (stdout, code) = run_src("qcapfield", src);
    assert_eq!(stdout, "14\n");
    assert_eq!(code, 0);
}

// -- T-repoint (array element): a scalar snapshot stored through a reference --

#[test]
fn capturing_scalar_in_array_element_snapshots() {
    // The `!`/`+!` store boundary (an array element via reference), re-pointed:
    // `[ x + ]` snapshots `x = 10`; read back and called with 4 gives 14.
    let src = ": one ( -- [ i64 -- i64 ] ) [ 1 + ] ;\n\
               : main ( -- )\n\
               10 | x |\n\
               one 2 fill | a |\n\
               &!a 1 >usize &!> [ x + ] !\n\
               &a 1 &> @ 4 swap call . ;\n";
    let (stdout, code) = run_src("qcaparray", src);
    assert_eq!(stdout, "14\n");
    assert_eq!(code, 0);
}

// -- T-repoint (nested): a scalar captured through a *nested* quotation -------

#[test]
fn capturing_scalar_through_nested_quotation_snapshots() {
    // `x` is read inside a nested `[ x + ]`, not at the stored quotation's own
    // top level. The capture scan recurses into nested quotation bodies, so the
    // outer `[ [ x + ] call ]` snapshots `x` and stores admissibly: 4 + 10 = 14.
    let src = "type: Holder q [ i64 -- i64 ] ;\n\
               : main ( -- ) 10 | x | [ [ x + ] call ] Holder Holder>q 4 swap call . ;\n";
    let (stdout, code) = run_src("qcapnested", src);
    assert_eq!(stdout, "14\n");
    assert_eq!(code, 0);
}

// -- T-makeb: an outer-rooted reference capture (a `&T` parameter) is admitted -

#[test]
fn make_b_captures_outer_rooted_reference_admits() {
    // `make-b`'s closure captures `r`, a `&[i64 4]` *parameter*: its referent
    // is rooted outside `make-b`'s frame (in `main`'s `a`, still live at the
    // call), so the escaping capture is admitted. The env holds the reference;
    // reading `r[0] = 5` and adding the input 4 gives 9.
    let src = ": make-b ( &[i64 4] -- [ i64 -- i64 ] ) | r | [ r 0 >usize &> @ + ] ;\n\
               : main ( -- ) 5 4 fill | a | &a make-b 4 swap call . ;\n";
    let (stdout, code) = run_src("qmakeb", src);
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);
    // T-env-inline: the materialized body takes the declared `i64` input plus
    // one trailing `Ptr` env parameter (R17). Dropping the env param would
    // shorten this list and the reference could not reach the body.
    assert_eq!(
        materialized_quot_params(src),
        vec![
            IrType::Int {
                bits: 64,
                signed: true
            },
            IrType::Ptr
        ],
        "a one-capture materialized body has its declared input plus a Ptr env param"
    );
}

// -- T-makea: a frame-rooted capture escaping its owning frame is rejected ----

#[test]
fn make_a_captures_frame_local_past_owning_frame_error() {
    // `make-a`'s closure borrows `arr`, a `[i64 4]` bound *inside* `make-a`;
    // returning the closure would let it outlive `arr`'s storage. Unlike
    // `make-b`'s parameter, this is frame-rooted, so the escaping boundary is a
    // past-owning-frame error (R15/R24), asserted whole.
    let err = check_error(
        ": make-a ( -- [ i64 -- i64 ] ) 5 4 fill | arr | [ &arr 0 >usize &> @ + ] ;\n\
         : main ( -- ) make-a 4 swap call . ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure captures `arr`, a local of this frame, whose storage does not survive the return (line 1)"
    );
}

// -- T-quot-cap-deferred: capturing a quotation-typed name is deferred --------

#[test]
fn capturing_quotation_typed_name_is_deferred() {
    // The outer `[ q call ]` captures `q`, itself a quotation local. A
    // quotation env slot is two words and needs a recursive surviving-set fold
    // (R15 case 4), so it is deferred at every boundary rather than admitted.
    let err = check_error(
        ": wrap ( -- [ i64 -- i64 ] ) [ 1 + ] | q | [ q call ] ;\n\
         : main ( -- ) wrap 4 swap call . ;\n",
    );
    assert_eq!(
        err,
        "error: capturing a quotation value by name is deferred (line 1)"
    );
}

// -- T-multi-esc: a 2+-capture escaping closure is deferred (single-word env) -

#[test]
fn escaping_closure_with_two_captures_is_deferred() {
    // Phase 1's inline env holds one word; `[ x y + + ]` captures two scalars,
    // which needs a heap env (R18), deferred.
    let err = check_error(
        ": mk ( -- [ i64 -- i64 ] ) 10 | x | 20 | y | [ x y + + ] ;\n\
         : main ( -- ) mk 4 swap call . ;\n",
    );
    assert_eq!(
        err,
        "error: an escaping closure may capture at most one reference (a heap env is deferred) (line 1)"
    );
}

// -- T-splice-cap: a capturing literal at a direct `call` still splices -------

#[test]
fn capturing_literal_spliced_still_works() {
    // Unchanged from the combinator slices: a capturing literal consumed by a
    // direct `call` is spliced in place (no materialization, no boundary), so
    // it reads the enclosing local and runs.
    let src = ": main ( -- ) 10 | x | 4 [ x + ] call . ;\n";
    let (stdout, code) = run_src("qsplice", src);
    assert_eq!(stdout, "14\n");
    assert_eq!(code, 0);
    assert!(
        !emits_call_indirect(src),
        "a spliced literal must not lower to an indirect call"
    );
}

// -- T-join: two differing quotation arms materialize against the declared ---
// -- output row and are indirect-called ---------------------------------------

#[test]
fn two_differing_quotation_arms_materialize_and_call() {
    // Each arm leaves a *different* literal; the join has no single body to
    // splice, so it materializes each against the word's declared
    // `[ i64 -- i64 ]` output (R11) and `call` at the site dispatches
    // indirectly through whichever aggregate the branch left.
    let src = ": pick ( bool -- [ i64 -- i64 ] ) if [ 1 + ] else [ 2 + ] end ;\n\
               : main ( -- ) true pick 4 swap call . false pick 4 swap call . ;\n";
    let (stdout, code) = run_src("qjoin", src);
    assert_eq!(stdout, "5\n6\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "a materialized branch-join quotation is indirect-called"
    );
}

// -- T-join-same: one literal in both arms stays a splice (no materialize) ----

#[test]
fn same_quotation_both_arms_still_splices() {
    // One literal bound before the `if`, named in both arms: the equal `Known`
    // ids forward the marker (no `Phi`, no materialization), so `call` after
    // the join splices exactly as today and emits no indirect call.
    let src = ": main ( -- ) [ 1 + ] | q | true if q else q end 5 swap call . ;\n";
    let (stdout, code) = run_src("qjoinsame", src);
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);
    assert!(
        !emits_call_indirect(src),
        "the same literal in both arms is spliced, never indirect-called"
    );
}

// -- T-repoint-join: a scalar snapshot in one arm of a materializing join -----

#[test]
fn capturing_scalar_at_join_snapshots() {
    // A materializing join (the word declares a `[ i64 -- i64 ]` output) whose
    // `true` arm captures the scalar `x`. 7b re-points 7a's rejection: the arm
    // snapshots `x = 10` and both arms materialize (the `false` arm captures
    // nothing, a null env). `true`: 4 + 10 = 14; `false`: 4 + 2 = 6.
    let src = ": pick ( bool -- [ i64 -- i64 ] ) 10 | x | if [ x + ] else [ 2 + ] end ;\n\
               : main ( -- ) true pick 4 swap call . false pick 4 swap call . ;\n";
    let (stdout, code) = run_src("qcapjoin", src);
    assert_eq!(stdout, "14\n6\n");
    assert_eq!(code, 0);
}

// -- T-times: `times` over an erased quotation is one indirect call in a loop -

#[test]
fn times_over_erased_quotation_runs_constant_stack() {
    // `acc` returns an *erased* quotation (a word-output materialization
    // boundary); `times` drives it. The checker accepts the abstract
    // `[ i64 i64 -- i64 ]` effect and lowering emits exactly one `CallIndirect`
    // inside the loop body (D6: constant stack, one indirect call per
    // iteration), summing 0+1+2+3+4 = 10.
    let src = ": acc ( -- [ i64 i64 -- i64 ] ) [ + ] ;\n\
               : main ( -- ) 0 5 acc times . ;\n";
    let (stdout, code) = run_src("qtimes", src);
    assert_eq!(stdout, "10\n");
    assert_eq!(code, 0);
    assert_eq!(
        count_call_indirect(src),
        1,
        "a `times`-erased loop has exactly one indirect call, in the body"
    );
}

// -- T-dogfood: examples/vm_table.sth matches examples/vm.sth byte-for-byte ---

#[test]
fn vm_table_dispatch_matches_clause_version() {
    // `examples/vm_table.sth` replaces `examples/vm.sth`'s clause-dispatched
    // `run` with a `decode` clause (the one unavoidable elimination, Q5) plus a
    // table of nine uniform, match-free `[ Vm -- Vm ]` handlers, indirect-called
    // by `dispatch`. Parity: same bytecode program, same result.
    let table_binary = common::build_example("examples/vm_table.sth");
    let table_stdout = std::process::Command::new(&table_binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("vm_table binary should run")
        .stdout;
    std::fs::remove_file(&table_binary).ok();

    let clause_binary = common::build_example("examples/vm.sth");
    let clause_stdout = std::process::Command::new(&clause_binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("vm binary should run")
        .stdout;
    std::fs::remove_file(&clause_binary).ok();

    assert_eq!(
        table_stdout, clause_stdout,
        "the table-dispatched VM must match the clause-dispatched one byte-for-byte"
    );

    let src = std::fs::read_to_string("examples/vm_table.sth").expect("read vm_table.sth");
    assert!(
        emits_call_indirect(&src),
        "the table dispatch path must emit at least one indirect call"
    );

    // M5: the table only proves the feature if the handlers are genuinely
    // match-free. Inlining a clause match back into a handler (defeating the
    // decode/execute split) must be caught here, not just by lowering.
    let tokens = lexer::lex(&src).expect("lexing vm_table.sth should succeed");
    let module = parser::parse(&tokens).expect("parsing vm_table.sth should succeed");
    let handler_names = [
        "h-push", "h-add", "h-sub", "h-mul", "h-load", "h-store", "h-jz", "h-jmp", "h-halt",
    ];
    for name in handler_names {
        let word = module
            .words
            .iter()
            .find(|w| w.name == name)
            .unwrap_or_else(|| panic!("vm_table.sth should define `{name}`"));
        assert!(
            matches!(word.body, WordBody::Terms { .. }),
            "handler `{name}` must be match-free (a term body, not a clause body)"
        );
    }
}
