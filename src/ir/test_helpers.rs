//! Shared test helpers for the `ir` module tree.

use super::destructors::recursive_disposal_path;
use super::*;
use crate::ast::Line;
use crate::check::check;
use crate::lexer::lex;
use crate::parser::{parse, parse_line};

pub(super) fn lower_src(src: &str) -> IrModule {
    let tokens = lex(src).unwrap();
    let mut module = parse(&tokens).unwrap();
    check(&mut module).unwrap();
    lower(&module).unwrap()
}

/// A scalar-only resource with a `drop` overload whose body has one
/// observable effect (a `Print` no synthesized glue ever emits), so "the
/// override is the destructor" is assertable on instructions.
pub(super) const FILE_RESOURCE: &str = "type: File fd i64 ; : drop ( File -- ) | f | f File>fd . ;";

/// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
/// primitive in Slice 8c: an ordinary one-field struct with a `drop`
/// overload, so it is linear for the same reason any resource is (R3 of
/// slice 8b), not by any compiler-known bit. Always the first struct in a
/// source string that uses it, so every other struct's `StructId` shifts
/// up by one relative to a spy-free program.
pub(super) const SPY_DEF: &str =
    "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy>tag . ;\n";

/// Every symbol an `IrFunc` calls, in emission order: what "the override
/// ran instead of the glue" is asserted on, rather than a substring of the
/// emitted text.
pub(super) fn call_symbols(func: &IrFunc) -> Vec<&str> {
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
pub(super) fn is_call_instr(i: &Instr) -> bool {
    matches!(i, Instr::Call(..) | Instr::CallIndirect(..))
}

pub(super) fn func<'a>(module: &'a IrModule, name: &str) -> &'a IrFunc {
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

pub(super) fn structs_of(src: &str) -> Structs {
    let tokens = lex(src).unwrap();
    let mut module = parse(&tokens).unwrap();
    check(&mut module).unwrap();
    Structs::from_structs(&module.structs)
}

pub(super) fn enums_of(src: &str) -> Enums {
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
pub(super) struct Probe {
    pub(super) structs: Structs,
    pub(super) enums: Enums,
    pub(super) arrays: Arrays,
    pub(super) cells: Cells,
    pub(super) refs: Refs,
}

impl Probe {
    pub(super) fn new(src: &str) -> Probe {
        Probe::with_overrides(src, &[])
    }

    /// A `Probe` whose named structs each carry a `drop` overload, set the
    /// way `check` sets it but without a `: drop` word in the source.
    /// Deliberately not written as a program: an override body on a
    /// disposal cycle must dispose something that leads back to its own
    /// receiver, which R6's self-recursion rejection refuses, so R7's
    /// cycle boundary is reachable from the registries but not from a
    /// module that type-checks.
    pub(super) fn with_overrides(src: &str, overridden: &[&str]) -> Probe {
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

    pub(super) fn regs(&self) -> Registries<'_> {
        Registries {
            structs: &self.structs,
            enums: &self.enums,
            arrays: &self.arrays,
            cells: &self.cells,
            refs: &self.refs,
            statics: empty_statics(),
        }
    }

    pub(super) fn struct_id(&self, name: &str) -> StructId {
        match self.struct_ty(name) {
            IrType::Struct(id) => id,
            other => unreachable!("{other:?}"),
        }
    }

    pub(super) fn struct_ty(&self, name: &str) -> IrType {
        let idx = self
            .structs
            .layouts
            .iter()
            .position(|l| l.name == name)
            .expect("declared struct");
        IrType::Struct(StructId::from_index(idx))
    }

    pub(super) fn enum_ty(&self, name: &str) -> IrType {
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
    pub(super) fn cell(&self, payload: IrType) -> OwnedCellId {
        let idx = self
            .cells
            .payload
            .iter()
            .position(|&p| p == payload)
            .expect("interned cell");
        OwnedCellId::from_index(idx)
    }

    pub(super) fn path(&self, ty: IrType) -> Option<Vec<PathStep>> {
        recursive_disposal_path(ty, self.regs())
    }
}

pub(super) fn layout<'a>(s: &'a Structs, name: &str) -> &'a StructLayout {
    s.layouts.iter().find(|l| l.name == name).expect("layout")
}

