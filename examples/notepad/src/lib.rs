//! notepad-chaton — a persistent scratch notepad, as a WASM component.
//!
//! Loads its notes on open and saves on Esc — through the host's `read-file` / `write-file`.
//! Note `host::read_file` returns `Option<String>` *directly* (the WIT type), no buffer dance:
//! the win of moving to the component model.

// wit-bindgen 0.36's generated export glue isn't edition-2024 lint-clean yet (unsafe-in-unsafe).
#![allow(unsafe_op_in_unsafe_fn)]

wit_bindgen::generate!({ world: "chaton", path: "../../wit" });

use chatons::plugin::host;
use std::cell::RefCell;

const PATH: &str = "/tmp/chaton-notes.txt";

struct Notepad {
    buf: String,
}

impl Notepad {
    fn new() -> Self {
        Notepad { buf: host::read_file(PATH).unwrap_or_default() }
    }

    fn draw(&self) {
        host::render(&format!(
            "  📝 chatons notepad  →  {PATH}\n  ────────────────────────────────────────\n{}▌\n\n  loads on open · type freely · Backspace · Esc saves & quits",
            self.buf
        ));
    }
}

thread_local! {
    static STATE: RefCell<Notepad> = RefCell::new(Notepad::new());
}

struct App;

impl Guest for App {
    fn init() {
        STATE.with_borrow(|s| s.draw());
    }

    fn on_key(k: Key) -> bool {
        STATE.with_borrow_mut(|s| {
            match k {
                Key::Escape => {
                    host::write_file(PATH, &s.buf);
                    return false;
                }
                Key::Enter => s.buf.push('\n'),
                Key::Backspace => {
                    s.buf.pop();
                }
                Key::Text(c) => s.buf.push(c),
                Key::Other(_) => {}
            }
            s.draw();
            true
        })
    }
}

export!(App);
