//! Cross-pool arbitrage scout.
//!
//! Watches a [`PoolMap`](mev_radar_pools::PoolMap) and emits an
//! [`ArbEvent`] every time the spread between any two pools quoting the
//! same pair exceeds a configurable basis-point threshold.
//!
//! v0.1 prices are derived from latest observed swap (DEX-agnostic
//! amount-deltas), so the scout signals on **executed** spreads, not
//! resting-orderbook spreads. Bookable depth is out of scope for v0.1.

use std::collections::HashMap;

use mev_radar_dex::Dex;
use mev_radar_pools::{Pair, PoolMap, PoolQuote};
use serde::{Deserialize, Serialize};

/// One detected arb opportunity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArbEvent {
    pub pair: Pair,
    pub slot: u64,
    pub spread_bps: u32,
    pub buy_dex: Dex,
    pub buy_pool: String,
    pub buy_price: f64,
    pub sell_dex: Dex,
    pub sell_pool: String,
    pub sell_price: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct ArbConfig {
    pub min_spread_bps: u32,
}

impl Default for ArbConfig {
    fn default() -> Self { Self { min_spread_bps: 30 } }
}

/// Detect arb opportunities in the [`PoolMap`].
///
/// Algorithm: bucket quotes by pair, then for each pair compute the
/// **best** arb — lowest-price buy pool vs highest-price sell pool —
/// and emit if the spread crosses `min_spread_bps`. Per-pair work is
/// O(P log P) for the sort, so the whole detector is
/// O(N + Σ P_i log P_i) where N is the total quote count and P_i is
/// the number of pools quoting pair i.
///
/// Output is **sorted deterministically** by `(slot, pair, spread_bps
/// desc)` so golden-file replay tests are stable across runs and across
/// HashMap iteration-order changes.
///
/// Returns at most one event per pair; if you need every pair-pair
/// combination above threshold you'd want to change this to a windowed
/// emit (not the v0.1 product shape).
#[must_use]
pub fn detect(map: &PoolMap, cfg: ArbConfig) -> Vec<ArbEvent> {
    let mut by_pair: HashMap<&Pair, Vec<&PoolQuote>> = HashMap::new();

    for ((_pool, pair), quote) in map.iter_quotes() {
        by_pair.entry(pair).or_default().push(quote);
    }

    let mut out = Vec::with_capacity(by_pair.len());

    for (pair, mut quotes) in by_pair {
        if quotes.len() < 2 {
            continue;
        }

        // Sort ascending by price — buy is first, sell is last.
        quotes.sort_by(|a, b| {
            a.price
                .partial_cmp(&b.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let buy = quotes[0];
        let sell = *quotes.last().expect("len >= 2");

        let bps = spread_bps(buy.price, sell.price);
        if bps < cfg.min_spread_bps {
            continue;
        }

        let _ = pair;
        out.push(arb_event(buy, sell, bps));
    }

    // Stable output ordering — see U-01 in AUDIT.md.
    out.sort_by(|a, b| {
        a.slot
            .cmp(&b.slot)
            .then_with(|| a.pair.0.cmp(&b.pair.0))
            .then_with(|| a.pair.1.cmp(&b.pair.1))
            .then_with(|| b.spread_bps.cmp(&a.spread_bps))
    });

    out
}

fn spread_bps(buy: f64, sell: f64) -> u32 {
    if buy <= 0.0 {
        return 0;
    }
    let pct = (sell - buy) / buy;
    let bps = (pct * 10_000.0).round();
    bps.max(0.0).min(u32::MAX as f64) as u32
}

fn arb_event(buy: &PoolQuote, sell: &PoolQuote, bps: u32) -> ArbEvent {
    ArbEvent {
        pair: buy.pair.clone(),
        slot: buy.slot.max(sell.slot),
        spread_bps: bps,
        buy_dex: buy.dex,
        buy_pool: buy.pool.clone(),
        buy_price: buy.price,
        sell_dex: sell.dex,
        sell_pool: sell.pool.clone(),
        sell_price: sell.price,
    }
}

#[cfg(test)]
mod tests {
    use mev_radar_dex::SwapEvent;

    use super::*;

    fn ev(pool: &str, dex: Dex, ain: u64, aout: u64) -> SwapEvent {
        SwapEvent {
            dex,
            slot: 1,
            signature: "sig".into(),
            signer: "signer".into(),
            pool: pool.into(),
            mint_in: "USDC".into(),
            mint_out: "SOL".into(),
            amount_in: ain,
            amount_out: aout,
        }
    }

    #[test]
    fn detects_30bps_spread() {
        let mut map = PoolMap::new();
        // Pool A: 1_000_000 USDC -> 10_000 SOL  → price 100 USDC/SOL
        map.ingest(&ev("A", Dex::RaydiumAmmV4, 1_000_000, 10_000));
        // Pool B: 1_003_000 USDC -> 10_000 SOL  → price 100.3 USDC/SOL (+30 bps)
        map.ingest(&ev("B", Dex::Whirlpools, 1_003_000, 10_000));

        let events = detect(&map, ArbConfig { min_spread_bps: 30 });
        assert_eq!(events.len(), 1);
        assert!(events[0].spread_bps >= 30);
    }

    #[test]
    fn ignores_below_threshold() {
        let mut map = PoolMap::new();
        map.ingest(&ev("A", Dex::RaydiumAmmV4, 1_000_000, 10_000));
        // +1 bps only → below 30 bps threshold
        map.ingest(&ev("B", Dex::Whirlpools, 1_000_100, 10_000));

        let events = detect(&map, ArbConfig { min_spread_bps: 30 });
        assert!(events.is_empty());
    }
}
