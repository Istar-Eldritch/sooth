use super::*;
use crate::ast::{
    ArrayDecl, ArrayId, GenericEnumDecl, GenericStructDecl, GenericVariantDecl, OwnedCellDecl,
};
use crate::lexer::lex;

/// P7.S3k: the callee-side walk context for a fixture that drives
/// `poly_term`/`poly_walk_arms` directly. The registry is empty, so no
/// cross-call arm can fire and no record survives -- both of which every
/// one of those fixtures wants. A macro rather than a function because
/// `CrossCtx` borrows two temporaries, which only live as long as the
/// statement they are written in.
macro_rules! scratch_cross {
    () => {
        &mut CrossCtx {
            env: &PolyEnv::new(),
            calls: &mut Vec::new(),
        }
    };
}

fn check_src(src: &str) -> Result<(), String> {
    checked_like_a_build(src).map(|_| ())
}

/// A source checked the way `driver::assemble_module` checks one: the
/// declaration checks first (P7.S3e -- `check_impl_decls` is what resolves
/// each `impl:` binding to the word it names, and no call site can resolve
/// an obligation without it, which `parse_with_core` now runs for every
/// caller), then the body/call-site pass, returning the checked module
/// alongside what R17's pre-pass recorded.
fn checked_like_a_build(src: &str) -> Result<(Module, Vec<WordObligations>), String> {
    let tokens = lex(src).unwrap();
    let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
    let recorded = super::super::check_module(&mut module)?;
    Ok((module, recorded))
}

/// A source checked the way a real build checks one, *including* the
/// per-module name mangling every build applies even to a single file
/// (`assemble_module`'s `always_mangle`). `check_src` skips
/// `resolve_modules`, so a call to a `lib/` word arrives under its bare
/// spelling there -- the test-harness artefact that kept the deleted
/// six-name comparison carve-out (P7.S3k R7) looking alive. Through here,
/// `gt` arrives as `gt__mN`, which is what a real call site holds.
///
/// The declaration pre-passes run *before* `resolve_modules`, inside
/// `parse_with_core`, exactly as `assemble_module` orders them:
/// `check_impl_decls` resolves each binding by the name the parser's
/// desugar synthesized, which only agrees with `WordDef::name` pre-mangle.
fn check_src_mangled(src: &str) -> Result<(), String> {
    checked_like_a_build_mangled(src).map(|_| ())
}

/// `check_src_mangled` keeping the checked `Module`, for a pin on what
/// checking *recorded* (a minted dispatch symbol) rather than merely that
/// it succeeded. Mangled rather than `checked_like_a_build` because an
/// instantiation symbol carries the member word's per-module suffix
/// (`odd;Odd;0;Box['T0]__m0`), which only a mangled run produces.
fn checked_like_a_build_mangled(src: &str) -> Result<Module, String> {
    let tokens = lex(src).unwrap();
    let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
    crate::resolve::resolve_modules(&mut module, true).unwrap();
    super::super::check_module(&mut module)?;
    Ok(module)
}

/// The cross-call records a checked source produced, keyed by the
/// polymorphic word whose body made them (P7.S3k R2).
fn cross_calls_of(src: &str) -> HashMap<String, Vec<PolyCrossCall>> {
    let (module, _) = checked_like_a_build(src).expect("the fixture checks");
    module.poly_cross_calls
}

/// P7.S3e (R7/R17): the obligations the pre-pass recorded, keyed by the
/// polymorphic word whose body recorded them.
/// P7b.S2 (S2-16, poly caller): an HKT member call from a polymorphic
/// body unifies the declared member sig against the caller's slots --
/// the obligation records the call site's operand slots (the binding
/// map's carrier, S2-9), and the resolve loop composes them into
/// θ_call, never the empty subst.
#[test]
fn poly_caller_member_call_records_slot_map_and_composes_theta_call() {
    let (_, recorded) = checked_like_a_build(
        "type: Opt['T] | None | Some 'T ;\n\
             trait: Functor['F: * -> *] :\n\
             map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             impl: Functor for Opt\n\
             : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Opt? ;\n\
             ;\n\
             : apply['F: Functor 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) map ;\n\
             : main ( -- ) ;\n",
    )
    .expect("the fixture checks");
    // The body walk recorded the member call as an obligation carrying
    // the call site's operand slots: the App-headed dispatchable operand
    // (caller space) and the declared quotation slot.
    let apply = recorded
        .iter()
        .find(|w| w.name == "apply")
        .expect("apply's obligations recorded");
    assert_eq!(apply.obligations.len(), 1, "{apply:?}");
    let ob = &apply.obligations[0];
    assert_eq!(ob.member, "map");
    assert_eq!(ob.slots.len(), 2, "one slot per declared member input");
    assert!(
        matches!(ob.slots[0], PolyType::App { head: 0, .. }),
        "the dispatchable operand is App-headed in the caller's space: {:?}",
        ob.slots[0]
    );
    // A caller instantiation whose θ grounds the header to a CtorImage
    // (the App unification binds it from the operand) and every other
    // variable from a value slot, resolves the obligation through the
    // CtorImage path: the member word's θ_call -- never the empty subst.
    let (module, _) = checked_like_a_build(
            "type: Opt['T] | None | Some 'T ;\n\
             trait: Functor['F: * -> *] :\n\
             map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             impl: Functor for Opt\n\
             : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Opt? ;\n\
             ;\n\
             : apply['F: Functor 'T 'U] ( 'F['T] [ 'T -- 'U ] 'U -- 'F['U] ) rot rot map swap drop ;\n\
             : main ( -- ) 3 mkopt [ drop True ] True apply drop ;\n\
             : mkopt ( i64 -- Opt[i64] ) Some ;\n",
        )
        .expect("the fixture checks");
    let monos: Vec<&crate::ast::CallInst> = module
        .transitive_instantiations
        .iter()
        .filter(|inst| inst.callee.contains("map;Functor"))
        .collect();
    assert!(!monos.is_empty(), "the member word's θ_call is recorded");
    for inst in &monos {
        assert!(
            !inst.subst.ty.is_empty(),
            "no empty subst for a CtorImage winner: ({}, {:?})",
            inst.callee,
            inst.subst
        );
        // Canonical order at construction (the P7.S3t invariant).
        let ids: Vec<u32> = inst.subst.ty.iter().map(|(v, _)| *v).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "θ_call sorted: {:?}", inst.subst);
    }
    // The composed θ_call grounds the member's own variables: 'ctor0 →
    // i64, 'U → Bool (the member word's union id space).
    assert_eq!(monos[0].subst.ty[0].1, Type::I64);
}

/// P7b.S2 (S2-16, poly caller): a member operand that does not unify
/// against the declared sig stays a located member-call operand error,
/// now naming the offending slot position. A pinned member argument
/// (`'F[i64]`) against a caller slot carrying its own variable is the
/// mismatch: the declared `Concrete` never binds a caller variable at
/// body-check (S2-6's pinned-slot reading).
#[test]
fn poly_caller_member_operand_mismatch_names_the_slot() {
    let err = check_src(
        "trait: P2['F: * -> *] : pick ( 'F[i64] -- ) ; ;\n\
             : caller['F: P2 'T] ( 'F['T] -- ) pick ;\n",
    )
    .expect_err("a pinned member arg cannot bind the caller's variable");
    assert!(
        err.contains("expects"),
        "the declared-vs-found shapes are named: {err}"
    );
    assert!(
        err.contains("in operand slot 0"),
        "the offending slot position is named: {err}"
    );
}

// P7b.S2 (S2-8/S2-9): the compatibility-conditioned tie rule, end to end
// through `resolve_user_bound`'s per-site loop. The fixtures use a
// two-variable constructor: a one-variable ctor's pinned spelling
// (`for Opt[i64]`) is a complete application, so it parses to a
// `Concrete` target and the S2-6 fence rejects App-headed members for it
// at parse time -- a pinned HKT impl only exists for a partially-applied
// ctor (`for R2[i64]`), whose desugar is the `Generic` pattern with
// `Concrete` pins the tie rule arbitrates among. Each fixture drives a
// poly caller (`apply`) instantiated at a concrete operand, which
// grounds θ('F) to a bare `CtorImage` -- the only place the tie rule
// runs (S2-8: the mono path dispatches on the fully concrete operand
// through the existing `Concrete`/`Generic` matcher arms instead).

/// S2-8, incompatible pin disqualified: at an `R2[Bool Bool]` operand
/// the pinned candidate (`for R2[i64]`) matches on constructor identity
/// alone, but its pin disagrees with the grounded operands and is
/// disqualified at the member call site -- the bare `for R2` impl
/// serves. Without the compatibility condition the specificity order
/// would hand the site to the pinned impl (it is the more specific
/// pattern) and the call would die at the pinned member input.
#[test]
fn ctor_image_dispatch_serves_via_the_unpinned_impl_when_the_pin_disagrees() {
    let (module, _) = checked_like_a_build(
        "type: R2['T 'E] | Ok 'T | Err 'E ;\n\
             trait: Functor['F: * -> * -> *] :\n\
             consume ( 'F['T 'E] -- ) ;\n\
             ;\n\
             impl: Functor for R2\n\
               : consume drop ;\n\
             ;\n\
             impl: Functor for R2[i64]\n\
               : consume drop ;\n\
             ;\n\
             : apply['F: Functor 'T 'E] ( 'F['T 'E] -- ) consume ;\n\
             : mkokb ( Bool -- R2[Bool Bool] ) Ok ;\n\
             : main ( -- ) True mkokb apply ;\n",
    )
    .expect("the incompatible pin is disqualified; the bare target serves");
    let monos: Vec<&crate::ast::CallInst> = module
        .transitive_instantiations
        .iter()
        .filter(|inst| inst.callee.contains("consume;Functor"))
        .collect();
    assert_eq!(
        monos.len(),
        1,
        "exactly the serving impl's member word mints: {:?}",
        monos.iter().map(|i| &i.callee).collect::<Vec<_>>()
    );
    // The bare `for R2` impl's member word: its desugared target shape
    // renders `R2['T0 'T1]` (two fresh pattern variables), where the
    // pinned impl's renders `R2[i64 'T0]` (the pin, then the padded
    // fresh var -- the FIRST var interned in the target builder).
    assert!(
        monos[0].callee.contains("R2['T0 'T1]"),
        "the bare `for R2` impl served the site: {}",
        monos[0].callee
    );
}

/// S2-8, more pins preferred among compatible: at an `R2[Bool Bool]`
/// operand both candidates are compatible (the bare one's pattern
/// variables bind, the pinned one's pin matches), and the tie rule hands
/// the site to the more-pinned `for R2[Bool]` impl -- the
/// concrete-beats-generic principle, conditioned on the pin actually
/// unifying with the grounded operands.
#[test]
fn ctor_image_dispatch_prefers_the_compatible_pinned_impl_over_the_bare_one() {
    let (module, _) = checked_like_a_build(
        "type: R2['T 'E] | Ok 'T | Err 'E ;\n\
             trait: Functor['F: * -> * -> *] :\n\
             consume ( 'F['T 'E] -- ) ;\n\
             ;\n\
             impl: Functor for R2\n\
               : consume drop ;\n\
             ;\n\
             impl: Functor for R2[Bool]\n\
               : consume drop ;\n\
             ;\n\
             : apply['F: Functor 'T 'E] ( 'F['T 'E] -- ) consume ;\n\
             : mkokb ( Bool -- R2[Bool Bool] ) Ok ;\n\
             : main ( -- ) True mkokb apply ;\n",
    )
    .expect("the compatible pinned candidate wins");
    let monos: Vec<&crate::ast::CallInst> = module
        .transitive_instantiations
        .iter()
        .filter(|inst| inst.callee.contains("consume;Functor"))
        .collect();
    assert_eq!(monos.len(), 1, "one impl serves: {:?}", monos);
    // The pinned `for R2[Bool]` impl's member word: its desugared target
    // shape renders `R2[Bool 'T0]` (one pin, one fresh pattern
    // variable), where the bare impl's renders `R2['T0 'T1]`.
    assert!(
        monos[0].callee.contains("R2[Bool 'T0]"),
        "the pinned `for R2[Bool]` impl served the site: {}",
        monos[0].callee
    );
}

/// S2-8, identical pin-shape tie: two one-pin candidates whose pins both
/// unify with the grounded operands (`for R2[Bool]` pinning slot 0,
/// `for R2['A Bool]` pinning slot 1) both survive compatibility at the
/// site -- the ambiguity error, located at the member call (the per-site
/// rule's span), not at the caller's instantiation site. The targets are
/// not alpha-equivalent (the pins sit in different slots), so both
/// register.
#[test]
fn ctor_image_dispatch_identical_pin_shape_tie_is_ambiguity_error() {
    let err = check_src(
        "type: R2['T 'E] | Ok 'T | Err 'E ;\n\
             trait: Functor['F: * -> * -> *] :\n\
             consume ( 'F['T 'E] -- ) ;\n\
             ;\n\
             impl: Functor for R2[Bool]\n\
               : consume drop ;\n\
             ;\n\
             impl: Functor for R2['A Bool]\n\
               : consume drop ;\n\
             ;\n\
             : apply['F: Functor 'T 'E] ( 'F['T 'E] -- ) consume ;\n\
             : mkokb ( Bool -- R2[Bool Bool] ) Ok ;\n\
             : main ( -- ) True mkokb apply ;\n",
    )
    .expect_err("two same-tier survivors at one site are the ambiguity");
    assert!(
        err.contains("ambiguous `impl:` dispatch for `Functor` at `R2`"),
        "{err}"
    );
    // Both same-tier targets are named, under their user spellings.
    assert!(err.contains("`R2[Bool]`"), "{err}");
    assert!(err.contains("`R2['A Bool]`"), "{err}");
    // Located at the member call inside `apply` (line 11) -- the
    // per-site rule's span -- not at `main`'s call site (line 13), which
    // is what the generic specificity order's pre-emption produced
    // before the compatibility-conditioned rule governed.
    assert!(err.contains("line 11, col "), "{err}");
}

/// P7b.S2 (S2-16, mono caller): a member name no trait declares falls
/// through to the unchanged unknown-word error (existing goldens hold).
#[test]
fn mono_member_name_unknown_word_falls_through_unchanged() {
    let err = check_src(
        "type: Opt['T] | None | Some 'T ;\n\
             trait: Functor['F: * -> *] :\n\
             map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             : main ( -- ) 3 notamember drop ;\n",
    )
    .expect_err("unknown word");
    assert!(err.contains("unknown word `notamember`"), "{err}");
}

/// P7b.S2 (S2-16, mono caller): the member name exists but no impl of
/// any declaring trait dispatches on these operands -- a located error
/// naming the candidates, not the generic unknown-word report.
#[test]
fn mono_member_without_dispatching_impl_is_located_error() {
    let err = check_src(
        "type: Opt['T] | None | Some 'T ;\n\
             type: P2 n i64 ;\n\
             trait: Functor['F: * -> *] :\n\
             map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             : main ( -- ) 3 P2 [ drop True ] map[i64 Bool] drop ;\n",
    )
    .expect_err("no impl of Functor dispatches on P2");
    assert!(
        err.contains("is a trait member of"),
        "the member candidates are named: {err}"
    );
    assert!(err.contains("no `impl:`"), "{err}");
}

/// P7b.S2 (S2-16, mono caller): two traits declare a member of the same
/// name and an impl of each fits the concrete operand -- the mono
/// ambiguity, with its own wording (not the poly-bounds report the
/// shared builder renders): the claiming traits are named and the
/// remedy is the module qualifier, which is what a mono caller has.
#[test]
fn mono_member_claimed_by_two_traits_is_located_error_naming_both() {
    let err = check_src(
        "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             type: Point x i64 y i64 ;\n\
             impl: A for Point\n\
               : t1 | p | p drop ;\n\
             ;\n\
             impl: B for Point\n\
               : t1 | p | p drop ;\n\
             ;\n\
             : main ( -- ) 1 2 Point |p| &p t1 p drop ;\n",
    )
    .expect_err("both impls fit the &Point operand");
    // The mono wording: the claiming traits, then the qualifier remedy.
    assert!(
        err.contains("`t1` in `main` (line 10, col 32) is a trait member of both `A` and `B`"),
        "{err}"
    );
    assert!(
        err.contains("qualify the call with the claiming trait's module"),
        "{err}"
    );
}

/// S2-16 (final-review fix): a mono call resolving through a *concrete*
/// impl's member may not carry an explicit type-argument list. The
/// widened `poly_call_takes_type_args` clause admits the spelling for
/// member names because the generic branch reads it (W2's θ seeding);
/// a concrete target's member sig has no free variables to bind, so the
/// list is provably meaningless and is rejected, not silently dropped.
#[test]
fn mono_concrete_member_call_with_explicit_type_args_is_error() {
    let err = check_src(
        "type: Opt['T] | None | Some 'T ;\n\
             trait: Functor['F] : size ( 'F -- i64 ) ; ;\n\
             impl: Functor for Opt[i64]\n\
               : size drop 1 ;\n\
             ;\n\
             : main ( -- ) 5 Some size[i64] drop ;\n",
    )
    .expect_err("explicit type args on a concrete member call");
    assert!(
        err.contains("takes no type arguments"),
        "the meaningless list is rejected: {err}"
    );
}

/// S2-16 (final-review fix, corner): a user's own mono word that merely
/// shares a trait member's name is claimed by the env route first -- the
/// route that wins the bare call and cannot read an argument list -- so
/// the widened explicit-instantiation clause must not admit the name,
/// and the pre-S2-16 rejection stands (the list used to be silently
/// dropped into the env call).
#[test]
fn mono_word_colliding_with_member_name_rejects_explicit_type_args() {
    let err = check_src(
        "trait: Functor['F] : size ( 'F -- i64 ) ; ;\n\
             : size ( i64 -- i64 ) drop 7 ;\n\
             : main ( -- ) 5 size[i64] . ;\n",
    )
    .expect_err("explicit type args on a name the env route claims");
    assert!(
        err.contains("takes no type arguments"),
        "the collision restores the pre-widening rejection: {err}"
    );
}

/// P7b.S6 Phase 4 (R4): `empty` is nullary -- `Monoid`'s own type
/// variable never appears in an input, so `dispatchable_input_pos`
/// returns `None` for every candidate and the ordinary operand-dispatch
/// loop never wins. An explicit `empty[i64]` grounds the trait variable
/// directly from the call site's type argument instead.
#[test]
fn mono_nullary_member_grounds_from_explicit_type_args() {
    check_src(
        "trait: Monoid['T] :\n\
             empty ( -- 'T ) ;\n\
             : combine ( 'T 'T -- 'T ) ;\n\
             ;\n\
             impl: Monoid for i64\n\
               : empty 5 ;\n\
               : combine add ;\n\
             ;\n\
             : main ( -- ) 7 empty[i64] combine drop ;\n",
    )
    .expect("empty[i64] grounds Monoid's 'T from the explicit type argument");
}

/// The generic-target twin of the fixture above, whose `impl:` target is
/// `Opt['ctor0]` rather than a concrete `i64`.
fn nullary_member_generic_target_src() -> &'static str {
    "type: Opt['T] | None | Some 'T ;\n\
         trait: Monoid['T] :\n\
         empty ( -- 'T ) ;\n\
         : combine ( 'T 'T -- 'T ) ;\n\
         ;\n\
         impl: Monoid for Opt\n\
           : empty None ;\n\
           : combine drop ;\n\
         ;\n\
         : mkopt ( i64 -- Opt[i64] ) Some ;\n\
         : main ( -- ) 1 mkopt drop empty[Opt[i64]] drop ;\n"
}

/// P7b.S8b Phase 1 (R4): over a *generic* impl target the call-site type
/// argument is the dispatch operand, not variable #0's value -- the
/// member word's variables are the impl target's own (`build_member_var_union`,
/// `src/parser.rs:768`), so the written `Opt[i64]` belongs one level below
/// `'ctor0`. Seeding it positionally (P7.S3t's contract, right for every
/// other caller) minted the monomorph at `Opt[Opt[i64]]`; the impl-target
/// equation `find_bound_impl` already computed says `'ctor0 := i64`.
/// Asserted on the minted enum registry, the one place the wrong
/// instantiation is visible without lowering.
#[test]
fn nullary_member_over_a_generic_target_mints_the_one_level_instantiation() {
    let (module, _) = checked_like_a_build(nullary_member_generic_target_src())
        .expect("the generic-target nullary fixture checks");
    let names: Vec<&str> = module.enums.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"Opt[i64]"),
        "the member's own instantiation is minted: {names:?}"
    );
    assert!(
        !names.contains(&"Opt[Opt[i64]]"),
        "the positionally-seeded one-level-too-deep mint is gone: {names:?}"
    );
}

/// P7b.S8b Phase 1 (R4): the seed replaces the explicit list's *binding*,
/// not the list's own arity check. `check_poly_call`'s
/// `instantiation_arity_error` still reads `type_args`, so a surplus
/// argument stays the located error it was before the channel existed.
#[test]
fn nullary_member_with_surplus_type_args_is_still_an_arity_error() {
    let err = check_src(
        "type: Opt['T] | None | Some 'T ;\n\
             trait: Monoid['T] :\n\
             empty ( -- 'T ) ;\n\
             : combine ( 'T 'T -- 'T ) ;\n\
             ;\n\
             impl: Monoid for Opt\n\
               : empty None ;\n\
               : combine drop ;\n\
             ;\n\
             : main ( -- ) empty[Opt[i64] i64] drop ;\n",
    )
    .expect_err("two type arguments for one declared variable");
    assert!(
        err.contains("declares 1 type variable") && err.contains("given 2 type arguments"),
        "the untouched arity gate still fires: {err}"
    );
}

/// P7b.S6 Phase 4 (R5): Q1 rules out consuming-context inference for this
/// slice -- bare `empty` with no explicit instantiation is a located
/// error naming the `empty[i64]` remedy, not a lookahead and not a panic.
#[test]
fn bare_nullary_member_without_instantiation_is_located_error() {
    let err = check_src(
        "trait: Monoid['T] :\n\
             empty ( -- 'T ) ;\n\
             : combine ( 'T 'T -- 'T ) ;\n\
             ;\n\
             impl: Monoid for i64\n\
               : empty 5 ;\n\
               : combine add ;\n\
             ;\n\
             : main ( -- ) empty drop ;\n",
    )
    .expect_err("bare empty cannot ground Monoid's 'T from context");
    assert!(err.contains("no operand to dispatch on"), "{err}");
    assert!(
        err.contains("empty[i64]"),
        "the remedy names explicit instantiation: {err}"
    );
}

/// P7b.S6 Phase 4 (R4) scope fence: an operand-carrying member (`size`
/// has a dispatchable input) never falls into the zero-dispatchable-input
/// branch, even when the call carries an explicit type argument and the
/// ordinary operand-dispatch loop finds no impl -- it still reports the
/// ordinary no-dispatch error rather than wrongly grounding on the type
/// argument and picking an unrelated impl.
#[test]
fn mono_operand_carrying_member_with_type_args_and_no_operand_match_is_no_dispatch_error() {
    let err = check_src(
        "type: Opt['T] | None | Some 'T ;\n\
             type: P2 n i64 ;\n\
             trait: Sizer['F] : size ( 'F -- i64 ) ; ;\n\
             impl: Sizer for Opt[i64]\n\
               : size drop 1 ;\n\
             ;\n\
             : main ( -- ) 3 P2 size[i64] drop ;\n",
    )
    .expect_err(
        "P2 has no Sizer impl; the explicit i64 argument must not ground it via P2's operand",
    );
    assert!(
        err.contains("no `impl:`"),
        "the ordinary no-dispatch error, not a wrongly grounded dispatch: {err}"
    );
}

/// P7b.S2 (S2-16, mono caller): a concrete-impl member may legally
/// declare a quotation parameter (S2-3's Quotation arm over a concrete
/// target grounds to a real `Type::Quotation`), and a mono caller's
/// literal fills it materialized -- the same R8/D4 boundary the ordinary
/// env word-call path applies, not the blanket quotation reject.
#[test]
fn mono_member_concrete_impl_declared_quotation_param_materializes() {
    check_src(
        "type: Box['T] v 'T ;\n\
             trait: Runner['F] : run ( 'F [ i64 -- ] -- ) ; ;\n\
             impl: Runner for Box[i64]\n\
               : run swap drop 1 swap call ;\n\
             ;\n\
             : mkbox ( i64 -- Box[i64] ) Box ;\n\
             : main ( -- ) 1 mkbox [ drop ] run ;\n",
    )
    .expect("the declared quotation slot materializes the literal");
}

/// P7b.S2 (S2-16, mono caller): the blanket quotation reject survives
/// for slots the member does not declare as a quotation -- the
/// materialization above is gated strictly on the declared slot's type.
#[test]
fn mono_member_concrete_impl_undeclared_quotation_arg_is_rejected() {
    let err = check_src(
        "type: Box['T] v 'T ;\n\
             trait: Sizer['F] : size ( 'F i64 -- i64 ) ; ;\n\
             impl: Sizer for Box[i64]\n\
               : size drop drop 7 ;\n\
             ;\n\
             : mkbox ( i64 -- Box[i64] ) Box ;\n\
             : main ( -- ) 1 mkbox [ drop ] size drop ;\n",
    )
    .expect_err("the i64 slot is not a quotation boundary");
    assert!(
        err.contains("a quotation cannot be passed to `size`")
            && err.contains("in `main` (line 7)"),
        "{err}"
    );
}

/// P7b.S2 (S2-10): the cross-call fence is NOT lifted. A named poly-word
/// cross-call carrying an App slot still rejects with p8's exact fence
/// text (S1-17.i's scope fence, amended R7) -- member calls never reach
/// this site (`poly_trait_member_call` fronts them), so the fence
/// governs only non-member cross-calls, unchanged.
#[test]
fn non_member_app_cross_call_still_rejects_with_p8_fence_text() {
    let err = check_src(
        "type: Opt['T] | None | Some 'T ;\n\
             : inner['G 'T] ( 'G['T] -- ) drop ;\n\
             : outer['G 'T] ( 'G['T] -- ) inner ;\n\
             : main ( -- ) ;\n",
    )
    .expect_err("a poly cross-call with an App slot is fenced");
    assert!(
        err.contains("cannot call the polymorphic word `inner`"),
        "{err}"
    );
    assert!(
        err.contains(
            "a higher-kinded application in a cross-called polymorphic word is not \
                 yet supported from a polymorphic body"
        ),
        "{err}"
    );
}

/// P7b.S2 (S2-15.f, splice guard a): an application-headed member called
/// inside a combinator splice is a located error, not
/// `ground_member_type`'s unreachable-App panic.
///
/// What this fixture honestly pins is the body-check half only: an HKT
/// member call from a bounded poly body is operand-checked against the
/// caller's abstract slots (the first `map` unifies), and the second
/// `map`'s missing operands are a located error, never a panic on the
/// App-headed signature. This fixture's `twice` is deliberately
/// *non*-`inline`, so it never reaches `check_poly_combinator_standalone`
/// at all. The `inline` twin of this shape no longer hits a fence there:
/// `splice_member_hkt_error` is deleted (Phase 3, S3-7), and grounding
/// against S3-6's `CtorImage` stand-in raises the *tagged*
/// `splice_member_ctor_image_error`, which the standalone check rescues
/// so the body is re-checked at each real splice (see
/// `class_two_member_call_is_rescued_for_recheck_at_the_splice`). What
/// this fixture still pins, for the non-combinator path, is unchanged:
/// the second `map`'s underflow is a located error, not a panic.
#[test]
fn splice_member_call_of_hkt_member_is_located_error() {
    let err = check_src(
        "type: Opt['T] | None | Some 'T ;\n\
             trait: Functor['F: * -> *] :\n\
             map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             : twice['F: Functor 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) map map ;\n",
    )
    .expect_err("map map needs a second quotation");
    // The located underflow at the second `map` (line 5) -- the body
    // check rejects the shape for its own reason, having already unified
    // the first App-headed operand without panicking.
    assert!(
        err.contains("stack effect mismatch in `twice` (line 5)"),
        "{err}"
    );
    assert!(
        err.contains("`map` needs 2 values, but the stack holds 1"),
        "{err}"
    );
}

fn obligations_of(src: &str) -> HashMap<String, Vec<TraitObligation>> {
    let (_, recorded) = checked_like_a_build(src).expect("the fixture checks");
    recorded
        .into_iter()
        .map(|w| (w.name, w.obligations))
        .collect()
}

/// A trait, a concrete implementing word, and the `impl:` binding them --
/// the preamble every bound-dispatch fixture below needs. `Point` rather
/// than `i64` because a scalar local has no address to borrow.
const SHOW: &str = "type: Point x i64 y i64 ;\n\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for Point\n\
           : show | p | p drop ;\n\
         ;\n";

/// P7.S3e (R7): a bounded body's member call records an obligation --
/// which trait, which member, which of the word's own type variables --
/// and no symbol: `'T` is still abstract at this point.
#[test]
fn trait_member_call_records_an_obligation() {
    let recorded = obligations_of(&format!(
        "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n: main ( -- ) ;\n"
    ));
    let obs = recorded
        .get("shows")
        .expect("the bounded word was pre-passed");
    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].var, 0);
    assert_eq!(obs[0].member, "show");
    assert_eq!(obs[0].span.line, 6);
    // Index 1: the pre-seeded `Copy` predicate entry occupies 0, `Show`
    // (declared in this fixture's own source, ahead of the appended
    // `core::cmp`, which declares its own `Ord` after it) is next -- a
    // whole-program `TraitId` is what was recorded, not a per-module or
    // per-word one.
    assert_eq!(obs[0].trait_id, TraitId::from_index(1));
}

/// The obligation list is keyed by every non-combinator poly word the
/// pre-pass reached, not only the ones that recorded something -- an
/// absent key means "never pre-passed", which is what makes R17's
/// order-independence claim checkable rather than indistinguishable from
/// "recorded nothing".
#[test]
fn the_prepass_keys_every_noncombinator_poly_word() {
    let recorded = obligations_of(&format!(
        "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : ident ( 'U -- 'U ) ;\n\
             : main ( -- ) ;\n"
    ));
    assert_eq!(recorded["ident"], Vec::new());
    assert_eq!(recorded["shows"].len(), 1);
}

/// R17/decision 10: the obligation is recorded in both source orders --
/// the bounded body is reached whether its monomorphic caller is declared
/// before or after it.
///
/// This does *not* pin the hoist. The map is fully populated by the time
/// `check_module` returns either way, so relocating the pre-pass to after
/// the main word loop leaves this test green; that the obligation is
/// recorded *early enough* only becomes observable in Phase 3, once the
/// call-site bound loop consumes it. The hoist's own witness is
/// `check::tests::a_poly_body_diagnostic_precedes_a_monomorphic_one_declared_before_it`.
#[test]
fn the_obligation_is_recorded_in_either_declaration_order() {
    let caller_first = format!(
        "{SHOW}: main ( -- ) 1 2 Point |p| &p shows p drop ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n"
    );
    let callee_first = format!(
        "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows p drop ;\n"
    );
    assert_eq!(obligations_of(&caller_first)["shows"].len(), 1);
    assert_eq!(obligations_of(&callee_first)["shows"].len(), 1);
}

/// R17: the pre-pass hoist *replaces* the in-loop `check_poly_body` call
/// rather than supplementing it (documented at `src/check.rs`, above the
/// pre-pass loop). A bounded body that also mints a concrete generic
/// struct instantiation pins the claim directly: if the deleted in-loop
/// call were mistakenly restored alongside the pre-pass, the body would
/// be checked twice. (In practice `GenericTypes::instantiate_struct`
/// dedupes structurally by `(idx, module, args)` across flushes, so a
/// duplicate check of the *same* body is currently idempotent even under
/// that mutation -- confirmed by hand -- but this pins the doc's literal
/// "observed exactly once" claim and would catch a future change to that
/// dedup key.)
#[test]
fn a_generic_struct_referenced_by_a_bounded_body_mints_exactly_once() {
    let src = format!(
        "{SHOW}type: Box['T] val 'T ;\n\
             : shows ['T: Show] ( &'T -- ) show 7 Box drop ;\n\
             : main ( -- ) 1 2 Point |p| &p shows p drop ;\n"
    );
    let (module, _) = checked_like_a_build(&src).expect("the fixture checks");
    // `Point` (SHOW's own preamble) plus exactly one `Box[i64]`
    // instantiation -- two concrete structs, not three.
    assert_eq!(
        module.structs.len(),
        2,
        "Box[i64] should mint exactly once: {:#?}",
        module.structs.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

/// R7: the member's declared outputs are pushed, with the trait's own type
/// variable rewritten to the bounded variable being dispatched on.
#[test]
fn trait_member_call_pushes_the_declared_outputs() {
    check_src(
        "type: Point x i64 y i64 ;\n\
             trait: Clone['T] : clone ( &'T -- 'T ) ; ;\n\
             impl: Clone for Point\n\
               : clone | p | p drop 1 2 Point ;\n\
             ;\n\
             : cloned ['T: Clone] ( &'T -- 'T ) clone ;\n\
             : main ( -- ) ;\n",
    )
    .expect("the member's `'T` output grounds to the caller's own variable");
}

/// The output really is the *member's*, not a pass-through: declaring the
/// wrong one is a stack-shape mismatch. Asserted on the residual-stack
/// line, not just on the word name: with dispatch disabled the same
/// fixture fails as an unknown-word error that names `cloned` too.
#[test]
fn trait_member_output_is_the_declared_one() {
    let err = check_src(
        "type: Point x i64 y i64 ;\n\
             trait: Clone['T] : clone ( &'T -- 'T ) ; ;\n\
             impl: Clone for Point\n\
               : clone | p | p drop 1 2 Point ;\n\
             ;\n\
             : cloned ['T: Clone] ( &'T -- ) clone ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("cloned") && err.contains("body leaves `'T`"),
        "{err}"
    );
}

/// R7: an operand that does not match the member's declared signature is a
/// located rejection naming the trait and the member.
#[test]
fn trait_member_operand_mismatch_is_located() {
    let err = check_src(&format!(
        "{SHOW}: shows ['T: Show] ( 'T -- ) show ;\n: main ( -- ) ;\n"
    ))
    .unwrap_err();
    assert!(
        err.contains("`show` of `Show` in `shows` (line 6, col 30) expects `&'T`, found `'T`"),
        "{err}"
    );
}

/// P7.S3p: bound-directed dispatch runs ahead of `env.get`, so a concrete
/// word of the member's name does not capture the call -- the guard the
/// S3e review's mis-dispatch counterexample asked for. Pinned here rather
/// than as a golden: a real build mangles the call site to the concrete
/// word's own symbol first (S3e R18's ruled outcome), which is what
/// `tests/phase7_slice3e.rs` covers.
#[test]
fn a_member_call_beats_a_concrete_word_of_the_same_name() {
    let recorded = obligations_of(
        "type: Point x i64 y i64 ;\n\
             trait: Indexable['T] : at ( &'T i64 -- i64 ) ; ;\n\
             impl: Indexable for Point\n\
               : at | p n | n drop p &x @ ;\n\
             ;\n\
             : at ( i64 -- i64 ) 900 add ;\n\
             : uses ['T: Indexable] ( &'T -- i64 ) 0 at ;\n\
             : main ( -- ) ;\n",
    );
    let obs = &recorded["uses"];
    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].member, "at");
}

/// P7.S3p (ruling 1): the receiver need not be the member's last input.
/// `at`'s receiver is `inputs[0]`, with an `i64` above it, and the
/// dispatched variable comes off the *bound* -- pinned by declaring the
/// bounded variable second (id 1), so the stack slot a top-of-stack read
/// would have seen is the `i64` operand and no dispatch would happen at
/// all.
#[test]
fn a_non_trailing_receiver_dispatches_on_the_bound_variable() {
    let recorded = obligations_of(
        "type: Point x i64 y i64 ;\n\
             trait: Indexable['T] : at ( &'T i64 -- i64 ) ; ;\n\
             impl: Indexable for Point\n\
               : at | p n | n drop p &x @ ;\n\
             ;\n\
             : uses ['T: Indexable] ( 'U &'T -- 'U i64 ) 0 at ;\n\
             : main ( -- ) ;\n",
    );
    let obs = &recorded["uses"];
    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].member, "at");
    assert_eq!(obs[0].var, 1);
}

/// P7.S3p (ruling 2): selection is by member name alone -- a mismatched
/// operand at the receiver's position is the located member diagnostic,
/// never a fall-through to ordinary dispatch (which would report `at` as
/// an unknown word and lose the trait entirely). The bound is on a bare
/// `'T` where `at` declares `&'T`, so a structural *selection* gate would
/// decline the candidate here.
#[test]
fn a_non_trailing_receiver_mismatch_is_located_not_unknown() {
    let err = check_src(
        "trait: Indexable['T] : at ( &'T i64 -- i64 ) ; ;\n\
             : uses ['T: Indexable] ( 'T -- i64 ) 0 at ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`at` of `Indexable` in `uses` (line 2, col 40) expects `&'T`, found `'T`"),
        "{err}"
    );
}

/// `substitute_member_var` witness: every other fixture in this file puts
/// the bound variable at index 0, identical to the trait member's own
/// `'T` (also id 0 in its own `PolySig`), so an identity-stubbed rewrite
/// would pass them all. Here the bound variable (`'T`) is declared
/// *second*, at id 1, behind an unrelated `'U` at id 0 -- an identity
/// rewrite would leave `show`'s declared `&'T` input at var 0, which is
/// `'U` in this signature, not the bound `'T`, and the call would be
/// rejected as a mismatch it is not.
#[test]
fn trait_member_dispatch_rewrites_a_non_first_bound_variable() {
    check_src(&format!(
        "{SHOW}: shows ['T: Show] ( 'U &'T -- 'U ) show ;\n: main ( -- ) ;\n"
    ))
    .expect("show's receiver rewrites to 'T (id 1), not 'U (id 0)");
}

/// R12/decision 5: composing two traits that happen to require the same
/// member name is legal to *declare* -- the rejection belongs to the
/// ambiguous call, not to a body that never makes one.
#[test]
fn two_bounds_sharing_a_member_name_are_legal_to_declare() {
    check_src(
        "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             : f ['T: A B] ( &'T -- ) drop ;\n\
             : main ( -- ) ;\n",
    )
    .expect("declaring both bounds is legal without calling the shared member");
}

