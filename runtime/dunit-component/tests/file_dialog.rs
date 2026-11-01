use dunit_component::{
    Application, Component, DirectorySource, FileDialog, FileDialogLabels as Labels,
    FileDialogMessage as M, FileDialogMode as Mode, FileDialogOptions as Options,
    FileDialogOutcome as Outcome, FileEntry as Entry, FileFilter as Filter, FileKind as Kind, Font,
    Key, Renderer, TableState, Theme, View,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
#[derive(Clone)]
struct Source(Rc<RefCell<BTreeMap<String, Vec<Entry>>>>);
impl DirectorySource for Source {
    fn read_directory(&mut self, p: &str) -> Result<Vec<Entry>, String> {
        self.0
            .borrow()
            .get(p)
            .cloned()
            .ok_or_else(|| format!("cannot read {p}"))
    }
    fn metadata(&mut self, p: &str) -> Result<Option<Kind>, String> {
        let fs = self.0.borrow();
        if fs.contains_key(p) {
            return Ok(Some(Kind::Directory));
        }
        let (parent, name) = p.rsplit_once('/').unwrap_or(("", ""));
        let parent = if parent.is_empty() { "/" } else { parent };
        Ok(fs
            .get(parent)
            .and_then(|entries| entries.iter().find(|e| e.name == name).map(|e| e.kind)))
    }
}
fn entry(name: &str, kind: Kind) -> Entry {
    Entry {
        name: name.into(),
        kind,
        size: (kind == Kind::File).then_some(42),
    }
}
fn source() -> Source {
    Source(Rc::new(RefCell::new(BTreeMap::from([
        (
            "/".into(),
            vec![
                entry("docs", Kind::Directory),
                entry("a.rs", Kind::File),
                entry("b.txt", Kind::File),
                entry(".hidden.rs", Kind::File),
            ],
        ),
        ("/docs".into(), vec![entry("nested.rs", Kind::File)]),
    ]))))
}
fn labels() -> Labels {
    Labels {
        location: "Location".into(),
        name: "Filename".into(),
        files: "Files".into(),
        back: "Back".into(),
        forward: "Forward".into(),
        up: "Up".into(),
        refresh: "Go".into(),
        enter: "Enter folder".into(),
        accept: "Choose".into(),
        cancel: "Cancel".into(),
        hidden: "Hidden".into(),
        filter: "Filter".into(),
        name_column: "Name".into(),
        kind_column: "Kind".into(),
        size_column: "Size".into(),
        file_kind: "File".into(),
        directory_kind: "Folder".into(),
        device_kind: "Device".into(),
        replace_title: "Replace?".into(),
        replace_message: "Replace existing file?".into(),
        replace_accept: "Replace".into(),
        invalid_path: "Invalid path".into(),
        invalid_name: "Invalid name".into(),
        missing_file: "Missing file".into(),
        not_directory: "Not a directory".into(),
        invalid_entries: "Invalid entries".into(),
    }
}
fn options(mode: Mode) -> Options {
    Options {
        key: "picker".into(),
        title: "Choose file".into(),
        directory: "/".into(),
        filename: String::new(),
        mode,
        filters: vec![
            Filter::new("rs", "Rust", vec!["rs".into()]),
            Filter::new("all", "All", vec![]),
        ],
        labels: labels(),
    }
}
fn dialog(mode: Mode) -> FileDialog<Source> {
    FileDialog::new(source(), options(mode)).unwrap()
}
#[test]
fn filters_keep_directories_and_hidden_toggle_is_explicit() {
    let mut d = dialog(Mode::Open);
    assert_eq!(
        d.entries()
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        vec!["docs", "a.rs"]
    );
    d.update(M::Hidden);
    assert_eq!(d.entries().len(), 3);
    d.update(M::Filter("all".into()));
    assert_eq!(d.entries().len(), 4);
    d.update(M::Filter("missing".into()));
    assert_eq!(d.entries().len(), 4);
}
#[test]
fn location_navigation_history_and_errors_preserve_previous_listing() {
    let mut d = dialog(Mode::Open);
    d.update(M::Location("/docs".into()));
    d.update(M::Refresh);
    assert_eq!(d.directory(), "/docs");
    assert_eq!(d.entries()[0].name, "nested.rs");
    d.update(M::Back);
    assert_eq!(d.directory(), "/");
    d.update(M::Forward);
    assert_eq!(d.directory(), "/docs");
    d.update(M::Location("/missing".into()));
    d.update(M::Refresh);
    assert!(d.error().is_some());
    assert_eq!(d.directory(), "/docs");
    assert_eq!(d.entries()[0].name, "nested.rs");
    d.update(M::Up);
    assert_eq!(d.directory(), "/");
}
#[test]
fn open_validates_actual_metadata_and_rejects_missing_or_device() {
    let mut d = dialog(Mode::Open);
    d.update(M::Name("not-found.rs".into()));
    d.update(M::Accept);
    assert!(d.outcome().is_none() && d.error().is_some());
    d.update(M::Name("a.rs".into()));
    d.update(M::Accept);
    assert_eq!(d.outcome(), Some(&Outcome::Accepted("/a.rs".into())));
}
#[test]
fn stale_listing_cannot_accept_removed_file() {
    let s = source();
    let mut d = FileDialog::new(s.clone(), options(Mode::Open)).unwrap();
    d.update(M::Name("a.rs".into()));
    s.0.borrow_mut()
        .get_mut("/")
        .unwrap()
        .retain(|e| e.name != "a.rs");
    d.update(M::Accept);
    assert!(d.outcome().is_none());
    assert_eq!(d.error(), Some("Missing file"));
}
#[test]
fn save_requires_overwrite_confirmation_and_does_not_write() {
    let mut d = dialog(Mode::Save);
    d.update(M::Name("a.rs".into()));
    d.update(M::Accept);
    assert!(d.outcome().is_none());
    assert_eq!(d.view().key(), "picker/replace");
    d.update(M::Decline);
    assert_eq!(d.view().key(), "picker");
    d.update(M::Accept);
    d.update(M::Replace);
    assert_eq!(d.outcome(), Some(&Outcome::Accepted("/a.rs".into())));
    assert_eq!(d.provider().0.borrow()["/"].len(), 4);
}
#[test]
fn overwrite_confirmation_is_bound_to_the_original_path() {
    let mut d = dialog(Mode::Save);
    d.update(M::Name("a.rs".into()));
    d.update(M::Accept);
    d.update(M::Name("different.rs".into()));
    d.update(M::Replace);
    assert_eq!(d.outcome(), Some(&Outcome::Accepted("/a.rs".into())));
}
#[test]
fn save_new_name_and_directory_modes_validate_parent_and_kind() {
    let mut d = dialog(Mode::Save);
    for name in ["", "..", "a/b", "x\0y"] {
        d.update(M::Name(name.into()));
        d.update(M::Accept);
        assert!(d.outcome().is_none());
    }
    d.update(M::Name("new.rs".into()));
    d.update(M::Accept);
    assert_eq!(d.outcome(), Some(&Outcome::Accepted("/new.rs".into())));
    let mut d = dialog(Mode::Directory);
    d.update(M::Accept);
    assert_eq!(d.outcome(), Some(&Outcome::Accepted("/".into())));
    let mut d = dialog(Mode::Directory);
    d.update(M::Name("a.rs".into()));
    d.update(M::Accept);
    assert_eq!(d.error(), Some("Not a directory"));
}
#[test]
fn selected_directory_opens_and_table_sort_keeps_selection() {
    let mut d = dialog(Mode::Open);
    let mut state = TableState::default();
    state.selection.current = Some("docs".into());
    state.selection.selected.insert("docs".into());
    d.update(M::Table(state));
    d.update(M::Accept);
    assert_eq!(d.directory(), "/docs");
    assert!(d.outcome().is_none());
}
#[test]
fn invalid_provider_names_and_duplicate_entries_fail_without_partial_apply() {
    let s = source();
    s.0.borrow_mut()
        .insert("/bad".into(), vec![entry("../escape", Kind::File)]);
    let mut d = FileDialog::new(s.clone(), options(Mode::Open)).unwrap();
    d.update(M::Location("/bad".into()));
    d.update(M::Refresh);
    assert_eq!(d.error(), Some("Invalid entries"));
    assert_eq!(d.directory(), "/");
    s.0.borrow_mut().insert(
        "/bad".into(),
        vec![entry("x", Kind::File), entry("x", Kind::File)],
    );
    d.update(M::Refresh);
    assert_eq!(d.error(), Some("Invalid entries"));
}
#[derive(Clone)]
enum ParentMessage {
    Open,
    Picker(M),
}
struct Parent {
    picker: Option<FileDialog<Source>>,
    result: Option<Outcome>,
}
impl Component for Parent {
    type Message = ParentMessage;
    fn update(&mut self, m: ParentMessage) {
        match m {
            ParentMessage::Open => self.picker = Some(dialog(Mode::Open)),
            ParentMessage::Picker(m) => {
                let d = self.picker.as_mut().unwrap();
                d.update(m);
                if d.outcome().is_some() {
                    self.result = d.outcome().cloned();
                    self.picker = None;
                }
            }
        }
    }
    fn view(&self) -> View<ParentMessage> {
        View::modal(
            "host",
            View::button("open", "Choose", ParentMessage::Open),
            self.picker
                .as_ref()
                .map(|d| d.view().map_messages(ParentMessage::Picker)),
        )
    }
}
#[test]
fn reusable_dialog_maps_all_callbacks_into_parent_and_restores_opener() {
    let mut a = Application::new(Parent {
        picker: None,
        result: None,
    })
    .unwrap();
    a.key(Key::plain('\t')).unwrap();
    a.key(Key::plain(' ')).unwrap();
    assert_eq!(a.active_dialog(), Some("picker"));
    a.update(ParentMessage::Picker(M::Name("a.rs".into())))
        .unwrap();
    a.key(Key::plain('\r')).unwrap();
    assert_eq!(
        a.component().result,
        Some(Outcome::Accepted("/a.rs".into()))
    );
    assert_eq!(a.focused_key(), Some("open"));
}
#[test]
fn location_input_submit_navigates_instead_of_accepting_the_dialog() {
    let mut a = Application::new(dialog(Mode::Open)).unwrap();
    a.pointer(dunit_component::PointerEvent::Down, Some("picker/location"))
        .unwrap();
    a.edit(dunit_component::EditCommand::SelectAll).unwrap();
    for c in "/docs".chars() {
        a.key(Key::plain(c)).unwrap();
    }
    a.key(Key::plain('\r')).unwrap();
    assert_eq!(a.component().directory(), "/docs");
    assert!(a.component().outcome().is_none());
}
#[test]
fn dialog_layout_is_bounded_and_rows_are_virtualized_in_both_themes() {
    for profile in [
        include_str!("../../../assets/dwm/sdk/dark.dss"),
        include_str!("../../../assets/dwm/sdk/light.dss"),
    ] {
        let t = Theme::parse(
            profile,
            include_str!("../../../assets/dwm/sdk/base.dss"),
            "",
        )
        .unwrap();
        let mut r = Renderer::new(
            Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
            t.cascade,
            t.background,
        );
        let d = dialog(Mode::Open);
        for (w, h) in [(640, 500), (220, 160), (640, 500)] {
            r.render(&d.view(), w, h).unwrap();
            assert!(r.list_visible_rows("picker/files").unwrap() <= 2);
            for key in ["picker/accept", "picker/cancel"] {
                let rect = r.widget_rect(key).unwrap();
                assert!(
                    rect.x >= 0.
                        && rect.y >= 0.
                        && rect.x + rect.w <= w as f32
                        && rect.y + rect.h <= h as f32
                );
                assert_eq!(
                    r.hit_key(rect.x + rect.w / 2., rect.y + rect.h / 2.),
                    Some(key)
                );
            }
        }
    }
}

#[test]
fn failed_folder_accept_and_history_navigation_preserve_filename_draft() {
    let mut d = dialog(Mode::Open);
    d.provider().0.borrow_mut().remove("/docs");
    d.update(M::Name("docs".into()));
    d.update(M::Accept);
    assert_eq!(d.directory(), "/");
    assert_eq!(d.filename(), "docs");
    assert_eq!(d.error(), Some("cannot read /docs"));
    let mut d = dialog(Mode::Save);
    d.update(M::Location("/docs".into()));
    d.update(M::Refresh);
    d.update(M::Back);
    d.provider().0.borrow_mut().remove("/docs");
    d.update(M::Name("draft.rs".into()));
    d.update(M::Forward);
    assert_eq!(d.directory(), "/");
    assert_eq!(d.filename(), "draft.rs");
    assert_eq!(d.error(), Some("cannot read /docs"));
}
