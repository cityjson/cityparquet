//! Decoding values, and the per-cell invariants of geometry, geometry
//! properties and appearance (spec 03 "Invariants", spec 04 "Invariants").

use super::*;

// ---------------------------------------------------------------------------
// Decoding values
// ---------------------------------------------------------------------------

/// Hand every record batch of `file` to `f`, with the file row offset of its
/// first row, one batch at a time — decoded from the Parquet schema alone:
/// the `ARROW:schema` entry, if any, is ignored (spec 02: a reader MUST NOT
/// require it).
pub(super) fn for_each_batch(
    file: &PackageFile,
    r: &mut Reporter,
    mut f: impl FnMut(&mut Reporter, &RecordBatch, usize),
) {
    let unreadable = |r: &mut Reporter, e: String| {
        r.error(
            "package.unreadable-parquet",
            PACKAGE,
            Some(&file.name),
            format!("cannot decode the values: {e}"),
        );
    };
    let reader = fs::File::open(&file.path)
        .map_err(|e| e.to_string())
        .and_then(|f| {
            ParquetRecordBatchReaderBuilder::try_new_with_options(
                f,
                ArrowReaderOptions::new().with_skip_arrow_metadata(true),
            )
            .map_err(|e| e.to_string())
        })
        .and_then(|b| b.build().map_err(|e| e.to_string()));
    let reader = match reader {
        Ok(reader) => reader,
        Err(e) => return unreadable(r, e),
    };
    let mut offset = 0;
    for batch in reader {
        match batch {
            Ok(batch) => {
                f(r, &batch, offset);
                offset += batch.num_rows();
            }
            Err(e) => return unreadable(r, e.to_string()),
        }
    }
}

pub(super) fn column<'a>(batch: &'a RecordBatch, name: &str) -> Option<&'a ArrayRef> {
    batch.column_by_name(name)
}

pub(super) fn string_at(array: &ArrayRef, row: usize) -> Option<String> {
    if array.is_null(row) {
        return None;
    }
    array
        .as_string_opt::<i32>()
        .map(|a| a.value(row).to_string())
}

pub(super) fn list_len(list: &ListArray, row: usize) -> usize {
    if list.is_null(row) {
        0
    } else {
        list.value_length(row) as usize
    }
}

// ---------------------------------------------------------------------------
// Per-LoD geometry, properties and appearance (spec 03, spec 04)
// ---------------------------------------------------------------------------

/// What one geometry column's values carried, across the file.
#[derive(Default)]
pub(super) struct GeometryStats {
    pub(super) non_null: usize,
    pub(super) top_level_types: BTreeSet<u32>,
    pub(super) all_codes: BTreeSet<u32>,
}

impl GeometryStats {
    /// Every WKB type code inside GeoParquet's `[1001, 1007]` subset (spec
    /// 05 "The declaration rule"); `None` when the column carries no value.
    pub(super) fn geoparquet_legal(&self) -> Option<bool> {
        (self.non_null > 0).then(|| self.all_codes.iter().all(|c| (1001..=1007).contains(c)))
    }
}

/// The four per-LoD columns of one LoD in one batch.
pub(super) struct LodArrays<'a> {
    pub(super) geometry: Option<&'a BinaryArray>,
    pub(super) properties: Option<&'a StructArray>,
    pub(super) material: Option<&'a MapArray>,
    pub(super) texture: Option<&'a MapArray>,
}

impl<'a> LodArrays<'a> {
    pub(super) fn of(batch: &'a RecordBatch, suffix: &str) -> Self {
        Self {
            geometry: column(batch, &format!("geometry_{suffix}"))
                .and_then(|a| a.as_binary_opt::<i32>()),
            properties: column(batch, &format!("geometry_properties_{suffix}"))
                .and_then(|a| a.as_struct_opt()),
            material: column(batch, &format!("material_{suffix}")).and_then(|a| a.as_map_opt()),
            texture: column(batch, &format!("texture_{suffix}")).and_then(|a| a.as_map_opt()),
        }
    }
}

pub(super) struct CellContext<'a> {
    pub(super) file: &'a str,
    pub(super) suffix: &'a str,
    pub(super) row: usize,
}

