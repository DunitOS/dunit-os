use core::fmt;

/// A negative native Dunit errno. No translation through Linux error tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(pub isize);
pub type Result<T> = core::result::Result<T, Error>;
impl Error {
    pub fn raw_os_error(self) -> isize {
        self.0
    }
    pub fn name(self) -> &'static str {
        libdunit::error_name(self.0)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}
impl core::error::Error for Error {}
pub(crate) fn checked(value: isize) -> Result<usize> {
    if value < 0 {
        Err(Error(value))
    } else {
        Ok(value as usize)
    }
}
