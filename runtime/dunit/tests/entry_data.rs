//! This is the only host test that initializes the process-entry globals.
//! No native syscall is executed, and pointers remain live until reset below.
#[test]
fn initialized_arguments_and_environment_are_owned_and_validated() {
    let argv = [b"probe\0".as_ptr(), "я\0".as_ptr(), core::ptr::null()];
    let envp = [
        b"KEY=value=with=equals\0".as_ptr(),
        b"EMPTY=\0".as_ptr(),
        core::ptr::null(),
    ];
    libdunit::init_runtime(2, argv.as_ptr(), envp.as_ptr());
    let args = dunit::args::args().unwrap();
    let value = dunit::env::var("KEY").unwrap();
    assert_eq!(args, ["probe", "я"]);
    assert_eq!(value.as_deref(), Some("value=with=equals"));
    assert_eq!(dunit::env::var("EMPTY").unwrap().as_deref(), Some(""));
    assert_eq!(dunit::env::var("MISSING").unwrap(), None);
    assert_eq!(
        dunit::env::var("bad=key"),
        Err(dunit::Error(dunit::errno::EINVAL))
    );
    assert_eq!(
        dunit::env::var("bad\0key"),
        Err(dunit::Error(dunit::errno::EINVAL))
    );
    let invalid = [b"\xff\0".as_ptr(), core::ptr::null()];
    libdunit::init_runtime(1, invalid.as_ptr(), envp.as_ptr());
    assert_eq!(dunit::args::args(), Err(dunit::Error(dunit::errno::EINVAL)));
    libdunit::init_runtime(0, core::ptr::null(), core::ptr::null());
    assert_eq!(args, ["probe", "я"]);
    assert_eq!(value.as_deref(), Some("value=with=equals"));
}
