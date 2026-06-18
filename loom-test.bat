@echo off
cargo test --features loom --tests -- --test-threads=1 --nocapture %*
