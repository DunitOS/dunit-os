#![no_std]
#![no_main]
extern crate alloc;
mod app;

#[dunit::main]
fn main() -> dunit::Result<()> {
    let arguments = dunit::args::args()?;
    let mut console = app::Console::new(dunit::io::stdin(), dunit::io::stdout());
    app::run(&mut console, arguments.get(1..).unwrap_or(&[])).map_err(|error| match error {
        app::AppError::Io(error) => error,
        app::AppError::Input(error) => {
            dunit::println!("setup: input error {}", error.raw_os_error())
                .err()
                .unwrap_or(error)
        }
        app::AppError::Command(_) => dunit::Error(dunit::errno::EINVAL),
    })
}
