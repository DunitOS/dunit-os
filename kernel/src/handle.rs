//! Таблица хэндлов ядра с правами (capabilities).
//!
//! Каждый процесс владеет своей [`HandleTable`]: отображение непрозрачного
//! числового `Handle` в [`HandleEntry`] — объект ядра плюс битовая маска прав.
//! Операция над хэндлом разрешена только если в маске взведён нужный бит; права
//! можно лишь СУЖАТЬ (при `dup`/`transfer`), но никогда не расширять — это и есть
//! неподделываемость capability.
//!
//! Права намеренно разнородны и покрывают разные типы объектов:
//! - `READ`/`WRITE`/`MAP` — объект памяти [`HandleObject::Memory`];
//! - `SIGNAL` — конечная точка [`HandleObject::Endpoint`] (сигнал процессу);
//! - `DISPLAY_MASTER` — эксклюзивное владение фреймбуфером [`HandleObject::Display`];
//! - `INPUT_MASTER` — эксклюзивный источник ввода (клавиатура/мышь) [`HandleObject::Input`];
//! - `TRANSFER` — можно ли передать хэндл другому процессу.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::process::ProcessId;

pub const RIGHT_READ: u32 = 1 << 0;
pub const RIGHT_WRITE: u32 = 1 << 1;
pub const RIGHT_MAP: u32 = 1 << 2;
pub const RIGHT_SIGNAL: u32 = 1 << 3;
pub const RIGHT_TRANSFER: u32 = 1 << 4;
pub const RIGHT_DISPLAY_MASTER: u32 = 1 << 5;
pub const RIGHT_INPUT_MASTER: u32 = 1 << 6;
pub const RIGHT_NET_CONTROL: u32 = 1 << 7;
pub const RIGHT_NET_RAW: u32 = 1 << 8;
pub const RIGHT_NET_CONNECT: u32 = 1 << 9;
pub const RIGHT_NET_SERVICE: u32 = 1 << 10;

pub const RIGHTS_ALL: u32 = RIGHT_READ
    | RIGHT_WRITE
    | RIGHT_MAP
    | RIGHT_SIGNAL
    | RIGHT_TRANSFER
    | RIGHT_DISPLAY_MASTER
    | RIGHT_INPUT_MASTER
    | RIGHT_NET_CONTROL
    | RIGHT_NET_RAW
    | RIGHT_NET_CONNECT
    | RIGHT_NET_SERVICE;

/// Верхняя граница размера объекта памяти за одним хэндлом (защита от исчерпания
/// кучи по запросу из userspace).
pub const MAX_MEMORY_OBJECT: usize = 1 << 20; // 1 MiB

pub type Handle = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleError {
    /// Хэндла нет в таблице вызывающего процесса.
    BadHandle,
    /// В маске прав нет нужного бита.
    AccessDenied,
    /// Тип объекта не поддерживает операцию.
    WrongType,
    /// Дисплеем уже владеет другой процесс.
    DisplayBusy,
    /// Слишком большой запрошенный объект памяти.
    TooLarge,
    /// Не удалось отобразить объект в адресное пространство.
    MapFailed,
    /// Процесс-получатель передачи/сигнала не найден.
    NoSuchTarget,
}

/// One process-owned grant. A device reference records the generation of the
/// queue opened through it so revoking an old grant cannot stop a new session.
pub struct NetDevice {
    pub index: u32,
    pub queue_generation: core::sync::atomic::AtomicU64,
}
impl NetDevice {
    pub fn new(index: u32) -> Self {
        Self { index, queue_generation: core::sync::atomic::AtomicU64::new(0) }
    }
}
impl Drop for NetDevice {
    fn drop(&mut self) {
        let generation = *self.queue_generation.get_mut();
        if generation != 0 {
            crate::drivers::net::packet::close(generation);
        }
    }
}

/// Объект ядра, на который ссылается хэндл.
pub enum HandleObject {
    NetRoot,
    NetDevice(Arc<NetDevice>),
    /// Nontransferable scope for outbound application sockets.
    NetConnect { scope: u32 },
    /// Actual filesystem open-description authority; transferable with attenuation.
    File(Arc<crate::fs::file_cap::FileCapability>),
    /// Буфер байтов в куче ядра. Права READ/WRITE/MAP.
    Memory(Vec<u8>),
    /// Ссылка на процесс-получатель сигналов. Право SIGNAL.
    Endpoint(ProcessId),
    /// Мастер-владение дисплеем (эксклюзивно на всю систему). Право DISPLAY_MASTER.
    Display,
    /// Мастер-источник ввода (эксклюзивно на всю систему). Право INPUT_MASTER.
    Input,
    /// Разделяемый буфер физических фреймов (zero-copy). `id` — ключ в глобальном
    /// реестре разделяемых объектов ядра; служит и идентичностью буфера (например,
    /// для запрета повторного импорта одного буфера в gui-v1). Права READ/WRITE/
    /// MAP/TRANSFER; учёт ссылок ведёт слой процесса при dup/close/teardown.
    SharedFrames { id: u64 },
}

