extern crate alloc;
use alloc::{collections::BTreeSet, vec};
use dunit::ui::{Attrs, Component, ComponentWindow, Key, Renderer, Sizing, Theme, View};
use dunit::{
    time::{Duration, Instant},
    window::WindowEvent,
};
#[derive(Clone)]
enum Msg {
    Toggle,
    Noop,
}
struct Demo(bool);
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        if matches!(message, Msg::Toggle) {
            self.0 = !self.0;
        }
    }
    fn view(&self) -> View<Msg> {
        let card = View::text("card", "DSS transitions").layout(Attrs {
            width: Sizing::Px(360.),
            height: Sizing::Px(100.),
            ..Attrs::default()
        });
        View::column(
            "root",
            vec![
                View::text("title", "Dunit SDK motion"),
                if self.0 { card.class("active") } else { card },
            ],
        )
        .on_key(Key::plain(' '), Msg::Toggle)
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
        alloc::format!("[SDKMOTION] {label} hash={:016x}", hash(pixels))
    )
}
fn static_frame(
    profile: &str,
    base: &str,
    overlay: &str,
    active: bool,
) -> dunit::Result<alloc::vec::Vec<u32>> {
    let theme = Theme::parse(profile, base, overlay).unwrap();
    let (_, font) = dunit::ui::load_theme(overlay)?;
    let mut renderer = Renderer::new(font, theme.cascade, theme.background);
    renderer.render(&Demo(active).view(), 480, 240).unwrap();
    Ok(renderer.pixels().to_vec())
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let overlay = dunit::fs::read_to_string("/assets/dwm/sdk/motion.dss")?;
    let profile = dunit::fs::read_to_string("/sdk-theme.dss")?;
    let base = dunit::fs::read_to_string("/assets/dwm/sdk/base.dss")?;
    let config_path = "/system/share/dwm/default.toml";
    let original = dunit::fs::read_to_string(config_path)?;
    assert!(original.contains("anim = true"));
    let disabled = original.replacen("anim = true", "anim = false", 1);
    let idle = static_frame(&profile, &base, &overlay, false)?;
    let active = static_frame(&profile, &base, &overlay, true)?;
    let mut ui = ComponentWindow::open_themed_with_style(480, 240, Demo(false), &overlay)?;
    assert_eq!(ui.pixels(), idle);
    report("idle", ui.pixels())?;
    let mut stage = 0;
    let mut samples = BTreeSet::new();
    loop {
        let before = Instant::now()?;
        let event = ui.next_event(Duration::from_millis(25))?;
        if event.is_none() {
            assert!(before.elapsed()? >= Duration::from_millis(25));
        }
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        if stage == 0 && ui.component().0 && ui.is_animating() {
            let digest = hash(ui.pixels());
            if digest != hash(&idle) && digest != hash(&active) {
                assert!(ui.last_render().active_transitions > 0);
                assert!(ui.last_render().animation_time_ms.is_some());
                assert!(ui.last_uploaded_pixels() > 0);
                samples.insert(digest);
            }
        }
        match (stage, ui.component().0, ui.is_animating()) {
            (0, true, false) => {
                assert!(samples.len() >= 2);
                assert_eq!(ui.pixels(), active);
                ui.update(Msg::Noop)?;
                assert_eq!(ui.last_render().damaged_pixels, 0);
                dunit::fs::write(
                    "/sdk-theme.dss",
                    alloc::format!("{profile} Text#card {{transition:opacity 100ms;}}").as_bytes(),
                )?;
                assert!(ui.reload_theme().is_err());
                assert!(ui.theme_error().is_some());
                assert_eq!(ui.pixels(), active);
                dunit::fs::write("/sdk-theme.dss", profile.as_bytes())?;
                assert!(!ui.reload_theme()?);
                dunit::println!("[SDKMOTION] invalid retained OK")?;
                dunit::println!("[SDKMOTION] timed frames and timeout OK")?;
                report("active", ui.pixels())?;
                stage = 1;
            }
            (1, false, false) => {
                assert_eq!(ui.pixels(), idle);
                report("restored", ui.pixels())?;
                stage = 2;
            }
            (2, true, _) => {
                dunit::fs::write(config_path, disabled.as_bytes())?;
                assert!(ui.reload_theme()?);
                // An application request cannot override the desktop policy.
                ui.set_motion_enabled(true)?;
                assert!(!ui.is_animating());
                assert_eq!(ui.pixels(), active);
                ui.update(Msg::Toggle)?;
                assert!(!ui.is_animating());
                assert_eq!(ui.pixels(), idle);
                ui.update(Msg::Toggle)?;
                assert!(!ui.is_animating());
                assert_eq!(ui.pixels(), active);
                report("reduced", ui.pixels())?;
                stage = 3;
            }
            (3, false, false) => {
                dunit::fs::write(config_path, original.as_bytes())?;
                assert!(ui.reload_theme()?);
                ui.update(Msg::Toggle)?;
                assert!(ui.is_animating());
                stage = 4;
                break;
            }
            _ => {}
        }
    }
    assert_eq!(stage, 4);
    ui.close()?;
    dunit::println!("[SDKMOTION] ok")?;
    Ok(())
}
