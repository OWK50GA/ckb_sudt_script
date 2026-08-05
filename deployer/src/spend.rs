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
    core::{DepType, ScriptHashType, TransactionView},
    packed::{CellDep, CellInput, CellOutput, OutPoint, Script, WitnessArgs},
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

    let [
        string_code_cell_outpoint,
        string_preimage,
        string_idl_path,
        string_locked_cell_outpoint,
    ] = [args.next(), args.next(), args.next(), args.next()].map(|x| x.expect("Missing arg"));

    let idl_path = string_idl_path.as_str();
    let bytes_code_cell_outpoint: [u8; 32] =
        hex::decode(string_code_cell_outpoint)?.try_into().unwrap();
    let packed_code_cell_outpoint = bytes_code_cell_outpoint.pack();
    let bytes_preimage = hex::decode(string_preimage)?;
    let preimage: &[u8] = &bytes_preimage;
    let code_cell_outpoint = OutPoint::new(packed_code_cell_outpoint, 0);

    // The locked cell — the one actually being spent
    let bytes_locked_cell: [u8; 32] = hex::decode(string_locked_cell_outpoint)?
        .try_into()
        .unwrap();
    let locked_cell_outpoint = OutPoint::new(bytes_locked_cell.pack(), 0);

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

    // Cell dep pointing at the code cell (the deployed binary).
    let code_cell_dep = CellDep::new_builder()
        .out_point(code_cell_outpoint.clone())
        .dep_type(DepType::Code.into())
        .build();

    let mut cell_dep_resolver = DefaultCellDepResolver::from_genesis(
        &ckb_client.get_block_by_number(0.into())?.unwrap().into(),
    )?;

    // Register simple-lock code dep so the VM can load the script bytecode
    let simple_lock_script = Script::new_builder()
        .code_hash(code_hash_packed.clone())
        .hash_type(ScriptHashType::Data1.into())
        .args(Bytes::from(blake2b_256(preimage).to_vec()).pack())
        .build();
    let simple_lock_script_id = ScriptId::new_data(simple_lock_script.calc_script_hash().unpack());
    cell_dep_resolver.insert(
        simple_lock_script_id,
        code_cell_dep.clone(),
        "simple-lock".to_string(),
    );

    let header_dep_resolver = DefaultHeaderDepResolver::new(&ckb_rpc);
    let tx_dep_provider = DefaultTransactionDependencyProvider::new(&ckb_rpc, 10);

    // Secp256k1 unlocker for funding inputs (fee payment from wallet)
    let secret_key = SecretKey::from_byte_array(&spender_key)?;
    let signer = SecpCkbRawKeySigner::new_with_secret_keys(vec![secret_key]);
    let sighash_unlocker = SecpSighashUnlocker::from(Box::new(signer) as Box<_>);
    let sighash_script_id = ScriptId::new_type(SIGHASH_TYPE_HASH.clone());
    let mut unlockers: HashMap<ScriptId, Box<dyn ScriptUnlocker>> = HashMap::new();
    unlockers.insert(sighash_script_id, Box::new(sighash_unlocker));

    // The witness for the simple-lock input is the wire-encoded preimage.
    let simple_lock_witness = WitnessArgs::new_builder()
        .lock(Some(Bytes::from(wire)).pack())
        .build();

    // Output: return capacity to the spender's address
    let output = CellOutput::new_builder()
        .capacity((61u64 * 100_000_000u64).pack())
        .lock(spender_lock.clone())
        .build();

    let mut cell_collector = DefaultCellCollector::new(&ckb_rpc);

    let placeholder_witness = WitnessArgs::new_builder()
        .lock(Some(Bytes::from(vec![0u8; 65])).pack())
        .build();
    let balancer = CapacityBalancer::new_simple(spender_lock, placeholder_witness, 1000);

    // Step A: let the builder collect fee-payer inputs and balance the tx.
    // build_unlocked also signs the secp256k1 inputs it collected.
    let builder = CapacityTransferBuilder::new(vec![(output, Bytes::new())]);
    let (balanced_tx, _) = builder.build_unlocked(
        &mut cell_collector,
        &cell_dep_resolver,
        &header_dep_resolver,
        &tx_dep_provider,
        &balancer,
        &unlockers,
    )?;

    // Step B: inject the locked cell as input[0] BEFORE signing.
    // We must prepend it (+ its witness placeholder) and add the code cell dep,
    // then re-sign the secp256k1 inputs so the signature covers the full tx.
    let locked_cell_input = CellInput::new_builder()
        .previous_output(locked_cell_outpoint)
        .build();

    // Prepend locked cell input and witness; append code cell dep.
    // Secp256k1 witnesses shift by 1 — replace the placeholder at the new index.
    let inputs: Vec<_> = std::iter::once(locked_cell_input)
        .chain(balanced_tx.inputs().into_iter())
        .collect();
    // Prepend simple-lock witness at index 0; existing witnesses shift to 1..
    let witnesses: Vec<_> = std::iter::once(simple_lock_witness.as_bytes().pack())
        .chain(balanced_tx.witnesses().into_iter())
        .collect();
    let cell_deps: Vec<_> = balanced_tx
        .cell_deps()
        .into_iter()
        .chain(std::iter::once(code_cell_dep))
        .collect();

    let unsigned_tx: TransactionView = balanced_tx
        .as_advanced_builder()
        .set_inputs(inputs)
        .set_witnesses(witnesses)
        .set_cell_deps(cell_deps)
        .build();

    // Step C: re-sign — the secp256k1 inputs are now at shifted indices,
    // and the tx hash changed, so the old signatures are invalid.
    let (signed_tx, _) = {
        let secret_key2 = SecretKey::from_byte_array(&spender_key)?;
        let signer2 = SecpCkbRawKeySigner::new_with_secret_keys(vec![secret_key2]);
        let sighash_unlocker2 = SecpSighashUnlocker::from(Box::new(signer2) as Box<_>);
        let sighash_script_id2 = ScriptId::new_type(SIGHASH_TYPE_HASH.clone());
        let mut unlockers2: HashMap<ScriptId, Box<dyn ScriptUnlocker>> = HashMap::new();
        unlockers2.insert(sighash_script_id2, Box::new(sighash_unlocker2));

        ckb_sdk::tx_builder::unlock_tx(unsigned_tx, &tx_dep_provider, &mut unlockers2)?
    };

    let tx_hash = ckb_client.send_transaction(signed_tx.data().into(), None)?;
    println!("Spend tx hash: {:#x}", tx_hash);
    println!("Cell spent successfully.");

    Ok(())
}
