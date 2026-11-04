extern crate alloc;
use alloc::vec;
use dunit::ui::{Component, ComponentWindow, Key, View};
use dunit::{time::Duration, window::WindowEvent};
struct Counter(u32);
impl Component for Counter {
    type Message = ();
    fn update(&mut self, _: ()) {
        self.0 += 1;
    }
    fn view(&self) -> View<()> {
        View::column(
            "root",
            vec![
                View::text("title", "Dunit SDK"),
                View::text("value", alloc::format!("State {}", self.0)),
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
fn background(profile: &str) -> u32 {
    let sheet = dunit::ui::style::parse(profile).unwrap();
    let c = sheet.tokens.color("window-bg").unwrap();
    0xff000000 | ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let dark = dunit::fs::read_to_string("/assets/dwm/sdk/dark.dss")?;
    let light = alloc::format!(
        "{}\n$shell-cursor-fill: #ff00ff;\n$shell-control-right-inset: 18;\n",
        dunit::fs::read_to_string("/assets/dwm/sdk/light.dss")?
    );
    let config_path = "/system/share/dwm/default.toml";
    // Setup's scalar setter and explicit partial import ran before this app.
    let saved = dunit::fs::read_to_string(config_path)?;
    assert_eq!(
        dwm_settings::value(&saved, "notifications.timeout_ms"),
        Some("4000")
    );
    assert_eq!(dwm_settings::value(&saved, "effects.blur_iters"), Some("2"));
    let before = dwm_settings::load();
    let mut edited = before;
    edited.notifications.timeout_ms = 4500;
    assert!(dwm_settings::save_changes(&before, &edited));
    let untouched = dunit::fs::read_to_string(config_path)?;
    assert!(dwm_settings::save_changes(&edited, &edited));
    assert_eq!(dunit::fs::read_to_string(config_path)?, untouched);
    let mut invalid = edited;
    invalid.layout.dock_w = 99999;
    assert!(!dwm_settings::save_changes(&edited, &invalid));
    assert_eq!(dunit::fs::read_to_string(config_path)?, untouched);
    edited.effects.anim_ms += 1;
    assert!(dwm_settings::save(&edited));
    let original = dunit::fs::read_to_string(config_path)?;
    for field in ["theme.panel", "layout.title_h", "effects.shadow_alpha"] {
        assert!(
            dwm_settings::value(&original, field).is_none(),
            "implicit {field} was pinned"
        );
    }
    assert_eq!(
        dwm_settings::value(&original, "notifications.timeout_ms"),
        Some("4500")
    );
    assert_eq!(
        dwm_settings::value(&original, "effects.blur_iters"),
        Some("2")
    );
    dunit::println!("[SDKTHEME] sparse scalar save retained inheritance OK")?;
    assert!(original.contains("\"/sdk-theme.dss\""));
    let switched = original.replacen("\"/sdk-theme.dss\"", "\"/sdk-theme-next.dss\"", 1);
    let mut ui = ComponentWindow::open_themed(480, 240, Counter(0))?;
    assert_eq!(ui.pixels()[0], background(&dark));
    dunit::println!(
        "{}",
        alloc::format!("[SDKTHEME] dark hash={:016x}", hash(ui.pixels()))
    )?;
    let mut stage = 0;
    let mut rejected = false;
    loop {
        let event = ui.next_event(Duration::from_millis(500))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        match (stage, ui.component().0) {
            (0, 1) => {
                dunit::fs::write("/sdk-theme-next.dss", light.as_bytes())?;
                dunit::fs::write(config_path, switched.as_bytes())?;
                ui.reload_desktop()?;
                stage = 1;
            }
            (1, 1) if ui.pixels()[0] == background(&light) => {
                assert_eq!(ui.last_render().damaged_pixels, 480 * 240);
                let before = hash(ui.pixels());
                dunit::fs::write(
                    "/sdk-theme-next.dss",
                    b"$window-bg: #ffffff; Text {color:$missing;}",
                )?;
                assert!(ui.reload_theme().is_err());
                assert!(ui.theme_error().is_some());
                assert!(ui.theme_error_detail().unwrap().contains("unknown token"));
                assert_eq!(hash(ui.pixels()), before);
                dunit::fs::write(
                    "/sdk-theme-next.dss",
                    alloc::format!("{} Column#root {{padding:-1;}}", light).as_bytes(),
                )?;
                assert!(ui.reload_theme().is_err());
                assert!(ui.theme_error_detail().unwrap().contains("padding"));
                assert_eq!(hash(ui.pixels()), before);
                // A valid SDK profile with invalid shell geometry must be
                // rejected by both native consumers, retaining the light scene.
                dunit::fs::write(
                    "/sdk-theme-next.dss",
                    alloc::format!("{}\n$shell-qs-width: 1;", light).as_bytes(),
                )?;
                assert!(ui.reload_theme().is_err());
                assert!(ui.theme_error_detail().unwrap().contains("shell-qs-width"));
                assert_eq!(hash(ui.pixels()), before);
                ui.reload_desktop()?;
                rejected = true;
                dunit::println!("[SDKTHEME] invalid retained OK")?;
                dunit::println!(
                    "{}",
                    alloc::format!("[SDKTHEME] light hash={:016x}", before)
                )?;
                stage = 2;
            }
            (2, 2) => {
                dunit::fs::write(config_path, original.as_bytes())?;
                ui.reload_desktop()?;
                stage = 3;
            }
            (3, 2) if ui.pixels()[0] == background(&dark) => {
                assert_eq!(ui.last_render().damaged_pixels, 480 * 240);
                assert!(ui.theme_error().is_none());
                assert!(rejected);
                dunit::println!(
                    "{}",
                    alloc::format!("[SDKTHEME] restored hash={:016x}", hash(ui.pixels()))
                )?;
                stage = 4;
            }
            (4, 3) => break,
            _ => {}
        }
    }
    assert_eq!(stage, 4);
    ui.close()?;
    dunit::println!("[SDKTHEME] ok")?;
    Ok(())
}
