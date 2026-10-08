//! Object tables (spec 02) and sidecars (spec 04): their columns and values.

use super::*;

// ---------------------------------------------------------------------------
// Per-LoD column set (spec 03 "Levels of detail")
// ---------------------------------------------------------------------------

pub(super) const LOD_PREFIXES: [&str; 4] =
    ["geometry", "geometry_properties", "material", "texture"];

/// Split `geometry_properties_lod2_2` into (`geometry_properties`, `lod2_2`).
pub(super) fn split_lod_column(name: &str) -> Option<(&'static str, &str)> {
    // Longest prefix first, so `geometry_properties_…` is not read as
    // `geometry_…`.
    for prefix in ["geometry_properties", "geometry", "material", "texture"] {
        if let Some(rest) = name.strip_prefix(prefix).and_then(|r| r.strip_prefix('_'))
            && rest.starts_with("lod")
        {
            return Some((prefix, rest));
        }
    }
    None
}

/// The expected field of a per-LoD column (spec 03/04).
pub(super) fn lod_field(prefix: &str, name: &str) -> Field {
    match prefix {
        "geometry_properties" => Field::new(name, geometry_properties_data_type(), true),
        "material" => Field::new(name, material_data_type(), true),
        "texture" => Field::new(name, texture_data_type(), true),
        _ => Field::new(name, DataType::Binary, true),
    }
}

/// Check the per-LoD quartets among `fields` and return the LoD suffixes
/// that have a geometry column, with each geometry column's annotation.
pub(super) fn check_lod_columns(
    r: &mut Reporter,
    file: &str,
    fields: &[Arc<Type>],
    attributes: &BTreeSet<&str>,
) -> BTreeMap<String, Option<Option<String>>> {
    let mut present: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut geometry: BTreeMap<String, Option<Option<String>>> = BTreeMap::new();
    for field in fields {
        let name = field.name();
        if attributes.contains(name) {
            continue;
        }
        // spec 03 "Levels of detail": every geometry column is suffixed;
        // there is no un-suffixed `geometry`, `geometry_properties`,
        // `material` or `texture` column.
        if LOD_PREFIXES.contains(&name) {
            r.error(
                "column.unsuffixed-geometry",
                GEOMETRY,
                Some(file),
                format!("`{name}` has no LoD suffix"),
            );
            continue;
        }
        let Some((prefix, suffix)) = split_lod_column(name) else {
            continue;
        };
        // spec 03 "Suffix grammar": `lod<major>_<minor>`; a suffix always
        // carries a minor.
        if Lod::from_column_suffix(suffix).is_none() {
            r.error(
                "column.lod-suffix",
                GEOMETRY,
                Some(file),
                format!("`{name}` does not carry a `lod<major>_<minor>` suffix"),
            );
            continue;
        }
        present.entry(suffix).or_default().insert(prefix);
        if prefix == "geometry" {
            let annotation = geometry_annotation(r, file, name, field);
            geometry.insert(suffix.to_string(), annotation);
        } else {
            check_type(
                r,
                file,
                if prefix == "geometry_properties" {
                    GEOMETRY
                } else {
                    APPEARANCE
                },
                name,
                &lod_field(prefix, name),
                field,
            );
        }
    }
    // spec 02 "Optional data is NULL, not an omitted column": every LoD with
    // a `geometry_lod*` column also has its `geometry_properties_lod*`,
    // `material_lod*` and `texture_lod*` columns; and no companion stands
    // without its geometry column.
    for (suffix, prefixes) in &present {
        if prefixes.contains("geometry") {
            for prefix in LOD_PREFIXES.iter().filter(|p| !prefixes.contains(*p)) {
                r.error(
                    "column.lod-companion",
                    OBJECT_TABLE,
                    Some(file),
                    format!("`geometry_{suffix}` has no companion `{prefix}_{suffix}`"),
                );
            }
        } else {
            r.error(
                "column.lod-companion",
                OBJECT_TABLE,
                Some(file),
                format!(
                    "LoD `{suffix}` has appearance or properties columns but no `geometry_{suffix}`"
                ),
            );
        }
    }
    geometry
}

