use std::{collections::HashMap, str::FromStr};

use ckb_hash::blake2b_256;
use ckb_jsonrpc_types::Either;
use ckb_sdk::{
    Address, CkbRpcClient, ScriptId,
    constants::SIGHASH_TYPE_HASH,
    rpc::ckb_indexer::{SearchKey, SearchMode},
    traits::{
        DefaultCellCollector, DefaultCellDepResolver, DefaultHeaderDepResolver,
        DefaultTransactionDependencyProvider, SecpCkbRawKeySigner,
    },
    tx_builder::{CapacityBalancer, TxBuilder, transfer::CapacityTransferBuilder},
    unlock::{ScriptUnlocker, SecpSighashUnlocker},
};
use ckb_types::{
    H256,
    bytes::Bytes,
    core::{DepType, ScriptHashType},
    packed::{CellDep, CellOutput, OutPoint, Script, WitnessArgs},
    prelude::{Builder, Entity, Pack, Unpack},
};
use secp256k1::SecretKey;

// pub const TESTNET_RPC: &str = "https://testnet.ckb.dev";

pub fn deploy_script(
    ckb_rpc: &str,
    deployer_address: &str,
    sender_key: SecretKey,
    script_path: &str,
) -> anyhow::Result<OutPoint> {
    let script_binary = std::fs::read(script_path)?;
    let script_size = script_binary.len();

    let required_capacity = (script_size as u64 + 100) * 100_000_000;

    println!("Script size: {script_size} bytes");
    println!("Required capacity: {} CKB", required_capacity / 100_000_000);

    let sender = Address::from_str(deployer_address).unwrap();

    let payload = sender.payload();

    // Build a cell that contains the script binary as its data.
    // This is what is called deploying the script
    let deployer_lock: Script = Script::from(payload);

    let code_cell_output = CellOutput::new_builder()
        .capacity(required_capacity.pack())
        .lock(deployer_lock)
        .build();

    let ckb_client = CkbRpcClient::new(ckb_rpc); // Try the url itself if fail

    let mut cell_collector = DefaultCellCollector::new(ckb_rpc);

    let blockview = (ckb_client.get_block_by_number(0.into())?).unwrap();
    let cell_dep_resolver = DefaultCellDepResolver::from_genesis(&blockview.into())?;
    let header_dep_resolver = DefaultHeaderDepResolver::new(ckb_rpc);
    let tx_dep_provider = DefaultTransactionDependencyProvider::new(ckb_rpc, 10);

    let signer = SecpCkbRawKeySigner::new_with_secret_keys(vec![sender_key]);
    let sighash_unlocker = SecpSighashUnlocker::from(Box::new(signer) as Box<_>);
    let sighash_script_id = ScriptId::new_type(SIGHASH_TYPE_HASH.clone());

    let mut unlockers: HashMap<ScriptId, Box<dyn ScriptUnlocker>> = HashMap::new();
    unlockers.insert(sighash_script_id, Box::new(sighash_unlocker));

    let placeholder_witness = ckb_types::packed::WitnessArgs::new_builder()
        .lock(Some(Bytes::from(vec![0u8; 65])).pack())
        .build();
    let balancer = CapacityBalancer::new_simple(sender.payload().into(), placeholder_witness, 1000);

    let builder =
        CapacityTransferBuilder::new(vec![(code_cell_output, Bytes::from(script_binary))]);

    let (tx, _) = builder.build_unlocked(
        &mut cell_collector,
        &cell_dep_resolver,
        &header_dep_resolver,
        &tx_dep_provider,
        &balancer,
        &unlockers,
    )?;

    let tx_hash = ckb_client.send_transaction(tx.data().into(), None)?;

    println!("Deployment tx hash: {:#x}", tx_hash);
    println!("Code cell Outpoint: {}:0x0", tx_hash);

    Ok(OutPoint::new(tx_hash.pack(), 0))
}

