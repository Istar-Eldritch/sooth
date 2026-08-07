//! Hand-rolled raw-mode line editor for the interactive REPL (tty only).
//!
//! Split out of `repl.rs` because it pulls a wholly divergent dependency set
//! (termios FFI, ANSI escape decoding, an fd read loop) and none of its
//! functions call the session's compile-eval stages: two of CLAUDE.md's split
//! signals. The piped (non-tty) REPL path never touches this module.
//!
//! The design keeps the *decision logic* pure so it is unit-testable without a
//! real terminal: `decode` turns a byte slice into a `Key`, and `Editor::apply`
//! transforms the edit buffer. Only `read_stdin_byte`, the `Termios` FFI, and
//! `RawModeGuard`'s syscalls are the thin untested-by-unit shell.

use std::ffi::{c_int, c_void};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

// --- termios raw-mode shell (F1: hand-declared FFI, no crate) --------------

extern "C" {
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn tcgetattr(fd: c_int, termios: *mut Termios) -> c_int;
    fn tcsetattr(fd: c_int, actions: c_int, termios: *const Termios) -> c_int;
}

// TCSAFLUSH discards queued input while switching modes; value 2 on both OSes.
const TCSAFLUSH: c_int = 2;

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; 32],
    c_ispeed: u32,
    c_ospeed: u32,
}

