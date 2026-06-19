# AGENTS.md

## Run

```powershell
cargo promises   # alias → test --tests -- --test-threads=1 --nocapture
cargo check --workspace
cargo build --workspace
```

All 20 tests pass.

## Test quirk — `--test-threads=1` is required

Loom uses global state. Parallel execution causes spurious failures.

## Build architecture

- `Cargo.toml`: `loom = "0.7"` (non-optional), edition 2024
- Source files: `use loom::sync::atomic::*`
- Test files: `use loom::*` directly

## Lock implementations (`src/`)

| File | Spin primitive | Ordering |
|------|---------------|----------|
| `spin_lock.rs` | `compare_exchange` | Acquire / Release |
| `ticket_lock.rs` | `fetch_add` + `load` | Relaxed / Acquire / Release |
| `clh_lock.rs` | `AtomicPtr::swap` | AcqRel / Acquire / Release |

All spin loops **must** call `spin_loop()` (`loom::hint::spin_loop`) to avoid Loom `max_branches` exceeded errors.

CLH lock's `Drop` uses `swap(null)` instead of `get_mut()` because Loom's `AtomicPtr` lacks `get_mut`.

All locks return a token from `lock()` that `unlock()` consumes.

## Test files (`tests/`)

| File | Count | Verifies |
|------|-------|----------|
| `multi_valued_memory.rs` | 1 | Load hoisting under `Relaxed` (witness-proven reachable) |
| `message_adjacency.rs` | 2 | RMW adjacency (no double-zero, 3-thread chain) |
| `views.rs` | 7 | RR/RW/WR/WW coherence + Release/Acquire + SC fence + relaxed control |
| `promises.rs` | 4 | Store hoisting: w/o dep, OOTA, syntactic dep, RW-coherence block |
| `lock_tests.rs` | 6 | SpinLock / TicketLock / CLHLock × (mutual_exclusion + message_passing) |

Tests that assert a behaviour **is reachable** use a witness `std::sync::Arc<std::sync::atomic::AtomicBool>` outside `loom::model` to capture the target state, then assert it was reached. Loom does not track std atomics, so the witness adds no branching overhead.

Promises scenarios 1 and 3: Loom does not support store hoisting, so `r1=r2=1` is unreachable. These tests run without outcome assertions — only verifying no UB or deadlock.
