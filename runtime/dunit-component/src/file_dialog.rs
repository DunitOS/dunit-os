//! A composable file dialog over the same browser data used by gui_files.
use crate::{
    Component, DialogAction, InputOptions, SelectOption, SelectionMode, TableColumn, TableModel,
    TableRow, TableState, TableValue, View,
};
use alloc::{format, string::String, vec, vec::Vec};
use dunit_widgets::file_browser::{child_path, normalize, parent, valid_name};
pub use dunit_widgets::file_browser::{FileEntry, FileFilter, FileKind};
pub trait DirectorySource {
    fn read_directory(&mut self, path: &str) -> core::result::Result<Vec<FileEntry>, String>;
    fn metadata(&mut self, path: &str) -> core::result::Result<Option<FileKind>, String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialogMode {
    Open,
    Save,
    Directory,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDialogLabels {
    pub location: String,
    pub name: String,
    pub files: String,
    pub back: String,
    pub forward: String,
    pub up: String,
    pub refresh: String,
    pub enter: String,
    pub accept: String,
    pub cancel: String,
    pub hidden: String,
    pub filter: String,
    pub name_column: String,
    pub kind_column: String,
    pub size_column: String,
    pub file_kind: String,
    pub directory_kind: String,
    pub device_kind: String,
    pub replace_title: String,
    pub replace_message: String,
    pub replace_accept: String,
    pub invalid_path: String,
    pub invalid_name: String,
    pub missing_file: String,
    pub not_directory: String,
    pub invalid_entries: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDialogOptions {
    pub key: String,
    pub title: String,
    pub directory: String,
    pub filename: String,
    pub mode: FileDialogMode,
    pub filters: Vec<FileFilter>,
    pub labels: FileDialogLabels,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDialogOutcome {
    Accepted(String),
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDialogMessage {
    Location(String),
    Name(String),
    Filter(String),
    Hidden,
    Table(TableState),
    Back,
    Forward,
    Up,
    Refresh,
    Enter,
    Accept,
    Cancel,
    Replace,
    Decline,
}
pub struct FileDialog<P: DirectorySource> {
    provider: P,
    options: FileDialogOptions,
    directory: String,
    location: String,
    name: String,
    entries: Vec<FileEntry>,
    filtered: Vec<FileEntry>,
    table: TableState,
    filter: Option<String>,
    hidden: bool,
    history: Vec<String>,
    cursor: usize,
    error: Option<String>,
    overwrite: Option<String>,
    outcome: Option<FileDialogOutcome>,
}
impl<P: DirectorySource> FileDialog<P> {
    pub fn new(provider: P, options: FileDialogOptions) -> core::result::Result<Self, String> {
        let directory = normalize(&options.directory, "/")
            .ok_or_else(|| options.labels.invalid_path.clone())?;
        if options.key.is_empty()
            || options.title.is_empty()
            || !options.directory.starts_with('/')
            || !options.filters.iter().all(|f| !f.key.is_empty())
        {
            return Err(options.labels.invalid_path.clone());
        }
        let mut keys = alloc::collections::BTreeSet::new();
        if options.filters.iter().any(|f| !keys.insert(&f.key)) {
            return Err(options.labels.invalid_path.clone());
        }
        let name = options.filename.clone();
        let filter = options.filters.first().map(|f| f.key.clone());
        let mut dialog = Self {
            provider,
            options,
            directory: directory.clone(),
            location: directory.clone(),
            name,
            entries: Vec::new(),
            filtered: Vec::new(),
            table: TableState::default(),
            filter,
            hidden: false,
            history: vec![directory],
            cursor: 0,
            error: None,
            overwrite: None,
            outcome: None,
        };
        let path = dialog.directory.clone();
        dialog.load(&path, false, None);
        Ok(dialog)
    }
    pub fn outcome(&self) -> Option<&FileDialogOutcome> {
        self.outcome.as_ref()
    }
    pub fn directory(&self) -> &str {
        &self.directory
    }
    pub fn entries(&self) -> &[FileEntry] {
        &self.filtered
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn filename(&self) -> &str {
        &self.name
    }
    pub fn table_state(&self) -> &TableState {
        &self.table
    }
    pub fn provider(&self) -> &P {
        &self.provider
    }
    fn filter_entries(&mut self) {
        let filter = self
            .filter
            .as_ref()
            .and_then(|key| self.options.filters.iter().find(|f| &f.key == key));
        self.filtered = self
            .entries
            .iter()
            .filter(|e| {
                (self.hidden || !e.name.starts_with('.')) && filter.is_none_or(|f| f.matches(e))
            })
            .cloned()
            .collect();
        self.filtered.sort_by(|a, b| {
            (a.kind != FileKind::Directory)
                .cmp(&(b.kind != FileKind::Directory))
                .then_with(|| a.name.cmp(&b.name))
        });
        self.table.retain(&self.model());
    }
    fn model(&self) -> TableModel {
        let l = &self.options.labels;
        TableModel::new(
            vec![
                TableColumn::new("name", &l.name_column),
                TableColumn::new("kind", &l.kind_column),
                TableColumn::new("size", &l.size_column),
            ],
            self.filtered
                .iter()
                .map(|e| {
                    TableRow::new(
                        &e.name,
                        vec![
                            TableValue::Text(e.name.clone()),
                            TableValue::Text(match e.kind {
                                FileKind::File => l.file_kind.clone(),
                                FileKind::Directory => l.directory_kind.clone(),
                                FileKind::Device => l.device_kind.clone(),
                            }),
                            e.size.map_or_else(
                                || TableValue::Text(String::new()),
                                TableValue::Unsigned,
                            ),
                        ],
                    )
                    .disabled(e.kind == FileKind::Device)
                })
                .collect(),
        )
        .expect("validated directory entries")
    }
    fn load(&mut self, path: &str, push: bool, cursor: Option<usize>) -> bool {
        let Some(path) = normalize(path, &self.directory) else {
            self.error = Some(self.options.labels.invalid_path.clone());
            return false;
        };
        match self.provider.read_directory(&path) {
            Err(e) => {
                self.error = Some(e);
                false
            }
            Ok(entries) => {
                let mut names = alloc::collections::BTreeSet::new();
                if entries
                    .iter()
                    .any(|e| !valid_name(&e.name) || !names.insert(&e.name))
                {
                    self.error = Some(self.options.labels.invalid_entries.clone());
                    return false;
                }
                self.directory = path.clone();
                self.location = path.clone();
                self.entries = entries;
                self.table = TableState::default();
                self.error = None;
                self.filter_entries();
                if push && self.history.get(self.cursor) != Some(&path) {
                    self.history.truncate(self.cursor + 1);
                    self.history.push(path);
                    self.cursor = self.history.len() - 1;
                } else if let Some(cursor) = cursor {
                    self.cursor = cursor;
                }
                true
            }
        }
    }
    fn selected(&self) -> Option<&FileEntry> {
        let key = self.table.selection.current.as_deref()?;
        self.filtered.iter().find(|e| e.name == key)
    }
    fn accept(&mut self) {
        let mode = self.options.mode;
        let selected = self.selected().cloned();
        let candidate = if mode == FileDialogMode::Directory && self.name.is_empty() {
            selected
                .filter(|e| e.kind == FileKind::Directory)
                .map_or_else(
                    || self.directory.clone(),
                    |e| child_path(&self.directory, &e.name),
                )
        } else {
            if !valid_name(&self.name) {
                self.error = Some(self.options.labels.invalid_name.clone());
                return;
            }
            child_path(&self.directory, &self.name)
        };
        match self.provider.metadata(&candidate) {
            Err(e) => self.error = Some(e),
            Ok(kind) => match (mode, kind) {
                (FileDialogMode::Directory, Some(FileKind::Directory)) => {
                    self.outcome = Some(FileDialogOutcome::Accepted(candidate))
                }
                (FileDialogMode::Directory, _) => {
                    self.error = Some(self.options.labels.not_directory.clone())
                }
                (_, Some(FileKind::Directory)) => {
                    if self.load(&candidate, true, None) {
                        self.name.clear();
                    }
                }
                (FileDialogMode::Open, Some(FileKind::File)) => {
                    self.outcome = Some(FileDialogOutcome::Accepted(candidate))
                }
                (FileDialogMode::Open, _) => {
                    self.error = Some(self.options.labels.missing_file.clone())
                }
                (FileDialogMode::Save, Some(FileKind::File)) => self.overwrite = Some(candidate),
                (FileDialogMode::Save, None) => match self.provider.metadata(&self.directory) {
                    Ok(Some(FileKind::Directory)) => {
                        self.outcome = Some(FileDialogOutcome::Accepted(candidate))
                    }
                    Ok(_) => self.error = Some(self.options.labels.not_directory.clone()),
                    Err(e) => self.error = Some(e),
                },
                (FileDialogMode::Save, Some(FileKind::Device)) => {
                    self.error = Some(self.options.labels.invalid_name.clone())
                }
            },
        }
    }
}
impl<P: DirectorySource> Component for FileDialog<P> {
    type Message = FileDialogMessage;
    fn update(&mut self, m: Self::Message) {
        if self.outcome.is_some() {
            return;
        }
        match m {
            FileDialogMessage::Location(s) => self.location = s,
            FileDialogMessage::Name(s) => {
                self.name = s;
                self.error = None;
            }
            FileDialogMessage::Filter(key) => {
                if self.options.filters.iter().any(|f| f.key == key) {
                    self.filter = Some(key);
                    self.filter_entries();
                }
            }
            FileDialogMessage::Hidden => {
                self.hidden = !self.hidden;
                self.filter_entries();
            }
            FileDialogMessage::Table(state) => {
                self.table = state;
                if let Some(e) = self.selected() {
                    if self.options.mode != FileDialogMode::Directory
                        || e.kind == FileKind::Directory
                    {
                        self.name = e.name.clone();
                    }
                }
            }
            FileDialogMessage::Back => {
                if self.cursor > 0 {
                    let i = self.cursor - 1;
                    let path = self.history[i].clone();
                    if self.load(&path, false, Some(i)) {
                        self.name.clear();
                    }
                }
            }
            FileDialogMessage::Forward => {
                if self.cursor + 1 < self.history.len() {
                    let i = self.cursor + 1;
                    let path = self.history[i].clone();
                    if self.load(&path, false, Some(i)) {
                        self.name.clear();
                    }
                }
            }
            FileDialogMessage::Up => {
                let path = parent(&self.directory);
                if self.load(&path, true, None) {
                    self.name.clear();
                }
            }
            FileDialogMessage::Refresh => {
                let path = self.location.clone();
                self.load(&path, true, None);
            }
            FileDialogMessage::Enter => {
                if let Some(e) = self.selected().filter(|e| e.kind == FileKind::Directory) {
                    let path = child_path(&self.directory, &e.name);
                    if self.load(&path, true, None) {
                        self.name.clear();
                    }
                }
            }
            FileDialogMessage::Accept => self.accept(),
            FileDialogMessage::Cancel => self.outcome = Some(FileDialogOutcome::Cancelled),
            FileDialogMessage::Replace => {
                if let Some(path) = self.overwrite.take() {
                    match self.provider.metadata(&path) {
                        Ok(None | Some(FileKind::File)) => match self
                            .provider
                            .metadata(&parent(&path))
                        {
                            Ok(Some(FileKind::Directory)) => {
                                self.outcome = Some(FileDialogOutcome::Accepted(path))
                            }
                            Ok(_) => self.error = Some(self.options.labels.not_directory.clone()),
                            Err(e) => self.error = Some(e),
                        },
                        Ok(_) => self.error = Some(self.options.labels.invalid_name.clone()),
                        Err(e) => self.error = Some(e),
                    }
                }
            }
            FileDialogMessage::Decline => self.overwrite = None,
        }
    }
    fn view(&self) -> View<Self::Message> {
        let key = &self.options.key;
        let l = &self.options.labels;
        let id = |suffix: &str| format!("{key}/{suffix}");
        if self.overwrite.is_some() {
            return View::message_box(
                id("replace"),
                &l.replace_title,
                &l.replace_message,
                vec![
                    DialogAction::new(
                        id("replace-accept"),
                        &l.replace_accept,
                        FileDialogMessage::Replace,
                    ),
                    DialogAction::new(id("replace-cancel"), &l.cancel, FileDialogMessage::Decline),
                ],
                Some(&id("replace-accept")),
                Some(FileDialogMessage::Decline),
            );
        }
        let mut navigation = vec![
            View::button(id("back"), &l.back, FileDialogMessage::Back).disabled(self.cursor == 0),
            View::button(id("forward"), &l.forward, FileDialogMessage::Forward)
                .disabled(self.cursor + 1 >= self.history.len()),
            View::button(id("up"), &l.up, FileDialogMessage::Up).disabled(self.directory == "/"),
            View::button(id("enter"), &l.enter, FileDialogMessage::Enter).disabled(
                !self
                    .selected()
                    .is_some_and(|e| e.kind == FileKind::Directory),
            ),
        ];
        navigation.push(View::button(
            id("refresh"),
            &l.refresh,
            FileDialogMessage::Refresh,
        ));
        let filter = if self.options.filters.is_empty() {
            View::text(id("filter"), &l.filter)
        } else {
            View::select(
                id("filter"),
                &l.filter,
                self.filter.as_deref(),
                self.options
                    .filters
                    .iter()
                    .map(|f| SelectOption::new(&f.key, &f.label))
                    .collect(),
                FileDialogMessage::Filter,
            )
        };
        let body = View::column(
            id("body"),
            vec![
                View::toolbar(id("navigation"), &l.files, navigation),
                View::input(
                    id("location"),
                    &l.location,
                    &self.location,
                    FileDialogMessage::Location,
                    InputOptions::default(),
                )
                .on_submit(FileDialogMessage::Refresh)
                .class("file-picker-location"),
                View::row(
                    id("filters"),
                    vec![
                        filter,
                        View::checkbox(
                            id("hidden"),
                            &l.hidden,
                            self.hidden,
                            FileDialogMessage::Hidden,
                        ),
                    ],
                ),
                View::table(
                    id("files"),
                    &l.files,
                    self.model(),
                    self.table.clone(),
                    SelectionMode::Single,
                    FileDialogMessage::Table,
                )
                .class("file-picker-files"),
                View::input(
                    id("name"),
                    &l.name,
                    &self.name,
                    FileDialogMessage::Name,
                    InputOptions::default(),
                )
                .class("file-picker-name"),
                View::status_bar(id("error"), &l.files, self.error.as_deref().unwrap_or("")),
            ],
        )
        .class("file-picker-body");
        View::dialog(
            key,
            &self.options.title,
            body,
            vec![
                DialogAction::new(id("accept"), &l.accept, FileDialogMessage::Accept),
                DialogAction::new(id("cancel"), &l.cancel, FileDialogMessage::Cancel),
            ],
            Some(&id("accept")),
            Some(FileDialogMessage::Cancel),
        )
        .class("file-picker-dialog")
    }
}
