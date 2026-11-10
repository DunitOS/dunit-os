use super::*;
struct Source;
impl DirectorySource for Source {
    fn read_directory(&mut self, path: &str) -> core::result::Result<Vec<FileEntry>, String> {
        assert_eq!(path, "/assets/tcc");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/tcc");
        Ok(std::fs::read_dir(root)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                FileEntry {
                    name: e.file_name().into_string().unwrap(),
                    kind: FileKind::File,
                    size: Some(e.metadata().unwrap().len()),
                }
            })
            .collect())
    }
    fn metadata(&mut self, path: &str) -> core::result::Result<Option<FileKind>, String> {
        Ok(if path == "/assets/tcc" {
            Some(FileKind::Directory)
        } else if path == "/assets/tcc/workspace.c" {
            Some(FileKind::File)
        } else {
            None
        })
    }
}
fn labels() -> DunitUiFileLabelsV1 {
    DunitUiFileLabelsV1 {
        location: b"Location".as_ptr(),
        location_len: 8,
        name: b"Name".as_ptr(),
        name_len: 4,
        files: b"Files".as_ptr(),
        files_len: 5,
        back: b"Back".as_ptr(),
        back_len: 4,
        forward: b"Forward".as_ptr(),
        forward_len: 7,
        up: b"Up".as_ptr(),
        up_len: 2,
        refresh: b"Refresh".as_ptr(),
        refresh_len: 7,
        enter: b"Enter".as_ptr(),
        enter_len: 5,
        accept: b"Open".as_ptr(),
        accept_len: 4,
        cancel: b"Cancel".as_ptr(),
        cancel_len: 6,
        hidden: b"Hidden".as_ptr(),
        hidden_len: 6,
        filter: b"Filter".as_ptr(),
        filter_len: 6,
        name_column: b"Name".as_ptr(),
        name_column_len: 4,
        kind_column: b"Kind".as_ptr(),
        kind_column_len: 4,
        size_column: b"Size".as_ptr(),
        size_column_len: 4,
        file_kind: b"File".as_ptr(),
        file_kind_len: 4,
        directory_kind: b"Directory".as_ptr(),
        directory_kind_len: 9,
        device_kind: b"Device".as_ptr(),
        device_kind_len: 6,
        replace_title: b"Replace file?".as_ptr(),
        replace_title_len: 13,
        replace_message: b"This file exists. Replace?".as_ptr(),
        replace_message_len: 26,
        replace_accept: b"Replace".as_ptr(),
        replace_accept_len: 7,
        invalid_path: b"Invalid path".as_ptr(),
        invalid_path_len: 12,
        invalid_name: b"Invalid name".as_ptr(),
        invalid_name_len: 12,
        missing_file: b"Missing file".as_ptr(),
        missing_file_len: 12,
        not_directory: b"Not a directory".as_ptr(),
        not_directory_len: 15,
        invalid_entries: b"Invalid entries".as_ptr(),
        invalid_entries_len: 15,
    }
}
#[test]
fn c_file_options_copy_into_the_real_filtered_sdk_dialog() {
    let labels = labels();
    let ext = DunitUiStringV1 {
        data: b"c".as_ptr(),
        length: 1,
    };
    let filter = DunitUiFileFilterV1 {
        key: b"c".as_ptr(),
        key_len: 1,
        label: b"C sources".as_ptr(),
        label_len: 9,
        extensions: &ext,
        extensions_len: 1,
    };
    let mut config = DunitUiFileDialogV1 {
        abi_version: 1,
        struct_size: core::mem::size_of::<DunitUiFileDialogV1>() as u32,
        mode: DUNIT_UI_FILE_OPEN,
        reserved: 0,
        key: b"picker".as_ptr(),
        key_len: 6,
        title: b"C SDK open file".as_ptr(),
        title_len: 15,
        directory: b"/assets/tcc".as_ptr(),
        directory_len: 11,
        filename: b"workspace.c".as_ptr(),
        filename_len: 11,
        filters: &filter,
        filters_len: 1,
        labels: &labels,
    };
    let options = unsafe { file_dialog::options(&config) }.unwrap();
    let direct = FileDialogOptions {
        key: "picker".into(),
        title: "C SDK open file".into(),
        directory: "/assets/tcc".into(),
        filename: "workspace.c".into(),
        mode: FileDialogMode::Open,
        filters: vec![FileFilter::new("c", "C sources", vec!["c".into()])],
        labels: FileDialogLabels {
            location: "Location".into(),
            name: "Name".into(),
            files: "Files".into(),
            back: "Back".into(),
            forward: "Forward".into(),
            up: "Up".into(),
            refresh: "Refresh".into(),
            enter: "Enter".into(),
            accept: "Open".into(),
            cancel: "Cancel".into(),
            hidden: "Hidden".into(),
            filter: "Filter".into(),
            name_column: "Name".into(),
            kind_column: "Kind".into(),
            size_column: "Size".into(),
            file_kind: "File".into(),
            directory_kind: "Directory".into(),
            device_kind: "Device".into(),
            replace_title: "Replace file?".into(),
            replace_message: "This file exists. Replace?".into(),
            replace_accept: "Replace".into(),
            invalid_path: "Invalid path".into(),
            invalid_name: "Invalid name".into(),
            missing_file: "Missing file".into(),
            not_directory: "Not a directory".into(),
            invalid_entries: "Invalid entries".into(),
        },
    };
    assert_eq!(options, direct);
    let mut adapted = Application::new(FileDialog::new(Source, options).unwrap()).unwrap();
    let direct = Application::new(FileDialog::new(Source, direct).unwrap()).unwrap();
    assert_eq!(adapted.active_dialog(), Some("picker"));
    assert!(adapted
        .component()
        .entries()
        .iter()
        .all(|e| e.name.ends_with(".c")));
    let mut expected = super::checkable_tests::renderer();
    let mut actual = super::checkable_tests::renderer();
    expected.render_application(&direct, 640, 500).unwrap();
    actual.render_application(&adapted, 640, 500).unwrap();
    assert_eq!(expected.pixels(), actual.pixels());
    if let Some(out) = std::env::var_os("DUNIT_C_GOLDEN_DIR") {
        let rgb: Vec<u8> = expected
            .pixels()
            .iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect();
        std::fs::write(
            std::path::Path::new(&out).join("c-ui-picker-initial.rgb"),
            rgb,
        )
        .unwrap();
    }
    adapted.key(Key::plain('\r')).unwrap();
    assert_eq!(
        adapted.component().outcome(),
        Some(&FileDialogOutcome::Accepted(
            "/assets/tcc/workspace.c".into()
        ))
    );
    for mode in [DUNIT_UI_FILE_SAVE, DUNIT_UI_FILE_DIRECTORY] {
        config.mode = mode;
        assert!(unsafe { file_dialog::options(&config) }.is_ok());
    }
    config.mode = 99;
    assert!(unsafe { file_dialog::options(&config) }.is_err());
    config.mode = DUNIT_UI_FILE_OPEN;
    config.filters = core::ptr::null();
    assert!(unsafe { file_dialog::options(&config) }.is_err());
    config.filters_len = 0;
    config.labels = core::ptr::null();
    assert!(unsafe { file_dialog::options(&config) }.is_err());
}
