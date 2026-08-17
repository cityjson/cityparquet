//! The read benchmark's shared vocabulary.
//!
//! The benchmark itself is a binary (`src/main.rs`): a coordinator that
//! drives a whole (format x scenario) matrix, and a `--child` worker it
//! spawns once per measurement. This library holds the parts that the
//! binary, the coordinator, and the integration tests must all spell
//! identically: [`format`], the set of formats measured, and [`naming`],
//! the input-extension convention every artefact path is derived through.

pub mod format;
pub mod naming;
