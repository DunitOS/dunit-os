use dunit::{
    time::Duration,
    window::{Window, WindowEvent, IN_DOWN, IN_KEY, IN_LEAVE, IN_MOVE, IN_SCROLL, IN_UP},
};
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut w = Window::open_legacy(320, 200)?;
    assert!(!w.input_lifecycle());
    w.draw(|f| f.pixels_mut().fill(0xff203040))?;
    dunit::println!("[LEGACY] ready")?;
    loop {
        match w.next_event(Duration::from_secs(30))? {
            Some(WindowEvent::Input(i)) => {
                assert!(matches!(
                    i.kind,
                    IN_KEY | IN_MOVE | IN_DOWN | IN_UP | IN_LEAVE | IN_SCROLL
                ));
                if matches!(i.kind, IN_MOVE | IN_DOWN | IN_UP | IN_LEAVE | IN_SCROLL) {
                    assert!(i.value <= 1, "legacy pointer bytes changed");
                }
                if i.kind == IN_KEY && i.value == b'l' as u32 {
                    assert_eq!(i.y, 0);
                    dunit::println!("[LEGACY] key and pointer bytes unchanged")?;
                }
                if i.kind == IN_KEY && i.value == b'x' as u32 {
                    break;
                }
            }
            Some(WindowEvent::Resized { .. }) => w.draw(|f| f.pixels_mut().fill(0xff203040))?,
            Some(WindowEvent::CloseRequested) => break,
            Some(WindowEvent::Protocol(_)) => {}
            None => {}
        }
    }
    w.close()?;
    dunit::println!("[LEGACY] ok")?;
    Ok(())
}
