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

## Fixed in audit follow-up pass

### C-03 (resolved) — commitment level is now configurable

`runtime.commitment` in TOML accepts `processed` / `confirmed` /
`finalized` (default `processed`). Threaded through `swaps`, `record`,
and `radar` via a new `Commitment` enum on `core::config`. Users who
need fork-stable signals at the cost of ~2-slot latency can flip the
config without recompiling.

### R-01 (resolved) — `record` now flushes on Ctrl-C

`record::run` takes an `Arc<Notify>` cancel handle and is split into
`run` (always calls `recorder.finish()`) and `run_inner` (the
cancellable subscribe loop). The binary spawns a Ctrl-C bridge that
notifies on signal, so the `BufWriter` is properly flushed and
shutdown — no more truncated tails on graceful exit.

### P-01 (resolved) — `arb::detect` is now O(N + Σ P_i log P_i)

Replaced the O(P²) pairwise sweep with a per-pair sort and pick of
`(min, max)` price quote. Emits at most one event per pair (the best
opportunity), which is also a more useful product shape than "every
combination above threshold". Iteration uses the new
`PoolMap::iter_quotes()` so we bucket by pair in one pass instead of
doing N `quotes_for_pair` scans.

### P-02 (resolved) — sandwich `by_pool` GC

Added `Detector::gc(now_slot)` that prunes both stale events **and**
empty deques across all pools. The radar loop calls it once per slot
transition. Test added: `gc_drops_inactive_pools`.

### U-01 (resolved) — arb output is now deterministic

`detect` sorts its output by `(slot, pair_base, pair_quote,
spread_bps desc)` before returning, so golden-file replay tests are
stable across HashMap iteration orders.

## Known caveats — still deferred to v0.2

### C-01 (C, medium) — tx-level amounts attribute to the first matched pool

Because of B-02's fix, a Jupiter route emits **one** event referencing
**one** pool, even if the route hit 3 pools. The reported amounts are
correct (they're the wallet's tx-net delta), but the pool attribution
is "first swap detected" — the other pools in the route get no event
and the arb detector misses cross-pool spreads on hop 2 / 3.
**Why deferred**: a real fix needs per-instruction `amount_in /
amount_out`, which requires either Codama-IDL-driven parsing or
hand-coded byte-layout decoders for each DEX. Both are v0.2 work — a
mitigation here would either still produce wrong numbers (per-pool
events with the same tx-level amounts → false arb signals) or block
on the same byte-layout work.

### C-02 (C, low) — `PoolMap` only refreshes on observed swaps

Pools that just receive deposits / withdrawals without swaps don't
update price. **Why deferred**: this needs per-DEX pool-account
decoders. Raydium AMM v4's reserve fields live on separate vault
token accounts (not the pool account), and Whirlpools needs sqrt-price
math from the `Whirlpool` account body. Both are doable but each is
~200-400 LoC of byte parsing — the right scope for a follow-up PR
with proper fixture-based tests.

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
