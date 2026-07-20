# AGENTS.md

## Commands

```powershell
cargo promises   # alias → test --tests -- --test-threads=1 --nocapture
cargo check
cargo build
```

## Test quirks — Loom

- `--test-threads=1` is **required** — Loom uses global state; parallel runs produce spurious failures.
- Loom's `fence(SeqCst)` is stronger than C++11: it uses global causality vectors (version-vector join).

## Build

- Single crate, edition 2024, `loom = "0.7"`.
- Source: `loom::sync::atomic::*` imports. Tests: `loom::sync::*`, `loom::thread` directly.

## Locks (`src/`)

All spin loops **must** call `loom::hint::spin_loop()` (avoids `max_branches`). CLH `Drop` uses `swap(null)` (Loom's `AtomicPtr` lacks `get_mut`). All locks return a token from `lock()` consumed by `unlock()`.

| File | Spin primitive | Ordering |
|------|---------------|----------|
| `spin_lock.rs` | `compare_exchange` | Acquire / Release |
| `ticket_lock.rs` | `fetch_add` + `load` | Relaxed / Acquire / Release |
| `clh_lock.rs` | `AtomicPtr::swap` | AcqRel / Acquire / Release |

## Theory

`relaxed memory concurrency.md` — Chinese companion doc on promising semantics & relaxed memory models.
`docs/boc.md` — Behaviour-Oriented Concurrency paper (English + Chinese translation, with annotations).

## Test files

| File | Count | Verifies |
|------|-------|----------|
| `multi_valued_memory.rs` | 1 | Load hoisting under `Relaxed` |
| `message_adjacency.rs` | 2 | RMW adjacency (no double-zero, 3-thread chain) |
| `views.rs` | 7 | RR/RW/WR/WW coherence + Release/Acquire + SC fence + relaxed control |
| `promises.rs` | 4 | Store hoisting: w/o dep, OOTA, syntactic dep, RW-coherence block |
| `lock_tests.rs` | 6 | SpinLock / TicketLock / CLHLock × (mutual_exclusion + message_passing) |

## Release

```bash
git tag v0.x.x
git push origin v0.x.x
gh release create v0.x.x --title "v0.x.x - Short Description" --notes-file /tmp/release-notes.md
```

Use `--notes-file` (not `--notes`) to avoid escaping issues with `"` and `\` in PowerShell.
