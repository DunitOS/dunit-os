//! Thin C entry to the SDK native Open/Save/Directory dialog.
use super::*;
pub(super) unsafe fn options(
    config: *const DunitUiFileDialogV1,
) -> dunit::Result<FileDialogOptions> {
    if config.is_null() {
        return Err(invalid());
    }
    let c = &*config;
    if c.abi_version != DUNIT_UI_ABI_VERSION
        || c.struct_size as usize != core::mem::size_of::<DunitUiFileDialogV1>()
        || c.reserved != 0
        || c.labels.is_null()
        || c.filters_len > MAX_VIEW_NODES
    {
        return Err(invalid());
    }
    let mode = match c.mode {
        DUNIT_UI_FILE_OPEN => FileDialogMode::Open,
        DUNIT_UI_FILE_SAVE => FileDialogMode::Save,
        DUNIT_UI_FILE_DIRECTORY => FileDialogMode::Directory,
        _ => return Err(invalid()),
    };
    let l = &*c.labels;
    let labels = FileDialogLabels {
        location: tree::text(l.location, l.location_len)?,
        name: tree::text(l.name, l.name_len)?,
        files: tree::text(l.files, l.files_len)?,
        back: tree::text(l.back, l.back_len)?,
        forward: tree::text(l.forward, l.forward_len)?,
        up: tree::text(l.up, l.up_len)?,
        refresh: tree::text(l.refresh, l.refresh_len)?,
        enter: tree::text(l.enter, l.enter_len)?,
        accept: tree::text(l.accept, l.accept_len)?,
        cancel: tree::text(l.cancel, l.cancel_len)?,
        hidden: tree::text(l.hidden, l.hidden_len)?,
        filter: tree::text(l.filter, l.filter_len)?,
        name_column: tree::text(l.name_column, l.name_column_len)?,
        kind_column: tree::text(l.kind_column, l.kind_column_len)?,
        size_column: tree::text(l.size_column, l.size_column_len)?,
        file_kind: tree::text(l.file_kind, l.file_kind_len)?,
        directory_kind: tree::text(l.directory_kind, l.directory_kind_len)?,
        device_kind: tree::text(l.device_kind, l.device_kind_len)?,
        replace_title: tree::text(l.replace_title, l.replace_title_len)?,
        replace_message: tree::text(l.replace_message, l.replace_message_len)?,
        replace_accept: tree::text(l.replace_accept, l.replace_accept_len)?,
        invalid_path: tree::text(l.invalid_path, l.invalid_path_len)?,
        invalid_name: tree::text(l.invalid_name, l.invalid_name_len)?,
        missing_file: tree::text(l.missing_file, l.missing_file_len)?,
        not_directory: tree::text(l.not_directory, l.not_directory_len)?,
        invalid_entries: tree::text(l.invalid_entries, l.invalid_entries_len)?,
    };
    if c.filters_len > 0 && c.filters.is_null() {
        return Err(invalid());
    }
    let filters = if c.filters_len == 0 {
        &[][..]
    } else {
        core::slice::from_raw_parts(c.filters, c.filters_len)
    };
    let mut owned = Vec::new();
    for f in filters {
        if f.extensions_len > MAX_VIEW_NODES || (f.extensions_len > 0 && f.extensions.is_null()) {
            return Err(invalid());
        }
        let ext = if f.extensions_len == 0 {
            &[][..]
        } else {
            core::slice::from_raw_parts(f.extensions, f.extensions_len)
        };
        let extensions = ext
            .iter()
            .map(|e| tree::text(e.data, e.length))
            .collect::<dunit::Result<Vec<_>>>()?;
        owned.push(FileFilter::new(
            tree::text(f.key, f.key_len)?,
            tree::text(f.label, f.label_len)?,
            extensions,
        ));
    }
    Ok(FileDialogOptions {
        key: tree::text(c.key, c.key_len)?,
        title: tree::text(c.title, c.title_len)?,
        directory: tree::text(c.directory, c.directory_len)?,
        filename: tree::text(c.filename, c.filename_len)?,
        mode,
        filters: owned,
        labels,
    })
}
/// Open one native SDK file dialog in a fresh process/window session.
/// # Safety
/// The loader, options and output spans obey the run v1 contract. The output
/// buffer and written pointer must be writable, distinct and valid throughout.
/// Returns 0 for a selected path (without NUL), 1 for cancellation, or an error.
#[no_mangle]
pub unsafe extern "C" fn dunit_ui_choose_file_v1(
    argc: i32,
    argv: *const *const u8,
    envp: *const *const u8,
    config: *const DunitUiFileDialogV1,
    buffer: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    if argc < 0
        || (argc > 0 && argv.is_null())
        || envp.is_null()
        || written.is_null()
        || (capacity > 0 && buffer.is_null())
        || capacity > DUNIT_UI_MAX_TEXT_BYTES as usize
    {
        return invalid().0 as i32;
    }
    *written = 0;
    let options = match options(config) {
        Ok(o) => o,
        Err(e) => return e.0 as i32,
    };
    let _session = match Session::enter() {
        Ok(s) => s,
        Err(e) => return e.0 as i32,
    };
    CONSUMED.store(true, Ordering::Release);
    dunit::__private::init_c_runtime(argc as usize, argv, envp);
    match dunit::ui::choose_file(options) {
        Ok(Some(path)) => {
            *written = path.len();
            if path.len() > capacity {
                return dunit::errno::ENOBUFS as i32;
            }
            if !path.is_empty() {
                core::ptr::copy_nonoverlapping(path.as_ptr(), buffer, path.len())
            }
            0
        }
        Ok(None) => 1,
        Err(e) => e.0 as i32,
    }
}
