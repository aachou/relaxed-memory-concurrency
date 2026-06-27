# AGENTS.md

## Run

```powershell
cargo promises   # alias → test --tests -- --test-threads=1 --nocapture
cargo check --workspace
cargo build --workspace
```

All 28 tests pass.

## Test quirk — `--test-threads=1` is required

Loom uses global state. Parallel execution causes spurious failures.

## Build architecture

- `Cargo.toml`: `loom = "0.7"` (non-optional), edition 2024
- Source files: `use loom::sync::atomic::*`
- Test files: `use loom::*` directly

## EBR implementation (`src/ebr.rs`)

- Fraser epoch algorithm, RFC `crossbeam-relaxed-memory.md` alignment
- `pin()`: `load(Relaxed)` → `store(Relaxed)` → `fence(SeqCst)`
- `unpin()`: `store(SENTINEL, Release)`
- `retire()`: `fence(SeqCst)` → `global_epoch.load(Relaxed)` → push to `retire_lists[epoch]`
- `try_advance()`: `load(Relaxed)` → `fence(SeqCst)` → check all threads → `fence(Acquire)` → `store(Release)` → free `list[(g+2)%3]`
- 3 global retire lists `Mutex<[Vec<usize>; 3]>` indexed by epoch
- Object freed after exactly 2 epoch advances from `retire()` call (uses `global_epoch`, not `local_epoch`)

## EBR test patterns

- Sequential tests (`test_basic_reclamation`, etc.): single-thread retire + epoch advancement
- RFC Case 1 (`test_rfc_case1_retire_before_pin`): shared `AtomicUsize` simulates data structure; `done` flag (Release/Acquire) verifies U's removal is visible to A when retire completes before A's pin
- RFC Case 2 (`test_rfc_case2_pin_before_retire`): A reads shared data while U concurrently retires; loom explores all interleavings verifying no UB
- Advance blocking (`test_pinned_thread_blocks_advance`): external `std::sync::atomic::AtomicBool` witness verifies loom finds an interleaving where `try_advance` is blocked by a pinned thread
- Concurrent safety (`test_concurrent_safety_fuzz`): external `std::sync::atomic::AtomicBool` witness pattern detects premature free (pinning thread still active when object freed)

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
| `ebr_tests.rs` | 8 | EBR GC: protocol correctness + RFC Case 1/2 concurrent + advance blocking + safety fuzz |
| `multi_valued_memory.rs` | 1 | Load hoisting under `Relaxed` (witness-proven reachable) |
| `message_adjacency.rs` | 2 | RMW adjacency (no double-zero, 3-thread chain) |
| `views.rs` | 7 | RR/RW/WR/WW coherence + Release/Acquire + SC fence + relaxed control |
| `promises.rs` | 4 | Store hoisting: w/o dep, OOTA, syntactic dep, RW-coherence block |
| `lock_tests.rs` | 6 | SpinLock / TicketLock / CLHLock × (mutual_exclusion + message_passing) |

Tests that assert a behaviour **is reachable** use a witness `std::sync::Arc<std::sync::atomic::AtomicBool>` outside `loom::model` to capture the target state, then assert it was reached. Loom does not track std atomics, so the witness adds no branching overhead.

Promises scenarios 1 and 3: Loom does not support store hoisting, so `r1=r2=1` is unreachable. These tests run without outcome assertions — only verifying no UB or deadlock.

## Release

```bash
# Bump version in Cargo.toml, then:
git tag v0.x.x
git push origin v0.x.x
gh release create v0.x.x --title "v0.x.x - Short Description" --notes-file /tmp/release-notes.md
```

**Note**: Use `--notes-file` with a temp file, not `--notes`. The latter causes escaping issues with `"` and `\` in PowerShell.
