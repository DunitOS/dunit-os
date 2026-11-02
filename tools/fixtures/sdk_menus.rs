extern crate alloc;
use alloc::{format, string::String, vec, vec::Vec};
use dunit::ui::{
    Attrs, Component, ComponentWindow, Key, MenuItem as I, SelectOption as O, Sizing, View,
};
use dunit::{
    io::Write,
    time::Duration,
    window::{WindowEvent, IN_UP},
};
struct Demo {
    selected: String,
    picks: u32,
    apply: u32,
    actions: Vec<&'static str>,
    done: bool,
}
#[derive(Clone)]
enum Msg {
    Pick(String),
    Action(&'static str),
    Apply,
    Done,
}
fn files() -> Vec<I<Msg>> {
    vec![
        I::action("open", "Open", Msg::Action("open")).mnemonic('o'),
        I::separator("sep"),
        I::action("locked", "Unavailable", Msg::Action("bad")).disabled(true),
        I::submenu(
            "recent",
            "Recent",
            vec![
                I::action("one", "One", Msg::Action("one")),
                I::action("locked-child", "Unavailable", Msg::Action("bad")).disabled(true),
                I::action("two", "Two", Msg::Action("two")),
            ],
        ),
        I::action("last", "Last", Msg::Action("last")),
    ]
}
fn fixed(v: View<Msg>, width: f32) -> View<Msg> {
    v.layout(Attrs {
        width: Sizing::Px(width),
        height: Sizing::Px(32.),
        ..Default::default()
    })
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Pick(s) => {
                self.selected = s;
                self.picks += 1;
            }
            Msg::Action(a) => self.actions.push(a),
            Msg::Apply => self.apply += 1,
            Msg::Done => self.done = true,
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "root",
            vec![
                View::menu_bar(
                    "bar",
                    vec![
                        fixed(View::menu("file", "File", files()).menu_mnemonic('f'), 80.),
                        fixed(
                            View::menu(
                                "edit",
                                "Edit",
                                vec![I::action("copy", "Copy", Msg::Action("copy"))],
                            )
                            .menu_mnemonic('e'),
                            80.,
                        ),
                    ],
                ),
                fixed(
                    View::select(
                        "pick",
                        "Language",
                        Some(&self.selected),
                        vec![
                            O::new("a", "Alpha"),
                            O::new("locked", "Unavailable").disabled(true),
                            O::new("b", "Beta"),
                            O::new("c", "Charlie"),
                        ],
                        Msg::Pick,
                    ),
                    240.,
                ),
                fixed(
                    View::button("help", "Hover / right-click here", Msg::Action("help"))
                        .tooltip("Delayed tooltip: this text wraps to fit the client viewport.")
                        .context_menu(vec![
                            I::action("inspect", "Inspect", Msg::Action("inspect")),
                            I::submenu(
                                "more",
                                "More",
                                vec![I::action("detail", "Detail", Msg::Action("detail"))],
                            ),
                        ]),
                    240.,
                ),
                fixed(View::button("apply", "Apply", Msg::Apply), 300.),
                View::text(
                    "status",
                    format!(
                        "Selected {} | picks {} | actions {}",
                        self.selected,
                        self.picks,
                        self.actions.len()
                    ),
                ),
                View::text("hint", "Alt+F / Alt+E | F10 | Shift+F10 | Esc"),
            ],
        )
        .on_key(Key::plain('q'), Msg::Done)
    }
}
fn find<'a>(v: &'a View<Msg>, key: &str) -> Option<&'a View<Msg>> {
    if v.key() == key {
        Some(v)
    } else {
        v.children().iter().find_map(|v| find(v, key))
    }
}
fn focused(ui: &ComponentWindow<Demo>, key: &str) -> bool {
    find(ui.view(), key).unwrap().interaction().focused
}
fn owner(ui: &ComponentWindow<Demo>, key: &str) -> bool {
    ui.popup_state().is_some_and(|p| p.owner == key)
}
fn path(ui: &ComponentWindow<Demo>, p: &[usize]) -> bool {
    ui.popup_state().is_some_and(|v| v.path == p)
}
fn marker(value: &str) -> dunit::Result<()> {
    dunit::io::stdout().write_all(format!("[MENUS] {value}\n").as_bytes())
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        640,
        400,
        Demo {
            selected: "a".into(),
            picks: 0,
            apply: 0,
            actions: Vec::new(),
            done: false,
        },
    )?;
    marker("ready")?;
    let mut stage = 0;
    let mut outside_armed = false;
    let mut tip_since = 0;
    loop {
        let event = ui.next_event(Duration::from_millis(100))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        assert!(!ui.component().actions.contains(&"bad"));
        // Native Enter opens this control on release. The harness must observe
        // the reopened popup before testing an outside click, not race its make.
        if stage == 6 && !outside_armed && owner(&ui, "pick") {
            assert!(ui.popup_state().is_some());
            outside_armed = true;
            marker("outside guard popup open OK")?;
        }
        let status = match stage {
            0 if focused(&ui, "file") => Some("bar tab focus OK"),
            1 if focused(&ui, "pick") => Some("bar one tab stop OK"),
            2 if owner(&ui, "pick") && path(&ui, &[0]) => Some("select open OK"),
            3 if owner(&ui, "pick") && path(&ui, &[2]) => {
                assert_eq!(ui.component().selected, "a");
                assert_eq!(ui.component().picks, 0);
                Some("disabled skip pending OK")
            }
            4 if ui.popup_state().is_none() => {
                assert_eq!(ui.component().selected, "a");
                assert!(focused(&ui, "pick"));
                Some("select escape OK")
            }
            5 if ui.component().selected == "b" => {
                assert_eq!(ui.component().picks, 1);
                assert!(ui.popup_state().is_none());
                Some("pointer option OK")
            }
            6 if outside_armed && ui.popup_state().is_none()
                && matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_UP) =>
            {
                assert_eq!(ui.component().apply, 0);
                Some("outside no click through OK")
            }
            7 if ui.component().apply == 1 => Some("ordinary button OK"),
            8 if ui.tooltip_candidate().is_some_and(|(key, _)| key == "help") => {
                assert!(ui.visible_tooltip().is_none());
                tip_since = ui.tooltip_candidate().unwrap().1;
                Some("tooltip armed OK")
            }
            9 if ui.visible_tooltip() == Some("help") => {
                assert!(
                    ui.last_render().animation_time_ms.unwrap()
                        >= tip_since + ui.tooltip_delay_ms("help").unwrap()
                );
                Some("tooltip deadline OK")
            }
            10 if focused(&ui, "file") && ui.visible_tooltip().is_none() => {
                Some("f10 dismisses tooltip OK")
            }
            11 if owner(&ui, "file") && path(&ui, &[0]) => Some("menu open OK"),
            12 if path(&ui, &[3]) => Some("menu disabled separator skip OK"),
            13 if path(&ui, &[3, 0]) => Some("submenu open OK"),
            14 if path(&ui, &[3, 2]) => Some("submenu disabled skip OK"),
            15 if ui.component().actions.contains(&"two") => {
                assert!(focused(&ui, "file"));
                assert!(ui.popup_state().is_none());
                Some("submenu action restore OK")
            }
            16 if owner(&ui, "edit") => Some("alt mnemonic OK"),
            17 if owner(&ui, "file") => Some("bar switch OK"),
            18 if ui.component().actions.contains(&"open") => Some("item mnemonic OK"),
            19 if ui
                .popup_state()
                .is_some_and(|p| p.owner == "help" && p.context) =>
            {
                Some("keyboard context OK")
            }
            20 if ui.component().actions.contains(&"detail") => {
                assert!(focused(&ui, "help"));
                assert!(ui.popup_state().is_none());
                Some("context submenu restore OK")
            }
            21 if ui.popup_state().is_some_and(|p| {
                p.owner == "help" && p.context && p.point == Some((130., 92.))
            }) =>
            {
                Some("right click context OK")
            }
            22 if ui.component().actions.contains(&"inspect") => {
                assert_eq!(ui.component().apply, 1);
                assert!(focused(&ui, "help"));
                Some("context pointer action OK")
            }
            23 if owner(&ui, "file") && path(&ui, &[3, 0]) => Some("resize menu ready"),
            24 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 640);
                assert!(owner(&ui, "file"));
                assert!(path(&ui, &[3, 0]));
                assert!(focused(&ui, "file"));
                Some("resize popup focus OK")
            }
            25 if path(&ui, &[3]) => Some("escape one level OK"),
            26 if ui.popup_state().is_none() => {
                assert!(focused(&ui, "file"));
                Some("escape restores bar OK")
            }
            27 if ui.component().done => break,
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
