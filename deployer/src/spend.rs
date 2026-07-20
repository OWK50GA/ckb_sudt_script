use std::{collections::HashMap, str::FromStr};

use ckb_hash::blake2b_256;
use ckb_idl_client::IdlClient;
use ckb_sdk::{
    Address, CkbRpcClient, ScriptId,
    constants::SIGHASH_TYPE_HASH,
    traits::{
        DefaultCellCollector, DefaultCellDepResolver, DefaultHeaderDepResolver,
        DefaultTransactionDependencyProvider, SecpCkbRawKeySigner,
    },
    tx_builder::{CapacityBalancer, TxBuilder, transfer::CapacityTransferBuilder},
    unlock::{ScriptUnlocker, SecpSighashUnlocker},
};
use ckb_types::{
    bytes::Bytes,
    core::{DepType, ScriptHashType},
    packed::{CellDep, CellOutput, OutPoint, Script, WitnessArgs},
    prelude::{Builder, Entity, Pack, Unpack},
};
use secp256k1::SecretKey;

use crate::config::Config;
use crate::deploy_script::{deploy_script, fetch_code_cell_data};

/// Deploy simple-lock with its IDL commitment appended to the cell data.
/// Returns the outpoint of the deployed code cell.
pub fn deploy_simple_lock() -> anyhow::Result<OutPoint> {
    let cfg = Config::from_env()?;
    let secret_key = cfg.secret_key()?;

    println!("Deploying from: {}", cfg.address);
    println!("RPC endpoint:   {}", cfg.ckb_rpc);

    let code_outpoint = deploy_script(
        &cfg.ckb_rpc,
        &cfg.address,
        secret_key,
        "./build/release/simple-lock",
        Some("./contracts/simple-lock/idl.json"),
    )?;

    println!("Code outpoint tx: {:#x}", code_outpoint.tx_hash());
    Ok(code_outpoint)
}

