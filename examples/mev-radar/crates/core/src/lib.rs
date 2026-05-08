//! Core building blocks for `mev-radar`.
//!
//! Day-1 scope: just the gRPC subscribe loop and config. Detector crates
//! (`arb`, `sandwich`, `pools`, `dex/*`) are built on top of this in later
//! days — see the plan in
//! `/root/.claude/plans/write-a-plan-for-valiant-coral.md`.

pub mod config;
pub mod error;
pub mod grpc;
pub mod radar;
pub mod record;
pub mod replay_run;
pub mod swaps;

pub use error::{Error, Result};