pub(super) fn check_geometry_cell(
    r: &mut Reporter,
    cx: &CellContext,
    arrays: &LodArrays,
    i: usize,
    ids: &SidecarIds,
    stats: &mut GeometryStats,
) -> Option<[f64; 6]> {
    let (file, suffix, row) = (Some(cx.file), cx.suffix, cx.row);
    let shape = match arrays.geometry {
        Some(g) if !g.is_null(i) => match parse_wkb(g.value(i)) {
            Ok(shape) => {
                stats.non_null += 1;
                stats.top_level_types.insert(shape.top());
                stats.all_codes.extend(shape.codes.iter().copied());
                Some(shape)
            }
            Err(e) => {
                r.error(
                    "value.wkb",
                    GEOMETRY,
                    file,
                    format!("row {row}: `geometry_{suffix}` is not CityParquet WKB: {e}"),
                );
                return None;
            }
        },
        _ => None,
    };

    // spec 04 "material / texture columns": a material/texture cell is
    // non-null only where the same row's geometry is non-null.
    for (name, cell) in [("material", arrays.material), ("texture", arrays.texture)] {
        if shape.is_none() && cell.is_some_and(|m| !m.is_null(i)) {
            r.error(
                "value.appearance-without-geometry",
                APPEARANCE,
                file,
                format!("row {row}: `{name}_{suffix}` is set where `geometry_{suffix}` is null"),
            );
        }
    }
    let shape = shape?;

    if let Some(props) = arrays.properties.filter(|p| !p.is_null(i)) {
        check_properties(r, cx, props, i, &shape);
    } else if shape.is_solid_family() {
        // spec 03 "Invariants": `shells` is non-null for solid-family
        // geometry, so its properties cannot be absent.
        r.error(
            "value.shells",
            GEOMETRY,
            file,
            format!("row {row}: solid geometry in `geometry_{suffix}` has no `shells`"),
        );
    }

    if let Some(material) = arrays.material.filter(|m| !m.is_null(i)) {
        check_material(r, cx, material, i, &shape, ids);
    }
    if let Some(texture) = arrays.texture.filter(|m| !m.is_null(i)) {
        check_texture(r, cx, texture, i, &shape, ids);
    }
    shape.extent
}

