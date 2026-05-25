#![cfg_attr(not(any(feature = "library", test)), no_std)]
#![cfg_attr(not(test), no_main)]

pub mod error;
pub mod witness;

use ckb_hash::blake2b_256;
use ckb_std::{ckb_constants::Source, ckb_types::{bytes::Bytes, prelude::*}};
use error::Error;

#[cfg(any(feature = "library", test))]
extern crate alloc;

#[cfg(not(any(feature = "library", test)))]
ckb_std::entry!(program_entry);
#[cfg(not(any(feature = "library", test)))]
ckb_std::default_alloc!(16384, 1258306, 64);

pub fn program_entry() -> i8 {
    match check_hash() {
        Ok(()) => 0,
        Err(e) => e as i8,
    }
}

fn check_hash() -> Result<(), Error> {
    let script = ckb_std::high_level::load_script()?;
    let args: Bytes = script.args().unpack();

    // Args must be exactly 32 bytes — the expected blake2b-256 hash of the preimage
    if args.len() != 32 {
        return Err(Error::InvalidArgsLength);
    }
    let expected: [u8; 32] = args[..32].try_into().map_err(|_| Error::Encoding)?;

    // Load the preimage from the witness lock field
    let witness_args = ckb_std::high_level::load_witness_args(0, Source::GroupInput)?;
    let preimage: Bytes = witness_args
        .lock()
        .to_opt()
        .ok_or(Error::MissingWitness)?
        .unpack();

    // Hash the preimage and compare
    let actual = blake2b_256(preimage.as_ref());

    if actual == expected {
        Ok(())
    } else {
        Err(Error::HashMismatch)
    }
}