/// R12/decision 5: the ambiguous *call* is the error, naming both traits,
/// the member, and the bound variable.
#[test]
fn ambiguous_trait_member_call_is_rejected() {
    let err = check_src(
        "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             : f ['T: A B] ( &'T -- ) t1 ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`t1` is required by both `A` and `B` on 'T (line 3, col 26)"),
        "{err}"
    );
    assert!(
        err.contains(
            "a member required by two of a variable's bounds cannot be called unqualified"
        ),
        "{err}"
    );
}

/// P7.S3p (ruling 4, amended): two traits declaring the member on
/// *different* variables are separated by the operands the call consumes,
/// the same way S3e's top-of-stack discovery separated them -- `&'U` on
/// top means `B`'s, and the `&'T` left below means `A`'s. Both dispatch,
/// each recording its own variable.
#[test]
fn cross_variable_candidates_are_separated_by_the_operands() {
    let recorded = obligations_of(
        "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             : f ['T: A 'U: B] ( &'T &'U -- ) t1 t1 ;\n\
             : main ( -- ) ;\n",
    );
    let vars: Vec<u32> = recorded["f"].iter().map(|o| o.var).collect();
    assert_eq!(vars, vec![1, 0]);
}

/// P7.S3p (ruling 4): a candidate declaring more inputs than the stack
/// holds cannot fit, so it does not compete -- `B`'s three-input `t1` is
/// out, leaving `A`'s the unique fit. The length check is not a narrowing
/// nicety: the window base is `stack.len() - inputs.len()`, so without it a
/// candidate wider than the stack underflows and the compiler panics on a
/// program that otherwise checks fine.
#[test]
fn a_candidate_wider_than_the_stack_does_not_fit() {
    let recorded = obligations_of(
        "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T i64 i64 -- ) ; ;\n\
             : f ['U: B 'T: A] ( &'U &'T -- ) t1 drop ;\n\
             : main ( -- ) ;\n",
    );
    let vars: Vec<u32> = recorded["f"].iter().map(|o| o.var).collect();
    assert_eq!(vars, vec![1]);
}

/// P7.S3p (ruling 4): a third bound on another variable does not turn the
/// same-variable ambiguity into a resolvable call -- `'T`'s two candidates
/// fit the operands equally, so the fit is not unique and the call is
/// still rejected. Without the uniqueness requirement one of the two is
/// picked by bound declaration order.
#[test]
fn a_same_variable_ambiguity_survives_a_bound_on_another_variable() {
    let err = check_src(
        "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             trait: C['T] : t1 ( &'T -- ) ; ;\n\
             : f ['U: C 'T: A B] ( &'U &'T -- ) t1 drop ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`t1` is required by `C` on 'U, `A` on 'T and `B` on 'T"),
        "{err}"
    );
}

/// P7.S3p (ruling 4, amended): when the operands fit *no* candidate,
/// nothing at the call site picks -- but this is not the same failure as
/// an ambiguous call: no module qualifier would fix it, since every
/// candidate's declared operands already disagree with the stack. The
/// diagnostic says so instead of the "cannot be called unqualified" note,
/// which is only true when qualifying would actually help.
#[test]
fn a_cross_variable_member_call_fitting_no_candidate_names_the_operand_mismatch() {
    let err = check_src(
        "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             : f ['T: A 'U: B] ( &'T &'U i64 -- ) t1 ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`t1` is required by `A` on 'T and `B` on 'U in `f` (line 3, col 38)"),
        "{err}"
    );
    assert!(
        err.contains(
            "the operands at this call match none of their declared shapes: \
                 `A` on 'T expects `&'T` and `B` on 'U expects `&'U`"
        ),
        "{err}"
    );
    assert!(!err.contains("cannot be called unqualified"), "{err}");
}

/// P7.S3p (ruling 4, amended): the no-fit path is reachable for a member
/// whose receiver is *trailing*, a shape S3e already admitted, where it
/// used to report the single declared shape it checked against. It must
/// stay as actionable now that there are several: the note names every
/// candidate's substituted input list, and the message keeps the enclosing
/// word.
#[test]
fn a_no_fit_call_on_a_trailing_receiver_member_names_every_declared_shape() {
    let err = check_src(
        "trait: A['T] : t1 ( i64 &'T -- ) ; ;\n\
             : f ['T: A 'U: A] ( &'T &'U -- ) t1 ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`t1` is required by `A` on 'T and `A` on 'U in `f` (line 2, col 34)"),
        "{err}"
    );
    assert!(
        err.contains(
            "none of their declared shapes: \
                 `A` on 'T expects `i64 &'T` and `A` on 'U expects `i64 &'U`"
        ),
        "{err}"
    );
}

/// P7.S3p (ruling 4): position is not a disambiguator either. `A`'s `t1`
/// takes the receiver last, `B`'s takes it first, both on one variable --
/// still the single-variable ambiguity, rendered exactly as the
/// same-position case.
#[test]
fn a_multi_position_duplicate_member_is_the_single_variable_ambiguity() {
    let err = check_src(
        "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T i64 -- ) ; ;\n\
             : f ['T: A B] ( &'T -- ) t1 ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`t1` is required by both `A` and `B` on 'T (line 3, col 26)"),
        "{err}"
    );
}

/// One trait named twice on one variable is not an ambiguity with itself:
/// `parse_capabilities` admits the repeat, and the two `Bound::User`
/// entries would otherwise reach the ambiguity arm naming `A` twice.
#[test]
fn a_repeated_bound_is_not_ambiguous_with_itself() {
    let recorded = obligations_of(
        "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             : f ['T: A A] ( &'T -- ) t1 ;\n\
             : main ( -- ) ;\n",
    );
    assert_eq!(recorded["f"].len(), 1);
    assert_eq!(recorded["f"][0].member, "t1");
}

/// P7.S3o Phase 3: a bare member call (`show`) inside a bounded inline
/// combinator now resolves at the splice site via dispatch injection into
/// `check_terms_relaxed`. The standalone check (i64 stand-in) accounts for
/// the member's stack effect without requiring an `impl: Show for i64`, and
/// the actual dispatch happens at each real splice site where θ is
/// concrete. This test now compiles successfully — the gate rejection is
/// gone, and the bare member is no longer an "unknown word".
#[test]
fn user_bound_on_a_combinator_compiles() {
    check_src(&format!(
        "{SHOW}: shows inline ['T: Show] ( &'T -- ) show ;\n: main ( -- ) ;\n"
    ))
    .expect("a bounded inline combinator calling a bare member compiles");
}

/// R10 barrier 1: a plain (non-builtin) member name over a *bare* type
/// variable. Before this slice such an operand reached
/// `poly_var_to_concrete_error` unconditionally, so bound-directed
/// dispatch takes nothing an ordinary `env` lookup could have had.
/// (Barrier 3, a `Ref`-to-variable operand, is
/// `trait_member_call_records_an_obligation` above; the coexistence half
/// of R10 needs real name mangling and lives in `driver`'s tests.)
#[test]
fn bound_dispatch_on_a_bare_variable_receiver() {
    let recorded = obligations_of(
        "trait: Eat['T] : eat ( 'T -- ) ; ;\n\
             : eats ['T: Eat] ( 'T -- ) eat ;\n\
             : main ( -- ) ;\n",
    );
    assert_eq!(recorded["eats"].len(), 1);
    assert_eq!(recorded["eats"][0].member, "eat");
}

/// R10 barrier 2: a member spelled as a name the dispatch cascade
/// intercepts ahead of bound-directed dispatch. Before
/// `poly_trait_member_call` moved to the front of `poly_call_term`, this
/// member was unreachable: the comparisons block (`matches!(name, "eq" |
/// ...)`) intercepted it first and demanded an `Ord` bound the trait never
/// declared. `main` carries R10's coexistence half for this barrier: the
/// builtin still wins a concrete receiver.
///
/// The six surface comparisons are the whole of that barrier now. P7.S3r
/// (R4) rejects a member spelled as a *name-dispatched* builtin at the
/// `trait:` declaration, so the operator-spelled sibling of this fixture (a
/// `Sum` trait with an `add` member) is no longer declarable -- and it never
/// exercised the partition anyway, since no dispatch-cascade arm matches
/// `add`.
#[test]
fn bound_dispatch_reaches_a_member_named_after_an_intercepting_builtin() {
    let recorded = obligations_of(
        "trait: Eq['T] : eq ( 'T 'T -- i64 ) ; ;\n\
             : eqs ['T: Eq] ( 'T 'T -- i64 ) eq ;\n\
             : main ( -- ) 1 2 eq drop ;\n",
    );
    assert_eq!(recorded["eqs"].len(), 1);
    assert_eq!(recorded["eqs"][0].member, "eq");
}

/// Two types, both satisfying one trait through their own `impl:` -- the
/// preamble the call-site resolution tests need (R8). A `shows` declared
/// after it lands on line 10, and its `show` call is the obligation's
/// span.
const TWO_SHOWS: &str = "type: Point x i64 y i64 ;\n\
         type: Blip n i64 ;\n\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for Point\n\
           : show | p | p drop ;\n\
         ;\n\
         impl: Show for Blip\n\
           : show | b | b drop ;\n\
         ;\n";

/// P7.S3e (R8/R9): the load-bearing new mechanism, read directly rather
/// than through a golden (a bound-directed call does not lower until
/// Phase 4). The call site resolves the obligation its callee's body
/// recorded against its own theta and records the implementing word's
/// lowering symbol, keyed by the *body* span that dispatched it -- never
/// the caller's.
#[test]
fn a_satisfied_bound_resolves_to_the_implementing_words_symbol() {
    let (module, _) = checked_like_a_build(&format!(
        "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows p drop ;\n"
    ))
    .expect("the fixture checks");
    let inst = module
        .instantiations
        .values()
        .find(|i| i.callee == "shows")
        .expect("the call site recorded an instantiation");
    let resolved: Vec<(u32, &str)> = inst
        .trait_calls
        .iter()
        .map(|(span, symbol)| (span.line, symbol.as_str()))
        .collect();
    assert_eq!(resolved, vec![(6, "show;Show;0;Point")]);
}

/// R8: two instantiations of one bounded word resolve to two distinct
/// symbols -- the same body span, under each instantiation's own
/// `CallInst`, which is what "per-instantiation" means.
#[test]
fn two_instantiations_resolve_to_two_distinct_symbols() {
    let (module, _) = checked_like_a_build(&format!(
        "{TWO_SHOWS}: shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows p drop 7 Blip |b| &b shows b drop ;\n"
    ))
    .expect("the fixture checks");
    let mut resolved: Vec<(String, u32, String)> = module
        .instantiations
        .values()
        .filter(|i| i.callee == "shows")
        .flat_map(|i| {
            let ty = i.subst.ty_of(0).expect("'T is grounded");
            i.trait_calls
                .iter()
                .map(move |(span, symbol)| (ty.name().to_string(), span.line, symbol.clone()))
        })
        .collect();
    resolved.sort();
    assert_eq!(
        resolved,
        vec![
            ("Blip".to_string(), 10, "show;Show;0;Blip".to_string()),
            ("Point".to_string(), 10, "show;Show;0;Point".to_string()),
        ]
    );
}

/// R8: which member the obligation names selects the binding. A trait with
/// two members, a body calling only the second, and two distinct member
/// bodies: resolving by position rather than by member name would
/// dispatch `hash` to `eq`'s synthesized word.
#[test]
fn the_obligations_member_name_selects_the_binding() {
    let (module, _) = checked_like_a_build(
        "type: Point x i64 y i64 ;\n\
             trait: Eq['T] : eq ( &'T &'T -- i64 ) ; : hash ( &'T -- i64 ) ; ;\n\
             impl: Eq for Point\n\
               : eq | a b | a drop b drop 1 ;\n\
               : hash | p | p drop 7 ;\n\
             ;\n\
             : hashes ['T: Eq] ( &'T -- i64 ) hash ;\n\
             : main ( -- ) 1 2 Point |p| &p hashes drop p drop ;\n",
    )
    .expect("the fixture checks");
    let resolved: Vec<&String> = module
        .instantiations
        .values()
        .filter(|i| i.callee == "hashes")
        .flat_map(|i| i.trait_calls.values())
        .collect();
    assert_eq!(resolved, vec!["hash;Eq;0;Point"]);
}

/// R8: a polymorphic *overload set* -- two bounded words sharing one name,
/// which is legal since their declared inputs differ. Each call site must
/// read back its own callee's obligations: they are recorded per
/// `(name, signature)`, and a name-keyed lookup would hand both call sites
/// the first candidate's obligation, resolving a body span belonging to a
/// word that was never called.
#[test]
fn each_overload_of_one_name_resolves_its_own_bodys_obligation() {
    let (module, _) = checked_like_a_build(&format!(
        "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : shows ['T: Show] ( &'T i64 -- ) drop show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows &p 3 shows p drop ;\n"
    ))
    .expect("the fixture checks");
    let mut sites: Vec<(u32, Vec<u32>)> = module
        .instantiations
        .iter()
        .filter(|(_, i)| i.callee == "shows")
        .map(|(span, i)| {
            let mut lines: Vec<u32> = i.trait_calls.keys().map(|s| s.line).collect();
            lines.sort();
            (span.col, lines)
        })
        .collect();
    sites.sort();
    // The one-input `shows` is called first, so it holds the lower column;
    // its `show` is on line 6, the two-input one's on line 7.
    assert_eq!(sites.len(), 2, "{sites:?}");
    assert_eq!(sites[0].1, vec![6], "{sites:?}");
    assert_eq!(sites[1].1, vec![7], "{sites:?}");
}

/// R8: two distinct bound variables on one word, each obligated to a
/// different trait, both resolved in one call -- the trait axis of
/// `resolve_user_bound`'s `.filter(|o| o.trait_id == trait_id && o.var ==
/// v)`. The two obligations differ in *both* conjuncts here, so this
/// fixture alone does not discriminate: the variable axis is pinned by
/// `one_trait_on_two_variables_resolves_each_span_against_its_own_theta`
/// and the trait axis by
/// `two_traits_on_one_variable_resolve_against_their_own_impl`.
#[test]
fn two_bounds_on_distinct_variables_each_resolve_their_own_obligation() {
    let (module, _) = checked_like_a_build(
        "type: PA n i64 ;\n\
             type: PB n i64 ;\n\
             trait: A['T] : ta ( &'T -- ) ; ;\n\
             trait: B['T] : tb ( &'T -- ) ; ;\n\
             impl: A for PA\n\
               : ta | p | p drop ;\n\
             ;\n\
             impl: B for PB\n\
               : tb | p | p drop ;\n\
             ;\n\
             : f ['T: A 'U: B] ( &'T &'U -- ) tb ta ;\n\
             : main ( -- ) 1 PA |a| 1 PB |b| &a &b f a drop b drop ;\n",
    )
    .expect("the fixture checks");
    let inst = module
        .instantiations
        .values()
        .find(|i| i.callee == "f")
        .expect("the call site recorded an instantiation");
    let mut resolved: Vec<&str> = inst.trait_calls.values().map(String::as_str).collect();
    resolved.sort();
    assert_eq!(resolved, vec!["ta;A;0;PA", "tb;B;0;PB"]);
}

/// R8: one trait, two bound variables, instantiated at two types that
/// each implement it. Both obligations name the same trait, so
/// `o.var == v` is the only conjunct separating them: without it each
/// bound's loop resolves *both* body spans against its own theta, and the
/// `'T` dispatch silently gets `'U`'s implementing word.
#[test]
fn one_trait_on_two_variables_resolves_each_span_against_its_own_theta() {
    let (module, _) = checked_like_a_build(
        "type: PA n i64 ;\n\
             type: PB n i64 ;\n\
             trait: A['T] : ta ( &'T -- ) ; ;\n\
             impl: A for PA\n\
               : ta | p | p drop ;\n\
             ;\n\
             impl: A for PB\n\
               : ta | p | p drop ;\n\
             ;\n\
             : f ['T: A 'U: A] ( &'T &'U -- ) ta ta ;\n\
             : main ( -- ) 1 PA |a| 1 PB |b| &a &b f a drop b drop ;\n",
    )
    .expect("the fixture checks");
    let inst = module
        .instantiations
        .values()
        .find(|i| i.callee == "f")
        .expect("the call site recorded an instantiation");
    let mut resolved: Vec<(u32, &str)> = inst
        .trait_calls
        .iter()
        .map(|(span, symbol)| (span.col, symbol.as_str()))
        .collect();
    resolved.sort();
    // The body's first `ta` (col 26) consumes the top input, `'U` = `PB`;
    // the second (col 29) consumes `'T` = `PA`.
    assert_eq!(resolved, vec![(34, "ta;A;0;PB"), (37, "ta;A;0;PA")]);
}

/// R8: two traits bounding *one* variable, both implemented for the type
/// it is instantiated at. Each bound's loop must see only its own trait's
/// obligation and only its own trait's `impl:`: dropping either
/// `trait_id` comparison (the obligation filter's or the registry
/// lookup's) makes one loop hunt for a member the other trait's impl
/// binds, and this legal program is rejected by R17's
/// internal-consistency error.
#[test]
fn two_traits_on_one_variable_resolve_against_their_own_impl() {
    let (module, _) = checked_like_a_build(
        "type: PA n i64 ;\n\
             trait: A['T] : ta ( &'T -- ) ; ;\n\
             trait: B['T] : tb ( &'T -- ) ; ;\n\
             impl: A for PA\n\
               : ta | p | p drop ;\n\
             ;\n\
             impl: B for PA\n\
               : tb | p | p drop ;\n\
             ;\n\
             : f ['T: A B] ( &'T &'T -- ) tb ta ;\n\
             : main ( -- ) 1 PA |a| &a &a f a drop ;\n",
    )
    .expect("the fixture checks");
    let inst = module
        .instantiations
        .values()
        .find(|i| i.callee == "f")
        .expect("the call site recorded an instantiation");
    let mut resolved: Vec<(u32, &str)> = inst
        .trait_calls
        .iter()
        .map(|(span, symbol)| (span.col, symbol.as_str()))
        .collect();
    resolved.sort();
    assert_eq!(resolved, vec![(30, "tb;B;0;PA"), (33, "ta;A;0;PA")]);
}

/// R8: a polymorphic combinator's body calling a bounded poly word. The
/// combinator itself is checked standalone and records nothing that
/// survives, but the bound it dispatches through is real -- so that path
/// must be handed the whole-program trait/impl tables, not the scratch
/// ones (whose trait table a user `TraitId` indexes past the end).
#[test]
fn a_bounded_call_inside_a_combinator_body_resolves() {
    checked_like_a_build(&format!(
        "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : appq inline ( &Point ~[ -- ] -- ) | f | f call shows ;\n\
             : main ( -- ) 1 2 Point |p| &p ~[ ] appq p drop ;\n"
    ))
    .expect("a satisfied bound dispatched from a combinator body checks");
}

/// R8/R9: `CallInst::trait_calls` is a pure function of `(callee, theta)`
/// -- its keys are the callee's own body spans, never the caller's -- so
/// two call sites at one instantiation record identical maps. Phase 4's
/// symbol-dedup step reads whichever it reaches first, which is only
/// sound if that holds.
#[test]
fn two_call_sites_at_one_instantiation_record_identical_maps() {
    let (module, _) = checked_like_a_build(&format!(
        "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows &p shows p drop ;\n"
    ))
    .expect("the fixture checks");
    let maps: Vec<&HashMap<Span, String>> = module
        .instantiations
        .values()
        .filter(|i| i.callee == "shows")
        .map(|i| &i.trait_calls)
        .collect();
    assert_eq!(maps.len(), 2, "two call sites");
    assert_eq!(maps[0], maps[1]);
    assert_eq!(
        maps[0].values().collect::<Vec<_>>(),
        vec!["show;Show;0;Point"]
    );
}

/// R8: the concrete type a bounded variable was instantiated with has no
/// `impl:` for the trait the bound names.
#[test]
fn an_unsatisfied_user_bound_names_the_missing_member_signature() {
    let err = check_src(&format!(
        "{SHOW}type: Blip n i64 ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 Blip |b| &b shows b drop ;\n"
    ))
    .unwrap_err();
    assert!(
        err.contains(
            "error: cannot instantiate `'T` of `shows` with `Blip` in `main` (line 8, col 29)"
        ),
        "{err}"
    );
    assert!(
        err.contains("`Blip` does not satisfy `Show`: no `( &Blip -- )` found"),
        "{err}"
    );
}

/// R8: every member's grounded signature is listed, not only the first --
/// an unsatisfied bound says what an `impl:` would have to provide in
/// full.
#[test]
fn an_unsatisfied_multi_member_bound_lists_every_member_signature() {
    let err = check_src(
        "type: Point x i64 y i64 ;\n\
             trait: Eq['T] : eq ( &'T &'T -- i64 ) ; : hash ( &'T -- i64 ) ; ;\n\
             : eqs ['T: Eq] ( &'T &'T -- i64 ) eq ;\n\
             : main ( -- ) 1 2 Point |p| &p &p eqs drop p drop ;\n",
    )
    .unwrap_err();
    assert!(
            err.contains(
                "`Point` does not satisfy `Eq`: no `( &Point &Point -- i64 )`, `( &Point -- i64 )` found"
            ),
            "{err}"
        );
}

/// R17: the backstop for a satisfied bound whose recorded obligation
/// resolves to nothing. `check_impl_decls` rejects an `impl:` binding no
/// word for a required member at its own declaration site, so the only way
/// into this state is to skip that check -- which is what this asserts:
/// the fixture that resolves cleanly in
/// `a_satisfied_bound_resolves_to_the_implementing_words_symbol` becomes a
/// located error, not a silently dropped call, when the two disagree.
///
/// P7.S3s (review fix): `parse_with_core` now runs `check_impl_decls` on
/// every caller's module (needed so `core::cmp`'s own `impl: Ord` blocks
/// resolve `cmp`), so this fixture's `impl: Show for Point` -- which does
/// bind `show` correctly -- resolves cleanly through that same call, the
/// same as it would in a real build. Reaching R17's backstop deliberately
/// now means undoing just that one resolution afterwards, on the `Show`
/// impl alone, rather than skipping the whole-module check `core::cmp`
/// still needs.
#[test]
fn an_unresolvable_obligation_on_a_satisfied_bound_is_a_located_error() {
    let src = format!(
        "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows p drop ;\n"
    );
    let tokens = lex(&src).unwrap();
    let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
    let show_impl = module
        .impls
        .iter_mut()
        .find(|i| {
            i.target.pattern
                == crate::ast::PolyType::Concrete(crate::ast::Type::Struct(
                    StructId::from_index(0),
                    "Point",
                ))
        })
        .expect("the fixture's `impl: Show for Point` parsed");
    assert!(
        !show_impl.resolved.is_empty(),
        "parse_with_core's check_impl_decls should have resolved `show`"
    );
    show_impl.resolved.clear();
    let err = check(&mut module).unwrap_err();
    assert_eq!(
            err,
            "error: `impl: Show for Point` binds no word for member `show`, dispatched at line 6, col 31 in the body of `shows` (instantiated at line 7, col 32 in `main`)"
        );
}

// P7.S4b (R12): unit tests for the factored `find_bound_impl`, the
// `candidate_bounds_discharge` filter, and the `bound_cycle_error`
// cycle detector.

/// R6: a bounded generic impl's `where`-clause bound discharges at a
/// concrete instantiation via recursive `find_bound_impl` — the element
/// type has its own concrete impl, so the candidate is kept and the program
/// compiles.
#[test]
fn find_bound_impl_recursive_discharge_succeeds() {
    let src = "type: Point x i64 y i64 ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             trait: Print['T] : print ( &'T -- ) ; ;\n\
             impl: Print for Point\n\
               : print | p | p drop ;\n\
             ;\n\
             impl: Show for array['T 4] where 'T: Print\n\
               : show | a | a drop ;\n\
             ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- )\n\
               1 2 Point |p|\n\
               p 4 fill |arr|\n\
               &arr shows\n\
               arr drop\n\
               p drop\n\
             ;\n";
    check_src(src).expect("bounded generic impl should compile when the bound discharges");
}

/// R6 edge: a bounded generic impl whose `where`-clause bound fails to
/// discharge (no matching impl for the concrete element type) is excluded
/// from the candidate set, leaving no candidate and producing the
/// unsatisfied-bound error.
#[test]
fn find_bound_impl_recursive_discharge_fails_when_bound_unmet() {
    let src = "type: Point x i64 y i64 ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             trait: Print['T] : print ( &'T -- ) ; ;\n\
             impl: Show for array['T 4] where 'T: Print\n\
               : show | a | a drop ;\n\
             ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- )\n\
               1 2 Point |p|\n\
               p 4 fill |arr|\n\
               &arr shows\n\
               arr drop\n\
               p drop\n\
             ;\n";
    let err = check_src(src).unwrap_err();
    assert!(
        err.contains("cannot instantiate `'T` of `shows` with `array[Point 4]`"),
        "{err}"
    );
    assert!(
        err.contains("`array[Point 4]` does not satisfy `Show`"),
        "{err}"
    );
}

/// R7: a self-referential bound cycle — `impl: Show for 'T where 'T: Show`
/// — is a located error at the impl declaration, not a stack overflow or
/// hang.
#[test]
fn bound_cycle_error_self_referential_impl_is_located_error() {
    let src = "type: Point x i64 y i64 ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for 'T where 'T: Show\n\
               : show | a | a drop ;\n\
             ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- )\n\
               1 2 Point |p|\n\
               &p shows\n\
               p drop\n\
             ;\n";
    let err = check_src(src).unwrap_err();
    assert!(err.contains("bound-discharge cycle"), "{err}");
    assert!(
        err.contains("`impl: Show for 'T` requires `Show for Point`"),
        "{err}"
    );
}

/// R7: a transitive cycle — `impl: A for 'T where 'T: B` and `impl: B for
/// 'T where 'T: A` — is a located error, not a hang.
#[test]
fn bound_cycle_error_transitive_cycle_is_located_error() {
    let src = "type: Point x i64 y i64 ;\n\
             trait: A['T] : a ( &'T -- ) ; ;\n\
             trait: B['T] : b ( &'T -- ) ; ;\n\
             impl: A for 'T where 'T: B\n\
               : a | x | x drop ;\n\
             ;\n\
             impl: B for 'T where 'T: A\n\
               : b | x | x drop ;\n\
             ;\n\
             : calls_a ['T: A] ( &'T -- ) a ;\n\
             : main ( -- )\n\
               1 2 Point |p|\n\
               &p calls_a\n\
               p drop\n\
             ;\n";
    let err = check_src(src).unwrap_err();
    assert!(err.contains("bound-discharge cycle"), "{err}");
}

/// R6: a `Copy` bound on a candidate's own variable is checked during
/// candidate filtering — a linear element type fails the `Copy` bound,
/// excluding the candidate.
#[test]
fn candidate_bounds_discharge_copy_bound_excludes_linear_type() {
    let src = "type: Spy tag i64 ;\n\
             : drop ( Spy -- ) | s | s Spy> drop ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for 'T where 'T: Copy\n\
               : show | a | a drop ;\n\
             ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- )\n\
               0 Spy |s|\n\
               &s shows\n\
               s drop\n\
             ;\n";
    let err = check_src(src).unwrap_err();
    assert!(
        err.contains("cannot instantiate `'T` of `shows` with `Spy`"),
        "{err}"
    );
}

/// R7: two independent calls to a bounded word both resolve the same
/// `(TraitId, Type)` via `find_bound_impl`, and neither interferes with
/// the other — the path-scoped visited-set is fresh for each top-level
/// `resolve_user_bound` call, so no false-positive cycle.
#[test]
fn bound_cycle_no_false_positive_on_independent_resolutions() {
    let src = "type: Point x i64 y i64 ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             trait: Print['T] : print ( &'T -- ) ; ;\n\
             impl: Print for Point\n\
               : print | p | p drop ;\n\
             ;\n\
             impl: Show for array['T 4] where 'T: Print\n\
               : show | a | a drop ;\n\
             ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- )\n\
               1 2 Point |p|\n\
               p 4 fill |arr|\n\
               &arr shows\n\
               &arr shows\n\
               arr drop\n\
               p drop\n\
             ;\n";
    check_src(src).expect("two independent resolutions should not be a cycle");
}

// A one-field struct with a `drop` overload: linear for the same reason any
// resource is, used to force the `Copy`-bound failure (X5).
const SPY: &str = "type: Spy tag i64 ;\n: drop ( Spy -- ) | s | s Spy> drop ;\n";
/// D3's leaf resource: one field, a `drop` override implemented exactly
/// as `examples/resources.sth`'s `Fd` (extracting the field via `Fd>`
/// inside `drop`'s own body -- exempted, since a word literally named
/// `drop` can only be the recognized override for the struct its declared
/// effect names).
const FD_DEF: &str = "type: Fd n i64 ;\n: drop ( Fd -- ) | h | h Fd> drop ;\n";
fn probe_word() -> WordDef {
    crate::test_support::bare_word("probe", 0)
}
/// `probe_word`'s `Ctx`. Separate from `probe_word` because `word_ctx`
/// borrows the `WordDef`, so the caller has to own it.
fn probe_ctx(word: &WordDef) -> Ctx<'_> {
    word_ctx(word, &[], &[], &[], None, &CombinatorIndex::new(), None)
}
/// A signature over no variables, for the unit tests that drive
/// `poly_term` directly rather than through a source program.
fn bare_sig() -> PolySig {
    PolySig {
        row_in: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: None,
        bounds: Vec::new(),
        ty_var_names: Vec::new(),
        ty_var_spans: Vec::new(),
        ty_kinds: Vec::new(),
        len_var_names: Vec::new(),
        len_var_spans: Vec::new(),
        row_var_names: Vec::new(),
    }
}
/// A checked module, for the tests that read a type fact back out of the
/// registries rather than only asserting a diagnostic.
fn checked_module(src: &str) -> Module {
    let tokens = lex(src).unwrap();
    let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
    check(&mut module).unwrap();
    module
}
/// P7 slice 3a phase 2 (R2/R4): the anti-placebo test for asymmetric
/// instantiation -- `unify_poly_input`'s `Generic` arm must bind each
/// header argument *positionally*, not just check that some binding
/// exists. A poly word consuming `Result['T 'E]` is called at both
/// `Result[i64 str]` and its swap `Result[str i64]`; if the arm collapsed
/// positional order (bound `'T`/`'E` from the wrong slot, or from a
/// symmetric key that cannot tell the two apart), one of the two calls
/// would bind `'T`/`'E` to the wrong concrete type, and this checks the
/// whole program still type-checks and runs.
#[test]
fn unify_poly_generic_binds_arguments_positionally() {
    // `show_is`/`show_si`'s own fully-concrete signatures mint
    // `Result[i64 str]`/`Result[str i64]` at parse time (R1's fold), the
    // same route `Err`'s two calls below resolve their constructor
    // through -- this test is about `reorder`'s own `unify_poly_input`
    // arm binding each swapped instantiation's arguments correctly, not
    // about R3 construction.
    // Strict-grounding amendment (260910): the bare `Err` used to
    // resolve through those parse-time mints (the retired sole-compatible
    // scope borrow); the calls now name their instantiations explicitly,
    // which leaves `reorder`'s two asymmetric instantiations -- this
    // test's actual subject -- byte-identical.
    let module = checked_module(
        "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : reorder ( 'T Result['T 'E] -- Result['T 'E] 'T ) swap ;\n\
             : show_is ( Result[i64 str] -- ) drop ;\n\
             : show_si ( Result[str i64] -- ) drop ;\n\
             : main ( -- )\n\
               1 \"boom\" Err[i64 str] reorder drop show_is\n\
               \"one\" 2 Err[str i64] reorder drop show_si ;\n",
    );
    assert!(module
        .words
        .iter()
        .any(|w| w.name == "reorder" && w.poly.is_some()));
}

/// A signature over two variables, `'F` (the application head, id 0)
/// and `'T` (its argument, id 1) -- the fixture the S1-10/S1-11 `App`
/// unit tests share.
fn app_sig() -> PolySig {
    PolySig {
        row_in: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: None,
        bounds: Vec::new(),
        ty_var_names: vec!["'F".to_string(), "'T".to_string()],
        ty_var_spans: vec![Span::default(), Span::default()],
        ty_kinds: Vec::new(),
        len_var_names: Vec::new(),
        len_var_spans: Vec::new(),
        row_var_names: Vec::new(),
    }
}

/// S1-17.i: a poly *cross-call* with an `App` slot stays a located
/// "unsupported" rejection, not a bare mismatch -- S2 owns
/// constructor-keyed dispatch.
#[test]
fn poly_cross_match_app_slot_is_unsupported_not_a_panic() {
    let callee_sig = app_sig();
    let caller_sig = app_sig();
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let mut mapping = Vec::new();
    let err = poly_cross_match(
        &PolyType::App {
            head: 0,
            args: vec![PolyType::Var(1)],
        },
        &PolyType::Var(0),
        &mut mapping,
        &callee_sig,
        &caller_sig,
        "f",
        Span::default(),
        &ctx,
    )
    .expect_err("an App-shaped declared slot must be rejected, not accepted");
    assert!(err.contains("supported"), "{err}");
}

/// S1-12 (R5): two call sites binding `'F` to *distinct* constructors
/// mint distinct mangled symbols -- the last-write-wins hazard
/// `CtorImage` resolves one abstraction level up from S12's own defect
/// -- while a duplicate call to the *same* constructor dedups onto one
/// symbol.
#[test]
fn hkt_two_constructor_call_sites_mint_distinct_symbols() {
    let module = checked_module(
        "type: Box['T] val 'T ;\n\
             type: Cell2['T] val 'T ;\n\
             type: SeedBox b Box[i64] ;\n\
             type: SeedCell c Cell2[i64] ;\n\
             : pass['F 'T] ( 'F['T] -- 'F['T] ) ;\n\
             : main ( -- )\n\
               5 Box pass drop\n\
               5 Box pass drop\n\
               5 Cell2 pass drop\n\
             ;\n",
    );
    let symbols: std::collections::HashSet<&str> = module
        .instantiations
        .values()
        .filter(|c| c.callee == "pass")
        .map(|c| c.symbol.as_str())
        .collect();
    assert_eq!(
            symbols.len(),
            2,
            "two distinct constructors (Box, Cell2) must mint two distinct symbols, and the duplicate Box call must dedup onto one of them"
        );
}

/// P7 slice 3a (R3): a poly word constructs a generic value whose header
/// argument the operand alone does not determine (`Err`'s payload never
/// mentions `Result`'s `'T`) -- the load-bearing case for the phantom-
/// argument backstop: the missing argument is recovered from the
/// enclosing word's own declared output naming the same header.
#[test]
fn poly_body_constructor_resolves_arguments_from_the_declared_output() {
    check_src(
        "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : wrap ( 'T -- Result['T i64] ) Ok ;\n\
             : main ( -- ) True wrap drop ;\n",
    )
    .expect("a phantom argument recovers from the declared output");
}

/// P7b.S2 (S2-13/F14): a zero-field variant ctor in a polymorphic arm
/// unifies with the ambient type variable -- the same binding a
/// field-carrying ctor gets from its operands, taken here from the
/// declared output naming the header. A concrete instantiation
/// registered elsewhere in the program (`mknone`'s declared output
/// registers `Option[i64]`) used to capture the arm's ctor call through
/// its generated zero-field word -- an empty input row trivially
/// exact-matches `poly_env_exact_match` -- and mint the mono
/// `Option[i64]` against the Some arm's `Option['U]`: the
/// arms-disagree error anchored at `poly_arm_output_disagreement_error`.
/// The fix routes the fieldless ctor through the symbolic path
/// unconditionally, so the registered instantiation is irrelevant.
#[test]
fn zero_field_ctor_unifies_with_ambient_var_in_poly_arm() {
    check_src(
        "type: Option['T] | None | Some 'T ;\n\
             : mapover['T 'U] ( Option['T] [ 'T -- 'U ] -- Option['U] )\n\
               swap\n\
               ~[ ( Some ) Some> swap call Some ]\n\
               ~[ ( None ) drop drop None ]\n\
               Option? ;\n\
             : mknone ( -- Option[i64] ) None ;\n\
             : main ( -- ) ;\n",
    )
    .expect("a zero-field ctor arm unifies with the ambient variable");
}

/// P7 slice 3a (R5.2): a generic constructor call whose header variable
/// is determined by neither its operands nor the enclosing word's
/// declared output (which does not name the header at all here) is a
/// located error, not a latent monomorphization failure.
#[test]
fn poly_body_constructor_undetermined_argument_is_error() {
    let err = check_src(
        "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : bad ( 'T i64 -- 'T ) Err drop ;\n\
             : main ( -- ) 1 2 bad drop ;\n",
    )
    .unwrap_err();
    assert!(err.contains("leaves the type variable"), "{err}");
    assert!(err.contains("'T"), "{err}");
}

/// P7 slice 3a (R5.3): a generic constructor call whose operands
/// disagree with each other over the header argument they both bind
/// (two fields sharing one type variable, called with two different
/// concrete types) is reported at the constructor call, during body
/// check, never deferred into a later synthesis/monomorphization step.
#[test]
fn poly_body_constructor_operand_mismatch_is_error() {
    let err = check_src(
        "type: Pair['T] val1 'T val2 'T ;\n\
             : mk ( 'T -- Pair['T] ) 1 swap Pair ;\n\
             : main ( -- ) \"oops\" mk drop ;\n",
    )
    .unwrap_err();
    assert!(err.contains("type mismatch in `mk`"), "{err}");
}

