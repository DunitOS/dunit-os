//! Native checks of file authority and atomic IPC attachment ownership.
use super::fail;
use libdunit::{file_cap as cap, RIGHT_READ as R, RIGHT_TRANSFER as T, RIGHT_WRITE as W};
fn check(ok: bool, why: &str) {
    if !ok {
        fail(why);
    }
}
fn handle(n: isize) -> u32 {
    check(n > 0, "file cap create/open");
    n as u32
}
fn close(h: u32) {
    check(libdunit::handle_close(h) == 0, "file cap close");
}
fn make(parent: u32, name: &str, kind: u32) -> u32 {
    handle(cap::create_child(parent, name, kind, R | W | T))
}
fn stats() -> u64 {
    let mut stats = libdunit::SystemStats::default();
    check(
        libdunit::get_system_stats(&mut stats) == 0,
        "file cap stats",
    );
    stats.fs_open_handles
}
fn receive(bytes: &mut [u8]) -> (usize, u32, u32) {
    let mut sender = 0;
    let mut attached = 0;
    let n = libdunit::ipc_recv_handle_blocking(bytes, &mut sender, &mut attached, 3000);
    check(n >= 0, "attached receive deadline");
    (n as usize, sender, attached)
}
pub fn peer() -> ! {
    let parent = libdunit::arg(2)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(|| fail("cap peer parent"));
    let mut bytes = [0; 256];
    let mut sender = 0;
    let mut attached = 0;
    check(
        libdunit::ipc_recv_handle(&mut bytes, &mut sender, &mut attached) == libdunit::EAGAIN,
        "peer opt in",
    );
    check(libdunit::ipc_send(parent, b"ready") == 5, "peer ready");
    let (n, sender, file) = receive(&mut bytes);
    check(
        n == 32 && sender == parent && file != 0,
        "peer actual attachment",
    );
    check(
        u32::from_le_bytes(bytes[24..28].try_into().unwrap()) == file,
        "peer patched handle",
    );
    let (n, sender, attached) = receive(&mut bytes);
    check(
        n == 2 && sender == parent && attached == 0 && &bytes[..n] == b"go",
        "peer read after move barrier",
    );
    let mut data = [0; 8];
    check(
        cap::read_at(file, 0, &mut data) == 8 && &data == b"original",
        "peer anchored inode",
    );
    check(
        cap::write_at(file, 0, b"bad") == libdunit::EPERM,
        "peer readonly write",
    );
    check(cap::remove(file) == libdunit::EPERM, "peer readonly unlink");
    check(
        libdunit::handle_transfer(file, parent) == libdunit::EPERM,
        "peer no onward transfer",
    );
    close(file);
    // A wire field claiming a handle does not create an attachment. Parent must
    // receive attachment=0, even when the packet contains its live handle id.
    check(
        libdunit::ipc_send(parent, b"forged attachment bytes") == 23,
        "peer forged ordinary packet",
    );
    check(
        libdunit::ipc_send(parent, b"verified") == 8,
        "peer success ack",
    );
    libdunit::exit(0)
}
pub fn run() {
    let pid = libdunit::get_pid();
    let temp = handle(cap::open("/tmp", R | W | T));
    let root = make(temp, &alloc::format!("caps-{pid}"), 2);
    let dest = make(root, "dest", 2);
    let file = make(root, "source", 1);
    check(cap::write_at(file, 0, b"original") == 8, "file cap write");
    let read = handle(libdunit::handle_dup(file, R));
    let mut bytes = [0; 256];
    check(
        cap::write_at(read, 0, b"bad") == libdunit::EPERM,
        "attenuated write",
    );
    check(
        cap::open_child(read, "child", R) == libdunit::ENOTDIR,
        "file is not directory",
    );
    for path in ["..", "../dest", "/tmp", "a/../b", "a//b", "."] {
        check(
            cap::open_child(root, path, R) == libdunit::EINVAL,
            "directory confinement",
        );
    }
    let rodir = handle(libdunit::handle_dup(root, R));
    check(
        cap::open_child(rodir, "source", R | W) == libdunit::EPERM,
        "child cannot widen rights",
    );
    check(
        cap::create_child(rodir, "bad", 1, R) == libdunit::EPERM,
        "readonly directory create",
    );
    let mut stat = libdunit::FileStat::default();
    check(
        cap::stat(dest, &mut stat) == 0 && stat.file_type == libdunit::FILE_TYPE_DIRECTORY,
        "directory stat",
    );
    let mut packet = [0; 32];
    packet[..4].copy_from_slice(b"FCP1");
    check(
        libdunit::ipc_send_handle(pid, file, R, &packet, 24) == libdunit::EOPNOTSUPP,
        "legacy inbox not poisoned",
    );
    check(
        libdunit::syscall6(
            libdunit::SYSCALL_IPC_RECEIVE_HANDLE,
            bytes.as_mut_ptr() as usize,
            256,
            0,
            0,
            0,
            0,
        ) == libdunit::EFAULT,
        "invalid receive outputs",
    );
    check(
        libdunit::ipc_send_handle(pid, file, R, &packet, 24) == libdunit::EOPNOTSUPP,
        "invalid receive does not opt in",
    );
    let mut sender = 0;
    let mut attached = 0;
    check(
        libdunit::ipc_recv_handle(&mut bytes, &mut sender, &mut attached) == libdunit::EAGAIN,
        "opt in empty receive",
    );
    check(
        libdunit::ipc_send_handle(pid, file, R, &packet, 24) == 32,
        "atomic file attachment send",
    );
    check(
        libdunit::ipc_recv_from(&mut bytes, &mut sender) == libdunit::EOPNOTSUPP,
        "legacy receive preserves attachment",
    );
    check(
        libdunit::ipc_recv_handle(&mut bytes[..8], &mut sender, &mut attached)
            == libdunit::EMSGSIZE,
        "short receive preserves attachment",
    );
    let (n, sender, received) = receive(&mut bytes);
    check(
        n == 32
            && sender == pid
            && received != 0
            && u32::from_le_bytes(bytes[24..28].try_into().unwrap()) == received,
        "authenticated attachment",
    );
    check(
        libdunit::handle_rights(received) == R as isize,
        "received rights",
    );
    close(received);
    let mut count = 0;
    loop {
        let n = libdunit::ipc_send(pid, b"fill");
        if n == libdunit::ENOBUFS {
            break;
        }
        check(n == 4, "fill queue");
        count += 1;
        check(count <= 1024, "queue bounded");
    }
    let before = stats();
    let probe = handle(cap::open_child(root, "source", R | T));
    check(stats() == before + 1, "probe anchor count");
    for _ in 0..64 {
        check(
            libdunit::ipc_send_handle(pid, probe, R, &packet, 24) == libdunit::ENOBUFS,
            "queue full rollback",
        );
    }
    check(
        libdunit::syscall6(
            libdunit::SYSCALL_IPC_SEND_HANDLE,
            pid as usize,
            probe as usize,
            R as usize,
            0,
            32,
            24,
        ) == libdunit::EFAULT,
        "invalid send mapping rollback",
    );
    check(
        libdunit::ipc_send_handle(u32::MAX, probe, R, &packet, 24) == libdunit::ENOENT,
        "dead recipient rollback",
    );
    close(probe);
    check(stats() == before, "failed sends leak no anchor references");
    for _ in 0..count {
        let (n, sender, attached) = receive(&mut bytes);
        check(
            n == 4 && sender == pid && attached == 0,
            "drain ordinary queue",
        );
    }
    libdunit::println("[FILE-CAPS] rights/confinement/atomic-rollback ok");
    let parent = alloc::format!("{pid}");
    let child = libdunit::process::spawn_args(
        "/app/handle_test",
        &["/app/handle_test", "--cap-peer", &parent],
    );
    check(child > 0, "spawn cap peer");
    let child = child as u32;
    let (n, sender, attached) = receive(&mut bytes);
    check(
        sender == child && attached == 0 && &bytes[..n] == b"ready",
        "peer receive opt in ready",
    );
    check(
        libdunit::ipc_send_handle(child, file, R, &packet, 24) == 32,
        "cross process file grant",
    );
    check(
        cap::relation(file, file) == cap::Relation::Same as isize,
        "same inode relation",
    );
    check(
        cap::relation(root, dest) == cap::Relation::SourceAncestor as isize,
        "directory ancestry relation",
    );
    check(cap::move_to(file, dest, "moved") == 0, "anchored move");
    check(
        cap::relation(file, dest) == cap::Relation::DestinationAncestor as isize,
        "ancestry follows moved inode",
    );
    check(
        cap::open_child(root, "source", R) == libdunit::ENOENT,
        "move removed old name",
    );
    check(libdunit::ipc_send(child, b"go") == 2, "move read barrier");
    let (n, sender, attached) = receive(&mut bytes);
    check(
        sender == child && attached == 0 && &bytes[..n] == b"forged attachment bytes",
        "wire forgery cannot fabricate attachment",
    );
    check(
        libdunit::handle_rights(file) == (R | W | T) as isize,
        "forgery leaves caller authority intact",
    );
    let (n, sender, attached) = receive(&mut bytes);
    check(
        sender == child && attached == 0 && &bytes[..n] == b"verified",
        "peer actual outcomes",
    );
    let mut status = libdunit::WaitStatus::empty();
    check(
        libdunit::wait(child, &mut status) == child as isize && status.exited() && status.code == 0,
        "peer exit",
    );
    let replacement = make(root, "source", 1);
    check(
        cap::write_at(replacement, 0, b"new") == 3,
        "replacement write",
    );
    let mut data = [0; 8];
    check(
        cap::read_at(read, 0, &mut data) == 8 && &data == b"original",
        "old name cannot retarget cap",
    );
    check(
        cap::move_to(root, dest, "cycle") == libdunit::EINVAL,
        "reject directory cycle",
    );
    for i in 0..100 {
        let h = make(dest, &alloc::format!("item-{i}"), 1);
        close(h);
    }
    check(cap::readdir(dest, &mut []) == 101, "true readdir count");
    let mut entries = [libdunit::DirEntry::empty(); 4];
    check(
        cap::readdir(dest, &mut entries) == 101,
        "bounded output true count",
    );
    for i in 0..100 {
        let h = handle(cap::open_child(dest, &alloc::format!("item-{i}"), R | W));
        check(cap::remove(h) == 0, "remove test child");
        close(h);
    }
    recursive_copy(root, dest, pid);
    check(cap::remove(file) == 0, "remove original");
    check(
        cap::read_at(read, 0, &mut data) == 8 && &data == b"original",
        "unlinked retained inode",
    );
    {
        use dunit_transfer::{file_ops, File, FileKind, OwnedHandle};
        let source = File {
            handle: unsafe { OwnedHandle::adopt(handle(libdunit::handle_dup(read, R))) },
            path: alloc::format!("/tmp/caps-{pid}/old-display-name"),
            kind: FileKind::File,
        };
        let target = unsafe { OwnedHandle::adopt(handle(libdunit::handle_dup(dest, R | W))) };
        check(
            file_ops::copy_into(&source, &target, "unlinked-copy").is_ok(),
            "local copy needs READ, not onward TRANSFER or a linked path",
        );
        let copied = handle(cap::open_child(dest, "unlinked-copy", R | W));
        check(
            cap::read_at(copied, 0, &mut data) == 8 && data == *b"original",
            "unlinked copy preserves original bytes",
        );
        check(cap::remove(copied) == 0, "unlinked copy cleanup");
        close(copied);
    }
    check(
        cap::remove(file) == libdunit::ENOENT,
        "removed cap cannot delete replacement",
    );
    check(cap::remove(replacement) == 0, "remove replacement");
    close(replacement);
    close(file);
    close(read);
    close(rodir);
    check(cap::remove(dest) == 0, "remove empty dest");
    close(dest);
    check(cap::remove(root) == 0, "remove own root");
    close(root);
    close(temp);
    libdunit::println("[FILE-CAPS] cross-process/identity/move/readdir ok");
}

