# mev-radar

Real-time arbitrage and sandwich-MEV observatory built on Yellowstone gRPC
(Dragon's Mouth) plus the local Vixen runtime crates.

Subscribes to live Solana DEX activity, decodes swaps via discriminator-
based parsers (Raydium AMM v4, Orca Whirlpools), and runs two streaming
detectors over the resulting [`SwapEvent`] stream:

- **Arbitrage scout** — cross-pool spread > threshold (basis points).
- **Sandwich classifier** — `[front-run, victim, back-run]` patterns
  attributable to one actor.

Purely observational — no transaction submission, no funded wallet.

## Status

v0.1: Days 1–7 of the plan are merged.

| Day | Crate | What it does |
| --- | --- | --- |
| 1   | `core::grpc` + `radar` bin | Connect, subscribe, slot-only count loop, Ping/Pong, reconnect |
| 2   | `dex` + `core::swaps`      | Detect Raydium / Whirlpools swaps; emit JSONL `SwapEvent`s |
| 3   | `pools`                    | `PoolMap` of latest observed price per (pool, pair) |
| 4   | `arb`                      | O(P²) cross-pool spread detector → `ArbEvent` JSONL |
| 5   | `sandwich`                 | Per-pool ring buffer + `[front, victim, back]` pattern matcher |
| 6   | `replay`                   | Length-prefixed prost record / replay format + `record` / `replay` subcommands |
| 7   | `tui`                      | ratatui dashboard + GH Actions CI |

## Quick start

```bash
mkdir -p ~/.config/mev-radar
cp examples/mev-radar/config.example.toml ~/.config/mev-radar/config.toml
# edit endpoint URL and pick an x_token_env var name

export YGRPC_TOKEN=<your_token>

# Smallest filter: count messages.
cargo run --manifest-path examples/mev-radar/Cargo.toml -p mev-radar -- \
    watch --endpoint mainnet --stats-interval 5

# Full radar: live arb spreads + sandwich detection, JSONL on stdout.
cargo run --manifest-path examples/mev-radar/Cargo.toml -p mev-radar --release -- \
    radar --endpoint mainnet | jq

# Capture 60s for offline iteration / CI.
cargo run --manifest-path examples/mev-radar/Cargo.toml -p mev-radar --release -- \
    record --endpoint mainnet --out cap.bin --duration-secs 60

# Replay (no quota burn, deterministic).
cargo run --manifest-path examples/mev-radar/Cargo.toml -p mev-radar --release -- \
    replay cap.bin

# TUI dashboard.
cargo run --manifest-path examples/mev-radar/Cargo.toml -p mev-radar --release -- \
    tui --endpoint mainnet
```

## JSONL output shape

`swaps`, `radar`, and `replay` print one tagged JSON object per line:

```json
{"kind":"swap","dex":"raydium_amm_v4","slot":309845112,"signature":"…","signer":"…","pool":"…","mint_in":"USDC","mint_out":"SOL","amount_in":1000000000,"amount_out":4920000}
{"kind":"arb","pair":["SOL","USDC"],"slot":309845112,"spread_bps":47,"buy_dex":"whirlpools","buy_pool":"…","buy_price":0.0049,"sell_dex":"raydium_amm_v4","sell_pool":"…","sell_price":0.00492}
{"kind":"sandwich","slot":309845115,"pool":"…","attacker":"…","front_signature":"…","back_signature":"…","victim_signatures":["…"],"front_amount_in":1000000000,"back_amount_out":1050000000,"extracted_amount":50000000}
```

## Workspace layout

| crate                  | role                                                          |
| ---------------------- | ------------------------------------------------------------- |
| `crates/core`          | gRPC subscribe loop, config, swaps/record/replay backends     |
| `crates/dex`           | Per-DEX swap detectors (discriminator-based)                  |
| `crates/pools`         | Observed-price `PoolMap` keyed on (pool, pair)                |
| `crates/arb`           | Cross-pool spread evaluator → `ArbEvent`                      |
| `crates/sandwich`      | Streaming sandwich-pattern classifier                         |
| `crates/replay`        | Length-prefixed prost record/replay format                    |
| `crates/tui`           | ratatui dashboard                                             |
| `crates/radar`         | The `mev-radar` binary tying it all together                  |

## Token / endpoint hygiene

Tokens are **never** taken on the command line. Pick one of:

- `x_token_env = "YGRPC_TOKEN"` — read from env at startup.
- `x_token_file = "/run/secrets/ygrpc_token"` — first non-blank line.

The watch loop sends a client-side `Ping` every 10s + replies to server
`Ping`s — load balancers like Cloudflare and Fly otherwise drop idle gRPC
streams (the `yellowstone-grpc` README warns about this explicitly).

## Caveats / known limitations

- Swap amounts come from token-balance deltas keyed on the signer.
  Multi-hop routes that pass through one wallet will collapse to a
  single net delta; single-pool swaps (the common case) are exact.
  Switching to Codama-IDL-driven decode is the v0.2 plan.
- `pools::PoolMap` updates only on observed swaps, not every account
  reserve change. Freshly-quoted pools lag. v0.2 will add account-data
  decoders.
- `SubscribeDeshred` is not used; the open-source Yellowstone server
  returns `UNIMPLEMENTED` per its README.

## License

Dual MIT / Apache-2.0.
