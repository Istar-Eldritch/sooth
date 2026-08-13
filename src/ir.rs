//! Backend-neutral IR.
//!
//! The compile-time virtual stack is lowered to SSA-shaped values here, and each
//! word becomes a function taking N inputs and returning M outputs. Control words
//! become basic blocks and branches. This IR feeds QBE today and a WASM sibling
//! lowering later, so it stays neutral: in particular `Ptr` is an opaque handle,
//! never assumed to be a native `u64`, so QBE (native pointers) and WASM
//! (linear-memory offsets) can each concretise it.

use std::collections::{HashMap, HashSet};
use std::mem;

use crate::ast::{
    ArrayDecl, ArrayId, CallInst, Clause, EnumDecl, EnumId, Len, Module, OwnedCellDecl,
    OwnedCellId, PolySig, PolyType, QuotEffect, RefDecl, Span, StackEffect, StructDecl, StructId,
    Subst, Term, TermKind, Type, TypedSlot, WordBody, WordDef,
};

mod destructors;
mod driver;
mod func_builder;
mod layout;
mod types;

pub(crate) use self::destructors::synthesize_aggregate_destructors;
use self::destructors::PathStep;
#[cfg(test)]
use self::destructors::{recursive_disposal_path, synthesize_struct_destructor};
use self::func_builder::{lower_materialized, lower_word_parts, word_ret_ty, EnvPlan, FuncBuilder};

use self::types::QuotId;
pub use self::types::{
    ir_type_of, quotation_layout, Arity, BinOp, Block, BlockId, CmpOp, Instr, IrFunc, IrModule,
    IrType, QuotSigId, QuotSigLayout, Resolver, Terminator, Value, ALLOC_SYMBOL, FREE_SYMBOL,
    OOB_TRAP_SYMBOL, TRACE_ALLOC_ENV, WORD_WIDTH,
};

pub(crate) use self::layout::{
    build_registries, carried_slot_bytes, ArrayLayout, Arrays, Cells, DropOverride, DropOverrides,
    EnumLayout, Enums, FieldLayout, Refs, Registries, StructLayout, Structs,
};
// `VariantLayout` has no non-test caller anywhere in the crate today (only a
// `repl.rs` test constructs one); it is still part of the historical `ir::*`
// re-export contract (spec surface list), so it stays reachable for tests
// without tripping `unused_imports` on a plain (non-test) build.
#[cfg(test)]
pub(crate) use self::layout::VariantLayout;
use self::layout::{
    cell_drop_symbol, enum_drop_symbol, field_is_linear, scalar_size_align, struct_drop_symbol,
    EnumWord, StructWord,
};
// `build_registries_ww`/`scalar_size_align_ww` have no non-test caller in
// `ir.rs` today.
#[cfg(test)]
use self::layout::{build_registries_ww, scalar_size_align_ww};

pub(crate) use self::driver::{collect_quot_sigs, lower_instantiation, lower_word};
pub use self::driver::{lower, lower_line};

#[cfg(test)]
use self::driver::subst_polytype;

/// A shared empty instantiation table for lowering paths with no polymorphic
/// call sites (the REPL, D2; destructor synthesis; unit tests), so
/// `FuncBuilder::new` can hand out a valid reference without every caller
/// threading one.
fn empty_instantiations() -> &'static HashMap<Span, CallInst> {
    static EMPTY: std::sync::OnceLock<HashMap<Span, CallInst>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

/// Slice 8a phase 2: the builtin-overload companion of `empty_instantiations`,
/// handed to every lowering path with no user builtin overloads (the REPL,
/// destructor synthesis, unit tests, and every corpus program).
fn empty_builtin_overloads() -> &'static HashMap<Span, String> {
    static EMPTY: std::sync::OnceLock<HashMap<Span, String>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

/// The poly-arity companion of `empty_instantiations`.
fn empty_poly_arities() -> &'static HashMap<String, usize> {
    static EMPTY: std::sync::OnceLock<HashMap<String, usize>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

