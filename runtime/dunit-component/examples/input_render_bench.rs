//! Host-only rendering benchmark for the native input-latency fixture scenes.
//! Run with `cargo run --release --example input_render_bench` from this crate.
//! Times are host monotonic wall durations, not exclusive CPU accounting.
use dunit_component::{
    Application, Attrs, Component, Font, InputOptions, Key, ListItem, ListSelection, PointerEvent,
    RenderStats, Renderer, SelectionMode, Sizing, Theme, View,
};
use std::{hint::black_box, time::Instant};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const EVENTS: usize = 24;
const TRIALS: usize = 5;

#[derive(Clone, Copy)]
enum Scene {
    Wheel,
    Short,
    Long,
}
impl Scene {
    fn field(self) -> &'static str {
        match self {
            Self::Wheel => "field-wheel-1",
            Self::Short => "field-short-1",
            Self::Long => "field-long-1",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Wheel => "wheel",
            Self::Short => "short",
            Self::Long => "long",
        }
    }
}
#[derive(Clone)]
enum Message {
    Text(String),
    Select(ListSelection),
}
struct Demo {
    scene: Scene,
    value: String,
    selection: ListSelection,
}
impl Component for Demo {
    type Message = Message;
    fn update(&mut self, message: Message) {
        match message {
            Message::Text(value) => self.value = value,
            Message::Select(selection) => self.selection = selection,
        }
    }
    fn view(&self) -> View<Message> {
        let control = match self.scene {
            Scene::Wheel => View::list(
                self.scene.field(),
                "Rows",
                (0..200)
                    .map(|i| {
                        ListItem::new(format!("r{i}"), format!("Row {i:03} - continuous scroll"))
                    })
                    .collect(),
                self.selection.clone(),
                SelectionMode::Single,
                Message::Select,
            )
            .layout(Attrs {
                height: Sizing::Grow(1.),
                width: Sizing::Grow(1.),
                ..Default::default()
            }),
            Scene::Long => View::text_area(
                self.scene.field(),
                "Long text",
                &self.value,
                Message::Text,
                InputOptions::default(),
            )
            .layout(Attrs {
                height: Sizing::Px(220.),
                width: Sizing::Px(600.),
                ..Default::default()
            }),
            Scene::Short => View::input(
                self.scene.field(),
                "Text",
                &self.value,
                Message::Text,
                InputOptions::default(),
            )
            .layout(Attrs {
                height: Sizing::Px(48.),
                width: Sizing::Px(600.),
                ..Default::default()
            }),
        };
        View::column(
            "root",
            vec![
                View::text("title", format!("Input latency - {}-1", self.scene.name())).layout(
                    Attrs {
                        height: Sizing::Px(32.),
                        ..Default::default()
                    },
                ),
                control,
            ],
        )
    }
}
#[derive(Debug)]
struct Trial {
    wall_us: u128,
    render_us: u128,
    update_us: u128,
    damaged_pixels: u64,
    damaged_frames: usize,
}
fn render(renderer: &mut Renderer, app: &Application<Demo>) -> RenderStats {
    // Fixed animation time keeps this an event/render benchmark, independent of
    // native timer scheduling, caret blinking and idle scrollbar deadlines.
    renderer
        .render_application_at(app, WIDTH, HEIGHT, 0)
        .unwrap()
}
fn trial(scene: Scene) -> Trial {
    let theme = Theme::parse(
        include_str!("../../../assets/dwm/sdk/dark.dss"),
        include_str!("../../../assets/dwm/sdk/base.dss"),
        "",
    )
    .unwrap();
    let step = theme.tokens.number("scroll-step").unwrap();
    let mut renderer = Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        theme.cascade,
        theme.background,
    );
    renderer.set_motion_enabled(false);
    let value = match scene {
        Scene::Long => "alpha beta gamma delta epsilon\n".repeat(400),
        _ => String::new(),
    };
    let base_len = value.len();
    let field = scene.field();
    let mut app = Application::new(Demo {
        scene,
        value,
        selection: ListSelection::default(),
    })
    .unwrap();
    render(&mut renderer, &app);
    match scene {
        Scene::Wheel => {
            app.pointer(PointerEvent::Move, Some(field)).unwrap();
        }
        _ => {
            assert!(app.key(Key::plain('\t')).unwrap());
            assert!(app.view().children()[1].interaction().focused);
        }
    }
    render(&mut renderer, &app);
    let mut result = Trial {
        wall_us: 0,
        render_us: 0,
        update_us: 0,
        damaged_pixels: 0,
        damaged_frames: 0,
    };
    let mut render_ns = 0;
    let mut update_ns = 0;
    let wall = Instant::now();
    for _ in 0..EVENTS {
        let update = Instant::now();
        match scene {
            Scene::Wheel => assert!(renderer.scroll_wheel(Some(field), 1)),
            _ => assert!(app.key(Key::plain('a')).unwrap()),
        }
        update_ns += update.elapsed().as_nanos();
        let paint = Instant::now();
        let stats = render(&mut renderer, &app);
        render_ns += paint.elapsed().as_nanos();
        result.damaged_pixels += stats.damaged_pixels;
        result.damaged_frames += usize::from(stats.damage.is_some());
    }
    result.wall_us = wall.elapsed().as_micros();
    result.render_us = render_ns / 1000;
    result.update_us = update_ns / 1000;
    black_box(renderer.pixels());
    assert_eq!(result.damaged_frames, EVENTS);
    match scene {
        Scene::Wheel => {
            assert!((renderer.scroll_offset(field).unwrap() - step * EVENTS as f32).abs() < 0.01);
            assert!(renderer.list_visible_rows(field).unwrap() < 30);
        }
        _ => {
            assert_eq!(app.component().value.len(), base_len + EVENTS);
            assert!(app.component().value.ends_with(&"a".repeat(EVENTS)));
        }
    }
    result
}
fn median(mut samples: Vec<u128>) -> u128 {
    samples.sort_unstable();
    samples[samples.len() / 2]
}
fn main() {
    for scene in [Scene::Wheel, Scene::Short, Scene::Long] {
        black_box(trial(scene)); // Complete unmeasured code/font/page warmup.
        let samples: Vec<_> = (0..TRIALS).map(|_| trial(scene)).collect();
        assert!(samples
            .iter()
            .all(|s| s.damaged_pixels == samples[0].damaged_pixels));
        let wall: Vec<_> = samples.iter().map(|s| s.wall_us).collect();
        let render: Vec<_> = samples.iter().map(|s| s.render_us).collect();
        let update: Vec<_> = samples.iter().map(|s| s.update_us).collect();
        println!(
            "{{\"case\":\"{}\",\"clock\":\"host_monotonic_wall\",\"viewport\":[{},{}],\"events\":{},\"warmups\":1,\"trials\":{},\"median_wall_us\":{},\"median_render_us\":{},\"median_update_us\":{},\"damaged_frames\":{},\"damaged_pixels\":{},\"wall_samples_us\":{:?},\"render_samples_us\":{:?},\"update_samples_us\":{:?}}}",
            scene.name(), WIDTH, HEIGHT, EVENTS, TRIALS,
            median(wall.clone()),median(render.clone()),median(update.clone()),
            samples[0].damaged_frames,samples[0].damaged_pixels,wall,render,update
        );
    }
}
