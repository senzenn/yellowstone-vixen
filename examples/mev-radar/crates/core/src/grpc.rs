//! Day-1 gRPC subscribe loop.
//!
//! Connects to a Yellowstone Dragon's Mouth endpoint, subscribes to slot
//! updates (cheapest possible filter), counts incoming `SubscribeUpdate`s
//! by type, sends a client-side `Ping` every 10s, replies to server
//! `Ping`s, and reconnects with exponential backoff on disconnect.
//!
//! In subsequent days the slot-only filter is replaced by per-DEX
//! transaction + account filters and the count loop is replaced by a
//! Vixen pipeline.

use std::{collections::HashMap, time::Duration};

use futures::{sink::SinkExt, stream::StreamExt};
use tokio::time::Instant;
use tracing::{debug, info, warn};
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::geyser::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest,
    SubscribeRequestFilterSlots, SubscribeRequestPing,
};

use crate::{
    config::EndpointConfig,
    error::{Error, Result},
};

const CLIENT_PING_INTERVAL: Duration = Duration::from_secs(10);
const RECONNECT_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Tunables for [`run_count_loop`].
#[derive(Debug, Clone, Copy)]
pub struct SubscribeOptions {
    pub stats_interval: Duration,
    pub max_messages: u64,
}

impl Default for SubscribeOptions {
    fn default() -> Self {
        Self {
            stats_interval: Duration::from_secs(5),
            max_messages: u64::MAX,
        }
    }
}

/// Counters tracked across the lifetime of a [`run_count_loop`] call.
///
/// Example output:
///
/// ```rust,ignore
/// Counters { messages: 1234, slots: 410, txs: 0, accounts: 0,
///            blocks: 0, block_meta: 0, entries: 0, server_pings: 12 }
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct Counters {
    pub messages: u64,
    pub slots: u64,
    pub txs: u64,
    pub accounts: u64,
    pub blocks: u64,
    pub block_meta: u64,
    pub entries: u64,
    pub server_pings: u64,
}

/// Subscribe to slot updates and count messages. Reconnects on disconnect
/// with exponential backoff. Returns when `max_messages` is hit.
pub async fn run_count_loop(
    endpoint: &EndpointConfig,
    opts: SubscribeOptions,
) -> Result<Counters> {
    let token = endpoint.resolve_token()?;
    let mut backoff = RECONNECT_BACKOFF_INITIAL;
    let mut total = Counters::default();

    loop {
        match subscribe_once(endpoint, token.as_deref(), opts, &mut total).await {
            Ok(()) => return Ok(total),

            Err(e) => {
                if total.messages >= opts.max_messages {
                    return Ok(total);
                }

                warn!(error = %e, retry_in = ?backoff, "stream disconnected; reconnecting");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_BACKOFF_MAX);
            },
        }
    }
}

async fn subscribe_once(
    endpoint: &EndpointConfig,
    token: Option<&str>,
    opts: SubscribeOptions,
    total: &mut Counters,
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

    info!(name = %endpoint.name, url = %endpoint.url, "connected");

    let initial = initial_subscribe_request();
    let (mut sub_tx, stream) = client
        .subscribe_with_request(Some(initial))
        .await
        .map_err(|e| Error::Grpc(format!("subscribe: {e}")))?;
    let mut stream = std::pin::pin!(stream);

    let mut ping_id: i32 = 1;
    let mut ping_deadline = Instant::now() + CLIENT_PING_INTERVAL;
    let mut stats_deadline = Instant::now() + opts.stats_interval;
    let mut window = Counters::default();

    loop {
        tokio::select! {
            // Client-side keepalive. Cloudflare / Fly drop idle gRPC
            // streams; the yellowstone-grpc README explicitly warns about
            // this.
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
                    msg_per_s = window.messages / dt,
                    total_msgs = total.messages,
                    total_slots = total.slots,
                    total_txs = total.txs,
                    total_accounts = total.accounts,
                    server_pings = total.server_pings,
                    "stats"
                );

                window = Counters::default();
                stats_deadline = Instant::now() + opts.stats_interval;
            }

            msg = stream.next() => {
                let Some(msg) = msg else {
                    return Err(Error::Grpc("stream closed by server".into()));
                };
                let update = msg.map_err(|e| Error::Grpc(format!("stream: {e}")))?;

                total.messages += 1;
                window.messages += 1;

                match &update.update_oneof {
                    Some(UpdateOneof::Slot(_)) => total.slots += 1,
                    Some(UpdateOneof::Transaction(_)) => total.txs += 1,
                    Some(UpdateOneof::Account(_)) => total.accounts += 1,
                    Some(UpdateOneof::Block(_)) => total.blocks += 1,
                    Some(UpdateOneof::BlockMeta(_)) => total.block_meta += 1,
                    Some(UpdateOneof::Entry(_)) => total.entries += 1,
                    Some(UpdateOneof::Ping(_)) => {
                        total.server_pings += 1;
                        ping_id = ping_id.wrapping_add(1);

                        // Reply to server ping. The yellowstone-grpc README
                        // describes this as the canonical pong path.
                        sub_tx
                            .send(SubscribeRequest {
                                ping: Some(SubscribeRequestPing { id: ping_id }),
                                ..Default::default()
                            })
                            .await
                            .map_err(|e| Error::Grpc(format!("pong: {e}")))?;
                    },
                    Some(UpdateOneof::Pong(_)) => debug!("pong"),
                    _ => {},
                }

                if total.messages >= opts.max_messages {
                    info!(total = total.messages, "message ceiling reached");
                    return Ok(());
                }
            }
        }
    }
}

/// Cheapest possible filter — slot updates at processed commitment, with
/// `filter_by_commitment` so we don't get a copy per stage. Replaced in
/// later days with per-DEX transaction + account filters.
fn initial_subscribe_request() -> SubscribeRequest {
    let mut slots = HashMap::new();
    slots.insert(
        "slots".to_string(),
        SubscribeRequestFilterSlots {
            filter_by_commitment: Some(true),
            interslot_updates: None,
        },
    );

    SubscribeRequest {
        slots,
        commitment: Some(CommitmentLevel::Processed as i32),
        ..Default::default()
    }
}
