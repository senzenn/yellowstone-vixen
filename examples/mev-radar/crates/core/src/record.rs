//! Day-6 `record` subcommand backend.
//!
//! Subscribes to the same DEX-targeted filter set as `swaps` and writes
//! every `SubscribeUpdate` to a file in `mev-radar-replay`'s on-disk
//! format. Stops after a configurable duration or `max_messages`.

use std::{collections::HashMap, path::Path, time::Duration};

use futures::{sink::SinkExt, stream::StreamExt};
use mev_radar_dex::program_ids::{raydium_amm_v4_program, whirlpools_program};
use mev_radar_replay::open_record;
use tokio::time::Instant;
use tracing::{info, warn};
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::geyser::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest,
    SubscribeRequestFilterTransactions, SubscribeRequestPing,
};

use crate::{
    config::EndpointConfig,
    error::{Error, Result},
};

const CLIENT_PING_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
pub struct RecordOptions {
    pub duration: Duration,
    pub max_messages: u64,
}

pub async fn run(endpoint: &EndpointConfig, out: &Path, opts: RecordOptions) -> Result<u64> {
    let token = endpoint.resolve_token()?;
    let mut recorder = open_record(out)
        .await
        .map_err(|e| Error::Grpc(format!("open record: {e}")))?;

    let mut client = GeyserGrpcClient::build_from_shared(endpoint.url.clone())
        .map_err(|e| Error::Grpc(format!("invalid endpoint url: {e}")))?
        .x_token(token)
        .map_err(|e| Error::Grpc(format!("x-token: {e}")))?
        .tls_config(ClientTlsConfig::new().with_native_roots())
        .map_err(|e| Error::Grpc(format!("tls: {e}")))?
        .connect()
        .await
        .map_err(|e| Error::Grpc(format!("connect: {e}")))?;

    info!(name = %endpoint.name, ?opts, "recording");

    let req = build_request();
    let (mut sub_tx, stream) = client
        .subscribe_with_request(Some(req))
        .await
        .map_err(|e| Error::Grpc(format!("subscribe: {e}")))?;
    let mut stream = std::pin::pin!(stream);

    let mut ping_id: i32 = 1;
    let mut ping_deadline = Instant::now() + CLIENT_PING_INTERVAL;
    let stop_at = Instant::now() + opts.duration;
    let mut written = 0u64;

    loop {
        tokio::select! {
            () = tokio::time::sleep_until(stop_at) => break,

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

            msg = stream.next() => {
                let Some(msg) = msg else {
                    warn!("stream closed early");
                    break;
                };
                let update = msg.map_err(|e| Error::Grpc(format!("stream: {e}")))?;

                if let Some(UpdateOneof::Ping(_)) = &update.update_oneof {
                    ping_id = ping_id.wrapping_add(1);
                    sub_tx
                        .send(SubscribeRequest {
                            ping: Some(SubscribeRequestPing { id: ping_id }),
                            ..Default::default()
                        })
                        .await
                        .map_err(|e| Error::Grpc(format!("pong: {e}")))?;
                }

                recorder
                    .write(&update)
                    .await
                    .map_err(|e| Error::Grpc(format!("record write: {e}")))?;
                written += 1;

                if opts.max_messages > 0 && written >= opts.max_messages {
                    break;
                }
            }
        }
    }

    let n = recorder
        .finish()
        .await
        .map_err(|e| Error::Grpc(format!("record finish: {e}")))?;

    info!(written = n, "record complete");
    Ok(n)
}

fn build_request() -> SubscribeRequest {
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
        commitment: Some(CommitmentLevel::Processed as i32),
        ..Default::default()
    }
}
