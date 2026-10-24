//! Rows regrouped into feature families while a package streams past, for
//! the exporters (spec 02 "Feature families are contiguous").

use std::collections::{HashMap, HashSet};
use std::path::Path;

use cityparquet_schema::CityParquetError;

use crate::Result;
use crate::export::table_display_name;
use crate::id_hash::id_hash;

/// What the assembler needs of a row.
pub(crate) trait FamilyRow {
    fn id(&self) -> &str;
    /// `None` reads as the row's own id.
    fn feature_id(&self) -> Option<&str>;
    fn children(&self) -> &[String];
}

impl<T> Default for Families<T> {
    fn default() -> Self {
        Self {
            current: None,
            parked: HashMap::new(),
            parked_order: Vec::new(),
            ever_parked: HashSet::new(),
            closed_in_table: HashSet::new(),
            written: HashSet::new(),
        }
    }
}

/// Rows grouped into feature families as they stream past, table by table.
///
/// Within one table a family's rows are one run. When the run ends, the
/// family is written if it is complete — its root row is present and so is
/// every object its rows name as a child — and parked otherwise, to be
/// joined by its rows in later tables. Families still parked after the last
/// table are written as they stand. The package is not trusted to keep the
/// contiguity rule: a run that re-opens, or rows that arrive for a family
/// already written, are refused rather than exported as a split feature.
pub(crate) struct Families<T> {
    current: Option<(String, Vec<T>)>,
    parked: HashMap<String, Vec<T>>,
    /// Parking order, so leftover families are written deterministically.
    parked_order: Vec<String>,
    /// Families parked at least once.
    ever_parked: HashSet<u128>,
    /// Families whose run in the current table has ended.
    closed_in_table: HashSet<u128>,
    /// Every family written, so a late row is refused: one 128-bit hash per
    /// feature of the package.
    written: HashSet<u128>,
}

impl<T: FamilyRow> Families<T> {
    pub(crate) fn push(
        &mut self,
        table: &Path,
        entry: T,
        emit: &mut impl FnMut(String, Vec<T>) -> Result<()>,
    ) -> Result<()> {
        let key = entry.feature_id().unwrap_or(entry.id());
        if let Some((current, entries)) = &mut self.current
            && current == key
        {
            entries.push(entry);
            return Ok(());
        }
        let key = key.to_string();
        self.close_run(emit)?;
        let hash = id_hash(&key);
        if self.closed_in_table.contains(&hash) {
            return Err(CityParquetError::Schema(format!(
                "feature `{key}`: its rows in {} are not contiguous (spec \"Feature families \
                 are contiguous\")",
                table_display_name(table)
            )));
        }
        if self.written.contains(&hash) {
            return Err(CityParquetError::Schema(format!(
                "feature `{key}` has rows in {} after its family was complete; no row names \
                 them as children",
                table_display_name(table)
            )));
        }
        let mut entries = self.parked.remove(&key).unwrap_or_default();
        entries.push(entry);
        self.current = Some((key, entries));
        Ok(())
    }

    fn close_run(&mut self, emit: &mut impl FnMut(String, Vec<T>) -> Result<()>) -> Result<()> {
        let Some((key, entries)) = self.current.take() else {
            return Ok(());
        };
        let hash = id_hash(&key);
        self.closed_in_table.insert(hash);
        if is_complete(&key, &entries) {
            self.written.insert(hash);
            emit(key, entries)
        } else {
            if self.ever_parked.insert(hash) {
                self.parked_order.push(key.clone());
            }
            self.parked.insert(key, entries);
            Ok(())
        }
    }

    pub(crate) fn end_table(
        &mut self,
        emit: &mut impl FnMut(String, Vec<T>) -> Result<()>,
    ) -> Result<()> {
        self.close_run(emit)?;
        self.closed_in_table.clear();
        Ok(())
    }

    /// Writes the families still parked; returns how many families were
    /// parked at all.
    pub(crate) fn finish(
        mut self,
        emit: &mut impl FnMut(String, Vec<T>) -> Result<()>,
    ) -> Result<usize> {
        self.close_run(emit)?;
        for key in std::mem::take(&mut self.parked_order) {
            if let Some(entries) = self.parked.remove(&key) {
                emit(key, entries)?;
            }
        }
        Ok(self.ever_parked.len())
    }
}

/// A family is complete when its root row is present and so is every object
/// its rows name as a child.
fn is_complete<T: FamilyRow>(key: &str, entries: &[T]) -> bool {
    let ids: HashSet<&str> = entries.iter().map(FamilyRow::id).collect();
    ids.contains(key)
        && entries.iter().all(|e| {
            e.children()
                .iter()
                .all(|child| ids.contains(child.as_str()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Row(&'static str, &'static str, Vec<String>);

    impl FamilyRow for Row {
        fn id(&self) -> &str {
            self.0
        }
        fn feature_id(&self) -> Option<&str> {
            Some(self.1)
        }
        fn children(&self) -> &[String] {
            &self.2
        }
    }

    /// A row for a family already written, in a later table, that none of
    /// the family's rows names as a child: exporting it would split the
    /// feature, so it is refused.
    #[test]
    fn a_row_arriving_after_its_family_was_written_is_refused() {
        let mut families = Families::default();
        let mut written = Vec::new();
        let mut emit = |key: String, _: Vec<Row>| {
            written.push(key);
            Ok(())
        };
        families
            .push(
                Path::new("building.parquet"),
                Row("b", "b", vec![]),
                &mut emit,
            )
            .unwrap();
        families.end_table(&mut emit).unwrap();
        let err = families
            .push(
                Path::new("generics.parquet"),
                Row("g", "b", vec![]),
                &mut emit,
            )
            .unwrap_err();
        assert!(
            format!("{err}").contains("after its family was complete"),
            "{err}"
        );
        assert_eq!(written, ["b"]);
    }
}
