extern crate alloc;
use alloc::vec;
use dunit::ui::{Attrs, Component, ComponentWindow, Key, Renderer, Sizing, View};
use dunit::{time::Duration, window::WindowEvent};
struct Gallery(u32);
impl Component for Gallery {
    type Message = ();
    fn update(&mut self, _: ()) {
        self.0 += 1;
    }
    fn view(&self) -> View<()> {
        View::column(
            "root",
            vec![
                View::text("title", "Dunit SDK effects"),
                View::text("card", "Rounded gradient card").layout(Attrs {
                    width: Sizing::Px(if self.0 == 1 || self.0 == 2 {
                        220.
                    } else {
                        360.
                    }),
                    height: Sizing::Px(100.),
                    ..Attrs::default()
                }),
            ],
        )
        .on_key(Key::plain(' '), ())
    }
}
fn hash(pixels: &[u32]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for p in pixels {
        for shift in [16, 8, 0] {
            hash = (hash ^ ((p >> shift) & 255) as u64).wrapping_mul(0x100000001b3);
        }
    }
    hash
}
fn report(label: &str, pixels: &[u32]) -> dunit::Result<()> {
    dunit::println!(
        "{}",
        alloc::format!("[SDKEFFECTS] {} hash={:016x}", label, hash(pixels))
    )
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let overlay = dunit::fs::read_to_string("/assets/dwm/sdk/cards.dss")?;
    let dark = dunit::fs::read_to_string("/assets/dwm/sdk/dark.dss")?;
    let light = dunit::fs::read_to_string("/assets/dwm/sdk/light.dss")?;
    let mut ui = ComponentWindow::open_themed_with_style(480, 240, Gallery(0), &overlay)?;
    report("dark", ui.pixels())?;
    let mut stage = 0;
    loop {
        let event = ui.next_event(Duration::from_millis(500))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        match (stage, ui.component().0) {
            (0, 1) => {
                let stats = ui.last_render();
                assert!(stats.damaged_pixels > 360 * 100 && stats.damaged_pixels < 480 * 240);
                let (theme, font) = dunit::ui::load_theme(&overlay)?;
                let mut full = Renderer::new(font, theme.cascade, theme.background);
                full.render(&ui.component().view(), 480, 240).unwrap();
                assert_eq!(ui.pixels(), full.pixels());
                dunit::println!("[SDKEFFECTS] partial repair OK")?;
                report("narrow", ui.pixels())?;
                stage = 1;
            }
            (1, 2) => {
                dunit::fs::write("/sdk-theme.dss", light.as_bytes())?;
                assert!(ui.reload_theme()?);
                assert_eq!(ui.last_render().damaged_pixels, 480 * 240);
                report("light", ui.pixels())?;
                stage = 2;
            }
            (2, 3) => {
                dunit::fs::write("/sdk-theme.dss", dark.as_bytes())?;
                assert!(ui.reload_theme()?);
                assert_eq!(ui.last_render().damaged_pixels, 480 * 240);
                report("restored", ui.pixels())?;
                stage = 3;
            }
            (3, 4) => break,
            _ => {}
        }
    }
    assert_eq!(stage, 3);
    ui.close()?;
    dunit::println!("[SDKEFFECTS] ok")?;
    Ok(())
}
