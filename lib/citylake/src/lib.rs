//! CityLake — a lakehouse runtime for CityParquet packages.

pub mod core;

#[cfg(feature = "server")]
pub mod app;
