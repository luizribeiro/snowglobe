//! The terminal's text cursor: hidden while snow runs, shown again when it
//! waits for an answer and when it exits

use std::io::{IsTerminal, Stderr, Write};

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
