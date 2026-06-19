# AGENTS.md

## Run

```powershell
cargo loom                          # alias: test --features loom --tests -- --test-threads=1 --nocapture
loom-test.bat                       # same as above
```

All 20 tests pass. Running without `--features loom` compiles the lib (std atomics) but skips all tests. Running without `--features loom` compiles the lib (std atomics) but skips all tests.

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
| `multi_valued_memory.rs` | 1 | Load hoisting under `Relaxed` (witness-proven reachable) |
| `message_adjacency.rs` | 2 | RMW adjacency (no double-zero, 3-thread chain) |
| `views.rs` | 7 | RR/RW/WR/WW coherence + Release/Acquire + SC fence + relaxed control |
| `promises.rs` | 4 | Store hoisting: w/o dep, OOTA, syntactic dep, RW-coherence block |
| `lock_tests.rs` | 6 | SpinLock / TicketLock / CLHLock × (mutual_exclusion + message_passing) |

Tests that assert a behaviour **is reachable** (store-buffering) use a witness `std::sync::Arc<std::sync::atomic::AtomicBool>` outside `loom::model` to capture the target state, then assert it was reached.  Loom does not track std atomics, so the witness adds no branching overhead.

Promises scenarios 1 and 3 (store hoisting w/o dep, syntactic dep) are **compiler optimizations** that Loom (C11-based) cannot model.  These tests run without outcome assertions — only verifying no UB or deadlock.
