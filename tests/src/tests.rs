use ckb_testtool::builtin::ALWAYS_SUCCESS;
use ckb_testtool::ckb_hash::blake2b_256;
use ckb_testtool::ckb_types::{bytes::Bytes, core::TransactionBuilder, packed::*, prelude::*};
use ckb_testtool::context::Context;

const MAX_CYCLES: u64 = 10_000_000;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Encode a u128 token amount as 16-byte little-endian (sUDT cell data format).
fn encode_amount(amount: u128) -> Bytes {
    Bytes::from(amount.to_le_bytes().to_vec())
}

/// Deploy the sUDT type script and return its out-point.
fn deploy_sudt(context: &mut Context) -> OutPoint {
    context.deploy_cell_by_name("ckb_sudt_script")
}

/// Build a dummy lock script used as the "owner" lock.
/// We use the always-success script so inputs locked by it can be consumed freely.
fn build_owner_lock(context: &mut Context) -> Script {
    let always_success_out_point = context.deploy_cell(ALWAYS_SUCCESS.clone());
    context
        .build_script(&always_success_out_point, Bytes::new())
        .expect("always-success lock script")
}

// ── passing tests ─────────────────────────────────────────────────────────────

/// Normal transfer: input amount == output amount. Should pass.
#[test]
fn test_sudt_transfer_equal_amounts() {
    let mut context = Context::default();
    let sudt_out_point = deploy_sudt(&mut context);
    let owner_lock = build_owner_lock(&mut context);

    // args = blake2b hash of the owner lock script
    let owner_lock_hash: [u8; 32] = blake2b_256(owner_lock.as_slice());
    let sudt_args = Bytes::from(owner_lock_hash.to_vec());

    let type_script = context
        .build_script(&sudt_out_point, sudt_args)
        .expect("type script");

    // Input cell: 100 tokens
    let input_out_point = context.create_cell(
        CellOutput::new_builder()
            .capacity(1000u64)
            .lock(owner_lock.clone())
            .type_(Some(type_script.clone()).pack())
            .build(),
        encode_amount(100),
    );
    let input = CellInput::new_builder()
        .previous_output(input_out_point)
        .build();

    // Output cell: 100 tokens (equal — valid transfer)
    let output = CellOutput::new_builder()
        .capacity(1000u64)
        .lock(owner_lock.clone())
        .type_(Some(type_script.clone()).pack())
        .build();

    let tx = TransactionBuilder::default()
        .input(input)
        .output(output)
        .output_data(encode_amount(100).pack())
        .build();
    let tx = context.complete_tx(tx);

    context
        .verify_tx(&tx, MAX_CYCLES)
        .expect("transfer equal amounts should pass");
}

/// Transfer where output < input (burning tokens). Should pass.
#[test]
fn test_sudt_transfer_burn_tokens() {
    let mut context = Context::default();
    let sudt_out_point = deploy_sudt(&mut context);
    let owner_lock = build_owner_lock(&mut context);

    let owner_lock_hash: [u8; 32] = blake2b_256(owner_lock.as_slice());
    let type_script = context
        .build_script(&sudt_out_point, Bytes::from(owner_lock_hash.to_vec()))
        .expect("type script");

    let input_out_point = context.create_cell(
        CellOutput::new_builder()
            .capacity(1000u64)
            .lock(owner_lock.clone())
            .type_(Some(type_script.clone()).pack())
            .build(),
        encode_amount(100),
    );
    let input = CellInput::new_builder()
        .previous_output(input_out_point)
        .build();

    // Output only 60 — burning 40 tokens
    let output = CellOutput::new_builder()
        .capacity(1000u64)
        .lock(owner_lock.clone())
        .type_(Some(type_script.clone()).pack())
        .build();

    let tx = TransactionBuilder::default()
        .input(input)
        .output(output)
        .output_data(encode_amount(60).pack())
        .build();
    let tx = context.complete_tx(tx);

    context
        .verify_tx(&tx, MAX_CYCLES)
        .expect("burning tokens should pass");
}

