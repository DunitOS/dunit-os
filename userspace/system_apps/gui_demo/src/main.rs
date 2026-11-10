#![no_std]
#![no_main]
//! Notification showcase. SDK owns controls, input routing and surface lifetime.
extern crate alloc;
use alloc::vec;
use dunit::ui::{Component, ComponentWindow, Renderer, View};
use dunit::{
    time::Duration,
    window::{PixelFormat, WindowEvent, IN_KEY},
};
#[derive(Clone, Copy)]
struct DemoButton {
    label: &'static str,
    notify: &'static str,
}

/// The fixed button column. Row 0 is the primary (accent) action; the rest post
/// distinct toasts so the notification queue/stack is visibly exercised.
const BUTTONS: [DemoButton; 4] = [
    DemoButton {
        label: "Say hello",
        notify: "Hello from gui_demo",
    },
    DemoButton {
        label: "Post info",
        notify: "Info: everything nominal",
    },
    DemoButton {
        label: "Warn me",
        notify: "Warning: this is a test",
    },
    DemoButton {
        label: "Report OK",
        notify: "All systems go",
    },
];

#[derive(Clone)]
enum Message {
    Notify(usize),
    Sent,
}
struct Demo {
    pending: Option<usize>,
}
impl Component for Demo {
    type Message = Message;
    fn update(&mut self, message: Message) {
        self.pending = match message {
            Message::Notify(i) => Some(i),
            Message::Sent => None,
        };
    }
    fn view(&self) -> View<Message> {
        let mut children = vec![
            View::text("title", "Dunit Demo"),
            View::text("subtitle", "Notifications / self-tests"),
        ];
        for (i, button) in BUTTONS.iter().enumerate() {
            children.push(
                View::button(
                    alloc::format!("notify{}", i),
                    button.label,
                    Message::Notify(i),
                )
                .class(if i == 0 { "primary" } else { "secondary" }),
            );
        }
        View::column("demo", children)
    }
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let cfg = dwm_settings::DemoCfg::load("gui_demo");
    let format = if cfg.bg_alpha < 255 {
        PixelFormat::Argb8888
    } else {
        PixelFormat::Xrgb8888
    };
    dunit::println!(
        "gui_demo: cfg from_file={} bg={:#010X} accent={:#010X} bg_alpha={} fmt={}",
        cfg.from_file as u32,
        cfg.bg,
        cfg.accent,
        cfg.bg_alpha,
        if cfg.bg_alpha < 255 {
            "argb8888"
        } else {
            "xrgb8888"
        }
    )?;
    let overlay = alloc::format!("Column#demo {{ padding: 10; gap: 10; background: #00000000; }} Text {{ color: #{:06X}; }} Text#title {{ font-size: 20; }} Text#subtitle {{ color: #{:06X}; font-size: 12; }} Button {{ background: #{:06X}; color: #{:06X}; height: fill; min-height: 32; }} Button.primary {{ background: #{:06X}; }}", cfg.text & 0xffffff, cfg.muted & 0xffffff, cfg.btn & 0xffffff, cfg.text & 0xffffff, cfg.accent & 0xffffff);
    let (theme, _) = dunit::ui::load_theme(&overlay)?;
    let desktop = dwm_settings::load();
    let font = dunit::canvas::Font::load(&[
        cfg.font.as_str(),
        desktop.desktop.font.as_str(),
        "/assets/fonts/DejaVuSans.ttf",
    ])?;
    let mut ui = ComponentWindow::open_format(
        260,
        300,
        Demo { pending: None },
        Renderer::new(
            font.into_inner(),
            theme.cascade,
            dunit::canvas::Color::rgba(
                (cfg.bg >> 16) as u8,
                (cfg.bg >> 8) as u8,
                cfg.bg as u8,
                cfg.bg_alpha as u8,
            ),
        ),
        format,
    )?;
    dunit::println!("gui_demo: surface presented OK")?;
    ui.notify("gui_demo started")?;
    dunit::println!("gui_demo: notify sent \"gui_demo started\"")?;
    loop {
        match ui.next_event(Duration::from_millis(40))? {
            Some(WindowEvent::CloseRequested) => break,
            Some(WindowEvent::Resized { .. }) => {
                dunit::println!("gui_demo: reconfigured OK")?;
            }
            Some(WindowEvent::Input(input)) if input.kind == IN_KEY => {
                if let Some(i) = (input.value as u8)
                    .checked_sub(b'1')
                    .filter(|i| (*i as usize) < BUTTONS.len())
                {
                    ui.update(Message::Notify(i as usize))?;
                }
            }
            _ => {}
        }
        if let Some(i) = ui.component().pending {
            ui.notify(BUTTONS[i].notify)?;
            dunit::println!("gui_demo: notify sent \"{}\"", BUTTONS[i].notify)?;
            ui.update(Message::Sent)?;
        }
    }
    ui.close()
}
