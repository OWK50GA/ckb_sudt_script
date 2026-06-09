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
    // Custom errors
    InvalidArgsLength = 10,
    SignatureInvalid = 11,
    TimelockNotMet = 12,
    MissingWitness = 13,
}

impl From<SysError> for Error {
    fn from(err: SysError) -> Self {
        match err {
            SysError::IndexOutOfBound => Self::IndexOutOfBound,
            SysError::ItemMissing => Self::ItemMissing,
            SysError::LengthNotEnough(_) => Self::LengthNotEnough,
            SysError::Encoding => Self::Encoding,
            SysError::InvalidFd => Self::InvalidFd,
            SysError::WaitFailure => Self::WaitFailure,
            SysError::OtherEndClosed => Self::OtherEndClosed,
            SysError::MaxVmsSpawned => Self::MaxVmsSpawned,
            SysError::MaxFdsCreated => Self::MaxFdsCreated,
            SysError::Unknown(code) => panic!("unexpected sys error {}", code),
        }
    }
}