/// R19: the combinator-body companion of `empty_instantiations`. A path with
/// no monomorphic quotation-taking words to inline (the REPL, D2; destructor
/// synthesis; unit tests) hands out this empty map.
fn empty_combinators() -> &'static HashMap<String, Vec<Term>> {
    static EMPTY: std::sync::OnceLock<HashMap<String, Vec<Term>>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Line, BOOL_ENUM_ID};
    use crate::check::check;
    use crate::lexer::lex;
    use crate::parser::{parse, parse_line};

    fn lower_src(src: &str) -> IrModule {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        lower(&module).unwrap()
    }

    /// A scalar-only resource with a `drop` overload whose body has one
    /// observable effect (a `Print` no synthesized glue ever emits), so "the
    /// override is the destructor" is assertable on instructions.
    const FILE_RESOURCE: &str = "type: File fd i64 ; : drop ( File -- ) | f | f File>fd . ;";

    /// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
    /// primitive in Slice 8c: an ordinary one-field struct with a `drop`
    /// overload, so it is linear for the same reason any resource is (R3 of
    /// slice 8b), not by any compiler-known bit. Always the first struct in a
    /// source string that uses it, so every other struct's `StructId` shifts
    /// up by one relative to a spy-free program.
    const SPY_DEF: &str =
        "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy>tag . ;\n";

    /// Every symbol an `IrFunc` calls, in emission order: what "the override
    /// ran instead of the glue" is asserted on, rather than a substring of the
    /// emitted text.
    fn call_symbols(func: &IrFunc) -> Vec<&str> {
        instrs(func)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(_, sym, _) => Some(sym.as_str()),
                // Slice 7a (R13a): an indirect call carries no symbol, so it
                // is reported with a sentinel. Widened *before* any lowering
                // can emit `CallIndirect`, so the combinator-splice units
                // (`each`/`while`) catch a splice that regresses into an
                // indirect call, not just a direct one.
                Instr::CallIndirect(..) => Some("<indirect>"),
                _ => None,
            })
            .collect()
    }

    /// Slice 7a (R13a): the shared "is this instruction a call" predicate,
    /// seeing both the direct `Call` and the indirect `CallIndirect`. Replaces
    /// the inline `matches!(i, Instr::Call(..))` closures so a lowering that
    /// regresses a splice into an indirect call is still counted as a call.
    fn is_call_instr(i: &Instr) -> bool {
        matches!(i, Instr::Call(..) | Instr::CallIndirect(..))
    }

    fn func<'a>(module: &'a IrModule, name: &str) -> &'a IrFunc {
        module
            .funcs
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "no emitted func `{name}`: {:?}",
                    module.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
                )
            })
    }

    #[test]
    fn two_drop_overloads_for_different_structs_do_not_collide() {
        // Criterion 16: neither override lands in the generic per-word
        // lowering pass (which would emit two QBE functions literally named
        // `drop`, the second colliding with the first), and each instead fills
        // its own struct's destructor symbol with its own body.
        let module = lower_src(
            "type: A x i64 ; type: B y i64 ; \
             : drop ( A -- ) | a | a A>x . ; : drop ( B -- ) | b | b B>y drop ; \
             : main ( -- ) 1 A drop 2 B drop ;",
        );
        assert!(
            module.funcs.iter().all(|f| f.name != "drop"),
            "an emitted IrFunc was literally named `drop`: {:?}",
            module.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let a = func(&module, &struct_drop_symbol(StructId::from_index(0), None));
        let b = func(&module, &struct_drop_symbol(StructId::from_index(1), None));
        // `A`'s body prints its field, `B`'s discards it: two distinct bodies
        // under two distinct symbols, not one shared or one clobbered.
        assert_eq!(count(a, |i| matches!(i, Instr::Print(_))), 1);
        assert_eq!(count(b, |i| matches!(i, Instr::Print(_))), 0);
    }

    #[test]
    fn ir_registers_overridden_struct_as_linear_despite_all_copy_fields() {
        // Criterion 20/R2: `StructLayout::is_linear` is the IR's own,
        // separately computed bit, folded from declared field types alone --
        // for a scalar-only resource that fold says `Copy`, so the override
        // has to force it. Without the force, no destructor would be
        // synthesized for `File` at all and `emit_drop`'s guard would discard
        // an `f drop` silently.
        let overridden = structs_of(&format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;"));
        let file = layout(&overridden, "File");
        assert!(file.is_linear);
        assert!(file.has_drop_overload);

        let plain = structs_of("type: File fd i64 ; : main ( -- ) 1 File drop ;");
        assert!(!layout(&plain, "File").is_linear);
    }

    #[test]
    fn lower_forces_drop_overload_linearity_even_when_check_never_ran() {
        // R1/R2 code-review fix: `lower` used to trust
        // `StructDecl::has_drop_overload`, a bit only `check::check` sets. A
        // module that reaches `lower` without having gone through `check`
        // (this test skips it, unlike `lower_src`) must still layout `File`
        // as linear and substitute the override, not silently emit nothing.
        let src = format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;");
        let tokens = lex(&src).unwrap();
        let module = parse(&tokens).unwrap();
        let ir_module = lower(&module).unwrap();
        let file = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(call_symbols(func(&ir_module, "main")), vec![file.as_str()]);
        let dtor = func(&ir_module, &file);
        assert_eq!(count(dtor, |i| matches!(i, Instr::Print(_))), 1);
    }

    #[test]
    fn drop_of_an_overridden_struct_calls_its_destructor_symbol() {
        // R2: the whole of dispatch. `lower_call`'s `"drop"` arm is unchanged
        // and still symbol-based; forcing `is_linear` is what makes
        // `emit_drop`'s guard pass, and the substituted body is what the
        // symbol now resolves to.
        let module = lower_src(&format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;"));
        let file = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(call_symbols(func(&module, "main")), vec![file.as_str()]);
        // The destructor is the user's body (one `.` of the field), not the
        // generic glue (which for an all-`Copy` struct emits nothing at all).
        let dtor = func(&module, &file);
        assert_eq!(count(dtor, |i| matches!(i, Instr::Print(_))), 1);
    }

    #[test]
    fn synthesize_destructor_of_resource_with_a_linear_field_uses_user_body_not_field_glue() {
        // Criterion 15/R5: the override runs *instead of* the field glue, not
        // before or alongside it. `Res`'s only field is linear, so the glue
        // would call `Inner`'s destructor symbol directly; the body hands the
        // field to `dispose` instead, so that call is the only one emitted.
        let module = lower_src(&format!(
            "{SPY_DEF}type: Inner s Spy ; type: Res i Inner ; \
             : dispose ( Inner -- ) drop ; \
             : drop ( Res -- ) | r | r Res> dispose ; \
             : main ( -- ) 1 Spy Inner Res drop ;"
        ));
        let inner = struct_drop_symbol(StructId::from_index(1), None);
        let res = struct_drop_symbol(StructId::from_index(2), None);
        assert_eq!(call_symbols(func(&module, &res)), vec!["dispose"]);
        // The glue that would have run is still emitted for `Inner` itself,
        // which has no override: `dispose`'s own `drop` calls it.
        assert_eq!(call_symbols(func(&module, "dispose")), vec![inner.as_str()]);
    }

    #[test]
    fn resource_field_disposed_via_its_own_drop_symbol() {
        // Criterion 13/R7 (ordinary composition): an enclosing struct's
        // per-field disposal calls each linear field's destructor rather than
        // inlining its fields, so a resource field is disposed through the
        // user's body with no new mechanism -- `Holder`'s glue prints nothing
        // itself, it calls `File`'s destructor, which prints.
        let module = lower_src(&format!(
            "{FILE_RESOURCE} type: Holder h File n i64 ; \
             : main ( -- ) 1 File 2 Holder drop ;"
        ));
        let file = struct_drop_symbol(StructId::from_index(0), None);
        let holder = func(&module, &struct_drop_symbol(StructId::from_index(1), None));
        assert_eq!(call_symbols(holder), vec![file.as_str()]);
        assert_eq!(count(holder, |i| matches!(i, Instr::Print(_))), 0);
    }

    #[test]
    fn synthesize_destructor_excludes_override_structs_from_a_fused_disposal_path() {
        // Criterion 14/R7 (the disposal-cycle case): `Chain`'s cycle runs back
        // to itself *through* `Res`. The fused loop inlines every intermediate
        // type's field projection instead of calling its destructor, so
        // fusing this cycle would bypass `Res`'s override and leak its
        // resource silently. With `Res` overridden the search stops there, so
        // `Chain` falls back to per-field disposal and reaches the override
        // through its own symbol.
        let src = "type: Res fd i64 next ^Chain ; type: Chain r Res ; : main ( -- ) ;";
        let plain = Probe::new(src);
        assert!(
            plain.path(plain.struct_ty("Chain")).is_some(),
            "without an override, `Chain` fuses its cycle into one loop"
        );

        let p = Probe::with_overrides(src, &["Res"]);
        assert_eq!(p.path(p.struct_ty("Chain")), None);
        // The search's own root is unaffected: whether `Res` is on a cycle is
        // moot, since its destructor is its override either way (R2).
        assert!(p.path(p.struct_ty("Res")).is_some());

        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let chain = synthesize_struct_destructor(p.struct_id("Chain"), &env, &resolve, p.regs());
        assert_eq!(
            call_symbols(&chain),
            vec![struct_drop_symbol(p.struct_id("Res"), None).as_str()]
        );
    }

    fn structs_of(src: &str) -> Structs {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        Structs::from_structs(&module.structs)
    }

    fn enums_of(src: &str) -> Enums {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        build_registries(
            &module.structs,
            &module.enums,
            &module.arrays,
            &module.owned_cells,
            &module.refs,
        )
        .1
    }

    /// A probe program's four registries, owned so `recursive_disposal_path`
    /// can be called on any of its types by name.
    struct Probe {
        structs: Structs,
        enums: Enums,
        arrays: Arrays,
        cells: Cells,
        refs: Refs,
    }

    impl Probe {
        fn new(src: &str) -> Probe {
            Probe::with_overrides(src, &[])
        }

        /// A `Probe` whose named structs each carry a `drop` overload, set the
        /// way `check` sets it but without a `: drop` word in the source.
        /// Deliberately not written as a program: an override body on a
        /// disposal cycle must dispose something that leads back to its own
        /// receiver, which R6's self-recursion rejection refuses, so R7's
        /// cycle boundary is reachable from the registries but not from a
        /// module that type-checks.
        fn with_overrides(src: &str, overridden: &[&str]) -> Probe {
            let tokens = lex(src).unwrap();
            let mut module = parse(&tokens).unwrap();
            check(&mut module).unwrap();
            for name in overridden {
                let decl = module
                    .structs
                    .iter_mut()
                    .find(|s| s.name == *name)
                    .expect("declared struct");
                decl.has_drop_overload = true;
            }
            let (structs, enums, arrays, cells, refs) = build_registries(
                &module.structs,
                &module.enums,
                &module.arrays,
                &module.owned_cells,
                &module.refs,
            );
            Probe {
                structs,
                enums,
                arrays,
                cells,
                refs,
            }
        }

        fn regs(&self) -> Registries<'_> {
            Registries {
                structs: &self.structs,
                enums: &self.enums,
                arrays: &self.arrays,
                cells: &self.cells,
                refs: &self.refs,
            }
        }

        fn struct_id(&self, name: &str) -> StructId {
            match self.struct_ty(name) {
                IrType::Struct(id) => id,
                other => unreachable!("{other:?}"),
            }
        }

        fn struct_ty(&self, name: &str) -> IrType {
            let idx = self
                .structs
                .layouts
                .iter()
                .position(|l| l.name == name)
                .expect("declared struct");
            IrType::Struct(StructId::from_index(idx))
        }

        fn enum_ty(&self, name: &str) -> IrType {
            let idx = self
                .enums
                .layouts
                .iter()
                .position(|l| l.name == name)
                .expect("declared enum");
            IrType::Enum(EnumId::from_index(idx))
        }

        /// The interned cell holding `payload`, so an expected `Unwrap` names
        /// its cell by what it points at rather than by a guessed index.
        fn cell(&self, payload: IrType) -> OwnedCellId {
            let idx = self
                .cells
                .payload
                .iter()
                .position(|&p| p == payload)
                .expect("interned cell");
            OwnedCellId::from_index(idx)
        }

        fn path(&self, ty: IrType) -> Option<Vec<PathStep>> {
            recursive_disposal_path(ty, self.regs())
        }
    }

    fn layout<'a>(s: &'a Structs, name: &str) -> &'a StructLayout {
        s.layouts.iter().find(|l| l.name == name).expect("layout")
    }

    fn enum_layout<'a>(e: &'a Enums, name: &str) -> &'a EnumLayout {
        e.layouts.iter().find(|l| l.name == name).expect("layout")
    }

    fn instrs(func: &IrFunc) -> Vec<&Instr> {
        func.blocks.iter().flat_map(|b| b.instrs.iter()).collect()
    }

    fn line_terms(src: &str) -> Vec<Term> {
        let tokens = lex(src).unwrap();
        match parse_line(&tokens).unwrap() {
            Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        }
    }

    fn count(func: &IrFunc, pred: impl Fn(&Instr) -> bool) -> usize {
        func.blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter(|i| pred(i))
            .count()
    }

    fn empty_builder<'a>(
        env: &'a HashMap<String, Arity>,
        resolve: Resolver<'a>,
        regs: Registries<'a>,
    ) -> FuncBuilder<'a> {
        FuncBuilder::new(env, resolve, regs, String::new())
    }

    #[test]
    fn quotation_literal_emits_no_instr_and_records_body() {
        // R12u: `lower_term`'s `TermKind::Quotation` arm mints a phantom
        // `Value` that defines no `Instr`, records `Value -> QuotId`, and
        // pushes it; the body is interned, not emitted.
        let env: HashMap<String, Arity> = HashMap::new();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let mut b = empty_builder(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
        );
        let term = &line_terms("[ + ]")[0];
        assert!(matches!(term.kind, TermKind::Quotation(_)));
        b.lower_term(term, false);
        assert!(
            b.cur_instrs.is_empty(),
            "a quotation literal emits no instruction: {:?}",
            b.cur_instrs
        );
        assert_eq!(b.stack.len(), 1);
        let v = b.stack[0];
        assert!(
            b.quot_bodies.contains_key(&v),
            "the phantom value is recorded in quot_bodies"
        );
        assert_eq!(b.quot_defs.len(), 1, "the body is interned once");
    }

    #[test]
    fn call_of_literal_emits_no_call_instr() {
        // Criterion 6b (R13): `[ + ] call` fuses in place, so lowered `main`
        // contains no `Instr::Call`; the phantom quotation never becomes a
        // runtime code value.
        let module = lower_src(": main ( -- ) 1 2 [ + ] call . ;");
        let main = func(&module, "main");
        assert_eq!(count(main, is_call_instr), 0);
        assert_eq!(
            count(main, |i| matches!(i, Instr::Bin(_, BinOp::Add, ..))),
            1
        );
    }

    #[test]
    fn times_lowers_to_a_loop_header_not_a_per_iteration_call() {
        // Criterion 6 (R14/R17): `times` builds a header `Block` carrying the
        // index `Phi`, sealed with a `Terminator::Jnz`, reached by a back-edge
        // `Terminator::Jmp`, with no per-iteration `Instr::Call`. The index
        // `Phi` + header `Jnz` are pinned because "header + back-edge `Jmp` + no
        // `Call`" alone also describes a one-trip or infinite loop.
        let simple = lower_src(": main ( -- ) 0 1000000 [ + ] times . ;");
        let main = func(&simple, "main");
        let header = loop_header(main);
        let hblock = header_block(main, header);
        assert!(
            !header_phis(hblock).is_empty(),
            "the header carries the index phi"
        );
        assert!(
            matches!(hblock.term, Terminator::Jnz(..)),
            "the header is sealed with a Jnz (index < count), got {:?}",
            hblock.term
        );
        let entry_id = main.blocks[0].id;
        assert!(
            main.blocks
                .iter()
                .any(|b| b.id != entry_id && matches!(b.term, Terminator::Jmp(h) if h == header)),
            "a non-entry body block back-edges to the header"
        );
        assert_eq!(
            count(main, is_call_instr),
            0,
            "no per-iteration Instr::Call"
        );

        // On 5a's source (a `Vec2` constructed each iteration): every `Alloc`
        // hoists into the entry block, none into the body block (R17). This is
        // the deterministic R17 witness, not the coarse `ulimit` run.
        let agg = lower_src(
            "type: Vec2 x i64 y i64 ;\n\
             : main ( -- ) 0 1000000 [ | i | i i Vec2 Vec2>x + ] times . ;",
        );
        let main = func(&agg, "main");
        let header = loop_header(main);
        let entry = &main.blocks[0];
        let body = main
            .blocks
            .iter()
            .find(|b| b.id != entry.id && matches!(b.term, Terminator::Jmp(h) if h == header))
            .expect("a body block back-edging to the header");
        assert!(
            entry.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
            "the per-iteration Vec2 Alloc hoists into the entry block"
        );
        assert!(
            !body.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
            "no Alloc in the loop body block (R17)"
        );
    }

    #[test]
    fn times_saves_and_restores_loop_state() {
        // R15u/U12: after the `times` arm returns, all five loop-state fields
        // (`header`/`entry_block`/`alloca_home`/`carried_slots`/`back_edges`)
        // are back to their pre-`times` values. `finalize_loop` clears only
        // two of them, so the arm's explicit save/restore is what lets a later
        // `Alloc` (or a second sequential `times`) not hoist into the dead
        // `times` preheader, and lets a second top-level loop reseat the
        // alloca home to its own entry. Dropping the `alloca_home` member from
        // the shared helper leaves it stuck at the first loop's entry and this
        // fails (mutation-test the guard, U12).
        let env: HashMap<String, Arity> = HashMap::new();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let mut b = empty_builder(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
        );
        // A `times` over an empty row: push the count, then intern a body that
        // consumes just the synthesized index (`[ drop ]`) so the row stays
        // empty and the back-edge arity matches the single index slot.
        let count = b.fresh_value(IrType::I64);
        b.push_instr(Instr::Const(count, 3));
        b.const_vals.insert(count, 3);
        b.stack.push(count);
        let quot_term = &line_terms("[ drop ]")[0];
        b.lower_term(quot_term, false);
        assert_eq!(b.stack.len(), 2, "count beneath the quotation phantom");

        let saved_header = b.header;
        let saved_entry = b.entry_block;
        let saved_alloca_home = b.alloca_home;
        b.lower_call(
            "times",
            Span {
                line: 1,
                col: 1,
                module: 0,
            },
            false,
        );

        assert_eq!(b.header, saved_header, "header restored");
        assert_eq!(b.entry_block, saved_entry, "entry_block restored");
        assert_eq!(b.alloca_home, saved_alloca_home, "alloca_home restored");
        assert!(b.carried_slots.is_empty(), "carried_slots restored");
        assert!(b.back_edges.is_empty(), "back_edges restored");

        // D4: the combinator mid-body site shares the same save/restore
        // helper as the `times` arm above. `lower_self_tail_combinator` is
        // called directly (bypassing the `self_tail` dispatch gate) with a
        // body that is itself the self-call (`foo`), so it back-edges to the
        // header exactly as a real `while` body would, and this exercises the
        // same four-field save/restore.
        let state = b.fresh_value(IrType::I64);
        b.push_instr(Instr::Const(state, 7));
        b.const_vals.insert(state, 7);
        b.stack.push(state);
        let saved_header = b.header;
        let saved_entry = b.entry_block;
        let saved_alloca_home = b.alloca_home;
        b.lower_self_tail_combinator("foo", &line_terms("foo"));

        assert_eq!(b.header, saved_header, "header restored (combinator site)");
        assert_eq!(
            b.entry_block, saved_entry,
            "entry_block restored (combinator site)"
        );
        assert_eq!(
            b.alloca_home, saved_alloca_home,
            "alloca_home restored (combinator site)"
        );
        assert!(
            b.carried_slots.is_empty(),
            "carried_slots restored (combinator site)"
        );
        assert!(
            b.back_edges.is_empty(),
            "back_edges restored (combinator site)"
        );
    }

    #[test]
    fn lower_max_emits_a_compare_and_select_no_call() {
        // R12: `max` lowers inline to `Cmp(Gt)` plus a `Phi`-joined select, no
        // `Instr::Call` and no monomorphization.
        let ir = lower_src(": main ( -- ) 3 5 max . ;");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        assert_eq!(
            count(main, |i| matches!(i, Instr::Cmp(_, CmpOp::Gt, ..))),
            1
        );
        assert_eq!(count(main, |i| matches!(i, Instr::Phi(..))), 1);
        assert_eq!(count(main, is_call_instr), 0);
    }

    #[test]
    fn lower_max_total_emits_no_float_compare() {
        // R13: `max-total` orders by the bit-pattern rule, so the emitted
        // `Cmp`s are all over the unsigned integer key, never `Instr::Cmp`
        // with a float operand.
        let ir = lower_src(": main ( -- ) 1.5 2.5 max-total . ;");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        let float_cmps = instrs(main)
            .iter()
            .filter(|i| match i {
                Instr::Cmp(_, _, a, _) => {
                    matches!(main.value_types[a.0 as usize], IrType::Float { .. })
                }
                _ => false,
            })
            .count();
        assert_eq!(float_cmps, 0);
        assert_eq!(count(main, is_call_instr), 0);
    }

    #[test]
    fn lower_two_output_word_returns_one_bundle_holding_both() {
        // Criterion 9 (R10): a two-output word's body ends in one `Ret` of the
        // synthesized bundle, with both outputs stored into it -- not a single
        // value returned and the other silently dropped.
        let ir = lower_src(": pair ( i64 -- i64 i64 ) dup ; : main ( -- ) 5 pair . . ;");
        let pair = ir.funcs.iter().find(|f| f.name == "pair").unwrap();
        let IrType::Struct(bundle) = pair.ret.expect("a two-output word returns its bundle") else {
            panic!("expected a struct return, got {:?}", pair.ret);
        };
        assert!(ir.structs[bundle.index()].bundle);
        assert_eq!(ir.structs[bundle.index()].fields.len(), 2);

        let last = pair.blocks.last().unwrap();
        let Terminator::Ret(Some(returned)) = last.term else {
            panic!("expected a value return, got {:?}", last.term);
        };
        assert_eq!(
            pair.value_types[returned.0 as usize],
            IrType::Struct(bundle)
        );
        assert_eq!(count(pair, |i| matches!(i, Instr::FieldStore(..))), 2);
    }

    #[test]
    fn lower_call_of_two_output_word_unpacks_the_bundle_onto_the_stack() {
        // R11: the caller reads both outputs back out of the returned bundle
        // (two field loads), so its lowering stack matches the stack the
        // checker verified -- the recon-3 desync that used to panic.
        let ir = lower_src(": pair ( i64 -- i64 i64 ) dup ; : main ( -- ) 5 pair . . ;");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        assert_eq!(count(main, |i| matches!(i, Instr::Call(Some(_), ..))), 1);
        assert_eq!(count(main, |i| matches!(i, Instr::FieldLoad(..))), 2);
        assert_eq!(count(main, |i| matches!(i, Instr::Print(_))), 2);
    }

    #[test]
    fn monomorphization_emits_one_mangled_func_per_instantiation() {
        // R9/R14: a polymorphic word is never emitted under its plain name;
        // instead one mangled `IrFunc` is emitted per distinct ground θ, and
        // each call site targets its own instantiation's symbol through the
        // R14 table, not `dupit`.
        let ir = lower_src(
            ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n\
             : main ( -- ) 5 dupit . . true dupit . . ;",
        );
        assert!(
            ir.funcs.iter().all(|f| f.name != "dupit"),
            "the polymorphic word must not lower under its plain name"
        );
        let mono: Vec<&str> = ir
            .funcs
            .iter()
            .map(|f| f.name.as_str())
            .filter(|n| n.starts_with("sooth_mono_dupit"))
            .collect();
        assert_eq!(mono.len(), 2, "one IrFunc per θ (i64 and bool)");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        let calls = call_symbols(main);
        for sym in &mono {
            assert!(calls.contains(sym), "main should call `{sym}` directly");
        }
    }

    #[test]
    fn lower_single_output_word_keeps_its_scalar_return() {
        // R2/R15: nothing about the bundle path reaches a word with one
        // output; it returns its scalar directly, as before the slice.
        let ir = lower_src(": inc ( i64 -- i64 ) 1 + ;");
        let inc = ir.funcs.iter().find(|f| f.name == "inc").unwrap();
        assert_eq!(inc.ret, Some(IrType::I64));
        assert!(ir.structs.is_empty());
    }

    #[test]
    fn lower_bundle_with_a_linear_field_gets_no_destructor() {
        // Criterion 10 (R10/R11, key risk 1): the bundle for `( -- ^i64 i64 )`
        // folds linear (its first field is an owning cell), yet no drop glue is
        // synthesized for it -- the glue would free the cell the caller's
        // unpack has already moved out.
        let ir =
            lower_src(": cell-and-tag ( -- ^i64 i64 ) 7 ^ 3 ; : main ( -- ) cell-and-tag . ^> . ;");
        let (idx, layout) = ir
            .structs
            .iter()
            .enumerate()
            .find(|(_, l)| l.bundle)
            .expect("the two-output word interned a bundle");
        assert!(
            layout.is_linear,
            "an owning-cell field folds the bundle linear"
        );
        let glue = format!("sooth_struct_drop_{idx}");
        assert!(
            !ir.funcs.iter().any(|f| f.name == glue),
            "a bundle must carry no destructor, found `{glue}`"
        );
    }

    #[test]
    fn lower_two_words_with_one_output_shape_share_one_bundle() {
        // R8: bundles are interned by output tuple, deduped structurally like
        // an array shape, so two words of the same shape share one struct and
        // a third shape gets its own.
        let ir = lower_src(
            ": pair ( i64 -- i64 i64 ) dup ;\n\
             : twice ( i64 -- i64 i64 ) dup ;\n\
             : flags ( -- bool bool ) true false ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(ir.structs.iter().filter(|l| l.bundle).count(), 2);
    }

    #[test]
    fn func_builder_new_threads_current_word_name() {
        // R5: FuncBuilder carries the word being lowered, set from `word.name`
        // in `lower_word`; the REPL path calls the same `lower_word` (no
        // REPL-specific plumbing), so this covers both callers.
        let env: HashMap<String, Arity> = HashMap::new();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let b = FuncBuilder::new(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
            "loop-word".to_string(),
        );
        assert_eq!(b.cur_word_name, "loop-word");
    }

    #[test]
    fn lower_borrow_of_cell_local_gives_the_pointer_a_place() {
        // `&^`/`&!^` project by *loading* the cell pointer out of the
        // place holding it, but a cell local's value already *is* that pointer
        // (an SSA temporary with no address), so borrowing one has to give it a
        // slot first. The load then reads that slot back.
        let ir = lower_src(": w ( -- i64 ) 7 ^ | c | &c &^ @ c ^> drop ;");
        let w = &ir.funcs[0];
        let alloc = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Alloc(v, size, _) if *size == WORD_WIDTH => Some(*v),
                _ => None,
            })
            .expect("borrowing a cell local allocs a one-word place");
        assert!(
            instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::Store(dst, _) if *dst == alloc)),
            "the cell pointer is stored into its new place: {:?}",
            instrs(w)
        );
        assert!(
            instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::Load(_, src) if *src == alloc)),
            "the projection loads the pointer back out: {:?}",
            instrs(w)
        );
    }

    #[test]
    fn lower_reference_through_a_branch_join_keeps_its_referent() {
        // A merged reference is still the opaque `Ptr`, which says nothing
        // about what it points at, so the join has to carry the referent shape
        // across or the projection past it has no field offset to use.
        let ir = lower_src(
            "type: V x i64 y i64 ;\n             : w ( bool -- i64 ) | c | 1 2 V | v | c if &v else &v end &V>x @ ;",
        );
        let w = &ir.funcs[0];
        let phi = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Phi(v, _) => Some(*v),
                _ => None,
            })
            .expect("the two arms merge their references in a phi");
        assert!(
            instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::PtrOffset(_, base, _) if *base == phi)),
            "the projection past the join offsets from the merged value: {:?}",
            instrs(w)
        );
    }

    #[test]
    fn lower_square_has_one_mul() {
        let ir = lower_src(": sq ( i64 -- i64 ) | n | n n * ;");
        let sq = &ir.funcs[0];
        let mul_count = instrs(sq)
            .iter()
            .filter(|i| matches!(i, Instr::Bin(_, BinOp::Mul, _, _)))
            .count();
        assert_eq!(mul_count, 1);
        let last = sq.blocks.last().unwrap();
        assert!(matches!(last.term, Terminator::Ret(Some(_))));
    }

    #[test]
    fn lower_dup_reuses_value_id() {
        // `dup +` squares: both operands must be the same SSA value, dup emits nothing.
        let ir = lower_src(": w ( i64 -- i64 ) dup + ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        assert!(is.iter().all(|i| !matches!(i, Instr::Const(..))));
        let bin = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(_, BinOp::Add, a, b) => Some((*a, *b)),
                _ => None,
            })
            .unwrap();
        assert_eq!(bin.0, bin.1);
    }

    #[test]
    fn lower_binding_emits_no_new_instr() {
        // R10: a binding is a compile-time rebinding of SSA values, so binding
        // the operands and mentioning them lowers to the same instructions as
        // leaving them on the stack. No `Instr` variant was added.
        let bound = lower_src(": w ( -- i64 ) 1 2 | a b | a b - ;");
        let plain = lower_src(": w ( -- i64 ) 1 2 - ;");
        assert_eq!(
            format!("{:?}", instrs(&bound.funcs[0])),
            format!("{:?}", instrs(&plain.funcs[0]))
        );
    }

    #[test]
    fn lower_swap_reorders_without_instr() {
        // `swap -` computes b - a instead of a - b, and swap itself emits no instr.
        let swapped = lower_src(": w ( i64 i64 -- i64 ) swap - ;");
        let plain = lower_src(": w ( i64 i64 -- i64 ) - ;");
        let operands = |ir: &IrModule| {
            instrs(&ir.funcs[0])
                .iter()
                .find_map(|i| match i {
                    Instr::Bin(_, BinOp::Sub, a, b) => Some((*a, *b)),
                    _ => None,
                })
                .unwrap()
        };
        let (sa, sb) = operands(&swapped);
        let (pa, pb) = operands(&plain);
        assert_eq!((sa, sb), (pb, pa));
        assert_eq!(instrs(&swapped.funcs[0]).len(), 1);
    }

    #[test]
    fn lower_drop_pops_without_instr() {
        let ir = lower_src(": w ( i64 i64 -- i64 ) drop ;");
        let w = &ir.funcs[0];
        assert!(instrs(w).is_empty());
        let last = w.blocks.last().unwrap();
        assert!(matches!(last.term, Terminator::Ret(Some(_))));
    }

    #[test]
    fn lower_if_emits_phi_at_join() {
        let ir = lower_src(": w ( bool -- i64 ) if 1 else 2 end ;");
        let w = &ir.funcs[0];
        let has_phi = instrs(w).iter().any(|i| matches!(i, Instr::Phi(..)));
        assert!(has_phi);
        assert!(w
            .blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))));
    }

    #[test]
    fn lower_line_marshals_all_inputs_and_outputs() {
        // `+` from a carried depth of 2 loads both slots and stores the single
        // result: D=2 loads, M=1 store.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let (func, _q, m, _) = lower_line(
            0,
            &line_terms("+"),
            2,
            &[Type::I64, Type::I64],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
        );
        assert_eq!(m, 1);
        assert_eq!(count(&func, |i| matches!(i, Instr::Load(..))), 2);
        assert_eq!(count(&func, |i| matches!(i, Instr::Store(..))), 1);
    }

    #[test]
    fn lower_line_returns_advanced_top() {
        // `2 3 +` from D=0 nets +1, so new_top = top + 8.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let (func, _q, m, _) = lower_line(
            0,
            &line_terms("2 3 +"),
            0,
            &[],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
        );
        assert_eq!(m, 1);
        let last = func.blocks.last().unwrap();
        let ret = match last.term {
            Terminator::Ret(Some(v)) => v,
            ref other => panic!("expected Ret(Some), got {other:?}"),
        };
        // The returned value is `top (%v1) + delta` with delta = 8.
        let is = instrs(&func);
        let (add_lhs, add_rhs) = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(d, BinOp::Add, a, b) if *d == ret => Some((*a, *b)),
                _ => None,
            })
            .expect("a top-advancing add");
        assert_eq!(add_lhs, Value(1), "add should read the `top` param %v1");
        let delta = is
            .iter()
            .find_map(|i| match i {
                Instr::Const(v, n) if *v == add_rhs => Some(*n),
                _ => None,
            })
            .expect("a delta const");
        assert_eq!(delta, 8);
    }

    #[test]
    fn carried_slot_bytes_scalar_is_eight_struct_is_aligned_aggregate() {
        // A scalar always occupies a byte-identical 8-byte carried cell (so
        // every scalar-only line marshals unchanged); a struct occupies its
        // aggregate size rounded up to a multiple of 8.
        let s = structs_of("type: Pair a i8 b i8 ;\ntype: Vec2 x i64 y i64 ;");
        assert_eq!(
            carried_slot_bytes(IrType::I64, &s, &Enums::default(), &Arrays::default()),
            8
        );
        assert_eq!(
            carried_slot_bytes(IrType::Bool, &s, &Enums::default(), &Arrays::default()),
            8
        );
        // Pair is two i8s = 2 bytes, rounded up to one 8-byte cell.
        assert_eq!(
            carried_slot_bytes(
                IrType::Struct(StructId::from_index(0)),
                &s,
                &Enums::default(),
                &Arrays::default()
            ),
            8
        );
        // Vec2 is two i64s = 16 bytes, already a multiple of 8.
        assert_eq!(
            carried_slot_bytes(
                IrType::Struct(StructId::from_index(1)),
                &s,
                &Enums::default(),
                &Arrays::default()
            ),
            16
        );
    }

    fn arrays_of(src: &str) -> Arrays {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        build_registries(
            &module.structs,
            &module.enums,
            &module.arrays,
            &module.owned_cells,
            &module.refs,
        )
        .2
    }

    fn module_of(src: &str) -> Module {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        module
    }

    #[test]
    fn word_width_parameter_sizes_size_types_not_a_literal_eight() {
        // Criterion 2 (structural): both size types' size/align derive from the
        // word width parameter, not a hardcoded `8`. At the default width it is
        // 8; flipping the parameter to 4 changes the derived size of a bare
        // `usize`/`isize` and of an aggregate that embeds one, proving no stray
        // literal.
        assert_eq!(scalar_size_align(IrType::Usize), (8, 8));
        assert_eq!(scalar_size_align_ww(IrType::Usize, 8), (8, 8));
        assert_eq!(scalar_size_align_ww(IrType::Usize, 4), (4, 4));
        assert_eq!(scalar_size_align(IrType::Isize), (8, 8));
        assert_eq!(scalar_size_align_ww(IrType::Isize, 8), (8, 8));
        assert_eq!(scalar_size_align_ww(IrType::Isize, 4), (4, 4));

        // A struct with two `usize` fields and an array of `usize`: both resize
        // with the parameter.
        let m = module_of(": w ( [usize 4] -- ) drop ;\ntype: Cursor a usize b usize ;");
        let (s8, _, a8, ..) =
            build_registries_ww(&m.structs, &m.enums, &m.arrays, &m.owned_cells, &m.refs, 8);
        let (s4, _, a4, ..) =
            build_registries_ww(&m.structs, &m.enums, &m.arrays, &m.owned_cells, &m.refs, 4);
        assert_eq!(s8.layouts[0].size, 16, "two usize fields at width 8");
        assert_eq!(s4.layouts[0].size, 8, "two usize fields at width 4");
        assert_eq!(a8.layouts[0].size, 32, "[usize 4] at width 8");
        assert_eq!(a4.layouts[0].size, 16, "[usize 4] at width 4");
    }

    #[test]
    fn ir_type_of_array_and_usize_map() {
        let m = module_of(": w ( [i64 4] usize -- ) drop drop ;");
        let arr = m.resolve_type_name("usize").unwrap();
        assert_eq!(ir_type_of(arr), IrType::Usize);
        // The `[i64 4]` shape is interned as ArrayId 0.
        assert_eq!(
            ir_type_of(Type::Array(ArrayId::from_index(0), "[i64 4]")),
            IrType::Array(ArrayId::from_index(0))
        );
    }

    #[test]
    fn array_layout_stride_size_align_from_element() {
        // M2: `stride = round_up(elem_size, elem_align)`, `size = count*stride`,
        // `align = elem_align`. An `i64` element: stride 8, size 32, align 8.
        let a = arrays_of(": w ( [i64 4] -- ) drop ;");
        assert_eq!((a.layouts[0].stride, a.layouts[0].size), (8, 32));
        assert_eq!(a.layouts[0].align, 8);
        // A sub-word `u8` element: stride 1, size 3, align 1.
        let b = arrays_of(": w ( [u8 3] -- ) drop ;");
        assert_eq!(
            (b.layouts[0].stride, b.layouts[0].size, b.layouts[0].align),
            (1, 3, 1)
        );
    }

    #[test]
    fn array_layout_nested_array_of_array_sizes_via_registry() {
        // M3: `[[i64 4] 2]` sizes its element (the inner `[i64 4]`, 32 bytes)
        // via the registry: outer stride 32, size 64, align 8.
        let a = arrays_of(": w ( [[i64 4] 2] -- ) drop ;");
        let outer = a.layouts.iter().find(|l| l.name == "[[i64 4] 2]").unwrap();
        assert_eq!((outer.stride, outer.size, outer.align), (32, 64, 8));
    }

    #[test]
    fn carried_slot_bytes_array_is_aligned_aggregate() {
        // R16/M2: a carried array slot occupies its size rounded up to a
        // multiple of 8. `[u8 3]` is 3 bytes, rounding up to one 8-byte cell.
        let a = arrays_of(": w ( [u8 3] -- ) drop ;");
        assert_eq!(
            carried_slot_bytes(
                IrType::Array(ArrayId::from_index(0)),
                &Structs::default(),
                &Enums::default(),
                &a
            ),
            8
        );
    }

    #[test]
    fn fill_lowering_instruction_count_is_independent_of_n() {
        // Slice 6h (D4): `fill`'s re-lowering is a counted loop, so its emitted
        // instruction count is identical at N 4 and 64 (the retired unrolled
        // lowering grew one FieldStore per element), and above a small floor so
        // an empty lowering cannot satisfy it. Replaces
        // `lower_fill_allocs_and_unrolls_n_stores`, whose name encoded the
        // removed unrolling.
        let n4 = count(&lower_src(": w ( -- ) 7 4 fill drop ;").funcs[0], |_| true);
        let n64 = count(&lower_src(": w ( -- ) 7 64 fill drop ;").funcs[0], |_| true);
        assert_eq!(n4, n64);
        assert!(n4 > 4, "not an empty lowering: {n4}");
    }

    #[test]
    fn fill_lowering_instruction_count_at_10000_equals_4() {
        // The compile-cost defect's durable proxy: the retired unrolled
        // lowering emitted one store per element, so N = 10000 was QBE-
        // quadratic on one straight-line block. The counted loop emits the
        // same instruction count at N = 10000 as at N = 4, so code size is O(1)
        // in the count (the re-measured wall-clock numbers are in the commit).
        let n4 = count(&lower_src(": w ( -- ) 7 4 fill drop ;").funcs[0], |_| true);
        let n10k = count(
            &lower_src(": w ( -- ) 7 10000 fill drop ;").funcs[0],
            |_| true,
        );
        assert_eq!(n4, n10k);
    }

    #[test]
    fn fill_lowering_uses_elem_addr_after_relowering() {
        // A real transition assertion: `fill` used `field_ptr`/`PtrOffset` with
        // a compile-time offset before slice 6h, so exactly one runtime
        // `ElemAddr` (the counted store loop's body) is the observable switch.
        // Its stride is the element stride (8 for `[i64]`), not the byte-
        // granular `1` the constructor's zero-init uses.
        let ir = lower_src(": w ( -- ) 7 4 fill drop ;");
        let w = &ir.funcs[0];
        let strides: Vec<i64> = w
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::ElemAddr(_, _, _, s) => Some(*s),
                _ => None,
            })
            .collect();
        assert_eq!(strides, vec![8], "one ElemAddr, element-strided");
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 1);
    }

    #[test]
    fn fill_lowering_result_reaches_a_reference_consumer() {
        // D4: the re-lowering must not disturb `fill`'s consumed operands nor
        // leave the array off the stack. A `fill` result that is then used
        // (indexed via a reference) lowers and reads back the seed, proving the
        // filled array survives the loop and reaches its consumer.
        //
        // This does NOT cover R19 surviving-capture-set forwarding
        // (`check.rs`'s `let surviving = element.surviving;` in
        // `check_array_word`'s "fill" arm): `Slot`/`surviving` is a check-time
        // concept the IR never sees (`lower_src` returns an `IrModule` with no
        // Slot-level information), so no IR-level assertion can exercise it.
        // The real regression test for that forwarding is
        // `check::tests::fill_forwards_surviving_set_so_a_returned_array_rejects_an_escaping_capture`
        // (an end-to-end located-error test, since deleting the forwarding
        // makes an unsound program wrongly build rather than change any IR
        // shape).
        let ir = lower_src(": w ( -- i64 ) 7 4 fill | a | &a 1 &> @ ;");
        let w = &ir.funcs[0];
        // One alloc for the array; the loop stores the seed; the consumer
        // reads it back through a reference projection.
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 1);
        assert!(count(w, |i| matches!(i, Instr::ElemAddr(..))) >= 2);
    }

    #[test]
    fn array_constructor_emits_exactly_one_alloc_of_correct_size() {
        // D3: the constructor allocs exactly one array slot, sized to the
        // layout (`[i64 10]` is 80 bytes / align 8), not one Alloc per element.
        let src = ": w ( -- ) [ i64 ; 10 ] drop ;";
        let ir = lower_src(src);
        let w = &ir.funcs[0];
        let (size, align) = {
            let a = arrays_of(src);
            (a.layouts[0].size, a.layouts[0].align)
        };
        assert_eq!((size, align), (80, 8));
        let allocs: Vec<(u32, u32)> = w
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::Alloc(_, s, al) => Some((*s, *al)),
                _ => None,
            })
            .collect();
        assert_eq!(allocs, vec![(size, align)]);
    }

    #[test]
    fn array_constructor_zero_init_uses_stride_one_and_bounds_by_layout_size() {
        // The zero-init loop is byte-granular: exactly one `ElemAddr` with
        // `stride == 1` (a stride of 8 would skip 7 of every 8 bytes), and its
        // loop bound is a `Const` equal to `ArrayLayout::size` (a bound of
        // `count` would zero only the first `count` bytes). An
        // instruction-*kind* assertion would catch neither mutation.
        let src = ": w ( -- ) [ i64 ; 10 ] drop ;";
        let ir = lower_src(src);
        let w = &ir.funcs[0];
        let size = arrays_of(src).layouts[0].size; // 80
        let strides: Vec<i64> = w
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::ElemAddr(_, _, _, s) => Some(*s),
                _ => None,
            })
            .collect();
        assert_eq!(strides, vec![1], "one byte-granular ElemAddr, stride 1");
        assert_eq!(
            count(w, |i| matches!(i, Instr::Const(_, v) if *v == size as i64)),
            1,
            "the loop bound is one Const equal to ArrayLayout::size"
        );
    }

    #[test]
    fn array_constructor_instruction_count_is_independent_of_count() {
        // A runtime zero-init loop is O(1) in Count: the emitted instruction
        // count is identical at 4 and 64 (an unrolled lowering would grow), and
        // above a small floor so an empty lowering cannot satisfy it.
        let n4 = count(&lower_src(": w ( -- ) [ i64 ; 4 ] drop ;").funcs[0], |_| {
            true
        });
        let n64 = count(
            &lower_src(": w ( -- ) [ i64 ; 64 ] drop ;").funcs[0],
            |_| true,
        );
        assert_eq!(n4, n64);
        assert!(n4 > 4, "not an empty lowering: {n4}");
    }

    #[test]
    fn lower_reference_element_read_is_elem_addr_and_load() {
        // `&>` addresses the element (`ElemAddr`); `@` loads it
        // (`FieldLoad`); neither allocs, since the array is never rebuilt.
        let ir = lower_src(": w ( [i64 4] -- i64 ) | a | &a 0 &> @ ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldLoad(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 0);
    }

    #[test]
    fn lower_reference_element_store_is_elem_addr_and_store_no_rebuild() {
        // `&!>` addresses the element; `!` stores directly, with no alloc and
        // no blit: replacing `set`'s whole-array rebuild is the point.
        let ir = lower_src(": w ( [i64 4] usize i64 -- ) | a i x | &!a i &!> x ! ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldStore(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::Blit(..))), 0);
    }

    #[test]
    fn lower_reference_element_runtime_index_emits_bounds_guard_and_trap_call() {
        // A runtime (non-literal) index guards the access with `index < N`
        // and jumps to a trap block that calls the OOB helper.
        let ir = lower_src(": w ( [i64 4] usize -- i64 ) | a i | &a i &> @ ;");
        let w = &ir.funcs[0];
        assert!(w
            .blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))));
        assert_eq!(
            count(
                w,
                |i| matches!(i, Instr::Call(None, sym, _) if sym == OOB_TRAP_SYMBOL)
            ),
            1
        );
    }

    #[test]
    fn lower_reference_element_constant_index_has_no_runtime_guard() {
        // A checked literal index is bounds-verified at compile time, so it
        // skips the runtime guard entirely — no branch, no trap call.
        let ir = lower_src(": w ( [i64 4] -- i64 ) | a | &a 0 &> @ ;");
        let w = &ir.funcs[0];
        assert!(!w
            .blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))));
        assert_eq!(
            count(
                w,
                |i| matches!(i, Instr::Call(None, sym, _) if sym == OOB_TRAP_SYMBOL)
            ),
            0
        );
    }

    #[test]
    fn lower_len_is_a_constant_with_no_memory_access() {
        // R18: `len` folds to a constant `usize` (the count) with no load and
        // no element addressing.
        let ir = lower_src(": w ( [i64 4] -- usize ) len swap drop ;");
        let w = &ir.funcs[0];
        assert!(instrs(w).iter().any(|i| matches!(i, Instr::Const(_, 4))));
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldLoad(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::Load(..))), 0);
    }

    #[test]
    fn str_literal_lowers_to_a_static_data_reference() {
        // R6: a `str` literal is exactly one `Instr::StrLit`, the backend's
        // hook to emit the static descriptor and take its address.
        let ir = lower_src(": w ( -- str ) \"hi\" ;");
        let w = &ir.funcs[0];
        assert_eq!(
            count(w, |i| matches!(i, Instr::StrLit(_, s) if s == "hi")),
            1
        );
    }

    #[test]
    fn len_of_str_lowers_to_str_len_with_no_call() {
        // R8: `len` on a `str` lowers to the dedicated `StrLen`
        // instruction, not a call and not a hand-written byte offset.
        let ir = lower_src(": w ( -- usize ) \"hi\" len ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::StrLen(..))), 1);
        assert_eq!(count(w, is_call_instr), 0);
    }

    #[test]
    fn cstr_conversion_lowers_to_str_ptr() {
        // R7: `cstr` lowers to the dedicated `StrPtr` instruction.
        let ir = lower_src(": w ( -- cstr ) \"hi\" cstr ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::StrPtr(..))), 1);
    }

    #[test]
    fn len_and_cstr_of_str_emit_no_byte_offset_instruction() {
        // Neither `len` nor `cstr` reads the descriptor via a hand-written
        // `field_ptr` offset (`PtrOffset` + `FieldLoad`) any more; both state
        // their intent through a dedicated instruction instead, keeping the
        // descriptor's layout a backend-only concern.
        let ir = lower_src(": w ( -- ) \"hi\" len drop \"hi\" cstr drop ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::PtrOffset(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldLoad(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::StrLen(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::StrPtr(..))), 1);
    }

    #[test]
    fn extern_call_lowers_to_a_call_with_the_declared_symbol() {
        // R1: an `extern:` declaration's C symbol, not its Sooth word name,
        // is what the emitted call names; binding a name that differs from
        // its symbol (`clen` bound to `strlen`) would not catch a lowering
        // bug that emitted `call $<word-name>` instead.
        let ir = lower_src(
            "extern: clen ( cstr -- usize ) \"strlen\" ;\n\
             : w ( -- usize ) \"hi\" cstr clen ;",
        );
        let w = &ir.funcs[0];
        let calls: Vec<&str> = w
            .blocks
            .iter()
            .flat_map(|b| &b.instrs)
            .filter_map(|i| match i {
                Instr::Call(_, sym, _) => Some(sym.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(calls, vec!["strlen"]);
    }

    #[test]
    fn lower_line_struct_slot_blits_in_and_out() {
        // A carried struct slot is copied out of the buffer on entry and back
        // on exit by aggregate blits, and the returned top advances by the
        // struct's aligned carried size. An empty line carries the one
        // Vec2 straight through: one prologue blit, one epilogue blit.
        let s = structs_of("type: Vec2 x i64 y i64 ;");
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let vec2 = Type::Struct(StructId::from_index(0), "Vec2");
        let (func, _q, m, out_bytes) = lower_line(
            0,
            &line_terms(""),
            1,
            &[vec2],
            &env,
            &resolve,
            Registries {
                structs: &s,
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
        );
        assert_eq!(m, 1);
        assert_eq!(out_bytes, 16);
        assert_eq!(count(&func, |i| matches!(i, Instr::Blit(..))), 2);
        // No scalar 8-byte-cell Load/Store touches a struct slot.
        assert_eq!(count(&func, |i| matches!(i, Instr::Load(..))), 0);
        assert_eq!(count(&func, |i| matches!(i, Instr::Store(..))), 0);
    }

    #[test]
    fn lower_line_carried_str_slot_keeps_its_own_ir_type() {
        // The carried-slot prologue's match used to fall through a `_` arm
        // for `str` (and other non-aggregate types), loading it as a bare
        // `IrType::I64` and losing the type a later `len`/`.`/`cstr` in the
        // line dispatches on. An empty line carries one `str` straight
        // through: the loaded value must keep `IrType::Str`.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let (func, _q, m, out_bytes) = lower_line(
            0,
            &line_terms(""),
            1,
            &[Type::Str],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
        );
        assert_eq!(m, 1);
        assert_eq!(out_bytes, 8);
        let loaded = instrs(&func)
            .iter()
            .find_map(|i| match i {
                Instr::Load(dst, _) => Some(*dst),
                _ => None,
            })
            .expect("a load of the carried str slot");
        assert_eq!(func.value_types[loaded.0 as usize], IrType::Str);
    }

    #[test]
    fn lower_line_scalar_only_uses_eight_byte_cells_and_no_blit() {
        // R16/NF3: a scalar-only line marshals exactly as before — 8-byte-cell
        // stores, `PtrOffset`s at multiples of 8, and never an aggregate
        // `Blit`. `+` from a carried depth of 2 reads cells 0/8 and writes the
        // single result at 0.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let (func, _q, m, out_bytes) = lower_line(
            0,
            &line_terms("+"),
            2,
            &[Type::I64, Type::I64],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
        );
        assert_eq!(m, 1);
        assert_eq!(out_bytes, 8);
        assert_eq!(count(&func, |i| matches!(i, Instr::Blit(..))), 0);
        let offsets: Vec<i64> = instrs(&func)
            .iter()
            .filter_map(|i| match i {
                Instr::PtrOffset(_, _, off) => Some(*off),
                _ => None,
            })
            .collect();
        assert_eq!(
            offsets,
            vec![0, 8, 0],
            "two input cells at 0/8, one output cell at 0"
        );
    }

    #[test]
    fn lower_line_carried_narrow_slot_relabels_after_load() {
        // Q2/R16: a `u8` carried slot loads as `l`-width `i64` from the buffer
        // (canonicalization keeps its low bits authoritative), then must be
        // relabeled to `IrType::Int { bits: 8, signed: false }` via `Conv` so a
        // later homogeneous op in the same line sees the real operand type.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let u8_ty = Type::from_name("u8").unwrap();
        let (func, _q, _m, _) = lower_line(
            0,
            &line_terms("1 >u8 +"),
            1,
            &[u8_ty],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
        );
        let conv_dst = instrs(&func)
            .iter()
            .find_map(|i| match i {
                Instr::Conv(dst, _) => Some(*dst),
                _ => None,
            })
            .expect("a Conv relabeling the loaded slot");
        assert_eq!(
            func.value_types[conv_dst.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
    }

    #[test]
    fn lower_call_uses_resolved_generation_symbol() {
        let mut env = HashMap::new();
        env.insert("sq".to_string(), (1usize, 1usize, None));
        let resolve = |name: &str| format!("{name}__gen2");
        let (func, _q, _m, _) = lower_line(
            0,
            &line_terms("5 sq"),
            0,
            &[],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
        );
        let calls: Vec<&str> = instrs(&func)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(_, sym, _) => Some(sym.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(calls, vec!["sq__gen2"]);
    }

    #[test]
    fn bool_enum_true_false_construct_0_and_1() {
        // Slice 9 (R2): `True`/`False` replace `TermKind::BoolLit`, lowering
        // to the same `0`/`1` scalar discriminant a bare `Const` produced
        // before this migration -- no memory aggregate, `IrType::Enum`
        // carrying `BOOL_ENUM_ID` (R1's general zero-payload-enum scalar
        // rule, not `IrType::Bool` directly).
        // Single-output words each, so neither triggers R10's bundle-return
        // packing (which would add its own, unrelated `Instr::Alloc` for the
        // bundle struct and muddy the "no aggregate" assertion below).
        let ir = lower_src(": t ( -- bool ) true ; : f ( -- bool ) false ;");
        let t = ir.funcs.iter().find(|f| f.name == "t").unwrap();
        let f = ir.funcs.iter().find(|f| f.name == "f").unwrap();
        assert_eq!(
            instrs(t).iter().find_map(|i| match i {
                Instr::Const(_, n) => Some(*n),
                _ => None,
            }),
            Some(1),
            "true -> 1"
        );
        assert_eq!(
            instrs(f).iter().find_map(|i| match i {
                Instr::Const(_, n) => Some(*n),
                _ => None,
            }),
            Some(0),
            "false -> 0"
        );
        assert!(
            !instrs(t).iter().any(|i| matches!(i, Instr::Alloc(..))),
            "a zero-payload enum construct must not allocate a memory aggregate"
        );
        let v = instrs(t)
            .iter()
            .find_map(|i| match i {
                Instr::Const(v, 1) => Some(*v),
                _ => None,
            })
            .expect("a const 1 for `true`");
        assert_eq!(t.value_types[v.0 as usize], IrType::Enum(BOOL_ENUM_ID));
    }

    #[test]
    fn lower_comparison_result_is_bool() {
        let ir = lower_src(": w ( i64 i64 -- bool ) > ;");
        let w = &ir.funcs[0];
        let v = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Cmp(v, CmpOp::Gt, _, _) => Some(*v),
                _ => None,
            })
            .expect("a Gt comparison");
        assert_eq!(w.value_types[v.0 as usize], IrType::Bool);
    }

    #[test]
    fn lower_print_emits_print_instr() {
        let ir = lower_src(": w ( i64 -- ) . ;");
        let w = &ir.funcs[0];
        assert!(instrs(w).iter().any(|i| matches!(i, Instr::Print(_))));
        let last = w.blocks.last().unwrap();
        assert!(matches!(last.term, Terminator::Ret(None)));
    }

    #[test]
    fn lower_print_on_bool_and_float_emits_same_print_instr() {
        // `.` lowers to one `Print` regardless of operand type: the IR stays
        // neutral and the backend dispatches on the value's own `IrType`.
        let bool_ir = lower_src(": w ( bool -- ) . ;");
        assert!(instrs(&bool_ir.funcs[0])
            .iter()
            .any(|i| matches!(i, Instr::Print(_))));
        let float_ir = lower_src(": w ( f64 -- ) . ;");
        assert!(instrs(&float_ir.funcs[0])
            .iter()
            .any(|i| matches!(i, Instr::Print(_))));
    }

    #[test]
    fn lower_line_carried_float_slot_loads_as_float() {
        // A carried `f64` slot loads at its float `IrType` (R20), so the value
        // re-enters as a true float rather than a stale `i64`; no `Conv`
        // relabel is needed (that path is integer-only).
        let terms = line_terms("dup");
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let f64_ty = Type::from_name("f64").unwrap();
        let (func, _q, _m, _) = lower_line(
            0,
            &terms,
            1,
            &[f64_ty],
            &env,
            &resolve,
            Registries {
                structs: &Structs::default(),
                enums: &Enums::default(),
                arrays: &Arrays::default(),
                cells: &Cells::default(),
                refs: &Refs::default(),
            },
            empty_instantiations(),
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
        );
        let loaded = func
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .find_map(|i| match i {
                Instr::Load(v, _) => Some(*v),
                _ => None,
            });
        let v = loaded.expect("a load in the prologue");
        assert_eq!(func.value_types[v.0 as usize], IrType::Float { bits: 64 });
        assert!(!func
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .any(|i| matches!(i, Instr::Conv(..))));
    }

    #[test]
    fn ir_type_of_each_width_expected() {
        let cases: &[(&str, u8, bool)] = &[
            ("i8", 8, true),
            ("i16", 16, true),
            ("i32", 32, true),
            ("i64", 64, true),
            ("u8", 8, false),
            ("u16", 16, false),
            ("u32", 32, false),
            ("u64", 64, false),
        ];
        for (name, bits, signed) in cases {
            let ty = Type::from_name(name).unwrap();
            assert_eq!(
                ir_type_of(ty),
                IrType::Int {
                    bits: *bits,
                    signed: *signed
                },
                "mapping {name}"
            );
        }
        // Slice 9 (R1/R2): `Bool` is `Type::Enum(BOOL_ENUM_ID, "bool")`, and
        // flows through the general enum arm above like any other enum --
        // whether its value ends up scalar or a memory aggregate is decided
        // by `EnumLayout::is_scalar`, not by a hard-coded arm here (which has
        // no registry access to consult).
        assert_eq!(ir_type_of(Type::BOOL), IrType::Enum(BOOL_ENUM_ID));
    }

    #[test]
    fn ir_type_of_float_widths_expected() {
        assert_eq!(
            ir_type_of(Type::from_name("f32").unwrap()),
            IrType::Float { bits: 32 }
        );
        assert_eq!(
            ir_type_of(Type::from_name("f64").unwrap()),
            IrType::Float { bits: 64 }
        );
    }

    #[test]
    fn lower_float_literal_is_constf_f64_typed() {
        let ir = lower_src(": w ( -- f64 ) 2.5 ;");
        let w = &ir.funcs[0];
        let v = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::ConstF(v, x) if *x == 2.5 => Some(*v),
                _ => None,
            })
            .expect("a ConstF for the float literal");
        assert_eq!(w.value_types[v.0 as usize], IrType::Float { bits: 64 });
    }

    #[test]
    fn lower_float_div_routes_to_div_op() {
        // `/` lowers to `BinOp::Div` whose result carries the float operand type.
        let ir = lower_src(": w ( -- f64 ) 1.0 2.0 / ;");
        let w = &ir.funcs[0];
        let v = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Div, _, _) => Some(*v),
                _ => None,
            })
            .expect("a Div bin op");
        assert_eq!(w.value_types[v.0 as usize], IrType::Float { bits: 64 });
    }

    #[test]
    fn lower_conv_pushes_target_typed_value() {
        // `5 >u8` lowers the literal, then a `Conv` whose dst carries the u8 type.
        let ir = lower_src(": w ( -- u8 ) 5 >u8 ;");
        let w = &ir.funcs[0];
        let dst = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Conv(dst, _) => Some(*dst),
                _ => None,
            })
            .expect("a Conv instr");
        assert_eq!(
            w.value_types[dst.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
    }

    #[test]
    fn lower_bitwise_and_or_xor_route_to_matching_binop() {
        let ir = lower_src(": w ( -- i32 ) 1 >i32 2 >i32 and 3 >i32 or 4 >i32 xor ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::And, _, _))));
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::Or, _, _))));
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::Xor, _, _))));
    }

    #[test]
    fn lower_not_emits_xor_with_neg1_const() {
        let ir = lower_src(": w ( -- u8 ) 5 >u8 not ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        let neg1 = is
            .iter()
            .find_map(|i| match i {
                Instr::Const(v, -1) => Some(*v),
                _ => None,
            })
            .expect("a -1 const");
        let xor = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Xor, _, b) if *b == neg1 => Some(*v),
                _ => None,
            })
            .expect("a xor against the -1 const");
        assert_eq!(
            w.value_types[xor.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
    }

    #[test]
    fn lower_not_on_bool_emits_xor_with_1_const_not_neg1() {
        // Type-directed `not`: on a `bool` it must flip the low bit
        // (`xor operand, 1`), not the integer-complement `xor operand, -1`,
        // since `-1`/`-2` are not valid canonical `bool` values.
        let ir = lower_src(": w ( -- bool ) true not ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        assert!(
            !is.iter().any(|i| matches!(i, Instr::Const(_, -1))),
            "bool `not` must not use a -1 mask"
        );
        let (xor_v, mask_operand) = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Xor, _, b) => Some((*v, *b)),
                _ => None,
            })
            .expect("a xor bin op");
        assert_eq!(w.value_types[xor_v.0 as usize], IrType::Enum(BOOL_ENUM_ID));
        let mask_const = is.iter().find_map(|i| match i {
            Instr::Const(v, n) if *v == mask_operand => Some(*n),
            _ => None,
        });
        assert_eq!(mask_const, Some(1));
    }

    #[test]
    fn lower_bitwise_and_or_xor_accept_bool_operands() {
        let ir =
            lower_src(": w ( -- bool ) true false and true false or drop true false xor drop ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        for op in [BinOp::And, BinOp::Or, BinOp::Xor] {
            let v = is
                .iter()
                .find_map(|i| match i {
                    Instr::Bin(v, o, ..) if *o == op => Some(*v),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected a {op:?} bin op"));
            assert_eq!(w.value_types[v.0 as usize], IrType::Enum(BOOL_ENUM_ID));
        }
    }

    #[test]
    fn lower_le_ge_ne_route_to_matching_cmpop() {
        let ir = lower_src(": w ( -- bool bool bool ) 1 2 <= 1 2 >= 1 2 <> ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        for op in [CmpOp::Le, CmpOp::Ge, CmpOp::Ne] {
            assert!(
                is.iter()
                    .any(|i| matches!(i, Instr::Cmp(_, o, _, _) if *o == op)),
                "expected a {op:?} comparison"
            );
        }
    }

    #[test]
    fn lower_shl_shr_route_to_matching_binop_with_lhs_type() {
        let ir = lower_src(": w ( -- u8 ) 200 >u8 3 shl 3 shr ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        let shl_ty = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Shl, _, _) => Some(*v),
                _ => None,
            })
            .expect("a Shl bin op");
        assert_eq!(
            w.value_types[shl_ty.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::Shr, _, _))));
    }

    #[test]
    fn lower_add_u8_result_is_u8_typed() {
        // Drive `lower_call`'s arithmetic arm with hand-typed u8 operands
        // directly, isolating the arm from parsing/checking, and assert the
        // result carries the operand type through to its `IrType`.
        let u8 = IrType::Int {
            bits: 8,
            signed: false,
        };
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let mut b = FuncBuilder::new(
            &env,
            &resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
            "w".to_string(),
        );
        let x = b.fresh_value(u8);
        let y = b.fresh_value(u8);
        b.stack = vec![x, y];
        b.lower_call("+", Span::default(), false);
        let top = *b.stack.last().unwrap();
        assert_eq!(b.value_type(top), u8);
    }

    #[test]
    fn struct_layout_flat_i64_fields_offsets_and_size() {
        let s = structs_of("type: Vec2 x i64 y i64 ;");
        let v = layout(&s, "Vec2");
        assert_eq!(v.size, 16);
        assert_eq!(v.align, 8);
        assert_eq!(v.fields[0].offset, 0);
        assert_eq!(v.fields[1].offset, 8);
    }

    #[test]
    fn struct_layout_packed_subword_fields_natural_alignment() {
        // Two `i8`s pack at 0 and 1; the `i64` aligns to 8; whole size 16.
        let s = structs_of("type: Packed p i8 q i8 r i64 ;");
        let p = layout(&s, "Packed");
        assert_eq!(
            (p.fields[0].offset, p.fields[1].offset, p.fields[2].offset),
            (0, 1, 8)
        );
        assert_eq!((p.size, p.align), (16, 8));
    }

    #[test]
    fn struct_layout_nested_uses_inner_size_and_align() {
        let s = structs_of("type: Vec2 x i64 y i64 ; type: Segment from Vec2 to Vec2 ;");
        let seg = layout(&s, "Segment");
        assert_eq!((seg.fields[0].offset, seg.fields[1].offset), (0, 16));
        assert_eq!((seg.size, seg.align), (32, 8));
    }

    #[test]
    fn struct_layout_zero_field_is_size_0_align_1() {
        let s = structs_of("type: Unit ;");
        let u = layout(&s, "Unit");
        assert_eq!((u.size, u.align), (0, 1));
        assert!(u.fields.is_empty());
    }

    #[test]
    fn lower_constructor_allocs_and_stores_each_field() {
        // The constructor allocs one aggregate slot and width-exact-stores both
        // fields; no aggregate copy for a flat struct.
        let ir = lower_src("type: Vec2 x i64 y i64 ; : mk ( i64 i64 -- Vec2 ) Vec2 ;");
        let mk = ir.funcs.iter().find(|f| f.name == "mk").unwrap();
        assert_eq!(count(mk, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(mk, |i| matches!(i, Instr::FieldStore(..))), 2);
    }

    #[test]
    fn lower_getter_is_single_field_load_no_copy() {
        let ir = lower_src("type: Vec2 x i64 y i64 ; : gx ( Vec2 -- i64 ) Vec2>x ;");
        let gx = ir.funcs.iter().find(|f| f.name == "gx").unwrap();
        assert_eq!(count(gx, |i| matches!(i, Instr::FieldLoad(..))), 1);
        assert_eq!(count(gx, |i| matches!(i, Instr::Blit(..))), 0);
        assert_eq!(count(gx, |i| matches!(i, Instr::Alloc(..))), 0);
    }

    #[test]
    fn lower_setter_allocs_new_blits_all_and_overwrites_one_field() {
        // Functional update: alloc a fresh aggregate, blit all bytes, then a
        // single width-exact store of the replaced field.
        let ir = lower_src("type: Vec2 x i64 y i64 ; : sx ( Vec2 i64 -- Vec2 ) Vec2<x ;");
        let sx = ir.funcs.iter().find(|f| f.name == "sx").unwrap();
        assert_eq!(count(sx, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(sx, |i| matches!(i, Instr::Blit(..))), 1);
        assert_eq!(count(sx, |i| matches!(i, Instr::FieldStore(..))), 1);
    }

    #[test]
    fn lower_dup_of_struct_allocs_and_blits() {
        // R14: `dup` of a struct copies the aggregate bytes (fresh alloc +
        // blit), unlike a scalar `dup` which reuses the value id. Single
        // output plus a `drop` of the extra copy, so this measures only
        // `dup`'s own copy, not the multi-output bundle-pack path.
        let ir = lower_src("type: Vec2 x i64 y i64 ; : d ( Vec2 -- Vec2 ) dup drop ;");
        let d = ir.funcs.iter().find(|f| f.name == "d").unwrap();
        assert_eq!(count(d, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(d, |i| matches!(i, Instr::Blit(..))), 1);
    }

    #[test]
    fn lower_destructure_loads_every_field() {
        let ir = lower_src("type: Vec2 x i64 y i64 ; : ex ( Vec2 -- i64 i64 ) Vec2> ;");
        let ex = ir.funcs.iter().find(|f| f.name == "ex").unwrap();
        assert_eq!(count(ex, |i| matches!(i, Instr::FieldLoad(..))), 2);
    }

    #[test]
    fn lower_zero_field_constructor_allocs_destructure_emits_nothing() {
        let ir = lower_src("type: Unit ; : u ( -- ) Unit Unit> ;");
        let u = ir.funcs.iter().find(|f| f.name == "u").unwrap();
        assert_eq!(count(u, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(u, |i| matches!(i, Instr::FieldLoad(..))), 0);
        assert_eq!(count(u, |i| matches!(i, Instr::Blit(..))), 0);
    }

    #[test]
    fn ir_type_of_struct_maps_to_struct_irtype() {
        let tokens = lex("type: Vec2 x i64 y i64 ;").unwrap();
        let module = parse(&tokens).unwrap();
        let ty = module.resolve_type_name("Vec2").unwrap();
        assert!(matches!(ir_type_of(ty), IrType::Struct(_)));
    }

    #[test]
    fn ir_type_of_enum_maps_to_enum_irtype() {
        let tokens = lex("type: Shape | Circle r f64 | Rect w f64 h f64 ;").unwrap();
        let module = parse(&tokens).unwrap();
        let ty = module.resolve_type_name("Shape").unwrap();
        assert!(matches!(ir_type_of(ty), IrType::Enum(_)));
    }

    #[test]
    fn enum_layout_tag_first_payload_at_max_variant_align() {
        // R13/M1: an i32 tag at offset 0, the payload rounded up to the
        // largest variant's align (8, for the f64 fields), so the tag's 4
        // trailing bytes are padding; size = payload_offset(8) + max payload
        // (Rect's two f64s = 16) = 24; align 8.
        let e = enums_of("type: Shape | Circle r f64 | Rect w f64 h f64 ;");
        let s = enum_layout(&e, "Shape");
        assert_eq!(s.tag_offset, 0);
        assert_eq!(
            s.tag_ty,
            IrType::Int {
                bits: 32,
                signed: true
            }
        );
        assert_eq!(s.payload_offset, 8);
        assert_eq!((s.size, s.align), (24, 8));
        // Circle: one f64 at payload-relative 0; Rect: two f64s at 0 and 8.
        assert_eq!(s.variants[0].fields[0].offset, 0);
        assert_eq!(
            (
                s.variants[1].fields[0].offset,
                s.variants[1].fields[1].offset
            ),
            (0, 8)
        );
    }

    #[test]
    fn zero_payload_enum_lowers_to_scalar_discriminant() {
        // R1 (D-A): the general rule -- any enum whose every variant carries
        // an empty payload lowers to a bare 1-byte scalar discriminant, no
        // payload region, no memory aggregate. Exercised on a *non-`Bool`*
        // enum, so this proves the rule is general, not a `Bool` carve-out.
        let e = enums_of("type: Dir | N | E | S | W ;");
        let d = enum_layout(&e, "Dir");
        assert!(d.is_scalar);
        assert_eq!(d.payload_offset, 0);
        assert_eq!((d.size, d.align), (1, 1));
        assert_eq!(d.variants.len(), 4);
        assert!(d.variants.iter().all(|v| v.fields.is_empty()));
    }

    #[test]
    fn payload_bearing_enum_layout_unchanged() {
        // R1: an enum with at least one payload-bearing variant keeps the
        // pre-existing tagged-aggregate layout untouched by the scalar rule.
        let e = enums_of("type: Shape | Circle r f64 | Rect w f64 h f64 ;");
        let s = enum_layout(&e, "Shape");
        assert!(!s.is_scalar);
        assert_eq!(s.payload_offset, 8);
        assert_eq!((s.size, s.align), (24, 8));
    }

    #[test]
    fn enum_layout_mixed_variant_field_widths_pack_within_payload() {
        // A variant with sub-word + i64 fields packs at natural alignment
        // within the payload; the largest variant sizes the payload.
        let e = enums_of("type: E | A x i8 y i64 | B v i16 ;");
        let s = enum_layout(&e, "E");
        // A: i8 at 0, i64 aligned to 8 -> offset 8, variant size 16, align 8.
        assert_eq!(
            (
                s.variants[0].fields[0].offset,
                s.variants[0].fields[1].offset
            ),
            (0, 8)
        );
        // payload align 8 (A's i64), payload_offset 8, max payload 16, size 24.
        assert_eq!(s.payload_offset, 8);
        assert_eq!((s.size, s.align), (24, 8));
    }

    #[test]
    fn enum_layout_nested_struct_payload_sized_via_combined_registry() {
        // D9: a variant field of struct type is sized via its layout (16 for a
        // two-f64 Vec2), not `scalar_size_align`.
        let (structs, enums, _arrays, _cells, _refs) = {
            let src = "type: Vec2 x f64 y f64 ; type: Shape | Dot p Vec2 | Unit ;";
            let tokens = lex(src).unwrap();
            let mut module = parse(&tokens).unwrap();
            check(&mut module).unwrap();
            build_registries(
                &module.structs,
                &module.enums,
                &module.arrays,
                &module.owned_cells,
                &module.refs,
            )
        };
        let _ = structs;
        let s = enum_layout(&enums, "Shape");
        // Dot's Vec2 payload: 16 bytes at payload-relative 0; payload align 8.
        assert_eq!(s.variants[0].fields[0].size, 16);
        assert_eq!(s.payload_offset, 8);
        assert_eq!((s.size, s.align), (24, 8));
    }

    #[test]
    fn struct_field_of_enum_type_sized_via_combined_registry() {
        // D9: a struct field of enum type is sized via the enum's layout, not
        // `scalar_size_align`; the struct places the next field past it.
        let (structs, _enums, _arrays, _cells, _refs) = {
            let src =
                "type: Shape | Circle r f64 | Rect w f64 h f64 ; type: Tagged k Shape n i64 ;";
            let tokens = lex(src).unwrap();
            let mut module = parse(&tokens).unwrap();
            check(&mut module).unwrap();
            build_registries(
                &module.structs,
                &module.enums,
                &module.arrays,
                &module.owned_cells,
                &module.refs,
            )
        };
        let t = layout(&structs, "Tagged");
        // Shape is 24 bytes align 8: k at 0 (size 24), n (i64) at 24; size 32.
        assert_eq!((t.fields[0].offset, t.fields[0].size), (0, 24));
        assert_eq!(t.fields[1].offset, 24);
        assert_eq!((t.size, t.align), (32, 8));
    }

    #[test]
    fn lower_constructor_allocs_stores_tag_and_each_field() {
        // R15: a variant constructor allocs the tagged aggregate, stores the
        // discriminant as a `Const`, then width-exact-stores each field. Rect
        // has two fields, so: one Alloc, one tag Const, three FieldStores
        // (tag + two fields).
        let ir = lower_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ; : mk ( f64 f64 -- Shape ) Rect ;",
        );
        let mk = ir.funcs.iter().find(|f| f.name == "mk").unwrap();
        assert_eq!(count(mk, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(mk, |i| matches!(i, Instr::FieldStore(..))), 3);
        // The tag store writes the variant index (Rect = 1).
        assert!(instrs(mk).iter().any(|i| matches!(i, Instr::Const(_, 1))));
    }

    #[test]
    fn lower_zero_field_constructor_stores_only_the_tag() {
        // A zero-field variant constructs with just the tag store: one Alloc,
        // one FieldStore (the tag), no payload store.
        let ir = lower_src("type: MaybeInt | None | Some v i64 ; : n ( -- MaybeInt ) None ;");
        let n = ir.funcs.iter().find(|f| f.name == "n").unwrap();
        assert_eq!(count(n, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(n, |i| matches!(i, Instr::FieldStore(..))), 1);
        // None is variant index 0.
        assert!(instrs(n).iter().any(|i| matches!(i, Instr::Const(_, 0))));
    }

    #[test]
    fn lower_dup_of_enum_allocs_and_blits() {
        // R15: `dup` of an enum copies the aggregate bytes (fresh alloc +
        // blit), like a struct and unlike a scalar. Single output plus a
        // `drop` of the extra copy, so this measures only `dup`'s own copy,
        // not the multi-output bundle-pack path.
        let ir = lower_src(
            "type: MaybeInt | None | Some v i64 ; : d ( MaybeInt -- MaybeInt ) dup drop ;",
        );
        let d = ir.funcs.iter().find(|f| f.name == "d").unwrap();
        assert_eq!(count(d, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(d, |i| matches!(i, Instr::Blit(..))), 1);
    }

    #[test]
    fn carried_slot_bytes_enum_is_aligned_aggregate() {
        // R17: a carried enum slot occupies its size rounded up to a multiple
        // of 8. Shape is 24 bytes (already a multiple of 8); a tag-only enum
        // (4 bytes pre-Slice-9, now a 1-byte scalar) rounds up to one 8-byte
        // cell either way. `enums_of` parses through the full pipeline, so
        // `bool` occupies the reserved index 0 (Slice 9, R2) ahead of the
        // source's own `Shape`/`Dir`.
        let e = enums_of("type: Shape | Circle r f64 | Rect w f64 h f64 ; type: Dir | N | S ;");
        assert_eq!(
            carried_slot_bytes(
                IrType::Enum(EnumId::from_index(1)),
                &Structs::default(),
                &e,
                &Arrays::default()
            ),
            24
        );
        assert_eq!(
            carried_slot_bytes(
                IrType::Enum(EnumId::from_index(2)),
                &Structs::default(),
                &e,
                &Arrays::default()
            ),
            8
        );
    }

    #[test]
    fn lower_line_enum_slot_blits_in_and_out() {
        // R17: a carried enum slot is copied out of the buffer on entry and
        // back on exit by aggregate blits, and the returned top advances by
        // the enum's aligned carried size. An empty line carries the one Shape
        // straight through: one prologue blit, one epilogue blit.
        let src = "type: Shape | Circle r f64 | Rect w f64 h f64 ;";
        let (structs, enums, arrays, cells, refs) = {
            let tokens = lex(src).unwrap();
            let mut module = parse(&tokens).unwrap();
            check(&mut module).unwrap();
            build_registries(
                &module.structs,
                &module.enums,
                &module.arrays,
                &module.owned_cells,
                &module.refs,
            )
        };
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        // `bool` occupies the reserved index 0 (Slice 9, R2), so `Shape` is 1.
        let shape = Type::Enum(EnumId::from_index(1), "Shape");
        let (func, _q, m, out_bytes) = lower_line(
            0,
            &line_terms(""),
            1,
            &[shape],
            &env,
            &resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
            empty_instantiations(),
            empty_builtin_overloads(),
            empty_poly_arities(),
            empty_combinators(),
        );
        assert_eq!(m, 1);
        assert_eq!(out_bytes, 24);
        assert_eq!(count(&func, |i| matches!(i, Instr::Blit(..))), 2);
        assert_eq!(count(&func, |i| matches!(i, Instr::Load(..))), 0);
        assert_eq!(count(&func, |i| matches!(i, Instr::Store(..))), 0);
    }

    #[test]
    fn lower_clause_word_builds_nway_dispatch_and_join_phi() {
        // R16: a clause word loads the discriminant (one FieldLoad on the
        // scrutinee tag), builds an N-way `Cmp(Eq)` compare-chain (N-1
        // compares for N variants, the last variant a fall-through), and
        // merges the clauses at a single join with one Phi per declared
        // output. A 4-variant enum: 3 Cmp(Eq), one Phi.
        let ir = lower_src(
            "type: Cmd | Halt | Push v i64 | Add | Dbl ;
             : run ( i64 Cmd -- i64 ) | Halt drop 0 | Push swap drop | Add 1 + | Dbl 2 * ;",
        );
        let run = ir.funcs.iter().find(|f| f.name == "run").unwrap();
        // Three `Cmp(Eq)` compares for four variants (the last falls through).
        assert_eq!(
            count(run, |i| matches!(i, Instr::Cmp(_, CmpOp::Eq, _, _))),
            3
        );
        // Exactly one Phi (single declared output) merging all four clauses.
        let phi_arms: Vec<usize> = run
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::Phi(_, arms) => Some(arms.len()),
                _ => None,
            })
            .collect();
        assert_eq!(phi_arms, vec![4]);
    }

    #[test]
    fn lower_single_variant_clause_word_jumps_without_compare() {
        // R16: a single-variant (newtype) enum needs no compare — the sole
        // clause is the terminal fall-through, reached by a direct jump.
        let ir = lower_src("type: Id | Wrap v i64 ; : unwrap ( Id -- i64 ) | Wrap ;");
        let unwrap = ir.funcs.iter().find(|f| f.name == "unwrap").unwrap();
        assert_eq!(count(unwrap, |i| matches!(i, Instr::Cmp(..))), 0);
    }

    /// The loop header of a self-tail-recursive word: the entry block (block 0)
    /// jumps to it (R6), so its id is the entry's `Jmp` target.
    fn loop_header(func: &IrFunc) -> BlockId {
        match func.blocks[0].term {
            Terminator::Jmp(h) => h,
            ref t => panic!("entry block should Jmp to the loop header, got {t:?}"),
        }
    }

    fn header_block(func: &IrFunc, header: BlockId) -> &Block {
        func.blocks.iter().find(|b| b.id == header).expect("header")
    }

    fn header_phis(block: &Block) -> Vec<&Vec<(BlockId, Value)>> {
        block
            .instrs
            .iter()
            .filter_map(|i| match i {
                Instr::Phi(_, arms) => Some(arms),
                _ => None,
            })
            .collect()
    }

    fn jmps_to(func: &IrFunc, target: BlockId) -> usize {
        func.blocks
            .iter()
            .filter(|b| matches!(b.term, Terminator::Jmp(h) if h == target))
            .count()
    }

    #[test]
    fn tail_self_call_lowers_to_back_edge_not_call() {
        // Criterion 2 (R6/R7/R8): a self-tail-recursive word lowers to a header
        // carrying one phi per loop-carried (input-arity) slot, and the tail
        // self-call is a `Jmp` back to that header with no `Instr::Call` to
        // self. `go` has input arity 2, so the header has two phis.
        let ir = lower_src(": go ( i64 i64 -- i64 ) dup 0 > if 1 - go else drop end ;");
        let f = &ir.funcs[0];
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        assert_eq!(phis.len(), 2, "one header phi per loop-carried slot");
        // Each phi has the entry arm plus the single back-edge arm.
        assert!(phis.iter().all(|arms| arms.len() == 2));
        // Entry + one back-edge both target the header.
        assert_eq!(jmps_to(f, header), 2);
        assert_eq!(
            count(f, is_call_instr),
            0,
            "tail self-call is a back-edge, not a Call"
        );
    }

    /// The header phi structure that matters for R11: how many phis, how many
    /// arms each has, and how many jumps target the header. Deliberately
    /// ignores the carried `Value`s themselves, since those differ between
    /// two independently-lowered programs even when the shape is identical.
    fn header_phi_shape(func: &IrFunc, header: BlockId) -> (usize, Vec<usize>, usize) {
        let phis = header_phis(header_block(func, header));
        let phi_count = phis.len();
        let arm_counts = phis.iter().map(|arms| arms.len()).collect();
        (phi_count, arm_counts, jmps_to(func, header))
    }

    #[test]
    fn lower_mid_body_binding_adds_no_header_phi() {
        // Criterion 22 (R11): a mid-body binding inside a self-tail-recursive
        // arm has its extent end at the arm's terminator, where the back-edge
        // sits, so no name is live across it and the header still carries
        // exactly one phi per loop-carried (input-arity) slot, unaffected by
        // the binding. Proved by comparing against a binding-free equivalent:
        // if a bound name ever leaked a phi onto the header, this source's
        // shape would diverge from the one below instead of both trivially
        // satisfying the same hard-coded numbers.
        let with_binding =
            lower_src(": go ( i64 i64 -- i64 ) dup 0 > if | x | 1 - x go else drop end ;");
        let without_binding =
            lower_src(": go ( i64 i64 -- i64 ) dup 0 > if 1 - go else drop end ;");
        let f1 = &with_binding.funcs[0];
        let f2 = &without_binding.funcs[0];
        let header1 = loop_header(f1);
        let header2 = loop_header(f2);
        let shape1 = header_phi_shape(f1, header1);
        let shape2 = header_phi_shape(f2, header2);
        assert_eq!(
            shape1, shape2,
            "a mid-body binding must not change the header's phi structure"
        );
        assert_eq!(shape1.0, 2, "one header phi per loop-carried slot");
    }

    #[test]
    fn non_tail_self_call_stays_a_call() {
        // R10: a self-call followed by more work (`fact *`) is not in tail
        // position, so it stays a real `Instr::Call` and no loop is built.
        let ir = lower_src(": fact ( i64 -- i64 ) dup 0 = if drop 1 else dup 1 - fact * end ;");
        let f = &ir.funcs[0];
        assert_eq!(
            count(f, is_call_instr),
            1,
            "non-tail self-call stays a real Call"
        );
        assert!(
            !matches!(f.blocks[0].term, Terminator::Jmp(_)),
            "a non-tail-recursive word builds no loop header"
        );
    }

    #[test]
    fn self_call_in_non_terminal_if_stays_a_call() {
        // R10 over-eager boundary: the `if` is followed by more terms
        // (`drop 5`), so it is non-terminal and its arms are not in tail
        // position; the self-call stays a real `Instr::Call`.
        let ir = lower_src(": w ( i64 -- i64 ) dup 0 > if w else drop 0 end drop 5 ;");
        let f = &ir.funcs[0];
        assert_eq!(count(f, is_call_instr), 1);
        assert!(!matches!(f.blocks[0].term, Terminator::Jmp(_)));
    }

    #[test]
    fn both_if_arms_tail_produce_two_back_edges() {
        // R8 multi-arm back-patch through `lower_if`: a self-tail-call in each
        // arm of a terminal `if` back-edges, so the single header phi gains two
        // back-edge arms on top of the entry arm (three total).
        let ir = lower_src(": go ( i64 -- i64 ) dup 0 > if 1 - go else 1 + go end ;");
        let f = &ir.funcs[0];
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        assert_eq!(phis.len(), 1);
        assert_eq!(phis[0].len(), 3, "entry arm + two back-edge arms");
        assert_eq!(jmps_to(f, header), 3);
        assert_eq!(count(f, is_call_instr), 0);
    }

    #[test]
    fn clause_tails_share_one_header() {
        // R9: a `|`-clause self-tail-recursive word gets a single header; each
        // clause's terminal self-call is one back-edge into it. Both clauses
        // here tail-recurse, so each header phi has three arms (entry + two
        // back-edges) and no `Instr::Call` to self remains.
        let ir = lower_src(
            "type: Flag | Go | Stop ; \
             : loop2 ( i64 Flag -- i64 ) | Go 1 - Go loop2 | Stop 1 + Stop loop2 ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "loop2").unwrap();
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        // Slice 9 (R1): `Flag` is zero-payload, so the general scalar-enum
        // rule makes it register-resident -- it never enters the aggregate-
        // staging path, so it keeps a header phi just like the `i64` slot
        // (both scalar): 2 phis, not 1.
        assert_eq!(phis.len(), 2, "both the i64 and the scalar Flag slot phi");
        assert!(phis.iter().all(|arms| arms.len() == 3));
        assert_eq!(jmps_to(f, header), 3, "entry + two clause back-edges");
        assert_eq!(count(f, is_call_instr), 0);
    }

    #[test]
    fn mixed_clause_header_and_join_predecessors_stay_disjoint() {
        // R9 / risk 5: some clauses back-edge and one is a base case that
        // `Ret`s. The loop header phi (preds = entry + tail clause ends) and
        // the Slice-4 dispatch-join phi (preds = non-tail clause ends) must
        // keep disjoint predecessor sets.
        let ir = lower_src(
            "type: Flag | Go | Stop ; \
             : run ( i64 Flag -- i64 ) | Go 1 - Stop run | Stop ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "run").unwrap();
        let header = loop_header(f);
        let hb = header_block(f, header);
        let hphis = header_phis(hb);
        // Slice 9 (R1): `Flag` is zero-payload, hence scalar, hence it also
        // keeps a header phi alongside the `i64` one (2, not 1).
        assert_eq!(hphis.len(), 2);
        // header preds: entry arm + the one Go back-edge.
        assert!(hphis.iter().all(|arms| arms.len() == 2));
        assert!(
            f.blocks
                .iter()
                .any(|b| matches!(b.term, Terminator::Ret(_))),
            "the Stop base case still Rets"
        );
        // Every phi that is not a header phi is a dispatch/join phi; its
        // predecessors must not overlap the header phi's predecessors.
        let header_preds: std::collections::HashSet<u32> = hphis
            .iter()
            .flat_map(|arms| arms.iter().map(|(p, _)| p.0))
            .collect();
        for block in &f.blocks {
            if block.id == header {
                continue;
            }
            for instr in &block.instrs {
                if let Instr::Phi(_, arms) = instr {
                    for (p, _) in arms {
                        assert!(
                            !header_preds.contains(&p.0),
                            "join phi pred {p:?} collides with a header phi pred"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn clause_tail_call_alloc_is_hoisted_to_entry_not_loop_body() {
        // A clause self-tail-call rebuilds its enum scrutinee on every
        // back-edge. `Stop` carries a payload here (Slice 9, R1: a
        // zero-payload variant's construct no longer allocs at all -- it is a
        // bare scalar `Const` -- so this test needs a payload-bearing variant
        // to keep exercising the alloc-hoisting invariant it is named for).
        // If that `Alloc` stayed in the loop body, QBE's `alloc*` would bump
        // the frame pointer every iteration and blow the stack well before
        // Phase 4's N >= 1_000_000 golden. It must land in the entry block
        // instead, so the loop body has none.
        let ir = lower_src(
            "type: Flag | Go | Stop n i64 ; \
             : run ( i64 Flag -- i64 ) | Go 1 - dup Stop run | Stop drop ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "run").unwrap();
        let header = loop_header(f);
        let entry = &f.blocks[0];
        assert!(
            entry.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
            "the Stop scrutinee's alloc should be hoisted into the entry block"
        );
        let entry_id = entry.id;
        for block in &f.blocks {
            if block.id == entry_id || block.id == header {
                continue;
            }
            assert!(
                !block.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
                "block {:?} in the loop body must not alloc",
                block.id
            );
        }
    }

    // Phase 4 Slice 3: the aggregate-staging loop transform (R1-R4, R1a).
    // Structural coverage beside the changed `begin_loop`/`finalize_loop`; the
    // runtime witnesses are the `tests/phase4_generics.rs` goldens.

    /// A self-tail loop carrying an i64 (scalar) and a re-produced `Box`
    /// (aggregate), so the aggregate slot stages rather than forwards.
    const STAGED_LOOP: &str = "type: Box n i64 ;\n\
         : mk ( i64 -- Box ) | n | n Box ;\n\
         : loop ( i64 Box -- Box ) | n b | n 0 = if b else n 1 - n mk loop end ;";

    #[test]
    fn aggregate_carried_slot_gets_no_header_phi_but_scalar_does() {
        // R2: the aggregate (`Box`) slot contributes no header phi (it reads
        // its entry-hoisted stable slot); the scalar (i64) slot keeps one.
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        assert_eq!(
            phis.len(),
            1,
            "only the i64 scalar slot carries a header phi"
        );
        // `len() == 1` alone would also pass a transform that kept the `Box`
        // slot's phi and dropped the scalar's; pin that the survivor carries
        // the i64 counter, not a `Box` pointer, so "but scalar does" is checked.
        let (_, incoming) = phis[0][0];
        assert_eq!(
            f.value_types[incoming.0 as usize],
            IrType::I64,
            "the surviving header phi carries the scalar slot, not the aggregate"
        );
    }

    #[test]
    fn aggregate_stable_slot_and_temp_are_entry_hoisted_not_in_the_body() {
        // R1/R9: the stable slot and staging temp are `alloc`ed in the entry
        // block, not per-iteration in the body (which would bump the frame
        // every iteration and break the constant-stack guarantee). `instrs`
        // flattens across blocks, so this iterates `func.blocks` directly.
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let entry = &f.blocks[0];
        let entry_allocs = entry
            .instrs
            .iter()
            .filter(|i| matches!(i, Instr::Alloc(..)))
            .count();
        assert!(
            entry_allocs >= 2,
            "the stable slot and temp allocs should be hoisted into the entry block, saw {entry_allocs}"
        );
        let entry_id = entry.id;
        for block in &f.blocks {
            if block.id == entry_id || block.id == header {
                continue;
            }
            assert!(
                !block.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
                "block {:?} in the loop body must not alloc",
                block.id
            );
        }
    }

    #[test]
    fn aggregate_init_blit_lands_in_the_entry_block() {
        // R3: `begin_loop` seeds the stable slot with the incoming param once,
        // in the entry block, so iteration 1 reads an initialised value. It is
        // the only Blit routed to the entry block (the back-edge staging blits
        // go to predecessor blocks).
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let entry = &f.blocks[0];
        assert!(
            entry.instrs.iter().any(|i| matches!(i, Instr::Blit(..))),
            "the entry-arm init blit should land in the entry block"
        );
    }

    /// The back-edge predecessor block of a self-tail loop: the non-entry block
    /// that jumps to the header.
    fn back_edge_pred(f: &IrFunc, header: BlockId) -> &Block {
        let entry_id = f.blocks[0].id;
        f.blocks
            .iter()
            .find(|b| b.id != entry_id && matches!(b.term, Terminator::Jmp(h) if h == header))
            .expect("a back-edge predecessor block")
    }

    #[test]
    fn back_edge_stages_reads_before_writes() {
        // R4: on a staged back-edge, every read-phase blit (a snapshot into a
        // temp) precedes every write-phase blit (a store into the stable slot).
        // A blit is write-phase when its source is an earlier blit's dest in
        // the same predecessor block. `instrs` flattens across blocks, so this
        // inspects the predecessor block directly.
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let pred = back_edge_pred(f, header);
        let mut written: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let mut seen_write = false;
        let mut blits = 0;
        for instr in &pred.instrs {
            if let Instr::Blit(src, dst, _) = instr {
                blits += 1;
                if written.contains(&src.0) {
                    seen_write = true;
                } else {
                    assert!(!seen_write, "a read-phase blit follows a write-phase blit");
                }
                written.insert(dst.0);
            }
        }
        assert!(
            blits >= 2,
            "the staged Box back-edge should emit a read and a write blit, saw {blits}"
        );
    }

    #[test]
    fn forwarded_in_place_aggregate_slot_emits_zero_back_edge_blits() {
        // R4: an aggregate carried unchanged (`prev`, its back-edge arg is
        // exactly its own stable slot) is forwarded in place and stages
        // nothing.
        let ir = lower_src(
            "type: Box n i64 ;\n\
             : mk ( i64 -- Box ) | n | n Box ;\n\
             : loop ( i64 Box -- Box ) | n prev | n 0 = if prev else n 1 - prev loop end ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let pred = back_edge_pred(f, header);
        assert_eq!(
            pred.instrs
                .iter()
                .filter(|i| matches!(i, Instr::Blit(..)))
                .count(),
            0,
            "a forwarded-in-place slot emits zero back-edge blits"
        );
    }

    #[test]
    fn recursive_type_destructor_is_not_transformed() {
        // R1a: the fused iterative destructor's `begin_loop` is gated OFF, so a
        // recursive type's synthesized destructor keeps its one header phi for
        // the carried node (R2 would drop it to zero) and gains no entry-block
        // init Blit (R3's blit is the only Blit the transform routes to the
        // entry block; the destructor's own copy-out lands in a body block).
        // This is the check that is red when the gate is missing.
        let ir = lower_src(
            "type: Res n i64 ;\n\
             : drop ( Res -- ) | r | r Res>n 5000 + . ;\n\
             : mkres ( i64 -- Res ) | n | n Res ;\n\
             type: List | Nil | Cons v Res next ^List ;\n\
             : w ( -- ) ;",
        );
        // Slice 9 (R2): `bool` occupies the reserved `EnumId(0)`, so `List`
        // (this source's only other enum) lands at `EnumId(1)`.
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_enum_drop_1")
            .expect("a fused destructor was synthesized for the recursive enum");
        let header = loop_header(dtor);
        let phis = header_phis(header_block(dtor, header));
        assert_eq!(
            phis.len(),
            1,
            "the ungated-off destructor keeps its one carried-node header phi"
        );
        let entry = &dtor.blocks[0];
        assert!(
            !entry.instrs.iter().any(|i| matches!(i, Instr::Blit(..))),
            "the destructor gains no entry-block init blit (R1a gate holds)"
        );
    }

    // Phase 3 Slice 1: the drop-spy's lowering (R5/R6/R16).

    #[test]
    fn lower_struct_constructor_emits_no_call_only_alloc_and_store() {
        // Constructing a linear struct value is inlined alloc + field
        // stores, not a runtime call: only `drop`'s own destructor call is
        // emitted.
        let ir = lower_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy drop ;"));
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let is = instrs(w);
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(
            count(
                w,
                |i| matches!(i, Instr::Call(_, sym, _) if sym != &spy_drop)
            ),
            0,
            "the constructor emits no call: {is:?}"
        );
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 1, "{is:?}");
        assert_eq!(
            count(w, |i| matches!(i, Instr::FieldStore(..))),
            1,
            "{is:?}"
        );
    }

    #[test]
    fn lower_drop_of_linear_value_calls_the_destructor() {
        let ir = lower_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy drop ;"));
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let calls: Vec<&String> = instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, args) if args.len() == 1 => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(
            calls,
            vec![spy_drop.as_str()],
            "expected one destructor call"
        );
    }

    #[test]
    fn lower_drop_of_copy_value_emits_no_destructor_call() {
        // R2: `drop` on a Copy value keeps its no-runtime-effect discard.
        let ir = lower_src(": w ( -- ) 7 drop ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, is_call_instr), 0);
    }

    // Phase 3 Slice 1, Phase 2: struct linearity + the synthesized destructor
    // (R7/R9/R11/R12).

    #[test]
    fn struct_layout_is_linear_iff_a_field_is_transitively() {
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Plain x i64 y i64 ; \
             type: Holds a Spy b i64 ; \
             type: Wraps h Holds ; \
             : w ( -- ) ;"
        ));
        assert!(ir.structs[0].is_linear, "Spy has a drop overload");
        assert!(!ir.structs[1].is_linear, "Plain has no linear field");
        assert!(ir.structs[2].is_linear, "Holds carries a Spy directly");
        assert!(ir.structs[3].is_linear, "Wraps carries one transitively");
    }

    #[test]
    fn struct_with_owned_cell_field_is_linear_and_pointer_sized() {
        // R4/R17: a cell is linear whatever its payload, so a struct holding one
        // is linear and gets drop glue; its field is a pointer, sized by the
        // same convention as `Ptr` rather than a second width assumption.
        let ir = lower_src("type: Boxed b ^i64 ; : w ( -- ) ;");
        let layout = &ir.structs[0];
        assert!(layout.is_linear, "a cell field makes its struct linear");
        assert_eq!((layout.size, layout.align), (8, 8));
        assert!(
            matches!(layout.fields[0].ty, IrType::OwnedCell(_)),
            "a cell field keeps its own `IrType`, not a bare `Ptr`: {:?}",
            layout.fields[0].ty
        );
        assert_eq!(scalar_size_align(layout.fields[0].ty), (8, 8));
    }

    #[test]
    fn lower_owned_cell_unwrap_scalar_loads_before_freeing() {
        // R13: `^>` must materialise the payload before calling `sooth_free`,
        // so the freed pointer is never handed to the stack.
        let ir = lower_src(": w ( -- i64 ) 5 ^ ^> ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        let load_at = is
            .iter()
            .position(|i| matches!(i, Instr::FieldLoad(..)))
            .expect("a FieldLoad");
        let free_at = is
            .iter()
            .position(|i| matches!(i, Instr::Call(None, sym, _) if sym == FREE_SYMBOL))
            .expect("a free call");
        assert!(
            load_at < free_at,
            "scalar payload must load before the cell frees: load at {load_at}, free at {free_at}"
        );
    }

    #[test]
    fn lower_owned_cell_unwrap_aggregate_blits_before_freeing() {
        // The aggregate counterpart of the scalar case above (R13): the copy-out
        // `Blit` must precede `sooth_free`, never aliasing the freed cell.
        let ir = lower_src("type: Point x i64 y i64 ; : w ( -- Point ) 1 2 Point ^ ^> ;");
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let is = instrs(w);
        let blit_at = is
            .iter()
            .position(|i| matches!(i, Instr::Blit(..)))
            .expect("a Blit");
        let free_at = is
            .iter()
            .position(|i| matches!(i, Instr::Call(None, sym, _) if sym == FREE_SYMBOL))
            .expect("a free call");
        assert!(
            blit_at < free_at,
            "aggregate payload must blit out before the cell frees: blit at {blit_at}, free at {free_at}"
        );
    }

    #[test]
    fn struct_linearity_agrees_across_the_checker_and_both_lowering_folds() {
        // Linearity is decided in three places over the same field lists:
        // `check::is_copy` walks `Type`, `ensure_struct` folds `IrType` inline
        // while `layouts` is still being built, and `field_is_linear` is what
        // every drop-glue site consults. If they ever disagree the checker
        // gates a `dup` the lowering then emits no glue for (or the reverse),
        // so pin all three rather than trusting three hand-kept matches.
        let src = format!(
            "{SPY_DEF}type: Plain x i64 y i64 ; \
                   type: Holds a Spy b i64 ; \
                   type: Wraps h Holds ; \
                   type: Deep w Wraps p Plain ; \
                   type: Item | Empty | Full v Spy ; \
                   type: EnumInStruct e Item ; \
                   type: StructInEnum | Some h Holds | None ; \
                   type: EnumInEnum | Inner i EnumInStruct | Outer ; \
                   type: PlainArr xs [i64 4] ; \
                   type: Boxed b ^i64 ; \
                   type: BoxedPlain p ^Plain ; \
                   type: MaybeBoxed | Full b ^i64 | Empty ; \
                   : w ( -- ) ;"
        );
        let tokens = lex(&src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        // `SpyArr` (a `[Spy 4]` field) is spliced in directly rather than
        // through source: Item 1's array-type-use rejection means no source
        // program can spell this declaration any more, but the predicate
        // must still be correct on the type alone. Reuses the real `Spy`
        // struct from `SPY_DEF` (already `has_drop_overload`, set by `check`
        // above) rather than hand-building a fixture, since `SPY_DEF` is
        // always prepended first and so is always struct index 0.
        let spy_id = StructId::from_index(0);
        let spy_name_static = module.structs[spy_id.index()].name_static;
        let spy_ty = Type::Struct(spy_id, spy_name_static);
        let spy_array_id = ArrayId::from_index(module.arrays.len());
        let spy_array_name: &'static str = "[Spy 4]";
        module.arrays.push(ArrayDecl {
            element: spy_ty,
            count: 4,
            name_static: spy_array_name,
        });
        module.structs.push(StructDecl {
            name: "SpyArr".to_string(),
            name_static: "SpyArr",
            fields: vec![("xs".to_string(), Type::Array(spy_array_id, spy_array_name))],
            span: crate::ast::Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        });
        let (structs, enums, arrays, ..) = build_registries(
            &module.structs,
            &module.enums,
            &module.arrays,
            &module.owned_cells,
            &module.refs,
        );
        for (idx, layout) in structs.layouts.iter().enumerate() {
            let ty = Type::Struct(StructId::from_index(idx), layout.name);
            assert_eq!(
                crate::check::is_copy(ty, &module.structs, &module.enums, &module.arrays),
                !layout.is_linear,
                "`{}`: checker says Copy={}, `ensure_struct` says linear={}",
                layout.name,
                crate::check::is_copy(ty, &module.structs, &module.enums, &module.arrays),
                layout.is_linear
            );
            // `Spy` itself is excluded here: it is linear purely because of
            // its `has_drop_overload` bit (an override on all-Copy fields),
            // not because any field is `field_is_linear`, a distinct case
            // already pinned by
            // `ir_registers_overridden_struct_as_linear_despite_all_copy_fields`.
            if idx != spy_id.index() {
                assert_eq!(
                    layout
                        .fields
                        .iter()
                        .any(|f| field_is_linear(f.ty, &structs, &enums, &arrays)),
                    layout.is_linear,
                    "`{}`: `field_is_linear` disagrees with the `ensure_struct` fold",
                    layout.name
                );
            }
        }
        // R7/R12 (Phase 4): the same three-way pin, over the enum registry's
        // `Type::Enum` arm of `is_copy` and the variant-payload fold
        // (`ensure_enum`/`layout_field_is_linear`), including transitivity
        // through a struct-in-enum and an enum-in-enum.
        for (idx, layout) in enums.layouts.iter().enumerate() {
            let ty = Type::Enum(EnumId::from_index(idx), layout.name);
            assert_eq!(
                crate::check::is_copy(ty, &module.structs, &module.enums, &module.arrays),
                !layout.is_linear,
                "`{}`: checker says Copy={}, `ensure_enum` says linear={}",
                layout.name,
                crate::check::is_copy(ty, &module.structs, &module.enums, &module.arrays),
                layout.is_linear
            );
            assert_eq!(
                layout
                    .variants
                    .iter()
                    .flat_map(|v| v.fields.iter())
                    .any(|f| field_is_linear(f.ty, &structs, &enums, &arrays)),
                layout.is_linear,
                "`{}`: `field_is_linear` disagrees with the `ensure_enum` fold",
                layout.name
            );
        }
        // Criterion (item 3): an array field is linear iff its element is,
        // transitively; `PlainArr` (an `[i64 4]` field) stays Copy, `SpyArr`
        // (a `[Spy 4]` field, spliced in above) is linear even though no
        // source program can declare that field any more, so the predicate
        // must be correct on the type alone.
        let plain_arr_idx = structs
            .layouts
            .iter()
            .position(|l| l.name == "PlainArr")
            .unwrap();
        let spy_arr_idx = structs
            .layouts
            .iter()
            .position(|l| l.name == "SpyArr")
            .unwrap();
        assert!(!structs.layouts[plain_arr_idx].is_linear);
        assert!(structs.layouts[spy_arr_idx].is_linear);
        let plain_arr_ty = Type::Struct(
            StructId::from_index(plain_arr_idx),
            structs.layouts[plain_arr_idx].name,
        );
        let spy_arr_ty = Type::Struct(
            StructId::from_index(spy_arr_idx),
            structs.layouts[spy_arr_idx].name,
        );
        assert!(crate::check::is_copy(
            plain_arr_ty,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        assert!(!crate::check::is_copy(
            spy_arr_ty,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
    }

    #[test]
    fn lower_appends_one_destructor_func_per_linear_struct_only() {
        // R12: a synthesized destructor exists for every linear struct type,
        // and only those (a Copy struct needs no glue, so gets no function).
        // `Plain` (index 1, Copy) gets no destructor; `Holds` (index 2,
        // linear) does.
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Plain x i64 y i64 ; \
             type: Holds a Spy b i64 ; \
             : w ( -- ) ;"
        ));
        assert!(ir.funcs.iter().any(|f| f.name == "sooth_struct_drop_2"));
        assert!(!ir.funcs.iter().any(|f| f.name == "sooth_struct_drop_1"));
    }

    #[test]
    fn lower_drop_of_whole_linear_struct_calls_its_synthesized_destructor() {
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Holds a Spy b i64 ; : w ( -- ) 1 Spy 2 Holds drop ;"
        ));
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let calls: Vec<&String> = instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, args) if args.len() == 1 => Some(sym),
                _ => None,
            })
            .collect();
        let holds_drop = struct_drop_symbol(StructId::from_index(1), None);
        assert_eq!(calls, vec![holds_drop.as_str()]);
    }

    #[test]
    fn synthesized_struct_destructor_drops_linear_fields_in_declaration_order() {
        // R12: struct -> drop its linear fields in declaration order. `Holds`
        // has a linear field (`a`) then a Copy one (`b`), so the destructor
        // calls `Spy`'s destructor exactly once, for `a`.
        let ir = lower_src(&format!("{SPY_DEF}type: Holds a Spy b i64 ; : w ( -- ) ;"));
        let holds_drop = struct_drop_symbol(StructId::from_index(1), None);
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == holds_drop)
            .expect("a destructor was synthesized for the linear struct");
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(calls, vec![spy_drop.as_str()]);
    }

    #[test]
    fn lower_appends_a_destructor_func_for_every_cell_even_a_copy_payload() {
        // R8: unlike the struct/enum filters above, *every* cell gets a
        // destructor, because `drop` on a cell must free it whatever its
        // payload is. `^i64`'s payload is Copy and it still gets one.
        let ir = lower_src(": w ( -- ) 5 ^ drop ;");
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_cell_drop_0")
            .expect("a Copy-payload cell still gets a destructor");
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            vec![FREE_SYMBOL],
            "a Copy payload frees and nothing else"
        );
    }

    #[test]
    fn synthesized_cell_destructor_frees_before_dropping_a_linear_aggregate_payload() {
        // An aggregate payload is copied out of the cell (a Blit), then
        // the block is freed, and only then does the copy's own drop
        // glue run. The `^Spy` golden covers the scalar payload at
        // runtime; this pins the aggregate path, where the copy-out must
        // still complete before anything else touches the block or the copy.
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Holds a Spy b i64 ; : w ( -- ) 1 Spy 2 Holds ^ drop ;"
        ));
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_cell_drop_0")
            .expect("a destructor was synthesized for the cell");
        let is = instrs(dtor);
        let blit_at = is
            .iter()
            .position(|i| matches!(i, Instr::Blit(..)))
            .expect("a copy-out Blit");
        let calls: Vec<(usize, &String)> = is
            .iter()
            .enumerate()
            .filter_map(|(at, i)| match i {
                Instr::Call(None, sym, _) => Some((at, sym)),
                _ => None,
            })
            .collect();
        let holds_drop = struct_drop_symbol(StructId::from_index(1), None);
        assert_eq!(
            calls
                .iter()
                .map(|(_, sym)| sym.as_str())
                .collect::<Vec<_>>(),
            vec![FREE_SYMBOL, holds_drop.as_str()],
            "the cell frees, then the payload's own destructor runs"
        );
        assert!(
            blit_at < calls[0].0,
            "the payload must be copied out before the block is freed: blit at {blit_at}, free at {}",
            calls[0].0
        );
    }

    // Phase 3 Slice 1, Phase 4: the synthesized enum destructor's own tag
    // dispatch (structural, not full-stdout: `tests/phase0.rs` covers the
    // 2-variant runtime behavior; these pin the shapes it doesn't reach).

    #[test]
    fn synthesized_enum_destructor_newtype_skips_the_tag_compare() {
        // R7/R12: a single-variant enum (n == 1) has nothing to dispatch on,
        // so the destructor jumps straight to the one variant block instead
        // of loading a tag and comparing it (the `n == 1` branch of
        // `dispatch_on_tag`, otherwise unreached by the 2-variant goldens).
        let ir = lower_src(&format!("{SPY_DEF}type: Box | Full v Spy ; : w ( -- ) ;"));
        // Slice 9 (R2): `bool` occupies the reserved `EnumId(0)`, so `Box`
        // lands at `EnumId(1)`.
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_enum_drop_1")
            .expect("a destructor was synthesized for the linear enum");
        assert_eq!(count(dtor, |i| matches!(i, Instr::Cmp(..))), 0);
        assert_eq!(
            dtor.blocks.len(),
            2,
            "a bare `Jmp` to the one variant block, no compare block"
        );
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(calls, vec![spy_drop.as_str()]);
    }

    #[test]
    fn synthesized_enum_destructor_three_variants_chains_through_a_middle_block() {
        // R7/R12: with 3 variants the compare chain has an intermediate block
        // between the first and last compare (`vi < n - 2` in
        // `dispatch_on_tag`), never built by the 2-variant goldens. Each of
        // the 3 variants gets its own block; only `Full`'s carries a drop.
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Item | Empty | Full v Spy | Named n i64 ; : w ( -- ) ;"
        ));
        // Slice 9 (R2): `bool` occupies the reserved `EnumId(0)`, so `Item`
        // lands at `EnumId(1)`.
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_enum_drop_1")
            .expect("a destructor was synthesized for the linear enum");
        assert_eq!(dtor.blocks.len(), 5, "2 compares + 3 variant blocks");
        assert_eq!(count(dtor, |i| matches!(i, Instr::Cmp(..))), 2);
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(calls, vec![spy_drop.as_str()]);
    }

    // Unit-level coverage of `recursive_disposal_path`'s path-finding: which
    // steps it finds for a shape, distinct from the runtime goldens in
    // tests/phase0.rs that prove those shapes actually dispose correctly.

    #[test]
    fn recursive_disposal_path_finds_indirect_nested_mutual_and_composed_cycles() {
        // The wrapper-struct list: the cell is one byval struct hop away from
        // the enum that owns it, so the path is a tag dispatch, a projection
        // into `Wrap`, then the unwrap.
        let p = Probe::new(
            "type: Wrap v i64 next ^List ;\n\
             type: List | Nil | Cons w Wrap ;\n\
             : main ( -- ) ;",
        );
        let list = p.enum_ty("List");
        assert_eq!(
            p.path(list),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(1),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Project { field: 0 },
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(list),
                        },
                    ]),
                ],
            }])
        );
        // The same cycle rooted at `Wrap` instead: one rotation of it, the
        // dispatch now mid-path (every type on the cycle gets its own
        // loop, entered from its own shape).
        assert_eq!(
            p.path(p.struct_ty("Wrap")),
            Some(vec![
                PathStep::Unwrap {
                    field: Some(1),
                    cell: p.cell(list),
                },
                PathStep::Branch {
                    enum_id: EnumId::from_index(1),
                    variants: vec![None, Some(vec![PathStep::Project { field: 0 }])],
                },
            ])
        );

        // `^^Self`: the outer unwrap names the field, the inner one cannot
        // (the current type *is* the cell at that point).
        let p = Probe::new(
            "type: L | Nil | Cons n i64 next ^^L ;\n\
             : main ( -- ) ;",
        );
        let l = p.enum_ty("L");
        let inner = p.cell(l);
        assert_eq!(
            p.path(l),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(1),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(IrType::OwnedCell(inner)),
                        },
                        PathStep::Unwrap {
                            field: None,
                            cell: inner,
                        },
                    ]),
                ],
            }])
        );

        // The mutual A/B chain, from both directions: `A` dispatches at entry,
        // `B` (a plain struct, no tag of its own) dispatches mid-path.
        let p = Probe::new(
            "type: A | ANil | ACons x i64 next ^B ;\n\
             type: B y i64 z ^A ;\n\
             : main ( -- ) ;",
        );
        let (a, b) = (p.enum_ty("A"), p.struct_ty("B"));
        assert_eq!(
            p.path(a),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(1),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(b),
                        },
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(a),
                        },
                    ]),
                ],
            }])
        );
        assert_eq!(
            p.path(b),
            Some(vec![
                PathStep::Unwrap {
                    field: Some(1),
                    cell: p.cell(a),
                },
                PathStep::Branch {
                    enum_id: EnumId::from_index(1),
                    variants: vec![
                        None,
                        Some(vec![PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(b),
                        }]),
                    ],
                },
            ])
        );

        // Composition: a wrapper struct sitting inside a two-type cycle, so
        // one path threads three unwraps through three distinct types.
        let p = Probe::new(
            "type: P q ^W ;\n\
             type: W m i64 next ^Q ;\n\
             type: Q r ^P ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(
            p.path(p.struct_ty("P")),
            Some(vec![
                PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(p.struct_ty("W")),
                },
                PathStep::Unwrap {
                    field: Some(1),
                    cell: p.cell(p.struct_ty("Q")),
                },
                PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(p.struct_ty("P")),
                },
            ])
        );
    }

    #[test]
    fn recursive_disposal_path_finds_multi_variant_and_enum_enum_mutual_cycles() {
        // Two independently recursive variants: both continue, because an
        // enum's variants are mutually exclusive at runtime and so are not
        // the simultaneously-live branching case a struct's own field choice
        // must narrow. Collapsing to one would regress a program that
        // already disposes in constant stack.
        let p = Probe::new(
            "type: T | Nil | X n i64 next ^T | Y m i64 next ^T ;\n\
             : main ( -- ) ;",
        );
        let t = p.enum_ty("T");
        let step = vec![PathStep::Unwrap {
            field: Some(1),
            cell: p.cell(t),
        }];
        assert_eq!(
            p.path(t),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(1),
                variants: vec![None, Some(step.clone()), Some(step)],
            }])
        );

        // The enum/enum mutual pair: two nested `Branch` steps, the inner one
        // dispatched partway along the path rather than at the entry.
        let p = Probe::new(
            "type: A | ANil | ACons x i64 next ^B ;\n\
             type: B | BNil | BCons y i64 next ^A ;\n\
             : main ( -- ) ;",
        );
        let (a, b) = (p.enum_ty("A"), p.enum_ty("B"));
        assert_eq!(
            p.path(a),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(1),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(b),
                        },
                        PathStep::Branch {
                            enum_id: EnumId::from_index(2),
                            variants: vec![
                                None,
                                Some(vec![PathStep::Unwrap {
                                    field: Some(1),
                                    cell: p.cell(a),
                                }]),
                            ],
                        },
                    ]),
                ],
            }])
        );
    }

    #[test]
    fn recursive_disposal_path_rejects_non_cyclic_and_misleading_shapes() {
        // No cell at all: nothing to walk.
        let p = Probe::new(&format!(
            "{SPY_DEF}type: Plain x i64 y Spy ;\n: main ( -- ) ;"
        ));
        assert_eq!(p.path(p.struct_ty("Plain")), None);

        // The bait is the *last* field, which is where the reverse-order scan
        // starts, and the genuine edge is indirect, so the direct-field tier
        // cannot short-circuit past it: the scan must try `bait`, walk into
        // `Bait` and `Leafy`, fail, and back up to `good`. A greedy search
        // that committed to the first cell field it saw would return `None`.
        let p = Probe::new(
            "type: Leafy v i64 ;\n\
             type: Bait c ^Leafy ;\n\
             type: Hop n ^Node ;\n\
             type: Node good Hop bait ^Bait ;\n\
             : main ( -- ) ;",
        );
        let node = p.struct_ty("Node");
        assert_eq!(
            p.path(node),
            Some(vec![
                PathStep::Project { field: 0 },
                PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(node),
                },
            ])
        );

        // `^^Other`: the walk does step through the inner cell (that is how
        // `^^Self` is found at all), and still bottoms out in a dead end.
        let p = Probe::new(
            "type: Other v i64 ;\n\
             type: Twice c ^^Other ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(p.path(p.struct_ty("Twice")), None);

        // Two unrelated self-recursive types: each finds its own edge and
        // neither path wanders into the other type.
        let p = Probe::new(
            "type: R1 n ^R1 ;\n\
             type: R2 n ^R2 ;\n\
             : main ( -- ) ;",
        );
        for name in ["R1", "R2"] {
            let ty = p.struct_ty(name);
            assert_eq!(
                p.path(ty),
                Some(vec![PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(ty),
                }])
            );
        }
    }

    #[test]
    fn recursive_disposal_path_prefers_direct_field_over_later_indirect_one() {
        // `a` is a direct `^Self` field; `b` is declared after it and also
        // reaches `Self`, but only by way of `Wrap`'s own cell field. Without
        // a preferred tier for direct fields, the reverse scan tries `b`
        // first and finds it succeeds, silently swapping in the longer route
        // and lengthening every iteration of the fused loop.
        let p = Probe::new(
            "type: Wrap v i64 n ^List ;\n\
             type: List a ^List b Wrap ;\n\
             : main ( -- ) ;",
        );
        let list = p.struct_ty("List");
        assert_eq!(
            p.path(list),
            Some(vec![PathStep::Unwrap {
                field: Some(0),
                cell: p.cell(list),
            }])
        );

        // The same trap one level up, between an enum's variants: each
        // variant picks its own edge independently, `Direct`'s direct one and
        // `Indirect`'s route through `Wrap`.
        let p = Probe::new(
            "type: Wrap v i64 n ^E ;\n\
             type: E | Nil | Direct d ^E | Indirect w Wrap ;\n\
             : main ( -- ) ;",
        );
        let e = p.enum_ty("E");
        assert_eq!(
            p.path(e),
            Some(vec![PathStep::Branch {
                enum_id: EnumId::from_index(1),
                variants: vec![
                    None,
                    Some(vec![PathStep::Unwrap {
                        field: Some(0),
                        cell: p.cell(e),
                    }]),
                    Some(vec![
                        PathStep::Project { field: 0 },
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(e),
                        },
                    ]),
                ],
            }])
        );
    }

    #[test]
    fn quotation_taking_word_emits_no_call_and_no_irfunc() {
        // Criterion 3b/R20: a monomorphic quotation-taking word is inlined, so
        // it mints no `IrFunc` and its caller emits no `Instr::Call`. The
        // lowered `main` is just `1 +` (the spliced literal over `3`), a pure
        // arithmetic body. Deleting the `combinator_indices` filter would put
        // an `apply` func back, and deleting the `lower_call` inline branch
        // would leave an `Instr::Call apply` in `main`.
        let ir = lower_src(
            ": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
             : main ( -- ) 3 [ 1 + ] apply . ;\n",
        );
        assert!(
            ir.funcs.iter().all(|f| f.name != "apply"),
            "a combinator mints no `IrFunc`, but one named `apply` was emitted"
        );
        let main = ir
            .funcs
            .iter()
            .find(|f| f.name == "main")
            .expect("`main` is emitted");
        assert!(
            call_symbols(main).is_empty(),
            "the inlined caller emits no `Instr::Call`, got: {:?}",
            call_symbols(main)
        );
    }

    #[test]
    fn abstract_forward_inlines_transitively_with_no_call() {
        // Criterion 10b (R21): transitive inlining. `outer` forwards its own
        // abstract quotation parameter to `inner`, so splicing `outer` into
        // `main` must in turn splice `inner` -- two levels, outermost-first.
        // The spec names this `map`-over-`each`. The shipped library keeps
        // `map`/`fold` as leaf combinators on cost grounds rather than scope
        // ones (building them on `each` is expressible, but inlining is total,
        // so composition depth is code size at every call site), so this
        // two-combinator chain stands in for that shape. It exercises the same
        // load-bearing property the criterion guards:
        // both combinators mint no `IrFunc` and `main` emits no `Instr::Call`.
        // Breaking the transitive splice (the `lower_call` combinator branch,
        // or the checker's abstract-forward accept) leaves an `Instr::Call`
        // for `inner` behind.
        let ir = lower_src(
            ": inner ( i64 [ i64 -- ] -- ) call ;\n\
             : outer ( i64 [ i64 -- ] -- ) inner ;\n\
             : main ( -- ) 7 [ 1 + . ] outer ;\n",
        );
        assert!(
            ir.funcs
                .iter()
                .all(|f| f.name != "inner" && f.name != "outer"),
            "both combinators are inlined and mint no `IrFunc`, got: {:?}",
            ir.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let main = ir
            .funcs
            .iter()
            .find(|f| f.name == "main")
            .expect("`main` is emitted");
        assert!(
            call_symbols(main).is_empty(),
            "transitive inlining leaves no `Instr::Call` in `main`, got: {:?}",
            call_symbols(main)
        );
    }

    #[test]
    fn each_lowers_to_a_loop_not_a_per_element_call() {
        // Criterion 14b (R19, load-bearing): the inlined `each` lowers to a
        // real loop -- an entry `Jmp` to a header carrying the index `Phi`,
        // sealed with a `Jnz`, reached by a back-edge `Jmp` -- with no
        // per-element `Instr::Call` (the element quotation is spliced, not
        // called). This is the *structural* constant-stack guarantee behind
        // criterion 14's equivalence witness: deleting the `lower_call` inline
        // branch would leave an `Instr::Call` for `each` and no loop, and
        // unrolling per element would drop the back-edge. `each` is defined
        // inline here so the unit needs no import closure.
        let ir = lower_src(
            ": each ( ['T 'N] [ 'T -- ] -- )\n\
             | f | len >i64 | count | | arr |\n\
             count [ | i | &arr i >usize &> @ f call ] times\n\
             arr drop ;\n\
             : main ( -- ) 0 4 fill [ . ] each ;\n",
        );
        assert!(
            ir.funcs.iter().all(|f| f.name != "each"),
            "the inlined `each` mints no IrFunc, got: {:?}",
            ir.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let main = func(&ir, "main");
        let header = loop_header(main);
        let hblock = header_block(main, header);
        assert!(
            !header_phis(hblock).is_empty(),
            "the header carries the index phi"
        );
        assert!(
            matches!(hblock.term, Terminator::Jnz(..)),
            "the header is sealed with a Jnz (index < count), got {:?}",
            hblock.term
        );
        let entry_id = main.blocks[0].id;
        assert!(
            main.blocks
                .iter()
                .any(|b| b.id != entry_id && matches!(b.term, Terminator::Jmp(h) if h == header)),
            "a non-entry body block back-edges to the header"
        );
        // The array read `&arr i &>` emits the mandatory `sooth_oob_trap`
        // bounds-check call (the hand-threaded twin emits it too); it is not a
        // per-element call to the combinator or its element quotation, so it is
        // excluded. What must be absent is any call to `each` or a spliced
        // element op: the loop body is the spliced literal, not a call.
        let user_calls: Vec<&str> = call_symbols(main)
            .into_iter()
            .filter(|s| *s != "sooth_oob_trap")
            .collect();
        assert!(
            user_calls.is_empty(),
            "the inlined `each` body is spliced, not called; unexpected calls: {user_calls:?}"
        );
    }

    #[test]
    fn while_lowers_to_a_back_edge_not_an_infinite_splice() {
        // U12 (R10, load-bearing): a self-tail combinator `while` lowers to a
        // real mid-body loop -- an entry `Jmp` to a header carrying the state
        // `Phi`, reached by a back-edge `Jmp` -- with no `Instr::Call` to
        // `while` and no re-splice. Deleting the back-edge branch in
        // `lower_call` would leave an `Instr::Call` to `while` (or splice the
        // body forever), not silently pass. `while` is defined inline so the
        // unit needs no import closure.
        let ir = lower_src(
            ": while ( 'a [ 'a -- 'a bool ] -- 'a ) | p | p call if p while else end ;\n\
             : main ( -- ) 0 [ dup 5 < if 1 + true else false end ] while . ;\n",
        );
        assert!(
            ir.funcs.iter().all(|f| f.name != "while"),
            "the inlined `while` mints no IrFunc, got: {:?}",
            ir.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let main = func(&ir, "main");
        let header = loop_header(main);
        let hblock = header_block(main, header);
        assert!(
            !header_phis(hblock).is_empty(),
            "the header carries the state phi"
        );
        let entry_id = main.blocks[0].id;
        assert!(
            main.blocks
                .iter()
                .any(|b| b.id != entry_id && matches!(b.term, Terminator::Jmp(h) if h == header)),
            "a non-entry body block back-edges to the header"
        );
        assert!(
            call_symbols(main).is_empty(),
            "the `while` body is spliced with a back-edge, not called; unexpected calls: {:?}",
            call_symbols(main)
        );
    }

    #[test]
    fn quotation_type_never_reaches_mangling() {
        // R7's `subst_polytype` `unreachable!` arm is only sound because R20's
        // lowering filter keeps a quotation type away from mangling. This
        // asserts the arm *is* the guard: it panics on a quotation, so
        // replacing the `unreachable!` with a real mapping (a silent accept)
        // flips this test from panic to value and fails it. (Slice 7a lifts
        // the sibling `ir_type_of` guard, which now maps a quotation to a
        // runtime value; see `ir_type_of_quotation_is_two_slot_aggregate`.)
        use crate::ast::PolyType;
        let poly_quot = PolyType::Quotation(
            vec![PolyType::Concrete(Type::I64)],
            Vec::new(),
            false,
            None,
            None,
        );
        let subst = Subst::default();
        assert!(
            std::panic::catch_unwind(|| subst_polytype(&poly_quot, &subst, &[])).is_err(),
            "`subst_polytype` on a quotation must hit the R7 `unreachable!` arm"
        );
    }

    #[test]
    fn ir_type_of_quotation_is_two_slot_aggregate() {
        // T-irtype (R2/R3): a quotation type maps to a runtime value ---
        // `IrType::Quotation` naming its effect --- with a fixed two-slot
        // `{ code@0, env@WORD_WIDTH }` layout: size `2*WORD_WIDTH`, align
        // `WORD_WIDTH`, every figure word-width-derived, not a hardcoded
        // 16/8. The carried effect gives value equality, so two structurally
        // equal effects share one `IrType`.
        use crate::ast::quotation_type;
        let ir = ir_type_of(quotation_type(vec![Type::I64], vec![Type::I64]));
        assert!(
            matches!(ir, IrType::Quotation(_)),
            "a quotation type maps to `IrType::Quotation`, got {ir:?}"
        );
        assert_eq!(
            ir,
            ir_type_of(quotation_type(vec![Type::I64], vec![Type::I64])),
            "structurally equal effects are one `IrType`"
        );
        assert_ne!(
            ir,
            ir_type_of(quotation_type(vec![Type::I64], vec![Type::BOOL])),
            "structurally different effects are distinct `IrType`s"
        );
        let layout = quotation_layout(WORD_WIDTH);
        assert_eq!(layout.code_offset, 0, "code slot at offset 0");
        assert_eq!(
            layout.env_offset, WORD_WIDTH,
            "env slot at offset WORD_WIDTH"
        );
        assert_eq!(layout.size, 2 * WORD_WIDTH, "two word-width slots");
        assert_eq!(layout.align, WORD_WIDTH);
    }
}
