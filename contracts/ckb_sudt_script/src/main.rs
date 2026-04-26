#![cfg_attr(not(any(feature = "library", test)), no_std)]
#![cfg_attr(not(test), no_main)]

pub mod error;

use ckb_std::{
    ckb_constants::Source,
    ckb_types::{bytes::Bytes, prelude::*},
    high_level::{QueryIter, load_cell_data, load_cell_type_hash, load_script},
};
use error::Error;

#[cfg(any(feature = "library", test))]
extern crate alloc;

#[cfg(not(any(feature = "library", test)))]
ckb_std::entry!(program_entry);
#[cfg(not(any(feature = "library", test)))]
// By default, the following heap configuration is used:
// * 16KB fixed heap
// * 1.2MB(rounded up to be 16-byte aligned) dynamic heap
// * Minimal memory block in dynamic heap is 64 bytes
// For more details, please refer to ckb-std's default_alloc macro
// and the buddy-alloc alloc implementation.
ckb_std::default_alloc!(16384, 1258306, 64);

pub fn program_entry() -> i8 {
    // ckb_std::debug!("This is a sample contract!");
    match sudt_main() {
        Ok(()) => 0,
        Err(e) => e as i8,
    }
}

// fn lock_to_script_hash() -> Result<(), Error> {
//     let mut index = 0;

//     loop {
//         match load_witness_args(index, Source::GroupInput) {
//             Ok(data) => {
//                 // let no_of_args = data.field_count();
//             },
//             Err(err) => {

//             }
//         }
//     }
// }

fn decode_amount(data: &[u8]) -> Option<u128> {
    if data.len() < 16 {
        return None;
    }

    // This is little-endian safe now, because length is checked
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&data[..16]);
    Some(u128::from_le_bytes(buf))
}

// Sum all token amounts in cells at `source` that share our typeScript
// We pass the GroupInput variant in one pass, and the GroupOutput in another
// Then make sure they're equal, unless its owner mode, where minting is possible
fn sum_amounts(source: Source) -> Result<u128, Error> {
    let mut total: u128 = 0;

    for data in QueryIter::new(load_cell_data, source) {
        let amount = decode_amount(&data).ok_or(Error::Encoding)?;
        total = total.checked_add(amount).ok_or(Error::Overflow)?;
    }

    Ok(total)
}

fn is_owner_mode(owner_lock_hash: &[u8; 32]) -> Result<bool, Error> {
    for lock_hash in QueryIter::new(load_cell_type_hash, Source::Input) {
        if lock_hash.is_none() {
            continue;
        };
        if &lock_hash.unwrap() == owner_lock_hash {
            return Ok(true);
        }
    }

    Ok(false)
}

fn sudt_main() -> Result<(), Error> {
    // This is used to load the script's args
    // For sUDT, args must be exactly 32 bytes (it is a blake2b-256 hash)
    let script = load_script()?;

    let args: Bytes = script.args().unpack();

    if args.len() != 32 {
        return Err(Error::ArgsLength);
    }

    let owner_lock_hash: [u8; 32] = args[..32].try_into().map_err(|_| Error::Encoding)?;

    // If in owner mode, we are skipping validation
    // Owner can mint and burn
    if is_owner_mode(&owner_lock_hash)? {
        return Ok(());
    }

    let input_sum = sum_amounts(Source::GroupInput)?;
    let output_sum = sum_amounts(Source::GroupOutput)?;

    if output_sum > input_sum {
        return Err(Error::OutputOverflow);
    }

    Ok(())
}
