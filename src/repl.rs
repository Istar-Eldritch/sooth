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

use crate::ast::{Line, Term, Type, WordDef};
use crate::check::{self, Sig};
use crate::driver;
use crate::ir::{self, IrModule};
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
/// output line.
pub fn format_stack(stack: &[i64]) -> String {
    if stack.is_empty() {
        return "stack: (empty)".to_string();
    }
    let vals: Vec<String> = stack.iter().map(|v| v.to_string()).collect();
    format!("stack: {}", vals.join(" "))
}

/// A REPL session: the accumulated word env, the persistent stack buffer, and
/// every loaded shared object (kept resident for the session's lifetime).
pub struct Session {
    env: HashMap<String, WordEntry>,
    buf: Vec<i64>,
    top: usize,
    /// The `Type` of each carried slot, one per live 8-byte buffer slot
    /// (`types.len() == top / 8`).
    types: Vec<Type>,
    libs: Vec<Library>,
    seq: u64,
}

impl Session {
    pub fn new() -> Session {
        Session {
            env: HashMap::new(),
            buf: Vec::new(),
            top: 0,
            types: Vec::new(),
            libs: Vec::new(),
            seq: 0,
        }
    }

    /// The checker's typed env: builtins plus every successfully-defined word.
    fn typed_env(&self) -> HashMap<String, Sig> {
        let mut env = check::builtin_table();
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
        let line = parser::parse_line(&tokens)?;
        match line {
            Line::Def(word) => self.eval_def(word, writer),
            Line::Expr(terms) => self.eval_expr(&terms, writer),
        }
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
        let mut func = {
            let resolve = resolver_with_override(&self.env, &name, &symbol);
            ir::lower_word(&word, &ir_lower_env, &resolve)
        };
        func.name = symbol.clone();

        let ssa = backend::qbe::emit(&IrModule { funcs: vec![func] })?;
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
        let entry_depth = self.top / 8;
        let net_stack = check::infer_line(terms, &self.types, &env)?;
        let net_depth = net_stack.len();

        let ir_lower_env = ir_arity_env(&env);

        self.seq += 1;
        let seq = self.seq;
        let (func, m) = {
            let resolve = resolver_for(&self.env);
            ir::lower_line(
                seq,
                terms,
                entry_depth,
                &self.types,
                &ir_lower_env,
                &resolve,
            )
        };
        // `m` (the wrapper's emitted store count) and `net_depth` (the checker's
        // independently-inferred net effect) are the same depth simulation and
        // must always agree; size the buffer from `m`, the number the wrapper
        // actually writes, and assert the checker agrees rather than trusting
        // two separately-computed counts to stay in sync as codegen evolves.
        debug_assert_eq!(
            m, net_depth,
            "lowering emitted a different depth than the checker inferred"
        );

        let ssa = backend::qbe::emit(&IrModule { funcs: vec![func] })?;
        let dir = driver::tempfile_dir()?;
        let so_path = dir.join(format!("line{seq}.so"));
        driver::compile_so(&ssa, &so_path)?;
        let lib = Library::open(&so_path)?;
        let sym = lib.symbol(&format!("sooth_line_{seq}"))?;
        // SAFETY: emitted as `export function l $sooth_line_{seq}(l %v0, l %v1)`,
        // i.e. a C-ABI `(u64, u64) -> u64` function on this 64-bit target,
        // matching the `(*mut u8, usize) -> usize` transmute below.
        let wrapper: extern "C" fn(*mut u8, usize) -> usize = unsafe { std::mem::transmute(sym) };

        if self.buf.len() < m {
            self.buf.resize(m, 0);
        }
        // Flush any host-buffered stdout first so it interleaves deterministically
        // with the loaded code's own `printf` (a separate C stdio buffer).
        writer
            .flush()
            .map_err(|e| format!("flushing stdout: {e}"))?;
        let base_ptr = self.buf.as_mut_ptr() as *mut u8;
        // SAFETY: `base_ptr` points into a `Vec<i64>` grown to at least `m` slots
        // (`m*8` bytes); `self.top` is the live byte length on entry, a multiple
        // of 8 and `<= self.buf.len() * 8`. The wrapper only reads/writes within
        // `[0, max(self.top, m*8))`.
        let new_top = wrapper(base_ptr, self.top);

        // Flush the loaded code's C stdio buffer so its `.`/printf output lands
        // on the fd before the host writes the residual-stack line.
        // SAFETY: fflush(NULL) flushes all open C streams; always sound.
        unsafe { fflush(std::ptr::null_mut()) };
        self.top = new_top;
        self.types = net_stack;
        self.libs.push(lib);

        let d = self.top / 8;
        writeln!(writer, "{}", format_stack(&self.buf[..d]))
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
        assert_eq!(format_stack(&[1, 2, 3]), "stack: 1 2 3");
    }

    #[test]
    fn format_stack_empty_is_marker() {
        assert_eq!(format_stack(&[]), "stack: (empty)");
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
