//! hello-chaton — a tiny interactive chaton.
//!
//! It keeps its own state (a keypress counter), reacts to keys via `on_key`, and redraws
//! through the host's `host_render`. `q` or `esc` quits. This is the whole v0.2 contract in
//! one file — the shape every chaton follows.

#[link(wasm_import_module = "chatons")]
extern "C" {
    fn host_render(ptr: *const u8, len: usize);
}

// Single-threaded wasm guest → plain mutable statics are fine (and the simplest way to show
// "a chaton holds state across events").
static mut COUNT: u32 = 0;
static mut LAST: u32 = 0;

fn draw() {
    let (count, last) = unsafe { (COUNT, LAST) };
    let last_disp = char::from_u32(last)
        .filter(|c| !c.is_control())
        .map(|c| c.to_string())
        .unwrap_or_else(|| "·".to_string());
    let screen = format!(
        "\n  🐈 chatons — hello\n\n  keys pressed : {count}\n  last key     : {last_disp}\n\n  press any key · q or esc to quit\n"
    );
    unsafe { host_render(screen.as_ptr(), screen.len()) };
}

/// Paint the first frame.
#[no_mangle]
pub extern "C" fn init() {
    draw();
}

/// Handle a key. Return 0 to quit, 1 to keep running.
#[no_mangle]
pub extern "C" fn on_key(code: u32) -> u32 {
    if code == 'q' as u32 || code == 27 {
        return 0;
    }
    unsafe {
        COUNT += 1;
        LAST = code;
    }
    draw();
    1
}
