//! hello-chaton — drives kitty, and shows an image.
//!
//! `n` → a new kitty tab appears (the chaton calls `kitty @ launch`).
//! `i` → an image is drawn inline via the kitty graphics protocol.
//! Both go through host functions — a sandboxed wasm plugin controlling kitty. Build your own
//! chaton (project launcher, dashboard, image browser, …) from this shape.

#[link(wasm_import_module = "chatons")]
extern "C" {
    fn host_render(ptr: *const u8, len: usize);
    fn kitty(ptr: *const u8, len: usize) -> i32;
    fn show_image(ptr: *const u8, len: usize) -> i32;
}

fn render_screen(s: &str) {
    unsafe { host_render(s.as_ptr(), s.len()) };
}

fn kitty_cmd(args: &str) -> i32 {
    unsafe { kitty(args.as_ptr(), args.len()) }
}

fn show(path: &str) -> i32 {
    unsafe { show_image(path.as_ptr(), path.len()) }
}

// Path is resolved relative to the chatons process cwd — run the demo from the repo root.
const IMG: &str = "examples/hello/cat.png";

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
        "\n  🐈 chatons — kitty bridge + graphics\n\n  tabs opened : {tabs}\n  last action : {last}\n\n  n  open a new kitty tab\n  i  show an image (kitty graphics)\n  q  quit\n"
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
    if code == 'i' as u32 {
        show(IMG); // leave the image on screen until the next keypress redraws text
        return 1;
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
