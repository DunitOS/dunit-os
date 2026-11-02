extern crate alloc;
use alloc::{format, string::String, vec};
use dunit::{
    time::Duration,
    ui::*,
    window::{WindowEvent, IN_KEY, IN_SCROLL, IN_UP},
};
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Wheel,
    Short,
    Long,
    Drag,
}
impl Mode {
    fn index(self) -> usize {
        match self {
            Self::Wheel => 0,
            Self::Short => 1,
            Self::Long => 2,
            Self::Drag => 3,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Wheel => "wheel",
            Self::Short => "short",
            Self::Long => "long",
            Self::Drag => "drag",
        }
    }
}
#[derive(Clone)]
enum Msg {
    Mode(Mode),
    Arm,
    Text(String),
    Select(ListSelection),
    Done,
}
struct Demo {
    mode: Mode,
    rounds: [usize; 4],
    armed: bool,
    value: String,
    selection: ListSelection,
    done: bool,
}
impl Demo {
    fn label(&self) -> String {
        format!("{}-{}", self.mode.name(), self.rounds[self.mode.index()])
    }
    fn field(&self) -> String {
        format!("field-{}", self.label())
    }
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Mode(mode) => {
                self.mode = mode;
                self.rounds[mode.index()] += 1;
                self.armed = false;
                self.selection = ListSelection::default();
                self.value = match mode {
                    Mode::Long => "alpha beta gamma delta epsilon\n".repeat(400),
                    Mode::Drag => {
                        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".into()
                    }
                    _ => String::new(),
                };
            }
            Msg::Arm => self.armed = true,
            Msg::Text(s) => self.value = s,
            Msg::Select(s) => self.selection = s,
            Msg::Done => self.done = true,
        }
    }
    fn view(&self) -> View<Msg> {
        let key = self.field();
        let control = match self.mode {
            Mode::Wheel => View::list(
                key,
                "Rows",
                (0..200)
                    .map(|i| {
                        ListItem::new(format!("r{i}"), format!("Row {i:03} - continuous scroll"))
                    })
                    .collect(),
                self.selection.clone(),
                SelectionMode::Single,
                Msg::Select,
            )
            .layout(Attrs {
                height: Sizing::Grow(1.),
                width: Sizing::Grow(1.),
                ..Default::default()
            }),
            Mode::Long => View::text_area(
                key,
                "Long text",
                &self.value,
                Msg::Text,
                InputOptions::default(),
            )
            .layout(Attrs {
                height: Sizing::Px(220.),
                width: Sizing::Px(600.),
                ..Default::default()
            }),
            _ => View::input(key, "Text", &self.value, Msg::Text, InputOptions::default()).layout(
                Attrs {
                    height: Sizing::Px(48.),
                    width: Sizing::Px(600.),
                    ..Default::default()
                },
            ),
        };
        View::column(
            "root",
            vec![
                View::text("title", format!("Input latency - {}", self.label())).layout(Attrs {
                    height: Sizing::Px(32.),
                    ..Default::default()
                }),
                control,
            ],
        )
        .on_key(
            Key {
                character: 'w',
                modifiers: 2,
            },
            Msg::Mode(Mode::Wheel),
        )
        .on_key(
            Key {
                character: 's',
                modifiers: 2,
            },
            Msg::Mode(Mode::Short),
        )
        .on_key(
            Key {
                character: 'l',
                modifiers: 2,
            },
            Msg::Mode(Mode::Long),
        )
        .on_key(
            Key {
                character: 'd',
                modifiers: 2,
            },
            Msg::Mode(Mode::Drag),
        )
        .on_key(
            Key {
                character: 'g',
                modifiers: 2,
            },
            Msg::Arm,
        )
        .on_key(
            Key {
                character: 'q',
                modifiers: 2,
            },
            Msg::Done,
        )
    }
}
fn marker(s: &str) -> dunit::Result<()> {
    dunit::println!("{}", format!("[PERF] {s}"))
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let (theme, _) = load_theme("")?;
    let step = theme.tokens.number("scroll-step").unwrap();
    let mut ui = ComponentWindow::open_themed(
        640,
        360,
        Demo {
            mode: Mode::Wheel,
            rounds: [0; 4],
            armed: false,
            value: String::new(),
            selection: ListSelection::default(),
            done: false,
        },
    )?;
    ui.enable_metrics(true);
    marker("ready")?;
    let mut label = String::new();
    let mut recording = false;
    let mut completed = false;
    let mut count = 0;
    let mut base_len = 0;
    let mut probed = false;
    loop {
        let event = ui.next_event(Duration::from_secs(30))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        if ui.component().done {
            break;
        }
        let now = ui.component().label();
        if now != label {
            label = now;
            recording = false;
            completed = false;
            count = 0;
            marker(&format!("{label} setup"))?;
        }
        if ui.component().armed && !recording && !completed {
            recording = true;
            count = 0;
            base_len = ui.component().value.len();
            probed = false;
            if matches!(ui.component().mode, Mode::Short | Mode::Long) {
                assert!(
                    ui.view().children()[1].interaction().focused,
                    "benchmark text field must own focus"
                );
            }
            ui.reset_metrics();
            marker(&format!("{label} ready"))?;
            continue;
        }
        if !recording {
            continue;
        }
        let mode = ui.component().mode;
        let field = ui.component().field();
        if !probed && matches!(mode, Mode::Short | Mode::Long) {
            if let Some(WindowEvent::Input(i)) = event {
                if i.kind == IN_KEY {
                    dunit::println!(
                        "{}",
                        format!(
                            "[PERF-PROBE] {label} scan={} mods={} char={} focused={} len={}",
                            i.x,
                            i.y,
                            i.value,
                            ui.view().children()[1].interaction().focused,
                            ui.component().value.len()
                        )
                    )?;
                    probed = true;
                }
            }
        }
        let done = match mode {
            Mode::Wheel => {
                if matches!(event,Some(WindowEvent::Input(i))if i.kind==IN_SCROLL) {
                    count += 1;
                }
                if count == 24 {
                    assert!((ui.scroll_offset(&field).unwrap() - step * 24.).abs() < 0.01);
                    true
                } else {
                    false
                }
            }
            Mode::Short | Mode::Long => {
                if matches!(event,Some(WindowEvent::Input(i))if i.kind==IN_KEY&&i.value==b'a' as u32&&i.y==0)
                {
                    count += 1;
                }
                if count == 24 {
                    assert_eq!(ui.component().value.len(), base_len + 24);
                    assert!(ui.component().value.ends_with(&"a".repeat(24)));
                    true
                } else {
                    false
                }
            }
            Mode::Drag => {
                if matches!(event,Some(WindowEvent::Input(i))if i.kind==IN_UP) {
                    let e = ui.view().children()[1].text_editor().unwrap();
                    assert!(e.selection().len() >= 30);
                    assert_eq!(
                        e.text(),
                        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
                    );
                    assert!(!e.can_undo());
                    true
                } else {
                    false
                }
            }
        };
        if done {
            let metrics = ui.metrics();
            marker(&format!("{label} done"))?;
            dunit::println!(
                "{}",
                format!("[PERF-METRICS] phase={label} accepted={count} {metrics:?}")
            )?;
            recording = false;
            completed = true;
        }
    }
    ui.close()?;
    marker("ok")?;
    Ok(())
}
