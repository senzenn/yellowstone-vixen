use std::time::Duration;

use anyhow::{anyhow, Context};
use clap::Parser;
use mev_radar_core::{
    config::Config,
    grpc::{run_count_loop, SubscribeOptions},
};
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
                max_messages: if max_messages == 0 {
                    u64::MAX
                } else {
                    max_messages
                },
            };

            tokio::select! {
                res = run_count_loop(ep, opts) => {
                    let totals = res?;
                    tracing::info!(?totals, "exited cleanly");
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("ctrl-c received, shutting down");
                }
            }
        },

        cli::Command::Record { .. } => {
            return Err(anyhow!("`record` lands on Day 6 (mev-radar-replay)"));
        },

        cli::Command::Replay { .. } => {
            return Err(anyhow!("`replay` lands on Day 6 (mev-radar-replay)"));
        },

        cli::Command::Tui { .. } => {
            return Err(anyhow!("`tui` lands on Day 7 (mev-radar-tui)"));
        },
    }

    Ok(())
}

fn init_tracing(filter_override: Option<&str>) {
    let filter = filter_override
        .map(EnvFilter::new)
        .unwrap_or_else(|| {
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,mev_radar=info,mev_radar_core=info"))
        });

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().compact())
        .init();
}
