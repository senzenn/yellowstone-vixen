# mev-radar v0.1 audit

This is a self-audit of the v0.1 codebase. Each finding has a severity,
the original failure mode, and the fix shipped (or, where intentionally
deferred, the workaround).

Severity legend:

- **C — correctness**: produces wrong output on real data
- **R — robustness**: panics / OOMs / data-loss on edge or hostile input
- **U — usability**: surprising behavior the user has to learn around
- **P — performance**: works correctly but scales poorly

---

## Fixed in this audit pass

### B-01 (C, high) — closed token accounts went undetected

`dex::balances::delta` iterated `post.iter()` only, so any token account
that existed in `pre` but was closed during the tx (very common for
intermediate ATAs in routes) had its negative delta silently dropped.
Knock-on effect: `derive_swap_event` would see no negative delta and
return `None`, dropping the entire swap from the stream.

**Fix**: `collect_deltas` now iterates the **union** of (account_index,
mint) pairs from both pre and post. Missing-side amounts default to 0 so
opens / closes both produce the right signed delta.

**Test added**: `dex::balances::tests::delta_picks_up_closed_account`.

### B-02 (C, high) — multi-instruction routes emitted N duplicate swaps per tx

Amounts are computed from tx-level pre/post token balances. A Jupiter
route with N swap instructions in one tx had `collect_swaps` invoke the
parsers N times, each returning the same `(mint_in, amount_in,
mint_out, amount_out)` with only `pool` differing. Knock-on: the pool
map updated N pools to the same price, the arb detector then saw "N
pools at the same price" (no spread), and the sandwich detector pushed
N copies into the same per-pool ring buffer, skewing the front-run
matching.

**Fix**: `collect_swaps` dedupes by transaction signature. The first
swap detected per tx wins; the rest are dropped. Future Codama-IDL
parsing will provide per-instruction amounts and the dedup can be
removed.

### B-03 (C, medium) — Whirlpools `swapV2` decoded with wrong account indices

The `swap` and `swapV2` Whirlpools instructions have different account
layouts. `swapV2` prefixes two `tokenProgram` accounts and a `memo` /
`transferHook` account before `tokenAuthority`, shifting `signer` from
index 1 → 3 and `whirlpool` from index 2 → 4. The original code applied
v1 indices to v2 too, attributing v2 swaps to whatever pubkey happened
to sit at indices 1/2 (typically a token program + signer pair, both
wrong).

**Fix**: branch on the discriminator and pick `(SWAP_*_SIGNER_IDX,
SWAP_*_POOL_IDX)` per variant.

### B-04 (U, high) — `tracing` logs corrupted JSONL on stdout

`tracing_subscriber::fmt::layer()` defaults to stdout, which the
`swaps` / `radar` / `replay` subcommands also use for JSONL output.
Piping into `jq` would die on the first log line.

**Fix**: `init_tracing` now installs the fmt layer with
`.with_writer(std::io::stderr)` so logs are on stderr and JSONL is
clean on stdout. Documented in main.rs.

### B-05 (R, medium) — `Player::next` would allocate up to 4 GiB on a corrupt frame length

`vec![0u8; len]` with `len` decoded from the file header — a corrupt
or hostile recording with `len = u32::MAX` would attempt to allocate
4 GiB before the read failed.

**Fix**: hard cap at `MAX_FRAME_BYTES = 16 MiB` (well above any real
`SubscribeUpdate`), returning `Error::FrameTooLarge` when exceeded.

**Test added**: `replay::tests::rejects_oversized_frame`.

### B-06 (U, medium) — sandwich detector flagged same-wallet roundtrips as sandwiches

A wallet that periodically does A→B then B→A trades for unrelated
reasons (e.g., LP rebalancing) was flagged whenever any other-signer
swap landed between them, regardless of whether the round-trip actually
made money.

