#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum ExitCode {
    Ok = 0,
    Failed = 1,
    ValidationFailed = 2,
    ConfigError = 3,
    RuntimeError = 4,
    NotFound = 5,
}

impl ExitCode {
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}
