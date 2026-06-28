//! launcher-chaton — one key to find and run any chaton.
//!
//! vim navigation by default (`j`/`k`, `g`/`G`), `/` to search, `enter` to run, `q`/esc to
//! quit — like matou, for chatons. Each chaton shows its icon (from the manifest). Runs the
//! selected one by opening it as a tagged overlay, then exits.

// wit-bindgen 0.36's generated export glue isn't edition-2024 lint-clean yet (unsafe-in-unsafe).
#![allow(unsafe_op_in_unsafe_fn)]

wit_bindgen::generate!({ world: "chaton", path: "../../wit" });

use chatons::plugin::host;
use chatons::plugin::types::ChatonInfo;
use std::cell::RefCell;

struct Launcher {
    all: Vec<ChatonInfo>,
    query: String,
    search: bool,
    cur: usize,
    pending_g: bool, // first `g` of a `gg` jump-to-top
}

impl Launcher {
    fn new() -> Self {
        Launcher {
            all: host::list_chatons(),
            query: String::new(),
            search: false,
            cur: 0,
            pending_g: false,
        }
    }

    fn view(&self) -> Vec<&ChatonInfo> {
        if self.query.is_empty() {
            return self.all.iter().collect();
        }
        let q = self.query.to_lowercase();
        self.all.iter().filter(|c| c.name.to_lowercase().contains(&q)).collect()
    }

    fn run(&self) {
        if let Some(c) = self.view().get(self.cur) {
            host::kitty(&format!(
                "launch --type=overlay --cwd=current --var chaton={0} chatons run {0}",
                c.name
            ));
        }
    }

    fn draw(&self) {
        let v = self.view();
        let mut s = if self.search {
            format!("\n  🐈 chatons    / {}▌\n\n", self.query)
        } else {
            "\n  🐈 chatons\n\n".to_string()
        };
        if v.is_empty() {
            s.push_str("  (no match)\n");
        }
        for (i, c) in v.iter().enumerate() {
            let cursor = if i == self.cur { "▌" } else { " " };
            s.push_str(&format!("  {cursor} {}  {}\n", c.icon, c.name));
        }
        let footer = if self.search {
            "type to filter · ↵ run · esc back"
        } else {
            "j/k move · / search · ↵ run · q quit"
        };
        s.push_str(&format!("\n  {footer}\n"));
        host::render(&s);
    }
}

thread_local! {
    static STATE: RefCell<Launcher> = RefCell::new(Launcher::new());
}

struct App;

impl Guest for App {
    fn init() {
        STATE.with_borrow(|s| s.draw());
    }

    fn on_key(k: Key) -> bool {
        STATE.with_borrow_mut(|s| {
            let n = s.view().len();
            if s.search {
                match k {
                    Key::Escape => {
                        s.search = false;
                        s.query.clear();
                        s.cur = 0;
                    }
                    Key::Enter => {
                        s.run();
                        return false;
                    }
                    Key::Backspace => {
                        s.query.pop();
                        s.cur = 0;
                    }
                    Key::Text(c) => {
                        s.query.push(c);
                        s.cur = 0;
                    }
                    _ => {}
                }
            } else {
                let pending_g = s.pending_g;
                s.pending_g = false; // any key cancels a half-typed `gg`
                match k {
                    Key::Text('q') | Key::Escape => return false,
                    Key::Text('j') => {
                        if s.cur + 1 < n {
                            s.cur += 1;
                        }
                    }
                    Key::Text('k') => s.cur = s.cur.saturating_sub(1),
                    Key::Text('g') => {
                        if pending_g {
                            s.cur = 0; // gg → top
                        } else {
                            s.pending_g = true; // first g, await the second
                        }
                    }
                    Key::Text('G') => s.cur = n.saturating_sub(1),
                    Key::Text('/') => {
                        s.search = true;
                        s.query.clear();
                        s.cur = 0;
                    }
                    Key::Enter => {
                        s.run();
                        return false;
                    }
                    _ => {}
                }
            }
            s.draw();
            true
        })
    }
}

export!(App);
