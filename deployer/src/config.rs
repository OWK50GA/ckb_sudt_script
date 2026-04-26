use anyhow::{Context, Result};

pub struct Config {
    pub private_key_bytes: [u8; 32],
    pub address: String,
    pub ckb_rpc: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // Load .env — silently ignored if missing (e.g. in CI)
        dotenvy::dotenv().ok();

        let private_key_hex =
            std::env::var("PRIVATE_KEY").context("PRIVATE_KEY not set in .env")?;

        let address =
            std::env::var("TESTNET_ADDRESS").context("TESTNET_ADDRESS not set in .env")?;

        let ckb_rpc =
            std::env::var("CKB_RPC").unwrap_or_else(|_| "https://testnet.ckb.dev:8114".to_string());

        let hex_str = private_key_hex.trim_start_matches("0x");
        let key_bytes = hex::decode(hex_str).context("PRIVATE_KEY is not valid hex")?;

        let private_key_bytes: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("PRIVATE_KEY must be exactly 32 bytes (64 hex chars)"))?;

        Ok(Self {
            private_key_bytes,
            address,
            ckb_rpc,
        })
    }

    pub fn secret_key(&self) -> Result<secp256k1::SecretKey> {
        secp256k1::SecretKey::from_byte_array(&self.private_key_bytes)
            .context("Invalid private key bytes")
    }
}
