use ckb_hash::blake2b_256;
use ckb_sdk::{Address, AddressPayload, NetworkType};
use ckb_sdk::constants::SIGHASH_TYPE_HASH;
use ckb_types::{bytes::Bytes, core::ScriptHashType, packed::Script, prelude::*};
use rand::rngs::OsRng;
use secp256k1::Secp256k1;

fn main() {
    let secp = Secp256k1::new();

    let secret_key = secp256k1::SecretKey::new(&mut OsRng);
    let public_key = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);

    // CKB uses blake160: first 20 bytes of blake2b-256 of the compressed pubkey
    let pub_bytes = public_key.serialize(); // 33 bytes compressed
    let hash = blake2b_256(pub_bytes);
    let blake160 = Bytes::from(hash[..20].to_vec());

    let lock_script = Script::new_builder()
        .code_hash(SIGHASH_TYPE_HASH.pack())
        .hash_type(ScriptHashType::Type.into())
        .args(blake160.pack())
        .build();

    let payload = AddressPayload::from(lock_script);
    let testnet_address = Address::new(NetworkType::Testnet, payload.clone(), true);
    let mainnet_address = Address::new(NetworkType::Mainnet, payload, true);

    let private_key_hex = format!("0x{}", hex::encode(secret_key.secret_bytes()));

    println!("─── Save these securely ───────────────────────────────");
    println!("PRIVATE_KEY={}", private_key_hex);
    println!("TESTNET_ADDRESS={}", testnet_address);
    println!("MAINNET_ADDRESS={}", mainnet_address);
    println!("───────────────────────────────────────────────────────");
    println!();
    println!("Fund your testnet address at:");
    println!("https://faucet.nervos.org/?address={}", testnet_address);
    println!();
    println!("Paste into your .env file:");
    println!("PRIVATE_KEY={}", private_key_hex);
    println!("TESTNET_ADDRESS={}", testnet_address);
    println!("CKB_RPC=https://testnet.ckb.dev:8114");
}
