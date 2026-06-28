//! qr-chaton — type text, get a live QR code you can scan with your phone.
//!
//! A self-contained mini-app: no kitty-launching, no files — just sandboxed compute (the
//! `qrcode` crate, compiled to wasm) rendering unicode blocks into the terminal. Proof a chaton
//! can be *anything*, not just a launcher. (It even works without kitty's graphics protocol.)

// wit-bindgen 0.36's generated export glue isn't edition-2024 lint-clean yet (unsafe-in-unsafe).
#![allow(unsafe_op_in_unsafe_fn)]

wit_bindgen::generate!({ world: "chaton", path: "../../wit" });

use chatons::plugin::host;
use qrcode::QrCode;
use qrcode::render::unicode;
use std::cell::RefCell;

struct Qr {
    buf: String,
}

impl Qr {
    fn new() -> Self {
        Qr { buf: String::new() }
    }

    fn draw(&self) {
        let mut s = format!("\n  🐈 chatons — QR\n\n  ❯ {}▌\n\n", self.buf);
        if self.buf.is_empty() {
            s.push_str("  type anything — a URL, wifi, a note — to make a scannable QR\n");
        } else {
            match QrCode::new(self.buf.as_bytes()) {
                Ok(code) => s.push_str(&code.render::<unicode::Dense1x2>().quiet_zone(true).build()),
                Err(_) => s.push_str("  (too long for a QR code)\n"),
            }
        }
        s.push_str("\n  Backspace · Esc to quit\n");
        host::render(&s);
    }
}

thread_local! {
    static STATE: RefCell<Qr> = RefCell::new(Qr::new());
}

struct App;

impl Guest for App {
    fn init() {
        STATE.with_borrow(|s| s.draw());
    }

    fn on_key(k: Key) -> bool {
        STATE.with_borrow_mut(|s| {
            match k {
                Key::Escape => return false, // 'q' is typeable text, so only Esc quits
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
