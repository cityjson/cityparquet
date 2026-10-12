//! Package layout (spec 01 "Directory layout", "By-module object-table layout")
//! and the cross-file extension declarations (spec 06).

use super::*;

// ---------------------------------------------------------------------------
// Package layout
// ---------------------------------------------------------------------------

/// The standard object-table files: one per CityGML 3.0 module holding
/// feature classes (spec 01 "The standard object-table files").
pub(super) fn core_object_table_files() -> BTreeSet<String> {
    TAXONOMY
        .iter()
        .map(|c| format!("{}.parquet", module_file(&ModuleKey::Core(c.module))))
        .collect()
}

/// `[a-z0-9]+(_[a-z0-9]+)*` — the snake_case body of a module file name.
pub(super) fn is_snake_case(s: &str) -> bool {
    !s.is_empty()
        && s.split('_').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

pub(super) fn check_layout(files: &[PackageFile], r: &mut Reporter) {
    let core = core_object_table_files();
    let tables: Vec<&PackageFile> = files
        .iter()
        .filter(|f| f.kind == FileKind::ObjectTable)
        .collect();
    // spec 01 "File roles": object tables — at least one is required.
    if tables.is_empty() {
        r.error(
            "package.no-object-table",
            PACKAGE,
            None,
            "the package has no object table".to_string(),
        );
    }
    for table in tables {
        if core.contains(&table.name) {
            continue;
        }
        // spec 01 "By-module object-table layout" / spec 06 "The namespace":
        // a file beyond the standard set is an extension module's, named
        // `<namespace>_<module in snake_case>.parquet`, and a file carrying
        // an extension name MUST declare that namespace in `city.extensions`.
        let stem = table.name.trim_end_matches(".parquet");
        let declared = table.declared_namespaces();
        let extension_module = stem
            .split_once('_')
            .is_some_and(|(ns, module)| declared.contains(ns) && is_snake_case(module));
        if !extension_module {
            r.error(
                "package.object-table-name",
                PACKAGE,
                Some(&table.name),
                "the file name is neither a CityGML 3.0 module file nor \
                 `<namespace>_<module>.parquet` for a namespace this file declares in \
                 `city.extensions`"
                    .to_string(),
            );
        }
    }
}

/// spec 06 "The extension declaration": every file of a package that
/// declares a namespace MUST declare it with the same value.
pub(super) fn check_extension_consistency(files: &[PackageFile], r: &mut Reporter) {
    let mut seen: BTreeMap<&str, (&Value, &str)> = BTreeMap::new();
    for file in files {
        let Some(city) = &file.city else { continue };
        for (ns, value) in &city.extensions {
            match seen.get(ns.as_str()) {
                Some((first, first_file)) if *first != value => r.error(
                    "extension.inconsistent",
                    EXTENSIONS,
                    Some(&file.name),
                    format!(
                        "namespace `{ns}` is declared differently from {first_file}'s declaration"
                    ),
                ),
                Some(_) => {}
                None => {
                    seen.insert(ns, (value, &file.name));
                }
            }
        }
    }
}
