extern crate alloc;
use alloc::vec;
use dunit::ui::{Attrs, Component, ComponentWindow, Key, Renderer, Sizing, View};
use dunit::{time::Duration, window::WindowEvent};
#[derive(Clone)]
enum Msg {
    Next,
    Noop,
    ToggleTitle,
}
struct Gallery(u32, bool);
fn size(width: f32, height: f32) -> Attrs {
    Attrs {
        width: Sizing::Px(width),
        height: Sizing::Px(height),
        ..Attrs::default()
    }
}
impl Component for Gallery {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        match message {
            Msg::Next => self.0 += 1,
            Msg::ToggleTitle => self.1 = !self.1,
            Msg::Noop => {}
        }
    }
    fn view(&self) -> View<Msg> {
        let stripes = (0..8)
            .map(|i| {
                View::text(alloc::format!("s{i}"), "").layout(size(
                    if self.0 == 1 || self.0 == 2 { 24. } else { 40. },
                    140.,
                ))
            })
            .collect();
        View::column(
            "root",
            vec![
                View::text("title", "Dunit SDK glass").class(if self.1 { "probe" } else { "idle" }),
                View::stack(
                    "scene",
                    vec![
                        View::row("pattern", stripes).layout(size(400., 140.)),
                        View::column(
                            "frame",
                            vec![View::column(
                                "glass",
                                vec![
                                    View::text("spill", "Rounded content").layout(size(280., 38.)),
                                    View::text("caption", "Blurred backdrop"),
                                ],
                            )
                            .layout(size(280., 100.))],
                        )
                        .layout(size(360., 140.)),
                    ],
                )
                .layout(size(400., 140.)),
            ],
        )
        .on_key(Key::plain(' '), Msg::Next)
    }
}
fn hash(pixels: &[u32]) -> u64 {
    let mut value = 0xcbf29ce484222325u64;
    for pixel in pixels {
        for shift in [16, 8, 0] {
            value = (value ^ ((pixel >> shift) & 255) as u64).wrapping_mul(0x100000001b3);
        }
    }
    value
}
fn report(label: &str, pixels: &[u32]) -> dunit::Result<()> {
    dunit::println!(
        "{}",
        alloc::format!("[SDKBACKDROP] {label} hash={:016x}", hash(pixels))
    )
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let overlay = alloc::format!(
        "{}\nText#title.probe {{color:$text;}}",
        dunit::fs::read_to_string("/assets/dwm/sdk/glass.dss")?
    );
    let dark = dunit::fs::read_to_string("/assets/dwm/sdk/dark.dss")?;
    let light = dunit::fs::read_to_string("/assets/dwm/sdk/light.dss")?;
    let mut ui = ComponentWindow::open_themed_with_style(480, 240, Gallery(0, false), &overlay)?;
    let initial = hash(ui.pixels());
    for (index, changed) in [true, false, true, false].into_iter().enumerate() {
        ui.update(Msg::ToggleTitle)?;
        let pixels = ui.last_render().damaged_pixels;
        assert!(pixels > 0 && pixels < 480 * 240);
        let uploaded = ui.last_uploaded_pixels();
        // The second backing needs an initial complete image. Thereafter both
        // retained buffers repair only this same independent title rectangle.
        assert_eq!(uploaded, if index == 0 { 480 * 240 } else { pixels });
        let (theme, font) = dunit::ui::load_theme(&overlay)?;
        let mut full = Renderer::new(font, theme.cascade, theme.background);
        full.render(&ui.component().view(), 480, 240).unwrap();
        assert_eq!(ui.pixels(), full.pixels());
        assert_eq!(hash(ui.pixels()) != initial, changed);
        ui.update(Msg::Noop)?;
        assert_eq!(ui.last_render().damaged_pixels, 0);
        dunit::println!(
            "{}",
            alloc::format!(
                "[SDKBACKDROP] independent damage={pixels} uploaded={uploaded} canvas=115200"
            )
        )?;
    }
    dunit::println!("[SDKBACKDROP] independent partial and restore OK")?;
    report("dark", ui.pixels())?;
    let mut stage = 0;
    loop {
        let event = ui.next_event(Duration::from_millis(500))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        match (stage, ui.component().0) {
            (0, 1) => {
                assert_eq!(ui.last_render().damaged_pixels, 480 * 240);
                assert_ne!(hash(ui.pixels()), initial);
                let (theme, font) = dunit::ui::load_theme(&overlay)?;
                let mut full = Renderer::new(font, theme.cascade, theme.background);
                full.render(&ui.component().view(), 480, 240).unwrap();
                assert_eq!(ui.pixels(), full.pixels());
                let before = hash(ui.pixels());
                ui.update(Msg::Noop)?;
                assert_eq!(ui.last_render().damaged_pixels, 0);
                assert_eq!(hash(ui.pixels()), before);
                dunit::println!("[SDKBACKDROP] scene rebuild and noop OK")?;
                report("changed", ui.pixels())?;
                stage = 1;
            }
            (1, 2) => {
                dunit::fs::write("/sdk-theme.dss", light.as_bytes())?;
                assert!(ui.reload_theme()?);
                let before = hash(ui.pixels());
                for rule in [
                    "Column#glass {backdrop-blur:65;}",
                    "Column#glass {clip:invalid;}",
                ] {
                    dunit::fs::write(
                        "/sdk-theme.dss",
                        alloc::format!("{light} {rule}").as_bytes(),
                    )?;
                    assert!(ui.reload_theme().is_err());
                    assert!(ui.theme_error().is_some());
                    assert_eq!(hash(ui.pixels()), before);
                }
                dunit::fs::write("/sdk-theme.dss", light.as_bytes())?;
                assert!(!ui.reload_theme()?);
                assert!(ui.theme_error().is_none());
                dunit::println!("[SDKBACKDROP] invalid retained OK")?;
                report("light", ui.pixels())?;
                stage = 2;
            }
            (2, 3) => {
                dunit::fs::write("/sdk-theme.dss", dark.as_bytes())?;
                assert!(ui.reload_theme()?);
                assert_eq!(hash(ui.pixels()), initial);
                report("restored", ui.pixels())?;
                stage = 3;
            }
            (3, 4) => break,
            _ => {}
        }
    }
    assert_eq!(stage, 3);
    ui.close()?;
    dunit::println!("[SDKBACKDROP] ok")?;
    Ok(())
}
