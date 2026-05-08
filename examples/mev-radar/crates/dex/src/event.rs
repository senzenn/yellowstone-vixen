use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dex {
    RaydiumAmmV4,
    Whirlpools,
}

impl Dex {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RaydiumAmmV4 => "raydium_amm_v4",
            Self::Whirlpools => "whirlpools",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// Token A → Token B.
    AToB,
    /// Token B → Token A.
    BToA,
}

/// A decoded DEX swap event.
///
/// `signer` is the wallet that signed the swap and is the one whose
/// pre/post token balances are differenced to compute `amount_in` /
/// `amount_out`. `pool` is the AMM pool / Whirlpool address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapEvent {
    pub dex: Dex,
    pub slot: u64,
    pub signature: String,
    pub signer: String,
    pub pool: String,
    pub mint_in: String,
    pub mint_out: String,
    pub amount_in: u64,
    pub amount_out: u64,
}

/// A coarse account update for one of the DEX pools we know how to track.
///
/// Day-3 (`mev-radar-pools`) consumes this to maintain reserve / sqrt-price
/// state. The raw account `data` is preserved verbatim so per-DEX decoders
/// can apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolUpdate {
    pub dex: Dex,
    pub slot: u64,
    pub pool: String,
    pub data: Vec<u8>,
}
