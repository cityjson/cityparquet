//! Benchmark variant identifiers — the one grammar behind `cityparquet bench
//! --variants` and `cityparquet-readbench run --variants`.
//!
//! An id is `<preset>[+hilbert][+rg<N>][+<codec>[<level>]]`: a
//! [`RecipePreset`] name, then any of three suffixes, each at most once, in
//! any order on input. [`Variant::id`] spells the same variant back in the
//! fixed order above, and that spelling is what the result CSVs carry.
//!
//! Only `zstd` takes a level (`zstd9`), because zstd is the codec CityParquet
//! ships with and the benchmark sweeps its effort. Every other codec runs at
//! the parquet-rs default the recipe carries (gzip 6, brotli 1), and a level
//! on one of them is a grammar error rather than a second sweep nobody
//! matched across codecs.

use parquet::basic::ZstdLevel;

use cityparquet_schema::{CityParquetError, Result};

use crate::package::RowOrder;
use crate::recipe::{Codec, RecipePreset, WriterRecipe};

/// The grammar, as printed in every rejection.
pub const GRAMMAR: &str = "<preset>[+hilbert][+rg<N>][+<codec>[<level>]]";

/// One parsed variant id. See the module doc for the grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    pub preset: RecipePreset,
    pub ordering: RowOrder,
    pub row_group_size: Option<usize>,
    pub compression: Option<Codec>,
    /// Only ever `Some` together with `compression == Some(Codec::Zstd)`.
    pub zstd_level: Option<i32>,
}

impl Variant {
    /// Parses an id; the error names the id and prints [`GRAMMAR`].
    pub fn parse(id: &str) -> Result<Variant> {
        let mut parts = id.split('+');
        let preset_name = parts.next().unwrap_or("");
        let preset = RecipePreset::parse(preset_name).ok_or_else(|| grammar_err(id, None))?;

        let mut ordering = RowOrder::Source;
        let mut row_group_size: Option<usize> = None;
        let mut compression: Option<Codec> = None;
        let mut zstd_level: Option<i32> = None;
        let mut seen_hilbert = false;
        for part in parts {
            if let Some(digits) = part.strip_prefix("rg") {
                if row_group_size.is_some() {
                    return Err(grammar_err(id, None));
                }
                let n: usize = digits.parse().map_err(|_| grammar_err(id, None))?;
                if n == 0 {
                    return Err(grammar_err(id, None));
                }
                row_group_size = Some(n);
                continue;
            }
            if let Some((codec, level)) = split_codec_token(part) {
                if compression.is_some() {
                    return Err(grammar_err(id, None));
                }
                match (codec, level) {
                    (_, None) => {}
                    (Codec::Zstd, Some(level)) => {
                        ZstdLevel::try_new(level).map_err(|e| {
                            grammar_err(
                                id,
                                Some(&format!("zstd level {level} is out of range: {e}")),
                            )
                        })?;
                        zstd_level = Some(level);
                    }
                    (other, Some(_)) => {
                        return Err(grammar_err(
                            id,
                            Some(&format!("only zstd takes a level, not {}", other.name())),
                        ));
                    }
                }
                compression = Some(codec);
                continue;
            }
            match part {
                "hilbert" if !seen_hilbert => {
                    seen_hilbert = true;
                    ordering = RowOrder::Hilbert;
                }
                _ => return Err(grammar_err(id, None)),
            }
        }

        Ok(Variant {
            preset,
            ordering,
            row_group_size,
            compression,
            zstd_level,
        })
    }

    /// The canonical spelling: preset, then `+hilbert`, `+rg<N>`, `+<codec>[<level>]`.
    pub fn id(&self) -> String {
        let mut id = self.preset.name().to_string();
        if self.ordering == RowOrder::Hilbert {
            id.push_str("+hilbert");
        }
        if let Some(n) = self.row_group_size {
            id.push_str(&format!("+rg{n}"));
        }
        if let Some(codec) = self.compression {
            id.push('+');
            id.push_str(codec.name());
            if let Some(level) = self.zstd_level {
                id.push_str(&level.to_string());
            }
        }
        id
    }

    /// The preset's recipe with this variant's row-group size, codec and
    /// zstd level applied on top.
    pub fn recipe(&self) -> WriterRecipe {
        let mut recipe = self.preset.recipe();
        if let Some(row_group_size) = self.row_group_size {
            recipe.row_group_size = row_group_size;
        }
        if let Some(compression) = self.compression {
            recipe.compression = Some(compression);
        }
        if let Some(level) = self.zstd_level {
            recipe.zstd_level = level;
        }
        recipe
    }

    pub fn ordering(&self) -> RowOrder {
        self.ordering
    }
}

