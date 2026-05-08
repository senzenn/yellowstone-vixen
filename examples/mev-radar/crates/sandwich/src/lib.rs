//! Sandwich-MEV classifier.
//!
//! Streaming detector. Each [`SwapEvent`] is fed in via [`Detector::push`]
//! and the detector buffers the last N slots' worth of events per pool.
//! On every new event we check if any prior swap from the same signer on
//! the same pool can be paired with the new one as `[front, victim,
//! back]`:
//!
//! 1. `front` and `back` share a signer, share a pool, and have inverse
//!    `(mint_in, mint_out)` directions.
//! 2. There is **at least one** other swap on the same pool **between**
//!    `front` and `back` (the victim) that is not from the same signer.
//!
//! When all three are present we emit a [`SandwichEvent`] with the
//! attacker's amounts and the victim's signature(s).
//!
//! v0.1 ignores fee accounting (we don't have post-USD prices yet) — the
//! event reports raw token amounts and a coarse `extracted_amount`
//! (`back.amount_out - front.amount_in`).

use std::collections::VecDeque;

use mev_radar_dex::SwapEvent;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandwichEvent {
    pub slot: u64,
    pub pool: String,
    pub attacker: String,
    pub front_signature: String,
    pub back_signature: String,
    pub victim_signatures: Vec<String>,
    pub front_amount_in: u64,
    pub back_amount_out: u64,
    /// Coarse value extracted: `back_amount_out - front_amount_in`, in
    /// `mint_in` units (same mint on both legs by construction).
    pub extracted_amount: i128,
}

#[derive(Debug, Clone, Copy)]
pub struct SandwichConfig {
    pub window_slots: u64,
}

impl Default for SandwichConfig {
    fn default() -> Self { Self { window_slots: 1 } }
}

#[derive(Debug, Default)]
pub struct Detector {
    cfg: SandwichConfig,
    /// Per-pool ring of recent events.
    by_pool: std::collections::HashMap<String, VecDeque<SwapEvent>>,
}

impl Detector {
    #[must_use]
    pub fn new(cfg: SandwichConfig) -> Self { Self { cfg, by_pool: Default::default() } }

    /// Ingest a swap event. Returns any sandwiches detected by this push.
    pub fn push(&mut self, ev: SwapEvent) -> Vec<SandwichEvent> {
        // Garbage-collect stale events in this pool.
        let buf = self.by_pool.entry(ev.pool.clone()).or_default();
        let cutoff = ev.slot.saturating_sub(self.cfg.window_slots);
        while buf.front().is_some_and(|e| e.slot < cutoff) {
            buf.pop_front();
        }

        let mut hits = Vec::new();

        // Look for a prior swap from same signer with inverted direction
        // — that would be the `front`, and the new `ev` is the `back`.
        let mut front_idx = None;
        for (i, prior) in buf.iter().enumerate() {
            if prior.signer == ev.signer
                && prior.mint_in == ev.mint_out
                && prior.mint_out == ev.mint_in
            {
                front_idx = Some(i);
                break;
            }
        }

        if let Some(i) = front_idx {
            let front = buf[i].clone();
            // Victims = swaps strictly between front and back from other
            // signers, on the same pool, in the same direction as front.
            let victims: Vec<&SwapEvent> = buf
                .iter()
                .skip(i + 1)
                .filter(|s| {
                    s.signer != ev.signer
                        && s.mint_in == front.mint_in
                        && s.mint_out == front.mint_out
                })
                .collect();

            if !victims.is_empty() {
                hits.push(SandwichEvent {
                    slot: ev.slot,
                    pool: ev.pool.clone(),
                    attacker: ev.signer.clone(),
                    front_signature: front.signature.clone(),
                    back_signature: ev.signature.clone(),
                    victim_signatures: victims.iter().map(|v| v.signature.clone()).collect(),
                    front_amount_in: front.amount_in,
                    back_amount_out: ev.amount_out,
                    extracted_amount: ev.amount_out as i128 - front.amount_in as i128,
                });
            }
        }

        buf.push_back(ev);
        hits
    }
}

#[cfg(test)]
mod tests {
    use mev_radar_dex::Dex;

    use super::*;

    fn ev(slot: u64, sig: &str, signer: &str, in_: &str, out: &str, ain: u64, aout: u64) -> SwapEvent {
        SwapEvent {
            dex: Dex::RaydiumAmmV4,
            slot,
            signature: sig.into(),
            signer: signer.into(),
            pool: "P".into(),
            mint_in: in_.into(),
            mint_out: out.into(),
            amount_in: ain,
            amount_out: aout,
        }
    }

    #[test]
    fn detects_classic_sandwich() {
        let mut d = Detector::new(SandwichConfig { window_slots: 1 });
        assert!(d.push(ev(100, "front", "atk", "USDC", "SOL", 1_000, 10)).is_empty());
        assert!(d.push(ev(100, "victim", "alice", "USDC", "SOL", 500, 4)).is_empty());
        let hits = d.push(ev(100, "back", "atk", "SOL", "USDC", 10, 1_050));

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].attacker, "atk");
        assert_eq!(hits[0].victim_signatures, vec!["victim"]);
        assert_eq!(hits[0].extracted_amount, 50);
    }

    #[test]
    fn ignores_when_no_victim() {
        let mut d = Detector::new(SandwichConfig::default());
        d.push(ev(100, "front", "atk", "USDC", "SOL", 1_000, 10));
        let hits = d.push(ev(100, "back", "atk", "SOL", "USDC", 10, 1_050));
        assert!(hits.is_empty());
    }
}
