//! Exercise actual console pixels, cursor and clears against the display lease.
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
#[allow(dead_code)]
#[path = "../src/sync.rs"]
mod actual_sync;
mod sync {
    pub use crate::actual_sync::SpinLock as IrqSafeSpinLock;
}
static TEST_LOCK: Mutex<()> = Mutex::new(());
mod handle {
    use super::*;
    pub static OWNER: AtomicU64 = AtomicU64::new(0);
    pub fn display_owner() -> u64 {
        OWNER.load(Ordering::Acquire)
    }
}
mod serial {
    use super::*;
    pub static BYTES: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    pub fn serial_write_byte(byte: u8) {
        // Serial callbacks may reenter console snapshots: no console guard here.
        let _ = crate::terminal::get_cursor_info();
        BYTES.lock().unwrap().push(byte);
    }
}
#[path = "../src/console_font.rs"]
mod console_font;
#[path = "../src/terminal.rs"]
mod terminal;

#[test]
fn display_lease_preserves_scanout_and_console_resumes_after_release() {
    let _test = TEST_LOCK.lock().unwrap();
    // The published boot console keeps its framebuffer for the entire run.
    let pixels = Box::leak(vec![0x12345678; 96 * 64].into_boxed_slice());
    unsafe {
        terminal::init(pixels.as_mut_ptr(), 96, 64, 96 * 4);
    }
    let mut console = terminal::console().unwrap();
    console.clear_screen();
    console.write_str("before\n");
    let before = pixels.to_vec();
    handle::OWNER.store(1, Ordering::Release);
    console.set_fg_color(0xFF00FF);
    console.draw_char('X');
    console.reset_fg_color();
    console.write_str("during\n");
    console.draw_cursor(true);
    console.draw_cursor(false);
    console.scroll_view(-1);
    console.clear_top_area(1);
    console.clear_screen();
    console.set_theme(&term_theme::DEFAULT);
    console.write_display_str("leased\n");
    assert_eq!(
        pixels, before,
        "console must never overwrite a leased display"
    );
    assert!(serial::BYTES
        .lock()
        .unwrap()
        .windows(6)
        .any(|s| s == b"during"));
    handle::OWNER.store(0, Ordering::Release);
    console.write_display_str("resumed\n");
    assert_ne!(
        pixels, before,
        "console rendering must resume when the lease ends"
    );
    // A retained shell handle permits syscall output/snapshots on another thread.
    handle::OWNER.store(1, Ordering::Release);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                let mut output = terminal::console().unwrap();
                for _ in 0..200 {
                    output.write_display_str("text\n");
                    output.scroll_view(-1);
                    output.scroll_view(1);
                    let cursor = terminal::get_cursor_info().unwrap();
                    assert!(cursor.y < 64);
                }
            });
        }
    });
    console.write_str("after writers\n");
    handle::OWNER.store(0, Ordering::Release);
}

#[test]
fn independent_consoles_do_not_share_scrollback() {
    let _test = TEST_LOCK.lock().unwrap();
    let mut first_pixels = vec![0; 96 * 64];
    let mut second_pixels = vec![0; 96 * 64];
    let mut first = unsafe { terminal::FbConsole::new(first_pixels.as_mut_ptr(), 96, 64, 96 * 4) };
    let mut second =
        unsafe { terminal::FbConsole::new(second_pixels.as_mut_ptr(), 96, 64, 96 * 4) };
    first.clear_screen();
    first.draw_char('A');
    let before = first_pixels.clone();
    second.clear_screen();
    second.draw_char('B');
    first.scroll_view(0);
    first.draw_cursor(false);
    assert_eq!(
        first_pixels, before,
        "redraw must retain this console's own history"
    );
}