fn recursive_copy(root: u32, dest: u32, pid: u32) {
    use dunit_transfer::{file_ops, File, FileKind, OwnedHandle};
    let source = make(root, "copy-source", 2);
    let nested = make(source, "nested", 2);
    let contents = make(nested, "content", 1);
    check(
        cap::write_at(contents, 0, "данные".as_bytes()) == 12,
        "recursive source bytes",
    );
    check(cap::sync(contents) == 0, "explicit capability fsync");
    let source_file = File {
        handle: unsafe { OwnedHandle::adopt(handle(libdunit::handle_dup(source, R | T))) },
        path: alloc::format!("/tmp/caps-{pid}/copy-source"),
        kind: FileKind::Directory,
    };
    let destination = unsafe { OwnedHandle::adopt(handle(libdunit::handle_dup(dest, R | W | T))) };
    check(
        file_ops::copy_into(&source_file, &destination, "copied").is_ok(),
        "recursive capability copy",
    );
    let copied = handle(cap::open_child(dest, "copied", R | W | T));
    let copy_nested = handle(cap::open_child(copied, "nested", R | W | T));
    let copy_data = handle(cap::open_child(copy_nested, "content", R | W | T));
    let mut bytes = [0; 12];
    check(
        cap::read_at(copy_data, 0, &mut bytes) == 12 && bytes == "данные".as_bytes(),
        "actual recursive copy result",
    );
    let error = file_ops::copy_into(&source_file, &destination, "copied").unwrap_err();
    check(
        error.operation.0 == libdunit::EEXIST && error.cleanup.is_none(),
        "no overwrite in recursive copy",
    );
    check(
        cap::read_at(copy_data, 0, &mut bytes) == 12 && bytes == "данные".as_bytes(),
        "existing copy unchanged",
    );
    let inside = unsafe { OwnedHandle::adopt(handle(libdunit::handle_dup(nested, R | W | T))) };
    check(
        file_ops::copy_into(&source_file, &inside, "cycle")
            .unwrap_err()
            .operation
            .0
            == libdunit::EINVAL,
        "copy into actual subtree denied",
    );
    check(
        cap::open_child(nested, "cycle", R) == libdunit::ENOENT,
        "self-copy did not create nodes",
    );
    let mut rollback = file_ops::CopyJob::new(&source_file, &destination, "cancel-tree").unwrap();
    for _ in 0..6 {
        check(
            rollback.poll() == Ok(false),
            "nested copy yields before completion",
        );
    }
    rollback.request_cancel();
    check(
        rollback.poll() == Ok(false),
        "rollback yields after one node",
    );
    let pending = handle(cap::open_child(dest, "cancel-tree", R));
    close(pending);
    check(
        rollback.poll() == Ok(false),
        "rollback yields after directory child",
    );
    let error = rollback.poll().unwrap_err();
    check(
        error.operation.0 == libdunit::EINTR && error.cleanup.is_none(),
        "bounded rollback completes exactly",
    );
    check(
        cap::open_child(dest, "cancel-tree", R) == libdunit::ENOENT,
        "bounded rollback removes own root",
    );
    check(
        cap::read_at(contents, 0, &mut bytes) == 12 && bytes == "данные".as_bytes(),
        "bounded rollback preserves original source",
    );
    let large = make(root, "chunk-source", 1);
    let block = [0x5a; 12288];
    check(
        cap::write_at(large, 0, &block) == block.len() as isize,
        "large copy source",
    );
    let large_file = File {
        handle: unsafe { OwnedHandle::adopt(handle(libdunit::handle_dup(large, R | T))) },
        path: alloc::format!("/tmp/caps-{pid}/chunk-source"),
        kind: FileKind::File,
    };
    let mut job = file_ops::CopyJob::new(&large_file, &destination, "cancelled").unwrap();
    check(job.poll() == Ok(false), "copy node step yields");
    check(job.poll() == Ok(false), "copy chunk yields");
    let partial = handle(cap::open_child(dest, "cancelled", R));
    let mut stat = libdunit::FileStat::default();
    check(
        cap::stat(partial, &mut stat) == 0 && stat.size == 4096,
        "one poll writes at most 4096 bytes",
    );
    close(partial);
    let error = job.cancel().unwrap_err();
    check(
        error.operation.0 == libdunit::EINTR && error.cleanup.is_none(),
        "copy cancellation reports cleanup",
    );
    check(
        cap::open_child(dest, "cancelled", R) == libdunit::ENOENT,
        "cancel removes partial destination",
    );
    check(
        cap::read_at(large, 0, &mut [0; 1]) == 1,
        "cancel preserves source",
    );
    check(cap::remove(large) == 0, "large source cleanup");
    close(large);
    libdunit::println("[FILE-CAPS] bounded-copy/cancel/cleanup ok");
    let long_name = alloc::string::String::from_utf8(alloc::vec![b'x';65]).unwrap();
    let too_long = make(source, &long_name, 1);
    let error = file_ops::copy_into(&source_file, &destination, "failed").unwrap_err();
    check(
        error.operation.0 == libdunit::EFBIG && error.cleanup.is_none(),
        "directory listing failure is explicit",
    );
    check(
        cap::open_child(dest, "failed", R) == libdunit::ENOENT,
        "failed copy cleans only newly created nodes",
    );
    for file in [
        too_long,
        copy_data,
        copy_nested,
        copied,
        contents,
        nested,
        source,
    ] {
        check(cap::remove(file) == 0, "recursive fixture cleanup");
        close(file);
    }
    libdunit::println("[FILE-CAPS] recursive-copy/no-overwrite/self-copy/fsync ok");
}

