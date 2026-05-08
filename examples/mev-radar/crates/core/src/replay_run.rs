//! Day-6 `replay` subcommand backend.
//!
//! Reads a captured stream off disk and feeds the `Transaction` updates
//! into the same swap-decoder pipeline used by the live `swaps` and
//! `radar` subcommands. Pure: needs no network and burns no quota — this
//! is what makes the OSS release CI-able.

use std::path::Path;

use mev_radar_arb::ArbConfig;
use mev_radar_dex::SwapEvent;
use mev_radar_pools::PoolMap;
use mev_radar_replay::open_play;
use mev_radar_sandwich::{Detector as SandDetector, SandwichConfig};
use yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof;
use yellowstone_vixen_core::{instruction::InstructionUpdate, TransactionUpdate};

use crate::{
    error::{Error, Result},
    radar::Event,
};

#[derive(Debug, Clone, Copy)]
pub struct ReplayOptions {
    pub arb: ArbConfig,
    pub sandwich: SandwichConfig,
}

pub async fn run<F: FnMut(&Event)>(path: &Path, opts: ReplayOptions, mut on_event: F) -> Result<u64> {
    let mut player = open_play(path)
        .await
        .map_err(|e| Error::Grpc(format!("open replay: {e}")))?;

    let mut pools = PoolMap::new();
    let mut sand = SandDetector::new(opts.sandwich);
    let mut frames = 0u64;

    while let Some(update) = player
        .next()
        .await
        .map_err(|e| Error::Grpc(format!("replay read: {e}")))?
    {
        frames += 1;

        let Some(UpdateOneof::Transaction(txn)) = update.update_oneof else {
            continue;
        };

        let txn_update = TransactionUpdate { transaction: txn.transaction, slot: txn.slot };
        let Ok(ixs) = InstructionUpdate::build_from_txn(&txn_update) else {
            continue;
        };

        for ev in mev_radar_dex::collect_swaps(&ixs) {
            handle(&mut pools, &mut sand, opts.arb, ev, &mut on_event);
        }
    }

    Ok(frames)
}

fn handle(
    pools: &mut PoolMap,
    sand: &mut SandDetector,
    arb: ArbConfig,
    ev: SwapEvent,
    on_event: &mut impl FnMut(&Event),
) {
    on_event(&Event::Swap(ev.clone()));

    pools.ingest(&ev);
    for arb_ev in mev_radar_arb::detect(pools, arb) {
        on_event(&Event::Arb(arb_ev));
    }

    for hit in sand.push(ev) {
        on_event(&Event::Sandwich(hit));
    }
}