**Fix**: only emit `SandwichEvent` when `extracted_amount > 0`. Real
sandwiches always extract positive value in the front-leg's input mint;
roundtrips that lost or broke even were almost never attacks.

### B-07 (R, low) — silent drops when radar event channel is full

`tx.try_send` returned `Err(_)` and we discarded with `let _ = ...`. A
slow consumer would lose events silently; users couldn't tell the
difference between "no detections" and "tons of detections you'll never
see".

**Fix**: `radar::run` now counts drops with an `AtomicU64` and emits a
`tracing::warn!(total_drops, "radar event channel full; dropping")`
every 1024 drops. Documented in the function-level rustdoc that lossless
capture should use `record` (which writes raw `SubscribeUpdate`s before
in-process channelling).

---

## Known caveats — not fixed in v0.1, documented and tracked for v0.2

### C-01 (C, medium) — tx-level amounts attribute to the first matched pool

Because of B-02's fix, a Jupiter route emits **one** event referencing
**one** pool, even if the route hit 3 pools. The reported amounts are
correct (they're the wallet's tx-net delta), but the pool attribution
is "first swap detected" — the other pools in the route get no event
and the arb detector misses cross-pool spreads on hop 2 / 3. v0.2's
Codama-IDL parsing fixes this by giving per-instruction amounts.

### C-02 (C, low) — `PoolMap` only refreshes on observed swaps

Pools that just receive deposits / withdrawals without swaps don't
update price. v0.2 will add per-DEX account-data decoders so reserves
refresh on every account update, not just every swap.

### C-03 (C, low) — `Processed` commitment can produce events that are later rolled back

The radar subscribes at `CommitmentLevel::Processed` for lowest latency.
Forks can roll back arb signals or sandwich classifications. Switching
to `Confirmed` is a one-line change in `core::swaps::build_subscribe_request`
when a user wants stronger guarantees at the cost of ~2 slot latency.

### R-01 (R, low) — `record` on Ctrl-C may lose buffered data

Tokio's `select!` cancels the inner future when `ctrl_c` wins, dropping
the `BufWriter`-wrapped `Recorder` without calling `finish()`. The most
recent few writes may not flush. Cap is small (one `BufWriter`'s
default capacity, 8 KiB). Workaround for now: rely on `--duration-secs`
for clean exit. Proper fix: pass a `CancellationToken` into
`record::run` so it can call `recorder.finish()` on cancel.

### P-01 (P, low) — `arb::detect` is O(P²) per pair, run on every swap

For ≤ 50 pools per pair this is fine (≤ 2 500 comparisons per swap). At
500+ pools per pair it becomes the bottleneck. Fix would be incremental:
compute spreads against a sorted-by-price index instead of pairwise.

### P-02 (P, low) — `sandwich::Detector::by_pool` slowly leaks empty deques

We GC stale events only on the same pool's next push. A pool that goes
silent after one swap keeps its deque entry forever. A periodic
`gc_inactive(slot_threshold)` method would fix it; not pressing for
v0.1 since pool counts on Solana are bounded.

### U-01 (U, medium) — arb output ordering is non-deterministic

`PoolMap.pairs()` iterates a `HashMap`, so `arb::detect` returns events
in random order. Bad for golden-file replay tests. v0.2 should sort by
`(slot, pair, spread_bps desc)` for stability.

---

## Verification

After fixes:

```bash
cargo check --manifest-path examples/mev-radar/Cargo.toml --workspace
cargo clippy --manifest-path examples/mev-radar/Cargo.toml --workspace --all-targets -- -D warnings
cargo test  --manifest-path examples/mev-radar/Cargo.toml --workspace
```

12 unit tests pass across `dex::balances` (3 — incl. the closed-account
case), `arb` (2), `pools` (2), `sandwich` (2), `replay` (2 — incl. the
oversized-frame guard), `dex::program_ids` (2). No clippy warnings at
`-D warnings`. Binary builds clean.