/// Owner mode: the owner's lock script hash appears as a type hash of an input cell.
/// Owner can mint freely — output > input is allowed.
#[test]
fn test_sudt_owner_mode_mint() {
    let mut context = Context::default();
    let sudt_out_point = deploy_sudt(&mut context);
    let owner_lock = build_owner_lock(&mut context);

    let owner_lock_hash: [u8; 32] = blake2b_256(owner_lock.as_slice());
    let type_script = context
        .build_script(&sudt_out_point, Bytes::from(owner_lock_hash.to_vec()))
        .expect("type script");

    // A "governance" cell whose TYPE hash == owner_lock_hash.
    // We need a type script whose script hash equals owner_lock_hash.
    // The easiest way: deploy a cell whose type script hash we control.
    // Here we use the owner_lock itself as a type script on a separate input cell
    // so that load_cell_type_hash returns the owner_lock_hash for that input.
    let governance_cell_out_point = context.create_cell(
        CellOutput::new_builder()
            .capacity(500u64)
            .lock(owner_lock.clone())
            .type_(Some(owner_lock.clone()).pack()) // type script = owner_lock → its hash == owner_lock_hash
            .build(),
        Bytes::new(),
    );
    let governance_input = CellInput::new_builder()
        .previous_output(governance_cell_out_point)
        .build();

    // sUDT input: 0 tokens (minting from nothing)
    let sudt_input_out_point = context.create_cell(
        CellOutput::new_builder()
            .capacity(1000u64)
            .lock(owner_lock.clone())
            .type_(Some(type_script.clone()).pack())
            .build(),
        encode_amount(0),
    );
    let sudt_input = CellInput::new_builder()
        .previous_output(sudt_input_out_point)
        .build();

    // Output: 1000 tokens minted
    let output = CellOutput::new_builder()
        .capacity(1000u64)
        .lock(owner_lock.clone())
        .type_(Some(type_script.clone()).pack())
        .build();

    let tx = TransactionBuilder::default()
        .input(governance_input)
        .input(sudt_input)
        .output(output)
        .output_data(encode_amount(1000).pack())
        .build();
    let tx = context.complete_tx(tx);

    context
        .verify_tx(&tx, MAX_CYCLES)
        .expect("owner mode mint should pass");
}

/// Multiple input/output cells — sums must balance.
#[test]
fn test_sudt_multi_cell_transfer() {
    let mut context = Context::default();
    let sudt_out_point = deploy_sudt(&mut context);
    let owner_lock = build_owner_lock(&mut context);

    let owner_lock_hash: [u8; 32] = blake2b_256(owner_lock.as_slice());
    let type_script = context
        .build_script(&sudt_out_point, Bytes::from(owner_lock_hash.to_vec()))
        .expect("type script");

    // Two inputs: 60 + 40 = 100
    let mut make_input = |amount: u128| {
        let op = context.create_cell(
            CellOutput::new_builder()
                .capacity(1000u64)
                .lock(owner_lock.clone())
                .type_(Some(type_script.clone()).pack())
                .build(),
            encode_amount(amount),
        );
        CellInput::new_builder().previous_output(op).build()
    };

    let output = CellOutput::new_builder()
        .capacity(2000u64)
        .lock(owner_lock.clone())
        .type_(Some(type_script.clone()).pack())
        .build();

    let tx = TransactionBuilder::default()
        .input(make_input(60))
        .input(make_input(40))
        .output(output)
        .output_data(encode_amount(100).pack())
        .build();
    let tx = context.complete_tx(tx);

    context
        .verify_tx(&tx, MAX_CYCLES)
        .expect("multi-cell balanced transfer should pass");
}

// ── failing tests ─────────────────────────────────────────────────────────────

/// Output amount > input amount without owner mode. Should fail with OutputOverflow (12).
#[test]
fn test_sudt_inflation_attack_fails() {
    let mut context = Context::default();
    let sudt_out_point = deploy_sudt(&mut context);
    let owner_lock = build_owner_lock(&mut context);

    let owner_lock_hash: [u8; 32] = blake2b_256(owner_lock.as_slice());
    let type_script = context
        .build_script(&sudt_out_point, Bytes::from(owner_lock_hash.to_vec()))
        .expect("type script");

    let input_out_point = context.create_cell(
        CellOutput::new_builder()
            .capacity(1000u64)
            .lock(owner_lock.clone())
            .type_(Some(type_script.clone()).pack())
            .build(),
        encode_amount(100),
    );
    let input = CellInput::new_builder()
        .previous_output(input_out_point)
        .build();

    // Trying to output 200 from 100 input — inflation
    let output = CellOutput::new_builder()
        .capacity(1000u64)
        .lock(owner_lock.clone())
        .type_(Some(type_script.clone()).pack())
        .build();

    let tx = TransactionBuilder::default()
        .input(input)
        .output(output)
        .output_data(encode_amount(200).pack())
        .build();
    let tx = context.complete_tx(tx);

    let err = context.verify_tx(&tx, MAX_CYCLES).unwrap_err();
    assert!(
        err.to_string().contains("error code 12"),
        "expected OutputOverflow error, got: {err}"
    );
}

