#![no_std]
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use dunit::{
    args, env, errno, fs,
    io::{BufReader, Read, Write},
    process, system,
    time::{self, Duration, Instant},
    Error,
};

#[dunit::main]
fn main() -> dunit::Result<()> {
    let arguments = args::args()?;
    if arguments.get(1).map(|arg| arg.as_str()) == Some("--panic") {
        panic!("SDK entry probe");
    }
    assert!(arguments[0].ends_with("sdk_api_test"));
    if arguments.get(1).map(|arg| arg.as_str()) == Some("--error") {
        return Err(Error(errno::ENOENT));
    }
    assert_eq!(arguments.get(1).map(|arg| arg.as_str()), Some("--probe"));
    // Kernel-terminal exec supplies an empty environment in this fixture.
    assert_eq!(env::var("PATH")?, None);
    assert_eq!(env::var("DUNIT_SDK_MISSING_VALUE")?, None);
    assert_eq!(env::var("bad=key"), Err(Error(errno::EINVAL)));
    dunit::println!("[SDKAPI] entry args env OK")?;
    let original = fs::current_dir()?;
    fs::set_current_dir("/tmp")?;
    assert_eq!(fs::current_dir()?, "/tmp");
    let handles = system::stats()?.fs_open_handles;
    let text = "первая\n\nlast\ntrailer";
    fs::write("sdk-api.txt", text.as_bytes())?;
    assert_eq!(fs::read_to_string("sdk-api.txt")?, text);
    {
        let mut reader = BufReader::new(fs::File::open("sdk-api.txt")?);
        for expected in ["первая", "", "last", "trailer"] {
            assert_eq!(reader.read_line()?.as_deref(), Some(expected));
        }
        assert_eq!(reader.read_line()?, None);
    }
    {
        let mut file = fs::OpenOptions::new().append(true).open("sdk-api.txt")?;
        file.write_all(b"!")?;
        file.sync_all()?;
        assert_eq!(file.metadata()?.len, text.len() as u64 + 1);
        file.close()?;
    }
    {
        let mut file = fs::File::open("sdk-api.txt")?;
        assert_eq!(file.seek(fs::SeekFrom::End(-1))?, text.len() as u64);
        let mut last = Vec::new();
        assert_eq!(file.read_to_end(&mut last)?, 1);
        assert_eq!(last, b"!");
    }
    // Both read_exact and write_all must cross the native 64-KiB copy cap.
    let bulk: Vec<u8> = (0..200_003).map(|i| (i % 251) as u8).collect();
    fs::write("sdk-api-bulk.bin", &bulk)?;
    let mut copy = alloc::vec![0; bulk.len()];
    {
        let mut file = fs::File::open("sdk-api-bulk.bin")?;
        file.read_exact(&mut copy)?;
        assert_eq!(file.read(&mut [0])?, 0);
    }
    assert_eq!(copy, bulk);
    fs::remove_file("sdk-api-bulk.bin")?;
    dunit::println!("[SDKAPI] bulk exact read write OK")?;
    fs::rename("sdk-api.txt", "sdk-api-renamed.txt")?;
    assert_eq!(
        fs::metadata("sdk-api-renamed.txt")?.file_type,
        fs::FileType::File
    );
    fs::remove_file("sdk-api-renamed.txt")?;
    assert_eq!(fs::read("sdk-api-renamed.txt"), Err(Error(errno::ENOENT)));
    fs::write("sdk-api-binary.txt", &[0xff])?;
    assert_eq!(
        fs::read_to_string("sdk-api-binary.txt"),
        Err(Error(errno::EINVAL))
    );
    fs::remove_file("sdk-api-binary.txt")?;
    assert_eq!(system::stats()?.fs_open_handles, handles);
    fs::set_current_dir(&original)?;
    dunit::println!("[SDKAPI] files seek buffered UTF8 drop OK")?;
    let started = Instant::now()?;
    time::sleep(Duration::from_millis(30))?;
    assert!(started.elapsed()? >= Duration::from_millis(30));
    let deadline = Instant::now()?
        .checked_add(Duration::from_millis(20))
        .unwrap();
    let mut events = dunit::event::EventLoop::new(&[deadline.deadline(123)])?;
    let ready = events.wait(dunit::event::INFINITE)?;
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].token, 123);
    assert_eq!(ready[0].events, dunit::event::ELAPSED);
    dunit::println!("[SDKAPI] monotonic sleep deadline OK")?;
    let mut child = process::Child::spawn("exit_test")?;
    assert_ne!(child.id(), process::id());
    let status = child.wait()?;
    assert_eq!(status.code(), Some(7));
    assert!(!status.success());
    assert_eq!(child.wait()?, status);
    let mut child = process::Child::spawn("kill_target")?;
    assert_eq!(child.wait_timeout(Duration::from_millis(20))?, None);
    child.kill()?;
    assert_eq!(child.wait()?.code(), Some(-9));
    assert!(child.try_wait()?.is_some());
    dunit::println!("[SDKAPI] children timeout reap cache OK")?;
    dunit::println!("[SDKAPI] OK")?;
    Ok(())
}
