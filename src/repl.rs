//! REPL: compile each line through the normal pipeline to a shared object and
//! `dlopen` it into the session process (no interpreter, no JIT).
//!
//! `Session` owns the persistent stack buffer and the word env (arity +
//! generation + symbol); the read-eval-print loop lexes/parses/checks/lowers/
//! emits/compiles/loads each line exactly like `build`, differing only in
//! target (`.so` not a binary) and in carrying state across lines.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::{BufRead, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::ast::{Line, Span, StructDecl, Term, Type, WordDef};
use crate::check::{self, Sig};
use crate::driver;
use crate::ir::{self, IrModule, StructLayout};
use crate::lexer::Token;
use crate::{backend, lexer, parser};

// RTLD_NOW is 2 on both Linux and macOS; RTLD_GLOBAL's value differs.
const RTLD_NOW: c_int = 2;
#[cfg(target_os = "linux")]
const RTLD_GLOBAL: c_int = 0x100;
#[cfg(target_os = "macos")]
const RTLD_GLOBAL: c_int = 0x8;

extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
    fn fflush(stream: *mut c_void) -> c_int;
}

/// A loaded shared object. The session keeps every handle resident (never
/// `dlclose`) so symbols from earlier lines stay callable by later ones.
pub struct Library {
    handle: *mut c_void,
}

impl Library {
    /// Open a shared object with global visibility, so its exports resolve for
    /// objects loaded by later lines.
    pub fn open(path: &Path) -> Result<Library, String> {
        let cpath = CString::new(path.as_os_str().as_bytes())
            .map_err(|e| format!("path has interior nul: {e}"))?;
        // SAFETY: cpath is a valid nul-terminated C string for the call's duration.
        let handle = unsafe {
            dlerror(); // clear any stale error
            dlopen(cpath.as_ptr(), RTLD_NOW | RTLD_GLOBAL)
        };
        if handle.is_null() {
            return Err(format!("dlopen {path:?} failed: {}", last_dlerror()));
        }
        Ok(Library { handle })
    }

    /// Resolve an exported symbol to a raw pointer (caller transmutes to a fn).
    pub fn symbol(&self, name: &str) -> Result<*mut c_void, String> {
        let cname = CString::new(name).map_err(|e| format!("symbol has interior nul: {e}"))?;
        // SAFETY: handle came from a successful dlopen; cname is nul-terminated.
        let sym = unsafe {
            dlerror();
            dlsym(self.handle, cname.as_ptr())
        };
        if sym.is_null() {
            return Err(format!("dlsym {name:?} failed: {}", last_dlerror()));
        }
        Ok(sym)
    }
}

fn last_dlerror() -> String {
    // SAFETY: dlerror returns either null or a valid C string owned by libdl.
    unsafe {
        let p = dlerror();
        if p.is_null() {
            "unknown error".to_string()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// A session's knowledge of one user-defined word: its typed effect, the
/// generation counter it was last (re)defined at, and the mangled symbol that
/// generation exports. Redefinition bumps the generation and mints a new symbol;
/// calls compiled before the redefinition keep resolving to the old symbol (it's
/// still resident, never `dlclose`d), calls compiled after resolve to the new
/// one.
struct WordEntry {
    sig: Sig,
    generation: u64,
    symbol: String,
}

/// Derive ir's arity map (RK2) from the typed checker env: ir needs only the
/// input/output counts and the output `IrType`, not the full typed effect.
fn ir_arity_env(env: &HashMap<String, Sig>) -> HashMap<String, ir::Arity> {
    env.iter()
        .map(|(name, sig)| {
            let ret = sig.outputs.first().map(|&ty| ir::ir_type_of(ty));
            (name.clone(), (sig.inputs.len(), sig.outputs.len(), ret))
        })
        .collect()
}

/// The mangled export symbol for `name` at `generation`.
fn mangled_symbol(name: &str, generation: u64) -> String {
    format!("{name}__gen{generation}")
}

/// The generation a new definition of `name` should take: 0 if never defined,
/// else one past the current entry's generation.
fn next_generation(existing: Option<&WordEntry>) -> u64 {
    existing.map(|e| e.generation + 1).unwrap_or(0)
}

/// A resolver over the current generations (no override), for compiling an
/// expression line.
fn resolver_for(env: &HashMap<String, WordEntry>) -> impl Fn(&str) -> String + '_ {
    move |name: &str| {
        env.get(name)
            .map(|e| e.symbol.clone())
            .unwrap_or_else(|| name.to_string())
    }
}

/// A call-name resolver over the current generations in `env`, with
/// `override_name` forced to `override_symbol` regardless of what `env` says
/// (so a definition's own recursive calls bind its new generation, not
/// whatever `env` still holds from the previous definition).
fn resolver_with_override<'a>(
    env: &'a HashMap<String, WordEntry>,
    override_name: &'a str,
    override_symbol: &'a str,
) -> impl Fn(&str) -> String + 'a {
    move |name: &str| {
        if name == override_name {
            override_symbol.to_string()
        } else {
            env.get(name)
                .map(|e| e.symbol.clone())
                .unwrap_or_else(|| name.to_string())
        }
    }
}

