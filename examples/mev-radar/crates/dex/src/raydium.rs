//! Raydium AMM v4 swap detection.
//!
//! v4 instruction layout (per the open-source `raydium-amm` repo):
//!
//! - `0x09` — `swapBaseIn`
//! - `0x0b` — `swapBaseOut`
//!
//! Discriminator is the first byte of `ix.data`. Account layout for both
//! variants ends with `[..., user_source_token, user_destination_token,
//! user_source_owner]`. The `pool` (AMM ID) sits at index 1.
//!
//! Amounts and mints come from token-balance deltas keyed on
//! `user_source_owner` so the parser stays DEX-agnostic. This is robust to
//! future v4 layout tweaks but does require post-execution metadata, which
//! Yellowstone gRPC supplies on the regular `transactions` stream.

use yellowstone_vixen_core::instruction::InstructionUpdate;

use crate::{
    event::{Dex, SwapEvent},
    program_ids::raydium_amm_v4_program,
};

const DISC_SWAP_BASE_IN: u8 = 0x09;
const DISC_SWAP_BASE_OUT: u8 = 0x0b;

/// Detect a Raydium AMM v4 swap and decode it into a [`SwapEvent`].
///
/// Returns `None` if the instruction is not on Raydium AMM v4 or the
/// discriminator doesn't match a swap.
#[must_use]
pub fn try_parse_swap(ix: &InstructionUpdate) -> Option<SwapEvent> {
    if !ix.program.equals_ref(raydium_amm_v4_program()) {
        return None;
    }

    let disc = *ix.data.first()?;
    if disc != DISC_SWAP_BASE_IN && disc != DISC_SWAP_BASE_OUT {
        return None;
    }

    if ix.accounts.len() < 4 {
        return None;
    }

    let pool = bs58::encode(ix.accounts[1].as_slice()).into_string();

    // The signing wallet is the last account in v4 swap layouts.
    let signer = ix
        .accounts
        .last()
        .map(|pk| bs58::encode(pk.as_slice()).into_string())?;

    crate::balances::derive_swap_event(ix, Dex::RaydiumAmmV4, &signer, &pool)
}