#[cfg(target_os = "linux")]
mod flags {
    pub const ISIG: u32 = 0x0000_0001;
    pub const ICANON: u32 = 0x0000_0002;
    pub const ECHO: u32 = 0x0000_0008;
    pub const VTIME: usize = 5;
    pub const VMIN: usize = 6;
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Termios {
    c_iflag: u64,
    c_oflag: u64,
    c_cflag: u64,
    c_lflag: u64,
    c_cc: [u8; 20],
    c_ispeed: u64,
    c_ospeed: u64,
}

#[cfg(target_os = "macos")]
mod flags {
    pub const ISIG: u64 = 0x0000_0080;
    pub const ICANON: u64 = 0x0000_0100;
    pub const ECHO: u64 = 0x0000_0008;
    pub const VTIME: usize = 17;
    pub const VMIN: usize = 16;
}

impl Termios {
    fn zeroed() -> Termios {
        // SAFETY: `Termios` is a plain-old-data `repr(C)` struct of integers
        // and byte arrays; an all-zero bit pattern is a valid inhabitant.
        unsafe { std::mem::zeroed() }
    }
}

/// The cooked-to-raw transform, kept pure so it is unit-testable and so
/// `RawModeGuard` restores by re-applying the *saved* value, not by inverting
/// this. Clears `ISIG`/`ICANON`/`ECHO` (so Ctrl-C/Ctrl-D reach us as bytes and
/// nothing echoes) and sets a 1-byte blocking read (`VMIN=1`, `VTIME=0`).
/// Output post-processing (`OPOST`) is deliberately left on so `\n` still maps
/// to `\r\n` for the eval output the session writes.
fn raw_termios(cooked: &Termios) -> Termios {
    let mut raw = *cooked;
    raw.c_lflag &= !(flags::ISIG | flags::ICANON | flags::ECHO);
    raw.c_cc[flags::VMIN] = 1;
    raw.c_cc[flags::VTIME] = 0;
    raw
}

/// The seam that makes `RawModeGuard`'s restore-on-drop unit-testable: the
/// guard reads/writes terminal state through this rather than calling the
/// syscalls directly, so a test can supply a fake and assert the *saved* state
/// is what gets re-applied.
pub trait TermiosPort {
    fn get(&self) -> io::Result<Termios>;
    fn set(&self, t: &Termios) -> io::Result<()>;
}

/// The production port: `tcgetattr`/`tcsetattr` on a real fd.
pub struct FdTermios(c_int);

impl TermiosPort for FdTermios {
    fn get(&self) -> io::Result<Termios> {
        let mut t = Termios::zeroed();
        // SAFETY: `&mut t` points to a valid `Termios` for the call's
        // duration. The return value is checked below rather than assumed: a
        // failed call leaves `t` all-zero, which is not "still a usable
        // value" -- an all-zero termios written back to the real terminal on
        // `Drop` would wreck it, so a failure here must not be treated as a
        // saved cooked state at all.
        let rc = unsafe { tcgetattr(self.0, &mut t) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(t)
    }

    fn set(&self, t: &Termios) -> io::Result<()> {
        // SAFETY: `t` is a valid `Termios`; the fd is the one we were built with.
        let rc = unsafe { tcsetattr(self.0, TCSAFLUSH, t) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// D5: saves the cooked termios on construction, switches to raw, and restores
/// the saved value in `Drop`, so `:quit`, Ctrl-D, an unwinding eval error, and
/// a panic all leave the terminal cooked.
pub struct RawModeGuard<P: TermiosPort> {
    port: P,
    saved: Termios,
}

impl<P: TermiosPort> RawModeGuard<P> {
    /// Fails rather than entering raw mode on a garbage/unreadable termios: a
    /// failed `tcgetattr` or `tcsetattr` here means there is nothing sound to
    /// restore later, so the caller gets an error instead of a guard that
    /// would write back zeroed state on `Drop`.
    pub fn new(port: P) -> io::Result<RawModeGuard<P>> {
        let saved = port.get()?;
        port.set(&raw_termios(&saved))?;
        Ok(RawModeGuard { port, saved })
    }
}

impl<P: TermiosPort> Drop for RawModeGuard<P> {
    fn drop(&mut self) {
        // Best-effort: there is no fallback if the restore itself fails, and
        // `Drop` cannot propagate an error.
        let _ = self.port.set(&self.saved);
    }
}

/// Put stdin (fd 0) into raw mode for the interactive session.
pub fn raw_mode_stdin() -> io::Result<RawModeGuard<FdTermios>> {
    RawModeGuard::new(FdTermios(0))
}

/// Read one byte from fd 0. `None` is EOF (Ctrl-D delivered by the terminal as
/// a zero-length read once ICANON is off would instead be `\x04`, so a real
/// zero read here is a closed stdin).
pub fn read_stdin_byte() -> io::Result<Option<u8>> {
    let mut b = [0u8; 1];
    // SAFETY: `b` is a valid 1-byte buffer for the call's duration.
    let n = unsafe { read(0, b.as_mut_ptr() as *mut c_void, 1) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(b[0]))
}

// --- pure key decoding -----------------------------------------------------

/// One decoded input event. `Char` carries a raw byte (the buffer is
/// UTF-8-naive, R4), everything else is a control key.
#[derive(Debug, PartialEq)]
enum Key {
    Char(u8),
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    Backspace,
    Delete,
    Enter,
    Tab,
    CtrlC,
    CtrlD,
    Unknown,
}

#[derive(Debug)]
enum Decoded {
    Key(Key, usize),
    /// The pending bytes are a proper prefix of an escape sequence; wait for
    /// more before deciding (never forward a partial sequence to the lexer).
    NeedMore,
}

/// A simple (parameter-less) CSI/SS3 final byte's nav key, shared between the
/// `ESC [ <final>` (no-params arm) and `ESC O <final>` (SS3) paths: in
/// DECCKM/application-cursor-key mode (tmux/screen and some terminal configs
/// enable this) a terminal sends arrows/Home/End via SS3 instead of CSI, and
/// they mean the same key either way.
fn simple_nav_key(final_byte: u8) -> Option<Key> {
    match final_byte {
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'C' => Some(Key::Right),
        b'D' => Some(Key::Left),
        b'H' => Some(Key::Home),
        b'F' => Some(Key::End),
        _ => None,
    }
}

/// A real CSI sequence (arrow keys, modified arrows, function keys,
/// bracketed-paste markers) is well under 32 bytes; capping the scan here
/// bounds both CPU and `pending`'s growth against an unterminated `ESC [`
/// followed by an unbounded run of parameter bytes (garbled input, a pasted
/// binary blob, or hostile input) -- without a cap, every incoming byte
/// re-scans the whole accumulated prefix from scratch (`decode` has no state
/// of its own across calls), which is O(N^2) total and, empirically, over a
/// hundred CPU-seconds for a million garbage bytes fed one at a time.
const CSI_MAX_LEN: usize = 64;

/// Decode the next key from the front of `b`. An arrow/nav escape sequence is
/// consumed here in full (R6): its bytes are turned into a `Key` and never
/// reach `lexer::lex`.
fn decode(b: &[u8]) -> Decoded {
    if b.is_empty() {
        return Decoded::NeedMore;
    }
    match b[0] {
        0x1b => {
            if b.len() < 2 {
                return Decoded::NeedMore;
            }
            if b[1] == b'O' {
                // SS3: exactly one final byte, no parameters (some
                // terminals' F1-F4, and DECCKM-mode arrows/Home/End).
                if b.len() < 3 {
                    return Decoded::NeedMore;
                }
                let key = simple_nav_key(b[2]).unwrap_or(Key::Unknown);
                return Decoded::Key(key, 3);
            }
            if b[1] == b'[' {
                if b.len() < 3 {
                    return Decoded::NeedMore;
                }
                // CSI: `ESC [`, then any number of parameter bytes
                // (0x30..=0x3F: digits, `;`, `:`, ...), then any number of
                // intermediate bytes (0x20..=0x2F), then exactly one final
                // byte (0x40..=0x7E). Scanned to the final byte before
                // deciding a `Key` so a modified arrow (`ESC[1;5C`,
                // Ctrl-Right), a multi-digit function key (`ESC[15~`, F5), or
                // a bracketed-paste marker (`ESC[200~`) is consumed as one
                // unit -- never partially, which is what let an escape tail
                // (e.g. `5C`) leak into the buffer and reach `lexer::lex`,
                // the exact thing R6 forbids. Bounded by `CSI_MAX_LEN`.
                let mut i = 2;
                while i < b.len() && i < CSI_MAX_LEN && (0x30..=0x3f).contains(&b[i]) {
                    i += 1;
                }
                while i < b.len() && i < CSI_MAX_LEN && (0x20..=0x2f).contains(&b[i]) {
                    i += 1;
                }
                if i >= CSI_MAX_LEN {
                    // No final byte within a plausible sequence length: give
                    // up and discard everything scanned so far rather than
                    // waiting (possibly forever) for more, or rescanning an
                    // ever-growing `pending` on every subsequent byte.
                    return Decoded::Key(Key::Unknown, i);
                }
                if i >= b.len() {
                    return Decoded::NeedMore;
                }
                if !(0x40..=0x7e).contains(&b[i]) {
                    // A byte outside every CSI byte range with no final byte
                    // seen yet: give up on this sequence rather than waiting
                    // forever, consuming through the offending byte.
                    return Decoded::Key(Key::Unknown, i + 1);
                }
                let final_byte = b[i];
                let consumed = i + 1;
                let params = &b[2..i];
                let key = match final_byte {
                    b'~' => match params {
                        b"1" | b"7" => Key::Home,
                        b"4" | b"8" => Key::End,
                        b"3" => Key::Delete,
                        _ => Key::Unknown,
                    },
                    f if params.is_empty() => simple_nav_key(f).unwrap_or(Key::Unknown),
                    _ => Key::Unknown,
                };
                return Decoded::Key(key, consumed);
            }
            Decoded::Key(Key::Unknown, 2)
        }
        b'\r' | b'\n' => Decoded::Key(Key::Enter, 1),
        b'\t' => Decoded::Key(Key::Tab, 1),
        0x7f | 0x08 => Decoded::Key(Key::Backspace, 1),
        0x03 => Decoded::Key(Key::CtrlC, 1),
        0x04 => Decoded::Key(Key::CtrlD, 1),
        0x01 => Decoded::Key(Key::Home, 1), // Ctrl-A
        0x05 => Decoded::Key(Key::End, 1),  // Ctrl-E
        // Printable ASCII and any high byte (a UTF-8 lead/continuation byte):
        // inserted verbatim into the byte-naive buffer.
        c if c >= 0x20 => Decoded::Key(Key::Char(c), 1),
        _ => Decoded::Key(Key::Unknown, 1),
    }
}

// --- history ---------------------------------------------------------------

const HISTORY_CAP: usize = 1000;

/// In-memory ring plus a persistent file (R5). Blank lines and an immediate
/// duplicate of the previous entry are not recorded; the file is capped to
/// `cap` lines and rewritten from the (already-capped) ring on each commit.
pub struct History {
    entries: Vec<String>,
    path: Option<PathBuf>,
    cap: usize,
}

impl History {
    pub fn load() -> History {
        History::load_from(history_path(), HISTORY_CAP)
    }

    fn load_from(path: Option<PathBuf>, cap: usize) -> History {
        let mut entries = Vec::new();
        if let Some(p) = &path {
            if let Ok(text) = fs::read_to_string(p) {
                for line in text.lines() {
                    if !line.is_empty() {
                        entries.push(line.to_string());
                    }
                }
            }
        }
        if entries.len() > cap {
            entries.drain(..entries.len() - cap);
        }
        History { entries, path, cap }
    }

    fn record(&mut self, line: &str) -> io::Result<()> {
        if line.trim().is_empty() {
            return Ok(());
        }
        if self.entries.last().is_some_and(|s| s == line) {
            return Ok(());
        }
        self.entries.push(line.to_string());
        if self.entries.len() > self.cap {
            self.entries.drain(..self.entries.len() - self.cap);
        }
        if let Some(p) = &self.path {
            fs::write(p, self.entries.join("\n") + "\n")?;
        }
        Ok(())
    }
}

fn history_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SOOTH_HISTORY") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".sooth_history"))
}

// --- editor ----------------------------------------------------------------

/// The result of feeding input: something the driver must act on. Cursor moves
/// and edits produce no `Action` (they only re-render), so the driver loops on
/// `None` until a line is committed or the session ends.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Enter: one committed logical line, handed to the shared dispatch helper.
    Commit(String),
    /// Ctrl-C: abandon the current line, do not exit the process.
    Abort,
    /// Ctrl-D on an empty line: end of input.
    Eof,
}

/// The interactive edit state: a UTF-8-naive byte buffer, a byte cursor, and
/// history navigation. `pending` holds undecoded input bytes so a multi-byte
/// escape sequence spanning several 1-byte reads still decodes as one key.
pub struct Editor {
    prompt: String,
    /// Shown instead of `prompt` while `pending_lines` is non-empty (R10).
    continuation_prompt: String,
    buf: Vec<u8>,
    cursor: usize,
    pending: Vec<u8>,
    history: History,
    /// `Some(i)` while walking history at entry `i`; `None` while editing a
    /// fresh line. `stash` holds the fresh line set aside on the first recall.
    hist_nav: Option<usize>,
    stash: Vec<u8>,
    /// Committed physical lines not yet forming a `Complete` logical line
    /// (slice 2, R9/R10). Joined with `\n` and re-checked after each Enter.
    pending_lines: Vec<String>,
    /// Pure predicate injected so this module stays decoupled from the lexer
    /// (per the growth-structure split): `true` iff the joined pending text
    /// is a complete logical line.
    is_complete: fn(&str) -> bool,
    /// R23: the current `:words` name list, refreshed by the driver after
    /// each dispatched line so a newly defined word is completable right
    /// away. Tab-only; empty until the driver first sets it.
    words: Vec<String>,
}

impl Editor {
    pub fn new(
        prompt: &str,
        continuation_prompt: &str,
        history: History,
        is_complete: fn(&str) -> bool,
    ) -> Editor {
        Editor {
            prompt: prompt.to_string(),
            continuation_prompt: continuation_prompt.to_string(),
            buf: Vec::new(),
            cursor: 0,
            pending: Vec::new(),
            history,
            hist_nav: None,
            stash: Vec::new(),
            pending_lines: Vec::new(),
            is_complete,
            words: Vec::new(),
        }
    }

    /// R23: replace the word list Tab completes against.
    pub fn set_words(&mut self, words: Vec<String>) {
        self.words = words;
    }

    /// Feed one input byte; decode and apply as many complete keys as it
    /// completes, re-rendering to `out`. Returns an `Action` the moment one is
    /// produced (Enter/Ctrl-C/Ctrl-D).
    pub fn push_byte(&mut self, b: u8, out: &mut impl Write) -> io::Result<Option<Action>> {
        self.pending.push(b);
        loop {
            match decode(&self.pending) {
                Decoded::NeedMore => return Ok(None),
                Decoded::Key(key, consumed) => {
                    self.pending.drain(..consumed);
                    if let Some(action) = self.apply(key, out)? {
                        return Ok(Some(action));
                    }
                    if self.pending.is_empty() {
                        return Ok(None);
                    }
                }
            }
        }
    }

    fn apply(&mut self, key: Key, out: &mut impl Write) -> io::Result<Option<Action>> {
        match key {
            Key::Char(c) => {
                self.buf.insert(self.cursor, c);
                self.cursor += 1;
                self.redraw(out)?;
            }
            Key::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.redraw(out)?;
                }
            }
            Key::Right => {
                if self.cursor < self.buf.len() {
                    self.cursor += 1;
                    self.redraw(out)?;
                }
            }
            Key::Home => {
                self.cursor = 0;
                self.redraw(out)?;
            }
            Key::End => {
                self.cursor = self.buf.len();
                self.redraw(out)?;
            }
            Key::Backspace => {
                if self.cursor > 0 {
                    self.buf.remove(self.cursor - 1);
                    self.cursor -= 1;
                    self.redraw(out)?;
                }
            }
            Key::Delete => {
                if self.cursor < self.buf.len() {
                    self.buf.remove(self.cursor);
                    self.redraw(out)?;
                }
            }
            Key::Tab => {
                self.complete(out)?;
            }
            Key::Up => {
                self.history_prev();
                self.redraw(out)?;
            }
            Key::Down => {
                self.history_next();
                self.redraw(out)?;
            }
            Key::Enter => {
                let line = String::from_utf8_lossy(&self.buf).into_owned();
                // A history-file write failure (a read-only `$HOME`, a
                // `SOOTH_HISTORY` pointing at a directory, a full disk) is a
                // best-effort convenience, not part of the eval contract: it
                // must never end the session or skip the scope-end linear
                // disposal further up the call stack (`run_tty` -> `end_session`),
                // which an `Err` propagated out of here would do by bypassing
                // that whole path.
                if let Err(e) = self.history.record(&line) {
                    out.write_all(format!("\r\n(history not saved: {e})\r\n").as_bytes())?;
                }
                self.reset_line();
                self.pending_lines.push(line);
                let joined = self.pending_lines.join("\n");
                if (self.is_complete)(&joined) {
                    // R10: a complete logical line, possibly joined from
                    // several physical lines, dispatched as one unit.
                    self.pending_lines.clear();
                    return Ok(Some(Action::Commit(joined)));
                }
                // NeedMore: buffer the physical line and switch to the
                // continuation prompt instead of compiling a partial line.
                // The typed line must scroll up (like Commit/Abort/Eof do)
                // before redrawing, or the continuation prompt overwrites it.
                out.write_all(b"\r\n")?;
                self.redraw(out)?;
            }
            Key::CtrlC => {
                self.reset_line();
                // R11: Ctrl-C discards any pending multi-line buffer too,
                // not just the current line, returning to the primary prompt.
                self.pending_lines.clear();
                return Ok(Some(Action::Abort));
            }
            Key::CtrlD => {
                // Empty line: EOF. Mid-line: inert (a stray Ctrl-D shouldn't
                // eat a character you're editing).
                if self.buf.is_empty() {
                    return Ok(Some(Action::Eof));
                }
            }
            Key::Unknown => {}
        }
        Ok(None)
    }