/// Format the carried stack, bottom to top, for the session's per-expression
/// output line. `buf` holds the live carried bytes as 8-byte `i64` cells;
/// each slot's cell offset is computed from the per-slot sizes (a scalar is
/// one cell, a struct spans `ceil(size/8)` cells), so a scalar slot past a
/// struct still reads the right cell (R17, R18).
///
/// A struct slot renders as its type-name placeholder `<TypeName>`, reading no
/// field bytes (M4). A float slot is reinterpreted from its stored bits via
/// `from_bits` (R21): displaying its `i64` bit pattern would be meaningless. An
/// `f32` slot reads only the low 32 bits (it was stored 4-wide, Q2). A `bool`
/// slot displays as `true`/`false` (matching `.`, not the raw 0/1). An
/// unsigned slot displays as its unsigned value: the raw `i64` bit pattern of
/// a high-bit-set `u64` is negative and would otherwise misprint as such.
pub fn format_stack(buf: &[i64], types: &[Type], layouts: &[StructLayout]) -> String {
    if types.is_empty() {
        return "stack: (empty)".to_string();
    }
    let mut cell = 0usize;
    let mut vals = Vec::with_capacity(types.len());
    for ty in types {
        match ty {
            Type::Struct(id, name) => {
                vals.push(format!("<{name}>"));
                let size = layouts[id.index()].size as usize;
                cell += size.div_ceil(8);
            }
            _ => {
                let v = buf[cell];
                vals.push(match ty {
                    Type::Float(ft) if ft.bits() == 32 => {
                        f32::from_bits(v as u64 as u32).to_string()
                    }
                    Type::Float(_) => f64::from_bits(v as u64).to_string(),
                    Type::Bool => if v != 0 { "true" } else { "false" }.to_string(),
                    Type::Int(it) if !it.signed() => (v as u64).to_string(),
                    _ => v.to_string(),
                });
                cell += 1;
            }
        }
    }
    format!("stack: {}", vals.join(" "))
}

/// A REPL session: the accumulated word env, the persistent stack buffer, and
/// every loaded shared object (kept resident for the session's lifetime).
pub struct Session {
    env: HashMap<String, WordEntry>,
    /// The struct registry, one entry per `type:` line, in declaration order
    /// so `StructId` = index stays stable (a carried `Type::Struct` keeps
    /// referring to the same struct across lines). Field types resolve against
    /// earlier entries plus the entry being declared (R3).
    structs: Vec<StructDecl>,
    /// The carried stack, as 8-byte `i64` cells. `top` is the live byte
    /// length; a slot may span more than one cell (a struct), so the buffer is
    /// byte-addressable and slot offsets are computed from `types`, never
    /// `index * 8` (R17).
    buf: Vec<i64>,
    top: usize,
    /// The `Type` of each carried slot, in stack order (deepest first). Slot
    /// byte sizes vary (a struct spans its aggregate size), so
    /// `types.len() != top / 8` in general.
    types: Vec<Type>,
    libs: Vec<Library>,
    seq: u64,
}

impl Session {
    pub fn new() -> Session {
        Session {
            env: HashMap::new(),
            structs: Vec::new(),
            buf: Vec::new(),
            top: 0,
            types: Vec::new(),
            libs: Vec::new(),
            seq: 0,
        }
    }

