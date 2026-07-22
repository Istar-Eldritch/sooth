//! Backend-neutral IR.
//!
//! The compile-time virtual stack is lowered to SSA-shaped values here, and each
//! word becomes a function taking N inputs and returning M outputs. Control words
//! become basic blocks and branches. This IR feeds QBE today and a WASM sibling
//! lowering later, so it stays neutral: in particular `Ptr` is an opaque handle,
//! never assumed to be a native `u64`, so QBE (native pointers) and WASM
//! (linear-memory offsets) can each concretise it.

use std::collections::HashMap;
use std::mem;

use crate::ast::{Module, Term, TermKind, WordDef};

#[derive(Debug, Default)]
pub struct IrModule {
    pub funcs: Vec<IrFunc>,
}

#[derive(Debug)]
pub struct IrFunc {
    pub name: String,
    pub params: Vec<IrType>,
    pub ret: Option<IrType>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrType {
    Int,
    /// Opaque handle (backend-neutral-IR invariant): a native pointer under QBE,
    /// a linear-memory offset under a future WASM lowering. Used by the line
    /// wrapper's `%stack` parameter.
    Ptr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Value(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockId(pub u32);

#[derive(Debug)]
pub struct Block {
    pub id: BlockId,
    pub instrs: Vec<Instr>,
    pub term: Terminator,
}

#[derive(Debug)]
pub enum Instr {
    Const(Value, i64),
    Bin(Value, BinOp, Value, Value),
    Cmp(Value, CmpOp, Value, Value),
    Call(Option<Value>, String, Vec<Value>),
    Print(Value),
    Phi(Value, Vec<(BlockId, Value)>),
    /// `dst: Ptr = base + bytes`. Keeps `Ptr` opaque (no native-width assumption).
    PtrOffset(Value, Value, i64),
    /// `dst: Int = *ptr`.
    Load(Value, Value),
    /// `*ptr = val` (Int).
    Store(Value, Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Rem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Lt,
    Gt,
}

#[derive(Debug)]
pub enum Terminator {
    Ret(Option<Value>),
    Jnz(Value, BlockId, BlockId),
    Jmp(BlockId),
}

/// Declared arity of a user word: (inputs, outputs).
pub type Arity = (usize, usize);

/// Maps a called user-word name to the symbol it is emitted/linked as. The build
/// path uses identity; the REPL supplies generation-mangled symbols so a unit
/// links against the words it was compiled against.
pub type Resolver<'a> = &'a dyn Fn(&str) -> String;

pub fn lower(module: &Module) -> Result<IrModule, String> {
    let env: HashMap<String, Arity> = module
        .words
        .iter()
        .map(|w| {
            (
                w.name.clone(),
                (w.effect.inputs.len(), w.effect.outputs.len()),
            )
        })
        .collect();
    let resolve = |name: &str| name.to_string();

    let funcs = module
        .words
        .iter()
        .map(|w| lower_word(w, &env, &resolve))
        .collect();

    Ok(IrModule { funcs })
}

/// Lower a bare REPL line to a uniform-signature wrapper `sooth_line_{seq}`
/// `(Ptr stack, Int top) -> Int`. The prologue loads the whole carried stack
/// (`entry_depth` slots) from the buffer, the body runs in registers exactly
/// like a word, the epilogue stores the resulting `M` slots back, and it returns
/// the advanced top `top + (M - entry_depth) * 8`.
///
/// Returns the `IrFunc` alongside the emitted `M`, so the caller sizes its
/// buffer from the same number the wrapper actually stores, rather than from
/// a separately-computed depth that could in principle diverge.
pub fn lower_line(
    seq: u64,
    terms: &[Term],
    entry_depth: usize,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
) -> (IrFunc, usize) {
    let mut b = FuncBuilder::new(env, resolve);

    // Params occupy the first value ids: %v0 = stack base (Ptr), %v1 = top (Int).
    let base = b.fresh_value();
    let top = b.fresh_value();

    // Prologue: load slot `i` at byte offset `i*8`, deepest (slot 0) first.
    let mut stack = Vec::with_capacity(entry_depth);
    for i in 0..entry_depth {
        let ptr = b.fresh_value();
        b.push_instr(Instr::PtrOffset(ptr, base, (i * 8) as i64));
        let v = b.fresh_value();
        b.push_instr(Instr::Load(v, ptr));
        stack.push(v);
    }
    b.stack = stack;

    b.lower_terms(terms);

    // Epilogue: store the resulting M slots back to the buffer.
    let out = mem::take(&mut b.stack);
    let m = out.len();
    for (j, v) in out.iter().enumerate() {
        let ptr = b.fresh_value();
        b.push_instr(Instr::PtrOffset(ptr, base, (j * 8) as i64));
        b.push_instr(Instr::Store(ptr, *v));
    }

    // Return the advanced top; (M - entry_depth) may be negative.
    let delta = (m as i64 - entry_depth as i64) * 8;
    let delta_val = b.fresh_value();
    b.push_instr(Instr::Const(delta_val, delta));
    let new_top = b.fresh_value();
    b.push_instr(Instr::Bin(new_top, BinOp::Add, top, delta_val));
    b.seal_block(Terminator::Ret(Some(new_top)));

    let func = IrFunc {
        name: format!("sooth_line_{seq}"),
        params: vec![IrType::Ptr, IrType::Int],
        ret: Some(IrType::Int),
        blocks: b.blocks,
    };
    (func, m)
}

/// Lower a single word body against an external env/resolver. The REPL uses
/// this directly (renaming the returned `IrFunc.name` to a mangled symbol)
/// so a definition compiles against previously-loaded words.
pub(crate) fn lower_word(
    word: &WordDef,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
) -> IrFunc {
    let n_inputs = word.effect.inputs.len();
    let params = vec![IrType::Int; n_inputs];
    let ret = if word.effect.outputs.is_empty() {
        None
    } else {
        Some(IrType::Int)
    };

    let mut b = FuncBuilder::new(env, resolve);

    // Params occupy the first N value ids; leftmost input is deepest.
    let mut stack: Vec<Value> = (0..n_inputs).map(|_| b.fresh_value()).collect();

    // Bind `| ... |` locals: pop the top N params, leftmost local = deepest.
    let take = word.locals.len();
    let bound = stack.split_off(stack.len() - take);
    for (name, value) in word.locals.iter().zip(bound) {
        b.locals.insert(name.clone(), value);
    }
    b.stack = stack;

    b.lower_terms(&word.body);

    let result = if ret.is_some() { b.stack.pop() } else { None };
    b.seal_block(Terminator::Ret(result));

    IrFunc {
        name: word.name.clone(),
        params,
        ret,
        blocks: b.blocks,
    }
}

struct FuncBuilder<'a> {
    env: &'a HashMap<String, Arity>,
    resolve: Resolver<'a>,
    blocks: Vec<Block>,
    cur_id: BlockId,
    cur_instrs: Vec<Instr>,
    next_value: u32,
    next_block: u32,
    stack: Vec<Value>,
    locals: HashMap<String, Value>,
}

impl<'a> FuncBuilder<'a> {
    fn new(env: &'a HashMap<String, Arity>, resolve: Resolver<'a>) -> Self {
        FuncBuilder {
            env,
            resolve,
            blocks: Vec::new(),
            cur_id: BlockId(0),
            cur_instrs: Vec::new(),
            next_value: 0,
            next_block: 1, // block 0 is the entry, already current
            stack: Vec::new(),
            locals: HashMap::new(),
        }
    }

    fn fresh_value(&mut self) -> Value {
        let v = Value(self.next_value);
        self.next_value += 1;
        v
    }

    fn fresh_block(&mut self) -> BlockId {
        let b = BlockId(self.next_block);
        self.next_block += 1;
        b
    }

    fn push_instr(&mut self, instr: Instr) {
        self.cur_instrs.push(instr);
    }

    /// Seal the current block with `term` and append it to the function.
    fn seal_block(&mut self, term: Terminator) {
        let instrs = mem::take(&mut self.cur_instrs);
        self.blocks.push(Block {
            id: self.cur_id,
            instrs,
            term,
        });
    }

    /// Begin a fresh (empty) block; `cur_instrs` is already empty after a seal.
    fn start_block(&mut self, id: BlockId) {
        self.cur_id = id;
    }

    fn lower_terms(&mut self, terms: &[Term]) {
        for term in terms {
            self.lower_term(term);
        }
    }

    fn lower_term(&mut self, term: &Term) {
        match &term.kind {
            TermKind::IntLit(n) => {
                let v = self.fresh_value();
                self.push_instr(Instr::Const(v, *n));
                self.stack.push(v);
            }
            TermKind::Call(name) => self.lower_call(name),
            TermKind::If {
                then_branch,
                else_branch,
            } => self.lower_if(then_branch, else_branch),
        }
    }

    fn lower_call(&mut self, name: &str) {
        if let Some(&value) = self.locals.get(name) {
            self.stack.push(value); // i64 is Copy; reuse the value id.
            return;
        }
        match name {
            "dup" => {
                let top = *self.stack.last().expect("dup: non-empty stack");
                self.stack.push(top);
            }
            "drop" => {
                self.stack.pop().expect("drop: non-empty stack");
            }
            "swap" => {
                let n = self.stack.len();
                self.stack.swap(n - 1, n - 2);
            }
            "over" => {
                let below = self.stack[self.stack.len() - 2];
                self.stack.push(below);
            }
            "rot" => {
                // a b c -> b c a
                let n = self.stack.len();
                let a = self.stack[n - 3];
                self.stack[n - 3] = self.stack[n - 2];
                self.stack[n - 2] = self.stack[n - 1];
                self.stack[n - 1] = a;
            }
            "+" | "-" | "*" | "mod" => {
                let op = match name {
                    "+" => BinOp::Add,
                    "-" => BinOp::Sub,
                    "*" => BinOp::Mul,
                    _ => BinOp::Rem,
                };
                let rhs = self.stack.pop().expect("bin: rhs");
                let lhs = self.stack.pop().expect("bin: lhs");
                let v = self.fresh_value();
                self.push_instr(Instr::Bin(v, op, lhs, rhs));
                self.stack.push(v);
            }
            "=" | "<" | ">" => {
                let op = match name {
                    "=" => CmpOp::Eq,
                    "<" => CmpOp::Lt,
                    _ => CmpOp::Gt,
                };
                let rhs = self.stack.pop().expect("cmp: rhs");
                let lhs = self.stack.pop().expect("cmp: lhs");
                let v = self.fresh_value();
                self.push_instr(Instr::Cmp(v, op, lhs, rhs));
                self.stack.push(v);
            }
            "." => {
                let v = self.stack.pop().expect("print: value");
                self.push_instr(Instr::Print(v));
            }
            _ => {
                let (in_arity, out_arity) = *self.env.get(name).expect("checked user word exists");
                let split = self.stack.len() - in_arity;
                let args = self.stack.split_off(split);
                let ret = if out_arity == 1 {
                    Some(self.fresh_value())
                } else {
                    None
                };
                let sym = (self.resolve)(name);
                self.push_instr(Instr::Call(ret, sym, args));
                if let Some(v) = ret {
                    self.stack.push(v);
                }
            }
        }
    }

    fn lower_if(&mut self, then_branch: &[Term], else_branch: &[Term]) {
        let test = self.stack.pop().expect("if: test value");
        let then_id = self.fresh_block();
        let else_id = self.fresh_block();
        let join_id = self.fresh_block();

        let post_pop = self.stack.clone();
        self.seal_block(Terminator::Jnz(test, then_id, else_id));

        self.start_block(then_id);
        self.stack = post_pop.clone();
        self.lower_terms(then_branch);
        let then_stack = self.stack.clone();
        let then_pred = self.cur_id;
        self.seal_block(Terminator::Jmp(join_id));

        self.start_block(else_id);
        self.stack = post_pop;
        self.lower_terms(else_branch);
        let else_stack = self.stack.clone();
        let else_pred = self.cur_id;
        self.seal_block(Terminator::Jmp(join_id));

        self.start_block(join_id);
        let mut join_stack = Vec::with_capacity(then_stack.len());
        for (t, e) in then_stack.into_iter().zip(else_stack) {
            if t == e {
                join_stack.push(t);
            } else {
                let v = self.fresh_value();
                self.push_instr(Instr::Phi(v, vec![(then_pred, t), (else_pred, e)]));
                join_stack.push(v);
            }
        }
        self.stack = join_stack;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Line;
    use crate::check::check;
    use crate::lexer::lex;
    use crate::parser::{parse, parse_line};

    fn lower_src(src: &str) -> IrModule {
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        check(&module).unwrap();
        lower(&module).unwrap()
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
        let ir = lower_src(": w ( i64 -- i64 ) if 1 else 2 then ;");
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
        let (func, m) = lower_line(0, &line_terms("+"), 2, &env, &resolve);
        assert_eq!(m, 1);
        assert_eq!(count(&func, |i| matches!(i, Instr::Load(..))), 2);
        assert_eq!(count(&func, |i| matches!(i, Instr::Store(..))), 1);
    }

    #[test]
    fn lower_line_returns_advanced_top() {
        // `2 3 +` from D=0 nets +1, so new_top = top + 8.
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let (func, m) = lower_line(0, &line_terms("2 3 +"), 0, &env, &resolve);
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
    fn lower_call_uses_resolved_generation_symbol() {
        let mut env = HashMap::new();
        env.insert("sq".to_string(), (1usize, 1usize));
        let resolve = |name: &str| format!("{name}__gen2");
        let (func, _m) = lower_line(0, &line_terms("5 sq"), 0, &env, &resolve);
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
    fn lower_print_emits_print_instr() {
        let ir = lower_src(": w ( i64 -- ) . ;");
        let w = &ir.funcs[0];
        assert!(instrs(w).iter().any(|i| matches!(i, Instr::Print(_))));
        let last = w.blocks.last().unwrap();
        assert!(matches!(last.term, Terminator::Ret(None)));
    }
}