// ---------------------------------------------------------------------------
// Object tables
// ---------------------------------------------------------------------------

/// The fixed reserved columns, in the spec's order, as `cityparquet-schema`
/// renders them (spec 02 "Reserved columns").
pub(super) fn fixed_reserved_fields() -> Vec<Field> {
    let schema: Schema = CityParquetSchema {
        lods: vec![Lod::parse("0").expect("a valid LoD")],
        geoparquet_lods: Vec::new(),
        attributes: Vec::new(),
        crs: None,
        extension_namespaces: Vec::new(),
    }
    .to_arrow_schema()
    .expect("the canonical reserved schema renders");
    schema
        .fields()
        .iter()
        .filter(|f| split_lod_column(f.name()).is_none())
        .map(|f| f.as_ref().clone())
        .collect()
}

/// An attribute column's logical type (spec 02 "Attribute types and
/// promotion"): the recommended mapping is a SHOULD, the temporal rules are
/// MUSTs.
pub(super) fn check_attribute_type(r: &mut Reporter, file: &str, name: &str, t: &Type) {
    if !t.is_group() {
        let info = t.get_basic_info();
        match (t.get_physical_type(), info.logical_type_ref()) {
            (
                PhysicalType::INT64,
                Some(LogicalType::Timestamp {
                    is_adjusted_to_u_t_c,
                    unit,
                }),
            ) => {
                // spec 02 "Temporal columns": a TIMESTAMP column MUST be
                // marked UTC-adjusted.
                if !is_adjusted_to_u_t_c {
                    r.error(
                        "attribute.timestamp-utc",
                        OBJECT_TABLE,
                        Some(file),
                        format!("TIMESTAMP attribute `{name}` is not UTC-adjusted"),
                    );
                }
                // The unit is a writer's choice between MILLIS and MICROS.
                if matches!(unit, TimeUnit::NANOS) {
                    r.warn(
                        "attribute.timestamp-unit",
                        OBJECT_TABLE,
                        Some(file),
                        format!("TIMESTAMP attribute `{name}` is in NANOS, not MILLIS or MICROS"),
                    );
                }
                return;
            }
            (PhysicalType::INT32, Some(LogicalType::Date))
            | (PhysicalType::INT32 | PhysicalType::INT64, Some(LogicalType::Time { .. })) => {
                return;
            }
            _ => {}
        }
        // The legacy converted-type spelling of the same logical types; a
        // `TIMESTAMP_*` converted type is UTC-adjusted by definition.
        if info.logical_type_ref().is_none()
            && matches!(
                (t.get_physical_type(), info.converted_type()),
                (
                    PhysicalType::INT32,
                    ConvertedType::DATE | ConvertedType::TIME_MILLIS
                ) | (
                    PhysicalType::INT64,
                    ConvertedType::TIMESTAMP_MILLIS
                        | ConvertedType::TIMESTAMP_MICROS
                        | ConvertedType::TIME_MICROS
                )
            )
        {
            return;
        }
        if matches!(
            leaf_of(t),
            Some(Leaf::Boolean | Leaf::Bigint | Leaf::Double | Leaf::Varchar | Leaf::Json)
        ) {
            return;
        }
    } else if list_element(t).is_some_and(|e| leaf_of(e) == Some(Leaf::Varchar)) {
        return;
    }
    r.warn(
        "attribute.type",
        OBJECT_TABLE,
        Some(file),
        format!(
            "attribute `{name}` is {}, outside the recommended attribute types",
            describe(t)
        ),
    );
}

