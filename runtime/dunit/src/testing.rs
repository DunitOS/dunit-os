//! Native custom-test-framework runner. Applications declare #[dunit::test];
//! the compiler collects cases, and the SDK owns reporting and exit status.
use crate::{io::Write, Result};

pub trait Outcome {
    fn into_result(self) -> Result<()>;
}
impl Outcome for () {
    fn into_result(self) -> Result<()> {
        Ok(())
    }
}
impl Outcome for Result<()> {
    fn into_result(self) -> Result<()> {
        self
    }
}

pub trait Testable {
    fn run(&self) -> Result<()>;
}
impl<T: Fn() -> Result<()>> Testable for T {
    fn run(&self) -> Result<()> {
        self()
    }
}
fn line(arguments: core::fmt::Arguments<'_>) -> Result<()> {
    crate::io::stdout().write_all(alloc::format!("{}\n", arguments).as_bytes())
}
pub fn case(name: &str, test: impl FnOnce() -> Result<()>) -> Result<()> {
    line(format_args!("[SDKTEST] RUN {}", name))?;
    let result = test();
    match result {
        Ok(()) => line(format_args!("[SDKTEST] PASS {}", name)),
        Err(error) => {
            let _ = line(format_args!("[SDKTEST] FAIL {}: {}", name, error));
            Err(error)
        }
    }
}
pub fn runner(tests: &[&dyn Testable]) -> ! {
    if tests.is_empty() {
        let _ = line(format_args!("[SDKTEST] FAIL no tests discovered"));
        libdunit::exit(1);
    }
    for test in tests {
        if test.run().is_err() {
            libdunit::exit(1);
        }
    }
    match line(format_args!("[SDKTEST] OK count={}", tests.len())) {
        Ok(()) => libdunit::exit(0),
        Err(_) => libdunit::exit(1),
    }
}
/// # Safety
/// Same loader stack and one-time initialization contract as the normal entry.
pub unsafe fn start(
    argc: usize,
    argv: libdunit::RawArgv,
    envp: libdunit::RawEnvp,
    harness: fn(),
) -> ! {
    libdunit::init_runtime(argc, argv, envp);
    harness();
    let _ = line(format_args!("[SDKTEST] FAIL test harness returned"));
    libdunit::exit(1)
}