pub(super) fn check_properties(
    r: &mut Reporter,
    cx: &CellContext,
    props: &StructArray,
    i: usize,
    shape: &WkbShape,
) {
    let (file, suffix, row) = (Some(cx.file), cx.suffix, cx.row);
    let col = format!("geometry_properties_{suffix}");

    // spec 03 "Geometry properties and semantics": `type` is non-null and
    // disambiguates the CM type a WKB encoding stands for.
    match props.column_by_name("type").and_then(|t| string_at(t, i)) {
        None => r.error(
            "value.properties-type",
            GEOMETRY,
            file,
            format!("row {row}: `{col}.type` is null"),
        ),
        Some(cm) => match wkb_code_for_cm_type(&cm) {
            Some(code) if code == shape.top() => {}
            Some(code) => r.error(
                "value.properties-type",
                GEOMETRY,
                file,
                format!(
                    "row {row}: `{col}.type` is {cm}, which encodes as WKB type {code}, \
                     but the geometry is type {}",
                    shape.top()
                ),
            ),
            None => r.error(
                "value.properties-type",
                GEOMETRY,
                file,
                format!("row {row}: `{col}.type` is {cm}, not a CityGML CM geometry type"),
            ),
        },
    }

    // `surfaces`: a JSON array of objects, each with a `type`.
    let surfaces = props
        .column_by_name("surfaces")
        .and_then(|s| string_at(s, i));
    let surface_count = match &surfaces {
        None => None,
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Array(items))
                if items
                    .iter()
                    .all(|s| s.get("type").is_some_and(Value::is_string)) =>
            {
                Some(items.len())
            }
            _ => {
                r.error(
                    "value.surfaces",
                    GEOMETRY,
                    file,
                    format!(
                        "row {row}: `{col}.surfaces` is not a JSON array of surface objects with a `type`"
                    ),
                );
                None
            }
        },
    };

    let face_semantics = props
        .column_by_name("face_semantics")
        .and_then(|f| f.as_list_opt::<i32>());
    let fs_null = face_semantics.is_none_or(|f| f.is_null(i));
    // spec 03 "Invariants": `surfaces` and `face_semantics` are null together.
    if surfaces.is_none() != fs_null {
        r.error(
            "value.semantics-null-together",
            GEOMETRY,
            file,
            format!("row {row}: `{col}.surfaces` and `.face_semantics` are not null together"),
        );
    }
    // The value checks read a column only in the type the schema check
    // accepted; a column of another type has already been reported there.
    let fs_values = face_semantics.filter(|f| !f.is_null(i)).map(|f| f.value(i));
    if let Some(values) = fs_values
        .as_ref()
        .and_then(|v| v.as_primitive_opt::<Int32Type>())
    {
        // spec 03 "Invariants": len(face_semantics) MUST equal the WKB face
        // count, and every non-null entry MUST index an existing surface.
        if shape.is_polygonal() && values.len() != shape.faces.len() {
            r.error(
                "value.face-semantics",
                GEOMETRY,
                file,
                format!(
                    "row {row}: `{col}.face_semantics` has {} entries for {} WKB faces",
                    values.len(),
                    shape.faces.len()
                ),
            );
        }
        if let Some(n) = surface_count
            && values
                .iter()
                .flatten()
                .any(|idx| idx < 0 || idx as usize >= n)
        {
            r.error(
                "value.face-semantics",
                GEOMETRY,
                file,
                format!("row {row}: `{col}.face_semantics` indexes past the {n} surfaces"),
            );
        }
    }

    // spec 03 "Invariants": `shells` has one inner list per solid, non-null
    // all the way down, totalling the face count; null for non-solid types.
    let shells = props
        .column_by_name("shells")
        .and_then(|s| s.as_list_opt::<i32>())
        .filter(|s| !s.is_null(i));
    match (shells, shape.is_solid_family()) {
        (None, true) => r.error(
            "value.shells",
            GEOMETRY,
            file,
            format!("row {row}: `{col}.shells` is null for solid geometry"),
        ),
        (Some(_), false) => r.error(
            "value.shells",
            GEOMETRY,
            file,
            format!("row {row}: `{col}.shells` is set for non-solid geometry"),
        ),
        (Some(shells), true) => {
            let per_solid = shells.value(i);
            let Some(per_solid) = per_solid.as_list_opt::<i32>() else {
                return;
            };
            let mut ok = per_solid.len() == shape.solids.len() && per_solid.null_count() == 0;
            if ok {
                for (s, &faces) in shape.solids.iter().enumerate() {
                    let counts = per_solid.value(s);
                    let Some(counts) = counts.as_primitive_opt::<Int32Type>() else {
                        return;
                    };
                    let total: i64 = counts.iter().map(|c| c.unwrap_or(-1) as i64).sum();
                    ok &= counts.null_count() == 0 && total == faces as i64;
                }
            }
            if !ok {
                r.error(
                    "value.shells",
                    GEOMETRY,
                    file,
                    format!(
                        "row {row}: `{col}.shells` does not partition the faces of its {} solid(s)",
                        shape.solids.len()
                    ),
                );
            }
        }
        (None, false) => {}
    }
}

pub(super) fn check_material(
    r: &mut Reporter,
    cx: &CellContext,
    map: &MapArray,
    i: usize,
    shape: &WkbShape,
    ids: &SidecarIds,
) {
    let (file, row) = (Some(cx.file), cx.row);
    let col = format!("material_{}", cx.suffix);
    let offsets = map.value_offsets();
    let (start, end) = (offsets[i] as usize, offsets[i + 1] as usize);
    // spec 04 "Invariants": a map MUST NOT be empty and its values MUST be
    // non-null; each theme's list has one entry per WKB face, and every
    // non-null id MUST match a `materials.parquet` id.
    if start == end {
        r.error(
            "value.material",
            APPEARANCE,
            file,
            format!("row {row}: `{col}` is an empty map"),
        );
        return;
    }
    let Some(values) = map.values().as_list_opt::<i32>() else {
        return;
    };
    for e in start..end {
        if values.is_null(e) {
            r.error(
                "value.material",
                APPEARANCE,
                file,
                format!("row {row}: `{col}` has a null theme value"),
            );
            continue;
        }
        let per_face = values.value(e);
        let Some(per_face) = per_face.as_primitive_opt::<Int64Type>() else {
            return;
        };
        if shape.is_polygonal() && per_face.len() != shape.faces.len() {
            r.error(
                "value.material",
                APPEARANCE,
                file,
                format!(
                    "row {row}: a `{col}` theme has {} entries for {} WKB faces",
                    per_face.len(),
                    shape.faces.len()
                ),
            );
        }
        if let Some(id) = per_face
            .iter()
            .flatten()
            .find(|id| !ids.materials.contains(id))
        {
            r.error(
                "value.material",
                APPEARANCE,
                file,
                format!(
                    "row {row}: `{col}` references material id {id}, absent from materials.parquet"
                ),
            );
        }
    }
}