    /// The checker's typed env: builtins, the generated struct words (R7), plus
    /// every successfully-defined user word.
    fn typed_env(&self) -> HashMap<String, Sig> {
        let mut env = check::builtin_table();
        for (name, sig) in check::struct_generated_sigs(&self.structs) {
            env.insert(name, sig);
        }
        for (name, entry) in &self.env {
            env.insert(name.clone(), entry.sig.clone());
        }
        env
    }

    /// Evaluate one line of input, writing any success output to `writer`.
    /// On error, the session (env, stack) is left untouched; the caller
    /// prints the returned diagnostic.
    fn eval_line(&mut self, src: &str, writer: &mut impl Write) -> Result<(), String> {
        let tokens = lexer::lex(src)?;
        if matches!(tokens.first(), Some((Token::Word(w), _)) if w == "type:") {
            return self.eval_typedef(&tokens, writer);
        }
        let line = parser::parse_line_with_structs(&tokens, &self.structs)?;
        match line {
            Line::Def(word) => self.eval_def(word, writer),
            Line::Expr(terms) => self.eval_expr(&terms, writer),
        }
    }

    /// Register a `type:` struct declaration (R3/R5). The new name is appended
    /// to the registry first (so a self-reference in its own fields resolves,
    /// and is then rejected as recursion, X3); fields resolve against the whole
    /// registry. On any error the appended entry is rolled back, leaving the
    /// session untouched.
    fn eval_typedef(
        &mut self,
        tokens: &[(Token, Span)],
        writer: &mut impl Write,
    ) -> Result<(), String> {
        let (name, span) = match tokens.get(1) {
            Some((Token::Word(w), span)) => (w.clone(), *span),
            _ => return Err("parse error: `type:` must be followed by a struct name".to_string()),
        };
        let idx = self.structs.len();
        self.structs.push(StructDecl {
            name: name.clone(),
            name_static: Box::leak(name.clone().into_boxed_str()),
            fields: Vec::new(),
            span,
        });
        let result = parser::parse_typedef_line(tokens, &self.structs).and_then(|fields| {
            self.structs[idx].fields = fields;
            check::check_structs(&self.structs)
        });
        if let Err(e) = result {
            self.structs.pop();
            return Err(e);
        }
        writeln!(writer, "defined type {name}").map_err(|e| format!("writing stdout: {e}"))?;
        Ok(())
    }

    fn eval_def(&mut self, word: WordDef, writer: &mut impl Write) -> Result<(), String> {
        let name = word.name.clone();
        let sig = check::sig_of(&word.effect);

        let mut env = self.typed_env();
        check::check_def(&word, &env)?;

        let generation = next_generation(self.env.get(&name));
        let symbol = mangled_symbol(&name, generation);

        // Self-recursive calls in the body must bind this new generation, not
        // whatever generation `env` still holds for `name`; seed the definee's
        // own signature so ir derives its return type. The arity map for ir is
        // derived from the typed env (RK2): ir needs only counts + output type.
        env.insert(name.clone(), sig.clone());
        let ir_lower_env = ir_arity_env(&env);
        let structs = ir::Structs::from_structs(&self.structs);
        let mut func = {
            let resolve = resolver_with_override(&self.env, &name, &symbol);
            ir::lower_word(&word, &ir_lower_env, &resolve, &structs)
        };
        func.name = symbol.clone();

        let ssa = backend::qbe::emit(&IrModule {
            funcs: vec![func],
            structs: structs.layouts,
        })?;
        let dir = driver::tempfile_dir()?;
        let so_path = dir.join(format!("{name}_gen{generation}.so"));
        driver::compile_so(&ssa, &so_path)?;
        let lib = Library::open(&so_path)?;

        // Only commit on success: env stays untouched on any earlier failure.
        self.libs.push(lib);
        self.env.insert(
            name.clone(),
            WordEntry {
                sig,
                generation,
                symbol,
            },
        );
        writeln!(writer, "defined {name}").map_err(|e| format!("writing stdout: {e}"))?;
        Ok(())
    }

