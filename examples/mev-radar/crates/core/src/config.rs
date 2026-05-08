//! Config loaded from `$XDG_CONFIG_HOME/mev-radar/config.toml` (or `--config`).
//!
//! Token hygiene is non-negotiable: a token is never accepted on the CLI
//! directly. Endpoints declare either an env-var name (`x_token_env`) or a
//! file path (`x_token_file`); the actual secret never appears in the
//! config file or in `ps`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,

    #[serde(default)]
    pub detector: DetectorConfig,

    #[serde(default)]
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Deserialize)]
pub struct EndpointConfig {
    pub name: String,
    pub url: String,

    /// Env var holding the x-token. Preferred over `x_token_file`.
    #[serde(default)]
    pub x_token_env: Option<String>,

    /// File containing the x-token (single line, trimmed).
    #[serde(default)]
    pub x_token_file: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DetectorConfig {
    #[serde(default)]
    pub arb: ArbConfig,

    #[serde(default)]
    pub sandwich: SandwichConfig,
}

#[derive(Debug, Deserialize)]
pub struct ArbConfig {
    pub min_spread_bps: u32,
}

impl Default for ArbConfig {
    fn default() -> Self { Self { min_spread_bps: 30 } }
}

#[derive(Debug, Deserialize)]
pub struct SandwichConfig {
    pub window_slots: u32,
}

impl Default for SandwichConfig {
    fn default() -> Self { Self { window_slots: 1 } }
}

#[derive(Debug, Deserialize)]
pub struct RuntimeConfig {
    /// How often the watch loop logs throughput stats.
    pub stats_interval_secs: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self { Self { stats_interval_secs: 5 } }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&raw)?;

        Ok(cfg)
    }

    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("mev-radar").join("config.toml"))
    }

    pub fn endpoint(&self, name: &str) -> Result<&EndpointConfig> {
        self.endpoints
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| Error::Config(format!("endpoint `{name}` not in config")))
    }
}

impl EndpointConfig {
    /// Resolve the x-token from env var or file. Returns `None` if neither
    /// is configured (some endpoints don't require auth).
    pub fn resolve_token(&self) -> Result<Option<String>> {
        if let Some(env) = &self.x_token_env {
            let val = std::env::var(env)
                .map_err(|_| Error::Token(format!("env `{env}` not set")))?;

            return Ok(Some(val));
        }

        if let Some(path) = &self.x_token_file {
            let raw = std::fs::read_to_string(path)?;

            return Ok(Some(raw.trim().to_string()));
        }

        Ok(None)
    }
}
