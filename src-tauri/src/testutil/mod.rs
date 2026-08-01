//! Test doubles for OS-permission-bound surfaces (§9.3; CLAUDE.md §3 /
//! OPEN_QUESTIONS Q4). Declared unconditionally (not `#[cfg(test)]`) so both
//! this crate's unit tests and the `tests/` integration crate can use them,
//! matching the §11 layout (`testutil/ mocks.rs fixtures.rs wer.rs`).

pub mod mocks;
