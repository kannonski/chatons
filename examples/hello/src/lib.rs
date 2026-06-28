//! hello-chaton — the smallest possible chaton.
//!
//! It imports `host_render` from the host (module "chatons") and calls it once from `run()`.
//! That's the whole v0.1 contract: the host invokes `run`, the chaton talks back through a
//! host function. Everything else (events in, richer rendering, kitty actions) grows from here.

// Functions the host provides. `wasm_import_module` must match the host's Linker module name.
#[link(wasm_import_module = "chatons")]
extern "C" {
    fn host_render(ptr: *const u8, len: usize);
}

/// Entry point the host calls.
#[no_mangle]
pub extern "C" fn run() {
    let msg = b"hello from a chaton (Rust \xE2\x86\x92 WASM \xE2\x86\x92 host)";
    unsafe { host_render(msg.as_ptr(), msg.len()) };
}
