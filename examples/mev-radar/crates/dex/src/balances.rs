//! DEX-agnostic amount / mint extraction from token-balance deltas.
//!
//! Yellowstone supplies `pre_token_balances` and `post_token_balances` on
//! every successful transaction. Diffing those for a given owner gives
//! us `(mint_in, amount_in)` and `(mint_out, amount_out)` without having
//! to byte-decode each DEX's instruction layout.
//!
//! Limitations of this approach (acceptable for v0.1):
//!
//! - Multi-hop swaps that touch the same wallet across multiple pools
//!   in one tx will collapse to a single net delta. The arb / sandwich
//!   detectors care about per-instruction events, so until we have IDL
//!   parsing this can over-attribute amounts to one of the swaps in a
//!   route. Single-pool swaps (the common case) are exact.
//! - Wrapped-SOL legs require a separate `native_sol_delta` pass; this
//!   is left as a v0.2 follow-up.

use yellowstone_grpc_proto::solana::storage::confirmed_block::TokenBalance;
use yellowstone_vixen_core::instruction::InstructionUpdate;

use crate::event::{Dex, SwapEvent};

pub(crate) fn derive_swap_event(
    ix: &InstructionUpdate,
    dex: Dex,
    signer: &str,
    pool: &str,
) -> Option<SwapEvent> {
    let shared = &ix.shared;

    if shared.err.is_some() {
        return None;
    }

    let signature = bs58::encode(&shared.signature).into_string();
    let slot = shared.slot;

    let (mint_out, amount_out) = max_positive_delta(
        &shared.pre_token_balances,
        &shared.post_token_balances,
        signer,
    )?;
    let (mint_in, amount_in) = max_negative_delta(
        &shared.pre_token_balances,
        &shared.post_token_balances,
        signer,
    )?;

    Some(SwapEvent {
        dex,
        slot,
        signature,
        signer: signer.to_string(),
        pool: pool.to_string(),
        mint_in,
        mint_out,
        amount_in,
        amount_out,
    })
}

fn max_positive_delta(
    pre: &[TokenBalance],
    post: &[TokenBalance],
    owner: &str,
) -> Option<(String, u64)> {
    delta(pre, post, owner)
        .filter(|&(_, d)| d > 0)
        .max_by_key(|&(_, d)| d)
        .and_then(|(mint, d)| u64::try_from(d).ok().map(|v| (mint, v)))
}

fn max_negative_delta(
    pre: &[TokenBalance],
    post: &[TokenBalance],
    owner: &str,
) -> Option<(String, u64)> {
    delta(pre, post, owner)
        .filter(|&(_, d)| d < 0)
        .min_by_key(|&(_, d)| d)
        .and_then(|(mint, d)| u64::try_from(d.unsigned_abs()).ok().map(|v| (mint, v)))
}

fn delta<'a>(
    pre: &'a [TokenBalance],
    post: &'a [TokenBalance],
    owner: &'a str,
) -> impl Iterator<Item = (String, i128)> + 'a {
    post.iter()
        .filter(move |b| b.owner == owner)
        .filter_map(move |post_b| {
            let post_amt = post_b.ui_token_amount.as_ref()?.amount.parse::<u128>().ok()?;
            let pre_amt = pre
                .iter()
                .find(|p| p.account_index == post_b.account_index && p.owner == owner)
                .and_then(|p| p.ui_token_amount.as_ref())
                .and_then(|u| u.amount.parse::<u128>().ok())
                .unwrap_or(0);

            let d = post_amt as i128 - pre_amt as i128;

            Some((post_b.mint.clone(), d))
        })
}
