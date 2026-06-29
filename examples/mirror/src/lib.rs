//! mirror-chaton — open the current kitty tab in your local browser, and control it from there.
//!
//! It resolves the tab behind the overlay (`host.source-window`), starts the `chatons mirror`
//! daemon for it (a background process that *outlives* this overlay), and opens the browser at
//! the local URL. The daemon serves a live, controllable view on 127.0.0.1 — view + keystrokes.
//! Closing this panel leaves the mirror running; `s` stops it.

// wit-bindgen 0.36's generated export glue isn't edition-2024 lint-clean yet (unsafe-in-unsafe).
#![allow(unsafe_op_in_unsafe_fn)]

wit_bindgen::generate!({ world: "chaton", path: "../../wit" });

use chatons::plugin::host;
use std::cell::RefCell;

const PORT: u32 = 9123;
const URL: &str = "http://127.0.0.1:9123";

struct Mirror {
    window: String, // the kitty window id we're mirroring ("" if none found)
    running: bool,
}

impl Mirror {
    fn new() -> Self {
        let window = host::source_window();
        let running = !window.is_empty();
        if running {
            // the host spawns the daemon (detached, with the kitty socket env) so it keeps
            // serving after this overlay closes; then open the browser at the URL it returns
            host::start_mirror(&window, PORT);
            host::kitty(&format!("launch --type=background xdg-open {URL}"));
        }
        Mirror { window, running }
    }

    fn open_browser(&self) {
        host::kitty(&format!("launch --type=background xdg-open {URL}"));
    }

    fn stop(&mut self) {
        host::kitty("launch --type=background chatons mirror --stop");
        self.running = false;
    }

    fn draw(&self) {
        let mut s = String::from("\n  🪞 mirror\n\n");
        if self.window.is_empty() {
            s.push_str("  couldn't find a tab to mirror.\n");
            s.push_str("  open this from the tab you want to share.\n");
            s.push_str("\n  q close\n");
        } else if self.running {
            s.push_str(&format!("  serving this tab (window {}) at\n\n", self.window));
            s.push_str(&format!("  {URL}\n\n"));
            s.push_str("  open it in any local browser — view + control.\n");
            s.push_str("  keeps serving after you close this panel.\n");
            s.push_str("\n  o open browser   s stop   q close\n");
        } else {
            s.push_str("  mirror stopped.\n");
            s.push_str("\n  q close\n");
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
            match k {
                Key::Text('q') | Key::Escape => return false,
                Key::Text('o') => s.open_browser(),
                Key::Text('s') => s.stop(),
                _ => {}
            }
            s.draw();
            true
        })
    }
}

export!(App);