/// Disk-backed authority uses the same syscalls, rather than a path proxy.
/// The harness attaches a private copy-on-write test disk; the second boot
/// verifies synchronized bytes and removes only this test's own namespace.
pub fn persistent(verify: bool) -> ! {
    use dunit_transfer::{file_ops, File, FileKind, OwnedHandle};
    let before = stats();
    let volume = handle(cap::open("/persist", R | W | T));
    if verify {
        let root = handle(cap::open_child(volume, "m77-cap-proof", R | W | T));
        let dest = handle(cap::open_child(root, "dest", R | W | T));
        let moved = handle(cap::open_child(dest, "moved", R | W | T));
        let copied = handle(cap::open_child(dest, "copied", R | W | T));
        let replacement = handle(cap::open_child(root, "source", R | W | T));
        let mut data = [0; 8];
        check(
            cap::read_at(moved, 0, &mut data) == 8 && data == *b"original",
            "persistent moved inode bytes",
        );
        let mut copied_bytes = [0; 9];
        check(
            cap::read_at(copied, 0, &mut copied_bytes) == 9 && copied_bytes == *b"crosscopy",
            "persistent cross-backend copy bytes",
        );
        let mut replaced = [0; 11];
        check(
            cap::read_at(replacement, 0, &mut replaced) == 11 && replaced == *b"replacement",
            "persistent replacement isolated",
        );
        for file in [moved, copied, replacement, dest, root] {
            check(cap::remove(file) == 0, "persistent fixture cleanup");
            close(file);
        }
        close(volume);
        check(
            stats() == before,
            "persistent verification closes all descriptions",
        );
        libdunit::println("[FILE-CAPS] DunitFS reboot/copy/identity verified");
        libdunit::exit(0)
    }
    let root = make(volume, "m77-cap-proof", 2);
    let dest = make(root, "dest", 2);
    let file = make(root, "source", 1);
    check(
        cap::write_at(file, 0, b"original") == 8,
        "persistent source write",
    );
    let read = handle(libdunit::handle_dup(file, R | T));
    check(
        cap::move_to(file, dest, "moved") == 0,
        "persistent actual inode move",
    );
    let replacement = make(root, "source", 1);
    check(
        cap::write_at(replacement, 0, b"replacement") == 11,
        "persistent replacement write",
    );
    let mut data = [0; 8];
    check(
        cap::read_at(read, 0, &mut data) == 8 && data == *b"original",
        "persistent held cap does not retarget",
    );
    check(
        cap::write_at(read, 0, b"bad") == libdunit::EPERM,
        "persistent readonly write denied",
    );
    check(
        cap::open_child(root, "../dest", R) == libdunit::EINVAL,
        "persistent child traversal denied",
    );
    let memory = handle(cap::open("/tmp", R | W | T));
    check(
        cap::move_to(file, memory, "m77-cross-move") == libdunit::EOPNOTSUPP,
        "cross-filesystem move is explicit unsupported",
    );
    check(
        cap::read_at(file, 0, &mut data) == 8 && data == *b"original",
        "failed cross-filesystem move preserves source",
    );
    let source = make(memory, "m77-cross-copy", 1);
    check(
        cap::write_at(source, 0, b"crosscopy") == 9,
        "cross backend copy source",
    );
    {
        let source = File {
            handle: unsafe { OwnedHandle::adopt(handle(libdunit::handle_dup(source, R | T))) },
            path: "/tmp/m77-cross-copy".into(),
            kind: FileKind::File,
        };
        let target = unsafe { OwnedHandle::adopt(handle(libdunit::handle_dup(dest, R | W | T))) };
        check(
            file_ops::copy_into(&source, &target, "copied").is_ok(),
            "actual MemFS to DunitFS copy",
        );
    }
    let mut entries = [libdunit::DirEntry::empty(); 1];
    check(
        cap::readdir(dest, &mut entries) == 2,
        "persistent true directory count",
    );
    check(
        cap::relation(file, dest) == libdunit::file_cap::Relation::DestinationAncestor as isize,
        "persistent current inode ancestry",
    );
    for file in [file, replacement, dest, root] {
        check(cap::sync(file) == 0, "persistent explicit synchronization");
    }
    check(cap::remove(source) == 0, "cross copy memory cleanup");
    for file in [source, memory, read, file, replacement, dest, root, volume] {
        close(file);
    }
    check(
        stats() == before,
        "persistent producer closes all descriptions",
    );
    libdunit::println("[FILE-CAPS] DunitFS inode/copy/cross-fs/sync ok");
    libdunit::exit(0)
}
