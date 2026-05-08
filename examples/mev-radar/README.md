# mev-radar

Real-time arbitrage and sandwich-MEV observatory built on Yellowstone gRPC
(Dragon's Mouth) and Vixen.

Subscribes to live Solana DEX activity, decodes swaps via Vixen parsers,
and emits structured events for two detectors:

- **Arbitrage scout** — cross-pool spread > threshold (basis points).
- **Sandwich classifier** — `[front-run, victim, back-run]` patterns
  attributable to one actor.

Purely observational. No transaction submission, no funded wallet.

## Status

Day 1 of a ~7-day plan: workspace skeleton + raw `SubscribeUpdate` count
loop with client-side keepalive ping and reconnect. The detector stack
(per-DEX parsers, pool state, arb / sandwich classifiers, replay, TUI)
arrives in subsequent days.

## Quick start

```bash
mkdir -p ~/.config/mev-radar
cp examples/mev-radar/config.example.toml ~/.config/mev-radar/config.toml
# edit endpoint URL and pick an x_token_env var name

export YGRPC_TOKEN=<your_token>
cargo run --manifest-path examples/mev-radar/Cargo.toml -p mev-radar -- \
    watch --endpoint mainnet --stats-interval 5
```

Expected output (roughly):

```
2026-05-08T16:00:00Z  INFO  connected name="mainnet" url="..."
2026-05-08T16:00:05Z  INFO  stats msg_per_s=82 total_msgs=410 total_slots=410 ...
2026-05-08T16:00:10Z  INFO  stats msg_per_s=80 total_msgs=820 ...
^C
2026-05-08T16:00:13Z  INFO  ctrl-c received, shutting down
```

## Workspace layout

| crate | role |
| --- | --- |
| `crates/core` | gRPC subscribe loop, config, common types |
| `crates/radar` | the `mev-radar` binary + CLI |

Future crates (per the plan):
`mev-radar-dex/{raydium-amm-v4,whirlpools}`, `mev-radar-pools`,
`mev-radar-arb`, `mev-radar-sandwich`, `mev-radar-replay`, `mev-radar-tui`.

## Token / endpoint hygiene

Tokens are **never** taken on the command line. Pick one of:

- `x_token_env = "YGRPC_TOKEN"` — read from env at startup.
- `x_token_file = "/run/secrets/ygrpc_token"` — read first non-blank line
  from a file.

The `mev-radar` binary refuses to dump tokens to logs, and the watch loop
sends a client-side `Ping` every 10s + replies to server `Ping`s — load
balancers like Cloudflare and Fly otherwise drop idle gRPC streams (the
`yellowstone-grpc` README warns about this explicitly).

## License

Dual MIT / Apache-2.0.
