//! The database's top-level objects, streamed a batch of features at a
//! time.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use citygml_core::gml::{ObjectBuilder, XlinkBudget};
use citygml_core::model::{AdeIndex, GroupIndex, ModelAppearance, Module, QName, Template};
use citygml_core::reader::templates::Templates;
use citygml_core::reader::{Document, ObjectStream};
use citygml_core::{GmlObject, Result};

use crate::CityDb;
use crate::appearance::appearance;
use crate::assemble::{Assembler, Tally};
use crate::db::Session;
use crate::geometry::{self, Built};
use crate::rows::{self, load_mappings, textured};

/// The features the reader starts from, as an SQL condition on `feature f`:
/// current (not terminated) and contained by no other feature. A relief
/// feature among them is passed through to its components, and one of a
/// class with no CityJSON type is counted and left out.
pub(crate) const ROOTS: &str = "f.termination_date IS NULL AND NOT EXISTS (\
     SELECT 1 FROM {s}.property c WHERE c.val_feature_id = f.id AND c.val_relation_type = 1)";

/// What every pass over the database shares: the [`Document`] (the
/// templates the database's implicit geometries place, and which groups
/// reference which members) and where each template is.
pub(crate) struct Prepared {
    pub(crate) document: Arc<Document>,
    /// `implicit_geometry.id` -> the template's position.
    pub(crate) templates: HashMap<i64, usize>,
    /// What the templates leave out, by name.
    pub(crate) dropped: BTreeMap<String, usize>,
}

pub(crate) fn prepare(session: &mut Session) -> Result<Prepared> {
    let mut groups = GroupIndex::default();
    for row in session.query(
        "SELECT g.objectid, m.objectid \
         FROM {s}.property r \
         JOIN {s}.property p ON p.parent_id = r.id \
         JOIN {s}.feature g ON g.id = r.feature_id \
         JOIN {s}.feature m ON m.id = p.val_feature_id \
         JOIN {s}.datatype dt ON dt.id = r.datatype_id \
         WHERE dt.typename = 'Role' AND p.name = 'groupMember' \
           AND p.val_relation_type = 0 AND g.termination_date IS NULL \
         ORDER BY r.id",
        &[],
    )? {
        let group: Option<String> = row.get(0);
        let member: Option<String> = row.get(1);
        if let (Some(group), Some(member)) = (group, member) {
            groups.add(&group, &member);
        }
    }

    let mut dropped = BTreeMap::new();
    let mut rows = Vec::new();
    for row in session.query(
        "SELECT ig.id, gd.id, ST_AsBinary(gd.implicit_geometry), gd.geometry_properties::text, \
                (SELECT p.val_lod FROM {s}.property p \
                 WHERE p.val_implicitgeom_id = ig.id AND p.val_lod IS NOT NULL \
                 ORDER BY p.id LIMIT 1) \
         FROM {s}.implicit_geometry ig \
         LEFT JOIN {s}.geometry_data gd ON gd.id = ig.relative_geometry_id \
         ORDER BY ig.id",
        &[],
    )? {
        let id: i64 = row.get(0);
        let geometry: Option<i64> = row.get(1);
        let wkb: Option<Vec<u8>> = row.get(2);
        let properties: Option<String> = row.get(3);
        let lod: Option<String> = row.get(4);
        match (geometry, wkb, properties, lod) {
            (Some(g), Some(wkb), Some(properties), Some(lod)) => {
                rows.push((id, g, wkb, properties, lod));
            }
            // A library object (an external model file), or a template
            // nothing places.
            _ => {
                *dropped
                    .entry("core:ImplicitGeometry without a relative geometry".to_string())
                    .or_insert(0) += 1
            }
        }
    }
    let geometry_ids: Vec<i64> = rows.iter().map(|r| r.1).collect();
    let mut mappings = HashMap::new();
    let mut surface_data = HashMap::new();
    load_mappings(session, &geometry_ids, &mut mappings, &mut surface_data)?;

    let mut templates = Templates::default();
    let mut positions = HashMap::new();
    for (id, g, wkb, properties, lod) in rows {
        let properties: serde_json::Value = serde_json::from_str(&properties).map_err(|e| {
            citygml_core::Error::invalid(format!("geometry {g}: unreadable JSON: {e}"))
        })?;
        let own = mappings.get(&g).map_or(&[][..], Vec::as_slice);
        let built = geometry::build(&wkb, &properties, &textured(own))?;
        let template_id = properties
            .get("objectId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let mut b = ObjectBuilder::new(
            template_id.clone(),
            QName::module(Module::Core, "ImplicitGeometry"),
        );
        match built {
            Built::Solid(solid) => b.add_solid(lod, solid),
            Built::Surfaces(surfaces) => b.add_surfaces(lod, surfaces.into_faces()),
            Built::Lines(lines) => b.add_lines(lod, lines),
            Built::Points(points) => b.add_points(lod, points),
            Built::Unsupported(kind) => {
                *dropped.entry(format!("gml:{kind}")).or_insert(0) += 1;
                continue;
            }
        }
        let Some(geometry) = b.finish(&XlinkBudget::default())?.geometries.pop() else {
            continue;
        };
        positions.insert(id, templates.templates.len());
        templates.templates.push(Template {
            id: template_id,
            geometry,
            appearance: appearance([own], &surface_data, &mut dropped),
        });
    }
    let document = Document::new(
        ModelAppearance::default(),
        groups,
        templates,
        AdeIndex::default(),
    );
    Ok(Prepared {
        document: Arc::new(document),
        templates: positions,
        dropped,
    })
}

