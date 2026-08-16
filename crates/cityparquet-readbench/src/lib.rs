//! The read benchmark's shared vocabulary.
//!
//! The benchmark itself is a binary (`src/main.rs`): a coordinator that
//! drives a whole (format x scenario) matrix, and a `--child` worker it
//! spawns once per measurement. This library holds the parts that the
//! binary, the coordinator, and the integration tests must all spell
//! identically — today just [`format`], the set of formats measured.

pub mod format;