/// Slice 10a (R1): a fully-concrete `~` folds to `Concrete(InlineQuotation)`,
/// which the routing predicate must recognize -- else the word is not a
/// combinator, is lowered as an ordinary call, and reaches `ir_type_of`'s
/// `unreachable!`. Constructed directly, no parser.
#[test]
fn poly_input_is_quotation_recognizes_inline() {
    let inl = crate::ast::inline_quotation_type(vec![Type::I64], Vec::new());
    let ord = crate::ast::quotation_type(vec![Type::I64], Vec::new());
    assert!(poly_input_is_quotation(&PolyType::Concrete(inl)));
    assert!(poly_input_is_quotation(&PolyType::Concrete(ord)));
    assert!(!poly_input_is_quotation(&PolyType::Concrete(Type::I64)));
}
#[test]
fn poly_body_destructuring_drop_overloaded_type_is_error() {
    // Bug 2 (round-1 review): `poly_call_term` resolved a generated
    // accessor through the ordinary `env` lookup with no D3 guard at all,
    // so a generic word could destructure any drop-overloaded type and
    // skip its destructor.
    let err = check_src(&format!(
        "{FD_DEF}: sneak ( 'T -- 'T i64 ) 7 Fd Fd> ;\n: main ( -- ) 1 sneak drop drop ;\n"
    ))
    .unwrap_err();
    assert_eq!(
            err,
            "error: cannot destructure `Fd` in `sneak` (line 3): it defines `drop`, so moving its fields out would skip its destructor\n  note: dispose it with `drop`, or read a field through a borrow (`&`) instead of moving it out"
        );
}
#[test]
fn check_poly_call_rejects_a_quotation_argument() {
    // R9p: `check_poly_call` reads only `stack[base + i].ty`, so a quotation
    // does not *fail* unification, it *succeeds* binding `'T` to the
    // placeholder and monomorphizes a real call over a phantom. The guard
    // before `unify_poly_input` is what makes the R9 rejection reachable.
    let err = check_src(
        ": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;\n\
             : main ( -- ) [ add ] dupit drop drop ;\n",
    )
    .expect_err("a quotation passed to a polymorphic word should be rejected");
    assert!(
        err.contains("a quotation cannot be passed to `dupit`"),
        "check_poly_call should name `dupit`, got: {err}"
    );
}
/// R4: `reject_quotation_argument`'s new exact wording at `check_poly_call`'s
/// own R9p call site (a bare `PolyType::Var` position) -- the "slice 7"
/// parenthetical is retired, and the rest of the message is unchanged.
#[test]
fn reject_quotation_argument_wording_at_poly_var_position() {
    let err = check_src(
        ": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;\n\
             : main ( -- ) [ add ] dupit drop drop ;\n",
    )
    .expect_err("a quotation passed to a polymorphic word should be rejected");
    assert_eq!(
            err,
            "error: a quotation cannot be passed to `dupit`; only `call` accepts one in `main` (line 2)"
        );
}
/// P7 slice 3f (R1/R2): a `Known` literal quotation argument at a declared
/// ground `Type::Quotation` input materializes and the call succeeds, with
/// the quotation-typed input first among the declared inputs.
#[test]
fn check_poly_call_materializes_ground_quotation_first_position() {
    check_src(
        ": run_it_first ['T: Copy] ( [ i64 -- i64 ] 'T -- 'T ) swap drop ;\n\
             : main ( -- ) [ 1 add ] 7 run_it_first drop ;\n",
    )
    .expect("a ground quotation argument in the first position should materialize");
}
#[test]
fn check_poly_call_materializes_ground_quotation_middle_position() {
    check_src(
        ": run_it_mid ['T: Copy] ( 'T [ i64 -- i64 ] Bool -- 'T ) drop drop ;\n\
             : main ( -- ) 7 [ 1 add ] True run_it_mid drop ;\n",
    )
    .expect("a ground quotation argument in the middle position should materialize");
}
#[test]
fn check_poly_call_materializes_ground_quotation_last_position() {
    check_src(
        ": run_it_last ['T: Copy] ( 'T [ i64 -- i64 ] -- 'T ) drop ;\n\
             : main ( -- ) 7 [ 1 add ] run_it_last drop ;\n",
    )
    .expect("a ground quotation argument in the last position should materialize");
}
/// P7.S3l phase 2 (R9p closure): flipped from a rejection to an accept
/// case. A declared quotation whose brackets mention `'T` no longer
/// falls through R9p's blanket rejection once every variable the row
/// mentions is already bound by an earlier input (here, `'T` is bound by
/// the plain `7` argument before the quotation argument is reached) --
/// the literal is grounded through that `subst` and materialized exactly
/// as a ground declared quotation slot is.
#[test]
fn check_poly_call_materializes_an_abstract_quotation_argument() {
    check_src(
        ": run_abstract ['T: Copy] ( 'T [ 'T -- 'T ] -- 'T ) drop ;\n\
             : main ( -- ) 7 [ ] run_abstract drop ;\n",
    )
    .expect("a literal argument at an abstract quotation position should materialize");
}
/// P7.S3l phase 2 review: the two-pass split (mirroring
/// `check_poly_combinator_args`) makes grounding order-independent -- the
/// declared quotation slot comes *before* the plain input that binds
/// `'T`, the reverse of the sibling test above.
#[test]
fn check_poly_call_materializes_an_abstract_quotation_argument_declared_first() {
    check_src(
        ": run_abstract_first ['T: Copy] ( [ 'T -- 'T ] 'T -- 'T ) swap drop ;\n\
             : main ( -- ) [ ] 7 run_abstract_first drop ;\n",
    )
    .expect("grounding a quotation slot declared before its binding input should succeed");
}
/// R2: a capturing literal at the argument boundary runs the existing R15
/// admission path. An in-frame (non-escaping) capture is admitted; this
/// alone survives stubbing out the `check_capture_admission` call at this
/// call site, so it does not by itself prove the path is wired up -- see
/// the escaping-capture rejection below for that proof.
#[test]
fn check_poly_call_admits_a_capturing_literal_argument() {
    check_src(
        ": run_it ['T: Copy] ( 'T [ i64 -- i64 ] -- 'T ) drop ;\n\
             : main ( -- ) 3 | n | 7 [ n add ] run_it drop ;\n",
    )
    .expect("an in-frame capturing literal should be admitted at the argument boundary");
}
/// R2, discriminating: an escaping capture at the argument boundary must
/// hit `check_capture_admission`'s existing rejection -- proof this new
/// call site actually invokes it, not just present in the diff.
#[test]
fn check_poly_call_rejects_an_escaping_capturing_literal_argument() {
    let err = check_src(
        ": run_it ['T: Copy] ( 'T [ i64 -- i64 ] -- 'T ) drop ;\n\
             : main ( -- ) [ 1 add ] | q | 7 [ q call ] run_it drop ;\n",
    )
    .expect_err("an escaping capturing literal must be rejected at the argument boundary");
    assert!(
        err.contains("capturing a quotation value by name is deferred"),
        "{err}"
    );
}
/// P7b.S6 (R3/R7): `^`/`^>` newly routed in a poly body -- `^` wraps a
/// generic self-reference, `^>` unwraps it back, the pair this phase's
/// `List['T]` recursion needs (`Cons> | v rest | ... rest ^> ... fold`).
#[test]
fn poly_body_owned_cell_wrap_and_unwrap_round_trips() {
    check_src(
        "import: intrinsics * ;\n\
             : wrap_unwrap['T: Copy] ( 'T -- 'T ) ^ ^> ;\n\
             : main ( -- ) 7 wrap_unwrap drop ;\n",
    )
    .expect("a poly-body `^`/`^>` round trip should type-check");
}

/// The rejecting half: `^>` on an operand that is not itself an
/// `OwnedCell` (here a bare type variable) is a located type mismatch,
/// not a panic and not a silent unknown-word fallthrough.
#[test]
fn poly_body_owned_cell_unwrap_of_non_cell_operand_is_error() {
    let err = check_src(
        "import: intrinsics * ;\n\
             : bad_unwrap['T] ( 'T -- 'T ) ^> ;\n\
             : main ( -- ) 7 bad_unwrap drop ;\n",
    )
    .unwrap_err();
    assert!(err.contains("type mismatch"), "{err}");
    assert!(err.contains("^>"), "{err}");
}

/// Review fix (P7b.S6 Phase 3, P0): a poly-body `^` over a reference-
/// typed payload is a located error, the poly-side twin of
/// `check_owned_cell_word`'s `contains_reference` rejection -- without
/// this guard the same shape reached `word_families.rs`'s lowering-time
/// panic ("^'s payload shape is interned by the checker") instead.
#[test]
fn poly_body_owned_cell_construction_rejects_reference_payload() {
    let err = check_src(
        "type: Pt x i64 y i64 ;\n\
             : bad['T] ( &'T -- ) ^ ^> drop ;\n\
             : main ( -- ) 1 2 Pt | p | &p bad ;\n",
    )
    .unwrap_err();
    assert!(err.contains("a reference cannot be stored"), "{err}");
    assert!(err.contains("the payload `^` would store"), "{err}");
}

/// Post-implementation review fix: a poly-body `^`-built cell whose
/// payload never reaches this word's own declared input/output
/// signature (a body-internal temporary, immediately unwrapped and
/// dropped, never returned) must still have its concrete monomorphized
/// shape interned into the live `owned_cells` registry once a caller
/// grounds `'T` -- otherwise `word_families.rs`'s `cell_id_of` finds no
/// structural match at lowering and panics. Pre-fix this word's cell
/// shape (`^i64`) was absent from `checked.owned_cells`; post-fix it is
/// present.
#[test]
fn poly_body_internal_owned_cell_temporary_interns_into_live_registry() {
    let src = ": leak['T] ( 'T -- ) ^ drop ;\n\
             : main ( -- ) 5 leak ;\n";
    let (checked, _) =
        checked_like_a_build(src).expect("leak's body-internal cell should check and intern");
    assert!(
        checked.owned_cells.iter().any(|c| c.payload == Type::I64),
        "the concrete ^i64 cell shape should be interned into the live registry: {:?}",
        checked.owned_cells
    );
}

