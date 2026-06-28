//! chaton-sdk — write a kitty chaton (a WASM plugin for [chatons]) as a trait, not raw FFI.
//!
//! ```ignore
//! use chaton_sdk::{chaton, kitty, Chaton, Flow, Key, View};
//!
//! struct Mine { tabs: u32 }
//!
//! impl Chaton for Mine {
//!     fn new() -> Self { Mine { tabs: 0 } }
//!     fn on_key(&mut self, key: Key) -> Flow {
//!         match key {
//!             Key::Char('q') | Key::Esc => return Flow::Quit,
//!             Key::Char('n') => { kitty("launch --type=tab"); self.tabs += 1; }
//!             _ => {}
//!         }
//!         Flow::Continue
//!     }
//!     fn render(&self) -> View {
//!         View::text(format!("tabs: {}\nn new · q quit", self.tabs))
//!     }
//! }
//!
//! chaton!(Mine);
//! ```
//!
//! [chatons]: https://github.com/kannonski/chatons

// The host functions chatons provides. This block is the *only* raw FFI in the whole SDK —
// a chaton author never writes `unsafe extern`. (Edition 2024: `unsafe extern`, calls are unsafe.)
#[link(wasm_import_module = "chatons")]
unsafe extern "C" {
    #[link_name = "host_render"]
    fn raw_render(ptr: *const u8, len: usize);
    #[link_name = "kitty"]
    fn raw_kitty(ptr: *const u8, len: usize) -> i32;
    #[link_name = "show_image"]
    fn raw_show_image(ptr: *const u8, len: usize) -> i32;
    #[link_name = "write_file"]
    fn raw_write_file(ppath: *const u8, lpath: usize, pdata: *const u8, ldata: usize) -> i32;
}

/// Run `kitty @ <args>` (e.g. `kitty("launch --type=tab")`). Returns the exit code (0 = ok).
pub fn kitty(args: &str) -> i32 {
    unsafe { raw_kitty(args.as_ptr(), args.len()) }
}

/// Display a PNG inline via the kitty graphics protocol. Returns 0 on success. Prefer
/// [`View::image`] for view content; use this only for ad-hoc draws.
pub fn show_image(path: &str) -> i32 {
    unsafe { raw_show_image(path.as_ptr(), path.len()) }
}

/// Write `contents` to `path` on the host's filesystem. Returns 0 on success, -1 on failure.
/// (Reading a file back into the guest isn't supported yet — that needs the host→guest data
/// direction, a later addition.)
pub fn write_file(path: &str, contents: &str) -> i32 {
    unsafe {
        raw_write_file(
            path.as_ptr(),
            path.len(),
            contents.as_ptr(),
            contents.len(),
        )
    }
}

/// A key handed to [`Chaton::on_key`]. The host currently sends characters, Enter and Esc.
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Esc,
    Other(u32),
}

impl Key {
    #[doc(hidden)]
    pub fn from_code(code: u32) -> Key {
        match code {
            8 => Key::Backspace,
            13 => Key::Enter,
            27 => Key::Esc,
            c => char::from_u32(c).map(Key::Char).unwrap_or(Key::Other(c)),
        }
    }
}

/// What [`Chaton::on_key`] returns: keep running, or quit the chaton.
pub enum Flow {
    Continue,
    Quit,
}

/// A chaton's view for one frame: a text screen plus an optional inline image. The host paints
/// the text first, then the image over it, so the image survives the redraw.
pub struct View {
    pub text: String,
    pub image: Option<String>,
}

impl View {
    pub fn text(text: impl Into<String>) -> Self {
        View { text: text.into(), image: None }
    }

    /// Draw a PNG (by path) inline beneath the text this frame.
    pub fn image(mut self, path: impl Into<String>) -> Self {
        self.image = Some(path.into());
        self
    }
}

/// Implement this and hand it to [`chaton!`]. The host owns the loop, the terminal, and kitty;
/// your chaton owns its state and its view.
pub trait Chaton {
    /// Build the chaton (called once, before the first frame).
    fn new() -> Self;
    /// React to a key. Return [`Flow::Quit`] to exit.
    fn on_key(&mut self, key: Key) -> Flow;
    /// Produce this frame's view.
    fn render(&self) -> View;
}

/// Paint a view: text, then image (so the image isn't cleared by the text repaint).
#[doc(hidden)]
pub fn paint(view: &View) {
    unsafe { raw_render(view.text.as_ptr(), view.text.len()) };
    if let Some(path) = &view.image {
        let _ = show_image(path);
    }
}

/// Wire a [`Chaton`] type up as a chaton: generates the `init` / `on_key` wasm exports the host
/// calls, holds the instance, and paints after each event. No `unsafe`, no FFI in your code.
#[macro_export]
macro_rules! chaton {
    ($t:ty) => {
        static mut __CHATON: ::core::option::Option<$t> = ::core::option::Option::None;

        #[unsafe(no_mangle)]
        pub extern "C" fn init() {
            let c = <$t as $crate::Chaton>::new();
            unsafe { *(&raw mut __CHATON) = ::core::option::Option::Some(c) };
            let view = unsafe { (*(&raw const __CHATON)).as_ref().unwrap().render() };
            $crate::paint(&view);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn on_key(code: u32) -> u32 {
            let key = $crate::Key::from_code(code);
            let flow = unsafe { (*(&raw mut __CHATON)).as_mut().unwrap().on_key(key) };
            if let $crate::Flow::Quit = flow {
                return 0;
            }
            let view = unsafe { (*(&raw const __CHATON)).as_ref().unwrap().render() };
            $crate::paint(&view);
            1
        }
    };
}
