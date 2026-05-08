//! Orca Whirlpools swap detection.
//!
//! Anchor instructions are prefixed with an 8-byte discriminator
//! `sha256("global:<name>")[..8]`. We match `swap` and `swapV2` here.
//!
//! Per the Whirlpools IDL, the swap account layout puts the `Whirlpool`
//! account at index 2 and the user's `token_authority` (signer) at index
//! 1 (`swap`) or index 1 (`swapV2`). We extract those positions and
//! defer amount / mint decoding to token-balance deltas, identical to
//! the Raydium parser.

use yellowstone_vixen_core::instruction::InstructionUpdate;

use crate::{
    event::{Dex, SwapEvent},
    program_ids::whirlpools_program,
};

/// `sha256("global:swap")[..8]`.
const DISC_SWAP: [u8; 8] = [0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8];

/// `sha256("global:swapV2")[..8]`.
const DISC_SWAP_V2: [u8; 8] = [0x2b, 0x04, 0xed, 0x0b, 0x1a, 0xc9, 0x1e, 0x62];

#[must_use]
pub fn try_parse_swap(ix: &InstructionUpdate) -> Option<SwapEvent> {
    if !ix.program.equals_ref(whirlpools_program()) {
        return None;
    }

    if ix.data.len() < 8 {
        return None;
    }
    let disc = &ix.data[..8];
    if disc != DISC_SWAP && disc != DISC_SWAP_V2 {
        return None;
    }

    // `swap` accounts (per IDL):
    //   [0] token_program
    //   [1] token_authority  (signer)
    //   [2] whirlpool        (pool)
    //   ...
    if ix.accounts.len() < 3 {
        return None;
    }

    let signer = bs58::encode(ix.accounts[1].as_slice()).into_string();
    let pool = bs58::encode(ix.accounts[2].as_slice()).into_string();

    crate::balances::derive_swap_event(ix, Dex::Whirlpools, &signer, &pool)
}
