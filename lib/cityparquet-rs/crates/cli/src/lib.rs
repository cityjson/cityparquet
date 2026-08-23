//! Library surface for the `cityparquet` CLI crate: today this is just the
//! `bench` module, kept in a library target so integration tests (and the
//! `main.rs` binary, which auto-links against a same-package library target)
//! can call [`bench::run`] directly rather than shelling out to the built
//! binary.

pub mod bench;
