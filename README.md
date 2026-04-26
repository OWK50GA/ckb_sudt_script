# CKB sUDT

A Simple User Defined Token (sUDT) implementation on Nervos CKB, written in Rust. Includes the on-chain type script, a test suite, and an off-chain deployer binary.

## What is sUDT?

sUDT is CKB's standard for fungible tokens. Unlike ERC-20, there is no global contract — each token is identified by its **type script**, and token amounts live in the **data field** of individual cells. The type script enforces conservation: you cannot create tokens out of thin air unless you are the designated owner.

## Project Structure

```
.
├── contracts/
│   └── ckb_sudt_script/       # The on-chain type script (RISC-V, no_std)
│       └── src/
│           ├── main.rs        # Core sUDT logic
│           └── error.rs       # Error codes
├── deployer/                  # Off-chain tooling (std Rust, runs on your machine)
│   └── src/
│       ├── main.rs            # Deploy + mint entry point
│       ├── config.rs          # .env loader
│       ├── deploy_script.rs   # deploy_script / mint_tokens / transfer_tokens
│       └── bin/
│           └── keygen.rs      # One-time key + address generator
└── tests/                     # Integration tests using ckb-testtool
    └── src/
        └── tests.rs
```

## How the sUDT Script Works

The type script is identified by a `code_hash` (blake2b-256 of the binary) and carries a single argument: the **owner lock hash** — a 32-byte blake2b-256 hash of the issuer's lock script.

On every transaction involving sUDT cells, the script runs and enforces one of two modes:

**Normal mode** (anyone transferring tokens):
- Sum all input amounts across cells sharing this type script
- Sum all output amounts
- Reject if `output_sum > input_sum` (no inflation)
- Burning (output < input) is allowed

**Owner mode** (the issuer minting or burning):
- Triggered when any input cell has a **type script whose hash equals the owner lock hash**
- All conservation checks are skipped — the owner can mint freely

Token amounts are stored as **16-byte little-endian `u128`** at the start of each cell's data field.

### Error Codes

| Code | Name | Meaning |
|------|------|---------|
| 4 | Encoding | Cell data < 16 bytes, cannot decode amount |
| 10 | ArgsLength | Script args are not exactly 32 bytes |
| 11 | Overflow | Token sum overflowed u128 |
| 12 | OutputOverflow | Output tokens exceed input tokens |

## Prerequisites

```bash
# Install the RISC-V target for the on-chain contract
rustup target add riscv64imac-unknown-none-elf

# Clang is required to compile C dependencies (ckb-std uses it)
# On Ubuntu/Debian:
sudo apt install clang
# On macOS (Homebrew):
brew install llvm
```

## Building the Contract

```bash
make build
```

The compiled binary is placed at `build/release/ckb_sudt_script`.

## Running Tests

Tests run against the compiled binary using `ckb-testtool`, which simulates the CKB VM locally.

```bash
# Build the contract first, then run tests
make build
cargo test --package tests
```

The test suite covers:

- Equal-amount transfer (pass)
- Burning tokens — output < input (pass)
- Owner mode minting via governance cell (pass)
- Multi-cell balanced transfer (pass)
- Inflation attack — output > input (fail, error 12)
- Invalid args length (fail, error 10)
- Malformed cell data < 16 bytes (fail, error 4)

## Deploying to Testnet

### 1. Generate a key and address

Run this once and save the output securely:

```bash
cargo run --bin keygen
```

Output:
```
PRIVATE_KEY=0xabc123...
TESTNET_ADDRESS=ckt1qz...
MAINNET_ADDRESS=ckb1qz...

Fund your testnet address at:
https://faucet.nervos.org/?address=ckt1qz...
```

### 2. Create a `.env` file at the workspace root

```bash
PRIVATE_KEY=0xabc123...
TESTNET_ADDRESS=ckt1qz...
CKB_RPC=https://testnet.ckb.dev:8114
```

**Never commit this file.** It is already in `.gitignore`.

### 3. Fund your address

Visit the faucet URL printed by `keygen` and request testnet CKB. Wait for the transaction to confirm (usually under a minute).

### 4. Deploy and mint

```bash
cargo run --bin deployer
```

This will:
1. Deploy the `ckb_sudt_script` binary as a code cell on testnet
2. Print the code cell outpoint — **save this**, you need it to reference the script later
3. Mint 1,000,000 tokens to your own address

## Key Design Notes

**Scripts are cells.** There is no special "contract account". The compiled RISC-V binary is stored in the `data` field of an ordinary cell. Other cells reference it by `code_hash` (a hash of that data).

**The deployer does not add `secp256k1` as a direct dependency.** It uses the version re-exported transitively through `ckb-sdk` to avoid type mismatches from duplicate crate versions in the dependency graph.

**`ckb-hash` is not used in the contract.** It pulls in `blake2b-rs` which requires a C compiler for cross-compilation to RISC-V. The contract reads a pre-computed hash from its args instead. If you need to hash inside a script, use `blake2b-ref` (pure Rust).

## Workspace Crate Versions

The CKB ecosystem has two active version series that are not compatible with each other in the same dependency graph:

| Series | Used by |
|--------|---------|
| `ckb-*` `1.x` / `ckb-types 1.1` | `ckb-testtool 1.1`, `ckb-std 1.1` |
| `ckb-*` `0.202.x` | `ckb-sdk 4.x` |

The `tests` crate uses the `1.x` series. The `deployer` crate uses `0.202.x`. They coexist in this workspace because they do not share any crates that have exact-version pins in common — except `ckb-vm`, which is why `ckb-sdk 3.x` cannot be used here (it pins `ckb-vm = 0.24.13` while `ckb-testtool 1.1` pins `ckb-vm = 0.24.14`).
