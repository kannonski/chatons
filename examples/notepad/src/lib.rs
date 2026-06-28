//! notepad-chaton — a scratch notepad, written against `chaton-sdk`.
//!
//! Type freely, Backspace deletes, Esc saves to a file and quits. A real (if minimal) use of a
//! chaton: it captures text input and persists to disk through the host's `write_file`. Append
//! /backspace only for now (no cursor movement); loading the file back waits on the host→guest
//! data direction.

use chaton_sdk::{Chaton, Flow, Key, View, chaton, write_file};

const PATH: &str = "/tmp/chaton-notes.txt";

struct Notepad {
    buf: String,
}

impl Chaton for Notepad {
    fn new() -> Self {
        Notepad { buf: String::new() }
    }

    fn on_key(&mut self, key: Key) -> Flow {
        match key {
            Key::Esc => {
                write_file(PATH, &self.buf);
                return Flow::Quit;
            }
            Key::Enter => self.buf.push('\n'),
            Key::Backspace => {
                self.buf.pop();
            }
            Key::Char(c) => self.buf.push(c),
            Key::Other(_) => {}
        }
        Flow::Continue
    }

    fn render(&self) -> View {
        View::text(format!(
            "  📝 chatons notepad  →  {PATH}\n  ────────────────────────────────────────\n{}▌\n\n  type freely · Backspace deletes · Esc saves & quits",
            self.buf
        ))
    }
}

chaton!(Notepad);