    fn eval_expr(&mut self, terms: &[Term], writer: &mut impl Write) -> Result<(), String> {
        let env = self.typed_env();
        let entry_depth = self.types.len();
        let net_stack = check::infer_line(terms, &self.types, &env)?;
        let net_depth = net_stack.len();

        let ir_lower_env = ir_arity_env(&env);

        self.seq += 1;
        let seq = self.seq;
        let structs = ir::Structs::from_structs(&self.structs);
        let (func, m, out_bytes) = {
            let resolve = resolver_for(&self.env);
            ir::lower_line(
                seq,
                terms,
                entry_depth,
                &self.types,
                &ir_lower_env,
                &resolve,
                &structs,
            )
        };
        // `m` (the wrapper's emitted output slot count) and `net_depth` (the
        // checker's independently-inferred net effect) are the same depth
        // simulation and must always agree; `out_bytes` is what the wrapper
        // actually writes and sizes the buffer. Assert the checker agrees
        // rather than trusting two separately-computed counts to stay in sync
        // as codegen evolves.
        debug_assert_eq!(
            m, net_depth,
            "lowering emitted a different depth than the checker inferred"
        );

        let ssa = backend::qbe::emit(&IrModule {
            funcs: vec![func],
            structs: structs.layouts.clone(),
        })?;
        let dir = driver::tempfile_dir()?;
        let so_path = dir.join(format!("line{seq}.so"));
        driver::compile_so(&ssa, &so_path)?;
        let lib = Library::open(&so_path)?;
        let sym = lib.symbol(&format!("sooth_line_{seq}"))?;
        // SAFETY: emitted as `export function l $sooth_line_{seq}(l %v0, l %v1)`,
        // i.e. a C-ABI `(u64, u64) -> u64` function on this 64-bit target,
        // matching the `(*mut u8, usize) -> usize` transmute below.
        let wrapper: extern "C" fn(*mut u8, usize) -> usize = unsafe { std::mem::transmute(sym) };

        // Size the buffer (in 8-byte cells) to cover the wrapper's output
        // bytes; it already covers the entry bytes (`self.top`) from the line
        // that produced them. `out_bytes` is always a multiple of 8
        // (`carried_slot_bytes` rounds each slot up), so `div_ceil` is exact.
        let out_cells = out_bytes.div_ceil(8);
        if self.buf.len() < out_cells {
            self.buf.resize(out_cells, 0);
        }
        // Flush any host-buffered stdout first so it interleaves deterministically
        // with the loaded code's own `printf` (a separate C stdio buffer).
        writer
            .flush()
            .map_err(|e| format!("flushing stdout: {e}"))?;
        let base_ptr = self.buf.as_mut_ptr() as *mut u8;
        // SAFETY: `base_ptr` points into a `Vec<i64>` grown to at least
        // `out_cells` cells (`out_bytes` bytes); `self.top` is the live byte
        // length on entry, a multiple of 8 and `<= self.buf.len() * 8`. The
        // wrapper only reads/writes within `[0, max(self.top, out_bytes))`.
        let new_top = wrapper(base_ptr, self.top);

        // Flush the loaded code's C stdio buffer so its `.`/printf output lands
        // on the fd before the host writes the residual-stack line.
        // SAFETY: fflush(NULL) flushes all open C streams; always sound.
        unsafe { fflush(std::ptr::null_mut()) };
        self.top = new_top;
        self.types = net_stack;
        self.libs.push(lib);

        let cells = self.top / 8;
        writeln!(
            writer,
            "{}",
            format_stack(&self.buf[..cells], &self.types, &structs.layouts)
        )
        .map_err(|e| format!("writing stdout: {e}"))?;
        Ok(())
    }
}

impl Default for Session {
    fn default() -> Session {
        Session::new()
    }
}

