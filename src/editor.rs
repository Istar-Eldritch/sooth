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
    fn get(&self) -> Termios;
    fn set(&self, t: &Termios);
}

/// The production port: `tcgetattr`/`tcsetattr` on a real fd.
pub struct FdTermios(c_int);

impl TermiosPort for FdTermios {
    fn get(&self) -> Termios {
        let mut t = Termios::zeroed();
        // SAFETY: `&mut t` points to a valid `Termios`; a failed call leaves it
        // zeroed, which `raw_termios` still transforms into a usable value.
        unsafe {
            tcgetattr(self.0, &mut t);
        }
        t
    }

    fn set(&self, t: &Termios) {
        // SAFETY: `t` is a valid `Termios`; the fd is the one we were built with.
        unsafe {
            tcsetattr(self.0, TCSAFLUSH, t);
        }
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
    pub fn new(port: P) -> RawModeGuard<P> {
        let saved = port.get();
        port.set(&raw_termios(&saved));
        RawModeGuard { port, saved }
    }
}

impl<P: TermiosPort> Drop for RawModeGuard<P> {
    fn drop(&mut self) {
        self.port.set(&self.saved);
    }
}

/// Put stdin (fd 0) into raw mode for the interactive session.
pub fn raw_mode_stdin() -> RawModeGuard<FdTermios> {
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
    CtrlC,
    CtrlD,
    Unknown,
}

enum Decoded {
    Key(Key, usize),
    /// The pending bytes are a proper prefix of an escape sequence; wait for
    /// more before deciding (never forward a partial sequence to the lexer).
    NeedMore,
}

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
            if b[1] == b'[' || b[1] == b'O' {
                if b.len() < 3 {
                    return Decoded::NeedMore;
                }
                match b[2] {
                    b'A' => Decoded::Key(Key::Up, 3),
                    b'B' => Decoded::Key(Key::Down, 3),
                    b'C' => Decoded::Key(Key::Right, 3),
                    b'D' => Decoded::Key(Key::Left, 3),
                    b'H' => Decoded::Key(Key::Home, 3),
                    b'F' => Decoded::Key(Key::End, 3),
                    d @ b'0'..=b'9' => {
                        // A `CSI n ~` sequence (e.g. `\x1b[3~` = Delete).
                        if b.len() < 4 {
                            return Decoded::NeedMore;
                        }
                        if b[3] == b'~' {
                            let key = match d {
                                b'1' | b'7' => Key::Home,
                                b'4' | b'8' => Key::End,
                                b'3' => Key::Delete,
                                _ => Key::Unknown,
                            };
                            Decoded::Key(key, 4)
                        } else {
                            Decoded::Key(Key::Unknown, 4)
                        }
                    }
                    _ => Decoded::Key(Key::Unknown, 3),
                }
            } else {
                Decoded::Key(Key::Unknown, 2)
            }
        }
        b'\r' | b'\n' => Decoded::Key(Key::Enter, 1),
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
    buf: Vec<u8>,
    cursor: usize,
    pending: Vec<u8>,
    history: History,
    /// `Some(i)` while walking history at entry `i`; `None` while editing a
    /// fresh line. `stash` holds the fresh line set aside on the first recall.
    hist_nav: Option<usize>,
    stash: Vec<u8>,
}

impl Editor {
    pub fn new(prompt: &str, history: History) -> Editor {
        Editor {
            prompt: prompt.to_string(),
            buf: Vec::new(),
            cursor: 0,
            pending: Vec::new(),
            history,
            hist_nav: None,
            stash: Vec::new(),
        }
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
                self.history.record(&line)?;
                self.reset_line();
                return Ok(Some(Action::Commit(line)));
            }
            Key::CtrlC => {
                self.reset_line();
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
        out.write_all(b"\r")?;
        out.write_all(self.prompt.as_bytes())?;
        out.write_all(&self.buf)?;
        out.write_all(b"\x1b[K")?;
        out.write_all(b"\r")?;
        let col = self.prompt.len() + self.cursor;
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
        Editor::new("> ", history)
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
    fn editor_ctrl_c_abandons_line_not_process() {
        let mut ed = editor(empty_history());
        feed(&mut ed, b"abc");
        let actions = feed(&mut ed, b"\x03");
        assert_eq!(actions, vec![Action::Abort]);
        assert!(ed.buf.is_empty());
        assert_eq!(ed.cursor, 0);
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
        fn get(&self) -> Termios {
            *self.0.borrow()
        }
        fn set(&self, t: &Termios) {
            *self.0.borrow_mut() = *t;
        }
    }

    #[test]
    fn raw_mode_guard_restores_saved_termios_on_drop() {
        let mut cooked = Termios::zeroed();
        cooked.c_lflag = flags::ISIG | flags::ICANON | flags::ECHO;
        let shared = Rc::new(RefCell::new(cooked));

        {
            let _guard = RawModeGuard::new(FakePort(shared.clone()));
            // While the guard lives, the terminal is raw: echo/canonical off.
            assert_eq!(shared.borrow().c_lflag & flags::ECHO, 0);
            assert_eq!(shared.borrow().c_lflag & flags::ICANON, 0);
        }

        // On drop, the saved cooked state is re-applied verbatim.
        assert_eq!(*shared.borrow(), cooked);
    }
}