pub fn mint_tokens(
    ckb_rpc: &str,
    code_cell_outpoint: OutPoint,
    issuer_address: &str,
    issuer_key: SecretKey,
    recipient_address: &str,
    amount: u128,
) -> anyhow::Result<()> {
    let ckb_client = CkbRpcClient::new(ckb_rpc);

    let issuer_addr = Address::from_str(issuer_address).unwrap();
    let recipient_addr = Address::from_str(recipient_address).unwrap();

    let issuer_lock: Script = (issuer_addr.payload()).into();

    let owner_lock_hash = ckb_hash::blake2b_256(issuer_lock.as_slice());

    let code_hash: H256 = compute_code_hash(&ckb_client, &code_cell_outpoint)?.unpack();

    let sudt_type_script = Script::new_builder()
        .code_hash(compute_code_hash(&ckb_client, &code_cell_outpoint)?)
        .hash_type(ScriptHashType::Data1.into())
        .args(Bytes::from(owner_lock_hash.to_vec()).pack())
        .build();

    let recipient_lock: Script = (recipient_addr.payload()).into();
    let udt_cell = CellOutput::new_builder()
        .capacity((142u64 * 100_000_000).pack())
        .lock(recipient_lock)
        .type_(Some(sudt_type_script).pack())
        .build();

    let udt_data = Bytes::from(amount.to_be_bytes().to_vec());

    let sudt_cell_dep = CellDep::new_builder()
        .out_point(code_cell_outpoint)
        .dep_type(DepType::Code.into())
        .build();
    let sudt_script_id = ScriptId::new_data(code_hash);

    let mut cell_collector = DefaultCellCollector::new(ckb_rpc);
    let mut cell_dep_resolver = DefaultCellDepResolver::from_genesis(
        &ckb_client.get_block_by_number(0.into())?.unwrap().into(),
    )?;
    cell_dep_resolver.insert(sudt_script_id, sudt_cell_dep, "sudt".to_string());
    let header_dep_resolver = DefaultHeaderDepResolver::new(ckb_rpc);
    let tx_dep_provider = DefaultTransactionDependencyProvider::new(ckb_rpc, 10);

    let signer = SecpCkbRawKeySigner::new_with_secret_keys(vec![issuer_key]);
    let sighash_unlocker = SecpSighashUnlocker::from(Box::new(signer) as Box<_>);
    let sighash_script_id = ScriptId::new_type(SIGHASH_TYPE_HASH.clone());

    let mut unlockers: HashMap<ScriptId, Box<dyn ScriptUnlocker>> = HashMap::new();
    unlockers.insert(sighash_script_id, Box::new(sighash_unlocker));

    let placeholder_witness = WitnessArgs::new_builder()
        .lock(Some(Bytes::from(vec![0u8; 65])).pack())
        .build();
    let balancer = CapacityBalancer::new_simple(issuer_lock, placeholder_witness, 1000);

    let builder = CapacityTransferBuilder::new(vec![(udt_cell, udt_data)]);

    let (tx, _) = builder.build_unlocked(
        &mut cell_collector,
        &cell_dep_resolver,
        &header_dep_resolver,
        &tx_dep_provider,
        &balancer,
        &unlockers,
    )?;

    let tx_hash = ckb_client.send_transaction(tx.data().into(), None)?;
    println!("Mint tx hash: {:#x}", tx_hash);
    println!("Minted {} tokens to {}", amount, recipient_address);

    Ok(())
}

pub fn compute_code_hash(
    ckb_client: &CkbRpcClient,
    code_cell_outpoint: &OutPoint,
) -> anyhow::Result<ckb_types::packed::Byte32> {
    let tx_hash = code_cell_outpoint.tx_hash();
    let index: u32 = code_cell_outpoint.index().unpack();

    let tx = ckb_client
        .get_transaction(tx_hash.unpack())?
        .expect("deployment tx not found")
        .transaction
        .expect("tx data missing");

    // ResponseFormat<TransactionView> wraps Either<TransactionView, JsonBytes>.
    // The RPC default returns hex/molecule bytes (Either::Right).
    // We handle both variants.
    let packed_tx = match tx.inner {
        Either::Left(json_tx) => {
            let packed: ckb_types::packed::Transaction = json_tx.inner.into();
            packed
        }
        Either::Right(bytes) => ckb_types::packed::Transaction::from_slice(bytes.as_bytes())
            .map_err(|e| anyhow::anyhow!("failed to decode tx bytes: {e}"))?,
    };

    let data = packed_tx
        .raw()
        .outputs_data()
        .get(index as usize)
        .expect("output data missing");
    let raw: Bytes = data.unpack();

    Ok(blake2b_256(raw.as_ref()).pack())
}

