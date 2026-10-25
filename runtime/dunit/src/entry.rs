//! Implementation details used by the generated loader entry and panic handler.
use core::fmt::Write;

// Keep one bounded diagnostic intact in the serial stream. No allocation and
// no waiting: entry failure must terminate even if a terminal is full or gone.
struct Diagnostic {
    bytes: [u8; 1024],
    length: usize,
}
impl core::fmt::Write for Diagnostic {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let mut length = text.len().min(self.bytes.len() - self.length);
        while !text.is_char_boundary(length) {
            length -= 1;
        }
        self.bytes[self.length..self.length + length].copy_from_slice(&text.as_bytes()[..length]);
        self.length += length;
        if length == text.len() {
            Ok(())
        } else {
            Err(core::fmt::Error)
        }
    }
}
fn report(arguments: core::fmt::Arguments<'_>) {
    let mut diagnostic = Diagnostic {
        bytes: [0; 1024],
        length: 0,
    };
    let _ = diagnostic.write_fmt(arguments);
    libdunit::write(2, &diagnostic.bytes[..diagnostic.length]);
}

/// # Safety
/// The pointers must be the process's valid loader-provided argv/envp arrays,
/// remain mapped for the process lifetime and be initialized once before threads.
pub unsafe fn start(
    argc: usize,
    argv: libdunit::RawArgv,
    envp: libdunit::RawEnvp,
    main: fn() -> crate::Result<()>,
) -> ! {
    libdunit::init_runtime(argc, argv, envp);
    match main() {
        Ok(()) => libdunit::exit(0),
        Err(error) => {
            report(format_args!("application failed: {}\n", error));
            libdunit::exit(1)
        }
    }
}
pub fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    report(format_args!("[SDK] PANIC: {}\n", info));
    libdunit::exit(101)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entry_diagnostic_is_contiguous_bounded_utf8_without_allocation() {
        let mut diagnostic = Diagnostic {
            bytes: [0; 1024],
            length: 0,
        };
        diagnostic
            .write_fmt(format_args!(
                "application failed: {}\n",
                crate::Error(libdunit::ENOENT)
            ))
            .unwrap();
        assert_eq!(
            &diagnostic.bytes[..diagnostic.length],
            b"application failed: ENOENT\n"
        );
        diagnostic.length = 1023;
        assert!(diagnostic.write_str("я").is_err());
        assert_eq!(diagnostic.length, 1023);
        diagnostic.write_str("x").unwrap();
        assert_eq!(diagnostic.length, 1024);
        assert!(diagnostic.write_str("x").is_err());
    }
}
