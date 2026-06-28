//! launcher-chaton — one key to launch any chaton.
//!
//! Lists every installed chaton (`host::list_chatons`), filter by typing, `enter` runs the top
//! match — which the launcher opens as its own overlay (tagged so it self-toggles) and then
//! exits. One good keybinding for all chatons, instead of a key per chaton.

// wit-bindgen 0.36's generated export glue isn't edition-2024 lint-clean yet (unsafe-in-unsafe).
#![allow(unsafe_op_in_unsafe_fn)]

wit_bindgen::generate!({ world: "chaton", path: "../../wit" });

use chatons::plugin::host;
use std::cell::RefCell;

struct Launcher {
    all: Vec<String>,
    query: String,
}

impl Launcher {
    fn new() -> Self {
        Launcher { all: host::list_chatons(), query: String::new() }
    }

    fn matches(&self) -> Vec<&String> {
        let q = self.query.to_lowercase();
        self.all.iter().filter(|n| n.to_lowercase().contains(&q)).collect()
    }

    fn draw(&self) {
        let m = self.matches();
        let mut s = format!("\n  🐈 chatons\n\n  ❯ {}▌\n\n", self.query);
        if m.is_empty() {
            s.push_str("  (no match)\n");
        }
        for (i, name) in m.iter().enumerate() {
            let cursor = if i == 0 { "▌" } else { " " }; // top match runs on Enter
            s.push_str(&format!("  {cursor} {name}\n"));
        }
        s.push_str("\n  type to filter · ↵ run top match · esc\n");
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
            match k {
                Key::Escape => return false,
                Key::Enter => {
                    if let Some(name) = s.matches().first() {
                        // open the chosen chaton as an overlay (tagged so it self-toggles)
                        host::kitty(&format!(
                            "launch --type=overlay --cwd=current --var chaton={0} chatons run {0}",
                            name.as_str()
                        ));
                    }
                    return false; // launcher exits; the chosen chaton takes over
                }
                Key::Backspace => {
                    s.query.pop();
                }
                Key::Text(c) => s.query.push(c),
                _ => {}
            }
            s.draw();
            true
        })
    }
}

export!(App);
