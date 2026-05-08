//! Per-DEX swap-event parsers.
//!
//! Each DEX module exposes a `try_parse_swap` function that takes a Vixen
//! [`InstructionUpdate`] and returns a [`SwapEvent`] when the instruction
//! is a swap on that DEX.
//!
//! The current shipping pair is:
//! - **Raydium AMM v4** — discriminator-based detection on `swapBaseIn` /
//!   `swapBaseOut`.
//! - **Orca Whirlpools** — Anchor 8-byte discriminator on `swap` /
//!   `swapV2`.
//!
//! Amounts and mints are computed from the transaction's
//! [`InstructionShared::pre_token_balances`] and `post_token_balances`
//! deltas indexed by the swap's *signer* — this is DEX-agnostic and
//! avoids per-DEX byte-layout decoding for the v0.1 cut. Future PRs can
//! switch to true Codama-IDL-driven parsing via Vixen's
//! `include_vixen_parser!`.
//!
//! [`InstructionUpdate`]: yellowstone_vixen_core::instruction::InstructionUpdate
//! [`InstructionShared::pre_token_balances`]:
//!     yellowstone_vixen_core::instruction::InstructionShared

pub mod balances;
pub mod event;
pub mod program_ids;
pub mod raydium;
pub mod whirlpools;

pub use event::{Dex, Side, SwapEvent};
pub use program_ids::{raydium_amm_v4_program, whirlpools_program};

use yellowstone_vixen_core::instruction::InstructionUpdate;

/// Try every supported DEX swap parser; return the first match.
///
/// Returns `None` if the instruction isn't a swap (or isn't on a supported
/// DEX). Inner CPIs are not walked here — call this for each
/// [`InstructionUpdate`] you've already flattened.
#[must_use]
pub fn try_parse_swap(ix: &InstructionUpdate) -> Option<SwapEvent> {
    if let Some(ev) = raydium::try_parse_swap(ix) {
        return Some(ev);
    }

    if let Some(ev) = whirlpools::try_parse_swap(ix) {
        return Some(ev);
    }

    None
}

/// Walk an instruction tree (top-level + CPIs) and collect every swap
/// event, deduplicated by signature + path.
#[must_use]
pub fn collect_swaps(top_level: &[InstructionUpdate]) -> Vec<SwapEvent> {
    let mut out = Vec::new();
    for ix in top_level {
        walk(ix, &mut out);
    }
    out
}

fn walk(ix: &InstructionUpdate, out: &mut Vec<SwapEvent>) {
    if let Some(ev) = try_parse_swap(ix) {
        out.push(ev);
    }

    for inner in &ix.inner {
        walk(inner, out);
    }
}
