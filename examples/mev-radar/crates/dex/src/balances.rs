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
//!   in one tx will collapse to a single net delta. The `collect_swaps`
//!   walker dedupes by signature so a Jupiter-style route emits at most
//!   one event per tx — we lose per-leg granularity until v0.2 IDL
//!   parsing lands, but at least we don't pollute downstream detectors
//!   with N copies of the same numbers.
//! - Wrapped-SOL legs require a separate `native_sol_delta` pass; this
//!   is left as a v0.2 follow-up.

use std::collections::BTreeMap;

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

    let deltas = collect_deltas(&shared.pre_token_balances, &shared.post_token_balances, signer);

    let (mint_out, amount_out) = max_positive(&deltas)?;
    let (mint_in, amount_in) = max_negative(&deltas)?;

    let signature = bs58::encode(&shared.signature).into_string();

    Some(SwapEvent {
        dex,
        slot: shared.slot,
        signature,
        signer: signer.to_string(),
        pool: pool.to_string(),
        mint_in,
        mint_out,
        amount_in,
        amount_out,
    })
}

fn max_positive(deltas: &[(String, i128)]) -> Option<(String, u64)> {
    deltas
        .iter()
        .filter(|(_, d)| *d > 0)
        .max_by_key(|(_, d)| *d)
        .and_then(|(mint, d)| u64::try_from(*d).ok().map(|v| (mint.clone(), v)))
}

fn max_negative(deltas: &[(String, i128)]) -> Option<(String, u64)> {
    deltas
        .iter()
        .filter(|(_, d)| *d < 0)
        .min_by_key(|(_, d)| *d)
        .and_then(|(mint, d)| u64::try_from(d.unsigned_abs()).ok().map(|v| (mint.clone(), v)))
}

/// Compute net token-balance deltas for the given owner across pre + post.
///
/// Iterates the **union** of (account_index, mint) pairs from both pre
/// and post snapshots, so closed accounts (present only in pre) and
/// freshly-opened accounts (present only in post) are both reflected.
fn collect_deltas(pre: &[TokenBalance], post: &[TokenBalance], owner: &str) -> Vec<(String, i128)> {
    type Slot = (Option<u128>, Option<String>);

    let mut pre_by_idx: BTreeMap<u32, Slot> = BTreeMap::new();
    let mut post_by_idx: BTreeMap<u32, Slot> = BTreeMap::new();

    for b in pre {
        if b.owner != owner {
            continue;
        }
        let amt = b.ui_token_amount.as_ref().and_then(|u| u.amount.parse::<u128>().ok());
        pre_by_idx.insert(b.account_index, (amt, Some(b.mint.clone())));
    }

    for b in post {
        if b.owner != owner {
            continue;
        }
        let amt = b.ui_token_amount.as_ref().and_then(|u| u.amount.parse::<u128>().ok());
        post_by_idx.insert(b.account_index, (amt, Some(b.mint.clone())));
    }

    let indices: std::collections::BTreeSet<u32> =
        pre_by_idx.keys().chain(post_by_idx.keys()).copied().collect();

    let mut out = Vec::with_capacity(indices.len());
    for idx in indices {
        let pre_amt = pre_by_idx.get(&idx).and_then(|(a, _)| *a).unwrap_or(0);
        let post_amt = post_by_idx.get(&idx).and_then(|(a, _)| *a).unwrap_or(0);

        let mint = post_by_idx
            .get(&idx)
            .and_then(|(_, m)| m.clone())
            .or_else(|| pre_by_idx.get(&idx).and_then(|(_, m)| m.clone()));

        let Some(mint) = mint else { continue };
        let d = post_amt as i128 - pre_amt as i128;
        if d != 0 {
            out.push((mint, d));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use yellowstone_grpc_proto::solana::storage::confirmed_block::UiTokenAmount;

    use super::*;

    fn tb(idx: u32, owner: &str, mint: &str, amount: &str) -> TokenBalance {
        TokenBalance {
            account_index: idx,
            mint: mint.into(),
            owner: owner.into(),
            program_id: String::new(),
            ui_token_amount: Some(UiTokenAmount {
                ui_amount: 0.0,
                decimals: 0,
                amount: amount.into(),
                ui_amount_string: String::new(),
            }),
        }
    }

    #[test]
    fn delta_picks_up_closed_account() {
        // Owner has 1000 USDC in pre on account index 0, but the swap
        // closes that account (it's gone in post); receives 5 SOL on
        // account index 1, which is freshly opened (only in post).
        let pre = vec![tb(0, "alice", "USDC", "1000")];
        let post = vec![tb(1, "alice", "SOL", "5")];

        let deltas = collect_deltas(&pre, &post, "alice");

        // Expect both legs: -1000 USDC and +5 SOL
        let usdc = deltas.iter().find(|(m, _)| m == "USDC").expect("USDC delta missing");
        let sol = deltas.iter().find(|(m, _)| m == "SOL").expect("SOL delta missing");

        assert_eq!(usdc.1, -1000);
        assert_eq!(sol.1, 5);

        assert_eq!(max_negative(&deltas), Some(("USDC".into(), 1000)));
        assert_eq!(max_positive(&deltas), Some(("SOL".into(), 5)));
    }

    #[test]
    fn delta_ignores_other_owners() {
        let pre = vec![tb(0, "alice", "USDC", "1000"), tb(1, "bob", "USDC", "500")];
        let post = vec![tb(0, "alice", "USDC", "200"), tb(1, "bob", "USDC", "1300")];

        let alice = collect_deltas(&pre, &post, "alice");
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0], ("USDC".into(), -800));
    }

    #[test]
    fn unchanged_balances_yield_no_delta() {
        let pre = vec![tb(0, "alice", "USDC", "1000")];
        let post = vec![tb(0, "alice", "USDC", "1000")];

        let deltas = collect_deltas(&pre, &post, "alice");
        assert!(deltas.is_empty());
    }
}
