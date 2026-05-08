use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use mev_radar_core::{
    config::Config,
    grpc::{run_count_loop, SubscribeOptions},
    radar::{self, Event, RadarOptions},
    record::{self, RecordOptions},
    replay_run::{self, ReplayOptions},
    swaps::{self, SwapsOptions},
};
use mev_radar_tui::{Dashboard, DashboardState};
use tokio::sync::mpsc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();

    init_tracing(args.log.as_deref());

    let cfg_path = args
        .config
        .clone()
        .or_else(Config::default_path)
        .context("no config path; pass --config or set XDG_CONFIG_HOME")?;
    let config = Config::load(&cfg_path)
        .with_context(|| format!("loading config {}", cfg_path.display()))?;

    match args.command {
        cli::Command::Watch { endpoint, stats_interval, max_messages } => {
            let ep = config.endpoint(&endpoint)?;
            let opts = SubscribeOptions {
                stats_interval: Duration::from_secs(stats_interval),
                max_messages: if max_messages == 0 { u64::MAX } else { max_messages },
            };

            tokio::select! {
                res = run_count_loop(ep, opts) => {
                    let totals = res?;
                    tracing::info!(?totals, "exited cleanly");
                }
                _ = tokio::signal::ctrl_c() => tracing::info!("ctrl-c, shutting down"),
            }
        },

        cli::Command::Swaps { endpoint, stats_interval } => {
            let ep = config.endpoint(&endpoint)?;
            let opts = SwapsOptions { stats_interval: Duration::from_secs(stats_interval) };

            tokio::select! {
                res = swaps::run(ep, opts, |ev| emit_jsonl("swap", ev)) => {
                    res?;
                }
                _ = tokio::signal::ctrl_c() => tracing::info!("ctrl-c, shutting down"),
            }
        },

        cli::Command::Radar { endpoint } => {
            let ep = config.endpoint(&endpoint)?;
            let opts = RadarOptions::from_config(&config);
            let (tx, mut rx) = mpsc::channel::<Event>(2048);

            let consumer = tokio::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    emit_event(&ev);
                }
            });

            tokio::select! {
                res = radar::run(ep, opts, tx) => res?,
                _ = tokio::signal::ctrl_c() => tracing::info!("ctrl-c, shutting down"),
            }
            consumer.abort();
        },

        cli::Command::Record { endpoint, out, duration_secs, max_messages } => {
            let ep = config.endpoint(&endpoint)?;
            let opts = RecordOptions {
                duration: Duration::from_secs(duration_secs),
                max_messages,
            };

            tokio::select! {
                res = record::run(ep, &out, opts) => {
                    let n = res?;
                    tracing::info!(written = n, path = %out.display(), "recorded");
                }
                _ = tokio::signal::ctrl_c() => tracing::info!("ctrl-c, shutting down"),
            }
        },

        cli::Command::Replay { file } => {
            let opts = ReplayOptions {
                arb: mev_radar_arb::ArbConfig {
                    min_spread_bps: config.detector.arb.min_spread_bps,
                },
                sandwich: mev_radar_sandwich::SandwichConfig {
                    window_slots: u64::from(config.detector.sandwich.window_slots).max(1),
                },
            };
            let frames = replay_run::run(&file, opts, emit_event).await?;
            tracing::info!(frames, path = %file.display(), "replay complete");
        },

        cli::Command::Tui { endpoint } => run_tui(&config, &endpoint).await?,
    }

    Ok(())
}

async fn run_tui(config: &Config, endpoint: &str) -> anyhow::Result<()> {
    let ep = config.endpoint(endpoint)?;
    let opts = RadarOptions::from_config(config);
    let (tx, mut rx) = mpsc::channel::<Event>(4096);

    let radar_handle = tokio::spawn({
        let ep = ep.clone();
        async move { radar::run(&ep, opts, tx).await }
    });

    let mut dash = Dashboard::enter()?;
    let mut state = DashboardState {
        status_line: format!("connecting to {endpoint}…"),
        ..Default::default()
    };

    loop {
        if dash.should_quit()? {
            break;
        }

        // Drain any pending events without blocking.
        loop {
            match rx.try_recv() {
                Ok(Event::Swap(s)) => {
                    state.recent_swaps.push(s);
                    if state.recent_swaps.len() > 200 {
                        state.recent_swaps.drain(..100);
                    }
                },
                Ok(Event::Arb(a)) => {
                    state.top_spreads.insert(0, a);
                    state.top_spreads.truncate(20);
                },
                Ok(Event::Sandwich(s)) => {
                    state.recent_sandwiches.push(s);
                    if state.recent_sandwiches.len() > 200 {
                        state.recent_sandwiches.drain(..100);
                    }
                },
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        state.status_line = format!(
            "endpoint={} swaps={} arbs={} sandwiches={}",
            endpoint,
            state.recent_swaps.len(),
            state.top_spreads.len(),
            state.recent_sandwiches.len(),
        );
        dash.render(&state)?;

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    radar_handle.abort();
    Ok(())
}

fn emit_event(ev: &Event) {
    match ev {
        Event::Swap(s) => emit_jsonl("swap", s),
        Event::Arb(a) => emit_jsonl("arb", a),
        Event::Sandwich(s) => emit_jsonl("sandwich", s),
    }
}

fn emit_jsonl<T: serde::Serialize>(kind: &str, v: &T) {
    #[derive(serde::Serialize)]
    struct Tagged<'a, T: serde::Serialize> {
        kind: &'a str,
        #[serde(flatten)]
        body: &'a T,
    }
    match serde_json::to_string(&Tagged { kind, body: v }) {
        Ok(line) => println!("{line}"),
        Err(e) => tracing::warn!(error = %e, kind, "serialize"),
    }
}

fn init_tracing(filter_override: Option<&str>) {
    let filter = filter_override.map(EnvFilter::new).unwrap_or_else(|| {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,mev_radar=info,mev_radar_core=info"))
    });

    // Logs go to stderr so `swaps`, `radar`, and `replay` JSONL on
    // stdout stays uncontaminated. This is the difference between
    // `mev-radar radar | jq` working and silently dying on a `tracing
    // info` line that isn't valid JSON.
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr).compact())
        .init();
}
