use crate::sync::SpinLock;

const MAX_DEVICES: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Framebuffer,
    Input,
    Bus,
    UsbController,
    Block,
    Network,
    Other,
}

#[derive(Clone, Copy)]
pub struct DeviceRegistration {
    pub name: &'static str,
    pub class: DeviceClass,
    pub driver: &'static str,
}

struct RegistryState {
    devices: [Option<DeviceRegistration>; MAX_DEVICES],
    count: usize,
}

static REGISTRY: SpinLock<RegistryState> = SpinLock::new(RegistryState {
    devices: [None; MAX_DEVICES],
    count: 0,
});

pub fn register(name: &'static str, class: DeviceClass, driver: &'static str) {
    let mut state = REGISTRY.lock();
    let existing = state.devices[..state.count].iter()
        .position(|entry| entry.is_some_and(|entry| entry.name == name));
    let index = existing.unwrap_or(state.count);
    if index == state.devices.len() { return; }
    state.devices[index] = Some(DeviceRegistration { name, class, driver });
    if existing.is_none() { state.count += 1; }
    drop(state);
    publish_dev_node(name);
}

pub fn snapshot(out: &mut [Option<DeviceRegistration>]) -> usize {
    let state = REGISTRY.lock();
    let count = state.count.min(out.len());
    out[..count].copy_from_slice(&state.devices[..count]);
    count
}

pub fn class_name(class: DeviceClass) -> &'static str {
    match class {
        DeviceClass::Framebuffer => "framebuffer",
        DeviceClass::Input => "input",
        DeviceClass::Bus => "bus",
        DeviceClass::UsbController => "usb-controller",
        DeviceClass::Block => "block",
        DeviceClass::Network => "network",
        DeviceClass::Other => "other",
    }
}

fn publish_dev_node(name: &str) {
    let mut path = [0u8; 69];
    let prefix = b"/dev/";
    path[..prefix.len()].copy_from_slice(prefix);

    let name_bytes = name.as_bytes();
    let len = name_bytes.len().min(path.len() - prefix.len());
    path[prefix.len()..prefix.len() + len].copy_from_slice(&name_bytes[..len]);

    if let Ok(path) = core::str::from_utf8(&path[..prefix.len() + len]) {
        crate::fs::vfs::register_device_node(path);
    }
}
