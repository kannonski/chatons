//! fend-chaton — a unit-aware calculator, powered by [`fend-core`](https://github.com/printfn/fend).
//!
//! Type an expression and the result updates live: `3 miles to km`, `5 GBP in EUR`, `2^64`,
//! `sqrt 2`, `1 byte to bits`. Esc quits. A real GitHub project ported into a chaton with zero
//! new host API — just the input loop + the crate.

// wit-bindgen 0.36's generated export glue isn't edition-2024 lint-clean yet (unsafe-in-unsafe).
#![allow(unsafe_op_in_unsafe_fn)]

wit_bindgen::generate!({ world: "chaton", path: "../../wit" });

use chatons::plugin::host;
use fend_core::{Context, ExchangeRateFnV2, ExchangeRateFnV2Options};
use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;

// fend wants `units of <code> per 1 base-unit` (we use USD as the base) — return per_usd as-is.
struct Rates(HashMap<String, f64>);

impl ExchangeRateFnV2 for Rates {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _options: &ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn Error + Send + Sync + 'static>> {
        self.0
            .get(currency)
            .copied()
            .ok_or_else(|| format!("no rate for {currency}").into())
    }
}

struct Fend {
    buf: String,
    ctx: Context,
}

impl Fend {
    fn new() -> Self {
        let mut ctx = Context::new();
        // currency conversion needs live rates; the host fetches them (the chaton can't network)
        let rates: HashMap<String, f64> =
            host::exchange_rates().into_iter().map(|r| (r.code, r.per_usd)).collect();
        if !rates.is_empty() {
            ctx.set_exchange_rate_handler_v2(Rates(rates));
        }
        Fend { buf: String::new(), ctx }
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