/// The top-level objects of a database, in feature id order: what
/// [`CityDb::objects`] returns.
pub struct Objects<'a> {
    db: &'a CityDb,
    prepared: Arc<Prepared>,
    /// The id of the last top-level feature read.
    after: i64,
    exhausted: bool,
    queue: VecDeque<GmlObject>,
    budget: XlinkBudget,
    emitted: usize,
    tally: Tally,
    ade_geometries: BTreeMap<String, usize>,
}

impl<'a> Objects<'a> {
    pub(crate) fn new(db: &'a CityDb, prepared: Arc<Prepared>) -> Self {
        let tally = Tally {
            dropped: prepared.dropped.clone(),
            ..Tally::default()
        };
        Self {
            db,
            prepared,
            after: i64::MIN,
            exhausted: false,
            queue: VecDeque::new(),
            budget: XlinkBudget::default(),
            emitted: 0,
            tally,
            ade_geometries: BTreeMap::new(),
        }
    }

    /// Properties, appearances and geometries the model has no place for,
    /// by name (`core:lod2TerrainIntersectionCurve`,
    /// `app:GeoreferencedTexture`). Complete once the stream is drained.
    pub fn dropped(&self) -> &BTreeMap<String, usize> {
        &self.tally.dropped
    }

    /// Everything the reader has left out so far, by name: what
    /// [`Self::dropped`] counts and the members of classes it does not read.
    /// Complete once the stream is drained.
    pub fn left_out(&self) -> BTreeMap<String, usize> {
        let mut left_out = self.tally.dropped.clone();
        for (name, n) in &self.tally.skipped_members {
            *left_out.entry(name.clone()).or_insert(0) += n;
        }
        left_out
    }

    fn next_batch(&mut self) -> Result<()> {
        let limit = self.db.batch_features as i64;
        let sql = format!(
            "SELECT f.id FROM {{s}}.feature f WHERE {ROOTS} AND f.id > $1 ORDER BY f.id LIMIT $2"
        );
        let batch = {
            let mut session = self.db.session()?;
            let roots: Vec<i64> = session
                .query(&sql, &[&self.after, &limit])?
                .iter()
                .map(|row| row.get(0))
                .collect();
            if (roots.len() as i64) < limit {
                self.exhausted = true;
            }
            let Some(&last) = roots.last() else {
                return Ok(());
            };
            self.after = last;
            (rows::load(&mut session, &roots)?, roots)
        };
        let (rows, roots) = batch;
        let mut assembler = Assembler {
            batch: &rows,
            catalogue: &self.db.catalogue,
            templates: &self.prepared.templates,
            tally: &mut self.tally,
            seen: Default::default(),
        };
        let mut builders = Vec::new();
        for root in roots {
            builders.extend(assembler.top_level(root)?);
        }
        for builder in builders {
            let mut object = builder.finish(&self.budget)?;
            self.prepared
                .document
                .complete(&mut object, &mut self.ade_geometries);
            self.queue.push_back(object);
        }
        Ok(())
    }
}

impl Iterator for Objects<'_> {
    type Item = Result<GmlObject>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.queue.is_empty() && !self.exhausted {
            if let Err(e) = self.next_batch() {
                self.exhausted = true;
                return Some(Err(e));
            }
        }
        let object = self.queue.pop_front()?;
        self.emitted += 1;
        Some(Ok(object))
    }
}

impl ObjectStream for Objects<'_> {
    fn document(&self) -> &Arc<Document> {
        &self.prepared.document
    }

    fn emitted_members(&self) -> usize {
        self.emitted
    }

    fn skipped_members(&self) -> &BTreeMap<String, usize> {
        &self.tally.skipped_members
    }

    fn ade_members(&self) -> &BTreeMap<String, usize> {
        &self.tally.ade_members
    }

    fn ade_geometries(&self) -> &BTreeMap<String, usize> {
        &self.ade_geometries
    }
}
