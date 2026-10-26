extern crate alloc;
use alloc::{format, vec};
use dunit::ui::{
    style, Attrs, Cascade, Color, Component, ComponentWindow, Font, Key, Renderer, Sizing, View,
};
use dunit::{
    time::Duration,
    window::{WindowEvent, IN_KEY},
};

struct Counter {
    count: u32,
}
#[derive(Clone)]
enum Msg {
    Increment,
}
impl Component for Counter {
    type Message = Msg;
    fn update(&mut self, _: Msg) {
        self.count += 1;
    }
    fn view(&self) -> View<Msg> {
        let line = |id: &str, text: alloc::string::String| {
            View::text(id, text).layout(Attrs {
                height: Sizing::Px(32.0),
                ..Attrs::default()
            })
        };
        View::column(
            "root",
            vec![
                line("title", "Component counter".into()),
                line("count", format!("Count: {}", self.count)),
                line("hint", "Press Space".into()),
            ],
        )
        .on_key(Key::plain(' '), Msg::Increment)
    }
}

#[dunit::main]
fn main() -> dunit::Result<()> {
    let font = Font::parse(dunit::fs::read(dwm_settings::load().desktop.font.as_str())?)
        .map_err(|_| dunit::Error(dunit::errno::EINVAL))?;
    let mut cascade = Cascade::new();
    cascade.push(
        style::parse("Column { background: #202830; } Text { color: #ffffff; font-size: 16; }")
            .unwrap(),
    );
    let renderer = Renderer::new(font, cascade, Color::rgb(0, 0, 0));
    let initial = (480, 240);
    let mut ui = ComponentWindow::open(initial.0, initial.1, Counter { count: 0 }, renderer)?;
    assert_eq!(ui.last_render().damaged_pixels, 480 * 240);
    dunit::println!("[SDKUI] ready")?;
    let mut changed = false;
    let mut maximized = false;
    let mut restored = false;
    loop {
        match ui.next_event(Duration::from_secs(60))? {
            Some(WindowEvent::Input(input))
                if input.kind == IN_KEY && input.value == b' ' as u32 =>
            {
                assert_eq!(ui.component().count, 1);
                assert_eq!(ui.view().children()[1].content(), Some("Count: 1"));
                dunit::println!("[SDKUI] input update OK")?;
                let damage = ui
                    .last_render()
                    .damage
                    .expect("changed label had no damage");
                assert_eq!(
                    (damage.x, damage.y, damage.width, damage.height),
                    (0, 32, 480, 32)
                );
                assert_eq!(ui.last_render().damaged_pixels, 480 * 32);
                changed = true;
                dunit::println!("[SDKUI] partial repaint OK")?;
            }
            Some(WindowEvent::Resized { .. }) => {
                assert_eq!(ui.component().count, 1);
                if ui.size() != initial {
                    maximized = true;
                    dunit::println!("[SDKUI] resized OK")?;
                } else if maximized {
                    restored = true;
                    dunit::println!("[SDKUI] restored OK")?;
                }
            }
            Some(WindowEvent::CloseRequested) => break,
            _ => {}
        }
    }
    assert!(changed && maximized && restored);
    ui.close()?;
    dunit::println!("[SDKUI] ok")?;
    Ok(())
}