    /// R23: complete the word ending at the cursor against `self.words`. The
    /// word being typed is the byte-naive whitespace-delimited token
    /// immediately before the cursor; the first name in the (sorted) list
    /// that has it as a proper prefix is spliced in. A no-op if nothing
    /// matches, or if the token already equals a full word.
    fn complete(&mut self, out: &mut impl Write) -> io::Result<()> {
        let typed = String::from_utf8_lossy(&self.buf[..self.cursor]).into_owned();
        let start = typed
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let prefix = &typed[start..];
        if prefix.is_empty() {
            return Ok(());
        }
        let Some(word) = self
            .words
            .iter()
            .find(|w| w.starts_with(prefix) && w.as_str() != prefix)
        else {
            return Ok(());
        };
        for &b in &word.as_bytes()[prefix.len()..] {
            self.buf.insert(self.cursor, b);
            self.cursor += 1;
        }
        self.redraw(out)
    }

    fn reset_line(&mut self) {
        self.buf.clear();
        self.cursor = 0;
        self.hist_nav = None;
        self.stash.clear();
    }

    fn history_prev(&mut self) {
        if self.history.entries.is_empty() {
            return;
        }
        let idx = match self.hist_nav {
            None => {
                self.stash = self.buf.clone();
                self.history.entries.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.hist_nav = Some(idx);
        // A recalled entry is cloned into the buffer, so editing it never
        // mutates the stored entry (R5).
        self.buf = self.history.entries[idx].clone().into_bytes();
        self.cursor = self.buf.len();
    }

    fn history_next(&mut self) {
        let Some(i) = self.hist_nav else {
            return;
        };
        if i + 1 < self.history.entries.len() {
            self.hist_nav = Some(i + 1);
            self.buf = self.history.entries[i + 1].clone().into_bytes();
        } else {
            self.hist_nav = None;
            self.buf = std::mem::take(&mut self.stash);
        }
        self.cursor = self.buf.len();
    }

    /// Rewrite the current line in place: carriage-return to column 0, redraw
    /// prompt + buffer, erase any trailing remnant, then reposition the cursor.
    /// Assumes an ASCII prompt (its byte length is its column width).
    pub fn redraw(&self, out: &mut impl Write) -> io::Result<()> {
        let prompt = if self.pending_lines.is_empty() {
            &self.prompt
        } else {
            &self.continuation_prompt
        };
        out.write_all(b"\r")?;
        out.write_all(prompt.as_bytes())?;
        out.write_all(&self.buf)?;
        out.write_all(b"\x1b[K")?;
        out.write_all(b"\r")?;
        let col = prompt.len() + self.cursor;
        if col > 0 {
            write!(out, "\x1b[{col}C")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn editor(history: History) -> Editor {
        Editor::new("> ", "... ", history, |_| true)
    }

    fn empty_history() -> History {
        History::load_from(None, HISTORY_CAP)
    }

    /// Drive a byte script against an editor, discarding rendered output,
    /// returning any actions produced.
    fn feed(ed: &mut Editor, input: &[u8]) -> Vec<Action> {
        let mut sink = Vec::new();
        let mut actions = Vec::new();
        for &b in input {
            if let Some(a) = ed.push_byte(b, &mut sink).unwrap() {
                actions.push(a);
            }
        }
        actions
    }

    #[test]
    fn editor_arrow_keys_move_cursor_expected() {
        let mut ed = editor(empty_history());
        feed(&mut ed, b"abc");
        assert_eq!(ed.cursor, 3);
        feed(&mut ed, b"\x1b[D"); // left
        assert_eq!(ed.cursor, 2);
        feed(&mut ed, b"\x1b[D\x1b[D"); // left left
        assert_eq!(ed.cursor, 0);
        feed(&mut ed, b"\x1b[C"); // right
        assert_eq!(ed.cursor, 1);
        feed(&mut ed, b"\x1b[F"); // end
        assert_eq!(ed.cursor, 3);
        feed(&mut ed, b"\x1b[H"); // home
        assert_eq!(ed.cursor, 0);
    }

    #[test]
    fn editor_backspace_deletes_char_before_cursor() {
        let mut ed = editor(empty_history());
        feed(&mut ed, b"abc");
        feed(&mut ed, b"\x7f");
        assert_eq!(ed.buf, b"ab");
        assert_eq!(ed.cursor, 2);
        // Backspace mid-line removes the char before the cursor, not at it.
        feed(&mut ed, b"\x1b[D"); // cursor between a and b
        feed(&mut ed, b"\x7f");
        assert_eq!(ed.buf, b"b");
        assert_eq!(ed.cursor, 0);
    }

    #[test]
    fn editor_up_arrow_recalls_previous_entry() {
        let mut h = empty_history();
        h.record("foo").unwrap();
        h.record("bar").unwrap();
        let mut ed = editor(h);
        feed(&mut ed, b"\x1b[A"); // up
        assert_eq!(ed.buf, b"bar");
        assert_eq!(ed.cursor, 3);
        feed(&mut ed, b"\x1b[A"); // up again
        assert_eq!(ed.buf, b"foo");
        feed(&mut ed, b"\x1b[B"); // down
        assert_eq!(ed.buf, b"bar");

        // R5: editing a recalled entry must not mutate the stored one -- the
        // recall clones into the buffer, so this makes that clone
        // load-bearing rather than incidental.
        feed(&mut ed, b"\x7f"); // backspace: buf becomes "ba"
        assert_eq!(ed.buf, b"ba");
        assert_eq!(
            ed.history.entries,
            vec!["foo".to_string(), "bar".to_string()],
            "editing a recalled entry must not mutate the stored history"
        );
    }

    #[test]
    fn editor_escape_sequence_not_forwarded_to_lexer() {
        let mut ed = editor(empty_history());
        // An arrow escape in the middle of a real line must be swallowed: it
        // never lands in the buffer, so the committed line lexes cleanly.
        feed(&mut ed, b"1 2");
        feed(&mut ed, b"\x1b[C"); // right at end: no-op, escape consumed
        feed(&mut ed, b" +");
        let actions = feed(&mut ed, b"\r");
        let line = match &actions[..] {
            [Action::Commit(l)] => l.clone(),
            other => panic!("expected one Commit, got {other:?}"),
        };
        assert_eq!(line, "1 2 +");
        assert!(
            !line.as_bytes().contains(&0x1b),
            "escape byte reached lexer"
        );
        crate::lexer::lex(&line).expect("committed line lexes cleanly");
    }

    #[test]
    fn editor_parameterized_csi_sequences_leave_no_tail_bytes() {
        // R6: a modified arrow (Ctrl-Right), a multi-digit function key
        // (F5), and a bracketed-paste marker must each be consumed as one
        // unit, with zero leaked bytes reaching the buffer -- the bug this
        // guards against left a literal `5C` (or a bare `~`) typed into the
        // line, because the old decoder assumed every CSI sequence was
        // exactly 4 bytes.
        for seq in [
            &b"\x1b[1;5C"[..], // Ctrl-Right
            &b"\x1b[15~"[..],  // F5
            &b"\x1b[200~"[..], // bracketed-paste start marker
        ] {
            let mut ed = editor(empty_history());
            feed(&mut ed, b"ab");
            feed(&mut ed, seq);
            feed(&mut ed, b"cd");
            assert_eq!(
                ed.buf, b"abcd",
                "sequence {seq:?} leaked bytes into the buffer: {:?}",
                ed.buf
            );
        }
    }

    #[test]
    fn editor_unterminated_csi_sequence_is_capped_not_unbounded() {
        // Without CSI_MAX_LEN, an unterminated `ESC [` followed by a run of
        // parameter bytes and no final byte rescans the whole accumulated
        // prefix from scratch on every incoming byte (empirically, 100+ CPU
        // seconds for 1,000,000 garbage bytes fed one at a time). Called
        // directly on `decode` for a precise, deterministic bound: it must
        // give up at `CSI_MAX_LEN`, not scan the full (much longer) input.
        let mut seq = vec![0x1b, b'['];
        seq.extend(std::iter::repeat_n(b'9', 300));
        match decode(&seq) {
            Decoded::Key(Key::Unknown, consumed) => {
                assert_eq!(
                    consumed, CSI_MAX_LEN,
                    "an unterminated CSI sequence must be abandoned at the cap, not scan indefinitely"
                );
            }
            other => panic!("expected Key::Unknown capped at {CSI_MAX_LEN}, got {other:?}"),
        }

        // End to end through the real byte-at-a-time driver: this must
        // complete promptly (no O(N^2) rescan) and never let `pending` grow
        // past the cap.
        let mut ed = editor(empty_history());
        feed(&mut ed, &[0x1b, b'[']);
        for _ in 0..300 {
            feed(&mut ed, b"9");
            assert!(
                ed.pending.len() <= CSI_MAX_LEN,
                "pending grew past the cap: {}",
                ed.pending.len()
            );
        }
    }

    #[test]
    fn editor_ss3_arrow_keys_decode_same_as_csi() {
        // DECCKM/application-cursor-key mode (tmux/screen and some terminal
        // configs) sends arrows/Home/End via SS3 (`ESC O <letter>`) rather
        // than CSI; they must decode to the same key as their CSI form.
        let mut ed_ss3 = editor(empty_history());
        feed(&mut ed_ss3, b"abc");
        feed(&mut ed_ss3, b"\x1bOD"); // SS3 left
        assert_eq!(ed_ss3.cursor, 2, "SS3 left did not move the cursor");

        let mut ed_csi = editor(empty_history());
        feed(&mut ed_csi, b"abc");
        feed(&mut ed_csi, b"\x1b[D"); // CSI left
        assert_eq!(
            ed_ss3.cursor, ed_csi.cursor,
            "SS3 and CSI left must land the cursor identically"
        );

        let mut ed_right = editor(empty_history());
        feed(&mut ed_right, b"abc");
        feed(&mut ed_right, b"\x1b[D\x1b[D"); // move left twice
        feed(&mut ed_right, b"\x1bOC"); // SS3 right
        assert_eq!(ed_right.cursor, 2, "SS3 right did not move the cursor");

        // An SS3 sequence still never leaks into the buffer (R6).
        assert_eq!(ed_ss3.buf, b"abc");
        assert_eq!(ed_right.buf, b"abc");
    }

    #[test]
    fn editor_ctrl_c_abandons_line_not_process() {
        let mut ed = editor(empty_history());
        feed(&mut ed, b"abc");
        let actions = feed(&mut ed, b"\x03");
        assert_eq!(actions, vec![Action::Abort]);
        assert!(ed.buf.is_empty());
        assert_eq!(ed.cursor, 0);
    }

    #[test]
    fn editor_multiline_def_submits_as_one_line() {
        // R10/R14: real completeness predicate, driven by two physical lines
        // of an unclosed `:` definition followed by the closing `;`.
        let mut ed = Editor::new("> ", "... ", empty_history(), crate::repl::text_is_complete);
        let none = feed(&mut ed, b": sq ( i64 -- i64 )\r");
        assert!(none.is_empty(), "unclosed def must not commit yet");
        let actions = feed(&mut ed, b"dup * ;\r");
        assert_eq!(
            actions,
            vec![Action::Commit(": sq ( i64 -- i64 )\ndup * ;".to_string())]
        );
    }

    #[test]
    fn editor_continuation_redraw_scrolls_past_prior_line() {
        // The first physical line must not be clobbered by the continuation
        // prompt: NeedMore has to emit \r\n before redrawing, exactly like
        // Commit/Abort/Eof do in the tty loop, so the line scrolls up.
        let mut ed = Editor::new("> ", "... ", empty_history(), crate::repl::text_is_complete);
        let mut sink = Vec::new();
        for &b in b": sq ( i64 -- i64 )" {
            ed.push_byte(b, &mut sink).unwrap();
        }
        sink.clear();
        let action = ed.push_byte(b'\r', &mut sink).unwrap();
        assert!(action.is_none(), "unclosed def must not commit yet");
        assert!(
            sink.starts_with(b"\r\n"),
            "continuation redraw must scroll past the typed line, got {sink:?}"
        );
    }

    #[test]
    fn continuation_ctrl_c_discards_pending_buffer() {
        // R11: Ctrl-C with a non-empty pending multi-line buffer discards the
        // buffer (not the process) and returns to the primary prompt.
        let mut ed = Editor::new("> ", "... ", empty_history(), crate::repl::text_is_complete);
        let none = feed(&mut ed, b": sq ( i64 -- i64 )\r");
        assert!(none.is_empty());
        assert!(!ed.pending_lines.is_empty());

        let actions = feed(&mut ed, b"\x03");
        assert_eq!(actions, vec![Action::Abort]);
        assert!(ed.pending_lines.is_empty());

        // A fresh, complete line after the abort commits normally, proving
        // the process (and the editor) survived.
        let actions = feed(&mut ed, b"1 2 +\r");
        assert_eq!(actions, vec![Action::Commit("1 2 +".to_string())]);
    }

    #[test]
    fn editor_ctrl_d_empty_line_is_eof() {
        let mut ed = editor(empty_history());
        let actions = feed(&mut ed, b"\x04");
        assert_eq!(actions, vec![Action::Eof]);
        // Mid-line Ctrl-D is inert: no action, buffer untouched.
        feed(&mut ed, b"xy");
        let none = feed(&mut ed, b"\x04");
        assert!(none.is_empty());
        assert_eq!(ed.buf, b"xy");
    }

    #[test]
    fn editor_tab_completes_against_word_list() {
        let mut ed = editor(empty_history());
        ed.set_words(vec!["square".to_string(), "squash".to_string()]);
        feed(&mut ed, b"sq");
        feed(&mut ed, b"\t");
        assert_eq!(ed.buf, b"square");
        // No match: a no-op, buffer untouched.
        feed(&mut ed, b" zz");
        let before = ed.buf.clone();
        feed(&mut ed, b"\t");
        assert_eq!(ed.buf, before);
    }

    #[test]
    fn editor_unwritable_history_path_does_not_abort_commit() {
        // A `SOOTH_HISTORY` pointing at a directory (or any other unwritable
        // path) must not turn a history-write failure into a fatal error:
        // Enter still commits the line -- the process (and the eventual
        // `end_session` disposal) survives, just without persisting it.
        let dir =
            std::env::temp_dir().join(format!("sooth-hist-unwritable-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap(); // a directory: `fs::write` on it fails
        let history = History::load_from(Some(dir.clone()), HISTORY_CAP);
        let mut ed = Editor::new("> ", "... ", history, |_| true);
        let mut out = Vec::new();
        for &b in b"1 2 +" {
            ed.push_byte(b, &mut out).unwrap();
        }
        let action = ed.push_byte(b'\r', &mut out).unwrap();
        assert_eq!(action, Some(Action::Commit("1 2 +".to_string())));
        let rendered = String::from_utf8_lossy(&out);
        assert!(
            rendered.contains("history not saved"),
            "expected a non-fatal warning, got: {rendered}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn editor_history_file_roundtrips_capped() {
        let dir = std::env::temp_dir().join(format!("sooth-hist-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history");
        let _ = fs::remove_file(&path);

        let mut h = History::load_from(Some(path.clone()), 3);
        h.record("a").unwrap();
        h.record("").unwrap(); // blank: not recorded
        h.record("b").unwrap();
        h.record("b").unwrap(); // immediate dup: not recorded
        h.record("c").unwrap();
        h.record("d").unwrap(); // overflows cap 3: "a" drops
        assert_eq!(h.entries, vec!["b", "c", "d"]);

        // Round-trip: a fresh session reads the same capped, deduped ring.
        let reloaded = History::load_from(Some(path.clone()), 3);
        assert_eq!(reloaded.entries, vec!["b", "c", "d"]);

        let _ = fs::remove_dir_all(&dir);
    }

    /// A fake `TermiosPort` over a shared cell, so a test can observe what the
    /// guard applies after the guard itself has been dropped.
    struct FakePort(Rc<RefCell<Termios>>);

    impl TermiosPort for FakePort {
        fn get(&self) -> io::Result<Termios> {
            Ok(*self.0.borrow())
        }
        fn set(&self, t: &Termios) -> io::Result<()> {
            *self.0.borrow_mut() = *t;
            Ok(())
        }
    }

    #[test]
    fn raw_mode_guard_restores_saved_termios_on_drop() {
        let mut cooked = Termios::zeroed();
        cooked.c_lflag = flags::ISIG | flags::ICANON | flags::ECHO;
        let shared = Rc::new(RefCell::new(cooked));

        {
            let _guard = RawModeGuard::new(FakePort(shared.clone())).unwrap();
            // While the guard lives, the terminal is raw: echo/canonical off.
            assert_eq!(shared.borrow().c_lflag & flags::ECHO, 0);
            assert_eq!(shared.borrow().c_lflag & flags::ICANON, 0);
        }

        // On drop, the saved cooked state is re-applied verbatim.
        assert_eq!(*shared.borrow(), cooked);
    }

    /// A `TermiosPort` whose `get`/`set` always fail, so `RawModeGuard::new`
    /// has something real to propagate instead of assuming success.
    struct FailingPort;

    impl TermiosPort for FailingPort {
        fn get(&self) -> io::Result<Termios> {
            Err(io::Error::other("tcgetattr failed"))
        }
        fn set(&self, _t: &Termios) -> io::Result<()> {
            Err(io::Error::other("tcsetattr failed"))
        }
    }

    #[test]
    fn raw_mode_guard_new_propagates_a_failed_tcgetattr() {
        // A false `SAFETY` comment used to claim a failed `tcgetattr` "still
        // transforms into a usable value"; it does not, and this asserts the
        // failure surfaces as an `Err` rather than a `RawModeGuard` built on
        // a zeroed termios that would later be written back to a real
        // terminal on `Drop`.
        assert!(RawModeGuard::new(FailingPort).is_err());
    }

    /// A `TermiosPort` whose `get` succeeds but `set` always fails, isolating
    /// the `tcsetattr`-failure half of `RawModeGuard::new` from the
    /// `tcgetattr`-failure half above.
    struct GetOkSetFailsPort;

    impl TermiosPort for GetOkSetFailsPort {
        fn get(&self) -> io::Result<Termios> {
            Ok(Termios::zeroed())
        }
        fn set(&self, _t: &Termios) -> io::Result<()> {
            Err(io::Error::other("tcsetattr failed"))
        }
    }

    #[test]
    fn raw_mode_guard_new_propagates_a_failed_tcsetattr() {
        assert!(RawModeGuard::new(GetOkSetFailsPort).is_err());
    }
}