/// Spend a simple-lock cell.
///
/// Flow:
///   1. Fetch the code cell data (binary + idl_commitment) and derive code_hash
///   2. Load the local IDL JSON
///   3. Verify the IDL against the on-chain idl_commitment (last 32 bytes)
///   4. Encode the proposed preimage witness in wire format
///   5. Validate the witness structurally with IdlClient — PSCT check
///   6. If both pass, build and submit the spend transaction
///
/// `locked_cell_outpoint` is the outpoint of the cell you want to spend.
/// `code_cell_outpoint` is the outpoint of the deployed simple-lock binary.
/// `preimage` is the raw preimage bytes (must hash to the 32-byte value in args).
pub fn spend_simple_lock(mut args: impl Iterator<Item = String>) -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    let Config {
        ckb_rpc,
        address: spender_address,
        private_key_bytes: spender_key,
    } = cfg;

    let [string_code_cell_outpoint, string_preimage, string_idl_path] =
        [args.next(), args.next(), args.next()].map(|x| x.expect("Missing arg"));

    let idl_path = string_idl_path.as_str();
    let bytes_code_cell_outpoint: [u8; 32] =
        hex::decode(string_code_cell_outpoint)?.try_into().unwrap();
    let packed_code_cell_outpoint = bytes_code_cell_outpoint.pack();
    let bytes_preimage = hex::decode(string_preimage)?;
    let preimage: &[u8] = &bytes_preimage;
    let code_cell_outpoint = OutPoint::new(packed_code_cell_outpoint, 0);

    let ckb_client = CkbRpcClient::new(&ckb_rpc);

    // ── Step 1: fetch code cell data and derive code_hash ────────────────────
    let (code_hash_packed, code_cell_data) =
        fetch_code_cell_data(&ckb_client, &code_cell_outpoint)?;
    let code_hash: [u8; 32] = code_hash_packed.unpack();

    println!("code_hash: {}", hex::encode(code_hash));

    // ── Step 2: load local IDL JSON ──────────────────────────────────────────
    let idl_json_bytes = std::fs::read(idl_path)
        .map_err(|e| anyhow::anyhow!("failed to read IDL file at {idl_path}: {e}"))?;

    // ── Step 3: verify IDL against the on-chain idl_commitment ───────────────
    let mut idl_client = IdlClient::new();

    idl_client
        .verify(code_hash, &idl_json_bytes, &code_cell_data)
        .map_err(|e| anyhow::anyhow!("IDL commitment verification failed: {e}"))?;

    println!("IDL commitment verified — IDL is authentic.");

    // Parse the IDL to get the witness field list.
    let idl_doc: ckb_idl_client::IdlDocument = serde_json::from_slice(&idl_json_bytes)
        .map_err(|e| anyhow::anyhow!("failed to parse IDL JSON: {e}"))?;

    // ── Step 4: encode the proposed witness in wire format ───────────────────
    // simple-lock has one field: preimage (bytes) — 4-byte LE length prefix + payload.
    let wire = {
        let mut buf = Vec::new();
        let len = preimage.len() as u32;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(preimage);
        buf
    };

    // ── Step 5: PSCT structural validation ───────────────────────────────────
    let validated = idl_client
        .validate_witness_bytes(&idl_doc.witness, &wire)
        .map_err(|e| anyhow::anyhow!("witness validation failed — malformed witness: {e}"))?;

    println!("Witness structurally valid. Decoded fields:");
    for f in &validated {
        println!("  {} ({}): {:?}", f.name, f.type_, f.value);
    }

    // ── Step 6: build and submit the spend transaction ───────────────────────
    let spender = Address::from_str(&spender_address)
        .map_err(|e| anyhow::anyhow!("invalid spender address: {e}"))?;
    let spender_lock: Script = spender.payload().into();

    // The lock script for the cell being spent is simple-lock.
    // code_hash is the hash of (binary + idl_commitment).
    let simple_lock_script = Script::new_builder()
        .code_hash(code_hash_packed.clone())
        .hash_type(ScriptHashType::Data1.into())
        .args(Bytes::from(blake2b_256(preimage).to_vec()).pack())
        .build();

    // Cell dep pointing at the code cell (the deployed binary).
    let code_cell_dep = CellDep::new_builder()
        .out_point(code_cell_outpoint.clone())
        .dep_type(DepType::Code.into())
        .build();

    let simple_lock_script_id = ScriptId::new_data(simple_lock_script.calc_script_hash().unpack());

    let mut cell_collector = DefaultCellCollector::new(&ckb_rpc);
    let mut cell_dep_resolver = DefaultCellDepResolver::from_genesis(
        &ckb_client.get_block_by_number(0.into())?.unwrap().into(),
    )?;
    cell_dep_resolver.insert(
        simple_lock_script_id,
        code_cell_dep,
        "simple-lock".to_string(),
    );

    let header_dep_resolver = DefaultHeaderDepResolver::new(&ckb_rpc);
    let tx_dep_provider = DefaultTransactionDependencyProvider::new(&ckb_rpc, 10);

    let signer =
        SecpCkbRawKeySigner::new_with_secret_keys(vec![SecretKey::from_byte_array(&spender_key)?]);
    let sighash_unlocker = SecpSighashUnlocker::from(Box::new(signer) as Box<_>);
    let sighash_script_id = ScriptId::new_type(SIGHASH_TYPE_HASH.clone());
    let mut unlockers: HashMap<ScriptId, Box<dyn ScriptUnlocker>> = HashMap::new();
    unlockers.insert(sighash_script_id, Box::new(sighash_unlocker));

    // The witness for the simple-lock input is the wire-encoded preimage.
    let witness_args = WitnessArgs::new_builder()
        .lock(Some(Bytes::from(wire)).pack())
        .build();

    // Output: send the capacity back to the spender's own address.
    let output = CellOutput::new_builder()
        .capacity((61u64 * 100_000_000u64).pack()) // CapacityBalancer fills this in
        .lock(spender_lock.clone())
        .build();

    let placeholder_witness = WitnessArgs::new_builder()
        .lock(Some(Bytes::from(vec![0u8; 65])).pack())
        .build();
    let balancer = CapacityBalancer::new_simple(spender_lock, placeholder_witness, 1000);

    let builder = CapacityTransferBuilder::new(vec![(output, Bytes::new())]);

    let (mut tx, _) = builder.build_unlocked(
        &mut cell_collector,
        &cell_dep_resolver,
        &header_dep_resolver,
        &tx_dep_provider,
        &balancer,
        &unlockers,
    )?;

    // Attach the real simple-lock witness.
    // The SDK builds the tx with a placeholder witness for the secp256k1 input;
    // we need to prepend our simple-lock witness for input index 0.
    let witnesses: Vec<_> = std::iter::once(witness_args.as_bytes().pack())
        .chain(tx.witnesses().into_iter().skip(1))
        .collect();
    tx = tx.as_advanced_builder().set_witnesses(witnesses).build();

    let tx_hash = ckb_client.send_transaction(tx.data().into(), None)?;
    println!("Spend tx hash: {:#x}", tx_hash);
    println!("Cell spent successfully.");

    Ok(())
}
