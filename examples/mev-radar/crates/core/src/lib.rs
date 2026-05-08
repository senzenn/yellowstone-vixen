//! Core building blocks for `mev-radar`.
//!
//! Day-1 scope: just the gRPC subscribe loop and config. Detector crates
//! (`arb`, `sandwich`, `pools`, `dex/*`) are built on top of this in later
//! days — see the plan in
//! `/root/.claude/plans/write-a-plan-for-valiant-coral.md`.

pub mod config;
pub mod error;
pub mod grpc;

pub use error::{Error, Result};
