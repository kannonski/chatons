//! hello-chaton — drives kitty.
//!
//! Press `n` and a new kitty tab appears: the chaton calls the host's `kitty()` function, which
//! runs `kitty @ launch --type=tab`. That's the v0.3 payoff — a sandboxed wasm plugin actually
//! controlling kitty. Build your own chaton (project launcher, dashboard, …) from this shape.

#[link(wasm_import_module = "chatons")]
extern "C" {
    fn host_render(ptr: *const u8, len: usize);
    fn kitty(ptr: *const u8, len: usize) -> i32;
}

fn render_screen(s: &str) {
    unsafe { host_render(s.as_ptr(), s.len()) };
}

/// Run `kitty @ <args>` through the host. Returns the exit code (0 = ok).
fn kitty_cmd(args: &str) -> i32 {
    unsafe { kitty(args.as_ptr(), args.len()) }
}

static mut TABS: u32 = 0;
static mut LAST_RC: i32 = 0;

fn draw() {
    let (tabs, last_rc) = unsafe { (TABS, LAST_RC) };
    let last = if tabs == 0 {
        "—".to_string()
    } else if last_rc == 0 {
        "✓ launched".to_string()
    } else {
        format!("✗ kitty exit {last_rc}")
    };
    let screen = format!(
        "\n  🐈 chatons — kitty bridge\n\n  tabs opened : {tabs}\n  last action : {last}\n\n  n  open a new kitty tab\n  q  quit\n"
    );
    render_screen(&screen);
}

#[no_mangle]
pub extern "C" fn init() {
    draw();
}

#[no_mangle]
pub extern "C" fn on_key(code: u32) -> u32 {
    if code == 'q' as u32 || code == 27 {
        return 0;
    }
    if code == 'n' as u32 {
        let rc = kitty_cmd("launch --type=tab");
        unsafe {
            TABS += 1;
            LAST_RC = rc;
        }
    }
    draw();
    1
}
