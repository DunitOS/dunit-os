#![no_std]
#![no_main]

//! Тест таблицы хэндлов с правами (capabilities).
//!
//! Проверяет все шесть прав и инварианты capability:
//! - READ/WRITE на объекте памяти,
//! - MAP (отображение объекта в адресное пространство),
//! - SIGNAL через конечную точку + приём сигналов,
//! - DISPLAY_MASTER (эксклюзивный дисплей),
//! - TRANSFER (передача хэндла) и запрет передачи без права,
//! - сужение прав через dup и запрет их расширения,
//! - невалидность хэндла после close.

extern crate alloc;
mod file_caps;
use core::panic::PanicInfo;

use libdunit::{
    RIGHT_DISPLAY_MASTER, RIGHT_MAP, RIGHT_READ, RIGHT_TRANSFER, RIGHT_WRITE,
};

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

fn fail(reason: &str) -> ! {
    libdunit::print("handle_test: FAIL ");
    libdunit::println(reason);
    libdunit::exit(1)
}

#[no_mangle]
pub extern "C" fn _start(argc:usize,argv:*const *const u8,envp:*const *const u8) -> ! {
    libdunit::init_runtime(argc,argv,envp);
    if libdunit::arg(1)==Some("--signal-peer") {
        let mut received=0u64;
        for _ in 0..10000 {
            let value=libdunit::handle_take_signals();
            if value<0 {fail("signal peer take");}
            received=received.saturating_add(value as u64);
            if received==10 {
                libdunit::println("[HANDLE-SIGNAL] cross-task sum=10 OK");
                libdunit::exit(0);
            }
            if received>10 {fail("signal peer total");}
            libdunit::yield_now();
        }
        fail("signal peer timeout");
    }
    if libdunit::arg(1)==Some("--cap-peer"){file_caps::peer();}
    if libdunit::arg(1)==Some("--persist-prepare"){file_caps::persistent(false);}
    if libdunit::arg(1)==Some("--persist-verify"){file_caps::persistent(true);}
    libdunit::println("handle_test: start");

    // 1) Создание объекта памяти. Права по умолчанию READ|WRITE|MAP|TRANSFER.
    let hmem = libdunit::handle_create_memory(64);
    if hmem <= 0 {
        fail("create memory");
    }
    let hmem = hmem as u32;
    let expected = (RIGHT_READ | RIGHT_WRITE | RIGHT_MAP | RIGHT_TRANSFER) as isize;
    if libdunit::handle_rights(hmem) != expected {
        fail("memory rights");
    }

    // 2) WRITE: право есть.
    let payload = b"HELLO";
    if libdunit::handle_write(hmem, payload) != payload.len() as isize {
        fail("write");
    }

    // 3) READ: право есть, содержимое совпадает.
    let mut buf = [0u8; 5];
    if libdunit::handle_read(hmem, &mut buf) != payload.len() as isize {
        fail("read len");
    }
    if &buf != payload {
        fail("read content");
    }

    // 4) Сужение прав через dup до READ. Новый хэндл имеет ровно RIGHT_READ.
    let hro = libdunit::handle_dup(hmem, RIGHT_READ);
    if hro <= 0 {
        fail("dup narrow");
    }
    let hro = hro as u32;
    if libdunit::handle_rights(hro) != RIGHT_READ as isize {
        fail("narrowed rights");
    }

    // 5) WRITE без права WRITE -> отказано.
    if libdunit::handle_write(hro, payload) != libdunit::EPERM {
        fail("write should be denied");
    }

    // 6) Попытка РАСШИРИТЬ права через dup (READ -> READ|WRITE) -> отказано.
    if libdunit::handle_dup(hro, RIGHT_READ | RIGHT_WRITE) != libdunit::EPERM {
        fail("widen should be denied");
    }

    // 7) MAP: отображаем объект и проверяем содержимое по адресу.
    let mapped = libdunit::handle_map(hmem, 0, 64);
    if mapped <= 0 {
        fail("map");
    }
    let mapped_ptr = mapped as usize as *const u8;
    for (i, &b) in payload.iter().enumerate() {
        let got = unsafe { core::ptr::read_volatile(mapped_ptr.add(i)) };
        if got != b {
            fail("mapped content");
        }
    }
    // MAP на хэндле без права MAP (hro = только READ) -> отказано.
    if libdunit::handle_map(hro, 0, 64) != libdunit::EPERM {
        fail("map should be denied");
    }

    // 8) SIGNAL: конечная точка к самому себе, доставка суммируется.
    let pid = libdunit::get_pid();
    let ep = libdunit::handle_create_endpoint(pid);
    if ep <= 0 {
        fail("create endpoint");
    }
    let ep = ep as u32;
    if libdunit::handle_signal(ep, 7) != 0 {
        fail("signal 7");
    }
    if libdunit::handle_signal(ep, 3) != 0 {
        fail("signal 3");
    }
    if libdunit::handle_take_signals() != 10 {
        fail("take signals");
    }
    // Повторный приём -> 0 (счётчик обнулён).
    if libdunit::handle_take_signals() != 0 {
        fail("signals not cleared");
    }

    // 9) SIGNAL на объекте памяти (нет права SIGNAL) -> отказано.
    if libdunit::handle_signal(hmem, 1) != libdunit::EPERM {
        fail("signal should be denied");
    }

    // Signal an independent task, then verify a reaped target is rejected.
    let child=libdunit::process::spawn_args("/app/handle_test", &["/app/handle_test", "--signal-peer"]);
    if child<=0 {fail("signal peer spawn");}
    let remote=libdunit::handle_create_endpoint(child as u32);
    if remote<=0 {fail("signal peer endpoint");}
    let remote=remote as u32;
    if libdunit::handle_signal(remote,7)!=0 || libdunit::handle_signal(remote,3)!=0 {
        fail("cross-task signal");
    }
    let mut status=libdunit::WaitStatus::empty();
    let mut reaped=false;
    for _ in 0..10000 {
        let result=libdunit::wait(child as u32,&mut status);
        if result==child {
            if !status.exited() || status.code!=0 {fail("signal peer status");}
            reaped=true;
            break;
        }
        if result!=libdunit::EAGAIN {fail("signal peer wait");}
        libdunit::yield_now();
    }
    if !reaped {fail("signal peer join timeout");}
    if libdunit::handle_signal(remote,1)!=libdunit::ENOENT {fail("reaped signal target");}
    if libdunit::handle_close(remote)!=0 {fail("signal endpoint close");}
    libdunit::println("[HANDLE-SIGNAL] self/cross-task/rights/reaped target OK");

    // 10) DISPLAY_MASTER: захват эксклюзивного дисплея.
    let hd = libdunit::handle_display_acquire();
    if hd <= 0 {
        fail("display acquire");
    }
    let hd = hd as u32;
    if libdunit::handle_rights(hd) & RIGHT_DISPLAY_MASTER as isize == 0 {
        fail("display rights");
    }

    // 11) TRANSFER (себе): хэндл-источник становится невалидным, а объект
    // доступен под новым хэндлом с теми же данными.
    let moved = libdunit::handle_transfer(hmem, pid);
    if moved <= 0 {
        fail("transfer");
    }
    let moved = moved as u32;
    if libdunit::handle_rights(hmem) != libdunit::EBADF {
        fail("source still valid after transfer");
    }
    let mut buf2 = [0u8; 5];
    if libdunit::handle_read(moved, &mut buf2) != payload.len() as isize || &buf2 != payload {
        fail("transferred content");
    }

    // 12) TRANSFER без права TRANSFER (hro = только READ) -> отказано.
    if libdunit::handle_transfer(hro, pid) != libdunit::EPERM {
        fail("transfer should be denied");
    }

    // 13) CLOSE: после закрытия хэндл невалиден.
    if libdunit::handle_close(moved) != 0 {
        fail("close");
    }
    if libdunit::handle_rights(moved) != libdunit::EBADF {
        fail("handle valid after close");
    }

    file_caps::run();
    libdunit::println("handle_test: OK");
    libdunit::exit(0)
}
