//! What a scenario RETURNED, as every runner reports it to the coordinator.
//!
//! The return rule: a spatial window returns the matching CityObjects'
//! identifiers and each one's highest-LoD geometry; an attribute filter
//! returns the matching identifiers; read all and an identifier lookup
//! return every field of every record they read. Returned geometry means
//! its coordinates were VISITED IN PLACE (real `f64` values, no conversion
//! into another representation), so a zero-copy format cannot win by never
//! touching them.
//!
//! A child reports these on stderr after its timed stdout line, one marker
//! line each, so the timed protocol and the results CSV keep their shape:
//!
//! - [`ID_DIGEST_MARKER`] `<count> <sum> <xor>` — an order-independent
//!   digest of the returned identifier set (see [`IdDigest`]).
//! - [`TOTALS_MARKER`] `<objects> <geometries> <semantic_faces> <min x>
//!   <min y> <min z> <max x> <max y> <max z>` — the comparable totals of read
//!   all and an identifier lookup (see [`ComparableTotals`]).
//! - [`RETURNED_GEOMETRY_MARKER`] `<geometries> <min x> <min y> <min z> <max
//!   x> <max y> <max z>` — the spatial window's returned geometries and the
//!   extent of their visited coordinates (see [`ReturnedGeometry`]).
//!
//! Extents are in the ARTEFACT's own axis order; the coordinator brings them
//! into one order before comparing.

use anyhow::{Context, Result, bail};

/// Marker of the [`IdDigest`] line.
pub const ID_DIGEST_MARKER: &str = "cityparquet-readbench: id-digest";
/// Marker of the [`ComparableTotals`] line.
pub const TOTALS_MARKER: &str = "cityparquet-readbench: totals";
/// Marker of the [`ReturnedGeometry`] line.
pub const RETURNED_GEOMETRY_MARKER: &str = "cityparquet-readbench: returned-geometry";

/// `[min x, min y, min z, max x, max y, max z]` of no coordinate at all.
pub const EMPTY_EXTENT: [f64; 6] = [
    f64::INFINITY,
    f64::INFINITY,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NEG_INFINITY,
    f64::NEG_INFINITY,
];

/// 64-bit FNV-1a over `bytes`: fixed constants, so every process (and every
/// format's child) hashes an identifier to the same value — unlike std's
/// randomly seeded `RandomState`.
#[inline]
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// An order-independent digest of a returned identifier set: the count,
/// plus the wrapping sum and the XOR of [`fnv1a64`] of each identifier's
/// UTF-8 bytes. It folds one identifier at a time, so a runner never
/// collects or sorts the identifiers inside its timed region; the cost is
/// one pass over each identifier's bytes. Two sets with the same digest are
/// the same set up to a 64-bit hash collision, and a duplicated identifier
/// changes `count` and `sum` (so a format returning an object twice fails).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdDigest {
    pub count: u64,
    pub sum: u64,
    pub xor: u64,
}

impl IdDigest {
    /// Folds one returned identifier in.
    #[inline]
    pub fn push(&mut self, id: &str) {
        let h = fnv1a64(id.as_bytes());
        self.count += 1;
        self.sum = self.sum.wrapping_add(h);
        self.xor ^= h;
    }

    /// The digest of every identifier in `ids`.
    pub fn of<'a>(ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut digest = Self::default();
        for id in ids {
            digest.push(id);
        }
        digest
    }
}

/// The totals of read all and an identifier lookup that must agree across
/// formats: objects read, geometries read, faces carrying a semantic-surface
/// reference, and the extent of every visited coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComparableTotals {
    pub objects: u64,
    pub geometries: u64,
    pub semantic_faces: u64,
    pub extent: [f64; 6],
}

impl Default for ComparableTotals {
    fn default() -> Self {
        Self {
            objects: 0,
            geometries: 0,
            semantic_faces: 0,
            extent: EMPTY_EXTENT,
        }
    }
}

impl From<&cityparquet::visit::VisitTotals> for ComparableTotals {
    fn from(t: &cityparquet::visit::VisitTotals) -> Self {
        Self {
            objects: t.objects,
            geometries: t.geometries,
            semantic_faces: t.semantic_faces,
            extent: t.extent,
        }
    }
}

/// The spatial window's returned geometries (one per returned object that
/// has any) and the extent of their visited coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReturnedGeometry {
    pub geometries: u64,
    pub extent: [f64; 6],
}

impl Default for ReturnedGeometry {
    fn default() -> Self {
        Self {
            geometries: 0,
            extent: EMPTY_EXTENT,
        }
    }
}

/// Everything a run reports about what it returned; each part is `None`
/// when the scenario (or the format, for now) does not report it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Returned {
    pub ids: Option<IdDigest>,
    pub totals: Option<ComparableTotals>,
    pub geometry: Option<ReturnedGeometry>,
}

