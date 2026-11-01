//! Native provider for the reusable SDK file chooser; no desktop special cases.
use super::*;
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeDirectorySource;
impl DirectorySource for NativeDirectorySource {
    fn read_directory(&mut self, path: &str) -> core::result::Result<Vec<FileEntry>, String> {
        crate::fs::read_dir(path)
            .map_err(|e| e.to_string())
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|e| {
                        let kind = match e.file_type {
                            crate::fs::FileType::File => FileKind::File,
                            crate::fs::FileType::Directory => FileKind::Directory,
                            crate::fs::FileType::Device => FileKind::Device,
                        };
                        let size = if kind == FileKind::File {
                            crate::fs::metadata(&file_browser::child_path(path, &e.name))
                                .ok()
                                .map(|m| m.len)
                        } else {
                            None
                        };
                        FileEntry {
                            name: e.name,
                            kind,
                            size,
                        }
                    })
                    .collect()
            })
    }
    fn metadata(&mut self, path: &str) -> core::result::Result<Option<FileKind>, String> {
        match crate::fs::metadata(path) {
            Ok(m) => Ok(Some(match m.file_type {
                crate::fs::FileType::File => FileKind::File,
                crate::fs::FileType::Directory => FileKind::Directory,
                crate::fs::FileType::Device => FileKind::Device,
            })),
            Err(e) if e.0 == libdunit::ENOENT => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}
/// Applications can embed FileDialog::view().map_messages(...) in their modal
/// host, or use this convenience window and handle its selected path explicitly.
pub fn choose_file(options: FileDialogOptions) -> NativeResult<Option<String>> {
    let (theme, _) = load_theme("")?;
    let size = |key: &str| {
        theme
            .tokens
            .number(key)
            .filter(|n| n.is_finite() && *n >= 1. && *n <= 4096.)
            .map(|n| n as u32)
            .ok_or(Error(libdunit::EINVAL))
    };
    let model =
        FileDialog::new(NativeDirectorySource, options).map_err(|_| Error(libdunit::EINVAL))?;
    let mut window = ComponentWindow::open_themed(
        size("file-picker-window-width")?,
        size("file-picker-window-height")?,
        model,
    )?;
    loop {
        let event = window.next_event(Duration::from_millis(1000))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            window.close()?;
            return Ok(None);
        }
        if let Some(outcome) = window.component().outcome().cloned() {
            window.close()?;
            return Ok(match outcome {
                FileDialogOutcome::Accepted(path) => Some(path),
                FileDialogOutcome::Cancelled => None,
            });
        }
    }
}
