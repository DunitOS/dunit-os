//! Exercise actual console pixels, cursor and clears against the display lease.
use std::sync::{Mutex, atomic::{AtomicU64, Ordering}};
mod handle {
    use super::*;
    pub static OWNER: AtomicU64 = AtomicU64::new(0);
    pub fn display_owner() -> u64 { OWNER.load(Ordering::Acquire) }
}
mod serial {
    use super::*;
    pub static BYTES: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    pub fn serial_write_byte(byte: u8) { BYTES.lock().unwrap().push(byte); }
}
#[path = "../src/console_font.rs"] mod console_font;
#[path = "../src/terminal.rs"] mod terminal;

#[test]
fn display_lease_preserves_scanout_and_console_resumes_after_release() {
    let mut pixels = vec![0x12345678; 96 * 64];
    let mut console = terminal::FbConsole::new(pixels.as_mut_ptr(),96,64,96*4);
    console.clear_screen();
    console.write_str("before\n");
    let before = pixels.clone();
    handle::OWNER.store(1, Ordering::Release);
    console.write_str("during\n");
    console.draw_cursor(true);
    console.draw_cursor(false);
    console.scroll_view(-1);
    console.clear_top_area(1);
    console.clear_screen();
    console.set_theme(&term_theme::DEFAULT);
    console.write_display_str("leased\n");
    assert_eq!(pixels,before, "console must never overwrite a leased display");
    assert!(serial::BYTES.lock().unwrap().windows(6).any(|s|s==b"during"));
    handle::OWNER.store(0, Ordering::Release);
    console.write_display_str("resumed\n");
    assert_ne!(pixels,before, "console rendering must resume when the lease ends");
}
