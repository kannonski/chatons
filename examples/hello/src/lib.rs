//! hello-chaton — the reference chaton, a WASM component implementing the `chaton` WIT world.
//!
//! `wit_bindgen::generate!` turns ../../wit/chaton.wit into a `Guest` trait to implement and
//! `host::*` functions to call. The chaton holds state in a `thread_local`, paints by calling
//! `host::render`, and drives kitty via `host::kitty`. `n` opens a tab, `i` toggles an image,
//! `q` quits. This is how you write a chaton — copy it (in any language with WIT bindings).

// wit-bindgen 0.36's generated export glue isn't edition-2024 lint-clean yet (unsafe-in-unsafe).
#![allow(unsafe_op_in_unsafe_fn)]

wit_bindgen::generate!({ world: "chaton", path: "../../wit" });

use chatons::plugin::host;
use std::cell::RefCell;

struct Hello {
    tabs: u32,
    last_rc: i32,
    image: bool,
}

impl Hello {
    fn new() -> Self {
        Hello { tabs: 0, last_rc: 0, image: false }
    }

    fn draw(&self) {
        let last = if self.tabs == 0 {
            "—".to_string()
        } else if self.last_rc == 0 {
            "✓ launched".to_string()
        } else {
            format!("✗ kitty exit {}", self.last_rc)
        };
        host::render(&format!(
            "\n  🐈 chatons — hello (a wasm component)\n\n  tabs opened : {}\n  last action : {}\n\n  n  open a new kitty tab\n  i  toggle an inline image\n  q  quit\n",
            self.tabs, last
        ));
        if self.image {
            host::show_image("examples/hello/cat.png");
        }
    }
}

thread_local! {
    static STATE: RefCell<Hello> = RefCell::new(Hello::new());
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
                Key::Text('n') => {
                    s.last_rc = host::kitty("launch --type=tab");
                    s.tabs += 1;
                }
                Key::Text('i') => s.image = !s.image,
                _ => {}
            }
            s.draw();
            true
        })
    }
}

export!(App);