/// P7 slice 3f (R3): `call` on a genuine ground `Type::Quotation`
/// parameter -- a real value with no interned body to splice -- honours the
/// declared effect, popping its inputs and pushing its outputs.
#[test]
fn poly_call_term_calls_a_ground_quotation_param() {
    check_src(
        ": call_it ['T: Copy] ( 'T [ i64 -- i64 ] -- 'T i64 ) 1 swap call ;\n\
             : main ( -- ) 7 [ 1 add ] call_it drop drop ;\n",
    )
    .expect("`call` on a ground quotation parameter should honour its declared effect");
}
/// R3's *ordering*, the output side: the declared outputs are pushed in
/// declaration order, so the first one lands deepest. Every other R3 test
/// declares a single output, which cannot tell the push order from its
/// reverse. Checker-only rather than a golden because a quotation effect
/// with two outputs cannot yet be lowered (see the phase 3 note on
/// `intern_output_bundles`); the input side gets the golden instead.
#[test]
fn poly_call_on_a_ground_quotation_param_pushes_outputs_in_order() {
    check_src(": call_it ['T: Copy] ( 'T [ -- i64 Bool ] -- 'T i64 Bool ) call ;\n")
        .expect("the first declared output must land deepest");
}
/// R3's negative, the `PolyType::Concrete` renderer arm: a ground operand
/// at a popped position that simply is not the declared input type is a
/// located rejection, not a panic and not a silent coercion.
#[test]
fn poly_call_on_a_ground_quotation_param_ground_mismatch_is_error() {
    let err = check_src(
        ": call_it ['T: Copy] ( 'T [ i64 -- i64 ] -- 'T i64 ) True swap call ;\n\
             : main ( -- ) 7 [ 1 add ] call_it drop drop ;\n",
    )
    .expect_err("a wrong operand type at a declared input must be rejected");
    assert_eq!(
        err,
        "error: type mismatch in `call_it` (line 1)\n  \
             `call` expected `i64`, found `Bool`\n  note: declared ( -- )"
    );
}
/// R3's negative, the `poly_rendered_type_mismatch_error` arm: an operand
/// with no ground `Type` to render (here a bare `PolyType::Var`) at a
/// popped position. `type_mismatch_error` cannot render this side at all,
/// which is why the two arms exist.
#[test]
fn poly_call_on_a_ground_quotation_param_variable_operand_is_error() {
    let err = check_src(": call_it ['T: Copy] ( 'T [ i64 -- i64 ] -- i64 ) call ;\n")
        .expect_err("a type variable at a declared input must be rejected");
    assert_eq!(
        err,
        "error: type mismatch in `call_it` (line 1)\n  \
             `call` expected `i64`, found `'T`\n  note: declared ( -- )"
    );
}
/// R3's underflow arm, distinct from the bare-`call`-with-an-empty-stack
/// rejection above it: the quotation is there, the operands its declared
/// effect demands are not.
#[test]
fn poly_call_on_a_ground_quotation_param_underflow_is_error() {
    let err = check_src(": call_it ['T: Copy] ( 'T [ i64 -- i64 ] -- i64 ) swap drop call ;\n")
        .expect_err("a declared input with nothing beneath the quotation must be rejected");
    assert!(
        err.contains("`call` needs 1 values, but the stack holds 0"),
        "{err}"
    );
}
/// P7.S3l (R1/R2): flipped from S3f's pinned rejection -- an abstract
/// declared quotation parameter (`[ i64 -- 'T ]`, its output still
/// carrying the word's own type variable) is now `call`-able from the
/// word's own body. `swap drop` first discards the word's other `'T`
/// input so the quotation's own `'T` output is the sole thing left on
/// exit, matching the declared single-`'T` output -- the near miss
/// `check_poly_call_materializes_an_abstract_quotation_argument` (a
/// different guard, at the argument boundary, also flipped by this
/// slice's R5) does not cover: only the *output* side carries the
/// variable, so a dispatch
/// predicate that checked the declared inputs were ground (they are, a
/// single `i64`) would wrongly claim this one.
#[test]
fn poly_call_on_an_abstract_quotation_param_is_accepted() {
    check_src(": call_it ['T: Copy] ( 'T [ i64 -- 'T ] -- 'T ) swap drop 1 swap call ;\n")
        .expect("an abstract quotation parameter is now call-able");
}
/// R3's underflow arm, the abstract twin of
/// `poly_call_on_a_ground_quotation_param_underflow_is_error`: the
/// quotation is there, the operand its declared input demands is not.
#[test]
fn poly_call_on_an_abstract_quotation_param_underflow_is_error() {
    let err = check_src(": call_it ['T: Copy] ( 'T [ i64 -- 'T ] -- 'T ) swap drop call ;\n")
        .expect_err("a declared input with nothing beneath the quotation must be rejected");
    assert!(
        err.contains("`call` needs 1 values, but the stack holds 0"),
        "{err}"
    );
}
/// R2's ordering, the input side: two heterogeneous declared inputs, so
/// consuming them in the wrong order (rather than merely miscounting
/// them) is discriminated. Every other R2/R3 test in this file declares
/// a single input, which cannot tell deepest-first from its reverse.
#[test]
fn poly_call_on_an_abstract_quotation_param_pops_declared_inputs_deepest_first() {
    check_src(
        ": call_it ['T: Copy] ( 'T [ i64 Bool -- 'T ] -- 'T )\n\
               swap drop 1 True rot call\n\
             ;\n",
    )
    .expect("the deepest operand must satisfy the first declared input");
}
/// R2's ordering, the output side: two heterogeneous declared outputs,
/// so pushing them in the wrong order is discriminated. The existing
/// `poly_call_on_a_ground_quotation_param_pushes_outputs_in_order` pins
/// this for the ground twin only; the abstract arm has its own push loop
/// (`poly_call_abstract_quotation_param`) and needs its own witness.
#[test]
fn poly_call_on_an_abstract_quotation_param_pushes_outputs_in_order() {
    check_src(
        ": call_it ['T: Copy] ( 'T [ -- 'T Bool ] -- 'T Bool )\n\
               swap drop call\n\
             ;\n",
    )
    .expect("the first declared output must land deepest");
}
/// R3's mismatch arm: an operand whose `PolyType` is not structurally
/// equal to the declared input.
#[test]
fn poly_call_on_an_abstract_quotation_param_mismatch_is_error() {
    let err =
        check_src(": call_it ['T: Copy] ( 'T [ i64 -- 'T ] -- 'T ) swap drop True swap call ;\n")
            .expect_err("an operand not structurally equal to the declared input must be rejected");
    assert_eq!(
        err,
        "error: type mismatch in `call_it` (line 1)\n  \
             `call` expected `i64`, found `Bool`\n  note: declared ( -- )"
    );
}
/// L1's other side: the new arm is gated on the operand being a ground
/// quotation, not merely on it not being a `QuotLit` marker -- `call` on a
/// body local bound to a bare `'T` keeps its own rejection.
#[test]
fn poly_call_on_a_variable_local_is_still_error() {
    let err = check_src(": call_it ['T: Copy] ( 'T -- ) | a | a call ;\n")
        .expect_err("`call` on a type variable stays rejected");
    assert_eq!(
        err,
        "error: `call` is not permitted on the type variable `'T` in `call_it` (line 1)"
    );
}
#[test]
fn poly_term_admits_a_quotation_literal_as_a_marker_slot() {
    // P7 slice 3b (R2): the literal pushes a slot carrying its identity in
    // `quot` and a `pt` that is not a value type. Checked at the
    // `poly_term` level because no source program can observe the marker
    // directly: every route out of the body rejects it.
    let sig = bare_sig();
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let env: HashMap<String, Vec<Overload>> = HashMap::new();
    let mut scope = PolyScope::default();
    let mut overloads = HashMap::new();
    let quot_term = Term {
        kind: TermKind::Quotation(Vec::new(), true, None),
        span: Span::default(),
    };
    let stack = poly_term(
        &quot_term,
        Vec::new(),
        &mut scope,
        &sig,
        &ctx,
        &env,
        &CombinatorEnv::default(),
        &[],
        &[],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut overloads,
        &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
        scratch_cross!(),
        false,
    )
    .expect("a quotation literal is admitted in a polymorphic body");
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0].pt, PolyType::QuotLit);
    assert_eq!(
        stack[0].quot,
        Some(PolyQuotRef(0)),
        "the literal's identity rides the slot, not its `PolyType`"
    );
}
#[test]
fn poly_quotation_identity_moves_with_the_slot_under_swap() {
    // S3b L3: the literal's identity rides the slot, so a shuffle reorders
    // the indices with no special handling. Pinned here rather than through a
    // source program: a *tagged* literal must reach its eliminator by
    // written adjacency (the concrete path's rule), so no program can put
    // a shuffle between two arms.
    let sig = bare_sig();
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let env: HashMap<String, Vec<Overload>> = HashMap::new();
    let mut scope = PolyScope::default();
    let mut overloads = HashMap::new();
    let quot = Term {
        kind: TermKind::Quotation(Vec::new(), true, None),
        span: Span::default(),
    };
    let swap = Term {
        kind: TermKind::Call("swap".to_string(), Vec::new(), Vec::new()),
        span: Span::default(),
    };
    let mut stack = Vec::new();
    for term in [&quot, &quot, &swap] {
        stack = poly_term(
            term,
            stack,
            &mut scope,
            &sig,
            &ctx,
            &env,
            &CombinatorEnv::default(),
            &[],
            &[],
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut overloads,
            &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
            scratch_cross!(),
            false,
        )
        .expect("two literals then a swap");
    }
    assert_eq!(
        stack.iter().map(|s| s.quot).collect::<Vec<_>>(),
        vec![Some(PolyQuotRef(1)), Some(PolyQuotRef(0))],
        "`swap` reorders the identities with the slots"
    );
}
#[test]
fn poly_quotation_slot_is_not_copy() {
    // R2: the marker is not a value, so it is never `Copy` -- `dup` on one
    // must not silently mint a second slot pointing at one interned body.
    assert!(!poly_is_copy(
        &PolyType::QuotLit,
        &bare_sig(),
        &[],
        &[],
        &[]
    ));
    assert!(!is_reference_slot(&PolyType::QuotLit));
}
/// The enum the eliminator unit tests below write their arms against.
/// Declared `Circle` first so a test can write its arms `Rect`-first and
/// still be correct: arms are matched by annotation tag, never by slot
/// position.
const SHAPE: &str = "type: Shape | Circle r i64 | Rect w i64 h i64 ;\n";
#[test]
fn poly_eliminator_registry_intercept_precedes_env_dispatch() {
    // R2: the eliminator is intercepted by name ahead of the ordinary
    // `env` dispatch. The arms here are written in the reverse of the
    // enum's declaration order, so an implementation that paired arms to
    // variants positionally (which is what the `PolySig` the eliminator is
    // registered under would do) checks `( Rect )`'s `Rect>` against a
    // narrowed `Shape.Circle` and fails.
    //
    // Anti-placebo note: deleting the intercept does *not* reach env
    // dispatch on this path at all -- `poly_call_term` has no `PolyCtx`,
    // so the eliminator's `PolySig` (registered in `poly_env`) is
    // unreachable and the call falls through to `unknown word`. So the
    // mutation flips accept -> reject, just not via the positional
    // mismatch; the reversed arm order is what makes the *accept* here
    // evidence of tag matching rather than of position matching.
    assert!(
        check_src(&format!(
            "{SHAPE}\
                 : pick ( 'T Shape -- 'T )\n\
                   ~[ ( Rect )   Rect> mul drop ]\n\
                   ~[ ( Circle ) Circle> dup mul 3 mul drop ]\n\
                   Shape? ;\n\
                 : main ( -- ) 1 5 Circle pick drop ;\n"
        ))
        .is_ok(),
        "arms are matched by annotation tag, in any written order"
    );
}
#[test]
fn poly_arm_join_rejects_rigid_type_variable_disagreement() {
    // S3b L1: `'T` stays rigid across arms. One arm leaving `'T` and
    // another `i64` is a located rejection naming both sides in order, never a
    // mid-body bind of `'T := i64`.
    let err = check_src(&format!(
        "{SHAPE}\
             : bad ['T: Copy] ( 'T Shape -- 'T )\n\
               ~[ ( Rect )   Rect> drop drop dup ]\n\
               ~[ ( Circle ) Circle> ]\n\
               Shape? drop ;\n\
             : main ( -- ) ;\n"
    ))
    .expect_err("two arms leaving different types disagree");
    assert!(
        err.contains("an earlier one leaves `'T`, this one leaves `i64`"),
        "the pairing is asserted, not just the failure: {err}"
    );
}
#[test]
fn poly_arm_join_unions_borrows() {
    // S3b L4 (restated as S3b-follow L3): the arms' borrow tables are
    // unioned, not picked between. A missing record reads as "no
    // conflict", so dropping either arm's is a silent False accept -- both
    // directions are asserted, since "pick arm A" keeps `x` and drops `y`
    // and an `x`-only assertion would not flip.
    let program = |later: &str| {
        format!(
            "type: P a i64 ;\n\
                 {SHAPE}\
                 : bad ['T: Copy] ( 'T P P Shape -- 'T )\n\
                   | x y s | s\n\
                   ~[ ( Rect )   Rect> drop drop &!x ]\n\
                   ~[ ( Circle ) Circle> drop &!y ]\n\
                   Shape?\n\
                   {later} drop drop ;\n\
                 : main ( -- ) ;\n"
        )
    };
    for place in ["x", "y"] {
        let err = check_src(&program(place))
            .expect_err("both arms' borrows survive the merge, so either use conflicts");
        assert!(
            err.contains(&format!("cannot name `{place}`"))
                && err.contains("a mutable borrow of it is still live"),
            "the `{place}` record must survive the union: {err}"
        );
    }
}
/// P7 slice 3b-follow (R1): the pieces `poly_walk_arms` needs from a
/// caller that is not the eliminator -- a one-variable signature (so a
/// bare `'T` slot is non-`Copy` and carries a move obligation) and an arm
/// that binds one local and reads it back.
fn one_var_sig() -> PolySig {
    PolySig {
        ty_var_names: vec!["T".to_string()],
        ty_var_spans: Vec::new(),
        ty_kinds: Vec::new(),
        ..bare_sig()
    }
}
fn arm_binding(local: &str, consume: bool) -> Vec<Term> {
    let mut body = vec![Term {
        kind: TermKind::Bind(vec![local.to_string()]),
        span: Span::default(),
    }];
    if consume {
        body.push(Term {
            kind: TermKind::Call(local.to_string(), Vec::new(), Vec::new()),
            span: Span::default(),
        });
    }
    body
}
fn interned_arm(scope: &mut PolyScope, body: Vec<Term>) -> PolyArm {
    let quot = scope.intern_quotation(PolyQuotLit {
        body,
        span: Span::default(),
        is_inline: true,
        annot: None,
    });
    PolyArm {
        quot,
        input: vec![PolySlot::new(PolyType::Var(0))],
        declared_inputs: vec![PolyType::Concrete(Type::I64)],
        tail: false,
    }
}
#[test]
fn poly_walk_arms_truncates_arm_locals_before_joining_moves() {
    // R1: the `Scope::leave` analogue is what makes the N-arm
    // `Moves::join` sound -- it indexes each later arm's map by the first
    // arm's keys, so two arms binding *different* names panic outright
    // unless every arm is truncated back to the enclosing key set first.
    // Driven through the shared helper rather than an eliminator so the
    // machinery is pinned independently of its one caller today.
    let sig = one_var_sig();
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let env: HashMap<String, Vec<Overload>> = HashMap::new();
    let mut scope = PolyScope::default();
    let arms = vec![
        interned_arm(&mut scope, arm_binding("a", true)),
        interned_arm(&mut scope, arm_binding("b", true)),
    ];
    let mut exits: Vec<Vec<PolyType>> = Vec::new();
    poly_walk_arms(
        arms,
        "consumer",
        Span::default(),
        &mut scope,
        &sig,
        &ctx,
        &env,
        &CombinatorEnv::default(),
        &[],
        &[],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut HashMap::new(),
        &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
        scratch_cross!(),
        &mut |_, exit| {
            exits.push(exit.into_iter().map(|slot| slot.pt).collect());
            Ok(())
        },
    )
    .expect("two arms binding different locals join once both are truncated");
    assert_eq!(
        exits,
        vec![vec![PolyType::Var(0)], vec![PolyType::Var(0)]],
        "every arm's exit reaches the cross-arm rule, in written order"
    );
    assert!(
        scope.moves.states.is_empty() && scope.locals.is_empty(),
        "an arm-local never reaches the enclosing scope: {:?}",
        scope.moves.states
    );
}
#[test]
fn poly_walk_arms_rejects_an_arm_local_left_unconsumed() {
    // R1: the leak is rejected *before* the truncation erases it -- the
    // poly walk has no block scope, so nothing else would ever notice a
    // linear local bound inside an arm and dropped on the floor there.
    let sig = one_var_sig();
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let env: HashMap<String, Vec<Overload>> = HashMap::new();
    let mut scope = PolyScope::default();
    let arms = vec![interned_arm(&mut scope, arm_binding("a", false))];
    let err = poly_walk_arms(
        arms,
        "consumer",
        Span::default(),
        &mut scope,
        &sig,
        &ctx,
        &env,
        &CombinatorEnv::default(),
        &[],
        &[],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut HashMap::new(),
        &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
        scratch_cross!(),
        &mut |_, _| Ok(()),
    )
    .expect_err("a linear local bound in an arm and never read leaks");
    assert!(
        err.contains("the local `a` of type `T`, bound in an arm of `consumer`")
            && err.contains("is never consumed"),
        "unexpected message: {err}"
    );
}
#[test]
fn poly_row_combinator_admits_only_a_row_typed_inline_declaration() {
    // S3b-follow (R2): the dispatch's entry condition, decided from the
    // callee's declaration alone. `rowed` qualifies; `rowless` is the
    // concrete-consumer shape (P7.S3d) and must keep the rejection it has
    // today rather than be admitted through row machinery; `plain` is an
    // ordinary word. `if` is checked too, declared verbatim rather than
    // relying on injection: P8.S2 deleted the prelude, so `if` is an
    // ordinary `core::bool` word now (`lib/bool.sth`) and this test's
    // hand-parsed source has to declare it itself to have it registered
    // in `combinators` at all.
    let src = "type: Bool | False | True ;\n\
                   : rowed inline ( ..s ~[ ..s -- ..s ] -- ..s ) | f | f call ;\n\
                   : rowless inline ( array['T 4] ~[ 'T -- 'T ] -- array['T 4] ) | f | f call ;\n\
                   : plain ( i64 -- i64 ) 1 add ;\n\
                   : if inline ( ..a Bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )\n\
                     | e | | t | | c | c tag t e branch ;\n";
    let tokens = lex(src).unwrap();
    let module = crate::parser::parse(&tokens).unwrap();
    let combinators = collect_combinators(&module.words);
    assert!(
        poly_row_combinator(&combinators, "rowed").is_some(),
        "a row on a quotation parameter is what the dispatch grounds"
    );
    assert!(
        poly_row_combinator(&combinators, "rowless").is_none(),
        "a rowless quotation parameter is P7.S3d's shape, not this dispatch's"
    );
    assert!(poly_row_combinator(&combinators, "plain").is_none());
    assert!(
        poly_row_combinator(&combinators, "if").is_some(),
        "a same-module `if` (or the real `core::bool` one, mangled per module) still \
             registers under a single-module program's bare spelling"
    );
}
#[test]
fn poly_combinator_dispatch_precedes_the_quotlit_operand_window() {
    // S3b-follow (R2): the dispatch must sit ahead of *both* rejections
    // that used to catch this family. `unless` never reached the name
    // guard at all -- it is not one of the names that guard lists -- and
    // landed on the `QuotLit` operand window instead. Moving the dispatch
    // below that window makes this body fail again.
    let body = "over over gt ~[ drop ] ~[ swap drop ] unless";
    check_src(&format!(
        ": mymin ['T: Copy Ord] ( 'T 'T -- 'T ) {body} ;\n: main ( -- ) 2 9 mymin drop ;\n"
    ))
    .expect("`unless` reaches the dispatch");
    // The accept alone would also be satisfied by an implementation that
    // stopped checking the arms, so the arm rule is asserted to still
    // report through *this* dispatch rather than the operand window.
    let err = check_src(
        ": bad ['T: Copy Ord] ( 'T 'T -- 'T ) over over gt ~[ drop ] ~[ swap ] unless ;\n",
    )
    .expect_err("the arms leave different shapes");
    assert!(
        err.contains("the quotations passed to `unless` leave different stack shapes"),
        "`unless`'s arms must be checked by the dispatch: {err}"
    );
}
#[test]
fn poly_combinator_routes_by_the_declared_row_pair() {
    // R3: one dispatch, two routes. A parameter whose declared rows are
    // the same on both sides is held to its *declaration* (the seeded
    // entry row), and one whose rows differ is held only to its *sibling*
    // arms -- so the same arm body is legal under one and rejected under
    // the other. `same` and `differ` are otherwise identical, which is
    // what makes this a routing test rather than two unrelated checks.
    const BOTH: &str =
            ": same   inline ( ..a Bool ~[ ..a -- ..a ] ~[ ..a -- ..a ] -- ..a )\n\
               | same--e | | same--t | | same--c | same--c tag same--t same--e branch ;\n\
             : differ inline ( ..a Bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )\n\
               | differ--e | | differ--t | | differ--c | differ--c tag differ--t differ--e branch ;\n";
    // Both arms consume a slot of the row they entered with: a shape
    // change the siblings agree on, and a violation of a row declared the
    // same on both sides.
    let body = "over over gt ~[ drop ] ~[ swap drop ]";
    check_src(&format!(
        "{BOTH}: g ['T: Copy Ord] ( 'T 'T -- 'T ) {body} differ ;\n"
    ))
    .expect("the shape-changing route holds the arms to each other");
    let err = check_src(&format!(
        "{BOTH}: g ['T: Copy Ord] ( 'T 'T -- 'T 'T ) {body} same ;\n"
    ))
    .expect_err("a row declared the same on both sides fixes the exit");
    assert!(
        err.contains(
            "was declared `~[ ..a -- ..a ]`, but it leaves `'T` where that requires `'T 'T`"
        ),
        "the non-shape-changing route holds the arm to its declaration: {err}"
    );
}
#[test]
fn poly_combinator_shape_changing_exit_row_is_what_the_arms_agreed() {
    // R3: the exit row of a shape-changing call is the arms' agreed exit,
    // not the row the call was entered with. Pinned by the *caller's*
    // declared outputs: this body's arms each consume one slot of the two
    // they enter with, so an exit taken from the entry row would leave `'T
    // 'T` and disagree with the signature.
    let body = ": g ['T: Copy Ord] ( 'T 'T -- 'T ) over over gt ~[ drop ] ~[ swap drop ] if ;\n";
    check_src(body).expect("the exit row is the arms' own");
    let err = check_src(&body.replace("-- 'T )", "-- 'T 'T )"))
        .expect_err("the entry row is not handed back");
    assert!(
        err.contains("body leaves `'T`, but the declared outputs are `'T 'T`"),
        "unexpected message: {err}"
    );
}
#[test]
fn poly_combinator_declaring_a_row_no_arm_produces_is_located() {
    // R3: a signature promising an output row that none of its quotation
    // parameters produces has no account of that row at all -- the arms
    // agreed on nothing to hand back. Located, and named against the row
    // itself, rather than answered with the entry row the declaration
    // explicitly differs from.
    let err = check_src(
            ": weird inline ( ..a Bool ~[ ..a -- ..a ] -- ..b ) | weird--f | | weird--c | weird--c tag weird--f weird--f branch ;\n\
             : g ['T: Copy Ord] ( 'T 'T -- 'T ) over over gt ~[ ] weird ;\n",
        )
        .expect_err("the declared output row is ungroundable");
    assert!(
            err.contains(
                "`weird` declares `..b`, which a call in the polymorphic body of `g` (line 2) cannot ground"
            ),
            "unexpected message: {err}"
        );
}
#[test]
fn poly_combinator_grounds_the_row_to_the_caller_region() {
    // R3/L2: the declared row grounds to `stack[..base]` -- the caller
    // region *below* the combinator's fixed inputs -- once, at the
    // dispatch site. Pinned from both sides, since grounding it to the
    // whole stack or to nothing each breaks only one of them: the arm can
    // shuffle exactly the two slots the region holds, and reaching one
    // slot deeper underflows inside the arm.
    const TWICE: &str = ": twice inline ( ..s ~[ ..s -- ..s ] -- ..s ) | f | f call f call ;\n";
    check_src(&format!(
        "{TWICE}: g ['T: Copy Ord] ( 'T 'T -- 'T 'T ) ~[ swap ] twice ;\n"
    ))
    .expect("the arm walks over the grounded row");
    let err = check_src(&format!(
        "{TWICE}: g ['T: Copy Ord] ( 'T 'T -- 'T 'T ) ~[ drop drop drop ] twice ;\n"
    ))
    .expect_err("the region is the caller row, not the whole stack");
    assert!(
        err.contains("`drop` needs 1 values, but the stack holds 0"),
        "unexpected message: {err}"
    );
}
#[test]
fn poly_eliminator_arm_leaving_its_own_variant_is_error() {
    // R2 step 5b: with two arms, R3's rigid-arm-disagreement check
    // (different exit shapes) fires before this guard ever gets a chance
    // to look at the escaping `Type::Variant`, so a two-arm repro cannot
    // exercise it. A single-variant enum is exhaustive with one arm and
    // reaches this guard directly. Stubbing it out here builds and
    // double-drops the linear `Spy` payload underneath, since `is_copy`
    // falls through `Type::Variant` to `True`.
    let err = check_src(&format!(
        "{SPY}\
             type: One | A p Spy ;\n\
             : bad ['T: Copy] ( 'T One -- 'T ) ~[ ( A ) ] One? ;\n\
             : main ( -- ) 1 9 Spy A bad drop ;\n"
    ))
    .expect_err("an arm leaving its own narrowed variant unconsumed is an escape");
    assert!(
        err.contains("an arm of `One?` leaves `One.A` on the stack"),
        "unexpected message: {err}"
    );
}
#[test]
fn polyslot_int_val_folds_lits() {
    // R1: `int_val` carries what the deleted `lits` shadow did -- set on
    // `IntLit`, `None` elsewhere, truncated on `Bind`. Round-tripped
    // directly at the `poly_term` level since a bound local's own
    // literal-ness is discarded (D6), not observable through `check_src`.
    let sig = bare_sig();
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let env: HashMap<String, Vec<Overload>> = HashMap::new();
    let mut scope = PolyScope::default();
    let mut overloads = HashMap::new();
    let lit_term = Term {
        kind: TermKind::IntLit(9),
        span: Span::default(),
    };
    let stack = poly_term(
        &lit_term,
        Vec::new(),
        &mut scope,
        &sig,
        &ctx,
        &env,
        &CombinatorEnv::default(),
        &[],
        &[],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut overloads,
        &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
        scratch_cross!(),
        false,
    )
    .expect("an int literal should push a slot");
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0].pt, PolyType::Concrete(Type::I64));
    assert_eq!(stack[0].int_val, Some(9));

    let bind_term = Term {
        kind: TermKind::Bind(vec!["x".to_string()]),
        span: Span::default(),
    };
    let stack = poly_term(
        &bind_term,
        stack,
        &mut scope,
        &sig,
        &ctx,
        &env,
        &CombinatorEnv::default(),
        &[],
        &[],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut overloads,
        &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
        scratch_cross!(),
        false,
    )
    .expect("binding the literal should consume the slot");
    assert!(
        stack.is_empty(),
        "the bound literal's slot leaves the stack"
    );
    assert_eq!(scope.locals["x"], PolyType::Concrete(Type::I64));
}
#[test]
fn check_poly_copy_word_accepts_and_instantiates() {
    // R1/R4–R7: a `'T: Copy` word `dup`s its variable and is called at a
    // concrete `Copy` type; the body and the instantiation both check.
    check_src(": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;\n: main ( -- ) 5 dupit drop drop ;")
        .unwrap();
}
#[test]
fn check_poly_word_records_one_instantiation_per_concrete_shape() {
    // R8/R14: each distinct ground θ is recorded once, keyed by call span.
    let module = checked_module(
        ": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;\n\
             : main ( -- ) 5 dupit drop drop True dupit drop drop ;",
    );
    // Two call sites, two distinct θ (i64 and Bool): two instantiations.
    let symbols: std::collections::HashSet<&str> = module
        .instantiations
        .values()
        .map(|c| c.symbol.as_str())
        .collect();
    assert_eq!(module.instantiations.len(), 2);
    assert_eq!(symbols.len(), 2);
}
/// P7.S3t (R4/R6/R9): the seed is what a call site's explicit type
/// argument *does*. The fixture is the one shape inference cannot reach:
/// `'T` appears only in an output, so with no list `apply_subst` reports
/// it and no instantiation is recorded at all. (A word with no operand can
/// declare a `'T` output only if its body produces one, which before a
/// nullary trait member -- phase 3 -- means its own self-call.)
#[test]
fn an_explicit_instantiation_seeds_the_recorded_substitution() {
    let module = checked_module(": f ( -- 'T ) f ;\n: main ( -- ) f[i64] drop ;");
    let inst = module
        .instantiations
        .values()
        .find(|i| i.callee == "f")
        .expect("the seeded call site recorded an instantiation");
    assert_eq!(inst.subst.ty_of(0), Some(Type::I64));
    assert!(
        inst.symbol.contains("i64"),
        "the seeded theta mints the specialization: {}",
        inst.symbol
    );
    // The same call without the list: R9's revived diagnostic at the site
    // R9 describes, `apply_subst` walking the *declared outputs*. The
    // message also fires from pass 2's quotation grounding, where it
    // misdescribes an input as an output, so both witnesses use the
    // output shape and neither leans on that path.
    let bare =
        check_src(": f ( -- 'T ) f ;\n: main ( -- ) f drop ;").expect_err("nothing binds `'T`");
    assert_eq!(
        bare,
        "error: `f` in `main` (line 2) has output variable `'T` that no input binds\n  \
             note: supply it explicitly: `f[SomeType]`"
    );
}
/// P7.S3t (R9): the remedy names one `SomeType` per declared type
/// variable, not a single one regardless of arity -- a two-variable
/// callee's note must read `f[SomeType SomeType]`, since `f[SomeType]`
/// would just fail again on arity.
#[test]
fn an_unbound_output_note_matches_a_two_variable_signatures_arity() {
    let bare = check_src(": f ( 'T -- 'U ) f ;\n: main ( -- ) 1 f drop ;")
        .expect_err("nothing binds `'U`");
    assert_eq!(
        bare,
        "error: `f` in `main` (line 2) has output variable `'U` that no input binds\n  \
             note: supply it explicitly: `f[SomeType SomeType]`"
    );
}
/// P7.S3t (R5/R6): a seeded and an inferred call at the same θ are the
/// same specialization. `instantiation_symbol` renders `subst.ty` in
/// vector order, so the two paths agree only if both leave it sorted by
/// variable id -- and the *inferred* path does not: `check_poly_call`
/// defers a quotation input to pass 2, so `r` binds `'U` (id 1) before
/// `'T` (id 0). `id ( 'T -- 'T )` cannot witness this, having one
/// variable and so only one possible order; `r` is the smallest signature
/// that can. Without the sort, this call pair mints
/// `..._t1_f64_t0_i64` and `..._t0_i64_t1_f64` and monomorphizes `r`
/// twice.
#[test]
fn a_seeded_and_an_inferred_call_at_one_type_mint_one_symbol() {
    let module = checked_module(
        ": r ( [ 'T -- ] 'U 'T -- 'U ) drop swap drop ;\n\
             : main ( -- ) [ drop ] 2.5 7 r drop [ drop ] 2.5 7 r[i64 f64] drop ;",
    );
    let insts: Vec<&CallInst> = module
        .instantiations
        .values()
        .filter(|i| i.callee == "r")
        .collect();
    assert_eq!(insts.len(), 2, "two call sites are recorded separately");
    for inst in &insts {
        assert_eq!(
            inst.subst.ty,
            vec![(0, Type::I64), (1, Type::F64)],
            "theta is kept sorted by variable id: {:?}",
            inst.subst.ty
        );
    }
    let symbols: std::collections::HashSet<&str> =
        insts.iter().map(|i| i.symbol.as_str()).collect();
    assert_eq!(symbols.len(), 1, "one theta, one symbol: {symbols:?}");
}
/// P7.S3t (R5): `subst.len` goes out of order for the same reason `ty`
/// does -- pass 2 defers the quotation input, so `r` binds `'M` (id 1)
/// before `'N` (id 0) -- and is sorted for a weaker one. No seed path can
/// make two minting paths disagree on a length (R4), so this pins the
/// normalization `Subst`'s order-sensitive derived `Eq` documents, not a
/// divergence. The two lengths differ so the assertion cannot pass on a
/// vector that is merely the right size.
#[test]
fn a_deferred_quotation_input_leaves_the_length_substitution_sorted() {
    let module = checked_module(
        ": r ( [ array['T 'N] -- ] array['U 'M] array['T 'N] -- ) drop drop drop ;\n\
             : main ( -- ) [ drop ] 0 2 fill 0 3 fill r ;",
    );
    let inst = module
        .instantiations
        .values()
        .find(|i| i.callee == "r")
        .expect("the call site recorded an instantiation");
    assert_eq!(
        inst.subst.len,
        vec![(0, 3), (1, 2)],
        "theta is kept sorted by length-variable id: {:?}",
        inst.subst.len
    );
}
/// P7.S3t (R4): exact arity over the callee's declared *type* variables,
/// both directions, with the declared ones named -- the list is a
/// statement about the callee's signature, so the message renders that
/// signature's variables rather than just counting them.
#[test]
fn a_wrong_arity_instantiation_is_rejected() {
    let too_many = check_src(": id ( 'T -- 'T ) ;\n: main ( -- ) 7 id[i64 f64] drop ;")
        .expect_err("one declared variable, two given");
    assert_eq!(
        too_many,
        "error: `id` (line 2) declares 1 type variable (`'T`) but was given 2 type arguments"
    );
    let too_few =
        check_src(": pairwise ( 'T 'U -- ) drop drop ;\n: main ( -- ) 1 2.5 pairwise[i64] ;")
            .expect_err("two declared variables, one given");
    assert_eq!(
            too_few,
            "error: `pairwise` (line 2) declares 2 type variables (`'T`, `'U`) but was given 1 type argument"
        );
}
/// P7.S6b: a *type*-argument slot is not addressable against a length
/// variable -- `alen[i64]` supplies a type argument, but `alen` declares
/// no type variables (only a length one), so it is still an arity error
/// on the type side, unrelated to explicit length instantiation (which
/// this slice makes reachable via `alen[4]`, not `alen[i64]`).
#[test]
fn a_type_argument_cannot_bind_a_length_variable() {
    let err = check_src(
        ": alen ( array[i64 'N] -- array[i64 'N] usize ) len ;\n\
             : main ( -- ) 5 4 fill alen[i64] drop drop ;",
    )
    .expect_err("a length variable cannot be bound via a type-argument slot");
    assert_eq!(
        err,
        "error: `alen` (line 2) declares no type variables but was given 1 type argument"
    );
}
/// P7.S6b (R3): an explicit length argument binds `'N`, unified against a
/// concrete operand's own count -- the accept path for `sum[i64 4]`.
/// Non-`inline`, and reads `len` back rather than indexing (phase 4's
/// rationale applies identically here: an `inline` fixture would never
/// reach `check_poly_call` at all).
#[test]
fn an_explicit_length_argument_checks_clean_against_a_matching_operand() {
    check_src(
        ": sum['T 'N: Len] ( array['T 'N] -- usize ) len swap drop ;\n\
             : main ( -- ) 0 4 fill sum[i64 4] drop ;",
    )
    .expect("the explicit length agrees with the operand's own count");
}
/// P7.S6b (R3): a wrong explicit length *count* (not a disagreeing
/// operand) is the arity error, distinct from R4's conflict routing.
#[test]
fn a_wrong_explicit_length_argument_count_is_the_arity_error() {
    let err = check_src(
        ": pair['T 'N: Len 'M: Len] ( array['T 'N] array['T 'M] -- usize ) drop len swap drop ;\n\
             : main ( -- ) 0 4 fill 0 4 fill pair[i64 4] drop ;",
    )
    .expect_err("one length argument given, two declared");
    assert_eq!(
            err,
            "error: `pair` (line 2) declares 2 length variables (`'N`, `'M`) but was given 1 length argument"
        );
}
/// P7.S6b (R2a): the poly-body guard (`type_arguments_in_poly_body_error`)
/// widens to length arguments too -- a call inside a polymorphic word's
/// own body has no `Subst` to seed, so an explicit length list there is
/// rejected outright, not silently dropped.
#[test]
fn an_explicit_length_argument_inside_a_poly_body_is_rejected() {
    let err = check_src(
        ": sum['T 'N: Len] ( array['T 'N] -- usize ) len swap drop ;\n\
             : wrapper['T 'N: Len] ( array['T 'N] -- usize ) sum[i64 4] ;\n\
             : main ( -- ) 0 4 fill wrapper[i64 4] drop ;",
    )
    .expect_err("an explicit instantiation inside a poly body is rejected");
    assert_eq!(
            err,
            "error: `sum` in `wrapper` (line 2) cannot be explicitly instantiated inside a polymorphic word's own body\n  note: instantiate the enclosing word at its own call site instead; forwarding a type or length argument through a polymorphic body is not supported"
        );
}
/// P7.S6b (R2a): the non-poly dispatch route's `no_type_arguments_error`
/// guard widens identically -- a concrete (non-polymorphic) callee given
/// an explicit length argument is rejected, not silently dropped.
#[test]
fn an_explicit_length_argument_on_a_non_poly_callee_is_rejected() {
    let err = check_src(": addup ( i64 i64 -- i64 ) drop ;\n: main ( -- ) 1 2 addup[4] drop ;")
        .expect_err("a concrete word takes no explicit length argument");
    assert_eq!(
            err,
            "error: `addup` (line 2) takes no length arguments; only a call to a polymorphic word may be explicitly instantiated"
        );
}
/// P7.S6b: type and length arguments seed independently -- a callee
/// declaring both `'T` and `'N: Len` may be called with only the length
/// sublist (`sum[4]`), leaving `'T` to ordinary inference off the
/// operand. Each list's arity is checked only when that list is
/// non-empty, so a length-only call is not an arity error.
#[test]
fn a_length_only_explicit_argument_leaves_the_type_variable_to_inference() {
    check_src(
        ": sum['T 'N: Len] ( array['T 'N] -- usize ) len swap drop ;\n\
             : main ( -- ) 0 4 fill sum[4] drop ;",
    )
    .expect("the length argument seeds 'N; 'T is inferred as i64 from the operand");
}
/// P7.S6b: the call-site instantiation list is always type-sublist-then-
/// length-sublist (a lexical convention, not a declaration-order mirror).
/// Declaring the length variable *before* the type variable in the
/// header (`'N: Len 'T`, the reverse of every other test's `'T 'N: Len`)
/// must still check identically -- `ty_var_names`/`len_var_names` are
/// collected per-kind, independent of interleaving in the declaration.
#[test]
fn a_length_variable_declared_before_the_type_variable_still_checks() {
    check_src(
        ": sum['N: Len 'T] ( array['T 'N] -- usize ) len swap drop ;\n\
             : main ( -- ) 0 4 fill sum[i64 4] drop ;",
    )
    .expect("declaration order of 'N vs 'T does not affect call-site checking");
}
#[test]
fn check_generic_comparison_body_with_ord_checks_clean() {
    // P7.S3k (R1/R3/R7): re-expresses the retired
    // `check_poly_ord_word_accepts_comparison_body`. A generic body may
    // compare its own `'T` -- but through `lib/cmp.sth`'s real
    // `: gt ['T: Copy Ord] ( 'T 'T -- Bool )`, reached as a generic callee like
    // any other, not through a name-matched carve-out. Checked *mangled*,
    // so the callee arrives as `gt__mN` exactly as it does in a real
    // build; that is the shape the deleted carve-out could never see, and
    // the reason it was dead code.
    check_src_mangled(
        ": less ['T: Copy Ord] ( 'T 'T -- Bool ) gt ;\n: main ( -- ) 3 4 less drop ;",
    )
    .unwrap();
}
#[test]
fn check_generic_comparison_body_without_ord_is_error() {
    // P7.S3k (R3): the same body without the bound is a located call-site
    // error naming the missing `Ord`, not a deferred failure at whatever
    // type `less` is later instantiated at.
    let err =
        check_src_mangled(": less ['T: Copy] ( 'T 'T -- Bool ) gt ;\n: main ( -- ) ;").unwrap_err();
    assert!(
        err.contains("requires `Ord`") && err.contains("`less`"),
        "unexpected message: {err}"
    );
    assert!(!err.contains("__m"), "a mangled spelling leaked: {err}");
}
#[test]
fn check_concrete_overload_is_selected_over_an_ord_bounded_generic() {
    // P7.S3s (R6), the call-site half of criterion 6: `poly_sig_could_match`
    // is what makes a `Vec2 Vec2` call fall through the `Ord`-bounded
    // generic candidate to the concrete one of the same name. `Vec2` has no
    // `impl: Ord`, so the generic candidate must not admit these operands
    // even though unification alone would bind `'T` to anything.
    //
    // Asserted at the checker boundary rather than on a built program on
    // purpose: selecting the concrete candidate is the *correct* answer,
    // and a program that then calls it panics at lowering (`checked user
    // word exists`, `ir/func_builder/calls.rs`) because
    // `ast::overload_symbols` counts poly words when deciding a name is
    // overloaded, so the concrete word carries a `$$0`-suffixed symbol the
    // call site never records. That gap is pre-existing -- reproduced at
    // this slice's parent commit with `Bound::Ord` still in place -- and
    // orthogonal to `Ord`, so there is no run golden to pair with this;
    // `tests/phase7_slice3s_flip.rs` covers the declaration-time half.
    //
    // Mutation-verified: dropping `poly_sig_could_match`'s `impl:`
    // registry filter makes the generic candidate win, and this fails with
    // "`Vec2` does not satisfy `Ord`" from inside its instantiated body.
    check_src_mangled(
        "type: Vec2 x i64 y i64 ;\n\
             : mylt ['T: Copy Ord] ( 'T 'T -- Bool ) lt ;\n\
             : mylt ( Vec2 Vec2 -- Bool )\n\
               | a b | &a &x @ &b &x @ lt | r | a drop b drop r ;\n\
             : main ( -- ) 1 1 Vec2 2 2 Vec2 mylt drop ;\n",
    )
    .unwrap();
}
#[test]
fn check_poly_length_word_accepts_and_monomorphizes_len() {
    // R1/R5/R9: a length variable is opaque through `len`; the same word
    // instantiates at `array[i64 4]` and `array[i64 8]`.
    check_src(
        ": alen ( array[i64 'N] -- array[i64 'N] usize ) len ;\n\
             : main ( -- ) 5 4 fill alen drop drop 5 8 fill alen drop drop ;",
    )
    .unwrap();
}
#[test]
fn check_poly_row_word_accepts_and_expands_outputs() {
    // R1/R5/R7: a row-variable word passes its deeper stack through
    // untouched and duplicates the two `Copy` variables; the resolved
    // instantiation has four concrete outputs, so it interns a bundle.
    let module = checked_module(
        ": dup2 ['a: Copy 'b: Copy] ( ..s 'a 'b -- ..s 'a 'b 'a 'b ) over over ;\n\
             : main ( -- ) 1 2 dup2 drop drop drop drop ;",
    );
    assert_eq!(module.instantiations.len(), 1);
    let inst = module.instantiations.values().next().unwrap();
    assert_eq!(inst.out_arity, 4);
    assert!(inst.bundle.is_some());
}
#[test]
fn check_x4_type_variable_forced_to_two_concretes_names_both() {
    // X4: one `'T` unified to both `i64` and `Bool` at one call site names
    // both concrete types.
    let err = check_src(": pairwise ( 'T 'T -- ) drop drop ;\n: main ( -- ) 1 True pairwise ;")
        .unwrap_err();
    assert!(err.contains("'T"), "unexpected message: {err}");
    assert!(err.contains("i64"), "unexpected message: {err}");
    assert!(err.contains("Bool"), "unexpected message: {err}");
}
#[test]
fn check_x5_copy_bound_on_linear_type_names_variable_type_and_reason() {
    // X5: instantiating a `'T: Copy` word with a linear type is a located
    // call-site error naming the variable, the type, and the linear reason.
    let src = format!("{SPY}: idc ['T: Copy] ( 'T -- 'T ) ;\n: main ( -- ) 0 Spy idc drop ;");
    let err = check_src(&src).unwrap_err();
    assert!(err.contains("'T"), "unexpected message: {err}");
    assert!(err.contains("Spy"), "unexpected message: {err}");
    assert!(err.contains("linear"), "unexpected message: {err}");
}
#[test]
fn check_x6_ord_bound_on_non_ord_type_is_error() {
    // X6: instantiating a `'T: Ord` requirement with a non-`Ord` type is a
    // located error.
    // P7.S3k (R7): `Copy` joins the declaration. `gt` is `lib/cmp.sth`'s
    // `['T: Copy Ord] ( 'T 'T -- Bool )`, and the body's comparison now
    // discharges that whole bound set across the call (R3) instead of
    // being special-cased by name against `Ord` alone. The subject is
    // unchanged: `Bool` is `Copy` but not `Ord`, so it is `less`'s own
    // instantiation that fails, at `main`'s call site.
    let err = check_src(
        ": less ['T: Copy Ord] ( 'T 'T -- Bool ) gt ;\n: main ( -- ) True False less drop ;",
    )
    .unwrap_err();
    assert!(err.contains("'T"), "unexpected message: {err}");
    assert!(err.contains("Ord"), "unexpected message: {err}");
}
#[test]
fn check_x7_dup_of_unbounded_variable_names_missing_copy_bound() {
    // X7: `dup` of an unbounded `'T` inside a body names the variable and
    // the missing `Copy` bound.
    let err = check_src(": bad ( 'T -- 'T 'T ) dup ;\n: main ( -- ) ;").unwrap_err();
    assert!(err.contains("'T"), "unexpected message: {err}");
    assert!(err.contains("Copy"), "unexpected message: {err}");
}
#[test]
fn check_x8_compare_of_unbounded_variable_requires_ord() {
    // X8: `gt` on an unbounded `'T` inside a body requires an `Ord` bound.
    //
    // P7.S3k (R7): declared `'T: Copy` so `Ord` is the *only* bound
    // missing. The rule is now `gt`'s own declared bound set discharged
    // against this word's (R3), so an entirely unbounded `'T` names
    // whichever of the two comes first and would not pin `Ord`.
    let err = check_src(": bad ['T: Copy] ( 'T 'T -- Bool ) gt ;\n: main ( -- ) ;").unwrap_err();
    assert!(err.contains("'T"), "unexpected message: {err}");
    assert!(err.contains("Ord"), "unexpected message: {err}");
}
#[test]
fn check_poly_local_bound_and_never_read_is_unconsumed_error() {
    // A `'T` bound to a local and never read leaks: the polymorphic body
    // checker rejects it exactly as the monomorphic sibling rejects
    // `( ^i64 -- ) | x | ;`, naming the variable.
    let err = check_src(": leaky ( 'T -- ) | x | ;\n: main ( -- ) ;").unwrap_err();
    assert!(
        err.contains("linear value `x` is never consumed"),
        "unexpected message: {err}"
    );
}
#[test]
fn check_poly_local_read_twice_is_use_after_move() {
    // Reading a non-`Copy` local a second time is use-after-move: the
    // polymorphic checker rejects it as the monomorphic sibling rejects
    // `( ^i64 -- ^i64 ^i64 ) | x | x x ;`, naming the variable.
    let err = check_src(": twice ( 'T -- 'T 'T ) | x | x x ;\n: main ( -- ) ;").unwrap_err();
    assert!(err.contains("use after move"), "unexpected message: {err}");
    assert!(err.contains("local `x`"), "unexpected message: {err}");
}
#[test]
fn check_poly_local_rebound_while_in_scope_is_error() {
    // R4 twin of the monomorphic rebinding rejection: a second `| x |`
    // while `x` is still in scope would orphan the first binding, leaking
    // the non-`Copy` value parked in it. Reject at compile time, naming the
    // variable, exactly as `( ^i64 ^i64 -- ^i64 ) | x | | x | x ;` is.
    let err = check_src(": shadow ( 'T 'T -- 'T ) | x | | x | x ;\n: main ( -- ) ;").unwrap_err();
    assert!(err.contains("already bound"), "unexpected message: {err}");
    assert!(err.contains('x'), "unexpected message: {err}");
}
#[test]
fn check_poly_duplicate_local_in_bind_group_is_error() {
    // A name repeated inside one bind group (`| x x |`) orphans the first
    // binding before the cross-group rebind guard can see it: the poly
    // checker rejects it as the monomorphic sibling rejects
    // `( ^i64 ^i64 -- ^i64 ) | x x | x ;`, naming the variable.
    let err = check_src(": bad ( 'T 'T -- 'T ) | x x | x ;\n: main ( -- ) ;").unwrap_err();
    assert!(err.contains("duplicate local"), "unexpected message: {err}");
    assert!(err.contains('x'), "unexpected message: {err}");
}
#[test]
fn check_poly_local_named_after_variant_is_error() {
    // A local named after a registered variant shadows the value that
    // name constructs: the poly binder rejects it as the monomorphic
    // sibling `( i64 i64 -- i64 )` of the same body does, naming the
    // collision.
    let err = check_src(
            "type: Maybe | None | Some v i64 ;\n: f ( 'T i64 -- 'T ) drop | Some | Some ;\n: main ( -- ) 1 2 f drop ;",
        )
        .unwrap_err();
    assert!(
        err.contains("collides with the variant name `Some`"),
        "unexpected message: {err}"
    );
}
#[test]
fn poly_self_call_structural_match_produces_outputs() {
    // P7 slice 3g (R1): a self-call whose operand window structurally
    // matches the walking word's own `sig.inputs` truncates and pushes
    // `sig.outputs`, letting the body finish typechecking to the
    // declared effect.
    check_src(": rec ( 'T i64 -- 'T ) drop 3 rec ;\n: main ( -- ) ;\n")
        .expect("a structurally matching self-call typechecks");
}
#[test]
fn poly_self_call_operand_mismatch_is_located_error() {
    // D1's termination witness: a self-call whose operand window does
    // not structurally match `sig.inputs` is an ordinary located type
    // mismatch (`poly_rendered_type_mismatch_error`), never a check-time
    // loop or a backend panic.
    let err = check_src(": rec ( 'T i64 -- 'T ) drop True rec ;\n: main ( -- ) ;\n").unwrap_err();
    assert!(
        err.contains("type mismatch in `rec`"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("`rec` expected `i64`, found `Bool`"),
        "unexpected message: {err}"
    );
}
#[test]
fn poly_self_call_underflow_reuses_arity_error() {
    // Too few operands ahead of a self-call is an ordinary arity
    // shortfall: the same underflow diagnostic any other operand-arity
    // gap produces, not a bespoke self-call message.
    let err = check_src(": rec ( 'T i64 -- 'T ) drop rec ;\n: main ( -- ) ;\n").unwrap_err();
    assert!(
        err.contains("`rec` needs 2 values, but the stack holds 1"),
        "unexpected message: {err}"
    );
}
/// P7.S3g-follow (1c): the one shape that can put a *body-derived*
/// reference in a poly self-call's argument window, so every back-edge
/// reference test is built from it. A declared `&!array['T 4]` is the only
/// reference type a poly-body borrow can ever match: a body borrow is
/// always `PolyType::Ref`, while a fully concrete `&!Cell` parameter folds
/// to `Concrete(Type::Ref(..))` at parse time and the two never compare
/// equal, so the referent has to stay variable-bearing. An array is then
/// the only borrowable local a generic body admits (a bare `'T` might
/// instantiate to a scalar, and a `Generic` application is not on the
/// borrowable list).
///
/// `'T: Copy` is an input slot of its own, so the leading `| r a b n |`
/// binds four of the five declared inputs and each arm opens over a
/// residual `'T`. Two array parameters, because the borrowed one cannot
/// also be named for the value slot (that is the ordinary aliasing
/// rejection, not this guard).
fn self_tail_ref_loop(name: &str, recursive_arm: &str) -> String {
    format!(
        ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : {name} ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero ~[ drop r drop 0 ] ~[ {recursive_arm} ] if ;\n\
             : main ( -- ) ;\n"
    )
}
#[test]
fn poly_self_tail_reference_to_a_local_across_the_back_edge_is_error() {
    // P7.S3g-follow (1c): `&!a` is derived from a local of this frame, and
    // the self-call it rides is the loop's back-edge, where the header
    // rebinds every local -- so next iteration the name denotes different
    // storage than the reference points at. Clean at HEAD before this
    // guard, while the monomorphic twin of the same body was already
    // rejected by `check_reference_across_back_edge`.
    let err = check_src(&self_tail_ref_loop(
        "loopg",
        "r drop &!a b dup n 1 sub loopg",
    ))
    .unwrap_err();
    assert_eq!(
            err,
            "error: a reference to a local cannot cross a loop in `loopg` (line 4)\n  a reference derived from `a`, a local of this frame, crosses the self-tail-call back-edge to `loopg`: that local's storage does not survive to the next iteration\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
}
#[test]
fn poly_self_tail_reference_to_a_local_across_the_back_edge_through_an_eliminator_arm_is_error() {
    // P7.S3g-follow (1a) follow-up: an eliminator arm inherits the call's
    // own tail-ness in `poly_eliminator_call` ("every eliminator arm runs
    // ... in the call's own position"), exactly as an `if`/`unless` arm
    // does, so the same back-edge hazard reached through `Bool?` instead
    // of `if` must be caught by the same guard. Same body as
    // `poly_self_tail_reference_to_a_local_across_the_back_edge_is_error`,
    // with the `if` swapped for the `Bool?` eliminator `if` lowers
    // through, and a `drop` in each arm for the narrowed variant `Bool?`
    // hands each arm (payload-less, but still a stack value) that `if`
    // never gives its arms.
    let err = check_src(
        ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero\n\
             ~[ ( False ) drop r drop &!a b dup n 1 sub loopg ]\n\
             ~[ ( True ) drop drop r drop 0 ]\n\
             Bool? ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: a reference to a local cannot cross a loop in `loopg` (line 5)\n  a reference derived from `a`, a local of this frame, crosses the self-tail-call back-edge to `loopg`: that local's storage does not survive to the next iteration\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
}
#[test]
fn poly_self_tail_reference_rooted_in_a_spliced_block_local_is_error() {
    // P7.S3g-follow (1c): the same hazard one `call`-splice deeper. `x` is
    // a local of the *spliced* block, whose storage is a slot of this same
    // frame, so a reference to it dies at the back-edge exactly as `&!a`
    // does. The splice exit `retain`s the enclosing locals but keeps the
    // borrow records, so `x` is gone from `scope.locals` by the time the
    // self-call is checked -- which is why the guard reads
    // `PolyBorrow::static_rooted` instead of looking the place up.
    let err = check_src(
            ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero ~[ drop r drop 0 ] ~[ r drop a ~[ | x | &!x ] call b dup n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
    assert_eq!(
            err,
            "error: a reference to a local cannot cross a loop in `loopg` (line 4)\n  a reference derived from `x`, a local of this frame, crosses the self-tail-call back-edge to `loopg`: that local's storage does not survive to the next iteration\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
}
#[test]
fn poly_self_tail_reference_rooted_in_a_local_shadowing_a_static_is_error() {
    // The second reason the guard cannot re-look-up the place: `A` names
    // both a static and a local here, and the borrow site resolves locals
    // first, so the record is rooted in the frame's slot however the name
    // reads from outside. Answering "is this a static?" by name at the
    // self-call would exempt it and let a dead reference cross.
    let err = check_src(
        "static: A i64 = 0 ;\n\
             : iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r A b n |\n\
             n iszero ~[ drop r drop 0 ] ~[ r drop &!A b dup n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: a reference to a local cannot cross a loop in `loopg` (line 5)\n  a reference derived from `A`, a local of this frame, crosses the self-tail-call back-edge to `loopg`: that local's storage does not survive to the next iteration\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
}
#[test]
fn poly_self_tail_dropped_borrow_then_forwarded_ref_is_over_conservative() {
    // `poly_combinator_call`'s doc comment on `tail_slots` (and the spec)
    // claim a concrete cost for crediting every arm with the call's own
    // tail-ness instead of refining per arm: a body that borrows a local
    // in *any* arm and then tail-recurses is rejected whichever arm the
    // borrow sat in, even when that borrow is dropped before the
    // back-edge and only a forwarded parameter reference actually rides
    // it. Pinned here rather than left to a doc comment, so a later
    // per-arm refinement has something to flip green. The monomorphic
    // twin (same body, `'T` replaced by `i64` and no residual bound slot)
    // accepts it: `check_reference_across_back_edge` sees the borrow of
    // `a` already dropped and only `r` -- an incoming reference parameter
    // -- crossing.
    let err = check_src(
        ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero ~[ drop r drop 0 ] ~[ &!a drop r b dup n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: a reference to a local cannot cross a loop in `loopg` (line 4)\n  a reference derived from `a`, a local of this frame, crosses the self-tail-call back-edge to `loopg`: that local's storage does not survive to the next iteration\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    check_src(
        ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ( &!array[i64 4] array[i64 4] array[i64 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero ~[ r drop 0 ] ~[ &!a drop r b dup n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
    )
    .expect("the monomorphic twin's dropped borrow does not ride the back-edge");
}
#[test]
fn poly_self_tail_reference_parameter_forwarded_across_the_back_edge_is_ok() {
    // The accept case the guard must not swallow: `r` is the *incoming*
    // reference parameter, whose referent lives in an ancestor frame that
    // outlives every iteration. Nothing in this body borrows, so nothing
    // is recorded to reject.
    check_src(&self_tail_ref_loop("loopg", "r b dup n 1 sub loopg"))
        .expect("a reference parameter may cross the back-edge");
}
#[test]
fn poly_non_tail_self_call_carrying_a_local_reference_is_ok() {
    // The per-term half of the gate (1a): the first arm's self-call has a
    // term after it, so it is *not* the back-edge -- it lowers as ordinary
    // recursion, into a fresh frame whose locals are bound once, and a
    // reference to a local is fine there. Written out rather than built by
    // `self_tail_ref_loop` because both halves of the gate have to be true
    // at once: the *second* arm carries the word's real back-edge (with a
    // parameter reference, which is legal), so `is_self_tail_call` holds
    // and `tail` is the only thing telling the two calls apart.
    check_src(
        ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero ~[ r drop &!a b dup n 1 sub loopg drop 0 ]\n\
             ~[ r b dup n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
    )
    .expect("a non-tail self-call is not a back-edge");
}
#[test]
fn poly_self_tail_call_in_a_builtin_named_word_skips_the_back_edge_guard() {
    // The word-level half of the gate (1a): `has_self_tail_call` refuses
    // every builtin spelling, so lowering gives a generic `lt` no loop
    // header however its body is written. The guard must agree, or it
    // rejects a program that lowers as ordinary recursion. Same body as
    // the rejection above, renamed.
    check_src(&self_tail_ref_loop("lt", "r drop &!a b dup n 1 sub lt"))
        .expect("a builtin-named word never gets the loop shape");
}
#[test]
fn poly_self_tail_reference_rooted_in_a_static_is_ok() {
    // A static's data-segment storage survives every iteration, unlike a
    // local's slot, so its borrow record must not be read as a hazard --
    // the poly twin of `static_ref_crosses_self_tail_call_back_edge_ok`.
    // The recorded `&!COUNT` is what the guard sees here; the reference
    // actually crossing is the parameter `r`.
    check_src(&format!(
        "static: COUNT i64 = 0 ;\n{}",
        self_tail_ref_loop("loopg", "&!COUNT drop r b dup n 1 sub loopg")
    ))
    .expect("a static-rooted borrow is not a local of this frame");
}
#[test]
fn poly_self_tail_call_with_no_reference_argument_ignores_a_live_local_borrow() {
    // The other half of the rule: a live borrow of a local is only a
    // hazard when a reference actually rides the back-edge. Here `&a` is
    // parked in a `Copy` local (a shared reference, so nothing demands it
    // be consumed) which keeps the record live under the coarse liveness,
    // while the call's own window carries no reference at all.
    check_src(
        ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T array['T 4] i64 -- i64 )\n\
             | a n |\n\
             n iszero ~[ drop 0 ] ~[ &a | p | a n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
    )
    .expect("a live local borrow alone is not a back-edge hazard");
}
#[test]
fn poly_self_tail_linear_forwarded_into_the_call_window_is_ok() {
    // The linear counterpart of the accept case: a `Spy` moved *into* the
    // recursive call's own argument window is forwarded, not stranded, so
    // the loop carries it as a back-edge operand with its single owner
    // intact. This is the whole of what a linear value can do at a poly
    // self-tail call -- see the two tests below.
    check_src(&format!(
        "{SPY}: iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( Spy 'T i64 -- Spy 'T )\n\
               dup iszero ~[ drop ] ~[ dup drop 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n"
    ))
    .expect("a linear value forwarded into the window stays legal");
}
#[test]
fn poly_self_tail_unconsumed_linear_local_is_error() {
    // Why there is no poly port of the monomorphic guard's *second*
    // clause (an unconsumed linear local at the back-edge): the general
    // end-of-body check already rejects the shape, one arm having
    // consumed `s` and the other not. The monomorphic clause exists only
    // to relocate that same rejection at the call (its own doc says so),
    // and a second message for an already-rejected program is not worth a
    // second rule. Pinned here so loosening the general check cannot open
    // the hole silently.
    let err = check_src(&format!(
        "{SPY}: iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( Spy 'T i64 -- 'T )\n\
               | s t n |\n\
               n iszero ~[ s drop t ] ~[ 9 Spy t n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n"
    ))
    .unwrap_err();
    assert_eq!(
            err,
            "error: linear value `s` is never consumed in `loopg`\n  `s` has type `Spy`, which is linear: drop it or return it (nothing is dropped for you)"
        );
}
#[test]
fn poly_self_tail_linear_stranded_below_the_call_window_is_not_well_typed() {
    // Why there is no poly port of the guard's *first* clause (a linear
    // value stranded below the argument window) either: a tail self-call
    // is the last term of a context whose exit row is the word's declared
    // outputs, and the call itself pushes exactly those outputs -- so
    // `stranded ++ outputs == outputs`, and nothing can be stranded in a
    // well-typed body. A generic body cannot even reach the shape from
    // below: unlike an inline combinator it walks no caller row, its
    // stack starts at `sig.inputs`.
    //
    // A tripwire, not a test of this slice's code: the day that exit-row
    // rule loosens, the stranded clause has to be written.
    let err = check_src(&format!(
        "{SPY}: iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T i64 -- Spy 'T )\n\
               | t n |\n\
               n iszero ~[ 9 Spy t ] ~[ 9 Spy t n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n"
    ))
    .unwrap_err();
    assert!(
            err.contains(
                "the quotations passed to `if` leave different stack shapes: an earlier one leaves `Spy 'T`, this one leaves `Spy Spy 'T`"
            ),
            "the stranded `Spy` shows up as the arms disagreeing: {err}"
        );
}
// -- P7.S3k: a generic word calling another generic word ---------------

/// P7.S3k (R1): the slice's headline shape. Replaces the retired
/// `poly_different_word_call_still_rejects` (and its `tests/` twins), which
/// pinned the `poly_calls_poly_word_error` narrowing this closes.
#[test]
fn check_generic_word_calls_same_module_generic_grounds() {
    check_src(": id ( 'T -- 'T ) ;\n: g ( 'T -- 'T ) id ;\n: main ( -- ) ;\n").unwrap();
}

/// P7.S3k (R1): the same call under the per-module mangling every real
/// build applies, which is the only thing that distinguishes an *imported*
/// callee from a same-module one at this level -- the arm dispatches on
/// `poly_env`, whose keys are post-mangle names, never on a spelling. The
/// end-to-end cross-module build golden is `tests/phase7_slice3k.rs`'s.
#[test]
fn check_generic_word_calls_mangled_generic_grounds() {
    check_src_mangled(": id ( 'T -- 'T ) ;\n: g ( 'T -- 'T ) id ;\n: main ( -- ) ;\n").unwrap();
}

/// P7.S3k (R2): what the walk actually produces -- one symbolic record per
/// grounded cross-call, keyed by the *containing* word, mapping the
/// callee's own variable to the caller's. Phase 2 composes exactly this
/// against a concrete θ, so its shape is the contract, not an incidental.
#[test]
fn check_generic_cross_call_records_the_caller_var_mapping() {
    let recorded = cross_calls_of(": id ( 'T -- 'T ) ;\n: g ( 'T -- 'T ) id ;\n: main ( -- ) ;\n");
    let calls = recorded.get("g").expect("`g`'s body made the call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].callee, "id");
    assert_eq!(calls[0].mapping, vec![(0, Image::CallerVar(0))]);
    // Keyed by the caller, not the callee: `id`'s own body calls nothing.
    assert!(!recorded.contains_key("id"), "recorded: {recorded:?}");
}

/// P7.S3k (R2/R6, the accept side of the growth rule): a *concrete* image.
/// The caller hands over an `i64` it built itself, so `'U` maps to a
/// ground type rather than to one of the caller's variables -- legal, and
/// the case a growth rule written as "reject anything but a bare variable"
/// would wrongly refuse.
#[test]
fn check_generic_cross_call_with_a_concrete_operand_grounds() {
    let recorded =
        cross_calls_of(": h ( 'U -- 'U ) ;\n: g ( 'T -- 'T ) 1 h drop ;\n: main ( -- ) ;\n");
    let calls = recorded.get("g").expect("`g`'s body made the call");
    assert_eq!(calls[0].mapping, vec![(0, Image::Concrete(Type::I64))]);
}

/// P7.S3k (R6, the distinction an implementer must not confuse): a callee
/// declaring its *own* compound parameter is structurally decomposed, so
/// the image of `'U` is the bare `'T` and nothing grew. The mirror of
/// `check_growing_cross_call_is_error` below, whose only difference is
/// which side wrote the wrapper.
#[test]
fn check_generic_cross_call_forwarding_a_reference_grounds() {
    let recorded =
        cross_calls_of(": peek ( &'U -- ) drop ;\n: g ( &'T -- ) peek ;\n: main ( -- ) ;\n");
    let calls = recorded.get("g").expect("`g`'s body made the call");
    assert_eq!(calls[0].mapping, vec![(0, Image::CallerVar(0))]);
}

/// P7.S3k (R3): a bound the callee needs and the caller does not declare
/// is a located error at the call site -- the user-declared-callee twin of
/// `check_generic_comparison_body_without_ord_is_error`'s library one.
#[test]
fn check_generic_cross_call_bound_mismatch_is_error() {
    let err = check_src(
        ": biggest ['U: Ord] ( 'U -- 'U ) ;\n: g ( 'T -- 'T ) biggest ;\n: main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`'U` of `biggest` requires `Ord`, which `'T` in `g` does not declare"),
        "unexpected message: {err}"
    );
}

/// P7.S3s (R2, `Bound::User` half of `poly_cross_bound_error`): the same
/// walk-time `Image::CallerVar` arm as `check_generic_cross_call_bound_
/// mismatch_is_error` above, but with a *user* trait rather than the
/// library `Ord` -- `g` forwards its own `'T` to `shows` without
/// declaring `Show`, so the callee's bound has no caller obligation to
/// discharge against. Pins the real trait name rendering in `bound`'s
/// `Bound::User(id)` arm, which had no coverage: collapsing it to the
/// placebo `"a user trait"` string left the whole suite green.
#[test]
fn check_generic_cross_call_forwarded_user_bound_mismatch_is_error() {
    let err = check_src(&format!(
        "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : g ( &'T -- ) shows ;\n\
             : main ( -- ) ;\n"
    ))
    .unwrap_err();
    assert!(
        err.contains("`'T` of `shows` requires `Show`, which `'T` in `g` does not declare"),
        "unexpected message: {err}"
    );
}

/// P7.S3k (R3): the same discharge against a *concrete* image runs the
/// ordinary predicate on the spot, so the caller's own bounds are not the
/// only route to a rejection. `Bool` is `Copy` but not `Ord`.
///
/// P7.S3s (review fix): `Ord` is now a `Bound::User`, and `compose`'s own
/// `resolve_user_bound` loop -- not the walk-time `Image::Concrete` arm,
/// which defers to it (R3) -- is what catches this. `compose` only runs
/// for a cross-call reached from `discover_transitive_instantiations`'s
/// fixpoint, itself seeded from a real instantiation, so `g` must be
/// called from `main`; the old fixture's `g` was dead code, which the
/// old `is_ord` (no registry, no reachability gate) still caught but
/// this design deliberately does not (R3's named residual).
#[test]
fn check_generic_cross_call_concrete_operand_failing_a_bound_is_error() {
    let err = check_src(
        ": biggest ['U: Ord] ( 'U -- 'U ) ;\n\
             : g ( 'T -- 'T ) True biggest drop ;\n\
             : main ( -- ) 1 g drop ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("cannot instantiate `'U` of `biggest` with `Bool`")
            && err.contains("does not satisfy `Ord`"),
        "unexpected message: {err}"
    );
}

/// P7.S3k (N1, review fix): the `Copy` discharge indexes the registries to
/// resolve a struct/enum image, and an instantiation this body's own walk
/// minted is not in them yet -- `check::check` appends it only after
/// `check_poly_body` returns. Both fixtures panicked inside `is_copy`'s
/// indexing before the guard, which N1 forbids outright; both are now
/// located. `Ord` needs no arm here (`is_ord` reads no registry) and a user
/// bound never reaches the loop (`poly_cross_signature_supported` rejects
/// one first).
///
/// The wider bug is not this slice's: `dup` on the same body-local
/// instantiation still panics through `poly_is_copy`, on a program with no
/// cross-call at all. See the spec's phase 2 finding 7.
#[test]
fn check_cross_call_copy_bound_on_a_body_local_instantiation_is_unsupported() {
    for (wrapper, ctor, rendered) in [
        ("type: Box['T] | Box 'T ;\n", "Box", "Box[i64]"),
        ("type: Cell['T] val 'T ;\n", "Cell", "Cell[i64]"),
    ] {
        let src = format!(
            "{wrapper}\
                 : h ['U: Copy] ( 'U -- ) drop ;\n\
                 : g ( 'T -- 'T ) 1 {ctor} h ;\n\
                 : main ( -- ) ;\n"
        );
        let err = check_src(&src).unwrap_err();
        assert!(
            err.contains(&format!(
                "discharging `Copy` on the body-local generic instantiation `{rendered}`"
            )) && err.contains("is not yet supported from a polymorphic body"),
            "unexpected message for {ctor}: {err}"
        );
    }
}

/// P7.S3k (R6): the caller wraps its own `'T` before handing it over, so
/// the image of the callee's bare `'U` is a compound over a caller
/// variable -- the growing case, rejected at the call site.
///
/// The wrapper is a generic **enum**, deliberately. An array wrapper
/// would be a placebo: the `[Type; Count]` array constructor is deleted
/// (P7.S5), so array construction in a polymorphic body has no surface
/// syntax and the growth rule would never be consulted. Sooth has no
/// generic structs, so a single-variant generic enum is the only
/// constructible wrapper. Do not add an array-based "second witness".
#[test]
fn check_growing_cross_call_is_error() {
    let err = check_src(
        "type: Box['T] | Box 'T ;\n\
             : h ( 'U -- 'U ) ;\n\
             : g ( 'T -- ) Box h drop ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("cannot pass `Box['T]` to `'U` of the polymorphic word `h`")
            && err.contains("builds a larger type at every hop"),
        "unexpected message: {err}"
    );
}

/// P7.S3k (R6, review fix): a *fully concrete* compound image (`&i64`,
/// built by `&n` on a scalar `static`) mentions no caller variable, so it
/// is not the growing case -- but folding it into `Image::Concrete` would
/// mint a fresh `RefId`, and the poly-body walk holds no mutable ref
/// registry to do that with. Pins the honest rejection
/// (`poly_cross_call_unsupported_error`) against a regression back to the
/// growth diagnostic, which would misdescribe this call as building a
/// larger type at every hop when nothing wraps `'T` at all.
#[test]
fn check_growing_cross_call_concrete_reference_is_unsupported_not_growth() {
    let err = check_src(
        "static: n i64 = 0 ;\n\
             : h ( 'U -- ) drop ;\n\
             : g ( 'T -- 'T ) &n h ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("cannot call the polymorphic word `h`")
            && err.contains("passing the concrete compound value `&i64`")
            && !err.contains("builds a larger type at every hop"),
        "unexpected message: {err}"
    );
}

/// P7.S3k (R2, review fix): two generic enums of the same arity (`Box`,
/// `Wrap`) are still distinguished by `(is_enum, idx, module)`, not just
/// arity -- the guard mutation a reviewer probed (weakening the compare
/// to arity-only) makes this silently compose `Wrap`'s variable against
/// `Box`'s argument instead of raising the ordinary mismatch below.
#[test]
fn check_generic_cross_call_same_arity_different_header_is_type_mismatch() {
    let err = check_src(
        "type: Box['T] | Box 'T ;\n\
             type: Wrap['U] | Wrap 'U ;\n\
             : unwrap ( Wrap['A] -- Wrap['A] ) ;\n\
             : g ( 'T -- ) Box unwrap drop ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("expected `Wrap['A]`") && err.contains("found `Box['T]`"),
        "unexpected message: {err}"
    );
}

/// P7.S3k (R2, review fix): the four structural guards in
/// `poly_cross_match`/`poly_cross_relate` that no test reached. Each was
/// deletable with the whole suite green, and each fails *open* -- the
/// operand-count guard into a subtract-overflow panic, the other three into
/// an accepted call: a `Bool` filling an `i64` parameter, a shared borrow
/// filling a mutable one, and a `array['T 4]` filling a `array['U 3]`. Table-driven
/// because the four share one shape (a callee the caller's operands cannot
/// fill) and differ only in which conjunct rejects them.
#[test]
fn check_cross_call_operands_the_callee_cannot_accept_are_errors() {
    for (fixture, expected) in [
        (
            ": h ( 'U 'U 'U -- ) drop drop drop ;\n\
                 : g ( 'T -- ) h ;\n: main ( -- ) ;\n",
            "`h` needs 3 values, but the stack holds 1",
        ),
        (
            ": h ( 'U i64 -- 'U ) drop ;\n\
                 : g ( 'T -- 'T ) True h ;\n: main ( -- ) ;\n",
            "`h` expected `i64`, found `Bool`",
        ),
        (
            ": h ( &!'U -- ) drop ;\n\
                 : g ( &'T -- ) h ;\n: main ( -- ) ;\n",
            "`h` expected `&!'U`, found `&'T`",
        ),
        (
            ": h ( &array['U 3] -- ) drop ;\n\
                 : g ( &array['T 4] -- ) h ;\n: main ( -- ) ;\n",
            "`h` expected `array['U 3]`, found `array['T 4]`",
        ),
    ] {
        let err = check_src(fixture).unwrap_err();
        assert!(err.contains(expected), "expected `{expected}`, got: {err}");
    }
}

/// P7.S3k (R2, the consistency requirement): one callee variable matched
/// against two different caller variables cannot be one type at any
/// instantiation. The symbolic twin of `poly_var_conflict_error`, which the
/// concrete path raises for the same shape against two ground types.
#[test]
fn check_inconsistent_cross_call_mapping_is_error() {
    let err = check_src(
        ": pair ( 'U 'U -- 'U 'U ) ;\n\
             : g ( 'A 'B -- 'A 'B ) pair ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("matched `'U` to both `'A` and `'B`"),
        "unexpected message: {err}"
    );
}

/// P7.S6a (R8a): a length lives in a header's own `len_args` too, not
/// only in a bare array's `Len::Var` -- `poly_mentions_len_var`'s
/// `Generic` arm must scan `len_args` as well as `args`, or this callee
/// slips the same cross-call rejection a bare-array length variable
/// already gets in the table below. Before the widening, `Buffer['T
/// 'N]` carried no length-mentioning leaf this guard could see, so the
/// call would have been silently admitted -- exactly the poly-body
/// cross-call this slice is out of scope for.
#[test]
fn check_cross_call_rejects_a_length_variable_carried_in_a_generic_header() {
    let err = check_src(
        "type: Buffer['T 'N: Len] data array['T 'N] ;\n\
             : capacity['T 'N: Len] ( Buffer['T 'N] -- usize ) drop 0 >usize ;\n\
             : g['T 'N: Len] ( Buffer['T 'N] -- usize ) capacity ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("a length variable in the callee's signature"),
        "unexpected message: {err}"
    );
}

/// P7.S6a (R8, round-4 review fix): a header carrying a *concrete*
/// length (spellable since R7) is invisible to `poly_mentions_len_var`
/// (it only matches `Len::Var`), so this cross-call reaches
/// `poly_cross_match`'s `Generic`/`Generic` arm rather than being turned
/// away earlier the way a length-*variable* header is above. Before this
/// arm compared `len_args`, the guard tuple ignored them entirely and
/// admitted `caller`'s `Buffer['T 8]` operand into `sink`'s declared
/// `Buffer['T 4]` parameter -- silently accepted at check time, then
/// either lowered against the wrong monomorph or tripped
/// `subst_polytype`'s `.expect` in `src/ir/driver.rs` when the other
/// monomorph was never separately minted.
#[test]
fn check_cross_call_rejects_a_mismatched_concrete_length_in_a_generic_header() {
    let err = check_src(
        "type: Buffer['T 'N: Len] data array['T 'N] ;\n\
             : sink['T]   ( Buffer['T 4] -- ) drop ;\n\
             : caller['T] ( Buffer['T 8] -- ) sink ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("expected `Buffer['T 4]`") && err.contains("found `Buffer['T 8]`"),
        "unexpected message: {err}"
    );
}

/// P7.S3k: the callee signature shapes a symbolic mapping cannot carry are
/// each a located rejection naming that shape, not the whole-feature
/// narrowing they replaced. Three of the four are pinned here (review
/// fix: the row case was missing, contradicting the deviations doc's
/// claim that it was already pinned); the fourth, a declared quotation
/// parameter, is pinned separately by
/// `poly_quotlit_against_legal_inline_quotation_param_rejects_at_the_cross_call`,
/// which needs a `~[ ]` fixture this table's plain `check_src` shape
/// cannot build. A fifth shape, a `Bound::User` on a mapped variable,
/// used to be pinned here too; P7.S3s R2 deletes that rejection outright
/// (`check_generic_cross_call_discharges_a_forwarded_user_bound` and
/// `check_generic_cross_call_concrete_image_with_no_impl_is_a_located_error`
/// pin the resolution it replaces), so it is no longer a member of this
/// table.
#[test]
fn check_cross_call_unsupported_callee_shapes_name_themselves() {
    for (fixture, what) in [
        (
            ": alen ( array['E 'N] -- array['E 'N] usize ) len ;\n\
                 : g ( array['T 4] -- array['T 4] ) alen drop ;\n: main ( -- ) ;\n",
            "a length variable in the callee's signature",
        ),
        (
            "type: Box['T] | Box 'T ;\n\
                 : box ( 'U -- Box['U] ) Box ;\n\
                 : g ( 'T -- ) box drop ;\n: main ( -- ) ;\n",
            "returning the compound type `Box['U]` from a polymorphic word",
        ),
        (
            ": dup2 ['a: Copy 'b: Copy] ( ..s 'a 'b -- ..s 'a 'b 'a 'b ) over over ;\n\
                 : g ['T: Copy] ( 'T -- 'T 'T 'T 'T ) dup2 ;\n: main ( -- ) ;\n",
            "calling a row-polymorphic word",
        ),
    ] {
        let src = format!("{SHOW}{fixture}");
        let err = check_src(&src).unwrap_err();
        assert!(
            err.contains(what) && err.contains("is not yet supported from a polymorphic body"),
            "expected `{what}`, got: {err}"
        );
    }
}

/// P7.S3s (R2): the headline capability -- a polymorphic body may call
/// another polymorphic word carrying a `Bound::User` on a forwarded
/// variable, and the composed callee's `trait_calls` resolves to the
/// implementing word's own lowering symbol, not merely a program that
/// type-checks. `g` forwards its own `'T: Show` to `shows`; `main`
/// instantiates `g` at `Point`, and the fixpoint must ground `shows`'s
/// body-recorded `show` obligation against that same `Point`.
#[test]
fn check_generic_cross_call_discharges_a_forwarded_user_bound() {
    let (module, _) = checked_like_a_build(&format!(
        "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : g ['T: Show] ( &'T -- ) shows ;\n\
             : main ( -- ) 1 2 Point |p| &p g p drop ;\n"
    ))
    .expect("the fixture checks");
    let composed = module
        .transitive_instantiations
        .iter()
        .find(|i| i.callee == "shows")
        .expect("g's cross-call to shows composed a monomorph");
    let resolved: Vec<&str> = composed.trait_calls.values().map(String::as_str).collect();
    assert_eq!(resolved, vec!["show;Show;0;Point"]);
}

/// P7.S3s (R3): the concrete-image half of the same loop. `g`'s body
/// builds `Other` itself, a struct already in the registry with no
/// `impl: Show`, and hands `&o` to `shows` -- the mapped operand is
/// `Image::Concrete(Other)` at walk time, never a forwarded caller
/// variable. `compose`'s `resolve_user_bound` loop is what raises this,
/// not a code change to the `(Image::Concrete(_), Bound::User(_))`
/// walk-time arm (R3): that arm still records `None` and defers.
#[test]
fn check_generic_cross_call_concrete_image_with_no_impl_is_a_located_error() {
    let err = check_src(&format!(
        "{SHOW}type: Other n i64 ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : g ( 'T -- ) drop 7 Other |o| &o shows o drop ;\n\
             : main ( -- ) 1 g ;\n"
    ))
    .unwrap_err();
    assert!(
        err.contains("`Other` does not satisfy `Show`: no `( &Other -- )` found"),
        "unexpected message: {err}"
    );
}

/// P7.S3k (R2/N1): a grounded cross-call records *only* the symbolic
/// mapping. It mints no instantiation of its own, so nothing in the
/// existing `Span`-keyed table moves -- phase 2's composition is what adds
/// one, from this record.
#[test]
fn check_generic_cross_call_records_no_instantiation() {
    let (module, _) = checked_like_a_build(
        ": id ( 'T -- 'T ) ;\n: g ( 'T -- 'T ) id ;\n: main ( -- ) 1 g drop ;\n",
    )
    .expect("the fixture checks");
    // `main`'s call to `g` is the one instantiation; `g`'s call to `id`
    // is not one yet.
    let callees: Vec<&str> = module
        .instantiations
        .values()
        .map(|c| c.callee.as_str())
        .collect();
    assert_eq!(callees, vec!["g"]);
    assert_eq!(module.poly_cross_calls["g"].len(), 1);
}

/// The composed monomorphs of a checked source, symbol-keyed, and the
/// routing map each concrete instantiation carries -- the two records
/// phase 2's fixpoint writes.
fn transitive_of(src: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let (module, _) = checked_like_a_build(src).expect("the fixture checks");
    let mut routed: Vec<Vec<String>> = module
        .instantiations
        .values()
        .map(|inst| {
            let mut syms: Vec<String> =
                inst.poly_calls.values().map(|c| c.symbol.clone()).collect();
            syms.sort();
            syms
        })
        .collect();
    routed.sort();
    (
        module
            .transitive_instantiations
            .iter()
            .map(|c| c.symbol.clone())
            .collect(),
        routed,
    )
}

/// P7.S3k (R4): the composition step. `main` instantiates `g` at `i64`,
/// `g`'s body calls `id`, and the fixpoint grounds that record into a real
/// `(id, i64)` monomorph -- routed at `g`'s own body span, and recorded
/// flat so lowering emits one `IrFunc` for it.
#[test]
fn transitive_discovery_composes_the_callee_instantiation() {
    let (transitive, routed) =
        transitive_of(": id ( 'T -- 'T ) ;\n: g ( 'T -- 'T ) id ;\n: main ( -- ) 1 g drop ;\n");
    assert_eq!(transitive, vec!["sooth_mono_id__t0_i64"]);
    assert_eq!(routed, vec![vec!["sooth_mono_id__t0_i64"]]);
}

/// P7.S3k (R4): once per *distinct* instantiation the caller reaches. Two
/// call sites at the same θ compose the same symbol, so the fixpoint's
/// dedup collapses them -- without it lowering would emit the callee's
/// `IrFunc` twice under one name and the assembler would refuse the module.
#[test]
fn transitive_discovery_dedups_repeated_instantiation_symbol() {
    let (transitive, routed) = transitive_of(
        ": id ( 'T -- 'T ) ;\n\
             : g ( 'T -- 'T ) id ;\n\
             : main ( -- ) 1 g drop 2 g drop ;\n",
    );
    assert_eq!(transitive, vec!["sooth_mono_id__t0_i64"]);
    // Both call sites still route -- deduping the *emit* set must not drop
    // either site's routing entry.
    assert_eq!(
        routed,
        vec![vec!["sooth_mono_id__t0_i64"], vec!["sooth_mono_id__t0_i64"]]
    );
}

/// P7.S3k (R4): and one *per* distinct θ. Three types, asymmetric with
/// each other, since two cannot tell "sorted" from "the 50% of
/// `HashMap` iteration orders that happen to already be sorted" -- the
/// dedup source (`insts`) iterates a map, so an unsorted `transitive`
/// vec is only randomized, not reliably out of order, at n=2.
#[test]
fn transitive_discovery_composes_one_monomorph_per_distinct_theta() {
    let (transitive, _) = transitive_of(
        ": id ( 'T -- 'T ) ;\n\
             : g ( 'T -- 'T ) id ;\n\
             : main ( -- ) 1 g drop \"x\" g drop 1 2 eq g drop ;\n",
    );
    assert_eq!(
        transitive,
        vec![
            "sooth_mono_id__t0_e0_Bool",
            "sooth_mono_id__t0_i64",
            "sooth_mono_id__t0_str"
        ]
    );
}

/// P7.S3k (R5/N3): a mutual cycle terminates because the second visit to
/// `(g, θ)` mints the symbol the first already claimed. One monomorph per
/// word, and `g`'s own seed entry is not duplicated into the flat set.
#[test]
fn transitive_discovery_terminates_on_a_mutual_cycle() {
    let (transitive, _) = transitive_of(
        ": h ( 'U -- 'U ) g 0 drop ;\n\
             : g ( 'T -- 'T ) h 0 drop ;\n\
             : main ( -- ) 1 g drop ;\n",
    );
    assert_eq!(transitive, vec!["sooth_mono_h__t0_i64"]);
}

/// P7.S3k (R4, phase 1 finding 1): an `inline` callee is *spliced* at the
/// call site and mints no `IrFunc`, so the fixpoint must compose nothing
/// for it. Routing the span would emit a call to a symbol that never
/// exists -- a link failure for a case that already worked. The record
/// itself is still made: how `h` is lowered is not the record's business.
#[test]
fn transitive_discovery_skips_an_inline_callee() {
    let src = ": h inline ( 'U -- 'U ) ;\n: g ( 'T -- 'T ) h ;\n: main ( -- ) 1 g drop ;\n";
    let (module, _) = checked_like_a_build(src).expect("the fixture checks");
    assert_eq!(module.poly_cross_calls["g"].len(), 1);
    assert!(
        module.transitive_instantiations.is_empty(),
        "composed: {:?}",
        module.transitive_instantiations
    );
    for inst in module.instantiations.values() {
        assert!(inst.poly_calls.is_empty(), "routed: {:?}", inst.poly_calls);
    }
}

/// P7.S3k (R4, review finding 1): unlike `h` above, `h`'s own body here
/// calls a polymorphic word. Nothing composes a θ for that inner call --
/// `h` is never walked by `check_poly_body` at all (it is checked
/// standalone, at a concrete dummy type), so the fixpoint has no record
/// of what `h` calls -- and lowering really does splice `h`'s generic
/// body at `g`'s call site. A located rejection, not the panic
/// ("checked user word exists") or the silent wrong-symbol routing this
/// gap used to reach.
#[test]
fn transitive_discovery_rejects_an_inline_callee_whose_body_calls_a_poly_word() {
    let src = ": id ( 'T -- 'T ) ;\n: h inline ( 'U -- 'U ) id ;\n: g ( 'T -- 'T ) h ;\n: main ( -- ) 1 g drop ;\n";
    let err =
        checked_like_a_build(src).expect_err("an inline callee's own cross-call is unroutable");
    assert!(
        err.contains("cannot call the polymorphic word `h`") && err.contains("inline"),
        "unexpected error: {err}"
    );
}

/// The same gap, one `inline` hop further: `g` calls `h`, `h` is `inline`
/// and itself calls `k`, another `inline` word whose own body calls a
/// polymorphic word. The rejection fires at `g`'s call to `h` -- a call
/// to *any* polymorphic word, inline or not, is what `body_calls_a_poly_word`
/// looks for, so it does not need to recurse into `k`'s body itself.
#[test]
fn transitive_discovery_rejects_an_inline_callee_two_hops_from_the_poly_call() {
    let src = ": id ( 'T -- 'T ) ;\n\
             : k inline ( 'V -- 'V ) id ;\n\
             : h inline ( 'U -- 'U ) k ;\n\
             : g ( 'T -- 'T ) h ;\n\
             : main ( -- ) 1 g drop ;\n";
    let err = checked_like_a_build(src).expect_err("a two-hop inline chain is unroutable too");
    assert!(
        err.contains("cannot call the polymorphic word `h`"),
        "unexpected error: {err}"
    );
}

/// P7.S3k (R4/R8): a composed callee returning two or more values carries
/// its interned return bundle on *both* records -- the flat emit entry and
/// the routing copy -- and they agree, since `intern_bundle_struct` dedups
/// by output tuple. Only the routing copy is read today (`lower_poly_call`
/// asks the call site what shape came back); pinned on both so a reader of
/// `transitive_instantiations` cannot find the invariant broken on one.
#[test]
fn transitive_discovery_interns_a_composed_bundle_on_both_records() {
    let (module, _) = checked_like_a_build(
        ": two ( 'T -- 'T i64 ) 9 ;\n\
             : g ( 'T -- 'T i64 ) two ;\n\
             : main ( -- ) 1 g drop drop ;\n",
    )
    .expect("the fixture checks");
    let [composed] = &module.transitive_instantiations[..] else {
        panic!(
            "one composed callee: {:?}",
            module.transitive_instantiations
        );
    };
    assert_eq!(composed.out_arity, 2);
    let bundle = composed.bundle.expect("the flat entry carries its bundle");
    let routed: Vec<Option<StructId>> = module
        .instantiations
        .values()
        .flat_map(|inst| inst.poly_calls.values().map(|c| c.bundle))
        .collect();
    assert_eq!(routed, vec![Some(bundle)]);
}

/// P7.S3k (R4/R2): a two-variable callee reached *both* from a concrete
/// caller and through a cross-call is monomorphized once. That holds only
/// if a composed θ orders its entries the way `unify_poly_input` does for
/// the concrete path, since `instantiation_symbol` reads them positionally.
/// Asymmetric (`i64`, `str`) on purpose: a symmetric pair cannot tell the
/// two orders apart.
#[test]
fn transitive_discovery_shares_a_callee_monomorph_with_the_concrete_path() {
    let (module, _) = checked_like_a_build(
        ": swap2 ( 'A 'B -- 'B 'A ) swap ;\n\
             : g ( 'X 'Y -- 'Y 'X ) swap2 ;\n\
             : main ( -- ) 1 \"s\" swap2 drop drop 2 \"t\" g drop drop ;\n",
    )
    .expect("the fixture checks");
    let concrete: Vec<&str> = module
        .instantiations
        .values()
        .filter(|i| i.callee == "swap2")
        .map(|i| i.symbol.as_str())
        .collect();
    assert_eq!(concrete, vec!["sooth_mono_swap2__t0_i64_t1_str"]);
    let routed: Vec<&str> = module
        .instantiations
        .values()
        .flat_map(|inst| inst.poly_calls.values().map(|c| c.symbol.as_str()))
        .collect();
    assert_eq!(routed, concrete, "the cross-call reaches that same symbol");
    // And so needs no monomorph of its own: the fixpoint's seed dedup
    // already claimed it. A composed θ ordered the other way would mint
    // `..._t0_str_t1_i64`, miss the dedup, and land a second `IrFunc` here.
    assert!(
        module.transitive_instantiations.is_empty(),
        "composed: {:?}",
        module.transitive_instantiations
    );
}

/// P7.S3k (N2): a program with no cross-call records composes nothing and
/// routes nothing, which is what makes the emitted IL of the existing
/// corpus identical by construction rather than by inspection.
#[test]
fn transitive_discovery_is_empty_without_a_cross_call() {
    let (transitive, routed) = transitive_of(": id ( 'T -- 'T ) ;\n: main ( -- ) 1 id drop ;\n");
    assert!(transitive.is_empty(), "composed: {transitive:?}");
    assert_eq!(routed, vec![Vec::<String>::new()]);
}

/// P7.S3k (R4/N1, phase 1 finding 2): a polymorphic overload set merges
/// into one `poly_cross_calls` entry, and each record's mapping indexes its
/// *own* candidate's variables -- so composing one candidate's mapping
/// against the other's θ would mint a wrong monomorph silently. Rejected,
/// located, from either side of the call.
#[test]
fn check_cross_call_through_an_overloaded_generic_word_is_error() {
    for (fixture, overloaded) in [
        (
            ": id ( 'T -- 'T ) ;\n\
                 : f ( 'T -- 'T ) id ;\n\
                 : f ( 'A 'B -- 'A 'B ) swap swap ;\n\
                 : main ( -- ) 1 f drop ;\n",
            "`f` names more than one polymorphic word",
        ),
        (
            ": id ( 'T -- 'T ) ;\n\
                 : id ( 'A 'B -- 'A 'B ) swap swap ;\n\
                 : g ( 'T -- 'T ) id ;\n\
                 : main ( -- ) 1 g drop ;\n",
            "`id` names more than one polymorphic word",
        ),
    ] {
        let err = check_src(fixture).unwrap_err();
        assert!(
            err.contains(overloaded),
            "expected `{overloaded}`, got: {err}"
        );
    }
}

#[test]
fn check_poly_body_with_if_accepts_choose() {
    // T1: a polymorphic body may branch. Slice 10c: `inline`, because
    // `if` is an ordinary word taking two quotation literals and a
    // non-spliced polymorphic body rejects a quotation outright; the
    // branch now runs through the ordinary splice, which is what makes the
    // move-join below the thing under test. `choose` consumes `a` and `b` on
    // both arms but at different sites; the move-join must recognise
    // `Moved`+`Moved` as consumed-once (not a leak), or `choose` would be
    // wrongly rejected at the word end (M1).
    assert!(
            check_src(
                ": choose inline ( 'T 'T Bool -- 'T ) | a b flag | flag ~[ a b drop ] ~[ b a drop ] if ;\n: main ( -- ) 1 2 True choose drop ;",
            )
            .is_ok(),
            "choose should type-check"
        );
}
/// Slice 10c: T2/T4/T5/T8 below drove `poly_walk`'s own `PolyType` move
/// tracker, which went with its branch arm -- `if` is an ordinary word
/// taking two quotations now, and a spliced body's arms are tracked by the
/// shared branch-and-join over concrete `Slot`s. Each is retargeted onto a
/// concrete linear type, so the guarantee it pinned (an arm-local leak, a
/// one-armed consume joining to `MaybeMoved`, a use after a `Moved` join)
/// is still guarded, at the site that now decides it. A stand-in `'T` no
/// longer works: the def-site check instantiates it at `i64`, which is
/// `Copy`, so nothing could leak.
#[test]
fn check_arm_local_unconsumed_is_error() {
    // T2: `y` is bound inside the `then` arm and never consumed in it.
    let err = check_src(&format!(
            "{SPY}: arm_leak ( Spy Spy Bool -- Spy ) | a b flag | flag ~[ a b | y | ] ~[ a drop b ] if ;\n: main ( -- ) ;",
        ))
        .unwrap_err();
    assert!(err.contains('y'), "names the arm-local: {err}");
    assert!(err.contains("never consumed"), "unexpected message: {err}");
}
#[test]
fn check_poly_branch_moved_on_both_arms_is_accepted() {
    // T3: `a`/`b` consumed on both arms (`Moved`+`Moved` => `Moved`), so
    // nothing leaks at the word end.
    assert!(
            check_src(
                ": both inline ( 'T 'T Bool -- ) | a b flag | flag ~[ a drop b drop ] ~[ b drop a drop ] if ;\n: main ( -- ) ;",
            )
            .is_ok(),
            "both should type-check"
        );
}
#[test]
fn check_branch_moved_on_one_arm_leaks() {
    // T4: `x` consumed on the `then` arm only (`Moved`+`Live` =>
    // `MaybeMoved`), which the leak check must count as still-unconsumed
    // (M3).
    let err = check_src(&format!(
        "{SPY}: one ( Spy Bool -- ) | x flag | flag ~[ x drop ] ~[ ] if ;\n: main ( -- ) ;"
    ))
    .unwrap_err();
    assert!(err.contains('x'), "names the leaked local: {err}");
    assert!(
        err.contains("is not consumed on every path"),
        "unexpected message: {err}"
    );
}
#[test]
fn check_branch_moved_on_neither_arm_leaks() {
    // T5: `x` untouched on both arms (`Live`+`Live` => `Live`); a value
    // parked in a local across a branch still leaks at the word end (M4).
    let err = check_src(&format!(
        "{SPY}: none ( Spy Bool -- ) | x flag | flag ~[ ] ~[ ] if ;\n: main ( -- ) ;"
    ))
    .unwrap_err();
    assert!(err.contains('x'), "names the leaked local: {err}");
    assert!(err.contains("never consumed"), "unexpected message: {err}");
}
#[test]
fn check_branch_condition_not_bool_is_error() {
    // T6: `if`'s condition must be a `Bool`. Slice 10c: the guard is now
    // `if`'s own declared parameter type rather than a hand-written arm,
    // and a spliced poly body reports the operand at its instantiated
    // stand-in type.
    let err = check_src(": bad inline ( 'T 'T -- 'T ) ~[ drop ] ~[ drop ] if ;\n: main ( -- ) ;")
        .unwrap_err();
    assert!(err.contains("if"), "names the `if`: {err}");
    assert!(err.contains("`Bool`"), "names the expected type: {err}");
}
#[test]
fn check_branch_depth_mismatch_is_error() {
    // T7: the arms leave different stack depths (then: 1, else: 2). Slice
    // 10c catches that at the *argument* site (R-P2-3), comparing one arm
    // literal's actual exit shape against its sibling's, rather than at a
    // join after both were walked. `'T`
    // carries a `Copy` bound so the repeated reads are not use-after-move,
    // leaving the depth mismatch as the sole failure this test proves.
    let err = check_src(
            ": bad inline ['T: Copy] ( 'T Bool -- 'T ) | x flag | flag ~[ x ] ~[ x x ] if ;\n: main ( -- ) ;",
        )
        .unwrap_err();
    assert!(
        err.contains("leave different stack shapes"),
        "unexpected message: {err}"
    );
}
#[test]
fn check_branch_use_after_join_is_error() {
    // T8: both arms consume `x` (the join is `Moved`), so the `x drop`
    // after the branch is a second read: use-after-move, not a leak.
    let err = check_src(&format!(
            "{SPY}: bad ( Spy Bool -- ) | x flag | flag ~[ x drop ] ~[ x drop ] if x drop ;\n: main ( -- ) ;"
        ))
        .unwrap_err();
    assert!(err.contains("use after move"), "unexpected message: {err}");
    assert!(err.contains('x'), "names the moved local: {err}");
}
#[test]
fn check_poly_dup_of_variable_element_array_names_type_variable() {
    // R7/`poly_copy_gate` array arm: `dup` of an array whose element is an
    // unbounded `'T` recurses to the element and names the variable, not a
    // fabricated `i64`.
    let err =
        check_src(": bad ( array['T 'N] -- array['T 'N] array['T 'N] ) dup ;\n: main ( -- ) ;")
            .unwrap_err();
    assert!(err.contains("'T"), "unexpected message: {err}");
    assert!(err.contains("Copy"), "unexpected message: {err}");
}
#[test]
fn check_poly_dup_of_linear_element_array_names_element_type() {
    // `poly_copy_gate` array arm: `dup` of a length-variable array whose
    // element is a concrete linear struct names that struct, never `i64`.
    let err = check_src(&format!(
        "{SPY}: bad ( array[Spy 'N] -- array[Spy 'N] array[Spy 'N] ) dup ;\n: main ( -- ) ;"
    ))
    .unwrap_err();
    assert!(err.contains("Spy"), "unexpected message: {err}");
    assert!(err.contains("linear"), "unexpected message: {err}");
}
#[test]
fn check_poly_dup_of_an_owning_cell_is_rejected() {
    // P7.S3n (R3): both halves of the cell's `Copy` answer at once.
    // `poly_is_copy` must say `false` -- a `^T` owns its payload at every
    // instantiation, so answering `true` would let two names free one
    // allocation -- and `poly_copy_gate` must then have an arm to render
    // it, since without one it falls to whichever sibling arm claims the
    // shape.
    let err = check_src(": bad ( ^'T -- ^'T ^'T ) dup ;\n: main ( -- ) ;").unwrap_err();
    assert_eq!(
            err,
            "error: cannot `dup` an owning cell in `bad` (line 1)\n  `^'T` is linear: it owns its payload, so duplicating it would free the same allocation twice"
        );
}
#[test]
fn poly_op_on_variable_error_names_a_reference() {
    // Slice 13 (review fix): `poly_op_on_variable_error`'s `Ref` describer
    // (`"a reference"`) is reachable from source -- `len` rejects a
    // reference the same way it rejects a bare type variable -- but had
    // no test asserting the exact wording.
    let err = check_src(": f ( &array['T 4] -- usize ) len ;\n").unwrap_err();
    assert_eq!(
        err,
        "error: `len` is not permitted on a reference in `f` (line 1)"
    );
}
#[test]
fn poly_type_str_renders_a_quotation_row() {
    // Slice 10a (R10): the row is a separate field on `PolyType::Quotation`,
    // not a slot in `ins`/`outs`, so `poly_type_str` must render it
    // explicitly as the leading element of each side -- dropping that
    // rendering must leave no trace of the row name in the output.
    let sig = PolySig {
        row_in: Some(0),
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: Some(0),
        bounds: Vec::new(),
        ty_var_names: Vec::new(),
        ty_var_spans: Vec::new(),
        ty_kinds: Vec::new(),
        len_var_names: Vec::new(),
        len_var_spans: Vec::new(),
        row_var_names: vec!["..s".to_string()],
    };
    let quot = PolyType::Quotation(
        vec![PolyType::Concrete(Type::I64)],
        Vec::new(),
        true,
        Some(0),
        Some(0),
    );
    assert_eq!(poly_type_str(&quot, &sig), "~[ ..s i64 -- ..s ]");
}

/// A signature over one type variable `'T` and one length variable `'N`,
/// the shape every Slice 13 reference test names its referent from.
fn ref_sig() -> PolySig {
    PolySig {
        row_in: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: None,
        bounds: Vec::new(),
        ty_var_names: vec!["'T".to_string()],
        ty_var_spans: Vec::new(),
        ty_kinds: Vec::new(),
        len_var_names: vec!["'N".to_string()],
        len_var_spans: Vec::new(),
        row_var_names: Vec::new(),
    }
}

fn poly_ref(referent: PolyType, mutable: bool) -> PolyType {
    PolyType::Ref(Box::new(referent), mutable)
}

#[test]
fn poly_type_str_renders_a_reference() {
    // Slice 13 (R-A9): the sigil is glued to the referent's own rendering,
    // so a poly reference reads back exactly as it was written.
    let sig = ref_sig();
    assert_eq!(
        poly_type_str(&poly_ref(PolyType::Var(0), false), &sig),
        "&'T"
    );
    assert_eq!(
        poly_type_str(&poly_ref(PolyType::Var(0), true), &sig),
        "&!'T"
    );
    let arr = PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(4));
    assert_eq!(poly_type_str(&poly_ref(arr, false), &sig), "&array['T 4]");
}

#[test]
fn poly_type_str_renders_a_generic_application() {
    // P7 slice 3a: `Name['A 'B]` in the signature's own variable
    // spellings -- `name` is cached on the variant, so no registry
    // lookup is needed to render it.
    let sig = PolySig {
        row_in: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: None,
        bounds: Vec::new(),
        ty_var_names: vec!["'T".to_string(), "'E".to_string()],
        ty_var_spans: Vec::new(),
        ty_kinds: Vec::new(),
        len_var_names: Vec::new(),
        len_var_spans: Vec::new(),
        row_var_names: Vec::new(),
    };
    let result = PolyType::Generic {
        is_enum: true,
        idx: 0,
        module: 0,
        args: vec![PolyType::Var(0), PolyType::Var(1)],
        len_args: vec![],
        name: "Result",
    };
    assert_eq!(poly_type_str(&result, &sig), "Result['T 'E]");
}

#[test]
fn poly_type_str_renders_a_variable_application() {
    // P7b.S1 (S1-14): `PolyType::App` renders as `'F['T]` -- the
    // applied variable's own surface spelling, then its arguments.
    let sig = PolySig {
        row_in: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: None,
        bounds: Vec::new(),
        ty_var_names: vec!["'F".to_string(), "'T".to_string()],
        ty_var_spans: Vec::new(),
        ty_kinds: Vec::new(),
        len_var_names: Vec::new(),
        len_var_spans: Vec::new(),
        row_var_names: Vec::new(),
    };
    let app = PolyType::App {
        head: 0,
        args: vec![PolyType::Var(1)],
    };
    assert_eq!(poly_type_str(&app, &sig), "'F['T]");
}

#[test]
fn poly_type_str_renders_member_local_var_past_caller_ty_var_names() {
    // P7b.S6 Phase 1 (R2.a, M3): a member-local `Var(n)` (e.g. `map`'s
    // own `'U`, index 1 in the member's table) must render against the
    // *member's* sig, not the caller's -- the caller's `ty_var_names` may
    // be shorter (here, length 1), which is exactly the index-out-of-
    // bounds panic `trait_member_operand_error` used to hit by rendering
    // both sides against one shared (caller) sig.
    let caller_sig = PolySig {
        row_in: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: None,
        bounds: Vec::new(),
        ty_var_names: vec!["'F".to_string()],
        ty_var_spans: Vec::new(),
        ty_kinds: Vec::new(),
        len_var_names: Vec::new(),
        len_var_spans: Vec::new(),
        row_var_names: Vec::new(),
    };
    let member_sig = PolySig {
        row_in: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: None,
        bounds: Vec::new(),
        ty_var_names: vec!["'T".to_string(), "'U".to_string()],
        ty_var_spans: Vec::new(),
        ty_kinds: Vec::new(),
        len_var_names: Vec::new(),
        len_var_spans: Vec::new(),
        row_var_names: Vec::new(),
    };
    // Member-local `Var(1)` would index off the end of the caller's
    // single-entry `ty_var_names`; rendered against the member sig it
    // resolves cleanly.
    assert_eq!(poly_type_str(&PolyType::Var(1), &member_sig), "'U");
    assert_eq!(poly_type_str(&PolyType::Var(0), &caller_sig), "'F");
}

#[test]
fn poly_type_str_renders_a_var_past_the_sig_tables_as_a_placeholder() {
    // P7b.S8b (round-3 review, P0): the twin of the test above. That one
    // fixes the *caller*, rendering each side against the sig that owns
    // it; this one covers the case where no such sig is in hand -- a
    // construction field carries its own header's variable ids, and the
    // only sig at the mismatch is the caller's. Rendering must degrade to
    // a placeholder, never index out of bounds: `k.sth`'s
    // `Holder['T] r Ring['T 3]` against a caller declaring no type
    // variables at all is a real program, and it used to ICE here.
    let sig = PolySig {
        row_in: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: None,
        bounds: Vec::new(),
        ty_var_names: Vec::new(),
        ty_var_spans: Vec::new(),
        ty_kinds: Vec::new(),
        len_var_names: Vec::new(),
        len_var_spans: Vec::new(),
        row_var_names: Vec::new(),
    };
    assert_eq!(poly_type_str(&PolyType::Var(0), &sig), "'?0");
    assert_eq!(
        poly_type_str(
            &PolyType::Array(Box::new(PolyType::Concrete(Type::I64)), Len::Var(2)),
            &sig
        ),
        "array[i64 '?len2]"
    );
    // The whole point of the placeholder: the two sides of a mismatch
    // stay distinguishable, and a length variable never reads as a type
    // one.
    assert_eq!(
        poly_type_str(
            &PolyType::App {
                head: 1,
                args: vec![PolyType::Var(0)],
            },
            &sig
        ),
        "'?1['?0]"
    );
}

#[test]
fn poly_type_str_renders_a_generic_application_with_len_args() {
    // P7.S6a (R3): the `Generic` arm's `len_args` widening -- a
    // length-carrying generic application must print its length
    // component (by the signature's own length-variable spelling for a
    // `Var`, literally for a `Concrete`) after the type args, not print
    // `Buffer[i64]` for `Buffer[i64 256]`.
    let sig = PolySig {
        row_in: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: None,
        bounds: Vec::new(),
        ty_var_names: Vec::new(),
        ty_var_spans: Vec::new(),
        ty_kinds: Vec::new(),
        len_var_names: vec!["'N".to_string()],
        len_var_spans: Vec::new(),
        row_var_names: Vec::new(),
    };
    let buffer = PolyType::Generic {
        is_enum: false,
        idx: 0,
        module: 0,
        args: vec![PolyType::Concrete(Type::I64)],
        len_args: vec![Len::Concrete(256), Len::Var(0)],
        name: "Buffer",
    };
    assert_eq!(poly_type_str(&buffer, &sig), "Buffer[i64 256 'N]");
}

#[test]
fn poly_generic_receiver_is_aggregate_projection() {
    // P7 slice 3a: an ungrounded generic application is a struct or enum
    // header, so it is a projection receiver exactly as a concrete
    // `Type::Struct`/`Type::Enum` is (R1's table).
    let stack = vec![PolySlot::new(PolyType::Generic {
        is_enum: true,
        idx: 0,
        module: 0,
        args: Vec::new(),
        len_args: vec![],
        name: "Result",
    })];
    assert!(receiver_is_aggregate_projection(&stack));
}

#[test]
fn poly_generic_slot_is_not_copy() {
    // P7 slice 3a (D5): a generic applied to a variable is
    // conservatively linear -- `dup`/`over` on it is rejected outright,
    // never derived per-argument.
    let err = check_src("type: Box['T] val 'T ;\n: dup-box ( Box['T] -- Box['T] Box['T] ) dup ;\n")
        .unwrap_err();
    assert_eq!(
            err,
            "error: cannot `dup` a generic type applied to a variable in `dup-box` (line 2)\n  `Box['T]` is conservatively linear: it may carry a linear argument at some instantiation, so it cannot be duplicated"
        );
}

#[test]
fn declared_poly_reference_signature_round_trips() {
    // Slice 13 (R-A10, the Part A exit criterion): a poly word may
    // *declare* a borrow, and the declaration survives parse + fold +
    // rendering unchanged. Producing one is Part B.
    let tokens = lex(": peek ( array['T 4] -- &array['T 4] ) ;").unwrap();
    let module = crate::test_support::parse_with_core(&tokens).unwrap();
    let sig = module.words[0].poly.as_ref().expect("poly sig present");
    assert_eq!(poly_type_str(&sig.outputs[0], sig), "&array['T 4]");
}

/// P7 slice 3c (R8.3): a slice inside a polymorphic body is the point of
/// the type, so each poly predicate is pinned over one. All four answer
/// through `PolyType::Concrete` delegation -- the element is concrete by
/// construction, so a slice never takes a poly shape of its own -- and
/// that delegation is exactly what these guard: deleting the monomorphic
/// `is_copy`/`is_ref` slice arms breaks the poly path too, silently.
#[test]
fn poly_is_copy_mutable_slice_is_not() {
    let mut slices = Vec::new();
    let shared = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
    let mutable = crate::ast::intern_slice_type(&mut slices, Type::I64, true);
    let sig = bare_sig();
    assert!(poly_is_copy(
        &PolyType::Concrete(shared),
        &sig,
        &[],
        &[],
        &[]
    ));
    assert!(!poly_is_copy(
        &PolyType::Concrete(mutable),
        &sig,
        &[],
        &[],
        &[]
    ));
    // The gate `dup`/`over` run: a mutable view is refused with the
    // exclusivity wording, not the linear-ownership wording, since a view
    // owns nothing.
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let err = poly_copy_gate(
        &PolyType::Concrete(mutable),
        "dup",
        &sig,
        &ctx,
        Span::default(),
        &[],
        &[],
        &[],
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: cannot `dup` a value of type `!Slice[i64]` in `probe` (line 0)\n  `!Slice[i64]` is exclusive: at most one may be live for a place, so copying it would make a second one; use it where it is, or borrow again once it is consumed\n  note: declared ( -- )"
        );
    poly_copy_gate(
        &PolyType::Concrete(shared),
        "dup",
        &sig,
        &ctx,
        Span::default(),
        &[],
        &[],
        &[],
    )
    .expect("a shared view is `Copy`");
}

/// P7 slice 3c (R8.3): a live slice keeps the borrow it was built from
/// observable, so `prune_dead_borrows` must not forget that borrow while
/// one is on the stack.
#[test]
fn is_reference_slot_true_for_slice() {
    let mut slices = Vec::new();
    let shared = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
    let mutable = crate::ast::intern_slice_type(&mut slices, Type::I64, true);
    assert!(is_reference_slot(&PolyType::Concrete(shared)));
    assert!(is_reference_slot(&PolyType::Concrete(mutable)));
    assert!(!is_reference_slot(&PolyType::Concrete(Type::I64)));
}

/// P7 slice 3c (R8.3): the poly renderer spells a slice the way the
/// signature does, so a diagnostic naming one is copy-pasteable source.
#[test]
fn poly_type_str_renders_slice() {
    let mut slices = Vec::new();
    let shared = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
    let mutable = crate::ast::intern_slice_type(&mut slices, Type::I64, true);
    let sig = bare_sig();
    assert_eq!(
        poly_type_str(&PolyType::Concrete(shared), &sig),
        "Slice[i64]"
    );
    assert_eq!(
        poly_type_str(&PolyType::Concrete(mutable), &sig),
        "!Slice[i64]"
    );
}

/// P7 slice 3c (R9.1, poly half): `len` answers a slice's carried length
/// and consumes the slot, like `str` and unlike the array arms -- an array
/// is a place that stays put, a slice is a value on the stack, so leaving
/// it would strand a residual slot in `0 s len >i64`.
#[test]
fn poly_len_over_a_slice_ok() {
    let mut slices = Vec::new();
    let shared = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
    let sig = bare_sig();
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let env: HashMap<String, Vec<Overload>> = HashMap::new();
    let mut scope = PolyScope::default();
    let mut overloads = HashMap::new();
    let stack = poly_term(
        &Term {
            kind: TermKind::Call("len".to_string(), Vec::new(), Vec::new()),
            span: Span::default(),
        },
        vec![PolySlot::new(PolyType::Concrete(shared))],
        &mut scope,
        &sig,
        &ctx,
        &env,
        &CombinatorEnv::default(),
        &[],
        &[],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut overloads,
        &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
        scratch_cross!(),
        false,
    )
    .expect("`len` answers a slice");
    assert_eq!(
        stack.iter().map(|s| s.pt.clone()).collect::<Vec<_>>(),
        vec![PolyType::Concrete(Type::Usize)]
    );
}

/// P7 slice 3c (R9.2/R10, phase 4): the poly walk's own slice arms --
/// `&>` on a view, `subslice`, and `slice` off a body borrow. Phase 3 left
/// all three as rejections (`&>` fell to `poly_op_on_variable_error`,
/// the two words to `unknown word`), so a generic body could `len` a view
/// and nothing else.
#[test]
fn poly_slice_words_index_subrange_and_construct() {
    // A view indexed inside a generic body yields an element reference,
    // and the sub-view keeps the receiver's own type.
    check_src(": f ( Slice[i64] 'T -- i64 'T ) | x | 0 >usize &> @ x ;\n").unwrap();
    check_src(": f ( Slice[i64] 'T -- usize 'T ) | x | 0 >usize 2 >usize subslice len x ;\n")
        .unwrap();
    // `slice` off a borrow taken in the body: the length may be generic
    // (a view erases it into a runtime length), the element may not.
    check_src(
        ": f ( array[i64 3] 'T -- array[i64 3] usize 'T ) | x | | a | &a slice len a swap x ;\n",
    )
    .unwrap();
    check_src(
        ": f ( array['T 3] 'T -- array['T 3] usize 'T ) | x | | a | &a slice len a swap x ;\n",
    )
    .unwrap_err();
    // ...and the view it builds inherits the borrow's mutability: a
    // shared one could not be written through here.
    check_src(
        ": f ( array[i64 3] i64 'T -- array[i64 3] 'T ) | x | | v | | a | \
             &!a slice 0 >usize &!> v ! a x ;\n",
    )
    .unwrap();
}

/// R1.2: a generic *element* is a locked non-goal, and the rejection says
/// so by name rather than reporting an unrelated shape mismatch -- a
/// generic *length* is fine, which is the whole point of a view.
#[test]
fn poly_slice_over_a_generic_element_is_a_located_error() {
    let err = check_src(
        ": f ( array['T 3] 'T -- array['T 3] usize 'T ) | x | | a | &a slice len a swap x ;\n",
    )
    .unwrap_err();
    assert_eq!(
        err,
        "error: `slice` over an array of `'T` in `f` (line 1) is not supported\n  \
             a view's element type must be concrete; only its length may be generic"
    );
}

/// R9.2 (poly half): the mutability of a slice receiver is part of the
/// match, exactly as it is on the concrete path, and the wording is the
/// same one -- the two paths must not disagree about one spelling. (The
/// empty `note: declared ( -- )` is the poly path's own: a polymorphic
/// word carries no concrete effect for the note to render, and every
/// mismatch it reports says the same.)
#[test]
fn poly_index_of_a_slice_matches_on_mutability() {
    check_src(": f ( !Slice[i64] i64 'T -- 'T ) | x | | v | 0 >usize &!> v ! x ;\n").unwrap();
    let err = check_src(": f ( !Slice[i64] 'T -- i64 'T ) | x | 0 >usize &> @ x ;\n").unwrap_err();
    assert_eq!(
        err,
        "error: type mismatch in `f` (line 1)\n  \
             `&>` expected a slice, found `!Slice[i64]`\n  \
             note: declared ( -- )"
    );
    let err = check_src(": f ( Slice[i64] i64 'T -- 'T ) | x | | v | 0 >usize &!> v ! x ;\n")
        .unwrap_err();
    assert!(
        err.contains("`&!>` expected a mutable slice, found `Slice[i64]`"),
        "unexpected message: {err}"
    );
}

/// The poly twin of `check_slice_offset`: a `usize` passes, an `i64`
/// literal passes (the monomorphic path admits one too), a computed `i64`
/// needs the explicit conversion, and a bare variable is a located error.
/// Unlike the array twin there is no count to bound a literal against.
#[test]
fn check_poly_slice_offset_admits_usize_and_literals_only() {
    let sig = ref_sig();
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let span = Span::default();
    let slot = |pt: PolyType, lit: Option<i64>| PolySlot {
        pt,
        int_val: lit,
        quot: None,
    };
    check_poly_slice_offset(
        &slot(PolyType::Concrete(Type::Usize), None),
        &ctx,
        span,
        "&>",
        &sig,
    )
    .expect("a `usize` offset passes");
    check_poly_slice_offset(
        &slot(PolyType::Concrete(Type::I64), Some(9999)),
        &ctx,
        span,
        "&>",
        &sig,
    )
    .expect("a literal needs no compile-time bound: the trap is at runtime");
    check_poly_slice_offset(
        &slot(PolyType::Concrete(Type::I64), None),
        &ctx,
        span,
        "&>",
        &sig,
    )
    .expect_err("a computed i64 needs the explicit `>usize`");
    check_poly_slice_offset(&slot(PolyType::Var(0), None), &ctx, span, "&>", &sig)
        .expect_err("a bare type variable is not an offset");
}

/// Call-site witness for `check_poly_slice_offset`: the direct-call test
/// above proves the function's own logic, but not that the three sites
/// wiring it in (`subslice`'s start and length, `&>`'s index) actually
/// call it. A computed (non-literal) `i64` operand at each site must be
/// rejected the same way a bare direct call is.
#[test]
fn poly_slice_offset_sites_reject_a_computed_i64() {
    let err = check_src(
        ": f ( Slice[i64] i64 'T -- usize 'T ) | x | | k | k 2 >usize subslice len x ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`subslice` mixes `usize` with a computed `i64`"),
        "unexpected message: {err}"
    );
    let err = check_src(
        ": f ( Slice[i64] i64 'T -- usize 'T ) | x | | k | 0 >usize k subslice len x ;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`subslice` mixes `usize` with a computed `i64`"),
        "unexpected message: {err}"
    );
    let err =
        check_src(": f ( Slice[i64] i64 'T -- i64 'T ) | x | | k | k &> @ x ;\n").unwrap_err();
    assert!(
        err.contains("`&>` mixes `usize` with a computed `i64`"),
        "unexpected message: {err}"
    );
}

/// R12 (poly half): a mutable view is exclusivity-tracked in a generic
/// body too -- but by the poly walk's *move* tracking, not by a reborrow:
/// a non-`Copy` local is consumed on read there, so a mutable view is
/// single-use per binding where the concrete path reborrows it. Ruled on
/// here rather than discovered in a golden (see the phase's exit notes).
#[test]
fn poly_mutable_slice_local_is_single_use() {
    check_src(": f ( Slice[i64] 'T -- usize usize 'T ) | x | | s | s len s len x ;\n").unwrap();
    let err = check_src(": f ( !Slice[i64] 'T -- usize usize 'T ) | x | | s | s len s len x ;\n")
        .unwrap_err();
    assert!(
        err.contains("use after move in `f`") && err.contains("local `s` is linear"),
        "unexpected message: {err}"
    );
}

#[test]
fn poly_is_copy_tracks_a_reference_mutability_not_its_referent() {
    // Slice 13 (D3/R-A5): mirrors the monomorphic `is_copy` on
    // `Type::Ref` -- shared is `Copy`, mutable is not, and the referent's
    // own linearity is irrelevant either way. Answering `True`
    // unconditionally would let a generic body freely `dup` an exclusive
    // borrow, an acceptance every concrete instantiation rejects.
    let sig = ref_sig();
    let linear_referent = PolyType::Var(0); // no `Copy` bound
    assert!(poly_is_copy(
        &poly_ref(linear_referent.clone(), false),
        &sig,
        &[],
        &[],
        &[]
    ));
    assert!(!poly_is_copy(
        &poly_ref(linear_referent, true),
        &sig,
        &[],
        &[],
        &[]
    ));
}

#[test]
fn poly_copy_gate_rejects_a_mutable_reference() {
    // Slice 13 (E1): the gate's reference arm is a real located
    // diagnostic, not an `unreachable!` -- `dup` on a `&!` must reject,
    // and on a `&` must still pass (the positive control).
    let sig = ref_sig();
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let span = Span {
        line: 7,
        col: 3,
        ..Span::default()
    };
    let err = poly_copy_gate(
        &poly_ref(PolyType::Var(0), true),
        "dup",
        &sig,
        &ctx,
        span,
        &[],
        &[],
        &[],
    )
    .expect_err("`dup` of a mutable reference must be rejected");
    assert_eq!(
            err,
            "error: cannot `dup` a mutable reference in `probe` (line 7)\n  `&!'T` is not `Copy`: duplicating it would let two names observe or mutate through one exclusive borrow",
        );
    poly_copy_gate(
        &poly_ref(PolyType::Var(0), false),
        "dup",
        &sig,
        &ctx,
        span,
        &[],
        &[],
        &[],
    )
    .expect("`dup` of a shared reference is permitted");
}

/// P7.S6a (R3, added review round 4): the `Operative::Generic`
/// construction site `poly_eliminator_call` reaches when its scrutinee
/// is an ungrounded `PolyType::Generic { is_enum: true, .. }` carries
/// the scrutinee's own `len_args` forward -- must fail if this
/// construction site (as opposed to its already-covered
/// destructure/consumption site) drops it.
///
/// Review fix: the prior version of this test called
/// `operative_generic_from_scrutinee` directly with hand-supplied
/// arguments, which only pins the helper's own body -- it would still
/// pass if the real call site (`poly_eliminator_call`'s match arm at
/// `:3171`) stopped calling the helper, or called it with the wrong
/// field (e.g. `args` twice, or a hardcoded `&[]`). This drives
/// `poly_eliminator_call` itself with a hand-built `Generic` scrutinee
/// and lets its `Full` arm destructure through `poly_destructure_generic`
/// (the already-covered sibling site), whose own `enum_sites.push` reads
/// the `len_args` off the *narrowed arm type* the match arm under test
/// built -- so a dropped or wrong `len_args` at `:3171` surfaces here as
/// a mismatched recorded site, not just as a mismatched helper return.
#[test]
fn operative_generic_construction_site_carries_len_args_from_its_scrutinee() {
    let mut generics = GenericTypes::with_bases(0, 0);
    generics.enums.push(GenericEnumDecl {
        name: "Ring".to_string(),
        ty_var_names: vec!["'T".to_string()],
        ty_kinds: Vec::new(),
        len_var_names: vec!["'N".to_string()],
        variants: vec![
            GenericVariantDecl {
                name: "Full".to_string(),
                fields: vec![("0".to_string(), PolyType::Var(0))],
                span: Span::default(),
            },
            GenericVariantDecl {
                name: "Empty".to_string(),
                fields: Vec::new(),
                span: Span::default(),
            },
        ],
        span: Span::default(),
        module: 0,
    });
    let cell = RefCell::new(generics);
    let scrutinee_len_args = vec![Len::Var(0), Len::Concrete(4)];
    let scrutinee_pt = PolyType::Generic {
        is_enum: true,
        idx: 0,
        module: 0,
        args: vec![PolyType::Var(0)],
        len_args: scrutinee_len_args.clone(),
        name: "Ring",
    };
    let mut scope = PolyScope::default();
    let full_quot = scope.intern_quotation(PolyQuotLit {
        body: vec![
            Term {
                kind: TermKind::Call("Full>".to_string(), Vec::new(), Vec::new()),
                span: Span::default(),
            },
            Term {
                kind: TermKind::Call("drop".to_string(), Vec::new(), Vec::new()),
                span: Span::default(),
            },
        ],
        span: Span::default(),
        is_inline: true,
        annot: Some(AnnotEffect {
            inputs: Vec::new(),
            outputs: Vec::new(),
            span: Span::default(),
            variant_tag: Some(VariantTag {
                name: "Full".to_string(),
                mode: VariantTagMode::Owning,
            }),
        }),
    });
    let empty_quot = scope.intern_quotation(PolyQuotLit {
        body: vec![Term {
            kind: TermKind::Call("Empty>".to_string(), Vec::new(), Vec::new()),
            span: Span::default(),
        }],
        span: Span::default(),
        is_inline: true,
        annot: Some(AnnotEffect {
            inputs: Vec::new(),
            outputs: Vec::new(),
            span: Span::default(),
            variant_tag: Some(VariantTag {
                name: "Empty".to_string(),
                mode: VariantTagMode::Owning,
            }),
        }),
    });
    let stack = vec![
        PolySlot::new(scrutinee_pt),
        PolySlot::quotation(full_quot),
        PolySlot::quotation(empty_quot),
    ];
    let sig = bare_sig();
    let probe = probe_word();
    let base_ctx = probe_ctx(&probe);
    let ctx = Ctx {
        generics: Some(&cell),
        ..base_ctx
    };
    let env: HashMap<String, Vec<Overload>> = HashMap::new();
    let mut obligations = Vec::new();
    let mut enum_sites = Vec::new();
    let mut tctx = TraitCtx::scratch(&mut obligations, &mut enum_sites);
    poly_eliminator_call(
        EliminatorTarget::Generic { idx: 0 },
        "Ring?",
        Span::default(),
        stack,
        &mut scope,
        &sig,
        &ctx,
        &env,
        &CombinatorEnv::default(),
        &[],
        &[],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut HashMap::new(),
        &mut tctx,
        scratch_cross!(),
        false,
    )
    .expect("a Full/Empty eliminator over a Generic scrutinee dispatches");
    // `poly_eliminator_call` records its own scrutinee site plus the
    // `Full` arm's own destructure site (the already-covered sibling in
    // `poly_destructure_generic`); every one of them must carry the
    // scrutinee's own `len_args` unchanged, since a dropped or
    // wrong-valued `len_args` at the construction site under test
    // (`:3171`) would poison every downstream site alike.
    assert!(!enum_sites.is_empty(), "at least one site is recorded");
    for (span, site) in &enum_sites {
        let PolyType::Generic { len_args, .. } = site else {
            panic!("the recorded site is a Generic scrutinee: {site:?}")
        };
        assert_eq!(
            len_args, &scrutinee_len_args,
            "every recorded site (span {span:?}) must carry the construction \
                 site's own len_args unchanged"
        );
    }
}

// -- Phase 2 (R-B1..R-B6): production and checking --------------------

#[test]
fn first_reads_an_array_element_through_a_poly_borrow() {
    // R-B2/R-B3/R-B4: the P2 read witness type-checks -- `&a` borrows
    // the aggregate local, `0` is a literal index bounds-checked against
    // the concrete length 4, and `@` fetches the `Copy` element.
    check_src(
        ": first['T: Copy] ( array['T 4] -- 'T ) | a | &a 0 &> @ ;\n\
             : main ( -- ) 10 4 fill first drop ;\n",
    )
    .expect("a shared prefix borrow, array-element ref, and fetch should check");
}

#[test]
fn poly_reference_word_rejects_borrowing_a_bare_variable_local() {
    // E2/D5: a bare `'T` local might instantiate to a scalar, which has
    // no address, so it is refused uniformly rather than deferred.
    let err = check_src(": badvar ['T: Copy] ( 'T -- 'T )\n  | t |\n  &t\n;\n").unwrap_err();
    assert_eq!(
            err,
            "error: cannot borrow the local `t` of type `'T` in `badvar` (line 3, col 3)\n  `'T` might instantiate to a scalar, which has no address; borrow an aggregate (a struct, enum, array, or owning cell) instead"
        );
}

#[test]
fn poly_reference_word_rejects_borrowing_a_concrete_scalar_local() {
    // Phase 2 review: D5's aggregate gate has two arms and only the
    // bare-variable one was covered. A concrete scalar local is not an
    // aggregate either, and takes the non-variable arm.
    let err =
        check_src(": g ['T: Copy] ( i64 'T -- 'T ) | n t | &n drop n drop t ;\n").unwrap_err();
    assert_eq!(
            err,
            "error: cannot borrow the local `n` of type `i64` in `g` (line 1, col 41)\n  only an aggregate (a struct, enum, array, or owning cell) is borrowable; `i64` is not"
        );
}

#[test]
fn borrowing_a_quotation_local_is_rejected() {
    // R-B8's `&q` witness. UPDATED after the slice 12 rebase: slice 12
    // retired `is_combinator`'s quotation-parameter inference leg (a word
    // now splices only when it *declares* `inline`), so `ap`'s ordinary,
    // non-`inline` `[ 'T -- 'T ]` parameter no longer makes it a
    // combinator -- it is checked as a genuine poly body, and
    // `poly_reference_word` itself rejects the quotation-typed local `f`
    // directly, rather than the splice path naming a monomorphic
    // instantiation. Second update (review): a quotation gets its own
    // wording (`poly_borrow_of_quotation_local_error`), not the generic
    // "not an aggregate" text -- a non-`inline` word's ordinary `[ ... ]`
    // parameter *is* a three-word aggregate at the ABI level, so that claim
    // is False at the representation the backend emits even though the
    // type system still refuses the borrow.
    let err = check_src(
            ": ap ( 'T [ 'T -- 'T ] -- 'T ) | x f | f &f drop x swap call ;\n: main ( -- ) 3 [ 1 add ] ap drop ;\n",
        )
        .unwrap_err();
    assert_eq!(
            err,
            "error: cannot borrow the local `f` of type `[ 'T -- 'T ]` in `ap` (line 1, col 42)\n  a quotation is not borrowable in a generic body"
        );
}

/// P7 slice 3c (R1.4): the widened `is_ref()` routes a slice scrutinee
/// into the *reference*-scrutinee arm, whose advice ("pass the owned
/// `Enum` instead") names nothing real for a view over a buffer. It gets
/// the plain mismatch instead -- the very message the concrete path
/// already gives the same scrutinee, so the two paths agree.
#[test]
fn poly_eliminator_with_a_slice_scrutinee_reports_a_plain_mismatch() {
    let err = check_src(
        "type: Shape | Circle r i64 | Square s i64 ;\n\
             : g ( 'T Slice[i64] -- 'T )\n  \
               ~[ ( Circle ) Circle> drop ] ~[ ( Square ) Square> drop ] Shape? ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert_eq!(
        err,
        "error: type mismatch in `g` (line 3)\n  \
             `Shape?` expected `Shape`, found `Slice[i64]`\n  \
             note: declared ( -- )"
    );
}

/// P7.S12 (R1.1/R5.1, R8.8): the branch is selected by the *scrutinee*,
/// never by the registry entry that got the call here. `Option` has a
/// monomorph, so R2.3 registers `EliminatorTarget::Concrete` -- and
/// `probe`'s scrutinee is still the ungrounded header, so the generic
/// branch has to run under a concrete gate.
#[test]
fn poly_eliminator_takes_the_generic_branch_under_a_concrete_gate() {
    check_src(
        "type: Option['T] | None | Some 'T ;\n\
             : mki ( i64 -- Option[i64] ) Some ;\n\
             : probe ( Option['T] -- i64 )\n  \
               ~[ ( Some ) drop 1 ] ~[ ( None ) drop 0 ] Option? ;\n\
             : main ( -- ) 7 mki probe drop ;\n",
    )
    .expect("an ungrounded scrutinee eliminates under a monomorph's gate");
}

/// The reverse direction (R8.8): nothing instantiates `Option` at parse
/// time, so the gate is `EliminatorTarget::Generic` -- and the scrutinee
/// is the concrete `Option[i64]` this body's own walk minted one term
/// earlier, so the *concrete* branch has to run under a generic gate.
#[test]
fn poly_eliminator_takes_the_concrete_branch_under_a_generic_gate() {
    check_src(
        "type: Option['T] | None | Some 'T ;\n\
             : probe ( 'T -- i64 )\n  \
               drop 5 Some ~[ ( Some ) drop 1 ] ~[ ( None ) drop 0 ] Option? ;\n\
             : main ( -- ) 7 probe drop ;\n",
    )
    .expect("a body-local mint eliminates under its header's gate");
}

/// R1.1: the gate still gates. A scrutinee of a *different* family is the
/// plain mismatch it always was -- only a different monomorph, or the
/// header, of the *same* family is admitted.
#[test]
fn poly_eliminator_rejects_a_scrutinee_of_another_generic_family() {
    let err = check_src(
        "type: Option['T] | None | Some 'T ;\n\
             type: Pair['A] | Nil | One 'A ;\n\
             : probe ( Pair['T] -- i64 )\n  \
               ~[ ( Some ) drop 1 ] ~[ ( None ) drop 0 ] Option? ;\n\
             : main ( -- ) ;\n",
    )
    .unwrap_err();
    assert_eq!(
        err,
        "error: type mismatch in `probe` (line 4)\n  \
             `Option?` expected `Option`, found `Pair['T]`\n  \
             note: declared ( -- )"
    );
}

/// R5.2/R5.4 (R8.8): a **two**-parameter header, eliminated at two
/// monomorphs that are each other's argument swap (`Res[i64 Pt]` and
/// `Res[Pt i64]`). The variant list comes off the header rather than off
/// any monomorph, and each arm's narrowed input carries the scrutinee's
/// own argument list unchanged (`args.clone()`, arity 2).
///
/// This pins arity and family, not positionality: nothing in this phase
/// *reads* a narrowed variant's fields, so a swap of the carried `args` is
/// unobservable until the destructure intercept lands (R8.3, whose
/// asymmetric instantiation is mutation 3's witness).
#[test]
fn poly_eliminator_narrows_a_two_parameter_header_at_swapped_monomorphs() {
    check_src(
        "type: Res['A 'B] | Ok v 'A | Err e 'B ;\n\
             type: Pt x i64 y i64 ;\n\
             : oki ( i64 -- Res[i64 Pt] ) Ok ;\n\
             : okp ( Pt -- Res[Pt i64] ) Ok ;\n\
             : is-ok ( Res['T 'U] -- i64 )\n  \
               ~[ ( Ok ) drop 1 ] ~[ ( Err ) drop 0 ] Res? ;\n\
             : main ( -- ) 7 oki is-ok drop 1 2 Pt okp is-ok drop ;\n",
    )
    .expect("one poly body eliminates two swapped monomorphs of one header");
}

/// R6.1 (R8.8): the destructure intercept pushes a variant's fields in
/// declared order, first field deepest. Two *distinct* field type
/// variables make the order observable at check time: `swap drop` keeps
/// what `Both>` left on top, which has to be `snd`'s `'B`.
#[test]
fn poly_destructure_pushes_variant_fields_first_field_deepest() {
    check_src(
        "type: Two['A 'B] | Both fst 'A snd 'B ;\n\
             : take-b ( Two['A 'B] -- 'B ) ~[ ( Both ) Both> swap drop ] Two? ;\n\
             : main ( -- ) ;\n",
    )
    .expect("the second field is left on top");
}

#[test]
fn poly_array_index_literal_unknown_length_requires_usize_conversion() {
    // P7.S6c (R1.1/R3.1): a *literal* index into a generic-length array
    // still cannot be statically bounds-checked -- there is no count to
    // check it against. A `usize` index, or a computed value converted
    // with `>usize`, is admitted instead and defers to runtime.
    let err =
        check_src(": badidx ( array['T 'N] -- 'T )\n  | a |\n  &a 0\n  &>\n  @\n;\n").unwrap_err();
    assert_eq!(
            err,
            "error: cannot index a generic-length array with a literal index in `badidx` (line 4, col 3)\n  the array's length is the type variable `'N`, so a literal index cannot be statically bounds-checked; use a `usize` index instead (a bound local, or `>usize` on a computed value), which defers the check to runtime"
        );
}

#[test]
fn poly_reference_word_rejects_borrowing_a_moved_local() {
    // E5: borrowing is not a move, but the referent still has to be
    // there -- a local already consumed holds nothing.
    let err = check_src(": badmove ( array['T 4] -- 'T )\n  | a |\n  a drop\n  &a 0 &> @\n;\n")
        .unwrap_err();
    assert_eq!(
            err,
            "error: use after move in `badmove` (line 4)\n  local `a` is linear and was moved at line 3, col 3, so it is used exactly once"
        );
}

#[test]
fn poly_reference_word_rejects_owning_cell_accessor_in_a_generic_body() {
    // R-B6/E4: `&^` never produces a variable-referent ref (no generic
    // structs/enums this slice), so it is out of scope regardless of
    // mutability -- a located error, not a silent unknown-word one.
    let err = check_src(": badcell ( 'T -- 'T )\n  &^\n;\n").unwrap_err();
    assert_eq!(
            err,
            "error: `&^` is not yet supported in a generic body, in `badcell` (line 2)\n  monomorphize this word (or write a concrete wrapper) to use `&^` today"
        );
}

#[test]
fn poly_reference_word_rejects_a_fused_accessor_spelling_in_a_generic_body() {
    // R-B6/E4: a leftover `&Struct>field` (retired in P7 slice 1) still
    // lexes as one `&`-prefixed token, and the `>`-bearing guard keeps it a
    // located error rather than a bare unknown-word one. The surviving
    // spelling's case is `projection_on_generic_receiver_body_is_error`.
    let err = check_src(": badfield ( 'T -- 'T )\n  &Point>x\n;\n").unwrap_err();
    assert_eq!(
            err,
            "error: `&Point>x` is not yet supported in a generic body, in `badfield` (line 2)\n  monomorphize this word (or write a concrete wrapper) to use `&Point>x` today"
        );
}

#[test]
fn poly_reference_word_rejects_add_in_place_in_a_generic_body() {
    // R-B4/R-B6: `+!` is permanently out of scope, unlike `!` (Phase 3).
    let err = check_src(": badaddstore ( 'T -- 'T )\n  +!\n;\n").unwrap_err();
    assert_eq!(
            err,
            "error: `+!` is not yet supported in a generic body, in `badaddstore` (line 2)\n  monomorphize this word (or write a concrete wrapper) to use `+!` today"
        );
}

#[test]
fn array_index_out_of_bounds_error_names_the_new_spelling() {
    // R9/R8: the array index out-of-bounds diagnostic names the new
    // `array[T N]` spelling in its effect annotation, per CLAUDE.md's
    // "diagnostics are behaviour".
    let err = check_src(": oob ( array[i64 4] -- i64 )\n  | a |\n  &a 9 &> @\n;\n").unwrap_err();
    assert!(err.contains("array index out of range"), "{err}");
    assert!(
        err.contains("array[i64 4]"),
        "error should name the new array spelling: {err}"
    );
}

#[test]
fn poly_reference_word_rejects_out_of_range_literal_index() {
    // R-B3: the literal `9` is statically bounds-checked against the
    // array's known length 4, mirroring the monomorphic `check_array_index`.
    let err =
        check_src(": oob['T: Copy] ( array['T 4] -- 'T )\n  | a |\n  &a 9 &> @\n;\n").unwrap_err();
    assert!(err.contains("array index out of range"), "{err}");
    assert!(err.contains("index 9"), "{err}");
    assert!(err.contains("length 4"), "{err}");
}

// -- Phase 3 (R-B3..R-B5): the mutable path and exclusivity -----------

#[test]
fn setat_writes_an_element_through_a_poly_mutable_borrow() {
    // R-B8's write witness, at the checker: `&!a` borrows mutably, `&!>`
    // takes a mutable element reference, `!` stores the `Copy` value
    // through it, and the array is returned afterwards -- the borrow is
    // dead by then, so naming `a` again is not a second name for
    // borrowed storage.
    check_src(
        ": setat['T: Copy] ( array['T 4] 'T -- array['T 4] ) | a v | &!a 2 &!> v ! a ;\n\
             : main ( -- ) 0 4 fill 99 setat drop ;\n",
    )
    .expect("a mutable prefix borrow, element ref, and store should check");
}

#[test]
fn poly_reference_word_rejects_two_live_mutable_borrows() {
    // E6/R-B5/OQ1: the hazard the poly body must catch itself, since a
    // plain generic word is checked once and never re-checked at its
    // instantiations. Rejected *at the second borrow site* (line 3, col
    // 7), naming the first (line 3, col 3).
    let err = check_src(
        ": twomut['T: Copy] ( array['T 4] -- array['T 4] )\n  | a |\n  &!a &!a drop drop a\n;\n",
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: `&!a` conflicts with a live borrow of `a` in `twomut` (line 3, col 7)\n  the mutable borrow taken at line 3, col 3 is still live\n  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
}

#[test]
fn poly_reference_word_rejects_a_shared_borrow_beside_a_live_mutable_one() {
    // E6, the other symmetric direction: a new *shared* borrow conflicts
    // with a live mutable one (never with another shared one).
    let err = check_src(
        ": mixed['T: Copy] ( array['T 4] -- array['T 4] )\n  | a |\n  &!a &a drop drop a\n;\n",
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: `&a` conflicts with a live borrow of `a` in `mixed` (line 3, col 7)\n  the mutable borrow taken at line 3, col 3 is still live\n  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
}

#[test]
fn poly_reference_word_accepts_two_live_shared_borrows() {
    // The positive control for the two rejections above: with no mutable
    // borrow in play there is nothing for exclusivity to protect, so two
    // live `&a` are fine. Without this, a rule that rejected *every*
    // second borrow would pass both negatives.
    check_src(": twoshared['T: Copy] ( array['T 4] -- array['T 4] ) | a | &a &a drop drop a ;\n")
        .expect("two shared borrows of one place do not conflict");
}

#[test]
fn poly_borrow_liveness_releases_a_borrow_once_its_reference_is_consumed() {
    // R-B5: the liveness approximation is not "live until the word ends"
    // -- `!` consumes the element reference, leaving no reference value
    // anywhere, so the first borrow is provably dead and the second
    // write is accepted. A word that can write only one element would be
    // a much weaker capability than the slice claims.
    check_src(
            ": settwo['T: Copy] ( array['T 4] 'T -- array['T 4] )\n  | a v |\n  &!a 0 &!> v !\n  &!a 1 &!> v !\n  a\n;\n",
        )
        .expect("a borrow whose reference is consumed is dead");
}

#[test]
fn poly_borrow_liveness_sees_a_reference_parked_in_a_local() {
    // R-B5: `prune_dead_borrows` scans the locals as well as the stack.
    // Binding the first `&!a` to `r` empties the stack while the
    // reference is still perfectly usable, so a stack-only scan would
    // call the borrow dead and admit a genuine second mutable borrow of
    // `a` -- two live `&!` to one place, the exact hazard R-B5 exists to
    // stop. Every other liveness case parks its reference on the stack.
    let err = check_src(
            ": hidden['T: Copy] ( array['T 4] 'T -- array['T 4] )\n  | a v |\n  &!a | r |\n  &!a 0 &!> v !\n  r 1 &!> v !\n  a\n;\n",
        )
        .unwrap_err();
    assert_eq!(
            err,
            "error: `&!a` conflicts with a live borrow of `a` in `hidden` (line 4, col 3)\n  the mutable borrow taken at line 3, col 3 is still live\n  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
}

#[test]
fn poly_call_term_accepts_naming_a_local_beside_a_live_shared_borrow() {
    // The positive control for the two naming-side rejections above, and
    // the mirror of `poly_reference_word_accepts_two_live_shared_borrows`
    // at the other site: naming a `Copy` aggregate neither moves it nor
    // aliases anything a *shared* borrow could mutate, so only a live
    // *mutable* borrow conflicts here. Without this, a naming check that
    // ignored the direction bit would pass both negatives.
    check_src(
            ": sharedname['T: Copy] ( array['T 4] -- array['T 4] 'T )\n  | a |\n  &a 0 &> @ | e |\n  &a a swap drop\n  e\n;\n",
        )
        .expect("a shared borrow does not stop a non-consuming name of its place");
}

#[test]
fn poly_borrow_liveness_is_coarse_across_places() {
    // R-B5's permitted conservatism, pinned as intentional rather than
    // left as an accidental divergence: `prune_dead_borrows` releases
    // *all* recorded borrows or none, so an unrelated live `&b` keeps
    // `a`'s already-consumed borrow recorded and the second `&!a` is
    // refused. The monomorphic checker accepts the same shape (its
    // `live_deriv` is per place), so this is an over-rejection, never a
    // missed hazard -- and it is legible as such from the note.
    let err = check_src(
            ": coarse['T: Copy] ( array['T 4] array['T 4] 'T -- array['T 4] array['T 4] )\n  | a b v |\n  &b\n  &!a 0 &!> v !\n  &!a 1 &!> v !\n  drop a b\n;\n",
        )
        .unwrap_err();
    assert!(
        err.starts_with(
            "error: `&!a` conflicts with a live borrow of `a` in `coarse` (line 5, col 3)"
        ),
        "{err}"
    );
    assert!(err.contains("conservatively treated as live"), "{err}");
}

#[test]
fn poly_call_term_rejects_consuming_a_borrowed_local() {
    // R-B5, the naming side: reading a linear local moves it out, and a
    // reference derived from it would be left aimed at storage its owner
    // gave away. Checking only at the borrow catches `a ... &!a` and
    // misses this, the same hazard with the terms swapped.
    let err = check_src(": consume ( array['T 4] -- array['T 4] )\n  | a |\n  &a a swap drop\n;\n")
        .unwrap_err();
    assert_eq!(
            err,
            "error: cannot consume the borrowed local `a` of type `array['T 4]` in `consume` (line 3, col 6)\n  the shared borrow taken at line 3, col 3 is still live\n  a place stays borrowed until every reference derived from it is consumed\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
}

#[test]
fn poly_call_term_rejects_naming_a_mutably_borrowed_local() {
    // R-B5, the naming side for a `Copy` aggregate: the read does not
    // consume it, but the name still denotes the storage the live `&!`
    // mutates.
    let err = check_src(
        ": alias['T: Copy] ( array['T 4] -- array['T 4] 'T )\n  | a |\n  &!a a swap 0 &!> @\n;\n",
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: cannot name `a` in `alias` (line 3, col 7): a mutable borrow of it is still live (line 3, col 3)\n  naming an aggregate does not copy it, so this name would denote the storage that borrow mutates\n  finish with the borrow first, or `dup` for an independent copy\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
}

#[test]
fn poly_body_rejects_dup_of_a_mutable_borrow_and_accepts_dup_of_a_shared_one() {
    // E1/D3/R-A5, now reachable end to end (Phase 1 could only reach the
    // gate directly, since no body could produce a `&!`): duplicating an
    // exclusive borrow would let two names mutate through it. The shared
    // half is the positive control -- a `&x` *is* `Copy`, so a rule that
    // rejected every `dup` of a reference would pass the negative alone.
    let err = check_src(
        ": dupmut['T: Copy] ( array['T 4] 'T -- array['T 4] ) | a v | &!a dup 0 &!> v ! drop a ;\n",
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: cannot `dup` a mutable reference in `dupmut` (line 1)\n  `&!array['T 4]` is not `Copy`: duplicating it would let two names observe or mutate through one exclusive borrow"
        );
    check_src(
            ": dupshared['T: Copy] ( array['T 4] -- array['T 4] 'T ) | a | &a dup drop 0 &> @ | e | a e ;\n",
        )
        .expect("a shared reference is `Copy` and may be duplicated");
}

#[test]
fn poly_body_store_rejects_a_shared_receiver() {
    // R-B4: `!` is `( &!T T -- )`. A shared receiver is a mutability
    // mismatch rendered off the receiver's own referent, the same shape
    // `&>` uses for the mirror-image mismatch.
    let err = check_src(
        ": rdstore['T: Copy] ( array['T 4] 'T -- array['T 4] ) | a v | &a 0 &> v ! a ;\n",
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: type mismatch in `rdstore` (line 1)\n  `!` expected `&!'T`, found `&'T`\n  note: declared ( -- )"
        );
}

#[test]
fn poly_body_store_rejects_a_value_of_another_type() {
    // R-B4: the stored value must unify with the referent -- an `i64`
    // literal is not a `'T`, at any instantiation but one.
    let err = check_src(
        ": wrongval['T: Copy] ( array['T 4] 'T -- array['T 4] ) | a v | &!a 0 &!> 5 ! v drop a ;\n",
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: type mismatch in `wrongval` (line 1)\n  `!` expected `'T`, found `i64`\n  note: declared ( -- )"
        );
}

#[test]
fn poly_body_store_rejects_a_non_copy_referent() {
    // R-B4: storing overwrites the old value, so a linear referent would
    // lose its drop obligation silently. Same X7 gate `@` already uses.
    let err = check_src(": linstore ( array['T 4] 'T -- array['T 4] ) | a v | &!a 0 &!> v ! a ;\n")
        .unwrap_err();
    assert_eq!(
            err,
            "error: cannot `!` the type variable `'T` in `linstore` (line 1)\n  `'T` has no `Copy` bound, and a linear value cannot be duplicated; declare `'T: Copy` if every instantiation is `Copy`"
        );
}

#[test]
fn poly_body_at_rejects_a_non_copy_referent() {
    // Phase 2 review: `@`'s `poly_copy_gate` call was reachable but
    // untested -- deleting it broke no test. A bare `'T` (no `Copy`
    // bound) fetched through a reference must still be rejected, the
    // same X7 reason `dup`/`over` already cover for a bare variable.
    let err = check_src(
        ": g ( array['T 4] -- 'T ) | a | &a 0 &> @ ;\n: main ( -- ) 10 4 fill g drop ;\n",
    )
    .unwrap_err();
    assert_eq!(
            err,
            "error: cannot `@` the type variable `'T` in `g` (line 1)\n  `'T` has no `Copy` bound, and a linear value cannot be duplicated; declare `'T: Copy` if every instantiation is `Copy`"
        );
}

/// P7 slice 2 review: `poly_reference_word`'s local-only lookup left a
/// generic word unable to borrow a module static at all, though R1 has no
/// monomorphic-only carve-out. `bump` never names `COUNT` as a local, so
/// this only type-checks if the static fallback fires.
#[test]
fn poly_body_can_borrow_a_module_static() {
    check_src(
        "static: COUNT i64 = 0 ;\n\
             : bump ['T: Copy] ( 'T -- 'T ) | v | &!COUNT @ 1 add &!COUNT swap ! v ;\n\
             : main ( -- ) 5 bump drop ;",
    )
    .unwrap();
}

/// The exclusivity scan applies to a poly-body static borrow exactly as
/// it does to a local's: two simultaneously live `&!COUNT` conflict.
#[test]
fn poly_body_two_live_mutable_static_borrows_conflict() {
    let err = check_src(
        "static: COUNT i64 = 0 ;\n\
             : bump ['T: Copy] ( 'T -- 'T ) &!COUNT &!COUNT drop drop ;\n\
             : main ( -- ) 5 bump drop ;",
    )
    .unwrap_err();
    assert!(
        err.contains("`&!COUNT` conflicts with a live borrow of `COUNT`"),
        "unexpected message: {err}"
    );
}

#[test]
fn poly_reference_word_rejects_shared_accessor_on_a_mutable_receiver() {
    // Phase 2 review: the `recv_mut != mutable` guard in `&>`'s arm was
    // reachable (a declared `&![...]` input) but untested -- deleting it
    // broke no test. `&>` on a mutable reference must still be rejected
    // rather than silently reading through it, and it names both sides
    // the way the monomorphic twin does (`&>` expected `&array[i64 4]`, found
    // `&!array[i64 4]`) rather than the operand-family text, which reads as
    // if `&>` never accepts a reference at all.
    let err = check_src(": rd['T: Copy] ( &!array['T 4] -- 'T )\n  0 &> @\n;\n").unwrap_err();
    assert_eq!(
            err,
            "error: type mismatch in `rd` (line 2)\n  `&>` expected `&array['T 4]`, found `&!array['T 4]`\n  note: declared ( -- )"
        );
}

#[test]
fn check_poly_array_index_bounds_checks_a_literal_and_requires_conversion_otherwise() {
    // R-B3, direct unit coverage of the helper mutation testing would
    // otherwise miss: a literal within range passes, one out of range
    // rejects, and a computed (non-literal) `i64` needs the explicit
    // `>usize` conversion the monomorphic checker also requires.
    let sig = ref_sig();
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    let span = Span::default();
    check_poly_array_index(
        &PolyType::Concrete(Type::I64),
        Some(2),
        Some(4),
        &ctx,
        span,
        "&>",
        &sig,
    )
    .expect("an in-range literal should pass");
    check_poly_array_index(
        &PolyType::Concrete(Type::I64),
        Some(9),
        Some(4),
        &ctx,
        span,
        "&>",
        &sig,
    )
    .expect_err("an out-of-range literal should reject");
    check_poly_array_index(
        &PolyType::Concrete(Type::I64),
        None,
        Some(4),
        &ctx,
        span,
        "&>",
        &sig,
    )
    .expect_err("a computed i64 needs the explicit >usize conversion");
    check_poly_array_index(
        &PolyType::Concrete(Type::Usize),
        None,
        Some(4),
        &ctx,
        span,
        "&>",
        &sig,
    )
    .expect("an already-usize index needs no literal at all");
    // P7.S6c (R2.4): an unknown length (`count = None`, the
    // generic-length deferral case) admits a `usize` index the same way
    // a known length does, and still requires the explicit `>usize`
    // conversion for a computed `i64` index.
    check_poly_array_index(
        &PolyType::Concrete(Type::Usize),
        None,
        None,
        &ctx,
        span,
        "&>",
        &sig,
    )
    .expect("a usize index needs no known length either");
    check_poly_array_index(
        &PolyType::Concrete(Type::I64),
        None,
        None,
        &ctx,
        span,
        "&>",
        &sig,
    )
    .expect_err("a computed i64 needs >usize even against an unknown length");
}

/// P7 slice 1 (R1): a field projection inside a generic body. `&f` carries
/// no `>`, so the pre-slice accessor guard (which tested `rest.contains('>')`)
/// no longer sees it, and without the receiver check the site falls through
/// to the local/static arm and reports "`x` is not a local" -- a wrong
/// diagnostic for a construct that is rejected for a different reason.
#[test]
fn projection_on_generic_receiver_body_is_error() {
    let err = check_src(
        "type: Point x i64 y i64 ;\n\
             : peek ( 'T -- 'T ) 3 4 Point &x @ drop drop ;\n\
             : main ( -- ) 7 peek drop ;",
    )
    .unwrap_err();
    assert!(
        err.contains("`&x` is not yet supported in a generic body"),
        "unexpected message: {err}"
    );
    assert!(
        !err.contains("is not a local"),
        "the local/static arm must not claim this site: {err}"
    );
}

/// P7 slice 3d (C1/R1): `call` on a body-local literal splices its body
/// in place against the live poly stack -- the poly analogue of
/// `check_terms_relaxed`'s own literal `call` splice.
#[test]
fn poly_call_on_literal_splices_body_in_place_ok() {
    check_src(
        ": bump ['T: Copy] ( 'T -- 'T 'T ) | x | [ x x ] call ;\n\
             : main ( -- ) 5 bump drop drop ;\n",
    )
    .expect("a literal's body should splice in place against the live stack");
}

/// P7 slice 3d (C1/R1): the splice is flavour-neutral, matching the
/// concrete path (`call` is not a materialization boundary, so `~` decides
/// nothing there either). This is the shape `tests/phase7_slice3b.rs`'s
/// deferred-combinator test used to carry before R1 narrowed that guard
/// off `call`, so without this nothing pins the `~[ ]` half.
#[test]
fn poly_call_on_inline_literal_splices_body_in_place_ok() {
    check_src(
        ": bump ['T: Copy] ( 'T -- 'T 'T ) | x | ~[ x x ] call ;\n\
             : main ( -- ) 5 bump drop drop ;\n",
    )
    .expect("an inline literal's body should splice in place too");
}

/// P7 slice 3d (C1/R1): `call` on a **non-literal** quotation operand
/// -- a declared parameter whose effect still carries a free `'T`, so it
/// stays `PolyType::Quotation` rather than folding to `PolyType::Concrete`
/// -- is a located outcome, never a panic and never `unknown word`.
/// P7.S3l flips this from a rejection: R1's new arm now dispatches this
/// shape (both sides of the declared effect carry `'T`) to
/// `poly_call_abstract_quotation_param`, which consumes/produces it
/// cleanly, so the body no longer rejects it at all.
#[test]
fn poly_call_on_non_literal_quotation_operand_is_accepted() {
    check_src(": caller ( 'T [ 'T -- 'T ] -- 'T ) call ;\n")
        .expect("a non-literal quotation operand at `call` is now accepted");
}

/// P7 slice 3d (C1/R1): `call` on an *empty* stack reports the ordinary
/// arity underflow, not `unknown word`. R1's arm owns every `call` in a
/// poly body now, so it owns this report too: before the arm existed the
/// name fell through to the env lookup, which has no `call` candidate.
#[test]
fn poly_call_on_empty_stack_is_underflow_error() {
    let err = check_src(
        ": uf ['T: Copy] ( 'T -- 'T )\n\
               | x | call x\n\
             ;\n\
             : main ( -- ) 1 uf drop ;\n",
    )
    .expect_err("`call` with nothing on the stack should underflow");
    assert!(
        err.contains("`call` needs 1 values, but the stack holds 0"),
        "{err}"
    );
    assert!(!err.contains("unknown word"), "{err}");
}

/// P7 slice 3d (C1/R1, L1): `call` on a plain non-quotation operand (a
/// concrete `i64`, not a quotation of any kind) falls into the same
/// `poly_op_on_variable_error` else-arm as the non-literal-quotation
/// case above -- `PolyType::Concrete` renders `` `i64` ``, never
/// `unknown word`. Untested behavioural change flagged in the Phase 1
/// review: before R1's arm existed, `call` fell through to the ordinary
/// env lookup and any operand type produced `unknown word`.
#[test]
fn poly_call_on_non_quotation_operand_is_located_error() {
    let err = check_src(
        ": caller ( 'T i64 -- 'T i64 )\n\
               call\n\
             ;\n\
             : main ( -- ) 1 2 caller drop drop ;\n",
    )
    .expect_err("a non-quotation operand at `call` should be rejected");
    assert_eq!(
        err,
        "error: `call` is not permitted on `i64` in `caller` (line 2)"
    );
    assert!(!err.contains("unknown word"), "{err}");
}

/// P7 slice 3d (C1/R1): the splice's own teardown, the poly analogue of
/// `Scope::leave` -- a linear local bound *inside* the spliced literal and
/// never consumed there leaks past `call` unless this is rejected before
/// the retain, exactly as an eliminator arm's own unconsumed binding does.
/// Reuses `poly_arm_local_not_consumed_error` rather than a fresh
/// message, so the report names `call` as the binding site.
#[test]
fn poly_call_on_literal_leaked_local_is_error() {
    let err = check_src(&format!(
        "{SPY}: bad ['T: Copy] ( 'T Spy -- 'T )\n\
               [ | s | ] call\n\
             ;\n\
             : main ( -- ) 7 Spy 1 swap bad drop ;\n"
    ))
    .expect_err("a local bound inside the splice and never consumed should be rejected");
    assert!(
        err.contains("the local `s` of type `Spy`, bound in an arm of `call` in `bad`")
            && err.contains("is never consumed"),
        "{err}"
    );
}

/// P7 slice 3d (R1): the splice's teardown also retains `scope.locals`/
/// `scope.moves` down to the pre-splice snapshot, the poly analogue of
/// `Scope::leave`'s own truncation -- without it, a local bound and
/// consumed *inside* the splice would still be registered in the
/// enclosing scope afterward, so a second reference to that name would
/// resolve as a stale local instead of the ordinary-word lookup it should
/// fall through to.
#[test]
fn poly_call_on_literal_retains_locals_past_splice() {
    let err = check_src(
        ": leaks ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | 3 [ | y | ] call x y\n\
             ;\n\
             : main ( -- ) 7 leaks drop drop ;\n",
    )
    .expect_err("`y` must not remain a resolvable local past the splice's own scope");
    assert!(err.contains("unknown word `y`"), "{err}");
}

/// P7 slice 3d (C2/R2): a body-local literal passed to a concrete `env`
/// word whose declared parameter is a ground `Type::Quotation` grounds
/// against that effect and the ordinary call proceeds.
#[test]
fn poly_quotlit_grounds_against_concrete_quotation_param_ok() {
    check_src(
            ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x [ 1 add ] 2 run1 ;\n : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect("a literal grounding against a concrete quotation parameter should be accepted");
}

/// P7 slice 3d (C2/R2): a `~[ ]` parameter can never reach the grounding
/// arm at all, so this does *not* pin the arm's own `Type::Quotation`
/// (not `Type::InlineQuotation`) exclusion -- R6's declaration gate
/// (`word_entry.rs`) rejects `run1`'s own declaration first, since a
/// non-inline word may not declare a `~[ ]` parameter, before any call
/// site is even checked. Confirmed even a *legal* `inline run1` never
/// reaches `chosen`: an inline word is spliced, not dispatched through
/// `env` by name, so the call instead falls through to `unknown word
/// run1__m0`. Both are pinned here so this rejection is never
/// mistaken for evidence of the grounding arm's own exclusion.
#[test]
fn poly_quotlit_against_declared_inline_quotation_param_rejects_at_declaration() {
    let err = check_src(
            ": run1 ( ~[ i64 -- i64 ] i64 -- i64 ) swap call ;\n : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x [ 1 add ] 2 run1 ;\n : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err("a non-inline word may not declare a `~[ ]` parameter");
    assert!(
        err.contains("declares an inline-quotation parameter") && err.contains("not `inline`"),
        "{err}"
    );
}

#[test]
fn poly_quotlit_against_legal_inline_quotation_param_rejects_at_the_cross_call() {
    // Pre-existing/unrelated to the slice that added this: any `~[ ]`-
    // bearing signature routes to the poly parser regardless of whether
    // it declares a type variable, so `run1` here lands in `poly_env`
    // despite carrying no `'T`.
    //
    // P7.S3k: `poly_call_term` *does* read `poly_env` now, so this shape
    // reaches the generic-callee arm rather than falling through it. It
    // is still a rejection, for a narrower and now accurate reason: a
    // quotation parameter has no runtime representation to pass across a
    // real call, so it is one of the declared shapes a symbolic mapping
    // cannot carry. Asserted on that reason and not just on the shared
    // first line, which both the old whole-feature narrowing and this
    // gate would satisfy.
    let err = check_src(
            ": run1 inline ( ~[ i64 -- i64 ] i64 -- i64 ) swap call ;\n : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x [ 1 add ] 2 run1 ;\n : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err("a quotation cannot be passed across a polymorphic call");
    assert!(
        err.contains("cannot call the polymorphic word `run1`")
            && err.contains("passing a quotation to a polymorphic word"),
        "{err}"
    );
}

/// P7 slice 3d (R2): the load-bearing shape for the operand-window
/// carve-out -- a non-builtin name's window is exactly one slot (its
/// top), so the carve-out only ever matters when the literal itself is
/// that top slot (the conventional quotation-last API shape). Every
/// other test in this file parks the literal *underneath* the window
/// (`x [ .. ] 2 run1`), where the window guard never even inspects it;
/// forcing the carve-out to always reject fails none of those, only
/// this one.
#[test]
fn poly_quotlit_grounds_when_it_is_the_top_of_window_operand_ok() {
    check_src(
            ": run0 ( i64 [ i64 -- i64 ] -- i64 ) call ;\n : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x 2 [ 1 add ] run0 ;\n : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect("a top-of-window literal must ground against a concrete quotation parameter");
}

/// Review fix (Bug 1): binding a quotation literal to a local and
/// reading it back loses its `PolyQuotRef` identity -- `| names |`
/// only records a bound slot's `PolyType`, never its `quot` (`poly.rs`'s
/// own local-binding loop), so a local read pushes a fresh
/// `PolySlot::new(pt)` with `quot: None` (the local-read arm, same
/// file). A re-read `QuotLit` slot reaching the grounding arm is
/// therefore not a value this slice can ground; it must be the located
/// rejection the operand-window guard already renders for an
/// ungroundable `QuotLit`, never the panic the unguarded `.expect(...)`
/// used to produce.
#[test]
fn poly_quotlit_bound_and_reread_is_located_error_not_panic() {
    let err = check_src(
        ": run0 ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 )\n\
               | x |\n\
               [ 1 add ] | q |\n\
               x 2 q run0\n\
             ;\n\
             : main ( -- ) 5 apply drop drop ;\n",
    )
    .expect_err(
        "a re-read quotation-literal marker must not reach the `.expect` it used to panic on",
    );
    assert_eq!(
        err, "error: `run0` is not permitted on a quotation literal in `apply` (line 5)",
        "{err}"
    );
}

/// Review fix (Bug 2): `poly_ground_quotation_literal` ported no flavour
/// check, so an inline `~[ ]` literal at a concrete word's ordinary
/// `Type::Quotation` parameter silently grounded and compiled -- the
/// mono twin (`check_literal_against_declared_effect`,
/// `literal_is_inline != is_inline`) rejects the identical shape with
/// `inline_literal_at_ordinary_param_error`.
#[test]
fn poly_quotlit_inline_literal_at_ordinary_param_is_error() {
    let err = check_src(
        ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x ~[ 1 add ] 2 run1 ;\n\
             : main ( -- ) 5 apply drop drop ;\n",
    )
    .expect_err("an inline literal at an ordinary quotation parameter must be rejected");
    assert!(
        err.contains("this quotation is inline `~[ ... ]`")
            && err.contains("`run1` expects `[ i64 -- i64 ]`"),
        "{err}"
    );
}

/// Review fix (Bug 3): `poly_ground_quotation_literal` never reconciled
/// an annotated literal against the declared parameter effect, so a
/// literal annotated with a disagreeing effect silently grounded and
/// compiled -- the mono twin runs `reconcile_annotation_with_parameter`
/// immediately after the flavour check and rejects the identical shape.
#[test]
fn poly_quotlit_disagreeing_annotation_is_error() {
    let err = check_src(
        ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x [ ( Bool -- Bool ) dup drop ] 2 run1 ;\n\
             : main ( -- ) 5 apply drop drop ;\n",
    )
    .expect_err("an annotation disagreeing with the declared parameter effect must be rejected");
    assert!(
        err.contains("annotated") && err.contains("but `run1` declares it"),
        "{err}"
    );
}

/// P7 slice 3d (C2): `poly_ground_quotation_literal`'s own teardown --
/// the poly analogue of `Scope::leave` -- rejects a non-`Copy` local the
/// grounded literal binds and never consumes, exactly as R1's splice
/// teardown does for `call`.
#[test]
fn poly_ground_quotation_literal_leaked_local_is_error() {
    let err = check_src(&format!(
        "{SPY}: run1 ( [ Spy -- i64 ] Spy -- i64 ) swap call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | x [ | s | 1 ] 1 Spy run1\n\
             ;\n\
             : main ( -- ) 5 apply drop drop ;\n"
    ))
    .expect_err("a local bound in the grounded literal and never consumed should be rejected");
    assert!(
        err.contains("the local `s` of type `Spy`, bound in an arm of `run1` in `apply`")
            && err.contains("is never consumed"),
        "{err}"
    );
}

/// P7 slice 3d (C2, R12): the poly twin of the concrete argument site's
/// D3 capture rule. The callee materializes the literal and `call`s it
/// twice, so a linear enclosing local consumed inside it is freed twice;
/// before the port this compiled clean and died with `free(): double free
/// detected in tcache 2` at run time, while the monomorphic twin of the
/// same body rejected it.
#[test]
fn poly_ground_quotation_literal_consuming_enclosing_linear_local_is_error() {
    let err = check_src(
        ": twice ( [ i64 -- i64 ] i64 -- i64 ) swap | q | q call q call ;\n\
             : ap ['T: Copy] ( 'T ^i64 -- 'T i64 ) | x c | x [ c drop 1 add ] 2 twice ;\n\
             : main ( -- ) 5 7 ^ ap drop drop ;\n",
    )
    .expect_err("a grounded literal consuming an enclosing linear local must be rejected");
    assert!(
        err.contains("the quotation passed to `twice` consumes the enclosing local `c`")
            && err.contains("(D3)"),
        "{err}"
    );
}

/// P7 slice 3d (C2, R12): the companion permissive half -- R12 forbids
/// *consuming* an enclosing local, not reading a `Copy` one, so the
/// capture check must not reject the shape a grounded literal exists
/// for. Widening the check from the consumed locals to every enclosing
/// name fails here.
#[test]
fn poly_ground_quotation_literal_reading_enclosing_copy_local_ok() {
    check_src(
        ": twice ( [ i64 -- i64 ] i64 -- i64 ) swap | q | q call q call ;\n\
             : ap ['T: Copy] ( 'T i64 -- 'T i64 ) | x c | x [ c add ] 2 twice ;\n\
             : main ( -- ) 5 7 ap drop drop ;\n",
    )
    .expect("a grounded literal may read an enclosing `Copy` local by value");
}

/// P7 slice 3d (C2, R12): what the capture check's `MoveState::Live`
/// precondition buys. The rule is about a *transition* across the
/// literal, not about the post-state: `c` is already `Moved` when the
/// literal is grounded and the literal never mentions it, so there is no
/// capture. Matching the post-state alone rejects this legal program.
#[test]
fn poly_ground_quotation_literal_local_consumed_before_the_literal_ok() {
    check_src(
        ": twice ( [ i64 -- i64 ] i64 -- i64 ) swap | q | q call q call ;\n\
             : ap ['T: Copy] ( 'T ^i64 -- 'T i64 ) | x c | c drop x [ 1 add ] 2 twice ;\n\
             : main ( -- ) 5 7 ^ ap drop drop ;\n",
    )
    .expect("a local consumed before the literal is not captured by it");
}

/// P7 slice 3d (C2, R12): the other half of D3 -- a grounded literal
/// that leaves a borrow of an enclosing place on its exit row -- is
/// rejected, but only because a `PolyType::Ref` slot can satisfy no
/// declared concrete output. The concrete twin of this body reports the
/// D3 rule by name; this asserts only that the program is refused, so a
/// future change that lets the two reference representations unify fails
/// here instead of silently admitting the capture.
#[test]
fn poly_ground_quotation_literal_borrowing_enclosing_place_is_error() {
    let err = check_src(
        "type: Pair a i64 b i64 ;\n\
             : takes ( [ -- &Pair ] -- ) drop ;\n\
             : ap ['T: Copy] ( 'T Pair -- 'T ) | x p | x [ &p ] takes ;\n\
             : main ( -- ) 5 1 2 Pair ap drop ;\n",
    )
    .expect_err("a grounded literal may not leave a borrow of an enclosing place on its row");
    assert!(err.contains("`takes` expected `[ -- &Pair ]`"), "{err}");
}

/// P7 slice 3d (C2): the teardown also retains `scope.locals`/
/// `scope.moves` back down to the pre-grounding snapshot -- without it,
/// a `Copy` local bound inside the grounded literal would still be
/// registered afterward, so a second reference to that name would
/// resolve as a stale local rather than the ordinary lookup it should
/// fall through to.
#[test]
fn poly_ground_quotation_literal_retains_locals_after_grounding() {
    let err = check_src(
        ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
             : leaks ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | x [ | y | 3 ] 2 run1 y\n\
             ;\n\
             : main ( -- ) 7 leaks drop drop ;\n",
    )
    .expect_err("`y` must not remain a resolvable local past the grounded literal's own scope");
    assert!(err.contains("unknown word `y`"), "{err}");
}

/// P7 slice 3d (C2): the grounded literal's exit stack must match
/// `eff.outputs` pointwise, not merely in arity -- a literal that leaves
/// the declared output type but the wrong shape is still a type
/// mismatch, not a silent pass.
#[test]
fn poly_ground_quotation_literal_output_mismatch_is_error() {
    let err = check_src(
        ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | x [ True ] 2 run1\n\
             ;\n\
             : main ( -- ) 5 apply drop drop ;\n",
    )
    .expect_err("a literal whose grounded body leaves the wrong output shape must be rejected");
    assert!(
        err.contains("`run1` expected `[ i64 -- i64 ]`") && err.contains("found `i64 Bool`"),
        "{err}"
    );
}

/// P7 slice 3d (C2): a same-arity mismatch -- the grounded literal
/// leaves exactly as many outputs as `eff.outputs` declares, but the
/// wrong type at that position, so only the pointwise half of the
/// check (not the arity half the mismatch test above exercises) can
/// catch it. Without it this would compile a type-confused program
/// silently.
#[test]
fn poly_ground_quotation_literal_output_type_mismatch_same_arity_is_error() {
    let err = check_src(
        ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | x [ drop True ] 2 run1\n\
             ;\n\
             : main ( -- ) 5 apply drop drop ;\n",
    )
    .expect_err(
        "a same-arity output whose type differs from the declared effect must still be rejected",
    );
    assert!(
        err.contains("`run1` expected `[ i64 -- i64 ]`") && err.contains("found `Bool`"),
        "{err}"
    );
}

/// P7 slice 3d (R2): the retained combinator guard still rejects
/// `branch`/`if`/`times`/`tag` on a quotation, unaffected by the
/// operand-window carve-out this phase adds.
#[test]
fn poly_call_term_still_rejects_branch_on_quotation() {
    // `times` no longer belongs to this test: S3b-follow's real
    // `poly_row_combinator` dispatch handles it (and `if`) whenever the
    // combinator is actually registered, which this minimal `check_src`
    // harness (`parse_with_core`, no library source) never does for a
    // bare name -- so the old scenario now falls through to `unknown
    // word` rather than exercising this guard at all. `branch` is a
    // compiler-known primitive, never a `CombinatorEnv` entry, so it
    // still reaches this guard regardless of what the harness loads.
    let err = check_src(
            ": apply ['T: Copy] ( 'T -- 'T ) | x | True [ ] [ ] branch drop ;\n : main ( -- ) 5 apply drop ;\n",
        )
        .expect_err("`branch` on a quotation should stay rejected");
    assert!(
        err.contains("is not yet supported") && err.contains("name no follow-up slice yet"),
        "{err}"
    );
}

/// P7 slice 3d (R2): a literal passed to an *overloaded* concrete name
/// is out of this slice's scope (the completeness gap/scoping note) --
/// the pre-existing operand-window guard still catches it when the
/// literal is the sole (top-of-window) operand, never `unknown word`.
#[test]
fn poly_quotlit_to_overloaded_concrete_name_is_located_rejection() {
    let err = check_src(
            ": run2 ( [ i64 -- i64 ] -- i64 ) 1 swap call ;\n : run2 ( i64 -- i64 ) 1 add ;\n : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x [ 1 add ] run2 ;\n : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err("an overloaded concrete name must not ground a quotation literal");
    assert_eq!(
        err,
        "error: `run2` is not permitted on a quotation literal in `apply` (line 3)"
    );
}

// P7.S4 (R2): `match_impl_target` unit tests.

// P7.S4 Phase 2 (R3): `specificity` / `is_strictly_more_specific` unit
// tests.
//
// The position-vector tests exercise the pure partial-order algorithm
// directly, covering the shared-variable scenarios that need
// `GenericTypes` to reach through `specificity`'s `collect_positions`
// walk. The `specificity` tests below exercise the full
// pattern-against-`ty` collection path for `Array` patterns (which need
// only an `ArrayDecl`).

// Review fix (P7.S4 Phase 2): `specificity` used to flatten each pattern
// independently and reject any pair of different lengths as
// incomparable, so a bare `'T` (1 position) could never lose to a
// deeper structural pattern (2+ positions) even when the deeper one is
// strictly more specific. `collect_paired_positions` walks both
// patterns together instead, so a depth mismatch is classified
// explicitly rather than silently producing differently-sized vectors.

// P7.S4b Phase 3 (R5/R12): bound-set tiebreak unit tests.

// -----------------------------------------------------------------
// P7.S11 phase 1 (R1/R2/R2.1/R3/R6): standalone-combinator grounding.
// -----------------------------------------------------------------

/// The R1/R2 floor: a constructor-free unbounded combinator whose
/// declared output applies a generic header grounds standalone. Without
/// R1 threading `generics` in, `apply_subst`'s `Generic` arm hits the
/// `ctx.generics()` `None` arm and rejects it at the def site.
#[test]
fn standalone_combinator_grounds_a_generic_output_without_a_constructor() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : relay inline ( 'T ~[ 'T -- Result['T i64] ] -- Result['T i64] ) call ;\n";
    check_src(src).expect("a constructor-free combinator grounds its output standalone");
}

/// R3: a declared top-level generic *input* slot is not yet groundable,
/// unlike a declared output -- the standing variable-bearing-application
/// restriction the S12 fixture already pins.
#[test]
fn standalone_combinator_generic_input_slot_is_still_rejected() {
    let src = "type: Option['T] | None | Some 'T ;\n\
             : probe inline ( Option['T] ~[ -- i64 ] -- i64 ) drop drop 0 ;\n";
    let err = check_src(src).expect_err("a top-level generic input slot must still be rejected");
    assert!(
        err.contains("names the generic type `Option['T]`")
            && err.contains("cannot yet be instantiated at a variable-bearing application"),
        "expected the standing variable-bearing restriction, got: {err}"
    );
}

/// The R3 scope twin: a generic nested inside a quotation-input effect's
/// row (not itself the top-level slot) is not rejected and grounds
/// normally. Fails if R3's guard recurses into slots instead of testing
/// the top level. A *concrete* declared output (unlike the R1/R2 floor
/// test above, whose output also applies the generic header) isolates
/// "nested-in-input grounds" from "a declared output grounds": nothing
/// but the quotation-input row's generic could make this pass.
#[test]
fn standalone_generic_nested_in_a_quotation_input_is_not_rejected() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : hold inline ( 'T ~[ 'T -- Result['T i64] ] -- i64 ) call drop 0 ;\n";
    check_src(src)
        .expect("a generic nested inside a quotation-input row must not be rejected by R3");
}

/// R2's discard guard, and mutation 6's killer: the standalone check's own
/// scratch mint must never reach the live `GenericTypes` cell.
/// `lookup_enum` is the public route to that state (round-3 fix: the
/// private `enum_keys`/`enum_resolved` fields round 2 specified are
/// unreachable from here). Checked both before the module checks (so the
/// fixture's own lack of a parse-time sibling monomorph is not assumed)
/// and after (so a leak into the cell that `check_module` eventually
/// restores onto `module.generics` would be caught).
#[test]
fn standalone_stand_in_monomorph_does_not_enter_the_live_registry() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : relay inline ( 'T ~[ 'T -- Result['T i64] ] -- Result['T i64] ) call ;\n";
    let tokens = lex(src).unwrap();
    let fresh = crate::test_support::parse_with_core(&tokens).unwrap();
    let result_idx = fresh
        .generics
        .enums
        .iter()
        .position(|e| e.name == "Result")
        .expect("the Result header is registered before any check runs");
    let header_module = fresh.generics.enums[result_idx].module;
    let live_enum_len = fresh.enums.len();
    assert!(
        fresh
            .generics
            .lookup_enum(result_idx, header_module, &[Type::I64, Type::I64], &[])
            .is_none(),
        "no parse-time sibling monomorph exists in this fixture"
    );
    let (module, _) = checked_like_a_build(src).expect("relay checks standalone clean");
    assert!(
        module
            .generics
            .lookup_enum(result_idx, header_module, &[Type::I64, Type::I64], &[])
            .is_none(),
        "the standalone check's own scratch mint must not reach the live GenericTypes cell"
    );
    assert!(
        module.generics.inst_enums.is_empty(),
        "nothing survives the standalone check into the live batch"
    );
    assert_eq!(
        module.enums.len(),
        live_enum_len,
        "no flushed monomorph beyond whatever parsing itself already registered"
    );
}

/// R6's other half, missed by the unit test above: a cell/ref shape
/// grounded through the *body* (not the signature -- `-- ^Result['T i64]`
/// and `-- &Result['T i64]` don't parse, the standing poly borrow-sigil
/// gap) must also stay word-scoped. `check_poly_combinator_standalone`
/// runs the body through the ordinary `check_word` path over
/// `&mut local.cells`/`&mut local.refs`, so `^` minting a cell payload
/// of the combinator's own grounded monomorph must never touch
/// `module.owned_cells`.
#[test]
fn standalone_grounding_a_cell_through_the_body_leaves_the_live_cells_untouched() {
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : hold inline ( 'T ~[ 'T -- Result['T i64] ] -- i64 ) call ^ drop 0 ;\n";
    let tokens = lex(src).unwrap();
    let parsed = crate::test_support::parse_with_core(&tokens).unwrap();
    let live_cell_len = parsed.owned_cells.len();
    let (checked, _) =
        checked_like_a_build(src).expect("hold checks standalone and the module compiles");
    assert_eq!(
        checked.owned_cells.len(),
        live_cell_len,
        "check_poly_combinator_standalone must not intern the body's cell into the live registry"
    );
}

/// R6's remaining halves: `refs` and `slices` (R6.4). A borrow and a slice
/// taken over a body local whose element is the combinator's own grounded
/// monomorph must intern into the word-scoped `local.refs`/`local.slices`,
/// never into the live registries. It has to go through the body, like the
/// cell test above: `-- &Result['T i64]` is rejected at the header with
/// `unknown type 'T` (the standing poly borrow-sigil gap), and a slice type
/// has no signature spelling at all -- one exists only where the `slice`
/// word mints it. The
/// `slices` half is what R6.4 added beyond the reported finding: a leaked
/// slice of a scratch monomorph reaches `build_slices`, which indexes
/// `enums.layouts[id.index()]` off the *live* registry the monomorph never
/// entered. The `4 fill` also witnesses `arrays` through the body, where
/// the signature-side array test covers only the declared-output route.
#[test]
fn standalone_grounding_a_borrow_and_a_slice_through_the_body_leaves_the_live_registries_untouched()
{
    let src = "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : hold inline ( 'T ~[ 'T -- Result['T i64] ] -- i64 )\n\
               call 4 fill | a | &a slice drop a drop 0 ;\n";
    let tokens = lex(src).unwrap();
    let parsed = crate::test_support::parse_with_core(&tokens).unwrap();
    let live_ref_len = parsed.refs.len();
    let live_slice_len = parsed.slices.len();
    let live_array_len = parsed.arrays.len();
    let (checked, _) =
        checked_like_a_build(src).expect("hold checks standalone and the module compiles");
    assert_eq!(
        checked.refs.len(),
        live_ref_len,
        "check_poly_combinator_standalone must not intern the body's borrow into the live registry"
    );
    assert_eq!(
        checked.slices.len(),
        live_slice_len,
        "check_poly_combinator_standalone must not intern the body's slice into the live registry"
    );
    assert_eq!(
        checked.arrays.len(),
        live_array_len,
        "nor the body's array, on the route the declared-output array test does not cover"
    );
}

/// P7b.S3 Phase 2 fixtures: a Functor trait over two structurally
/// identical single-parameter enums, `Opt` implemented first and `Opt2`
/// second, in that source order.
fn functor_two_impls_src(extra: &str) -> String {
    format!(
            "type: Opt['T] | None | Some 'T ;\n\
             type: Opt2['T] | None2 | Some2 'T ;\n\
             trait: Functor['F: * -> *] :\n\
             map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             impl: Functor for Opt\n\
             : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Opt? ;\n\
             ;\n\
             impl: Functor for Opt2\n\
             : map swap ~[ ( Some2 ) Some2> swap call Some2 ] ~[ ( None2 ) drop drop None2 ] Opt2? ;\n\
             ;\n\
             {extra}\n\
             : main ( -- ) ;\n"
        )
}

/// P7b.S3 (S3-6): the no-impl fallback. An Arrow-kinded bound with no
/// `impl:` in the program has no constructor to stand in for, so the
/// standalone check is skipped without erroring (the word is checked at
/// its splice sites, of which an impl-less bound can have none).
#[test]
fn arrow_bound_without_impl_skips_standalone_without_error() {
    let res = check_src(
        "type: Opt['T] | None | Some 'T ;\n\
             trait: Functor['F: * -> *] :\n\
             map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             : apply inline['F: Functor 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) map ;\n\
             : main ( -- ) ;\n",
    );
    assert!(res.is_ok(), "skipped, not erred: {res:?}");
}

/// P7b.S3 (S3-6): the rescue fires ONLY for the failure classes the
/// stand-in's arbitrary constructor can cause. An arity error (an extra
/// `drop` past what the declared effect leaves) is impl-independent and
/// stays a hard error -- and the internal grounding tag never reaches
/// the rendered message.
///
/// Measured (Phase 2 review fix): the pre-S3-6 abstract poly walk
/// (`check_poly_body`, R7) runs on every combinator before
/// `check_poly_combinator_standalone` is ever invoked, and it already
/// rejects a symbolic stack-depth mismatch like this one on its own --
/// so this fixture never reaches the standalone check's concrete
/// `check_word` call, let alone its rescue match arms, and this test
/// cannot by itself discriminate "the rescue declined to fire" from
/// "the rescue was never reached." It is kept because it is still the
/// correct behavior to pin (a `bad` combinator must be rejected, tag or
/// no tag) and because no impl-independent failure category was found,
/// after measurement, that survives the abstract walk to reach the
/// standalone check's own body pass for an Arrow-bound member call --
/// every member mention with an App-headed slot grounds through the
/// caller's θ (`ground_member_sig_via_theta`, S3-7 -- the fence it
/// replaced is deleted), whose stand-in failures raise the *tagged*
/// `splice_member_ctor_image_error` and are rescued rather than staying
/// hard -- which is what `class_one_grounding_failure_is_skipped` and
/// `class_two_member_call_is_rescued_for_recheck_at_the_splice`
/// below measure directly.
/// P7b.S3 (S3-6): the no-leak invariant `STAND_IN_GROUNDING_TAG`'s doc
/// and `check.rs`'s `strip_diagnostic_tags` both assert. The row-7 HKT
/// fixture (non-inline member, `inline` caller) raises the *tagged*
/// `splice_member_ctor_image_error` at a real splice site, where it is a
/// hard error rather than a rescue -- so the tag is on the raise and the
/// only thing between it and the user is the `check_module` boundary.
#[test]
fn a_tagged_raise_reaches_check_module_output_untagged() {
    let src = functor_two_impls_src(
        ": twice inline['F: Functor 'T] ( 'F['T] [ 'T -- 'T ] -- 'F['T] ) | q | q map q map ;\n\
             : mk ( i64 -- Opt[i64] ) Some ;\n\
             : go ( -- ) 1 mk [ ] twice drop ;",
    );
    let err = check_src(&src).expect_err("the row-7 HKT splice is rejected");
    assert!(
        err.contains("grounded against the constructor image"),
        "the fixture reaches the tagged raise site: {err}"
    );
    assert!(
        !err.contains('\u{1}'),
        "no tag byte survives the `check_module` boundary: {err:?}"
    );
}

#[test]
fn standalone_arity_error_is_not_rescued() {
    let res = check_src(&functor_two_impls_src(
        ": bad inline['F: Functor 'T] ( 'F['T] -- 'F['T] ) drop drop ;",
    ));
    let err = res.expect_err("an impl-independent body error stays hard");
    assert!(
        !err.contains('\u{1}'),
        "no internal tag byte in a rendered diagnostic: {err:?}"
    );
    assert!(
        err.contains("bad"),
        "the error names the failing word: {err}"
    );
}

/// P7b.S3 (S3-6) Phase 2 measurement: class 1 ("the rescue, first
/// class" at `check_poly_combinator_standalone`'s `grounded` match) is
/// a blanket over the combinator's *own* declared-effect grounding, not
/// gated on a tag. Measured attempt to trigger it with a legally
/// parsed fixture: every route tried --
/// - a bare (unapplied) Arrow-bound variable in the combinator's own
///   signature is rejected at the shared kind check before `check_module`
///   ever reaches grounding ("a higher-kinded variable never appears
///   bare"), for a word signature and a trait member signature alike;
/// - every declared length variable is pre-seeded (`STANDALONE_LEN`)
///   before grounding starts, so an unbound-length failure cannot occur;
/// - `arrow_stand_in`'s own fit check (`want_ty`/`want_len`) only ever
///   selects a candidate impl whose constructor arity structurally
///   matches the bound's own kind, so the App arm's length-domain
///   defense (`poly_app_len_domain_unsupported_error`) cannot fire from
///   a stand-in this function picked itself.
///
/// No legally-parsed program was found, after measurement, that reaches
/// this blanket with a genuine grounding failure -- it is retained as
/// defense-in-depth, not because a witness exists. This test pins that
/// absence: a combinator whose own signature *is* App-headed over a
/// fitting stand-in still checks clean.
#[test]
fn class_one_grounding_failure_is_skipped() {
    let res = check_src(&functor_two_impls_src(
        ": apply inline['F: Functor 'T] ( 'F['T] -- 'F['T] ) ;",
    ));
    assert!(
        res.is_ok(),
        "a fitting stand-in's App-headed grounding of the combinator's own \
             effect does not fail: {res:?}"
    );
}

/// P7b.S3 (S3-1.e): two splices of one member body at two θ record two
/// *different* operative `EnumId`s under the same body span -- the
/// in-process half of golden P#4, which cannot see `Module` state.
#[test]
fn two_splices_at_two_thetas_record_two_different_enum_ids() {
    let (module, _) = checked_like_a_build(
        "type: P v i64 w i64 ;\n\
             type: Opt['T 'E] | None | Some 'T 'E ;\n\
             trait: Take['F: * -> * -> *] :\n\
             take inline ( 'F['T 'E] 'E -- 'E ) ;\n\
             ;\n\
             impl: Take for Opt\n\
             : take | o d | o ~[ ( Some ) Some> swap drop d drop ] ~[ ( None ) drop d ] Opt? ;\n\
             ;\n\
             : take2 inline['F: Take 'T 'E] ( 'F['T 'E] 'E -- 'E ) take ;\n\
             : mk1 ( i64 i64 -- Opt[i64 i64] ) Some ;\n\
             : mk2 ( P i64 -- Opt[P i64] ) Some ;\n\
             : main ( -- ) 1 2 mk1 7 take2 drop\n\
             1 2 P 4 mk2 9 take2 drop ;\n",
    )
    .expect("the two-theta fixture checks");
    assert!(
        !module.splice_enum_words.is_empty(),
        "the spliced member body's enum sites are recorded per (uid, span)"
    );
    let mut per_span: HashMap<Span, std::collections::HashSet<EnumId>> = HashMap::new();
    for (&(_, span), &id) in &module.splice_enum_words {
        per_span.entry(span).or_default().insert(id);
    }
    assert!(
        per_span.values().any(|ids| ids.len() >= 2),
        "one body span must resolve to two different EnumIds across the two theta: {:?}",
        module.splice_enum_words
    );
    // The redirect: a site recorded per-splice leaves no span-keyed
    // `builtin_overloads` entry for the same span.
    for &(_, span) in module.splice_enum_words.keys() {
        assert!(
            !module.builtin_overloads.contains_key(&span),
            "a spliced enum site is redirected, not double-recorded: {span:?}"
        );
    }
}

/// The row-3 shape every S3-1.c pin below cuts from: an `inline` member
/// on a generic impl target, reached through a bound on an `inline`
/// caller.
fn sized_box_splice_src() -> &'static str {
    "trait: Sized['S] :\n\
         size inline ( 'S -- i64 ) ;\n\
         ;\n\
         type: Box['T] v 'T ;\n\
         impl: Sized for Box['T]\n\
         : size drop 1 ;\n\
         ;\n\
         : usesize inline['S: Sized] ( 'S -- i64 ) size ;\n\
         : mkbox ( i64 -- Box[i64] ) Box ;\n\
         : main ( -- ) 3 mkbox usesize drop ;\n"
}

/// P7b.S3 (S3-1.c, deviation 2): the splice-caller hop is intercepted by
/// `inline_combinator` AND leaves a `splice_trait_calls[(uid, span)]`
/// record -- the ACTUAL shape, not the spec's original "gains no entry"
/// wording -- whose value is the member's SYNTH name (`WordDef::name`,
/// `size;Sized;…`), lowering's routing key into its own combinator
/// splice. Never the `overload_symbols` symbol, which `$$`-suffixes on a
/// name collision.
#[test]
fn member_call_at_a_splice_site_records_the_members_synth_name() {
    let (module, _) =
        checked_like_a_build(sized_box_splice_src()).expect("the row-3 fixture checks");
    let member = module
        .words
        .iter()
        .find(|w| w.is_trait_member && w.declares_inline)
        .expect("the impl member word is a module word");
    assert!(
        !module.splice_trait_calls.is_empty(),
        "the hop records the splice site for lowering"
    );
    assert!(
        module
            .splice_trait_calls
            .values()
            .all(|v| v == &member.name),
        "every record carries the member's synth name, got: {:?}",
        module.splice_trait_calls
    );
}

/// P7b.S3 (S3-1.c): the combinator entry is found by the member's synth
/// name, never by `word_symbols[idx]`. No source can spell the synth
/// name's `;`, so the collision is injected post-parse: a second word
/// sharing the member's name makes `overload_symbols` `$$`-suffix both,
/// and a symbol-keyed `poly.combinators` lookup would then miss (the
/// `collect_combinators`-invariant `expect` panics and this test fails).
#[test]
fn member_combinator_entry_is_found_by_synth_name_not_suffixed_symbol() {
    let tokens = lex(sized_box_splice_src()).unwrap();
    let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
    let member_idx = module
        .words
        .iter()
        .position(|w| w.is_trait_member && w.declares_inline)
        .expect("the impl member word is a module word");
    let clone_name = module.words[member_idx].name.clone();
    module
        .words
        .push(crate::test_support::bare_word(&clone_name, 0));
    let symbols = crate::ast::overload_symbols(&module.words);
    assert!(
        symbols[member_idx].contains("$$"),
        "the injected duplicate makes the member's symbol diverge from its name: {}",
        symbols[member_idx]
    );
    let (module, _) = {
        super::super::check_module(&mut module).expect("the collided fixture still checks");
        (module, ())
    };
    let picked: Vec<&String> = module.splice_trait_calls.values().collect();
    assert!(
        !picked.is_empty() && picked.iter().all(|v| !v.contains("$$")),
        "the record and the lookup key on the synth name, not the suffixed symbol: {picked:?}"
    );
}

/// P7b.S3 (S3-8): the splice path and `resolve_user_bound` pick the SAME
/// impl for a two-fitting-impl program. `Box['T]` is declared before the
/// concrete-pinned `Box[i64]`; a first-match `find` would take the
/// generic one at a `Box[i64]` operand, while `find_bound_impl`'s R3
/// most-specific selection takes the concrete pin -- the rule the
/// non-splice caller already follows (S2's p11c golden).
#[test]
fn splice_path_selects_the_most_specific_impl_not_the_first_match() {
    let (module, _) = checked_like_a_build(
        "trait: Sized['S] :\n\
             size inline ( 'S -- i64 ) ;\n\
             ;\n\
             type: Box['T] v 'T ;\n\
             impl: Sized for Box['T]\n\
             : size drop 1 ;\n\
             ;\n\
             impl: Sized for Box[i64]\n\
             : size drop 2 ;\n\
             ;\n\
             : usesize inline['S: Sized] ( 'S -- i64 ) size ;\n\
             : mkbox ( i64 -- Box[i64] ) Box ;\n\
             : main ( -- ) 3 mkbox usesize drop ;\n",
    )
    .expect("the fixture checks");
    let picked: Vec<&String> = module.splice_trait_calls.values().collect();
    assert!(
        !picked.is_empty(),
        "the splice-caller hop recorded the member symbol"
    );
    assert!(
        picked.iter().all(|s| s.contains("Box[i64]")),
        "the splice path picks the concrete pin, as resolve_user_bound does: {picked:?}"
    );
}

/// P7b.S3 (S3-8): a candidate whose `where` bound fails to discharge is
/// not selected on the splice path. `Box[P]`'s element is a struct, not
/// Ord, so the sole impl's `where 'T: Ord` fails and the splice site
/// reports the unsatisfied bound instead of dispatching through it (the
/// pre-S3-8 bare `find` had no R6 discharge at all).
#[test]
fn undischarged_where_bound_is_not_selected_on_the_splice_path() {
    let res = check_src(
        "trait: Sized['S] :\n\
             size inline ( 'S -- i64 ) ;\n\
             ;\n\
             type: P v i64 ;\n\
             type: Box['T] v 'T ;\n\
             impl: Sized for Box['T] where 'T: Ord\n\
             : size drop 1 ;\n\
             ;\n\
             : usesize inline['S: Sized] ( 'S -- i64 ) size ;\n\
             : mkbox ( -- Box[P] ) 5 P Box ;\n\
             : main ( -- ) mkbox usesize drop ;\n",
    );
    let err = res.expect_err("the undischarged candidate is not selected");
    assert!(
        err.contains("does not satisfy `Sized`"),
        "the error names the unsatisfied-bound shape, not just the trait name: {err}"
    );
}

/// P7b.S3 (S3-10): the poly pre-pass records a member combinator, so
/// `TraitResolveCtx::word_sig_of` hits for its synthesized name -- the
/// lookup that stopped erroring at the `word_sig_of(word_sym)` call m2
/// localized.
#[test]
fn word_sig_of_hits_for_a_recorded_member_combinator() {
    let (module, recorded) = checked_like_a_build(
        "trait: Sized['S] :\n\
             size inline ( 'S -- i64 ) ;\n\
             ;\n\
             type: Box['T] v 'T ;\n\
             impl: Sized for Box['T]\n\
             : size drop 1 ;\n\
             ;\n\
             : usesize inline['S: Sized] ( 'S -- i64 ) size ;\n\
             : mkbox ( i64 -- Box[i64] ) Box ;\n\
             : main ( -- ) 3 mkbox usesize drop ;\n",
    )
    .expect("the fixture checks");
    let member = module
        .words
        .iter()
        .find(|w| w.is_trait_member)
        .expect("the synthesized member word exists");
    let tr = TraitResolveCtx {
        traits: &module.traits,
        impls: &module.impls,
        word_symbols: &[],
        words: &module.words,
        recorded: &recorded,
        enum_sites_recorded: &[],
        cell_sites_recorded: &[],
    };
    assert!(
        tr.word_sig_of(&member.name).is_some(),
        "the pre-pass recorded `{}` so its grounded sig is findable",
        member.name
    );
}

/// P7b.S8c (D1/REQ-1): the per-site mint arm's fixtures. A generic impl
/// target (`Box['T]`) whose member row carries a free type variable of
/// its own (`'U`) -- the shape whose match-only mint reached lowering
/// with `'U` unbound and panicked at `subst_polytype`'s `expect`
/// (`src/ir/driver.rs:579`). `slot` is the member's first (free-local)
/// operand position as declared, `boxed` the payload `mkbox` builds.
fn odd_box_src(slot: &str, boxed: &str, operand: &str) -> String {
    format!(
        "type: Box['T] v 'T ;\n\
             : mkbox ( {boxed} -- Box[{boxed}] ) Box ;\n\
             trait: Odd['T] : odd ( 'U {slot} -- ) ; ;\n\
             impl: Odd for Box['T] : odd drop drop ; ;\n\
             : consume ['T: Odd] ( 'U {slot} -- ) odd ;\n\
             : main ( -- ) {operand} ;\n"
    )
}

/// P7b.S8c (REQ-1/REQ-6): the symbols the mint arm recorded for a
/// checked source's bound dispatches, filtered to `members` -- most are
/// `instantiation_symbol` applied to the recorded `(member word, theta)`
/// pair, so the string *is* theta rendered, which is what makes a byte
/// pin here a pin on the composed substitution and not merely on a name;
/// but `trait_calls` also carries bare symbols from the concrete/lifted-
/// mono arm, and comparing the module's entire sorted symbol list would
/// make every fixture hostage to any future prelude dispatch recorded
/// alongside it.
fn minted_dispatch_symbols(src: &str, members: &[&str]) -> Vec<String> {
    let module = checked_like_a_build_mangled(src).expect("the fixture checks");
    let mut symbols: Vec<String> = module
        .instantiations
        .values()
        .flat_map(|i| i.trait_calls.values().cloned())
        .filter(|s| {
            members
                .iter()
                .any(|m| s.starts_with(&format!("sooth_mono_{m}_")))
        })
        .collect();
    symbols.sort();
    symbols
}

/// P7b.S8c (REQ-1): the plain-slot cell -- the shape `Foldable::fold`
/// rides in production. Two variables enter theta by two different
/// routes: `t0` from `find_bound_impl`'s impl-target match against
/// `Box[i64]`, and `t1` (the member row's own `'U`) from unifying the
/// member word's sig against this site's re-grounded operand slot. The
/// mint arm recorded only the former before this slice, so `t1` reached
/// lowering unbound.
#[test]
fn bound_dispatch_binds_a_plain_member_slot_from_the_call_site() {
    assert_eq!(
        minted_dispatch_symbols(&odd_box_src("'T", "i64", "7 7 mkbox consume"), &["odd"]),
        vec!["sooth_mono_odd_Odd_0_Box__T0___m0__t0_i64_t1_i64"]
    );
}

/// P7b.S8c (REQ-1): the `&'T` cell (`plainslot.sth`, the S8-review
/// repro). The trait-var operand arrives behind a reference, so the
/// member word's sig unifies through `unify_poly_input`'s `Ref` arm
/// rather than its `Generic` arm -- the other of the two binding routes
/// into theta.
#[test]
fn bound_dispatch_binds_a_ref_member_slot_from_the_call_site() {
    assert_eq!(
        minted_dispatch_symbols(
            &odd_box_src("&'T", "i64", "7 mkbox | b | 7 &b consume"),
            &["odd"]
        ),
        vec!["sooth_mono_odd_Odd_0_Box__T0___m0__t0_i64_t1_i64"]
    );
}

/// P7b.S8c (REQ-6), the grounding-*correctness* witness. The two cells
/// above bind `t0` and `t1` to the same `i64`, so their symbol would
/// render identically even if per-site unification had wrongly sourced
/// the member local's binding from the impl-target match. Here the box
/// holds a `str` and the free local operand is an `i64`: only a theta
/// whose `t1` came from *this site's slot* renders `t0_str_t1_i64`.
#[test]
fn bound_dispatch_at_an_asymmetric_site_binds_the_local_from_the_slot() {
    assert_eq!(
        minted_dispatch_symbols(
            &odd_box_src("&'T", "str", "\"x\" mkbox | b | 7 &b consume"),
            &["odd"]
        ),
        vec!["sooth_mono_odd_Odd_0_Box__T0___m0__t0_str_t1_i64"]
    );
}

/// P7b.S8c (REQ-1): slice8b's shipped `Monoid`-over-`List` dispatch
/// surface, the one committed shape that rides this arm
/// (`tests/phase7b_slice8b.rs`'s
/// `monoid_for_list_combine_through_bound_grounds_and_is_stable`), with
/// `core::list`'s header inlined since the unit harness resolves no
/// `import:`.
fn monoid_list_src() -> &'static str {
    "type: List['T] | Nil | Cons 'T rest ^List['T] ;\n\
         trait: Monoid['T] :\n\
           empty ( -- 'T ) ;\n\
           : combine ( 'T 'T -- 'T ) ;\n\
         ;\n\
         impl: Monoid for List\n\
           : empty Nil ;\n\
           : combine\n\
             swap\n\
             ~[ ( Nil ) drop ]\n\
             ~[ ( Cons ) Cons> | v rest | rest ^> swap combine v swap ^ Cons ]\n\
             List? ;\n\
         ;\n\
         : mkempty ( -- List[i64] ) Nil ;\n\
         : merge['T: Monoid] ( 'T 'T -- 'T ) combine ;\n\
         : mkempty2['T: Monoid] ( -- 'T ) empty ;\n\
         : main ( -- )\n\
           3 mkempty ^ Cons 3 mkempty ^ Cons merge drop\n\
           mkempty2[List[i64]] drop ;\n"
}

/// P7b.S8c (REQ-1), the no-churn pins, both halves of `Monoid`:
///
/// * `combine ( 'T 'T -- 'T )` is **trait-var-only** -- its row carries no
///   variable beyond the dissolved trait header var, so per-site
///   unification rebinds `t0` to the type the impl-target match already
///   gave it and contributes nothing new. `theta == subst`.
/// * `empty ( -- 'T )` is **nullary**: `TraitObligation::slots` holds one
///   entry per declared member *input*, so there are zero slots and the
///   per-site loop does not run at all. Its header binding lives in the
///   match subst alone, which is why theta must start as a clone of it
///   and only ever be extended -- a fresh theta would render no `t0` and
///   lowering would fire the `expect` on the member's *output*.
///
/// Both symbols carry exactly one entry (`t0_i64`), and `subst` for
/// `match(List['T0], List[i64])` is exactly that one entry, so
/// `theta ⊇ subst` plus a one-entry render is `theta == subst`. They are
/// byte-identical to what the match-only arm minted before this slice.
#[test]
fn bound_dispatch_of_the_monoid_members_mints_the_match_subst_unchanged() {
    assert_eq!(
        minted_dispatch_symbols(monoid_list_src(), &["combine", "empty"]),
        vec![
            "sooth_mono_combine_Monoid_0_List__T0___m0__t0_i64",
            "sooth_mono_empty_Monoid_0_List__T0___m0__t0_i64",
        ]
    );
}

/// P7b.S8c (REQ-1), the QuotLit fence. `unify_member_operand`'s
/// `(Var(v), found) => bind` arm accepts a written quotation literal
/// against a *plain* declared member slot (unlike a declared `Quotation`
/// slot, which it rejects), so the obligation clones a
/// `PolyType::QuotLit` slot. Re-grounding that slot would drive straight
/// into `apply_subst`'s `unreachable!`, so the fence runs before any
/// re-grounding. Located at the dispatch site, non-panic.
#[test]
fn bound_dispatch_with_a_quotation_literal_member_slot_is_a_located_error() {
    let err = checked_like_a_build_mangled(
        "type: Box['T] v 'T ;\n\
             : mkbox ( i64 -- Box[i64] ) Box ;\n\
             trait: Odd['T] : odd ( 'U &'T -- ) ; ;\n\
             impl: Odd for Box['T] : odd drop drop ; ;\n\
             : consume ['T: Odd] ( &'T -- ) [ drop ] swap odd ;\n\
             : main ( -- ) 7 mkbox | b | &b consume ;\n",
    )
    .expect_err("a written quotation literal has no type to instantiate the member at");
    assert_eq!(
            err,
            "error: `odd` of `Odd` in `main` (line 5, col 46) found a quotation literal in operand slot 0\n  a bound-dispatched member instantiates its signature at this site's operand types, and a written quotation literal has no type to instantiate at -- declare the slot as a quotation parameter, or pass the literal through one"
        );
}

/// P7b.S8c (REQ-1): the fail-closed tail's message. REQ-3 forbids
/// touching `subst_polytype`'s `expect`, so this located error is the
/// only fence standing between a member variable neither the impl-target
/// match nor this site's operands determine and a raw panic at
/// `src/ir/driver.rs:579`.
#[test]
fn member_unbound_variable_error_names_the_member_the_variable_and_the_site() {
    let probe = probe_word();
    let ctx = probe_ctx(&probe);
    assert_eq!(
            member_unbound_variable_error(
                &ctx,
                Span { line: 5, col: 46, ..Span::default() },
                "odd",
                "Odd",
                "'U",
            ),
            "error: `odd` of `Odd` in `probe` (line 5, col 46) leaves type variable `'U` unbound\n  the impl target's match and this site's operands together determine no type for `'U`, so the member has no instantiation here -- give it an operand position that fixes `'U`"
        );
}

/// P7b.S8c (D2/REQ-2): the concrete-winner arm's fixtures. `impl: Odd for
/// i64` is a genuinely concrete target, so `ground_member_type` pinned the
/// member row's *own* free variable (`'U`) to `i64` at registration and
/// the member word is registered `poly: None` at that grounded effect --
/// there is nothing to instantiate and no `PolySig` to unify, only the
/// site's operands to compare. `operand` is `main`'s body; the `List`
/// declaration is inlined because the unit harness resolves no `import:`.
fn odd_i64_src(operand: &str) -> String {
    format!(
        "type: List['T] | Nil | Cons 'T rest ^List['T] ;\n\
             trait: Odd['T] : odd ( 'T 'U -- ) ; ;\n\
             impl: Odd for i64 : odd drop drop ; ;\n\
             : consume ['T: Odd] ( 'T 'U -- ) odd ;\n\
             : push ( List[i64] i64 -- List[i64] ) swap ^ Cons ;\n\
             : main ( -- ) {operand} ;\n"
    )
}

/// P7b.S8c (D2/REQ-2), the hole: a `List[i64]` reaching an `i64`-pinned
/// member local through a bound dispatch. This arm minted the bare symbol
/// with no site check at all, so the operand flowed into the local slot
/// unexamined and the member body computed on its raw slot word (measured
/// in recon: a `List[i64]`'s head printed verbatim). Located now, naming
/// the member and the offending slot, in `trait_member_operand_error`'s
/// wording -- the same message the direct mono member call already
/// produced for this shape.
#[test]
fn concrete_target_member_dispatch_with_a_mismatched_slot_is_an_error() {
    let err = checked_like_a_build_mangled(&odd_i64_src("Nil 3 push | l | 7 l consume"))
        .expect_err("`List[i64]` disagrees with the `i64` the member local is pinned to");
    assert_eq!(
            err,
            "error: `odd` of `Odd` in `main` (line 4, col 34) expects `i64`, found `List[i64]` in operand slot 1"
        );
}

/// P7b.S8c (D2/REQ-2), the accept side of the same fixture: operands that
/// agree with the pinned local still dispatch. The ruling is "a free
/// member local at a concrete target stays pinned to the target type" --
/// the check rejects disagreement, it does not make the arm polymorphic,
/// so the `'U` slot admits an `i64` and nothing else.
#[test]
fn concrete_target_member_dispatch_with_matching_slots_checks() {
    checked_like_a_build_mangled(&odd_i64_src("7 5 consume"))
        .expect("matching slots pass the concrete site check");
}

/// P7b.S8c (REQ-2), the QuotLit fence on *this* arm. REQ-2's re-grounding
/// step drives the same `apply_subst` the mint arm's does, so it inherits
/// the same hazard: a written quotation literal in a plain member slot
/// would reach `apply_subst`'s `unreachable!`. Both arms call one shared
/// fence, so this pins the same bytes at the concrete-winner dispatch.
#[test]
fn concrete_target_member_dispatch_with_a_quotation_literal_slot_is_a_located_error() {
    let err = checked_like_a_build_mangled(
        "trait: Odd['T] : odd ( 'U 'T -- ) ; ;\n\
             impl: Odd for i64 : odd drop drop ; ;\n\
             : consume ['T: Odd] ( 'T -- ) [ drop ] swap odd ;\n\
             : main ( -- ) 7 consume ;\n",
    )
    .expect_err("a written quotation literal is not the `i64` the local is pinned to");
    assert_eq!(
            err,
            "error: `odd` of `Odd` in `main` (line 3, col 45) found a quotation literal in operand slot 0\n  a bound-dispatched member instantiates its signature at this site's operand types, and a written quotation literal has no type to instantiate at -- declare the slot as a quotation parameter, or pass the literal through one"
        );
}

/// P7b.S8c (REQ-2), the accept-side blast-radius pin. `core::cmp`'s `Ord`
/// (12 concrete impls) is the shipped surface this arm actually gates, and
/// a bound-dispatched `cmp` reaches the new check for real: stubbing the
/// check to reject unconditionally turns this test red, which is what
/// stops it from being a placebo that would pass with the arm untouched.
#[test]
fn ord_dispatch_through_a_bound_passes_the_concrete_site_check() {
    checked_like_a_build_mangled(
        ": go ['T: Ord] ( 'T 'T -- Ordering ) cmp ;\n\
             : main ( -- ) 1 2 go drop ;\n",
    )
    .expect("Ord's concrete-target bound dispatch passes the site check");
}

/// P7b.S6d-PREREQ (REQ-4d site 4): every dispatch path truncates the
/// operands and re-pushes outputs, and a bare `Slot::computed` there
/// laundered the borrow. This drives the top-level, non-splice
/// `check_poly_call` push (`poly.rs:8007`); the other three named pushes
/// each need their own witness (review round, P1-2):
/// `dispatch_output_push_inside_a_spliced_combinator_body_forwards_provenance`
/// (`:7971`), `dispatch_output_push_at_a_mono_member_call_forwards_provenance`
/// (`:2618`), and the comment above `resolve_splice_member_call`'s own push
/// (`:2090`, measured unreachable with a reference-bearing operand rather
/// than witnessed). This is not a future hazard: the bare-slice form
/// below **built and printed `99`** before this slice -- a write through
/// the exclusive `&!a` observed through a shared view of `a` handed
/// through a `( 'T -- 'T )` pass-through -- and deleting `thru` from the
/// same program already produced the rejection.
///
/// Measured mutation: forcing `(None, None)` in `push_dispatch_outputs`
/// makes both cases build.
#[test]
fn dispatch_output_push_forwards_the_operands_borrow_provenance() {
    let bare = check_src(
        ": thru ( 'T -- 'T ) ;\n\
             : main ( -- )\n  0 4 fill | a |\n  &a slice thru | s |\n  \
             &!a | r |\n  r 0 >usize &!> 99 !\n  s len drop\n  a drop\n;\n",
    )
    .unwrap_err();
    assert!(
        bare.contains("`&!a` conflicts with a live borrow of `a`"),
        "{bare}"
    );
    let aggregate = check_src(
        "type: Window view Slice[i64] lo usize ;\n\
             : thru ( 'T -- 'T ) ;\n\
             : main ( -- )\n  0 4 fill | a |\n  \
             &a slice 0 >usize Window thru | w |\n  \
             &!a | r |\n  r 0 >usize &!> 99 !\n  &w &view @ len drop\n  a drop\n;\n",
    )
    .unwrap_err();
    assert!(
        aggregate.contains("`&!a` conflicts with a live borrow of `a`"),
        "{aggregate}"
    );
}

/// P7b.S6d-PREREQ (P1-2, second push site): the bound-dispatch push at
/// `poly.rs:7971` -- reached only when a poly call happens *while
/// checking a spliced combinator body* (`prov.splice_uid` is `Some`),
/// which is a different branch of `check_poly_call` from the top-level
/// call the test above drives (`prov.splice_uid` is `None` there).
/// `usebody`'s own body is what gets spliced; `thru`'s call from inside
/// it is the one that carries `prov.splice_uid`.
///
/// Measured mutation: forcing this specific push (not the one above) to
/// drop the operand's deriv makes this program build silently.
#[test]
fn dispatch_output_push_inside_a_spliced_combinator_body_forwards_provenance() {
    let err = check_src(
        ": pass ( 'T -- 'T ) ;\n\
             : usebody inline['S] ( 'S -- 'S ) pass ;\n\
             : main ( -- )\n  0 4 fill | a |\n  \
             &a slice usebody | s |\n  \
             &!a | r |\n  r 0 >usize &!> 99 !\n  s len drop\n  a drop\n;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`&!a` conflicts with a live borrow of `a`"),
        "{err}"
    );
}

/// P7b.S6d-PREREQ (P1-2, third push site): `resolve_mono_member_call`'s
/// concrete-target push (`poly.rs:2618`), driven by a bare trait-member
/// call from a *mono* (non-generic, non-spliced) caller -- `main` calls
/// `thru` directly, so the member name resolves through the whole-program
/// trait/impl tables rather than through `env` or a combinator splice.
/// `thru` must declare `inline` for the concrete impl's own signature to
/// pass `check_reference_free_signature` at all (Ruling B bans a
/// reference-bearing output from any non-`declares_inline` word), which
/// is also why this witness needs a real trait, not a plain generic word.
///
/// Measured mutation: forcing this push to drop the operand's deriv makes
/// this program build silently.
#[test]
fn dispatch_output_push_at_a_mono_member_call_forwards_provenance() {
    let err = check_src(
        "trait: Thru['S] :\n  thru inline ( 'S -- 'S ) ;\n  ;\n\
             type: Window view Slice[i64] lo usize ;\n\
             impl: Thru for Window\n: thru ;\n;\n\
             : main ( -- )\n  0 4 fill | a |\n  \
             &a slice 0 >usize Window thru | w |\n  \
             &!a | r |\n  r 0 >usize &!> 99 !\n  &w &view @ len drop\n  a drop\n;\n",
    )
    .unwrap_err();
    assert!(
        err.contains("`&!a` conflicts with a live borrow of `a`"),
        "{err}"
    );
}
