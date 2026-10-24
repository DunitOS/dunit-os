//! Compile with rustc --test; exercises the actual PTY table against process hooks.
extern crate alloc;
mod process {
    pub type ProcessId = u64;
    pub fn process_exists(pid: ProcessId) -> bool { pid == 1 || pid == 2 }
    pub fn wake_poll_waiters() {}
}
#[path = "../kernel/src/pty/mod.rs"] mod pty;

#[test]
fn endpoint_geometry_ownership_readiness_and_close_eof() {
    let id = pty::create(1);
    assert!(pty::attach_slave(id, 99).is_none());
    let endpoint = pty::attach_slave(id, 1).unwrap();
    assert_eq!(pty::slave_readiness(id), Some((false, false)));
    assert_eq!(pty::slave_dimensions(id), Some([0; 8]));
    let geometry = [24, 0, 80, 0, 0, 0, 0, 0];
    assert!(pty::set_slave_dimensions(id, geometry));
    assert_eq!(pty::slave_dimensions(id), Some(geometry));
    assert_eq!(pty::master_write(id, 99, b"wrong"), None);
    assert_eq!(pty::master_write(id, 1, b"hello"), Some(5));
    assert_eq!(pty::slave_pending_input(id), Some(5));
    assert_eq!(pty::slave_readiness(id), Some((true, false)));
    assert_eq!(pty::slave_write_stdout(id, b"out"), Some(Ok(3)));
    let mut output = [0; 16];
    assert_eq!(pty::master_read(id, 1, &mut output), Some((3, false)));
    assert_eq!(&output[..3], b"out");
    let large = [7u8; pty::PTY_RING_CAP + 1];
    assert_eq!(pty::slave_write_stdout(id, &large), Some(Ok(pty::PTY_RING_CAP)));
    assert_eq!(pty::slave_output_readiness(id), Some((false, false)));
    assert_eq!(pty::slave_write_stdout(id, b"overflow"), Some(Ok(0)));
    assert_eq!(pty::master_read(id, 1, &mut output), Some((16, false)));
    assert_eq!(pty::slave_output_readiness(id), Some((true, false)));
    assert!(!pty::close(id, 99));
    assert!(pty::close(id, 1));
    assert_eq!(pty::master_write(id, 1, b"late"), None);
    assert_eq!(pty::slave_write_stdout(id, b"late"), Some(Err(())));
    assert_eq!(pty::slave_readiness(id), Some((true, true)));
    assert!(matches!(pty::slave_read_stdin(id, &mut output), Some(pty::SlaveRead::Data(5))));
    assert_eq!(&output[..5], b"hello");
    assert!(matches!(pty::slave_read_stdin(id, &mut output), Some(pty::SlaveRead::Eof)));
    let inherited = endpoint.clone();
    drop(endpoint);
    assert_eq!(pty::slave_dimensions(id), Some(geometry));
    drop(inherited);
    assert_eq!(pty::master_read(id, 1, &mut output), None);
    pty::on_process_exit(1);
}