pub(super) fn check_object_table(
    file: &PackageFile,
    ids: &SidecarIds,
    hierarchy: &mut HashMap<String, HierarchyRow>,
    r: &mut Reporter,
) {
    let name = file.name.as_str();
    let fields = file.root_fields();
    let empty = Vec::new();
    let listed: &Vec<String> = file.city.as_ref().map_or(&empty, |c| &c.attributes);
    let attributes: BTreeSet<&str> = listed.iter().map(String::as_str).collect();
    let field_named = |n: &str| fields.iter().find(|f| f.name() == n);

    // Reserved columns: present, and of the spec's logical type.
    let reserved = fixed_reserved_fields();
    for expected in &reserved {
        let spec = if expected.name() == "bbox" {
            GEOMETRY
        } else {
            OBJECT_TABLE
        };
        match field_named(expected.name()) {
            Some(actual) => check_type(r, name, spec, expected.name(), expected, actual),
            // spec 02 "Reserved columns" / "Optional data is NULL, not an
            // omitted column".
            None => r.error(
                "column.missing",
                OBJECT_TABLE,
                Some(name),
                format!("the reserved column `{}` is missing", expected.name()),
            ),
        }
    }
    let geometry = check_lod_columns(r, name, fields, &attributes);

    // spec 02 "Column naming and reservation rules": attribute names MUST
    // NOT collide with a reserved name; a column `city.attributes` does not
    // list is a reserved column whose name the spec fixes.
    let reserved_names: BTreeSet<&str> = reserved.iter().map(|f| f.name().as_str()).collect();
    for attribute in &attributes {
        if reserved_names.contains(attribute)
            || LOD_PREFIXES.contains(attribute)
            || split_lod_column(attribute).is_some_and(|(_, s)| geometry.contains_key(s))
        {
            r.error(
                "footer.attributes",
                OBJECT_TABLE,
                Some(name),
                format!("attribute `{attribute}` collides with a reserved column name"),
            );
        }
        match field_named(attribute) {
            Some(t) => check_attribute_type(r, name, attribute, t),
            None => r.error(
                "footer.attributes",
                METADATA,
                Some(name),
                format!("`city.attributes` lists `{attribute}`, which is not a column"),
            ),
        }
    }
    for field in fields {
        let n = field.name();
        let known = attributes.contains(n)
            || reserved_names.contains(n)
            || LOD_PREFIXES.contains(&n)
            || split_lod_column(n).is_some();
        if !known {
            r.error(
                "footer.attributes",
                OBJECT_TABLE,
                Some(name),
                format!("`{n}` is neither a reserved column nor listed in `city.attributes`"),
            );
        }
    }
    check_column_order(r, name, fields, &reserved, &attributes);

    // Values.
    let batches = read_batches(file, r);
    let mut stats: BTreeMap<String, GeometryStats> = geometry
        .keys()
        .map(|s| (s.clone(), GeometryStats::default()))
        .collect();
    let mut has_coordinates = false;
    let mut type_cache: HashMap<String, Option<(&'static str, String)>> = HashMap::new();
    let mut offset = 0;
    for batch in &batches {
        check_object_values(
            r,
            file,
            batch,
            offset,
            ids,
            &mut stats,
            &mut has_coordinates,
            &mut type_cache,
            hierarchy,
        );
        offset += batch.num_rows();
    }

    check_object_footer(r, file, &geometry, &stats, has_coordinates);
}

/// spec 02 "Reserved columns": reserved columns appear in the order of the
/// table, before any attribute column. The order is normative for writers,
/// but no MUST is attached and readers match by name, so a deviation is a
/// warning.
pub(super) fn check_column_order(
    r: &mut Reporter,
    file: &str,
    fields: &[Arc<Type>],
    reserved: &[Field],
    attributes: &BTreeSet<&str>,
) {
    let rank = |n: &str| -> Option<(usize, Option<Lod>, usize)> {
        let fixed = |k: &str| reserved.iter().position(|f| f.name() == k);
        if let Some((prefix, suffix)) = split_lod_column(n) {
            let lod = Lod::from_column_suffix(suffix)?;
            let within = LOD_PREFIXES.iter().position(|p| *p == prefix)?;
            // The LoD quartets sit between `bbox` and `implicit_geometry`.
            return Some((fixed("bbox")? + 1, Some(lod), within));
        }
        let at = fixed(n)?;
        let after_quartets = usize::from(at > fixed("bbox")?);
        Some((at + after_quartets, None, 0))
    };
    let mut last: Option<(usize, Option<Lod>, usize)> = None;
    let mut seen_attribute = false;
    for field in fields {
        let n = field.name();
        if attributes.contains(n) {
            seen_attribute = true;
            continue;
        }
        let Some(rk) = rank(n) else { continue };
        if seen_attribute || last.is_some_and(|l| rk < l) {
            r.warn(
                "column.order",
                OBJECT_TABLE,
                Some(file),
                format!("the reserved column `{n}` is out of the spec's column order"),
            );
            return;
        }
        last = Some(rk);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_object_values(
    r: &mut Reporter,
    file: &PackageFile,
    batch: &RecordBatch,
    offset: usize,
    ids: &SidecarIds,
    stats: &mut BTreeMap<String, GeometryStats>,
    has_coordinates: &mut bool,
    type_cache: &mut HashMap<String, Option<(&'static str, String)>>,
    hierarchy: &mut HashMap<String, HierarchyRow>,
) {
    let name = file.name.as_str();
    let f = Some(name);
    let id = column(batch, "id");
    let feature_id = column(batch, "feature_id");
    let object_type = column(batch, "object_type");
    let parents = column(batch, "parents").and_then(|a| a.as_list_opt::<i32>());
    let children = column(batch, "children").and_then(|a| a.as_list_opt::<i32>());
    let roles = column(batch, "children_roles").and_then(|a| a.as_list_opt::<i32>());
    let bbox = column(batch, "bbox").and_then(|a| a.as_struct_opt());
    let other = column(batch, "other");
    let implicit = column(batch, "implicit_geometry").and_then(|a| a.as_struct_opt());
    let address = column(batch, "address").and_then(|a| a.as_list_opt::<i32>());
    let lods: Vec<(String, LodArrays)> = stats
        .keys()
        .map(|s| (s.clone(), LodArrays::of(batch, s)))
        .collect();

    for i in 0..batch.num_rows() {
        let row = offset + i;
        // spec 02 "Reserved columns": `id`, `feature_id` and `object_type`
        // are required and non-null on every row.
        let mut required = |col: Option<&ArrayRef>, n: &str| {
            let value = col.and_then(|c| string_at(c, i));
            if col.is_some_and(|c| c.is_null(i)) {
                r.error(
                    "value.required-null",
                    OBJECT_TABLE,
                    f,
                    format!("row {row}: `{n}` is null"),
                );
            }
            value
        };
        let id_value = required(id, "id");
        let feature_value = required(feature_id, "feature_id");
        let type_value = required(object_type, "object_type");

        if let Some(t) = type_value {
            let verdict = type_cache
                .entry(t.clone())
                .or_insert_with(|| object_type_verdict(file, &t));
            if let Some((code, problem)) = verdict {
                r.error(code, PACKAGE, f, format!("object_type `{t}` {problem}"));
            }
        }

        if let Some(id_value) = id_value {
            // spec 02 "feature_id rule": the chain to a root follows the
            // FIRST entry of `parents`.
            let first_parent = parents.filter(|p| !p.is_null(i)).and_then(|p| {
                let values = p.value(i);
                let values = values.as_string_opt::<i32>()?;
                (!values.is_empty() && !values.is_null(0)).then(|| values.value(0).to_string())
            });
            hierarchy.insert(
                id_value,
                HierarchyRow {
                    first_parent,
                    feature_id: feature_value,
                    file: name.to_string(),
                },
            );
        }

        // spec 02: `children_roles`, when present, has exactly one entry per
        // child.
        if let Some(roles) = roles.filter(|r| !r.is_null(i)) {
            let n_children = children.map_or(0, |c| list_len(c, i));
            if list_len(roles, i) != n_children {
                r.error(
                    "value.children-roles",
                    OBJECT_TABLE,
                    f,
                    format!(
                        "row {row}: `children_roles` has {} entries for {n_children} children",
                        list_len(roles, i)
                    ),
                );
            }
        }

        // spec 02 "The `other` column": a cell MUST hold a JSON object.
        if let Some(text) = other.and_then(|o| string_at(o, i))
            && !matches!(serde_json::from_str::<Value>(&text), Ok(Value::Object(_)))
        {
            r.error(
                "value.other-not-object",
                OBJECT_TABLE,
                f,
                format!("row {row}: `other` does not hold a JSON object"),
            );
        }

        // spec 02 / 03: the six `bbox` fields are non-null in a non-null box.
        if let Some(bbox) = bbox.filter(|b| !b.is_null(i)) {
            *has_coordinates = true;
            if bbox.columns().iter().any(|c| c.is_null(i)) {
                r.error(
                    "value.bbox",
                    GEOMETRY,
                    f,
                    format!("row {row}: a `bbox` field is null"),
                );
            }
        }

        if let Some(implicit) = implicit.filter(|g| !g.is_null(i)) {
            check_implicit_reference(r, name, row, implicit, i, ids, has_coordinates);
        }

        // spec 02 "Addresses": `location` is a WKB MultiPointZ.
        if let Some(address) = address.filter(|a| !a.is_null(i)) {
            let items = address.value(i);
            if let Some(location) = items
                .as_struct_opt()
                .and_then(|s| s.column_by_name("location"))
                .and_then(|l| l.as_binary_opt::<i32>())
            {
                for bytes in location.iter().flatten() {
                    *has_coordinates = true;
                    if !matches!(parse_wkb(bytes), Ok(s) if s.top() == MULTIPOINT_Z) {
                        r.error(
                            "value.address-location",
                            OBJECT_TABLE,
                            f,
                            format!("row {row}: an address `location` is not a WKB MultiPointZ"),
                        );
                    }
                }
            }
        }

        for (suffix, arrays) in &lods {
            let cx = CellContext {
                file: name,
                suffix,
                row,
            };
            let stat = stats.get_mut(suffix).expect("one stats entry per LoD");
            let before = stat.non_null;
            check_geometry_cell(r, &cx, arrays, i, ids, stat);
            *has_coordinates |= stat.non_null > before;
        }
    }
}

/// `None` when `object_type` belongs in `file`, else the violation code and
/// what is wrong with it.
pub(super) fn object_type_verdict(
    file: &PackageFile,
    object_type: &str,
) -> Option<(&'static str, String)> {
    // spec 01 "By-module object-table layout": a feature's file is its
    // module; `object_type` is the CityGML 3.0 class name.
    if let Some(class) = TAXONOMY.iter().find(|c| c.citygml_class == object_type) {
        let expected = format!("{}.parquet", module_file(&ModuleKey::Core(class.module)));
        return (expected != file.name).then(|| {
            (
                "package.object-type-module",
                format!("belongs to a CityGML module stored in {expected}"),
            )
        });
    }
    // spec 06 "Extended feature types": an extension class is its class
    // name prefixed with a namespace the file declares.
    let declared = object_type
        .split_once('_')
        .is_some_and(|(ns, class)| file.declared_namespaces().contains(ns) && !class.is_empty());
    (!declared).then(|| {
        (
            "package.unknown-object-type",
            "is neither a CityGML 3.0 feature class nor prefixed with a namespace this file declares"
                .to_string(),
        )
    })
}

pub(super) fn check_implicit_reference(
    r: &mut Reporter,
    file: &str,
    row: usize,
    implicit: &StructArray,
    i: usize,
    ids: &SidecarIds,
    has_coordinates: &mut bool,
) {
    let f = Some(file);
    let id = implicit
        .column_by_name("id")
        .and_then(|c| c.as_primitive_opt::<Int64Type>())
        .filter(|c| !c.is_null(i))
        .map(|c| c.value(i));
    let point = implicit
        .column_by_name("point")
        .and_then(|c| c.as_binary_opt::<i32>())
        .filter(|c| !c.is_null(i))
        .map(|c| c.value(i));
    // spec 04 "implicit_geometries.parquet": a non-null value MUST carry
    // both `id` and `point`; `point` is a WKB PointZ; the matrix is exactly
    // 16 values when non-null.
    if id.is_none() || point.is_none() {
        r.error(
            "value.implicit-geometry",
            APPEARANCE,
            f,
            format!("row {row}: `implicit_geometry` lacks its `id` or `point`"),
        );
    }
    if let Some(point) = point {
        *has_coordinates = true;
        if read_point(point).is_err() {
            r.error(
                "value.implicit-geometry",
                APPEARANCE,
                f,
                format!("row {row}: `implicit_geometry.point` is not a WKB PointZ"),
            );
        }
    }
    if let Some(id) = id
        && !ids.implicit_geometries.contains(&id)
    {
        r.error(
            "value.implicit-geometry",
            APPEARANCE,
            f,
            format!(
                "row {row}: `implicit_geometry.id` {id} is absent from implicit_geometries.parquet"
            ),
        );
    }
    if let Some(matrix) = implicit
        .column_by_name("transformationMatrix")
        .and_then(|c| c.as_list_opt::<i32>())
        .filter(|c| !c.is_null(i))
    {
        let values = matrix.value(i);
        if values.len() != 16 || values.null_count() != 0 {
            r.error(
                "value.implicit-geometry",
                APPEARANCE,
                f,
                format!(
                    "row {row}: `implicit_geometry.transformationMatrix` has {} values, not 16",
                    values.len()
                ),
            );
        }
    }
}

/// spec 02 "feature_id rule": a root's `feature_id` is its `id`; a child's is
/// the `id` of the root reached through the first entry of `parents`.
pub(super) fn check_feature_ids(hierarchy: &HashMap<String, HierarchyRow>, r: &mut Reporter) {
    let mut ids: Vec<&String> = hierarchy.keys().collect();
    ids.sort();
    for id in ids {
        let row = &hierarchy[id];
        let Some(feature_id) = &row.feature_id else {
            continue;
        };
        let mut root = id;
        let mut seen = HashSet::new();
        let mut resolved = true;
        while let Some(parent) = hierarchy.get(root).and_then(|h| h.first_parent.as_ref()) {
            if !hierarchy.contains_key(parent) || !seen.insert(parent) {
                resolved = false;
                break;
            }
            root = parent;
        }
        if resolved && feature_id != root {
            r.error(
                "value.feature-id",
                OBJECT_TABLE,
                Some(&row.file),
                format!("`{id}` has feature_id `{feature_id}`, but its root (by first parent) is `{root}`"),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Sidecars (spec 04)
// ---------------------------------------------------------------------------

pub(super) fn check_sidecar(file: &PackageFile, ids: &mut SidecarIds, r: &mut Reporter) {
    let name = file.name.as_str();
    let f = Some(name);
    let fields = file.root_fields();
    let field_named = |n: &str| fields.iter().find(|t| t.name() == n);

    // spec 05 "Requirements depend on the file's role": a sidecar carries
    // none of `source_format`, `attributes`, `primary_column` or `columns`;
    // `implicit_geometries.parquet` carries no `city.crs`.
    if let Some(city) = &file.city {
        for key in ["source_format", "attributes", "primary_column", "columns"] {
            if city.raw.contains_key(key) {
                r.warn(
                    "footer.sidecar-fields",
                    METADATA,
                    f,
                    format!("a sidecar's `city` carries `{key}`"),
                );
            }
        }
        if file.kind == FileKind::ImplicitGeometries && city.crs != CrsKey::Absent {
            r.warn(
                "footer.sidecar-crs",
                APPEARANCE,
                f,
                "implicit_geometries.parquet carries a `city.crs` key; its relative geometries \
                 are in local coordinates"
                    .to_string(),
            );
        }
    }

    let expected: Vec<Field> = match file.kind {
        FileKind::Materials => materials_schema()
            .fields()
            .iter()
            .map(|f| f.as_ref().clone())
            .collect(),
        FileKind::Textures => textures_schema()
            .fields()
            .iter()
            .map(|f| f.as_ref().clone())
            .collect(),
        _ => vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ],
    };
    for field in &expected {
        match field_named(field.name()) {
            Some(actual) => check_type(r, name, APPEARANCE, field.name(), field, actual),
            None => r.error(
                "column.missing",
                APPEARANCE,
                f,
                format!("the column `{}` is missing", field.name()),
            ),
        }
    }
    let mut lods = BTreeMap::new();
    if file.kind == FileKind::ImplicitGeometries {
        lods = check_lod_columns(r, name, fields, &BTreeSet::new());
        // spec 05: annotated if and only if declared in `geo`, and this
        // sidecar declares none.
        for (suffix, annotation) in &lods {
            if annotation.is_some() {
                r.error(
                    "geo.annotation",
                    METADATA,
                    f,
                    format!("`geometry_{suffix}` carries the GEOMETRY logical type but no `geo` declares it"),
                );
            }
        }
    }
    for field in fields {
        let known = expected.iter().any(|e| e.name() == field.name())
            || (file.kind == FileKind::ImplicitGeometries
                && (split_lod_column(field.name()).is_some()
                    || LOD_PREFIXES.contains(&field.name())));
        if !known {
            r.warn(
                "column.unknown",
                APPEARANCE,
                f,
                format!("`{}` is not a column of this sidecar", field.name()),
            );
        }
    }

    let batches = read_batches(file, r);
    let mut seen = HashSet::new();
    let mut offset = 0;
    let mut stats: BTreeMap<String, GeometryStats> = lods
        .keys()
        .map(|s| (s.clone(), GeometryStats::default()))
        .collect();
    for batch in &batches {
        let id = column(batch, "id").and_then(|c| c.as_primitive_opt::<Int64Type>());
        for i in 0..batch.num_rows() {
            let row = offset + i;
            // spec 04: `id` is required, and unique across rows.
            match id.filter(|c| !c.is_null(i)).map(|c| c.value(i)) {
                None if id.is_some() => r.error(
                    "value.required-null",
                    APPEARANCE,
                    f,
                    format!("row {row}: `id` is null"),
                ),
                Some(v) if !seen.insert(v) => r.error(
                    "value.duplicate-id",
                    APPEARANCE,
                    f,
                    format!("row {row}: `id` {v} is not unique"),
                ),
                _ => {}
            }
            match file.kind {
                FileKind::Materials => check_material_row(r, name, batch, i, row),
                FileKind::Textures => check_texture_row(r, name, batch, i, row),
                _ => {}
            }
        }
        offset += batch.num_rows();
    }
    match file.kind {
        FileKind::Materials => ids.materials = seen,
        FileKind::Textures => ids.textures = seen,
        _ => ids.implicit_geometries = seen,
    }

    // The relative geometries, once every sidecar id is known.
    if file.kind == FileKind::ImplicitGeometries {
        let mut offset = 0;
        for batch in &batches {
            let arrays: Vec<(String, LodArrays)> = stats
                .keys()
                .map(|s| (s.clone(), LodArrays::of(batch, s)))
                .collect();
            for i in 0..batch.num_rows() {
                let row = offset + i;
                // spec 04 "implicit_geometries.parquet": a relative geometry
                // is a single geometry at a single LoD.
                let populated = arrays
                    .iter()
                    .filter(|(_, a)| a.geometry.is_some_and(|g| !g.is_null(i)))
                    .count();
                if populated != 1 {
                    r.error(
                        "value.implicit-row",
                        APPEARANCE,
                        f,
                        format!("row {row}: {populated} LoDs populated; a relative geometry has exactly one"),
                    );
                }
                for (suffix, a) in &arrays {
                    let cx = CellContext {
                        file: name,
                        suffix,
                        row,
                    };
                    let stat = stats.get_mut(suffix).expect("one stats entry per LoD");
                    check_geometry_cell(r, &cx, a, i, ids, stat);
                }
            }
            offset += batch.num_rows();
        }
    }
}

/// `count` non-null values in `[0, 1]` (spec 04: colour columns).
pub(super) fn is_colour(list: &ListArray, i: usize, count: usize) -> bool {
    let values = list.value(i);
    let Some(values) = values.as_primitive_opt::<Float64Type>() else {
        return false;
    };
    values.len() == count
        && values.null_count() == 0
        && values.iter().flatten().all(|v| (0.0..=1.0).contains(&v))
}

pub(super) fn check_material_row(
    r: &mut Reporter,
    file: &str,
    batch: &RecordBatch,
    i: usize,
    row: usize,
) {
    // spec 04 "materials.parquet": a colour column MUST hold exactly three
    // values, each in [0, 1], when non-null.
    for name in ["diffuseColor", "specularColor", "emissiveColor"] {
        if let Some(list) = column(batch, name).and_then(|c| c.as_list_opt::<i32>())
            && !list.is_null(i)
            && !is_colour(list, i, 3)
        {
            r.error(
                "value.colour",
                APPEARANCE,
                Some(file),
                format!("row {row}: `{name}` is not three values in [0, 1]"),
            );
        }
    }
    check_sidecar_other(r, file, batch, i, row);
}

pub(super) fn check_texture_row(
    r: &mut Reporter,
    file: &str,
    batch: &RecordBatch,
    i: usize,
    row: usize,
) {
    let f = Some(file);
    // spec 04 "textures.parquet": a row MUST carry `image_uri` or
    // `image_data`.
    let has = |n: &str| column(batch, n).is_some_and(|c| !c.is_null(i));
    if !has("image_uri") && !has("image_data") {
        r.error(
            "value.texture-image",
            APPEARANCE,
            f,
            format!("row {row}: neither `image_uri` nor `image_data` is set"),
        );
    }
    // `wrapMode` and `textureType` are enumerations: a value outside the
    // listed set is invalid.
    for (name, allowed) in [
        (
            "wrapMode",
            &["none", "wrap", "mirror", "clamp", "border"][..],
        ),
        ("textureType", &["unknown", "specific", "typical"][..]),
    ] {
        if let Some(value) = column(batch, name).and_then(|c| string_at(c, i))
            && !allowed.contains(&value.as_str())
        {
            r.error(
                "value.enumeration",
                APPEARANCE,
                f,
                format!("row {row}: `{name}` is \"{value}\", not one of {allowed:?}"),
            );
        }
    }
    // `borderColor` MUST hold exactly four values in [0, 1] when non-null.
    if let Some(list) = column(batch, "borderColor").and_then(|c| c.as_list_opt::<i32>())
        && !list.is_null(i)
        && !is_colour(list, i, 4)
    {
        r.error(
            "value.colour",
            APPEARANCE,
            f,
            format!("row {row}: `borderColor` is not four values in [0, 1]"),
        );
    }
    check_sidecar_other(r, file, batch, i, row);
}

/// The sidecars' `other` holds the unmapped members, as JSON.
pub(super) fn check_sidecar_other(
    r: &mut Reporter,
    file: &str,
    batch: &RecordBatch,
    i: usize,
    row: usize,
) {
    if let Some(text) = column(batch, "other").and_then(|c| string_at(c, i))
        && serde_json::from_str::<Value>(&text).is_err()
    {
        r.error(
            "value.other-not-json",
            APPEARANCE,
            Some(file),
            format!("row {row}: `other` is not valid JSON"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parquet::basic::Repetition;

    fn attribute(physical: PhysicalType, converted: ConvertedType) -> Type {
        Type::primitive_type_builder("attr", physical)
            .with_repetition(Repetition::OPTIONAL)
            .with_converted_type(converted)
            .build()
            .unwrap()
    }

    /// A writer may annotate a temporal column with the legacy converted
    /// type alone (DuckDB writes `DATE` that way); it is the same logical
    /// type, and a legacy `TIMESTAMP_*` is UTC-adjusted by definition.
    #[test]
    fn legacy_converted_temporal_attributes_are_the_spec_types() {
        for (physical, converted) in [
            (PhysicalType::INT32, ConvertedType::DATE),
            (PhysicalType::INT64, ConvertedType::TIMESTAMP_MILLIS),
            (PhysicalType::INT64, ConvertedType::TIMESTAMP_MICROS),
            (PhysicalType::INT32, ConvertedType::TIME_MILLIS),
            (PhysicalType::INT64, ConvertedType::TIME_MICROS),
        ] {
            let mut r = Reporter::default();
            check_attribute_type(&mut r, "f", "attr", &attribute(physical, converted));
            let report = r.finish();
            assert!(report.violations.is_empty(), "{converted}: {report}");
        }
    }
}
