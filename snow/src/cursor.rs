//! The terminal's text cursor: hidden while snow runs, shown again when it
//! waits for an answer and when it exits

use std::io::{IsTerminal, Stderr, Write};
use std::os::fd::{AsRawFd, RawFd};

const HIDE: &[u8] = b"\x1b[?25l";
const SHOW: &[u8] = b"\x1b[?25h";

pub struct Cursor<W: Write> {
    out: W,
    interactive: bool,
}

impl Cursor<Stderr> {
    pub fn stderr() -> Self {
        let out = std::io::stderr();
        let interactive = out.is_terminal();
        Self::new(out, interactive)
    }
}

impl<W: Write> Cursor<W> {
    pub fn new(out: W, interactive: bool) -> Self {
        Self { out, interactive }
    }

    pub fn hide(&mut self) {
        self.emit(HIDE);
    }

    pub fn show(&mut self) {
        self.emit(SHOW);
    }

    pub fn shown_while<T>(&mut self, wait: impl FnOnce() -> T) -> T {
        self.show();
        let waited = wait();
        self.hide();
        waited
    }

    /// A cursor left hidden by a failed write is only cosmetic, so it's no
    /// reason to stop
    fn emit(&mut self, escape: &[u8]) {
        if self.interactive {
            let _ = self.out.write_all(escape);
            let _ = self.out.flush();
        }
    }
}

/// Keeps the cursor hidden until dropped
pub struct HiddenCursor<W: Write>(Cursor<W>);

impl<W: Write> HiddenCursor<W> {
    pub fn new(mut cursor: Cursor<W>) -> Self {
        cursor.hide();
        Self(cursor)
    }
}

impl<W: Write> Drop for HiddenCursor<W> {
    fn drop(&mut self) {
        self.0.show();
    }
}

/// Drops the cursor escapes of prompts that hide the cursor while they wait
/// for a key. An unbuffered console Term writes each escape on its own, so
/// matching whole writes is enough.
#[derive(Debug)]
pub struct KeepsCursor<W: Write>(pub W);

impl<W: Write> Write for KeepsCursor<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf == HIDE || buf == SHOW {
            Ok(buf.len())
        } else {
            self.0.write(buf)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl<W: Write + AsRawFd> AsRawFd for KeepsCursor<W> {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

/// For exits that skip destructors, and so the `HiddenCursor` guard
pub fn exit(code: i32) -> ! {
    Cursor::stderr().show();
    std::process::exit(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(interactive: bool, act: impl FnOnce(&mut Cursor<&mut Vec<u8>>)) -> Vec<u8> {
        let mut out = Vec::new();
        act(&mut Cursor::new(&mut out, interactive));
        out
    }

    #[test]
    fn test_cursor_is_shown_while_waiting_and_hidden_after() {
        let mut answer = None;
        let out = written(true, |cursor| {
            answer = Some(cursor.shown_while(|| "yes"));
        });

        assert_eq!(answer, Some("yes"));
        assert_eq!(out, b"\x1b[?25h\x1b[?25l");
    }

    #[test]
    fn test_prompt_cursor_escapes_are_dropped_and_its_text_kept() {
        let mut out = KeepsCursor(Vec::new());

        for part in [HIDE, b"Switch anyway? [y/N] ".as_slice(), SHOW, b"no\n"] {
            out.write_all(part).unwrap();
        }

        assert_eq!(out.0, b"Switch anyway? [y/N] no\n");
    }

    #[test]
    fn test_console_term_cursor_escapes_are_dropped() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let out = KeepsCursor(file.reopen().unwrap());
        let term = dialoguer::console::Term::read_write_pair(std::io::stdin(), out);

        term.hide_cursor().unwrap();
        term.write_str("What now?").unwrap();
        term.show_cursor().unwrap();

        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), "What now?");
    }

    #[test]
    fn test_nothing_is_written_off_a_terminal() {
        let out = written(false, |cursor| {
            cursor.hide();
            cursor.show();
        });

        assert!(out.is_empty());
    }

    #[test]
    fn test_hidden_cursor_hides_at_once_and_shows_when_dropped() {
        let mut out = Vec::new();
        {
            let _hidden = HiddenCursor::new(Cursor::new(&mut out, true));
        }

        assert_eq!(out, b"\x1b[?25l\x1b[?25h");
    }

    #[test]
    fn test_hidden_cursor_shows_when_dropped_by_a_panic() {
        let mut out = Vec::new();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _hidden = HiddenCursor::new(Cursor::new(&mut out, true));
            panic!("command failed");
        }));

        assert!(unwound.is_err());
        assert_eq!(out, b"\x1b[?25l\x1b[?25h");
    }

    #[test]
    fn test_hidden_cursor_writes_nothing_off_a_terminal() {
        let mut out = Vec::new();
        drop(HiddenCursor::new(Cursor::new(&mut out, false)));

        assert!(out.is_empty());
    }
}