pub(super) fn enum_layout<'a>(e: &'a Enums, name: &str) -> &'a EnumLayout {
    e.layouts.iter().find(|l| l.name == name).expect("layout")
}

pub(super) fn instrs(func: &IrFunc) -> Vec<&Instr> {
    func.blocks.iter().flat_map(|b| b.instrs.iter()).collect()
}

pub(super) fn line_terms(src: &str) -> Vec<Term> {
    let tokens = lex(src).unwrap();
    match parse_line(&tokens).unwrap() {
        Line::Expr(terms) => terms,
        other => panic!("expected Expr, got {other:?}"),
    }
}

pub(super) fn count(func: &IrFunc, pred: impl Fn(&Instr) -> bool) -> usize {
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| pred(i))
        .count()
}

pub(super) fn empty_builder<'a>(
    env: &'a HashMap<String, Arity>,
    resolve: Resolver<'a>,
    regs: Registries<'a>,
) -> FuncBuilder<'a> {
    FuncBuilder::new(env, resolve, regs, String::new())
}

pub(super) fn arrays_of(src: &str) -> Arrays {
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

pub(super) fn module_of(src: &str) -> Module {
    let tokens = lex(src).unwrap();
    let mut module = parse(&tokens).unwrap();
    check(&mut module).unwrap();
    module
}

/// The loop header of a self-tail-recursive word: the entry block (block 0)
/// jumps to it (R6), so its id is the entry's `Jmp` target.
pub(super) fn loop_header(func: &IrFunc) -> BlockId {
    match func.blocks[0].term {
        Terminator::Jmp(h) => h,
        ref t => panic!("entry block should Jmp to the loop header, got {t:?}"),
    }
}

pub(super) fn header_block(func: &IrFunc, header: BlockId) -> &Block {
    func.blocks.iter().find(|b| b.id == header).expect("header")
}

pub(super) fn header_phis(block: &Block) -> Vec<&Vec<(BlockId, Value)>> {
    block
        .instrs
        .iter()
        .filter_map(|i| match i {
            Instr::Phi(_, arms) => Some(arms),
            _ => None,
        })
        .collect()
}

pub(super) fn jmps_to(func: &IrFunc, target: BlockId) -> usize {
    func.blocks
        .iter()
        .filter(|b| matches!(b.term, Terminator::Jmp(h) if h == target))
        .count()
}

/// The header phi structure that matters for R11: how many phis, how many
/// arms each has, and how many jumps target the header. Deliberately
/// ignores the carried `Value`s themselves, since those differ between
/// two independently-lowered programs even when the shape is identical.
pub(super) fn header_phi_shape(func: &IrFunc, header: BlockId) -> (usize, Vec<usize>, usize) {
    let phis = header_phis(header_block(func, header));
    let phi_count = phis.len();
    let arm_counts = phis.iter().map(|arms| arms.len()).collect();
    (phi_count, arm_counts, jmps_to(func, header))
}

/// The back-edge predecessor block of a self-tail loop: the non-entry block
/// that jumps to the header.
pub(super) fn back_edge_pred(f: &IrFunc, header: BlockId) -> &Block {
    let entry_id = f.blocks[0].id;
    f.blocks
        .iter()
        .find(|b| b.id != entry_id && matches!(b.term, Terminator::Jmp(h) if h == header))
        .expect("a back-edge predecessor block")
}

// Phase 4 Slice 3: the aggregate-staging loop transform (R1-R4, R1a).
// Structural coverage beside the changed `begin_loop`/`finalize_loop`; the
// runtime witnesses are the `tests/phase4_generics.rs` goldens.

/// A self-tail loop carrying an i64 (scalar) and a re-produced `Box`
/// (aggregate), so the aggregate slot stages rather than forwards.
pub(super) const STAGED_LOOP: &str = "type: Box n i64 ;\n\
         : mk ( i64 -- Box ) | n | n Box ;\n\
         : loop ( i64 Box -- Box ) | n b | n 0 = ~[ b ] ~[ n 1 - n mk loop ] if ;";
