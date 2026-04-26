mod config;
mod deploy_script;

use config::Config;
use deploy_script::{deploy_script, mint_tokens};

fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    let secret_key = cfg.secret_key()?;

    println!("Deploying from: {}", cfg.address);
    println!("RPC endpoint:   {}", cfg.ckb_rpc);

    let code_outpoint = deploy_script(
        &cfg.ckb_rpc,
        &cfg.address,
        secret_key,
        "./build/release/ckb_sudt_script",
    )?;

    println!(
        "Save this! Code OutPoint tx: {:#x}",
        code_outpoint.tx_hash()
    );

    mint_tokens(
        &cfg.ckb_rpc,
        code_outpoint,
        &cfg.address,
        secret_key,
        &cfg.address,
        1_000_000,
    )?;

    Ok(())
}
