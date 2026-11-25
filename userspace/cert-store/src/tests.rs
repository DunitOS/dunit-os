use super::*;

#[test]
fn dcrt_roundtrip_rotation_and_rollback_checks() {
    // Generated with OpenSSL: valid self-signed ECDSA P-256 CA certificate (2025..2035)
    // and valid second CA certificate for rotation tests.
    let ca1_pem = include_str!("../tests_ca1.pem");
    let ca2_pem = include_str!("../tests_ca2.pem");

    let b1 = TrustBundle::from_pem(1, 100, ca1_pem).unwrap();
    let blob1 = b1.encode();
    let mut store = ManagedStore::from_blob(&blob1).unwrap();
    assert_eq!(store.version(), 1);
    assert_eq!(store.anchors().len(), 1);

    // Corrupted checksum must fail cleanly.
    let mut bad = blob1.clone();
    bad[10] ^= 1;
    assert_eq!(TrustBundle::decode_verified(&bad).err(), Some(StoreError::ChecksumMismatch));

    // Rotating with stale version/generation must be rejected without modifying active store.
    let b_old = TrustBundle::from_pem(1, 100, ca2_pem).unwrap().encode();
    assert_eq!(store.rotate(&b_old), Err(StoreError::RollbackRejected));
    assert_eq!(store.version(), 1);

    // Valid rotation updates active to v2 and keeps v1 for rollback.
    let b2 = TrustBundle::from_pem(2, 200, ca2_pem).unwrap().encode();
    assert_eq!(store.rotate(&b2), Ok(2));
    assert_eq!(store.version(), 2);

    // Rollback restores v1 cleanly.
    assert_eq!(store.rollback(), Ok(1));
    assert_eq!(store.version(), 1);
    assert_eq!(store.rollback(), Err(StoreError::NoRollback));
}

