# Composable file dialogs

`FileDialog<P: DirectorySource>` supplies Open, Save and Directory selection over
shared file-browser entries/path normalization. All labels and filters belong to
`FileDialogOptions`; the component does not perform file writes. The native
`NativeDirectorySource` reads the filesystem through existing ABI calls. `gui_files`
uses the shared browser path helper, SDK menus and filename dialogs over its
icon canvas; see the [GUI migration receipt](gui-migration.md).

Embed `picker.view().map_messages(ParentMessage::Picker)` as the dialog argument of
`View::modal`, forwarding child messages through the parent model. It then shares
the real parent's layout, modal focus, popup/keyboard scope and opener restoration.
`dunit::ui::choose_file(options)` is a convenience standalone window returning an
optional path. It does not establish compositor modality over another window.

The dialog includes navigation history, Up, location entry (Enter navigates),
refresh, hidden-file toggle, extension filters, directory-first initial listing,
controlled sortable table, filename entry, errors and cancel/default actions.
Directories stay visible through extension filters; matching is ASCII case
insensitive. Device entries cannot be selected. Invalid/duplicate provider names
reject the complete candidate listing. Failed navigation preserves the previous
listing and filename draft. Relative locations normalize against the current
folder; supplied initial directory must be absolute.

Acceptance revalidates actual target metadata. Open requires a regular file;
Directory requires a directory; Save requires an existing parent directory and
valid single-segment filename. Existing files open a separate confirmation view.
Confirmation is bound to the original path and revalidates it again. Returning a
path is no filesystem transaction or overwrite guarantee: callers must handle
errors/races when they subsequently open/write it.

Native `fs::read_dir` grows its snapshot buffer to the existing ABI's 1024-entry
limit. A full maximum buffer returns `ENOBUFS` rather than silently presenting a
possibly truncated directory. Native metadata errors are reported by the dialog;
optional per-file size does not prevent displaying an otherwise valid entry.

Verification: host provider tests cover error rollback, stale entries, filters,
history, overwrite identity, all three modes and modal message composition. The
combined QEMU fixture lists real MemFS files, confirms/declines overwrite,
cancels the picker and verifies the existing file bytes remain unchanged.
