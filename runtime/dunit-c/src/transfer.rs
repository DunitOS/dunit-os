//! Callback-scoped access to the existing window transfer client. Local file
//! tokens own real attenuated capabilities; wire integers are never adopted.
use super::*;
use alloc::collections::BTreeMap;
use dunit_transfer::{Action, Client, File, Format, Kind, Offer, Payload, Status};
const MAX_FILES: usize = 1024;
#[derive(Default)]
pub(super) struct Files {
    next: u64,
    values: BTreeMap<u64, File>,
}
impl Files {
    fn insert(&mut self, file: File) -> dunit::Result<u64> {
        if self.values.len() >= MAX_FILES {
            return Err(dunit::Error(dunit::errno::ENOBUFS));
        }
        self.next = self.next.checked_add(1).ok_or_else(invalid)?;
        self.values.insert(self.next, file);
        Ok(self.next)
    }
    fn get(&self, id: u64) -> dunit::Result<&File> {
        self.values
            .get(&id)
            .ok_or(dunit::Error(dunit::errno::EBADF))
    }
}
#[derive(Default)]
pub(super) struct Commands {
    pub supported: bool,
    pub pointer_mode: Option<dunit::window::PointerMode>,
}
struct Context<'a> {
    commands: &'a mut Commands,
    client: &'a mut Client,
    files: &'a mut Files,
}
fn native(e: dunit_transfer::Error) -> dunit::Error {
    dunit::Error(e.0)
}
unsafe fn slice<'a, T>(p: *const T, n: usize, max: usize) -> dunit::Result<&'a [T]> {
    if n > max || (n > 0 && p.is_null()) {
        return Err(invalid());
    }
    Ok(if n == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(p, n)
    })
}
unsafe fn preferences(r: &DunitTransferRequestV1) -> dunit::Result<Offer> {
    let mut formats = Vec::new();
    for f in slice(r.formats, r.formats_len, dunit_transfer::MAX_FORMATS)? {
        formats.push(Format::parse(&tree::text(f.mime, f.mime_len)?).map_err(|_| invalid())?);
    }
    let offer = Offer {
        formats,
        actions: r.flags,
        preferred: Action::from_u32(r.preferred).ok_or_else(invalid)?,
    };
    offer.validate().map_err(|_| invalid())?;
    Ok(offer)
}
impl Context<'_> {
    unsafe fn request(
        &mut self,
        r: &DunitTransferRequestV1,
        out: &mut DunitTransferResultV1,
    ) -> dunit::Result<()> {
        if r.reserved != 0 {
            return Err(invalid());
        }
        *out = DunitTransferResultV1 {
            request: 0,
            capability: 0,
            size: 0,
            kind: 0,
            rights: 0,
        };
        match r.kind {
            DUNIT_C_FILE_OPEN | DUNIT_C_FILE_CHILD => {
                if r.flags & !dunit_transfer::RIGHT_WRITE != 0 {
                    return Err(invalid());
                }
                if self.files.values.len() >= MAX_FILES {
                    return Err(dunit::Error(dunit::errno::ENOBUFS));
                }
                let path = tree::text(r.data, r.data_len)?;
                let file = if r.kind == DUNIT_C_FILE_OPEN {
                    File::open(&path, r.flags != 0)
                } else {
                    self.files
                        .get(r.capability)?
                        .child_with_rights(&path, r.flags != 0)
                }
                .map_err(native)?;
                out.kind = file.kind as u32;
                out.rights = file.handle.rights().map_err(native)?;
                out.capability = self.files.insert(file)?;
            }
            DUNIT_C_FILE_CLOSE => {
                self.files
                    .values
                    .remove(&r.capability)
                    .ok_or(dunit::Error(dunit::errno::EBADF))?;
            }
            DUNIT_C_FILE_STAT => {
                let f = self.files.get(r.capability)?;
                out.size = f.stat().map_err(native)?.size as u64;
                out.kind = f.kind as u32;
                out.rights = f.handle.rights().map_err(native)?;
            }
            DUNIT_C_FILE_READ => {
                if r.buffer_len > DUNIT_UI_MAX_TEXT_BYTES as usize
                    || (r.buffer_len > 0 && r.buffer.is_null())
                {
                    return Err(invalid());
                }
                let buffer = if r.buffer_len == 0 {
                    &mut []
                } else {
                    core::slice::from_raw_parts_mut(r.buffer, r.buffer_len)
                };
                out.size = self
                    .files
                    .get(r.capability)?
                    .read_at(r.offset, buffer)
                    .map_err(native)? as u64;
            }
            _ => {
                let kind = Kind::from_u16(u16::try_from(r.kind).map_err(|_| invalid())?)
                    .ok_or_else(invalid)?;
                out.request = match kind {
                    Kind::ClipboardOffer | Kind::DragOffer => {
                        let preferred = Action::from_u32(r.preferred).ok_or_else(invalid)?;
                        let payload = if r.files_len == 0 {
                            Payload::text(&tree::text(r.data, r.data_len)?)
                        } else {
                            let mut files = Vec::new();
                            for id in slice(r.files, r.files_len, dunit_transfer::MAX_FILE_ITEMS)? {
                                let f = self.files.get(*id)?;
                                let rights = f.handle.rights().map_err(native)?;
                                let allowed = dunit_transfer::RIGHT_READ
                                    | dunit_transfer::RIGHT_TRANSFER
                                    | if r.flags & Action::Move as u32 != 0 {
                                        dunit_transfer::RIGHT_WRITE
                                    } else {
                                        0
                                    };
                                let handle =
                                    f.handle.duplicate(rights & allowed).map_err(native)?;
                                files.push(File {
                                    handle,
                                    path: f.path.clone(),
                                    kind: f.kind,
                                });
                            }
                            Payload::from_files(files)
                        }
                        .map_err(native)?;
                        if kind == Kind::ClipboardOffer {
                            self.client.publish_clipboard(payload, r.flags, preferred)
                        } else {
                            self.client.start_drag(payload, r.flags, preferred)
                        }
                    }
                    Kind::ClipboardRead => self.client.read_clipboard(preferences(r)?),
                    Kind::RegisterTarget => {
                        if r.formats_len == 0 {
                            self.client.unregister_target()
                        } else {
                            self.client.register_target(preferences(r)?)
                        }
                    }
                    Kind::RegisterOpener => self.client.register_opener(preferences(r)?),
                    Kind::DragAccept => {
                        if r.preferred == 0 {
                            self.client.drag_reject(r.transfer)
                        } else {
                            self.client.drag_accept(
                                r.transfer,
                                usize::try_from(r.offset).map_err(|_| invalid())?,
                                Action::from_u32(r.preferred).ok_or_else(invalid)?,
                            )
                        }
                    }
                    Kind::Complete => self
                        .client
                        .complete(r.transfer, Status::from_u32(r.flags).ok_or_else(invalid)?),
                    Kind::Cancel => self.client.cancel(r.transfer),
                    Kind::ClipboardClear => self.client.clear(),
                    Kind::ClipboardQuery => self.client.query(),
                    Kind::Grant => self
                        .client
                        .grant_destination(r.transfer, &self.files.get(r.capability)?.handle),
                    Kind::FileMove => self.client.file_move(
                        r.transfer,
                        dunit_transfer::FileMove {
                            item: u32::try_from(r.offset).map_err(|_| invalid())?,
                            destination: u32::try_from(r.capability).map_err(|_| invalid())?,
                            leaf: tree::text(r.data, r.data_len)?,
                        },
                    ),
                    _ => return Err(invalid()),
                }
                .map_err(native)?;
            }
        }
        Ok(())
    }
}
/// Issue a native SDK transfer/capability operation during a transfer callback.
/// # Safety
/// `session` is valid exclusively for the current callback invocation. Do not
/// retain it, call from another thread, recursively invoke the UI, or alias the
/// result/buffer with request storage. All spans must be readable for this call.
#[no_mangle]
pub unsafe extern "C" fn dunit_transfer_request_v1(
    session: *mut c_void,
    request: *const DunitTransferRequestV1,
    result: *mut DunitTransferResultV1,
) -> i32 {
    if session.is_null() || request.is_null() || result.is_null() {
        return invalid().0 as i32;
    }
    let context = &mut *(session as *mut Context<'_>);
    match context.request(&*request, &mut *result) {
        Ok(()) => 0,
        Err(e) => e.0 as i32,
    }
}
/// Queue a mode change in the current native-event callback session.
/// Zero means queued; DUNIT_GUI_IN_POINTER_MODE reports acceptance or refusal.
/// # Safety
/// The session must be the live pointer supplied to this callback invocation,
/// used on the UI thread without retaining it or re-entering the UI. Its scoped
/// lifetime is the same as for dunit_transfer_request_v1.
#[no_mangle]
pub unsafe extern "C" fn dunit_ui_pointer_mode_v1(session: *mut c_void, mode: u32) -> i32 {
    if session.is_null() {
        return invalid().0 as i32;
    }
    let Some(mode) = dunit::window::PointerMode::from_u32(mode) else {
        return invalid().0 as i32;
    };
    let context = &mut *(session as *mut Context<'_>);
    if !context.commands.supported {
        return dunit::errno::EOPNOTSUPP as i32;
    }
    if context.commands.pointer_mode.is_some() {
        return dunit::errno::EBUSY as i32;
    }
    context.commands.pointer_mode = Some(mode);
    0
}
pub(super) unsafe fn dispatch(
    client: &mut Client,
    files: &mut Files,
    user: *mut c_void,
    callback: DunitTransferCallbackV1,
    mut event: Option<dunit_transfer::Event>,
    input: Option<dunit::window::Input>,
    commands: &mut Commands,
) -> dunit::Result<i32> {
    let mut formats = Vec::new();
    let mut representations = Vec::new();
    let mut tokens = Vec::new();
    if let Some(e) = event.as_ref() {
        if let Some(offer) = &e.offer {
            for f in &offer.formats {
                let mime = f.name();
                formats.push(DunitTransferFormatV1 {
                    mime: mime.as_ptr(),
                    mime_len: mime.len(),
                });
            }
        }
    }
    // Own incoming handles before C sees tokens. They stay valid until close or
    // session end, independently of the lifetime of this event's borrowed data.
    if let Some(payload) = event.as_mut().and_then(|e| e.payload.as_mut()) {
        if files
            .values
            .len()
            .checked_add(payload.files.len())
            .ok_or_else(invalid)?
            > MAX_FILES
        {
            return Err(dunit::Error(dunit::errno::ENOBUFS));
        }
        for f in core::mem::take(&mut payload.files) {
            tokens.push(files.insert(f)?);
        }
        for r in &payload.bundle.representations {
            let mime = r.format.name();
            // File-list wire handle integers are private to the authenticated
            // SDK; the C surface receives token/path/rights records instead.
            let data = if r.format == Format::Files {
                &[][..]
            } else {
                r.data.as_slice()
            };
            representations.push(DunitTransferDataV1 {
                mime: mime.as_ptr(),
                mime_len: mime.len(),
                data: data.as_ptr(),
                data_len: data.len(),
            });
        }
    }
    let file_records: Vec<_> = tokens
        .iter()
        .map(|id| {
            let f = files.get(*id)?;
            Ok(DunitTransferFileV1 {
                capability: *id,
                path: f.path.as_ptr(),
                path_len: f.path.len(),
                kind: f.kind as u32,
                rights: f.handle.rights().map_err(native)?,
            })
        })
        .collect::<dunit::Result<_>>()?;
    // Path storage must remain valid even if C explicitly closes a token in the
    // callback. Copy these strings into callback-owned storage first.
    let paths: Vec<_> = tokens
        .iter()
        .map(|id| files.get(*id).map(|f| f.path.clone()))
        .collect::<dunit::Result<_>>()?;
    let file_records: Vec<_> = file_records
        .into_iter()
        .zip(&paths)
        .map(|(mut f, p)| {
            f.path = p.as_ptr();
            f.path_len = p.len();
            f
        })
        .collect();
    let input = input.map(|i| DunitUiNativeInputV1 {
        kind: i.kind as u32,
        x: i.x,
        y: i.y,
        value: i.value,
        button: i.pointer_button() as u32,
        modifiers: i.pointer_modifiers() as u32,
    });
    let mut e = DunitTransferEventV1 {
        input: input.as_ref().map_or(core::ptr::null(), |i| i as *const _),
        kind: 0,
        arg0: 0,
        arg1: 0,
        source: 0,
        request: 0,
        transfer: 0,
        formats: formats.as_ptr(),
        formats_len: formats.len(),
        files: file_records.as_ptr(),
        files_len: file_records.len(),
        representations: representations.as_ptr(),
        representations_len: representations.len(),
    };
    if let Some(native) = event.as_ref() {
        let m = native.message;
        e.kind = m.kind as u32;
        e.arg0 = m.arg0;
        e.arg1 = m.arg1;
        e.request = m.request;
        e.transfer = m.transfer;
        e.source = native.source as u32;
    }
    let mut context = Context {
        client,
        files,
        commands,
    };
    Ok(callback(user, &mut context as *mut _ as *mut c_void, &e))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(kind: u32) -> DunitTransferRequestV1 {
        DunitTransferRequestV1 {
            kind,
            flags: 0,
            preferred: 0,
            reserved: 0,
            transfer: 0,
            capability: 0,
            offset: 0,
            data: core::ptr::null(),
            data_len: 0,
            files: core::ptr::null(),
            files_len: 0,
            formats: core::ptr::null(),
            formats_len: 0,
            buffer: core::ptr::null_mut(),
            buffer_len: 0,
        }
    }
    #[test]
    fn invalid_transfer_requests_and_stale_tokens_fail_before_native_io() {
        let mut client = Client::new(42, 0);
        let mut files = Files::default();
        let mut commands = Commands::default();
        let mut context = Context {
            commands: &mut commands,
            client: &mut client,
            files: &mut files,
        };
        let p = &mut context as *mut _ as *mut c_void;
        let mut out = DunitTransferResultV1 {
            request: 99,
            capability: 99,
            size: 99,
            kind: 99,
            rights: 99,
        };
        for kind in [
            DUNIT_C_FILE_CHILD,
            DUNIT_C_FILE_READ,
            DUNIT_C_FILE_STAT,
            DUNIT_C_FILE_CLOSE,
        ] {
            let mut r = request(kind);
            r.capability = 1;
            assert_eq!(
                unsafe { dunit_transfer_request_v1(p, &r, &mut out) },
                dunit::errno::EBADF as i32
            );
        }
        let mut r = request(DUNIT_C_FILE_OPEN);
        r.data_len = 1;
        assert_eq!(
            unsafe { dunit_transfer_request_v1(p, &r, &mut out) },
            dunit::errno::EINVAL as i32
        );
        r = request(DUNIT_C_FILE_READ);
        r.buffer_len = 1;
        assert_eq!(
            unsafe { dunit_transfer_request_v1(p, &r, &mut out) },
            dunit::errno::EINVAL as i32
        );
        r = request(Kind::ClipboardRead as u32);
        r.formats_len = 1;
        r.preferred = Action::Copy as u32;
        r.flags = Action::Copy as u32;
        assert_eq!(
            unsafe { dunit_transfer_request_v1(p, &r, &mut out) },
            dunit::errno::EINVAL as i32
        );
        r = request(Kind::ClipboardClear as u32);
        assert_eq!(
            unsafe { dunit_transfer_request_v1(p, &r, &mut out) },
            dunit::errno::EOPNOTSUPP as i32
        );
        r = request(0x10001 + 0x10000);
        assert_eq!(
            unsafe { dunit_transfer_request_v1(p, &r, &mut out) },
            dunit::errno::EINVAL as i32
        );
        assert_eq!(
            unsafe { dunit_transfer_request_v1(core::ptr::null_mut(), &r, &mut out) },
            dunit::errno::EINVAL as i32
        );
    }
    #[test]
    fn native_sdk_payload_is_borrowed_during_the_scoped_c_callback() {
        unsafe extern "C" fn callback(
            user: *mut c_void,
            session: *mut c_void,
            event: *const DunitTransferEventV1,
        ) -> i32 {
            let e = &*event;
            assert_eq!(e.kind, Kind::Data as u32);
            assert_eq!(e.representations_len, 1);
            assert_eq!(e.files_len, 0);
            let r = &*e.representations;
            assert_eq!(
                tree::text(r.mime, r.mime_len).unwrap(),
                dunit_transfer::TEXT_MIME
            );
            assert_eq!(tree::text(r.data, r.data_len).unwrap(), "Owned UTF-8 ☕");
            let mut out = DunitTransferResultV1 {
                request: 0,
                capability: 0,
                size: 0,
                kind: 0,
                rights: 0,
            };
            let r = request(Kind::ClipboardClear as u32);
            assert_eq!(
                dunit_transfer_request_v1(session, &r, &mut out),
                dunit::errno::EOPNOTSUPP as i32
            );
            *(user as *mut bool) = true;
            2
        }
        let mut client = Client::new(42, 0);
        let mut files = Files::default();
        let mut seen = false;
        let event = dunit_transfer::Event {
            message: dunit_transfer::Message {
                kind: Kind::Data,
                request: 99,
                transfer: 100,
                arg0: 0,
                arg1: 0,
            },
            offer: None,
            payload: Some(Payload::text("Owned UTF-8 ☕").unwrap()),
            source: false,
        };
        assert_eq!(
            unsafe {
                dispatch(
                    &mut client,
                    &mut files,
                    &mut seen as *mut _ as *mut c_void,
                    callback,
                    Some(event),
                    None,
                    &mut Commands::default(),
                )
            }
            .unwrap(),
            2
        );
        assert!(seen);
    }
    #[test]
    fn pointer_command_is_scoped_bounded_and_rejects_unsupported_modes() {
        let mut client = Client::new(42, 0);
        let mut files = Files::default();
        let mut commands = Commands::default();
        let mut context = Context {
            client: &mut client,
            files: &mut files,
            commands: &mut commands,
        };
        let session = &mut context as *mut _ as *mut c_void;
        unsafe {
            assert_eq!(
                dunit_ui_pointer_mode_v1(core::ptr::null_mut(), 0),
                dunit::errno::EINVAL as i32
            );
            assert_eq!(
                dunit_ui_pointer_mode_v1(session, 99),
                dunit::errno::EINVAL as i32
            );
            assert_eq!(
                dunit_ui_pointer_mode_v1(session, 2),
                dunit::errno::EOPNOTSUPP as i32
            );
            context.commands.supported = true;
            assert_eq!(dunit_ui_pointer_mode_v1(session, 2), 0);
            assert_eq!(
                dunit_ui_pointer_mode_v1(session, 1),
                dunit::errno::EBUSY as i32
            );
        }
        assert_eq!(
            commands.pointer_mode,
            Some(dunit::window::PointerMode::Locked)
        );
    }
}
