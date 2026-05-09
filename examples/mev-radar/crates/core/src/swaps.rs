//! Day-2 `swaps` subcommand backend.
//!
//! Subscribes to transactions on Raydium AMM v4 + Whirlpools, decodes
//! them via the local instruction-walker (lifted from
//! `yellowstone_vixen_core::instruction::InstructionUpdate::build_from_txn`),
//! and emits one JSONL line per detected swap on the supplied callback.

use std::{collections::HashMap, time::Duration};

use futures::{sink::SinkExt, stream::StreamExt};
use mev_radar_dex::{
    program_ids::{raydium_amm_v4_program, whirlpools_program},
    SwapEvent,
};
use tokio::time::Instant;
use tracing::{debug, info, warn};
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::geyser::{
    subscribe_update::UpdateOneof, SubscribeRequest, SubscribeRequestFilterTransactions,
    SubscribeRequestPing,
};
use yellowstone_vixen_core::{instruction::InstructionUpdate, TransactionUpdate};

use crate::{
    config::{Commitment, EndpointConfig},
    error::{Error, Result},
};

const CLIENT_PING_INTERVAL: Duration = Duration::from_secs(10);
const RECONNECT_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
pub struct SwapsOptions {
    pub stats_interval: Duration,
    pub commitment: Commitment,
}

impl Default for SwapsOptions {
    fn default() -> Self {
        Self {
            stats_interval: Duration::from_secs(5),
            commitment: Commitment::default(),
        }
    }
}

/// Run the swap-detection loop. Each detected swap is passed to `on_swap`.
/// Reconnects on disconnect with exponential backoff.
pub async fn run<F: FnMut(&SwapEvent)>(
    endpoint: &EndpointConfig,
    opts: SwapsOptions,
    mut on_swap: F,
) -> Result<()> {
    let token = endpoint.resolve_token()?;
    let mut backoff = RECONNECT_BACKOFF_INITIAL;

    loop {
        match subscribe_once(endpoint, token.as_deref(), opts, &mut on_swap).await {
            Ok(()) => return Ok(()),

            Err(e) => {
                warn!(error = %e, retry_in = ?backoff, "stream disconnected; reconnecting");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_BACKOFF_MAX);
            },
        }
    }
}

async fn subscribe_once<F: FnMut(&SwapEvent)>(
    endpoint: &EndpointConfig,
    token: Option<&str>,
    opts: SwapsOptions,
    on_swap: &mut F,
) -> Result<()> {
    let mut client = GeyserGrpcClient::build_from_shared(endpoint.url.clone())
        .map_err(|e| Error::Grpc(format!("invalid endpoint url: {e}")))?
        .x_token(token.map(str::to_string))
        .map_err(|e| Error::Grpc(format!("x-token: {e}")))?
        .tls_config(ClientTlsConfig::new().with_native_roots())
        .map_err(|e| Error::Grpc(format!("tls: {e}")))?
        .connect()
        .await
        .map_err(|e| Error::Grpc(format!("connect: {e}")))?;

    info!(name = %endpoint.name, "connected (swaps mode)");

    let req = build_subscribe_request(opts.commitment);
    let (mut sub_tx, stream) = client
        .subscribe_with_request(Some(req))
        .await
        .map_err(|e| Error::Grpc(format!("subscribe: {e}")))?;
    let mut stream = std::pin::pin!(stream);

    let mut ping_id: i32 = 1;
    let mut ping_deadline = Instant::now() + CLIENT_PING_INTERVAL;
    let mut stats_deadline = Instant::now() + opts.stats_interval;
    let mut window_txs = 0u64;
    let mut window_swaps = 0u64;
    let mut total_txs = 0u64;
    let mut total_swaps = 0u64;

    loop {
        tokio::select! {
            () = tokio::time::sleep_until(ping_deadline) => {
                ping_id = ping_id.wrapping_add(1);
                ping_deadline = Instant::now() + CLIENT_PING_INTERVAL;

                sub_tx
                    .send(SubscribeRequest {
                        ping: Some(SubscribeRequestPing { id: ping_id }),
                        ..Default::default()
                    })
                    .await
                    .map_err(|e| Error::Grpc(format!("client ping: {e}")))?;
            }

            () = tokio::time::sleep_until(stats_deadline) => {
                let dt = opts.stats_interval.as_secs().max(1);
                info!(
                    txs_per_s = window_txs / dt,
                    swaps_per_s = window_swaps / dt,
                    total_txs,
                    total_swaps,
                    "stats"
                );

                window_txs = 0;
                window_swaps = 0;
                stats_deadline = Instant::now() + opts.stats_interval;
            }

            msg = stream.next() => {
                let Some(msg) = msg else {
                    return Err(Error::Grpc("stream closed by server".into()));
                };
                let update = msg.map_err(|e| Error::Grpc(format!("stream: {e}")))?;

                match update.update_oneof {
                    Some(UpdateOneof::Transaction(txn)) => {
                        total_txs += 1;
                        window_txs += 1;

                        let txn_update = TransactionUpdate {
                            transaction: txn.transaction,
                            slot: txn.slot,
                        };

                        let ixs = match InstructionUpdate::build_from_txn(&txn_update) {
                            Ok(ixs) => ixs,
                            Err(e) => {
                                debug!(?e, "skipping unbuildable txn");
                                continue;
                            },
                        };

                        for ev in mev_radar_dex::collect_swaps(&ixs) {
                            total_swaps += 1;
                            window_swaps += 1;
                            on_swap(&ev);
                        }
                    },
                    Some(UpdateOneof::Ping(_)) => {
                        ping_id = ping_id.wrapping_add(1);
                        sub_tx
                            .send(SubscribeRequest {
                                ping: Some(SubscribeRequestPing { id: ping_id }),
                                ..Default::default()
                            })
                            .await
                            .map_err(|e| Error::Grpc(format!("pong: {e}")))?;
                    },
                    _ => {},
                }
            }
        }
    }
}

fn build_subscribe_request(commitment: Commitment) -> SubscribeRequest {
    let mut transactions = HashMap::new();

    transactions.insert(
        "dexes".to_string(),
        SubscribeRequestFilterTransactions {
            vote: Some(false),
            failed: Some(false),
            signature: None,
            account_include: vec![
                bs58::encode(raydium_amm_v4_program().as_slice()).into_string(),
                bs58::encode(whirlpools_program().as_slice()).into_string(),
            ],
            account_exclude: vec![],
            account_required: vec![],
        },
    );

    SubscribeRequest {
        transactions,
        commitment: Some(commitment.as_grpc_i32()),
        ..Default::default()
    }
}
