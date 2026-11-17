//! Verify session policy without writing a guest filesystem.
extern crate alloc;
use alloc::vec::Vec;
use std::sync::Mutex;

mod settings {
    use super::*;
    pub const MAX_SESSION: usize = 32;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Id(pub &'static str);
    impl Id { pub fn as_str(&self) -> &str { self.0 } }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct WinGeom { pub id: Id, pub x: i32 }
    pub static STORE: Mutex<Vec<WinGeom>> = Mutex::new(Vec::new());
    pub static WRITES: Mutex<usize> = Mutex::new(0);
    pub fn load_session() -> Vec<WinGeom> { STORE.lock().unwrap().clone() }
    pub fn save_session(entries: &[WinGeom]) -> bool {
        *WRITES.lock().unwrap() += 1;
        *STORE.lock().unwrap() = entries.to_vec();
        true
    }
}
mod libdunit { pub fn println(_: &str) {} }
#[path = "../userspace/system_apps/gui_server/src/window_session.rs"]
mod window_session;

#[test]
fn disabling_restore_never_reopens_or_overwrites_the_previous_session() {
    use settings::{Id, WinGeom};
    let previous = WinGeom { id: Id("calculator"), x: 100 };
    let moved = WinGeom { id: Id("calculator"), x: 400 };
    *settings::STORE.lock().unwrap() = vec![previous];
    let mut session = window_session::Session::new(false);
    session.remember(moved);
    session.persist();
    assert_eq!(session.lookup("calculator"), None);
    assert_eq!(session.len(), 0);
    assert!(!session.save());
    assert_eq!(*settings::WRITES.lock().unwrap(), 0);
    assert_eq!(*settings::STORE.lock().unwrap(), vec![previous]);

    session.set_restore(true);
    assert_eq!(session.lookup("calculator"), Some(previous));
    session.remember(moved);
    session.set_restore(true); // A normal reload retains unsaved live placement.
    assert_eq!(session.entries(), &[moved]);
    session.persist();
    assert_eq!(*settings::STORE.lock().unwrap(), vec![moved]);
    assert_eq!(*settings::WRITES.lock().unwrap(), 1);

    session.set_restore(false);
    session.remember(previous);
    session.persist();
    assert_eq!(session.lookup("calculator"), None);
    assert_eq!(*settings::STORE.lock().unwrap(), vec![moved]);
    assert_eq!(*settings::WRITES.lock().unwrap(), 1);
    session.set_restore(true);
    assert_eq!(session.lookup("calculator"), Some(moved));
}
