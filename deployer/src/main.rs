mod config;
mod deploy_script;
pub mod spend;

use config::Config;
use deploy_script::{deploy_script, mint_tokens};

fn main() -> anyhow::Result<()> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "deploy-sudt".to_string());

    let args = std::env::args().skip(2);
    let config = Config::from_env()?;
    match command.as_str() {
        "deploy-sudt" => {
            deploy_and_mint_sudt()?;
        }
        "deploy-simple-lock" => {
            spend::deploy_simple_lock()?;
        }
        "spend-simple-lock" => {
            spend::spend_simple_lock(args)?;
        }
        "create-locked-cell" => {
            deploy_script::create_locked_cell(args, &config)?;
        }
        other => anyhow::bail!("unknown command: {other}"),
    }

    Ok(())
}

pub fn deploy_and_mint_sudt() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    let secret_key = cfg.secret_key()?;

    println!("Deploying from: {}", cfg.address);
    println!("RPC endpoint:   {}", cfg.ckb_rpc);

    let code_outpoint = deploy_script(
        &cfg.ckb_rpc,
        &cfg.address,
        secret_key,
        "./build/release/ckb_sudt_script",
        None,
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
