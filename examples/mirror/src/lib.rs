//! mirror-chaton — open a kitty tab in your local browser (view + control).
//!
//! Opens a small menu:
//!   • "mirror this tab" — stream the tab you opened this from (view + control your real session)
//!   • "new stream tab"  — spin up a fresh background tab and stream *that*; you drive it entirely
//!                         from the browser ("a tab displayed on the browser")
//! Either way it starts the `chatons mirror` daemon for the chosen window (which outlives this
//! panel) and opens the browser. In the running panel: o reopens the browser, s stops, q closes.

// wit-bindgen 0.36's generated export glue isn't edition-2024 lint-clean yet (unsafe-in-unsafe).
#![allow(unsafe_op_in_unsafe_fn)]

wit_bindgen::generate!({ world: "chaton", path: "../../wit" });

use chatons::plugin::host;
use std::cell::RefCell;

const PORT: u32 = 9123;
const URL: &str = "http://127.0.0.1:9123";
const ITEMS: [(&str, &str); 2] = [
    ("mirror this tab", "view + control the tab you're in"),
    ("new stream tab", "fresh tab, driven from the browser"),
];

enum View {
    Menu,
    Running,
    Done(String),
}

struct Mirror {
    view: View,
    cursor: usize,
    window: String,
}

impl Mirror {
    fn new() -> Self {
        Mirror { view: View::Menu, cursor: 0, window: String::new() }
    }

    /// Start the daemon for `window` and open the browser; empty window ⇒ failure.
    fn start(&mut self, window: String) {
        if window.is_empty() {
            self.view = View::Done("couldn't find / create a tab to stream.".into());
            return;
        }
        host::start_mirror(&window, PORT);
        host::kitty(&format!("launch --type=background xdg-open {URL}"));
        self.window = window;
        self.view = View::Running;
    }

    fn select(&mut self) {
        match self.cursor {
            // mirror the tab behind this overlay
            0 => {
                let w = host::source_window();
                self.start(w);
            }
            // create a fresh background tab and stream it (kitty @ launch prints its window id)
            1 => {
                let w = host::kitty_capture("launch --type=tab --cwd=current --keep-focus");
                self.start(w);
            }
            _ => {}
        }
    }

    fn open_browser(&self) {
        host::kitty(&format!("launch --type=background xdg-open {URL}"));
    }

    fn stop(&mut self) {
        host::kitty("launch --type=background chatons mirror --stop");
        self.view = View::Done("mirror stopped.".into());
    }

    fn draw(&self) {
        let mut s = String::from("\n  🪞 mirror\n\n");
        match &self.view {
            View::Menu => {
                for (i, (name, desc)) in ITEMS.iter().enumerate() {
                    let cur = if i == self.cursor { "▌" } else { " " };
                    s.push_str(&format!("  {cur} {name:16}  {desc}\n"));
                }
                s.push_str("\n  j/k move · ↵ select · q quit\n");
            }
            View::Running => {
                s.push_str(&format!("  serving window {} at\n\n  {URL}\n\n", self.window));
                s.push_str("  open it in any local browser — view + control.\n");
                s.push_str("  keeps serving after you close this panel.\n");
                s.push_str("\n  o open browser   s stop   q close\n");
            }
            View::Done(msg) => {
                s.push_str(&format!("  {msg}\n"));
                s.push_str("\n  q close\n");
            }
        }
        host::render(&s);
    }
}

thread_local! {
    static STATE: RefCell<Mirror> = RefCell::new(Mirror::new());
}

struct App;

impl Guest for App {
    fn init() {
        STATE.with_borrow(|s| s.draw());
    }

    fn on_key(k: Key) -> bool {
        STATE.with_borrow_mut(|s| {
            if matches!(k, Key::Text('q') | Key::Escape) {
                return false;
            }
            let in_menu = matches!(s.view, View::Menu);
            let in_running = matches!(s.view, View::Running);
            if in_menu {
                match k {
                    Key::Text('j') => s.cursor = (s.cursor + 1).min(ITEMS.len() - 1),
                    Key::Text('k') => s.cursor = s.cursor.saturating_sub(1),
                    Key::Enter => s.select(),
                    _ => {}
                }
            } else if in_running {
                match k {
                    Key::Text('o') => s.open_browser(),
                    Key::Text('s') => s.stop(),
                    _ => {}
                }
            }
            s.draw();
            true
        })
    }
}

export!(App);
