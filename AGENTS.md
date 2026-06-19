# AGENTS.md

## Run

```powershell
cargo promises   # alias → test --tests -- --test-threads=1 --nocapture
```

All 20 tests pass.

## Test quirk — `--test-threads=1` is required

Loom uses global state. Parallel test execution will cause spurious failures.

## Build architecture

- `Cargo.toml`: `loom = "0.7"` (non-optional, always available)
- Source files: always use `loom::sync::atomic::*`
- Test files: always use `loom::*` directly

## Lock implementations (`src/`)

| File | Spin primitive | Ordering |
|------|---------------|----------|
| `spin_lock.rs` | `compare_exchange` | Acquire / Release |
| `ticket_lock.rs` | `fetch_add` + `load` | Relaxed / Acquire / Release |
| `clh_lock.rs` | `AtomicPtr::swap` | AcqRel / Acquire / Release |

All spin loops **must** call `spin_loop()` (`loom::hint::spin_loop`) to avoid Loom `max_branches` exceeded errors.

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

Promises scenarios 1 and 3 (store hoisting w/o dep, syntactic dep): the LB pattern `r1=X;Y=r1 || r2=Y;X=1` is allowed by the C++11 axiomatic model (relaxed atomics permit store-load reordering for different locations).  Loom's operational model cannot find `r1=r2=1` because it executes per-thread ops in program order without reordering.  These tests run without outcome assertions — only verifying no UB or deadlock.

Promises scenario 2 (OOTA) differs: C++11 relaxed **also** allows `r1=r2=1` (the model does not track data dependencies), but Loom blocks it through sequential interleaving.  PS blocks it via data-dependency constraints on promises.  Scenario 2 has a correctness assertion (`r1=r2=1` must be impossible).

Scenario 4 (RW coherence): C++11 blocks `r3=1` via sequenced-before (read happens before write in same thread); PS blocks via promise-fulfillment failure.  Both agree `r1=r2=r3=1` is impossible.