fn extent_fields(e: &[f64; 6]) -> String {
    e.iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Returned {
    /// The marker lines of every reported part, in a fixed order.
    pub fn marker_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(d) = self.ids {
            lines.push(format!(
                "{ID_DIGEST_MARKER} {} {} {}",
                d.count, d.sum, d.xor
            ));
        }
        if let Some(t) = self.totals {
            lines.push(format!(
                "{TOTALS_MARKER} {} {} {} {}",
                t.objects,
                t.geometries,
                t.semantic_faces,
                extent_fields(&t.extent)
            ));
        }
        if let Some(g) = self.geometry {
            lines.push(format!(
                "{RETURNED_GEOMETRY_MARKER} {} {}",
                g.geometries,
                extent_fields(&g.extent)
            ));
        }
        lines
    }

    /// Reads every marker line in a child's `stderr` back; an absent marker
    /// leaves its part `None`, a malformed one is an error.
    pub fn parse(stderr: &str) -> Result<Self> {
        let mut out = Self::default();
        for line in stderr.lines() {
            if let Some(rest) = line.strip_prefix(ID_DIGEST_MARKER) {
                let [count, sum, xor] = ints::<3>(rest, ID_DIGEST_MARKER)?;
                out.ids = Some(IdDigest { count, sum, xor });
            } else if let Some(rest) = line.strip_prefix(TOTALS_MARKER) {
                let fields: Vec<&str> = rest.split_whitespace().collect();
                if fields.len() != 9 {
                    bail!("expected nine fields after '{TOTALS_MARKER}', got '{rest}'");
                }
                let [objects, geometries, semantic_faces] =
                    ints::<3>(&fields[..3].join(" "), TOTALS_MARKER)?;
                out.totals = Some(ComparableTotals {
                    objects,
                    geometries,
                    semantic_faces,
                    extent: floats(&fields[3..], TOTALS_MARKER)?,
                });
            } else if let Some(rest) = line.strip_prefix(RETURNED_GEOMETRY_MARKER) {
                let fields: Vec<&str> = rest.split_whitespace().collect();
                if fields.len() != 7 {
                    bail!("expected seven fields after '{RETURNED_GEOMETRY_MARKER}', got '{rest}'");
                }
                let [geometries] = ints::<1>(fields[0], RETURNED_GEOMETRY_MARKER)?;
                out.geometry = Some(ReturnedGeometry {
                    geometries,
                    extent: floats(&fields[1..], RETURNED_GEOMETRY_MARKER)?,
                });
            }
        }
        Ok(out)
    }
}

fn ints<const N: usize>(text: &str, marker: &str) -> Result<[u64; N]> {
    let values: Vec<u64> = text
        .split_whitespace()
        .map(|f| {
            f.parse::<u64>()
                .with_context(|| format!("parsing '{f}' after '{marker}'"))
        })
        .collect::<Result<_>>()?;
    values.try_into().map_err(|v: Vec<u64>| {
        anyhow::anyhow!("expected {N} integers after '{marker}', got {}", v.len())
    })
}

fn floats(fields: &[&str], marker: &str) -> Result<[f64; 6]> {
    let mut out = [0.0; 6];
    for (slot, f) in out.iter_mut().zip(fields) {
        *slot = f
            .parse::<f64>()
            .with_context(|| format!("parsing '{f}' after '{marker}'"))?;
    }
    Ok(out)
}

/// Whether two extents agree within `quantum` per axis (`[x, y, z]`): each
/// bound may differ by at most one quantisation step plus a relative 1e-9
/// for the `f64` rounding of a decoded value. Two empty extents agree.
pub fn extents_agree(a: &[f64; 6], b: &[f64; 6], quantum: [f64; 3]) -> bool {
    (0..6).all(|i| {
        let (x, y) = (a[i], b[i]);
        if x == y {
            return true;
        }
        let tol = quantum[i % 3] + 1e-9 * x.abs().max(y.abs());
        x.is_finite() && y.is_finite() && (x - y).abs() <= tol
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_does_not_depend_on_order() {
        let a = IdDigest::of(["NL.1", "NL.2", "NL.3"]);
        let b = IdDigest::of(["NL.3", "NL.1", "NL.2"]);
        assert_eq!(a, b);
        assert_eq!(a.count, 3);
        assert_ne!(a, IdDigest::of(["NL.1", "NL.2", "NL.4"]));
    }

    #[test]
    fn a_duplicated_identifier_changes_the_digest() {
        assert_ne!(IdDigest::of(["a", "b"]), IdDigest::of(["a", "b", "b"]));
    }

    #[test]
    fn fnv1a64_is_the_published_function() {
        // The FNV-1a 64 test vectors: "" and "a".
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn marker_lines_round_trip() {
        let returned = Returned {
            ids: Some(IdDigest::of(["x", "y"])),
            totals: Some(ComparableTotals {
                objects: 4,
                geometries: 5,
                semantic_faces: 6,
                extent: [1.5, 2.0, -3.25, 4.0, 5.0, 6.0],
            }),
            geometry: Some(ReturnedGeometry::default()),
        };
        let stderr = format!("log line\n{}\n", returned.marker_lines().join("\n"));
        assert_eq!(Returned::parse(&stderr).unwrap(), returned);
        assert_eq!(Returned::parse("nothing\n").unwrap(), Returned::default());
        assert!(Returned::parse(&format!("{TOTALS_MARKER} 1 2\n")).is_err());
    }

    #[test]
    fn extents_agree_within_one_quantum_and_no_further() {
        let a = [100.0, 200.0, 0.0, 110.0, 210.0, 30.0];
        let mut b = a;
        b[0] += 0.001;
        assert!(extents_agree(&a, &b, [0.001, 0.001, 0.001]));
        b[0] += 0.001;
        assert!(!extents_agree(&a, &b, [0.001, 0.001, 0.001]));
        assert!(extents_agree(&EMPTY_EXTENT, &EMPTY_EXTENT, [0.0; 3]));
        assert!(!extents_agree(&EMPTY_EXTENT, &a, [1.0; 3]));
    }
}