/// `"zstd9"` -> `(Zstd, Some(9))`; `"gzip"` -> `(Gzip, None)`; `"gzip6"` ->
/// `(Gzip, Some(6))` (the caller rejects it); anything else -> `None`.
fn split_codec_token(part: &str) -> Option<(Codec, Option<i32>)> {
    if let Some(codec) = Codec::parse(part) {
        return Some((codec, None));
    }
    let split = part.find(|c: char| c.is_ascii_digit())?;
    let (name, digits) = part.split_at(split);
    let codec = Codec::parse(name)?;
    let level: i32 = digits.parse().ok()?;
    Some((codec, Some(level)))
}

fn grammar_err(id: &str, detail: Option<&str>) -> CityParquetError {
    let presets: Vec<&str> = RecipePreset::ALL.iter().map(|p| p.name()).collect();
    let codecs: Vec<&str> = Codec::ALL.iter().map(|c| c.name()).collect();
    let detail = detail.map(|d| format!(" ({d})")).unwrap_or_default();
    CityParquetError::Schema(format!(
        "invalid variant '{id}'{detail}: expected `{GRAMMAR}` (each suffix at most once, <N> a \
         positive integer, <codec> one of: {}, <level> only after zstd, 1-22) where preset is \
         one of: {}",
        codecs.join(", "),
        presets.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEC_LIST: [&str; 9] = [
        "cityparquet",
        "cityparquet+zstd1",
        "cityparquet+zstd9",
        "cityparquet+zstd19",
        "cityparquet+lz4",
        "cityparquet+snappy",
        "cityparquet+gzip",
        "cityparquet+brotli",
        "cityparquet+uncompressed",
    ];
    const ROWGROUP_LIST: [&str; 5] = [
        "cityparquet",
        "cityparquet+rg32768",
        "cityparquet+rg8192",
        "cityparquet+rg2048",
        "cityparquet+rg512",
    ];

    #[test]
    fn the_two_benchmark_lists_round_trip_through_their_canonical_ids() {
        for id in CODEC_LIST.iter().chain(ROWGROUP_LIST.iter()) {
            let v = Variant::parse(id).unwrap();
            assert_eq!(
                v.id(),
                *id,
                "canonical spelling must be the list's spelling"
            );
            assert_eq!(Variant::parse(&v.id()).unwrap(), v);
        }
    }

    #[test]
    fn a_zstd_level_reaches_the_recipe_and_a_bare_zstd_keeps_the_default() {
        let nine = Variant::parse("cityparquet+zstd9").unwrap();
        assert_eq!(nine.compression, Some(Codec::Zstd));
        assert_eq!(nine.zstd_level, Some(9));
        assert_eq!(nine.recipe().zstd_level, 9);
        assert_eq!(nine.recipe().compression, Some(Codec::Zstd));

        let bare = Variant::parse("cityparquet+zstd").unwrap();
        assert_eq!(bare.zstd_level, None);
        assert_eq!(bare.recipe().zstd_level, 3);
        assert_eq!(bare.id(), "cityparquet+zstd");

        let default = Variant::parse("cityparquet").unwrap();
        assert_eq!(default.recipe(), WriterRecipe::default());
        assert_eq!(default.ordering(), RowOrder::Source);
    }

    #[test]
    fn only_zstd_takes_a_level() {
        for id in [
            "cityparquet+gzip6",
            "cityparquet+brotli11",
            "cityparquet+snappy1",
        ] {
            let err = Variant::parse(id).unwrap_err().to_string();
            assert!(err.contains(id), "{err}");
            assert!(err.contains("only zstd takes a level"), "{err}");
        }
    }

    #[test]
    fn a_zstd_level_outside_the_codec_range_is_rejected() {
        for id in ["cityparquet+zstd0", "cityparquet+zstd23"] {
            let err = Variant::parse(id).unwrap_err().to_string();
            assert!(err.contains(id), "{err}");
        }
    }

    #[test]
    fn duplicates_malformed_suffixes_and_unknown_presets_are_rejected_with_the_grammar() {
        for id in [
            "cityparquet+hilbert+hilbert",
            "cityparquet+rg4096+rg8192",
            "cityparquet+gzip+zstd",
            "cityparquet+rg0",
            "cityparquet+rg-1",
            "cityparquet+rgabc",
            "cityparquet+rg",
            "not-a-real-preset",
            "cityparquet+bogus",
            "",
        ] {
            let err = Variant::parse(id).unwrap_err().to_string();
            assert!(err.contains(id), "error must name the offending id: {err}");
            assert!(err.contains(GRAMMAR), "error must show the grammar: {err}");
        }
    }

    #[test]
    fn suffix_order_on_input_does_not_matter_but_the_id_is_canonical() {
        let a = Variant::parse("cityparquet+rg512+hilbert+gzip").unwrap();
        let b = Variant::parse("cityparquet+gzip+hilbert+rg512").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.id(), "cityparquet+hilbert+rg512+gzip");
        assert_eq!(a.ordering(), RowOrder::Hilbert);
        assert_eq!(a.recipe().row_group_size, 512);
        assert_eq!(a.recipe().compression, Some(Codec::Gzip));
    }
}
