//! fend-chaton — a unit-aware calculator, powered by [`fend-core`](https://github.com/printfn/fend).
//!
//! Type an expression and the result updates live: `3 miles to km`, `5 GBP in EUR`, `2^64`,
//! `sqrt 2`, `1 byte to bits`. Esc quits. A real GitHub project ported into a chaton with zero
//! new host API — just the input loop + the crate.

// wit-bindgen 0.36's generated export glue isn't edition-2024 lint-clean yet (unsafe-in-unsafe).
#![allow(unsafe_op_in_unsafe_fn)]

wit_bindgen::generate!({ world: "chaton", path: "../../wit" });

use chatons::plugin::host;
use std::cell::RefCell;

struct Fend {
    buf: String,
    ctx: fend_core::Context,
}

impl Fend {
    fn new() -> Self {
        Fend { buf: String::new(), ctx: fend_core::Context::new() }
    }

    fn draw(&mut self) {
        // evaluate live; while typing a partial/invalid expression, just show a dim ellipsis
        let result = if self.buf.trim().is_empty() {
            String::new()
        } else {
            match fend_core::evaluate(&self.buf, &mut self.ctx) {
                Ok(r) if !r.get_main_result().is_empty() => format!("  = {}", r.get_main_result()),
                Ok(_) => String::new(),
                Err(_) => "  …".to_string(),
            }
        };
        host::render(&format!(
            "\n  🐈 chatons — fend calculator\n\n  ❯ {}▌\n\n{}\n\n  e.g.  3 miles to km · 5 GBP in EUR · 2^64 · sqrt 2  ·  Esc to quit\n",
            self.buf, result
        ));
    }
}

thread_local! {
    static STATE: RefCell<Fend> = RefCell::new(Fend::new());
}

struct App;

impl Guest for App {
    fn init() {
        STATE.with_borrow_mut(|s| s.draw());
    }

    fn on_key(k: Key) -> bool {
        STATE.with_borrow_mut(|s| {
            match k {
                Key::Escape => return false, // letters are math (km, sin, pi), so only Esc quits
                Key::Backspace => {
                    s.buf.pop();
                }
                Key::Text(c) => s.buf.push(c),
                _ => {}
            }
            s.draw();
            true
        })
    }
}

export!(App);