#[allow(unused)]
pub fn transfer_tokens(
    ckb_rpc: &str,
    sudt_type_script: Script,
    code_cell_outpoint: OutPoint,
    sender_address: &str,
    sender_key_hex: &str,
    recipient_address: &str,
    transfer_amount: u128,
) -> anyhow::Result<()> {
    let ckb_client = CkbRpcClient::new(ckb_rpc);

    let sender_addr = Address::from_str(sender_address).unwrap();

    let mut clean_sender_key = [0_u8; 32];
    let sender_key_bytes = hex::decode(sender_key_hex)?;
    clean_sender_key.copy_from_slice(&sender_key_bytes);
    let sender_key = SecretKey::from_byte_array(&clean_sender_key)?;

    let recipient_addr = Address::from_str(recipient_address).unwrap();

    let sender_lock: Script = sender_addr.payload().into();
    let recipient_lock: Script = recipient_addr.payload().into();

    let mut cell_collector = DefaultCellCollector::new(ckb_rpc);
    let sudt_type_hash: [u8; 32] = sudt_type_script.calc_script_hash().unpack();

    let search_key = SearchKey {
        script: sender_lock.clone().into(),
        script_type: ckb_sdk::rpc::ckb_indexer::ScriptType::Lock,
        filter: Some(ckb_sdk::rpc::ckb_indexer::SearchKeyFilter {
            script: Some(sudt_type_script.clone().into()),
            ..Default::default()
        }),
        script_search_mode: Some(SearchMode::Partial),
        with_data: Some(true),
        group_by_transaction: Some(true),
    };

    let udt_cells = ckb_client.get_cells(
        search_key,
        ckb_sdk::rpc::ckb_indexer::Order::Asc,
        1000.into(),
        None,
    )?;

    let mut inputs = Vec::new();
    let mut input_total: u128 = 0;

    for cell in &udt_cells.objects {
        let cell_type_hash: Option<[u8; 32]> = cell.output.type_.as_ref().map(|s| {
            let script: Script = s.clone().into();
            script.calc_script_hash().unpack()
        });

        match cell_type_hash {
            Some(hash) if hash == sudt_type_hash => {
                let output_data = cell.output_data.as_ref().expect("output data not found").as_bytes();
                let amount = u128::from_le_bytes(output_data[..16].try_into().unwrap());
                inputs.push(cell.clone());
                input_total = input_total.checked_add(amount).expect("overflow");
            }
            _ => continue,
        }

        if input_total >= transfer_amount {
            break;
        }
    }

    if input_total < transfer_amount {
        anyhow::bail!(
            "Insufficient UDT balance: have {}, need {}",
            input_total,
            transfer_amount
        );
    }

    let change_amount = input_total - transfer_amount;

    let capacity_per_cell = 142_u64 * 100_000_000; // 142 CKB

    let recipient_cell = CellOutput::new_builder()
        .capacity(capacity_per_cell.pack())
        .lock(recipient_lock)
        .type_(Some(sudt_type_script.clone()).pack())
        .build();

    let mut outputs = vec![(
        recipient_cell,
        Bytes::from(transfer_amount.to_le_bytes().to_vec()),
    )];

    if change_amount > 0 {
        let change_cell = CellOutput::new_builder()
            .capacity(capacity_per_cell.pack())
            .lock(sender_lock.clone())
            .type_(Some(sudt_type_script.clone()).pack())
            .build();
        outputs.push((
            change_cell,
            Bytes::from(change_amount.to_le_bytes().to_vec()),
        ));
    }

    let code_hash: H256 = compute_code_hash(&ckb_client, &code_cell_outpoint)?.unpack();
    let sudt_script_id = ScriptId::new_data(code_hash);

    let sudt_cell_dep = CellDep::new_builder()
        .out_point(code_cell_outpoint)
        .dep_type(DepType::Code.into())
        .build();

    let mut cell_dep_resolver = DefaultCellDepResolver::from_genesis(
        &ckb_client.get_block_by_number(0.into())?.unwrap().into(),
    )?;

    cell_dep_resolver.insert(sudt_script_id, sudt_cell_dep, "sudt".to_string());

    let header_dep_resolver = DefaultHeaderDepResolver::new(ckb_rpc);
    let tx_dep_provider = DefaultTransactionDependencyProvider::new(ckb_rpc, 10);

    let signer = SecpCkbRawKeySigner::new_with_secret_keys(vec![sender_key]);
    let sighash_unlocker = SecpSighashUnlocker::from(Box::new(signer) as Box<_>);
    let sighash_script_id = ScriptId::new_type(SIGHASH_TYPE_HASH.clone());
    let mut unlockers: HashMap<ScriptId, Box<dyn ScriptUnlocker>> = HashMap::new();
    unlockers.insert(sighash_script_id, Box::new(sighash_unlocker));

    let placeholder_witness = WitnessArgs::new_builder()
        .lock(Some(Bytes::from(vec![0u8; 65])).pack())
        .build();
    let balancer = CapacityBalancer::new_simple(sender_lock, placeholder_witness, 1000);

    let builder = CapacityTransferBuilder::new(outputs);

    let (tx, _) = builder.build_unlocked(
        &mut cell_collector,
        &cell_dep_resolver,
        &header_dep_resolver,
        &tx_dep_provider,
        &balancer,
        &unlockers,
    )?;

    let tx_hash = ckb_client.send_transaction(tx.data().into(), None)?;
    println!("Transfer tx hash: {:#x}", tx_hash);
    println!(
        "Transferred {} tokens to {}",
        transfer_amount, recipient_address
    );

    Ok(())
}
