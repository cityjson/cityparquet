//! Derive a STAC Item from a written CityParquet package.
//!
//! Every field is read back from the files on disk — the Parquet footer, the
//! Arrow schema, and the dictionary-encoded `object_type` column. Nothing is
//! accumulated during conversion. This makes spec §13.2's rule ("where the
//! STAC Item and the Parquet footer disagree, the footer is authoritative")
//! true by construction rather than by discipline, and means a package written
//! by any conformant writer can be described, not just this crate's own.

pub mod attribute_type;