impl HandleObject {
    /// Move-клон объекта при передаче хэндла: буфер памяти переносится целиком,
    /// остальные объекты копируются по значению.
    fn take(self) -> HandleObject {
        self
    }
}

pub struct HandleEntry {
    pub object: HandleObject,
    pub rights: u32,
}

pub struct HandleTable {
    entries: BTreeMap<Handle, HandleEntry>,
    next: Handle,
}

impl HandleTable {
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            next: 1,
        }
    }

    fn allocate_id(&mut self) -> Handle {
        // Простой монотонный счётчик; переполнение u32 нереалистично для одного
        // процесса, но на всякий случай пропускаем 0 и занятые id.
        loop {
            let id = self.next;
            self.next = self.next.wrapping_add(1);
            if id != 0 && !self.entries.contains_key(&id) {
                return id;
            }
        }
    }

    /// Публикует объект с заданными правами и возвращает свежий хэндл.
    pub fn insert(&mut self, object: HandleObject, rights: u32) -> Handle {
        let id = self.allocate_id();
        self.entries.insert(
            id,
            HandleEntry {
                object,
                rights: rights & RIGHTS_ALL,
            },
        );
        id
    }

    pub fn get(&self, handle: Handle) -> Result<&HandleEntry, HandleError> {
        self.entries.get(&handle).ok_or(HandleError::BadHandle)
    }

    pub fn get_mut(&mut self, handle: Handle) -> Result<&mut HandleEntry, HandleError> {
        self.entries.get_mut(&handle).ok_or(HandleError::BadHandle)
    }

    pub fn remove(&mut self, handle: Handle) -> Result<HandleEntry, HandleError> {
        self.entries.remove(&handle).ok_or(HandleError::BadHandle)
    }

    pub fn rights(&self, handle: Handle) -> Result<u32, HandleError> {
        self.get(handle).map(|entry| entry.rights)
    }

    /// Проверяет, что хэндл существует и обладает ВСЕМИ битами `required`.
    pub fn require(&self, handle: Handle, required: u32) -> Result<&HandleEntry, HandleError> {
        let entry = self.get(handle)?;
        if entry.rights & required != required {
            return Err(HandleError::AccessDenied);
        }
        Ok(entry)
    }

    pub fn require_mut(
        &mut self,
        handle: Handle,
        required: u32,
    ) -> Result<&mut HandleEntry, HandleError> {
        let entry = self.get_mut(handle)?;
        if entry.rights & required != required {
            return Err(HandleError::AccessDenied);
        }
        Ok(entry)
    }

    /// Дублирует хэндл с сужением прав. `new_rights` обязан быть подмножеством
    /// прав исходного хэндла — иначе `AccessDenied` (нельзя расширить права).
    pub fn duplicate(&mut self, handle: Handle, new_rights: u32) -> Result<Handle, HandleError> {
        let current = self.get(handle)?;
        let new_rights = new_rights & RIGHTS_ALL;
        if new_rights & !current.rights != 0 {
            return Err(HandleError::AccessDenied);
        }
        let object = match &current.object {
            HandleObject::NetRoot | HandleObject::NetConnect { .. } => {
                return Err(HandleError::AccessDenied);
            }
            HandleObject::NetDevice(device) => HandleObject::NetDevice(device.clone()),
            HandleObject::File(file) => HandleObject::File(Arc::clone(file)),
            HandleObject::Memory(data) => HandleObject::Memory(data.clone()),
            HandleObject::Endpoint(pid) => HandleObject::Endpoint(*pid),
            HandleObject::Display => HandleObject::Display,
            HandleObject::Input => HandleObject::Input,
            HandleObject::SharedFrames { id } => HandleObject::SharedFrames { id: *id },
        };
        Ok(self.insert(object, new_rights))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Забирает из таблицы объект+права для передачи в другую таблицу.
    pub fn take_for_transfer(&mut self, handle: Handle) -> Result<(HandleObject, u32), HandleError> {
        let entry = self.remove(handle)?;
        Ok((entry.object.take(), entry.rights))
    }

    /// Итерирует объекты для teardown (например, чтобы освободить дисплей).
    pub fn drain_objects(&mut self) -> Vec<HandleObject> {
        core::mem::take(&mut self.entries)
            .into_values()
            .map(|entry| entry.object)
            .collect()
    }
}

impl Default for HandleTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Display ownership is serialized with active scanout writes and modesets.
pub fn try_acquire_display(pid: ProcessId) -> bool {
    crate::drivers::display::try_acquire(pid.0)
}

pub fn release_display(pid: ProcessId) {
    crate::drivers::display::release(pid.0);
}

pub fn display_owner() -> u64 {
    crate::drivers::display::owner()
}

/// Every acquired input handle retains the exclusive device lease.
pub fn try_acquire_input(pid: ProcessId) -> bool {
    crate::input::try_acquire(pid.0)
}

/// Final release clears the old queue before another owner can acquire it.
pub fn release_input(pid: ProcessId) {
    crate::input::release(pid.0);
}
