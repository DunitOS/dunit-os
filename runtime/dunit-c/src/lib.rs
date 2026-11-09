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
mod tree;

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
    tree: View<u64>,
    nodes: Vec<DunitUiNodeV1>,
    stop: bool,
    error: Option<dunit::Error>,
    decorate: Option<DunitUiDecorateV1>,
}
impl Component for Model {
    type Message = u64;
    fn update(&mut self, message: u64) {
        let result = unsafe { (self.callbacks.update.unwrap())(self.callbacks.user, message) };
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
        match unsafe { build_decorated(&self.callbacks, &mut self.nodes, self.decorate) } {
            Ok(tree) => self.tree = tree,
            Err(error) => self.error = Some(error),
        }
    }
    fn view(&self) -> View<u64> {
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
        let mut nodes = node_buffer();
        let tree = build_decorated(&callbacks, &mut nodes, decorate)?;
        Ok(Self {
            callbacks,
            tree,
            nodes,
            stop: false,
            error: None,
            decorate,
        })
    }
}
#[cfg(test)]
unsafe fn build(
    callbacks: &DunitUiCallbacksV1,
    nodes: &mut [DunitUiNodeV1],
) -> dunit::Result<View<u64>> {
    build_decorated(callbacks, nodes, None)
}
unsafe fn build_decorated(
    callbacks: &DunitUiCallbacksV1,
    nodes: &mut [DunitUiNodeV1],
    decorate: Option<DunitUiDecorateV1>,
) -> dunit::Result<View<u64>> {
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
    if decorate.is_some() {
        tree::convert_decorated(&nodes[..count as usize], callbacks.user, decorate)
    } else {
        tree::convert(&nodes[..count as usize])
    }
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
    run(argc, argv, envp, width, height, callbacks, None)
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
    if style.is_null() {
        return invalid().0 as i32;
    }
    let style = *style;
    if style.abi_version != DUNIT_UI_ABI_VERSION
        || style.struct_size as usize != core::mem::size_of::<DunitUiStyleV1>()
    {
        return invalid().0 as i32;
    }
    run(argc, argv, envp, width, height, callbacks, Some(style))
}

unsafe fn run(
    argc: i32,
    argv: *const *const u8,
    envp: *const *const u8,
    width: u32,
    height: u32,
    callbacks: *const DunitUiCallbacksV1,
    style: Option<DunitUiStyleV1>,
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
        let model = if decorate.is_none() {
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
mod radio_tests;
#[cfg(test)]
mod style_tests;
#[cfg(test)]
mod tests;
