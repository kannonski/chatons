//! notepad-chaton — a persistent scratch notepad, written against `chaton-sdk`.
//!
//! Loads its notes on open (`read_file`), type freely, Backspace deletes, Esc saves and quits
//! (`write_file`). A real use of a chaton: text input + persistence, both directions of the
//! host data bridge. Append/backspace only for now (no cursor movement).

use chaton_sdk::{Chaton, Flow, Key, View, chaton, read_file, write_file};

const PATH: &str = "/tmp/chaton-notes.txt";

struct Notepad {
    buf: String,
}

impl Chaton for Notepad {
    fn new() -> Self {
        Notepad { buf: read_file(PATH).unwrap_or_default() }
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
            "  📝 chatons notepad  →  {PATH}\n  ────────────────────────────────────────\n{}▌\n\n  loads on open · type freely · Backspace · Esc saves & quits",
            self.buf
        ))
    }
}

chaton!(Notepad);
