//! Browser state and filesystem authority, separate from painting and IPC.
use super::*;
use alloc::collections::BTreeMap;
use dunit::transfer::{Error as TransferError, File, FileKind, NativeResult, OwnedHandle, Payload};
use dunit_widgets::selection::{ListItem, ListSelection, SelectionMode};
pub(super) struct Item {
    pub name: String,
    pub kind: IconKind,
}
impl Item {
    pub fn is_dir(&self) -> bool {
        self.kind == IconKind::Folder
    }
}
struct Location {
    path: String,
    authority: Option<File>,
    restricted: bool,
    received: BTreeMap<String, File>,
}
pub(super) struct Preview {
    pub path: String,
    pub text: String,
    pub row: usize,
    _file: File,
}
pub(super) struct Files {
    pub generation: u64,
    pub path: String,
    pub items: Vec<Item>,
    pub selection: ListSelection,
    pub total: usize,
    pub w: i32,
    pub h: i32,
    pub pal: Palette,
    pub clear_bg: Color,
    pub status: String,
    pub scroll_row: usize,
    pub preview: Option<Preview>,
    received: BTreeMap<String, File>,
    authority: Option<File>,
    restricted: bool,
    parents: Vec<Location>,
    held: BTreeMap<String, File>,
}
fn checked(n: isize) -> NativeResult<usize> {
    if n < 0 {
        Err(TransferError(n))
    } else {
        Ok(n as usize)
    }
}
fn duplicate(file: &File, write: bool) -> NativeResult<File> {
    let rights = libdunit::RIGHT_READ
        | libdunit::RIGHT_TRANSFER
        | if write { libdunit::RIGHT_WRITE } else { 0 };
    Ok(File {
        handle: file.handle.duplicate(rights)?,
        path: file.path.clone(),
        kind: file.kind,
    })
}
impl Files {
    pub fn new(pal: Palette, clear_bg: Color) -> Self {
        let mut files = Self {
            generation: 0,
            path: "/".into(),
            items: Vec::new(),
            selection: ListSelection::default(),
            total: 0,
            w: W as i32,
            h: H as i32,
            pal,
            clear_bg,
            status: String::new(),
            scroll_row: 0,
            preview: None,
            received: BTreeMap::new(),
            authority: None,
            restricted: false,
            parents: Vec::new(),
            held: BTreeMap::new(),
        };
        files.reload();
        files
    }
    pub fn cols(&self) -> i32 {
        ((self.w - 2 * PAD) / CELL_W).max(1)
    }
    pub fn rows(&self) -> i32 {
        ((self.h - HEADER_H - STATUS_H) / CELL_H).max(1)
    }
    pub fn cap(&self) -> usize {
        (self.cols() * self.rows()) as usize
    }
    pub fn has_parent(&self) -> bool {
        if self.restricted {
            !self.parents.is_empty()
        } else {
            self.path != "/"
        }
    }
    pub fn current_index(&self) -> Option<usize> {
        self.selection
            .current
            .as_ref()
            .and_then(|key| self.items.iter().position(|item| &item.name == key))
    }
    pub fn single_index(&self) -> Option<usize> {
        if self.selection.selected.len() == 1 {
            self.selection
                .selected
                .iter()
                .next()
                .and_then(|key| self.items.iter().position(|item| &item.name == key))
        } else {
            None
        }
    }
    fn choices(&self) -> Vec<ListItem> {
        self.items
            .iter()
            .map(|item| ListItem::new(&item.name, &item.name))
            .collect()
    }
    pub fn selected(&self, index: usize) -> bool {
        self.items
            .get(index)
            .is_some_and(|item| self.selection.selected.contains(&item.name))
    }
    pub fn choose(&mut self, index: usize, mods: u8) {
        let ctrl = mods & libdunit::KEYMOD_CTRL != 0;
        let shift = mods & libdunit::KEYMOD_SHIFT != 0;
        self.selection = self.selection.choose(
            &self.choices(),
            SelectionMode::Multiple,
            index,
            shift,
            ctrl,
            ctrl && !shift,
        );
        self.capture_selected();
        self.reveal_current();
    }
    pub fn select_all(&mut self) {
        self.selection = self
            .selection
            .select_all(&self.choices(), SelectionMode::Multiple);
        self.capture_selected();
    }
    pub fn clear_selection(&mut self) {
        self.selection = ListSelection::default();
        self.held.clear();
    }
    fn capture_selected(&mut self) {
        self.held
            .retain(|name, _| self.selection.selected.contains(name));
        let names: Vec<_> = self
            .selection
            .selected
            .iter()
            .filter(|name| !self.held.contains_key(*name))
            .cloned()
            .collect();
        for name in names {
            match self.open_file(&name, false) {
                Ok(file) => {
                    self.held.insert(name, file);
                }
                Err(error) => self.error("File authority", error),
            }
        }
    }
    pub fn can_write(&self) -> bool {
        if self.preview.is_some() {
            return false;
        }
        self.authority
            .as_ref()
            .map_or(!self.restricted && self.path == "/", |f| {
                f.handle
                    .rights()
                    .is_ok_and(|r| r & libdunit::RIGHT_WRITE != 0)
            })
    }
    pub fn can_export(&self, cut: bool) -> bool {
        !self.selection.selected.is_empty()
            && self.held.len() == self.selection.selected.len()
            && (!cut || self.can_write())
    }
    pub fn child_path(&self, name: &str) -> String {
        dunit_widgets::file_browser::child_path(&self.path, name)
    }
    fn open_file(&self, name: &str, write: bool) -> NativeResult<File> {
        if let Some(parent) = &self.authority {
            return parent.child_with_rights(name, write);
        }
        if self.restricted {
            return self
                .received
                .get(name)
                .ok_or(TransferError(libdunit::ENOENT))
                .and_then(|file| duplicate(file, write));
        }
        File::open(&self.child_path(name), write)
    }
    pub fn directory(&self) -> NativeResult<OwnedHandle> {
        if let Some(file) = &self.authority {
            return file.handle.duplicate(
                libdunit::RIGHT_READ | libdunit::RIGHT_WRITE | libdunit::RIGHT_TRANSFER,
            );
        }
        if self.restricted {
            return Err(TransferError(libdunit::EACCES));
        }
        Ok(File::open(&self.path, true)?.handle)
    }
    pub fn destination(&self, index: Option<usize>) -> NativeResult<OwnedHandle> {
        if let Some(index) = index.filter(|i| self.items.get(*i).is_some_and(Item::is_dir)) {
            return Ok(self.open_file(&self.items[index].name, true)?.handle);
        }
        self.directory()
    }
    pub fn payload(&self, cut: bool) -> NativeResult<Payload> {
        if self.selection.selected.is_empty() {
            return Err(TransferError(libdunit::EINVAL));
        }
        let mut files = Vec::new();
        for item in &self.items {
            if !self.selection.selected.contains(&item.name) {
                continue;
            }
            let held = self
                .held
                .get(&item.name)
                .ok_or(TransferError(libdunit::EOPNOTSUPP))?;
            let file = if cut {
                let writable = self.open_file(&item.name, true)?;
                if checked(libdunit::file_cap::relation(
                    held.handle.raw(),
                    writable.handle.raw(),
                ))? != libdunit::file_cap::Relation::Same as usize
                {
                    return Err(TransferError(libdunit::EAGAIN));
                }
                writable
            } else {
                duplicate(held, false)?
            };
            files.push(file);
        }
        Payload::from_files(files)
    }
    pub fn error(&mut self, operation: &str, error: TransferError) {
        self.status = alloc::format!("{}: {}", operation, libdunit::error_name(error.0));
        libdunit::println(&alloc::format!("gui_files: {}", self.status));
    }
    fn listing(
        path: &str,
        authority: Option<&File>,
        received: &BTreeMap<String, File>,
    ) -> NativeResult<Vec<Item>> {
        if !received.is_empty() {
            return Ok(received
                .iter()
                .map(|(name, file)| Item {
                    name: name.clone(),
                    kind: classify(name, file.kind == FileKind::Directory, false),
                })
                .collect());
        }
        let entries = if let Some(file) = authority {
            file.entries()?
        } else {
            // Legacy virtual/mechanism directories cannot provide File caps.
            // Grow until completeness is known; never pretend 1024 is a total.
            let mut capacity = 64;
            loop {
                let mut entries = alloc::vec![libdunit::DirEntry::empty();capacity];
                let n = checked(libdunit::readdir(path, &mut entries))?;
                if n < capacity {
                    entries.truncate(n);
                    break entries;
                }
                if capacity == 1024 {
                    return Err(TransferError(libdunit::ENOBUFS));
                }
                capacity *= 2;
            }
        };
        let mut items: Vec<_> = entries
            .into_iter()
            .map(|e| Item {
                name: String::from(e.name()),
                kind: classify(
                    e.name(),
                    e.file_type == libdunit::FILE_TYPE_DIRECTORY,
                    path == "/app",
                ),
            })
            .collect();
        items.sort_by(|a, b| {
            b.is_dir()
                .cmp(&a.is_dir())
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(items)
    }
    pub fn reload(&mut self) {
        match Self::listing(&self.path, self.authority.as_ref(), &self.received) {
            Ok(items) => {
                self.items = items;
                self.total = self.items.len();
                self.selection.retain(&self.choices());
                // A same-name replacement must not silently retarget a selection.
                let stale: Vec<_> = self
                    .selection
                    .selected
                    .iter()
                    .filter(|name| {
                        self.held.get(*name).is_some_and(|old| {
                            self.open_file(name, false).map_or(true, |new| {
                                libdunit::file_cap::relation(old.handle.raw(), new.handle.raw())
                                    != libdunit::file_cap::Relation::Same as isize
                            })
                        })
                    })
                    .cloned()
                    .collect();
                for name in stale {
                    self.selection.selected.remove(&name);
                    self.held.remove(&name);
                }
                self.held
                    .retain(|name, _| self.selection.selected.contains(name));
                self.clamp_scroll();
            }
            Err(error) => self.error("Read directory", error),
        }
    }
    fn enter(&mut self, path: String, authority: Option<File>, restricted: bool) {
        match Self::listing(&path, authority.as_ref(), &BTreeMap::new()) {
            Ok(items) => {
                self.generation = self.generation.wrapping_add(1);
                self.parents.push(Location {
                    path: core::mem::replace(&mut self.path, path),
                    authority: self.authority.take(),
                    restricted: self.restricted,
                    received: core::mem::take(&mut self.received),
                });
                self.authority = authority;
                self.restricted = restricted;
                self.items = items;
                self.total = self.items.len();
                self.clear_selection();
                self.scroll_row = 0;
                self.status.clear();
            }
            Err(error) => self.error("Open directory", error),
        }
    }
    pub fn open(&mut self, index: usize) {
        let Some(item) = self.items.get(index) else {
            return;
        };
        if !item.is_dir() {
            let result = self
                .open_file(&item.name, false)
                .and_then(|file| Self::preview_file(file));
            match result {
                Ok(preview) => self.preview = Some(preview),
                Err(error) => self.error("Open file", error),
            }
            return;
        }
        let name = item.name.clone();
        let path = self.child_path(&name);
        let authority = match self.open_file(&name, true) {
            Ok(file) => Some(file),
            Err(error) if matches!(error.0, libdunit::EPERM | libdunit::EACCES) => {
                match self.open_file(&name, false) {
                    Ok(file) => Some(file),
                    Err(error) => {
                        self.error("Open directory", error);
                        return;
                    }
                }
            }
            Err(error) if error.0 == libdunit::EOPNOTSUPP && !self.restricted => None,
            Err(error) => {
                self.error("Open directory", error);
                return;
            }
        };
        let path = authority.as_ref().map_or(path, |file| file.path.clone());
        self.enter(path, authority, self.restricted);
    }
    pub fn open_capability(&mut self, file: File) -> NativeResult<()> {
        if file.kind != FileKind::Directory {
            return Err(TransferError(libdunit::ENOTDIR));
        }
        let items = Self::listing(&file.path, Some(&file), &BTreeMap::new())?;
        self.generation = self.generation.wrapping_add(1);
        self.path = file.path.clone();
        self.received.clear();
        self.preview = None;
        self.authority = Some(file);
        self.restricted = true;
        self.parents.clear();
        self.items = items;
        self.total = self.items.len();
        self.clear_selection();
        self.scroll_row = 0;
        Ok(())
    }
    fn preview_file(file: File) -> NativeResult<Preview> {
        let bytes = file.read_all(64 * 1024)?;
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                let mut text = String::new();
                for chunk in error.as_bytes().chunks(16) {
                    for byte in chunk {
                        use core::fmt::Write;
                        let _ = write!(text, "{:02x} ", byte);
                    }
                    text.push('\n');
                }
                text
            }
        };
        Ok(Preview {
            path: file.path.clone(),
            text,
            row: 0,
            _file: file,
        })
    }
    pub fn open_initial(&mut self, file: File) -> NativeResult<()> {
        if file.kind == FileKind::Directory {
            let path = file.path.clone();
            let items = Self::listing(&path, Some(&file), &BTreeMap::new())?;
            self.path = path;
            self.authority = Some(file);
            self.restricted = false;
            self.received.clear();
            self.parents.clear();
            self.items = items;
            self.total = self.items.len();
            self.clear_selection();
            self.scroll_row = 0;
            self.generation = self.generation.wrapping_add(1);
            Ok(())
        } else {
            self.open_received(alloc::vec![file])
        }
    }
    pub fn open_received(&mut self, mut incoming: Vec<File>) -> NativeResult<()> {
        if incoming.is_empty() {
            return Err(TransferError(libdunit::EINVAL));
        }
        if incoming.len() == 1 {
            let file = incoming.pop().unwrap();
            if file.kind == FileKind::Directory {
                return self.open_capability(file);
            }
            let preview = Self::preview_file(file)?;
            self.preview = Some(preview);
            self.generation = self.generation.wrapping_add(1);
            return Ok(());
        }
        let mut received = BTreeMap::new();
        for file in incoming {
            if received.insert(file.path.clone(), file).is_some() {
                return Err(TransferError(libdunit::EINVAL));
            }
        }
        let items = Self::listing("Received items", None, &received)?;
        self.received = received;
        self.path = "Received items".into();
        self.authority = None;
        self.restricted = true;
        self.parents.clear();
        self.items = items;
        self.total = self.items.len();
        self.clear_selection();
        self.scroll_row = 0;
        self.preview = None;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
    pub fn go_parent(&mut self) {
        if self.parents.is_empty() && !self.restricted && self.path != "/" {
            let path = dunit_widgets::file_browser::parent(&self.path);
            if path == "/" {
                let items = Self::listing("/", None, &BTreeMap::new());
                match items {
                    Ok(items) => {
                        self.path = "/".into();
                        self.authority = None;
                        self.items = items;
                        self.total = self.items.len();
                        self.clear_selection();
                        self.generation = self.generation.wrapping_add(1);
                    }
                    Err(error) => self.error("Parent directory", error),
                }
                return;
            }
            match File::open(&path, true).and_then(|file| self.open_initial(file)) {
                Ok(()) => {}
                Err(error) => self.error("Parent directory", error),
            }
            return;
        }
        if let Some(parent) = self.parents.pop() {
            match Self::listing(&parent.path, parent.authority.as_ref(), &parent.received) {
                Ok(items) => {
                    self.generation = self.generation.wrapping_add(1);
                    self.path = parent.path;
                    self.authority = parent.authority;
                    self.restricted = parent.restricted;
                    self.received = parent.received;
                    self.items = items;
                    self.total = self.items.len();
                    self.clear_selection();
                    self.scroll_row = 0;
                }
                Err(error) => {
                    self.parents.push(parent);
                    self.error("Parent directory", error);
                }
            }
        }
    }
    pub fn make_dir(&mut self, name: &str) -> bool {
        let result = if !self.restricted && self.path == "/" {
            checked(libdunit::mkdir(&self.child_path(name))).map(|_| ())
        } else {
            self.directory().and_then(|dir| {
                checked(libdunit::file_cap::create_child(
                    dir.raw(),
                    name,
                    libdunit::FILE_TYPE_DIRECTORY,
                    libdunit::RIGHT_READ | libdunit::RIGHT_WRITE | libdunit::RIGHT_TRANSFER,
                ))
                .map(|h| {
                    libdunit::handle_close(h as u32);
                })
            })
        };
        match result {
            Ok(()) => {
                self.status = "Folder created".into();
                self.reload();
                true
            }
            Err(error) => {
                self.error("Create folder", error);
                false
            }
        }
    }
    pub fn rename_selected(&mut self, name: &str) -> bool {
        let result = self
            .single_index()
            .ok_or(TransferError(libdunit::EINVAL))
            .and_then(|index| self.open_file(&self.items[index].name, true))
            .and_then(|source| {
                let held = self
                    .held
                    .get(
                        &self.items[self.single_index().ok_or(TransferError(libdunit::EINVAL))?]
                            .name,
                    )
                    .ok_or(TransferError(libdunit::EAGAIN))?;
                if libdunit::file_cap::relation(held.handle.raw(), source.handle.raw())
                    != libdunit::file_cap::Relation::Same as isize
                {
                    return Err(TransferError(libdunit::EAGAIN));
                }
                self.directory().and_then(|dir| {
                    checked(libdunit::file_cap::move_to(
                        source.handle.raw(),
                        dir.raw(),
                        name,
                    ))
                    .map(|_| ())
                })
            });
        match result {
            Ok(()) => {
                self.status = "Renamed".into();
                self.reload();
                true
            }
            Err(error) => {
                self.error("Rename", error);
                false
            }
        }
    }
    pub fn delete_selected(&mut self) {
        let names: Vec<_> = self.selection.selected.iter().cloned().collect();
        let mut removed = 0;
        for name in names {
            let result = self.open_file(&name, true).and_then(|file| {
                let old = self
                    .held
                    .get(&name)
                    .ok_or(TransferError(libdunit::EAGAIN))?;
                if libdunit::file_cap::relation(old.handle.raw(), file.handle.raw())
                    != libdunit::file_cap::Relation::Same as isize
                {
                    return Err(TransferError(libdunit::EAGAIN));
                }
                checked(libdunit::file_cap::remove(file.handle.raw())).map(|_| ())
            });
            match result {
                Ok(()) => removed += 1,
                Err(error) => {
                    self.error("Delete", error);
                    self.reload();
                    return;
                }
            }
        }
        self.status = alloc::format!("Deleted {} item(s)", removed);
        self.reload();
    }
    pub fn clamp_scroll(&mut self) {
        let rows =
            (self.items.len() + usize::from(self.has_parent())).div_ceil(self.cols() as usize);
        self.scroll_row = self
            .scroll_row
            .min(rows.saturating_sub(self.rows() as usize));
    }
    pub fn scroll(&mut self, delta: i32) {
        if delta < 0 {
            self.scroll_row = self
                .scroll_row
                .saturating_sub(delta.unsigned_abs() as usize);
        } else {
            self.scroll_row = self.scroll_row.saturating_add(delta as usize);
        }
        self.clamp_scroll();
    }
    pub fn reveal_current(&mut self) {
        if let Some(index) = self.current_index() {
            let row = (index + usize::from(self.has_parent())) / (self.cols() as usize);
            if row < self.scroll_row {
                self.scroll_row = row;
            } else if row >= self.scroll_row + self.rows() as usize {
                self.scroll_row = row + 1 - self.rows() as usize;
            }
        }
        self.clamp_scroll();
    }
}
