//! Synchronous C callbacks over the existing native component/window runtime.
#![cfg_attr(target_os = "none", no_std)]
extern crate alloc;
use alloc::{string::String, vec, vec::Vec};
use core::{
    ffi::c_void,
    sync::atomic::{AtomicBool, Ordering},
};
use dunit::{time::Duration, ui::*, window::WindowEvent};
mod generated;
pub use generated::*;
mod number;
mod text;
mod tree;

#[derive(Clone)]
enum Message {
    Action(u64),
    Edit(u64, String),
    Number(u64, i64),
}
impl From<u64> for Message {
    fn from(value: u64) -> Self {
        Self::Action(value)
    }
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static CONSUMED: AtomicBool = AtomicBool::new(false);
struct Session;
impl Drop for Session {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}
struct Model {
    callbacks: DunitUiCallbacksV1,
    tree: View<Message>,
    nodes: Vec<DunitUiNodeV1>,
    stop: bool,
    error: Option<dunit::Error>,
    decorate: Option<DunitUiDecorateV1>,
    text: Option<DunitUiTextV1>,
    numbers: Option<DunitUiNumbersV1>,
}
impl Component for Model {
    type Message = Message;
    fn update(&mut self, message: Message) {
        let result = unsafe {
            match message {
                Message::Action(id) => (self.callbacks.update.unwrap())(self.callbacks.user, id),
                Message::Edit(id, value) => match self.text.and_then(|config| config.changed) {
                    Some(changed) => changed(self.callbacks.user, id, value.as_ptr(), value.len()),
                    None => dunit::errno::EINVAL as i32,
                },
                Message::Number(id, value) => {
                    match self.numbers.and_then(|config| config.changed) {
                        Some(changed) => changed(self.callbacks.user, id, value),
                        None => dunit::errno::EINVAL as i32,
                    }
                }
            }
        };
        if result == 1 {
            self.stop = true;
            return;
        }
        if result != 0 {
            self.error = Some(dunit::Error(if result < 0 {
                result as isize
            } else {
                dunit::errno::EINVAL
            }));
            return;
        }
        match unsafe {
            build_configured(
                &self.callbacks,
                &mut self.nodes,
                self.decorate,
                self.text.as_ref(),
                self.numbers.as_ref(),
            )
        } {
            Ok(tree) => self.tree = tree,
            Err(error) => self.error = Some(error),
        }
    }
    fn view(&self) -> View<Message> {
        self.tree.clone()
    }
}
fn invalid() -> dunit::Error {
    dunit::Error(dunit::errno::EINVAL)
}
fn node_buffer() -> Vec<DunitUiNodeV1> {
    let empty = DunitUiNodeV1 {
        kind: 0,
        parent: 0,
        key: core::ptr::null(),
        key_len: 0,
        text: core::ptr::null(),
        text_len: 0,
        message: 0,
    };
    vec![empty; MAX_VIEW_NODES]
}
impl Model {
    unsafe fn new(callbacks: DunitUiCallbacksV1) -> dunit::Result<Self> {
        Self::new_decorated(callbacks, None)
    }
    unsafe fn new_decorated(
        callbacks: DunitUiCallbacksV1,
        decorate: Option<DunitUiDecorateV1>,
    ) -> dunit::Result<Self> {
        Self::new_configured(callbacks, decorate, None)
    }
    unsafe fn new_configured(
        callbacks: DunitUiCallbacksV1,
        decorate: Option<DunitUiDecorateV1>,
        text: Option<DunitUiTextV1>,
    ) -> dunit::Result<Self> {
        Self::new_app(callbacks, decorate, text, None)
    }
    unsafe fn new_app(
        callbacks: DunitUiCallbacksV1,
        decorate: Option<DunitUiDecorateV1>,
        text: Option<DunitUiTextV1>,
        numbers: Option<DunitUiNumbersV1>,
    ) -> dunit::Result<Self> {
        let mut nodes = node_buffer();
        let tree = build_configured(
            &callbacks,
            &mut nodes,
            decorate,
            text.as_ref(),
            numbers.as_ref(),
        )?;
        Ok(Self {
            callbacks,
            tree,
            nodes,
            stop: false,
            error: None,
            decorate,
            text,
            numbers,
        })
    }
}
#[cfg(test)]
unsafe fn build(
    callbacks: &DunitUiCallbacksV1,
    nodes: &mut [DunitUiNodeV1],
) -> dunit::Result<View<Message>> {
    build_configured(callbacks, nodes, None, None, None)
}
unsafe fn build_configured(
    callbacks: &DunitUiCallbacksV1,
    nodes: &mut [DunitUiNodeV1],
    decorate: Option<DunitUiDecorateV1>,
    text: Option<&DunitUiTextV1>,
    numbers: Option<&DunitUiNumbersV1>,
) -> dunit::Result<View<Message>> {
    let count = (callbacks.build.unwrap())(callbacks.user, nodes.as_mut_ptr(), nodes.len() as u32);
    if count <= 0 {
        return Err(if count < 0 {
            dunit::Error(count as isize)
        } else {
            invalid()
        });
    }
    if count as usize > nodes.len() {
        return Err(invalid());
    }
    tree::convert_app(
        &nodes[..count as usize],
        callbacks.user,
        decorate,
        text,
        numbers,
    )
}

/// Run one themed window per process. Foreign pointers/callbacks obey the
/// lifetime/length contract in docs/sdk/c-ui.md; errors are negative DUNIT_E*.
///
/// # Safety
/// The configuration, loader vectors and callbacks must remain valid for this
/// synchronous call. Callbacks must fully initialize their declared nodes and
/// keep referenced byte spans readable until copied; no unwinding or longjmp.
#[no_mangle]
pub unsafe extern "C" fn dunit_ui_run_v1(
    argc: i32,
    argv: *const *const u8,
    envp: *const *const u8,
    width: u32,
    height: u32,
    callbacks: *const DunitUiCallbacksV1,
) -> i32 {
    run(argc, argv, envp, width, height, callbacks, None, None, None)
}

/// Open using the SDK application DSS layer and optional per-node appearance callback.
/// # Safety
/// The v1 run contract applies; style spans must remain readable until copied.
#[no_mangle]
pub unsafe extern "C" fn dunit_ui_run_styled_v1(
    argc: i32,
    argv: *const *const u8,
    envp: *const *const u8,
    width: u32,
    height: u32,
    callbacks: *const DunitUiCallbacksV1,
    style: *const DunitUiStyleV1,
) -> i32 {
    let style = match read_style(style) {
        Ok(style) => style,
        Err(error) => return error.0 as i32,
    };
    run(
        argc,
        argv,
        envp,
        width,
        height,
        callbacks,
        Some(style),
        None,
        None,
    )
}

unsafe fn read_style(style: *const DunitUiStyleV1) -> dunit::Result<DunitUiStyleV1> {
    if style.is_null() {
        return Err(invalid());
    }
    let style = *style;
    if style.abi_version != DUNIT_UI_ABI_VERSION
        || style.struct_size as usize != core::mem::size_of::<DunitUiStyleV1>()
    {
        return Err(invalid());
    }
    Ok(style)
}

/// Controlled SDK input fields; style may be null for the installed theme alone.
/// # Safety
/// The v1 run contract applies. Proposed text is borrowed only during `changed`.
#[no_mangle]
pub unsafe extern "C" fn dunit_ui_run_text_v1(
    argc: i32,
    argv: *const *const u8,
    envp: *const *const u8,
    width: u32,
    height: u32,
    callbacks: *const DunitUiCallbacksV1,
    style: *const DunitUiStyleV1,
    text: *const DunitUiTextV1,
) -> i32 {
    let text = match read_text(text) {
        Ok(text) => text,
        Err(error) => return error.0 as i32,
    };
    let style = if style.is_null() {
        None
    } else {
        match read_style(style) {
            Ok(style) => Some(style),
            Err(error) => return error.0 as i32,
        }
    };
    run(
        argc,
        argv,
        envp,
        width,
        height,
        callbacks,
        style,
        Some(text),
        None,
    )
}

unsafe fn read_text(text: *const DunitUiTextV1) -> dunit::Result<DunitUiTextV1> {
    if text.is_null() {
        return Err(invalid());
    }
    let text = *text;
    if text.abi_version != DUNIT_UI_ABI_VERSION
        || text.struct_size as usize != core::mem::size_of::<DunitUiTextV1>()
        || text.changed.is_none()
    {
        return Err(invalid());
    }
    Ok(text)
}

/// Open one SDK application with optional style, text and numeric bindings.
/// # Safety
/// The v1 run contract applies to the application record and all nested records.
#[no_mangle]
pub unsafe extern "C" fn dunit_ui_run_app_v1(
    argc: i32,
    argv: *const *const u8,
    envp: *const *const u8,
    width: u32,
    height: u32,
    callbacks: *const DunitUiCallbacksV1,
    app: *const DunitUiAppV1,
) -> i32 {
    if app.is_null() {
        return invalid().0 as i32;
    }
    let app = *app;
    if app.abi_version != DUNIT_UI_ABI_VERSION
        || app.struct_size as usize != core::mem::size_of::<DunitUiAppV1>()
    {
        return invalid().0 as i32;
    }
    let style = if app.style.is_null() {
        None
    } else {
        match read_style(app.style) {
            Ok(style) => Some(style),
            Err(error) => return error.0 as i32,
        }
    };
    let text = if app.text.is_null() {
        None
    } else {
        match read_text(app.text) {
            Ok(text) => Some(text),
            Err(error) => return error.0 as i32,
        }
    };
    let numbers = if app.numbers.is_null() {
        None
    } else {
        let numbers = *app.numbers;
        if numbers.abi_version != DUNIT_UI_ABI_VERSION
            || numbers.struct_size as usize != core::mem::size_of::<DunitUiNumbersV1>()
        {
            return invalid().0 as i32;
        }
        Some(numbers)
    };
    run(
        argc, argv, envp, width, height, callbacks, style, text, numbers,
    )
}

unsafe fn run(
    argc: i32,
    argv: *const *const u8,
    envp: *const *const u8,
    width: u32,
    height: u32,
    callbacks: *const DunitUiCallbacksV1,
    style: Option<DunitUiStyleV1>,
    text: Option<DunitUiTextV1>,
    numbers: Option<DunitUiNumbersV1>,
) -> i32 {
    if callbacks.is_null()
        || argc < 0
        || (argc > 0 && argv.is_null())
        || envp.is_null()
        || width == 0
        || height == 0
    {
        return invalid().0 as i32;
    }
    let callbacks = *callbacks;
    if callbacks.abi_version != DUNIT_UI_ABI_VERSION
        || callbacks.struct_size as usize != core::mem::size_of::<DunitUiCallbacksV1>()
        || callbacks.build.is_none()
        || callbacks.update.is_none()
    {
        return invalid().0 as i32;
    }
    let (overlay, decorate) = match style {
        Some(style) => match tree::text(style.stylesheet, style.stylesheet_len) {
            Ok(text) => (text, style.decorate),
            Err(error) => return error.0 as i32,
        },
        None => (String::new(), None),
    };
    if RUNNING
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return dunit::errno::EBUSY as i32;
    }
    let _session = Session;
    if CONSUMED.load(Ordering::Acquire) {
        return dunit::errno::EPIPE as i32;
    }
    let result = (|| {
        let model = if numbers.is_some() {
            Model::new_app(callbacks, decorate, text, numbers)?
        } else if text.is_some() {
            Model::new_configured(callbacks, decorate, text)?
        } else if decorate.is_none() {
            Model::new(callbacks)?
        } else {
            Model::new_decorated(callbacks, decorate)?
        };
        CONSUMED.store(true, Ordering::Release);
        dunit::__private::init_c_runtime(argc as usize, argv, envp);
        let mut ui = ComponentWindow::open_themed_with_style(width, height, model, &overlay)?;
        let result = loop {
            match ui.next_event(Duration::from_secs(30)) {
                Err(error) => break Err(error),
                Ok(Some(WindowEvent::CloseRequested)) => break Ok(()),
                _ => {}
            }
            if let Some(error) = ui.component().error {
                break Err(error);
            }
            if ui.component().stop {
                break Ok(());
            }
        };
        let closed = ui.close();
        result.and(closed)
    })();
    match result {
        Ok(()) => 0,
        Err(error) => error.0 as i32,
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    dunit::__private::panic(info)
}

#[cfg(test)]
mod checkable_tests;
#[cfg(test)]
mod disabled_tests;
#[cfg(test)]
mod number_tests;
#[cfg(test)]
mod radio_tests;
#[cfg(test)]
mod style_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_tests;
