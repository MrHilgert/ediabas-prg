use std::io;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Protocol(String),
    Checksum { expected: u8, got: u8 },
    Timeout,
    EcuError(u8),
    Vm(String),
    Prg(String),
    NotSupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e)                       => write!(f, "I/O: {e}"),
            Error::Protocol(s)                 => write!(f, "Protocol: {s}"),
            Error::Checksum { expected, got }  => write!(f, "Checksum mismatch: expected {expected:#04x}, got {got:#04x}"),
            Error::Timeout                     => write!(f, "Timeout: no response from ECU"),
            Error::EcuError(code)              => write!(f, "ECU error: {code:#04x}"),
            Error::Vm(s)                       => write!(f, "VM: {s}"),
            Error::Prg(s)                      => write!(f, "PRG: {s}"),
            Error::NotSupported(s)             => write!(f, "Not supported: {s}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self { Error::Io(e) }
}
