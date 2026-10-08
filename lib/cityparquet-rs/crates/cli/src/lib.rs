//! Library surface for the `cityparquet` CLI crate: the `bench` harness and,
//! with the `collection` feature, the STAC Collection writer — kept in a
//! library target so integration tests (and the `main.rs` binary, which
//! auto-links against a same-package library target) can call them directly
//! rather than shelling out to the built binary.

pub mod bench;
#[cfg(feature = "collection")]
pub mod collection;
