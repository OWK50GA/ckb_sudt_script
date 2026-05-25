extern crate alloc;
use alloc::vec::Vec;
use ckb_idl_derive::CkbWitness;

/// Witness for the simple-lock script.
///
/// The lock unlocks a cell if blake2b_256(preimage) equals the 32-byte hash
/// stored in the script args. The preimage is provided by the spender.
#[derive(CkbWitness)]
pub struct Witness {
    #[witness(description = "Preimage whose blake2b-256 hash must match the hash in script args")]
    pub preimage: Vec<u8>,
}
