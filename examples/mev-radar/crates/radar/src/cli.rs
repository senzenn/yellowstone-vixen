use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "mev-radar",
    version,
    about = "Real-time arbitrage and sandwich-MEV observatory on Yellowstone gRPC"
)]
pub struct Cli {
    /// Path to config (defaults to $XDG_CONFIG_HOME/mev-radar/config.toml).
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Tracing filter. Overrides RUST_LOG.
    #[arg(long, global = true)]
    pub log: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Day 1: subscribe to gRPC and print message counts.
    Watch {
        /// Endpoint name from config.
        #[arg(long)]
        endpoint: String,

        /// Stats logging interval in seconds.
        #[arg(long, default_value_t = 5)]
        stats_interval: u64,

        /// Maximum messages before clean exit (0 = unlimited).
        #[arg(long, default_value_t = 0)]
        max_messages: u64,
    },

    /// Day 6: record a stream slice to a file.
    Record {
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 60)]
        duration_secs: u64,
    },

    /// Day 6: replay a captured stream.
    Replay {
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },

    /// Day 7: open the TUI dashboard.
    Tui {
        #[arg(long)]
        endpoint: String,
    },
}