/// The read-eval-print loop: blank lines are skipped silently, EOF exits
/// cleanly, and any stage error prints the diagnostic without mutating
/// session state.
pub fn run(mut reader: impl BufRead, mut writer: impl Write) -> Result<(), String> {
    let mut session = Session::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("reading stdin: {e}"))?;
        if n == 0 {
            return Ok(());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Err(e) = session.eval_line(trimmed, &mut writer) {
            writeln!(writer, "{e}").map_err(|e| format!("writing stdout: {e}"))?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend, check, driver, ir, lexer, parser};

    #[test]
    fn compiled_word_is_dlsymable_and_callable() {
        let src = ": sq ( i64 -- i64 ) | n | n n * ;";
        let tokens = lexer::lex(src).unwrap();
        let module = parser::parse(&tokens).unwrap();
        check::check(&module).unwrap();
        let ir = ir::lower(&module).unwrap();
        let ssa = backend::qbe::emit(&ir).unwrap();

        let dir = driver::tempfile_dir().unwrap();
        let so = dir.join("libsq.so");
        driver::compile_so(&ssa, &so).expect("compile_so should succeed");

        let lib = Library::open(&so).expect("dlopen should succeed");
        let sym = lib.symbol("sq").expect("dlsym should find the word");
        // SAFETY: `sq` was emitted as `export function l $sq(l %v0)`, i.e. a
        // C-ABI `l`-taking, `l`-returning function on this 64-bit target.
        let sq: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(sym) };
        assert_eq!(sq(5), 25);
    }

    #[test]
    fn format_stack_bottom_to_top() {
        let types = vec![Type::I64, Type::I64, Type::I64];
        assert_eq!(format_stack(&[1, 2, 3], &types, &[]), "stack: 1 2 3");
    }

    #[test]
    fn format_stack_empty_is_marker() {
        assert_eq!(format_stack(&[], &[], &[]), "stack: (empty)");
    }

    #[test]
    fn format_stack_f64_slot_renders_float_not_bits() {
        // A carried `f64` displays its value, not the `i64` bit pattern (R21).
        let bits = 2.5f64.to_bits() as i64;
        assert_eq!(format_stack(&[bits], &[Type::F64], &[]), "stack: 2.5");
    }

    #[test]
    fn format_stack_f32_slot_reads_low_32_bits() {
        // An `f32` slot stores 4 bytes; display reads the low 32 bits (Q2/R21).
        let bits = 1.5f32.to_bits() as u64 as i64;
        let f32_ty = Type::from_name("f32").unwrap();
        assert_eq!(format_stack(&[bits], &[f32_ty], &[]), "stack: 1.5");
    }

    #[test]
    fn format_stack_bool_slot_displays_as_true_or_false() {
        // Matches `.`'s print semantics: `true`/`false`, not the raw 0/1.
        assert_eq!(
            format_stack(&[1, 0], &[Type::Bool, Type::Bool], &[]),
            "stack: true false"
        );
    }

    #[test]
    fn format_stack_struct_slot_shows_placeholder_and_offsets_past_it() {
        use crate::ast::StructId;
        // A 16-byte struct (two 8-byte cells) at StructId 0, then a scalar
        // slot. The struct renders as its `<Vec2>` placeholder reading no
        // field bytes (M4), and the trailing scalar reads the cell *past* the
        // struct's two cells, not `index * 8` (R18).
        let layouts = vec![StructLayout {
            name: "Vec2",
            size: 16,
            align: 8,
            fields: vec![],
        }];
        let vec2 = Type::Struct(StructId::from_index(0), "Vec2");
        assert_eq!(
            format_stack(&[5, 6, 99], &[vec2, Type::I64], &layouts),
            "stack: <Vec2> 99"
        );
    }

    #[test]
    fn format_stack_unsigned_slot_displays_unsigned_not_negative() {
        // A `u64` with the high bit set stores a negative `i64` bit pattern;
        // display must render its unsigned value, not that negative number.
        let u64_ty = Type::from_name("u64").unwrap();
        assert_eq!(
            format_stack(&[-1], &[u64_ty], &[]),
            "stack: 18446744073709551615"
        );
    }

    fn entry(generation: u64) -> WordEntry {
        WordEntry {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![Type::I64],
            },
            generation,
            symbol: mangled_symbol("sq", generation),
        }
    }

    #[test]
    fn resolve_binds_current_generation() {
        let mut env = HashMap::new();
        env.insert("sq".to_string(), entry(2));
        let resolve = resolver_with_override(&env, "__none__", "__none__");
        assert_eq!(resolve("sq"), "sq__gen2");
        assert_eq!(resolve("dup"), "dup");
    }

    #[test]
    fn redefinition_bumps_generation() {
        let mut env = HashMap::new();
        assert_eq!(next_generation(env.get("sq")), 0);
        env.insert("sq".to_string(), entry(0));
        assert_eq!(next_generation(env.get("sq")), 1);
    }
}
