# AGENTS.md

## Run

```powershell
cargo loom                          # alias: test --features loom --tests -- --test-threads=1 --nocapture
loom-test.bat                       # same as above
```

All 21 tests pass. Running without `--features loom` compiles the lib (std atomics) but skips all tests.

## Test quirk — `--test-threads=1` is required

Loom uses global state. Parallel test execution will cause spurious failures. The `cargo loom` alias enforces this.

## Build architecture

- `build.rs`: detects `CARGO_FEATURE_LOOM` → emits `cargo:rustc-cfg=loom`
- `Cargo.toml`: `loom = { version = "0.7", optional = true }`
- Source files: `#[cfg(loom)]` → `loom::sync::atomic::*`, `#[cfg(not(loom))]` → `std::sync::atomic::*`
- Test files: always use `loom::*` directly (only compile under `--features loom`)

## Lock implementations (`src/`)

| File | Spin primitive | Ordering |
|------|---------------|----------|
| `spin_lock.rs` | `compare_exchange` | Acquire / Release |
| `ticket_lock.rs` | `fetch_add` + `load` | Relaxed / Acquire / Release |
| `clh_lock.rs` | `AtomicPtr::swap` | AcqRel / Acquire / Release |

All spin loops **must** call `spin_loop()` (conditional: `loom::hint::spin_loop` / `std::hint::spin_loop`) to avoid Loom `max_branches` exceeded errors.

CLH lock's `Drop` uses `swap(null)` instead of `get_mut()` because Loom's `AtomicPtr` lacks `get_mut`.

## Test files (`tests/`)

| File | Tests | What it verifies |
|------|-------|-----------------|
| `multi_valued_memory.rs` | 2 | Load hoisting under `Relaxed` |
| `message_adjacency.rs` | 2 | RMW adjacency (no double-zero, 3-thread chain) |
| `views.rs` | 7 | RR/RW/WR/WW coherence + Release/Acquire + SC fence + relaxed control |
| `promises.rs` | 4 | Store hoisting: w/o dep, OOTA, syntactic dep, RW-coherence block |
| `lock_tests.rs` | 6 | SpinLock / TicketLock / CLHLock × (mutual_exclusion + message_passing) |

Tests that assert a behaviour **is reachable** (wo-dep promises, store-buffering) use `let _v = ...` without assertion — Loom exploring all schedules without error is the verification.
