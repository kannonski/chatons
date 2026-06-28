//! hello-chaton — the reference chaton, written against `chaton-sdk`.
//!
//! No `unsafe`, no `extern`, no `#[no_mangle]` — just a struct + the `Chaton` trait + `chaton!`.
//! `n` opens a kitty tab, `i` toggles an inline image, `q` quits. Copy this to start your own.

use chaton_sdk::{Chaton, Flow, Key, View, chaton, kitty};

struct Hello {
    tabs: u32,
    last_rc: i32,
    image: bool,
}

impl Chaton for Hello {
    fn new() -> Self {
        Hello { tabs: 0, last_rc: 0, image: false }
    }

    fn on_key(&mut self, key: Key) -> Flow {
        match key {
            Key::Char('q') | Key::Esc => return Flow::Quit,
            Key::Char('n') => {
                self.last_rc = kitty("launch --type=tab");
                self.tabs += 1;
            }
            Key::Char('i') => self.image = !self.image,
            _ => {}
        }
        Flow::Continue
    }

    fn render(&self) -> View {
        let last = if self.tabs == 0 {
            "—".to_string()
        } else if self.last_rc == 0 {
            "✓ launched".to_string()
        } else {
            format!("✗ kitty exit {}", self.last_rc)
        };
        let text = format!(
            "\n  🐈 chatons — hello (via chaton-sdk)\n\n  tabs opened : {}\n  last action : {}\n\n  n  open a new kitty tab\n  i  toggle an inline image\n  q  quit\n",
            self.tabs, last
        );
        let view = View::text(text);
        if self.image {
            view.image("examples/hello/cat.png")
        } else {
            view
        }
    }
}

chaton!(Hello);
