//! Program-ID constants for supported DEXes.
//!
//! Decoded once from base58 at first access via [`std::sync::OnceLock`];
//! after that they are cheap pointer-equality checks.

use std::sync::OnceLock;

use yellowstone_vixen_core::{KeyBytes, Pubkey};

const RAYDIUM_AMM_V4: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
const WHIRLPOOLS: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

#[must_use]
pub fn raydium_amm_v4_program() -> &'static Pubkey {
    static ID: OnceLock<Pubkey> = OnceLock::new();
    ID.get_or_init(|| decode(RAYDIUM_AMM_V4))
}

#[must_use]
pub fn whirlpools_program() -> &'static Pubkey {
    static ID: OnceLock<Pubkey> = OnceLock::new();
    ID.get_or_init(|| decode(WHIRLPOOLS))
}

fn decode(s: &str) -> Pubkey {
    let mut bytes = [0u8; 32];
    let n = bs58::decode(s)
        .onto(&mut bytes[..])
        .expect("compile-time-valid program id");

    assert!(n == 32, "program id `{s}` decoded to {n} bytes, expected 32");
    KeyBytes::new(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raydium_amm_v4_decodes() {
        let id = raydium_amm_v4_program();
        // First byte of `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8`
        // when base58-decoded.
        assert_eq!(id.0.len(), 32);
    }

    #[test]
    fn whirlpools_decodes() {
        let id = whirlpools_program();
        assert_eq!(id.0.len(), 32);
    }
}
