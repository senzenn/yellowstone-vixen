//! Orca Whirlpools swap detection.
//!
//! Anchor instructions are prefixed with an 8-byte discriminator
//! `sha256("global:<name>")[..8]`. We match `swap` and `swapV2` here.
//! The two have different account layouts (swapV2 prefixes two token
//! programs + a memo/transfer-hook account before the signer), so we
//! branch on the discriminator to pick the right slot for `signer`
//! and `whirlpool`.

use yellowstone_vixen_core::instruction::InstructionUpdate;

use crate::{
    event::{Dex, SwapEvent},
    program_ids::whirlpools_program,
};

/// `sha256("global:swap")[..8]`.
const DISC_SWAP: [u8; 8] = [0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8];

/// `sha256("global:swapV2")[..8]`.
const DISC_SWAP_V2: [u8; 8] = [0x2b, 0x04, 0xed, 0x0b, 0x1a, 0xc9, 0x1e, 0x62];

/// Account index of `tokenAuthority` (signer) in each variant.
const SWAP_SIGNER_IDX: usize = 1;
const SWAP_POOL_IDX: usize = 2;

const SWAP_V2_SIGNER_IDX: usize = 3;
const SWAP_V2_POOL_IDX: usize = 4;

#[must_use]
pub fn try_parse_swap(ix: &InstructionUpdate) -> Option<SwapEvent> {
    if !ix.program.equals_ref(whirlpools_program()) {
        return None;
    }

    if ix.data.len() < 8 {
        return None;
    }

    let disc = &ix.data[..8];
    let (signer_idx, pool_idx) = if disc == DISC_SWAP {
        (SWAP_SIGNER_IDX, SWAP_POOL_IDX)
    } else if disc == DISC_SWAP_V2 {
        (SWAP_V2_SIGNER_IDX, SWAP_V2_POOL_IDX)
    } else {
        return None;
    };

    if ix.accounts.len() <= pool_idx {
        return None;
    }

    let signer = bs58::encode(ix.accounts[signer_idx].as_slice()).into_string();
    let pool = bs58::encode(ix.accounts[pool_idx].as_slice()).into_string();

    crate::balances::derive_swap_event(ix, Dex::Whirlpools, &signer, &pool)
}
