# Agent guidance for this repo

## Commands

- `cargo test` — run all tests with standard Rust concurrency
- `cargo loom-test` — run tests under Loom exhaustive model checking (defined in `.cargo/config.toml`)

## Loom gating

`src/test/loom.rs` conditionally re-exports `std::*` or `loom::*` based on the `check-loom` feature. All tests call `loom::model(f)` which wraps `loom::model(f)` when `check-loom` is active, otherwise runs `f()` directly.

Tests import `loom::sync::Arc`, `loom::sync::atomic::{AtomicUsize, Ordering, fence}`, and `loom::thread` instead of `std` equivalents — this works because of the re-export layer.

## Test quirks

- `relaxed_no_sync` in `tests/views.rs` has `#[should_panic]` — it is meant to fail under Loom.
- Some promise tests (`store_hoisting_wo_dep`, `store_hoisting_syntactic_dep`) use an external `reached` flag whose assertion is currently commented out — the tests always pass with `assert!(true)`. Do not uncomment blindly; Loom does not support store hoisting.
- Integration tests live in `tests/` (4 files: `multi_valued_memory`, `message_adjacency`, `views`, `promises`).
