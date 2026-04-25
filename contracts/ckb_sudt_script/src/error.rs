use ckb_std::error::SysError;

#[cfg(test)]
extern crate alloc;

#[repr(i8)]
pub enum Error {
    IndexOutOfBound = 1,
    ItemMissing,
    LengthNotEnough,
    Encoding,
    InvalidFd,
    WaitFailure,
    OtherEndClosed,
    MaxVmsSpawned,
    MaxFdsCreated,
    // Add customized errors here...
    ArgsLength = 10,
    Overflow = 11,
    OutputOverflow = 12
}

impl From<SysError> for Error {
    fn from(err: SysError) -> Self {
        match err {
            SysError::IndexOutOfBound => Self::IndexOutOfBound,
            SysError::ItemMissing => Self::ItemMissing,
            SysError::LengthNotEnough(_) => Self::LengthNotEnough,
            SysError::Encoding => Self::Encoding,
            SysError::Unknown(err_code) => panic!("unexpected sys error {}", err_code),
            // _ => panic!("Unknown error")
            SysError::InvalidFd => Self::InvalidFd,
            SysError::MaxFdsCreated => Self::MaxFdsCreated,
            SysError::MaxVmsSpawned => Self::MaxVmsSpawned,
            SysError::OtherEndClosed => Self::OtherEndClosed,
            SysError::WaitFailure => Self::WaitFailure,
        }
    }
}