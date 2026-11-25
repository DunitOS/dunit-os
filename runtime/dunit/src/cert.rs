//! Read-only versioned X.509 trust store bindings.
use crate::{fs, Result};
pub use dunit_cert_store::{parse_pem_certs, ManagedStore, StoreError, TrustBundle};
pub use dunit_tls_core::Certificate;

const DEFAULT_STORE_PATH: &str = "/system/etc/ssl/roots.dcrt";
const MAX_STORE_BYTES: usize = 512 * 1024;

pub fn load_store_blob(path: &str) -> Result<ManagedStore> {
    let bytes = fs::read_bounded(path, MAX_STORE_BYTES)?;
    ManagedStore::from_blob(&bytes).map_err(|_| crate::Error(libdunit::EINVAL))
}

pub fn load_default_roots() -> alloc::vec::Vec<Certificate> {
    match load_store_blob(DEFAULT_STORE_PATH) {
        Ok(store) => store.anchors().to_vec(),
        Err(_) => alloc::vec::Vec::new(),
    }
}
