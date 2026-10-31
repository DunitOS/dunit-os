extern crate alloc;
use alloc::{format, vec};
use dunit::ui::{
    Attrs, Component, ComponentWindow, Key, ProgressOptions, ProgressState, Sizing, View,
};
use dunit::{
    io::Write,
    time::{Duration, Instant},
    window::{WindowEvent, IN_UP},
};
struct Demo {
    value: i64,
    running: bool,
    steps: u32,
    toggles: u32,
    done: bool,
}
#[derive(Clone)]
enum Msg {
    Step,
    Toggle,
    Done,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Step => {
                self.value = (self.value + 25).min(100);
                self.steps += 1;
            }
            Msg::Toggle => {
                self.running = !self.running;
                self.toggles += 1;
            }
            Msg::Done => self.done = true,
        }
    }
    fn view(&self) -> View<Msg> {
        let bar = |v: View<Msg>| {
            v.layout(Attrs {
                width: Sizing::Px(300.),
                height: Sizing::Px(32.),
                ..Default::default()
            })
        };
        View::column(
            "root",
            vec![
                bar(View::progress(
                    "download",
                    "Download",
                    self.value,
                    ProgressOptions::new(0, 100),
                )),
                bar(View::busy("work", "Working", self.running)),
                bar(View::progress(
                    "disabled",
                    "Disabled",
                    60,
                    ProgressOptions::new(0, 100),
                ))
                .disabled(true),
                bar(View::busy("paused", "Paused", false)),
                View::button("step", "Advance 25%", Msg::Step),
                View::button("toggle", "Pause / resume busy", Msg::Toggle),
                View::text(
                    "status",
                    format!("Progress {}% | busy {}", self.value, self.running),
                ),
                View::text("help", "Bars: progress / busy / disabled / paused"),
            ],
        )
        .on_key(Key::plain('q'), Msg::Done)
    }
}
fn focused(ui: &ComponentWindow<Demo>, key: &str) -> bool {
    ui.view()
        .children()
        .iter()
        .find(|v| v.key() == key)
        .unwrap()
        .interaction()
        .focused
}
fn marker(value: &str) -> dunit::Result<()> {
    dunit::io::stdout().write_all(format!("[PROGRESS] {value}\n").as_bytes())
}
fn wait(ui: &mut ComponentWindow<Demo>, ms: u64) -> dunit::Result<()> {
    let start = Instant::now()?;
    // Drive the event pump at least once, and require a real quiet timeout.
    // Pending protocol events must not turn this into a clock-only wait.
    loop {
        let event = ui.next_event(Duration::from_millis(ms))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        if event.is_none() {
            return Ok(());
        }
        assert!(
            start.elapsed()? < Duration::from_secs(5),
            "event queue never became idle"
        );
    }
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        480,
        320,
        Demo {
            value: 25,
            running: true,
            steps: 0,
            toggles: 0,
            done: false,
        },
    )?;
    assert_eq!(ui.last_render().active_busy_indicators, 1);
    assert!(ui.is_animating());
    let initial = ui.last_render().animation_time_ms.unwrap();
    let initial_pixels = ui.pixels().to_vec();
    // TCG can spend longer painting/publishing than a sampling interval.
    // Wait for the same required clock advance under an overall deadline,
    // without imposing an unmeasured frame-rate budget on the renderer.
    let deadline = Instant::now()?;
    loop {
        wait(&mut ui, 240)?;
        if ui.last_render().animation_time_ms.unwrap() >= initial + 100 {
            break;
        }
        assert!(
            deadline.elapsed()? < Duration::from_secs(10),
            "busy sampling stalled"
        );
    }
    assert!(
        ui.last_render().animation_time_ms.unwrap() >= initial + 100,
        "busy clock did not advance: initial={}, last={:?}",
        initial,
        ui.last_render().animation_time_ms
    );
    assert_eq!(ui.last_render().active_busy_indicators, 1);
    assert!(ui.last_render().damaged_pixels > 0 && ui.last_render().damaged_pixels < 480 * 320);
    assert_eq!(ui.component().steps, 0);
    assert_ne!(initial_pixels, ui.pixels());
    marker("timed busy OK")?;
    ui.set_motion_enabled(false)?;
    assert!(!ui.is_animating());
    assert_eq!(ui.last_render().active_busy_indicators, 0);
    let still = ui.last_render().animation_time_ms;
    wait(&mut ui, 200)?;
    assert_eq!(ui.last_render().animation_time_ms, still);
    marker("reduced motion no timer OK")?;
    ui.set_motion_enabled(true)?;
    assert!(ui.is_animating());
    assert_eq!(ui.last_render().active_busy_indicators, 1);
    marker("ready")?;
    let mut stage = 0;
    loop {
        let event = ui.next_event(Duration::from_secs(30))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let status = match stage {
            0 if focused(&ui, "step") => Some("passive tab skip OK"),
            1 if ui.component().value == 50 => {
                assert_eq!(ui.component().steps, 1);
                Some("model update OK")
            }
            2 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_UP) => {
                assert_eq!(ui.component().steps, 1);
                assert!(!focused(&ui, "step"));
                assert!(!ui.view().children()[0].interaction().pressed);
                Some("passive pointer OK")
            }
            3 if ui.component().value == 75 => Some("next step OK"),
            4 if ui.component().value == 100 => {
                assert_eq!(ui.component().steps, 3);
                assert_eq!(
                    ui.view().children()[0].progress_state(),
                    Some(ProgressState::Determinate {
                        value: 100,
                        options: ProgressOptions::new(0, 100)
                    })
                );
                Some("complete OK")
            }
            5 if !ui.component().running => {
                assert!(focused(&ui, "toggle"));
                assert_eq!(ui.component().toggles, 1);
                assert!(!ui.is_animating());
                assert_eq!(ui.last_render().active_busy_indicators, 0);
                let still = ui.last_render().animation_time_ms;
                wait(&mut ui, 200)?;
                assert_eq!(ui.last_render().animation_time_ms, still);
                Some("pause no timer OK")
            }
            6 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 480);
                assert!(focused(&ui, "toggle"));
                assert!(!ui.is_animating());
                Some("resize focus OK")
            }
            7 if ui.component().running => {
                assert_eq!(ui.component().toggles, 2);
                assert!(ui.is_animating());
                assert_eq!(ui.last_render().active_busy_indicators, 1);
                Some("resume OK")
            }
            8 if ui.component().done => break,
            _ => None,
        };
        if let Some(status) = status {
            marker(status)?;
            stage += 1;
        }
    }
    ui.close()?;
    marker("ok")?;
    Ok(())
}
