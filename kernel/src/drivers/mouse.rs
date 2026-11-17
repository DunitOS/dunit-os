//! IRQ-driven PS/2 mouse. Packet assembly and its negotiated format share a lock.
#[path = "ps2_packet.rs"]
mod ps2_packet;
use crate::sync::IrqSafeSpinLock;

static PACKET: IrqSafeSpinLock<ps2_packet::Decoder> =
    IrqSafeSpinLock::new(ps2_packet::Decoder::new());

pub fn init() {
    unsafe {
        wait_write();
        outb(0x64, 0xA8);

        wait_write();
        outb(0x64, 0x20);
        wait_read();
        let mut status = inb(0x60);
        status |= 0x02;
        status &= !0x20;

        wait_write();
        outb(0x64, 0x60);
        wait_write();
        outb(0x60, status);

        mouse_write(0xF6);
        let _ = mouse_read_ack();

        mouse_set_sample_rate(200);
        mouse_set_sample_rate(100);
        mouse_set_sample_rate(80);
        mouse_write(0xF2);
        let _ = mouse_read_ack();
        let device_id = mouse_read_ack();
        mouse_write(0xF4);
        let enabled = mouse_read_ack();
        crate::input::set_mouse_relative_source(enabled==0xFA);

        drain_output();
        PACKET.lock().reset(device_id == 3);
    }
}

pub fn push_packet_byte(byte: u8) {
    let mut decoder = PACKET.lock();
    if let Some(packet) = decoder.push(byte) {
        crate::input::push_mouse_relative(packet.dx, packet.dy, packet.buttons, packet.wheel);
    }
}

unsafe fn inb(port: u16) -> u8 {
    let result: u8;
    core::arch::asm!("in al, dx", out("al") result, in("dx") port, options(nomem, nostack));
    result
}

unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack));
}

unsafe fn wait_read() {
    for _ in 0..100000 {
        if (inb(0x64) & 0x01) != 0 {
            break;
        }
        core::arch::asm!("pause");
    }
}

unsafe fn wait_write() {
    for _ in 0..100000 {
        if (inb(0x64) & 0x02) == 0 {
            break;
        }
        core::arch::asm!("pause");
    }
}

unsafe fn mouse_write(value: u8) {
    wait_write();
    outb(0x64, 0xD4);
    wait_write();
    outb(0x60, value);
}

unsafe fn mouse_set_sample_rate(rate: u8) {
    mouse_write(0xF3);
    let _ = mouse_read_ack();
    mouse_write(rate);
    let _ = mouse_read_ack();
}

unsafe fn mouse_read_ack() -> u8 {
    wait_read();
    inb(0x60)
}

unsafe fn drain_output() {
    for _ in 0..32 {
        if (inb(0x64) & 0x01) == 0 {
            break;
        }
        let _ = inb(0x60);
    }
}
