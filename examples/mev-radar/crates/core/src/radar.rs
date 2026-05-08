//! Day-4/5/7 unified MEV-radar loop.
//!
//! Subscribes to Raydium AMM v4 + Whirlpools transactions, decodes swaps
//! via [`mev_radar_dex`], updates [`mev_radar_pools::PoolMap`], runs
//! [`mev_radar_arb::detect`] on every swap, and feeds events into
//! [`mev_radar_sandwich::Detector`]. Outputs flow to a user-supplied
//! [`Sink`].

use std::time::Duration;

use mev_radar_arb::{ArbConfig, ArbEvent};
use mev_radar_dex::SwapEvent;
use mev_radar_pools::PoolMap;
use mev_radar_sandwich::{Detector as SandDetector, SandwichConfig, SandwichEvent};
use tokio::sync::mpsc;

use crate::{
    config::{Config, EndpointConfig},
    error::Result,
    swaps::{self, SwapsOptions},
};

/// Anything the radar produces.
#[derive(Debug, Clone)]
pub enum Event {
    Swap(SwapEvent),
    Arb(ArbEvent),
    Sandwich(SandwichEvent),
}

/// Full radar configuration drawn from the user's [`Config`].
#[derive(Debug, Clone, Copy)]
pub struct RadarOptions {
    pub stats_interval: Duration,
    pub arb: ArbConfig,
    pub sandwich: SandwichConfig,
}

impl RadarOptions {
    #[must_use]
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            stats_interval: Duration::from_secs(cfg.runtime.stats_interval_secs.max(1)),
            arb: ArbConfig {
                min_spread_bps: cfg.detector.arb.min_spread_bps,
            },
            sandwich: SandwichConfig {
                window_slots: u64::from(cfg.detector.sandwich.window_slots).max(1),
            },
        }
    }
}

/// Run the radar. Each detection is sent on the channel returned by the
/// caller. The channel is closed when the radar exits.
pub async fn run(endpoint: &EndpointConfig, opts: RadarOptions, tx: mpsc::Sender<Event>) -> Result<()> {
    let mut pools = PoolMap::new();
    let mut sand = SandDetector::new(opts.sandwich);

    swaps::run(
        endpoint,
        SwapsOptions { stats_interval: opts.stats_interval },
        |ev: &SwapEvent| {
            // Best-effort send; if the consumer is gone we drop silently.
            let _ = tx.try_send(Event::Swap(ev.clone()));

            pools.ingest(ev);

            for arb in mev_radar_arb::detect(&pools, opts.arb) {
                let _ = tx.try_send(Event::Arb(arb));
            }

            for hit in sand.push(ev.clone()) {
                let _ = tx.try_send(Event::Sandwich(hit));
            }
        },
    )
    .await
}