pub(super) fn check_texture(
    r: &mut Reporter,
    cx: &CellContext,
    map: &MapArray,
    i: usize,
    shape: &WkbShape,
    ids: &SidecarIds,
) {
    let (file, row) = (Some(cx.file), cx.row);
    let col = format!("texture_{}", cx.suffix);
    let fail = |r: &mut Reporter, what: String| {
        r.error(
            "value.texture",
            APPEARANCE,
            file,
            format!("row {row}: `{col}` {what}"),
        );
    };
    let offsets = map.value_offsets();
    let (start, end) = (offsets[i] as usize, offsets[i + 1] as usize);
    if start == end {
        fail(r, "is an empty map".to_string());
        return;
    }
    let Some(themes) = map.values().as_list_opt::<i32>() else {
        return;
    };
    // spec 04 "Invariants": per theme, one entry per WKB face, one ring
    // struct per ring of that face, `id`/`uv` null together, `uv` one
    // `[u, v]` pair per distinct ring vertex (the closing repeat has none),
    // and every non-null id MUST match a `textures.parquet` id.
    for e in start..end {
        if themes.is_null(e) {
            fail(r, "has a null theme value".to_string());
            continue;
        }
        let faces = themes.value(e);
        let Some(faces) = faces.as_list_opt::<i32>() else {
            return;
        };
        if shape.is_polygonal() && faces.len() != shape.faces.len() {
            fail(
                r,
                format!(
                    "has a theme of {} faces for {} WKB faces",
                    faces.len(),
                    shape.faces.len()
                ),
            );
            continue;
        }
        for (f, rings_of_face) in shape.faces.iter().enumerate().take(faces.len()) {
            if faces.is_null(f) {
                fail(r, format!("has a null entry for face {f}"));
                continue;
            }
            let rings = faces.value(f);
            let Some(rings) = rings.as_struct_opt() else {
                return;
            };
            if rings.len() != rings_of_face.len() {
                fail(
                    r,
                    format!(
                        "has {} ring entries for face {f}'s {} rings",
                        rings.len(),
                        rings_of_face.len()
                    ),
                );
                continue;
            }
            let (Some(tex_id), Some(uv)) = (
                rings
                    .column_by_name("id")
                    .and_then(|c| c.as_primitive_opt::<Int64Type>()),
                rings
                    .column_by_name("uv")
                    .and_then(|c| c.as_list_opt::<i32>()),
            ) else {
                return;
            };
            for (k, &points) in rings_of_face.iter().enumerate() {
                if rings.is_null(k) || tex_id.is_null(k) != uv.is_null(k) {
                    fail(
                        r,
                        format!(
                            "face {f} ring {k}: the ring is null, or `id` and `uv` are not null together"
                        ),
                    );
                    continue;
                }
                if tex_id.is_null(k) {
                    continue;
                }
                if !ids.textures.contains(&tex_id.value(k)) {
                    fail(
                        r,
                        format!(
                            "references texture id {}, absent from textures.parquet",
                            tex_id.value(k)
                        ),
                    );
                }
                let pairs = uv.value(k);
                let Some(pairs) = pairs.as_list_opt::<i32>() else {
                    return;
                };
                let well_formed = pairs.null_count() == 0
                    && (0..pairs.len()).all(|p| {
                        let pair = pairs.value(p);
                        pair.len() == 2 && pair.null_count() == 0
                    });
                if pairs.len() + 1 != points || !well_formed {
                    fail(
                        r,
                        format!(
                            "face {f} ring {k}: `uv` must hold {} non-null [u, v] pairs",
                            points.saturating_sub(1)
                        ),
                    );
                }
            }
        }
    }
}