/// Script args are not 32 bytes. Should fail with ArgsLength (10).
#[test]
fn test_sudt_invalid_args_length_fails() {
    let mut context = Context::default();
    let sudt_out_point = deploy_sudt(&mut context);
    let owner_lock = build_owner_lock(&mut context);

    // Only 16 bytes instead of 32
    let bad_args = Bytes::from(vec![0u8; 16]);
    let type_script = context
        .build_script(&sudt_out_point, bad_args)
        .expect("type script");

    let input_out_point = context.create_cell(
        CellOutput::new_builder()
            .capacity(1000u64)
            .lock(owner_lock.clone())
            .type_(Some(type_script.clone()).pack())
            .build(),
        encode_amount(100),
    );
    let input = CellInput::new_builder()
        .previous_output(input_out_point)
        .build();

    let output = CellOutput::new_builder()
        .capacity(1000u64)
        .lock(owner_lock.clone())
        .type_(Some(type_script.clone()).pack())
        .build();

    let tx = TransactionBuilder::default()
        .input(input)
        .output(output)
        .output_data(encode_amount(100).pack())
        .build();
    let tx = context.complete_tx(tx);

    let err = context.verify_tx(&tx, MAX_CYCLES).unwrap_err();
    assert!(
        err.to_string().contains("error code 10"),
        "expected ArgsLength error, got: {err}"
    );
}

/// Cell data is less than 16 bytes — can't decode amount. Should fail with Encoding (4).
#[test]
fn test_sudt_malformed_cell_data_fails() {
    let mut context = Context::default();
    let sudt_out_point = deploy_sudt(&mut context);
    let owner_lock = build_owner_lock(&mut context);

    let owner_lock_hash: [u8; 32] = blake2b_256(owner_lock.as_slice());
    let type_script = context
        .build_script(&sudt_out_point, Bytes::from(owner_lock_hash.to_vec()))
        .expect("type script");

    // Only 8 bytes — too short to decode a u128
    let bad_data = Bytes::from(vec![0u8; 8]);

    let input_out_point = context.create_cell(
        CellOutput::new_builder()
            .capacity(1000u64)
            .lock(owner_lock.clone())
            .type_(Some(type_script.clone()).pack())
            .build(),
        bad_data.clone(),
    );
    let input = CellInput::new_builder()
        .previous_output(input_out_point)
        .build();

    let output = CellOutput::new_builder()
        .capacity(1000u64)
        .lock(owner_lock.clone())
        .type_(Some(type_script.clone()).pack())
        .build();

    let tx = TransactionBuilder::default()
        .input(input)
        .output(output)
        .output_data(bad_data.pack())
        .build();
    let tx = context.complete_tx(tx);

    let err = context.verify_tx(&tx, MAX_CYCLES).unwrap_err();
    assert!(
        err.to_string().contains("error code 4"),
        "expected Encoding error, got: {err}"
    );
}

// generated unit test for contract simple-lock
// #[test]
// fn test_simple_lock() {
//     // deploy contract
//     let mut context = Context::default();
//     let out_point = context.deploy_cell_by_name("simple-lock");

//     // prepare scripts
//     let lock_script = context
//         .build_script(&out_point, Bytes::from(vec![42]))
//         .expect("script");

//     // prepare cells
//     let input_out_point = context.create_cell(
//         CellOutput::new_builder()
//             .capacity(1000)
//             .lock(lock_script.clone())
//             .build(),
//         Bytes::new(),
//     );
//     let input = CellInput::new_builder()
//         .previous_output(input_out_point)
//         .build();
//     let outputs = vec![
//         CellOutput::new_builder()
//             .capacity(500)
//             .lock(lock_script.clone())
//             .build(),
//         CellOutput::new_builder()
//             .capacity(500)
//             .lock(lock_script)
//             .build(),
//     ];

//     let outputs_data = vec![Bytes::new(); 2];

//     // build transaction
//     let tx = TransactionBuilder::default()
//         .input(input)
//         .outputs(outputs)
//         .outputs_data(outputs_data.pack())
//         .build();
//     let tx = context.complete_tx(tx);

//     // run
//     let cycles = context
//         .verify_tx(&tx, 10_000_000)
//         .expect("pass verification");
//     println!("consume cycles: {}", cycles);
// }
