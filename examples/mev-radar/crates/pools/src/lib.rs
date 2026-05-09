//! Pool / pair price state for mev-radar.
//!
//! v0.1 derives prices from observed swaps (DEX-agnostic), avoiding byte
//! layout decoding of each DEX's pool accounts. Trade-off: we only update
//! prices on the slots a pool actually swaps in, so freshly-quoted pools
//! lag behind. For arb-scout / sandwich-detection use cases this is
//! sufficient because both rely on swap activity to fire.
//!
//! v0.2 will add account-data decoding so the price refreshes on every
//! pool reserve change, not just every swap.

use std::collections::HashMap;

use mev_radar_dex::{Dex, SwapEvent};
use serde::{Deserialize, Serialize};

/// A canonical pair key — sorted mints so `(A, B)` and `(B, A)` collide.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pair(pub String, pub String);

impl Pair {
    #[must_use]
    pub fn new(a: &str, b: &str) -> Self {
        if a <= b {
            Self(a.to_string(), b.to_string())
        } else {
            Self(b.to_string(), a.to_string())
        }
    }

    #[must_use]
    pub fn base(&self) -> &str { &self.0 }

    #[must_use]
    pub fn quote(&self) -> &str { &self.1 }
}

/// Latest observed price for one (pool, pair) pairing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolQuote {
    pub dex: Dex,
    pub pool: String,
    pub pair: Pair,
    /// Price in `quote_per_base` units (scaled by mint decimals — left to
    /// the caller because mint decimals are not on the SwapEvent).
    pub price: f64,
    pub slot: u64,
    pub last_signature: String,
}

/// Map from `(pool, pair)` to latest [`PoolQuote`].
#[derive(Debug, Default)]
pub struct PoolMap {
    quotes: HashMap<(String, Pair), PoolQuote>,
}

impl PoolMap {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Update the map from a [`SwapEvent`]. Returns the new quote if the
    /// event produced a parseable price, else `None` (e.g. divide-by-zero).
    pub fn ingest(&mut self, ev: &SwapEvent) -> Option<&PoolQuote> {
        if ev.amount_in == 0 {
            return None;
        }

        let pair = Pair::new(&ev.mint_in, &ev.mint_out);

        // Convention: price is `quote_per_base`. We orient so the
        // alphabetically-smaller mint is base.
        let price = if pair.base() == ev.mint_in {
            ev.amount_out as f64 / ev.amount_in as f64
        } else {
            ev.amount_in as f64 / ev.amount_out as f64
        };

        if !price.is_finite() || price <= 0.0 {
            return None;
        }

        let key = (ev.pool.clone(), pair.clone());
        let quote = PoolQuote {
            dex: ev.dex,
            pool: ev.pool.clone(),
            pair,
            price,
            slot: ev.slot,
            last_signature: ev.signature.clone(),
        };

        self.quotes.insert(key.clone(), quote);
        self.quotes.get(&key)
    }

    /// All quotes for a pair across all pools.
    #[must_use]
    pub fn quotes_for_pair(&self, pair: &Pair) -> Vec<&PoolQuote> {
        self.quotes
            .iter()
            .filter_map(|((_pool, p), q)| (p == pair).then_some(q))
            .collect()
    }

    /// Iterate all pairs currently tracked.
    pub fn pairs(&self) -> impl Iterator<Item = &Pair> {
        self.quotes.keys().map(|(_p, pair)| pair)
    }

    /// Iterate every (pool, pair) → quote entry. The arb detector needs
    /// this to bucket by pair without hitting `quotes_for_pair` once
    /// per pair (which is O(N) each).
    pub fn iter_quotes(&self) -> impl Iterator<Item = (&(String, Pair), &PoolQuote)> {
        self.quotes.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize { self.quotes.len() }

    #[must_use]
    pub fn is_empty(&self) -> bool { self.quotes.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(pool: &str, mint_in: &str, mint_out: &str, ain: u64, aout: u64) -> SwapEvent {
        SwapEvent {
            dex: Dex::RaydiumAmmV4,
            slot: 1,
            signature: "sig".into(),
            signer: "signer".into(),
            pool: pool.into(),
            mint_in: mint_in.into(),
            mint_out: mint_out.into(),
            amount_in: ain,
            amount_out: aout,
        }
    }

    #[test]
    fn pair_canonicalizes() {
        let a = Pair::new("Z", "A");
        let b = Pair::new("A", "Z");
        assert_eq!(a, b);
        assert_eq!(a.base(), "A");
        assert_eq!(a.quote(), "Z");
    }

    #[test]
    fn ingest_records_price() {
        let mut m = PoolMap::new();

        // Sold 100 USDC, got 1 SOL. Pair canonicalizes to ("SOL", "USDC")
        // because "SOL" < "USDC" alphabetically, so quote-per-base
        // = USDC per SOL = 100.
        m.ingest(&ev("pool1", "USDC", "SOL", 100_000_000, 1_000_000));

        let pair = Pair::new("USDC", "SOL");
        let qs = m.quotes_for_pair(&pair);
        assert_eq!(qs.len(), 1);
        assert!((qs[0].price - 100.0).abs() < 1e-9);
    }
}
