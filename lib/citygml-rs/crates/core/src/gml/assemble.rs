//! Assemble one city object's geometry from what a reader collected, with
//! every `xlink:href` resolved, into a [`GmlObject`].
//!
//! A reader walks an object's subtree once and hands the builder what it
//! finds: solids whose surface members may `xlink` forward, identified
//! polygons and surface aggregates (the xlink targets), the boundary surfaces
//! with the polygons each holds, and standalone `lodN` surfaces. Only
//! [`ObjectBuilder::finish`], once the whole subtree has been read, resolves
//! the references and decides which geometry each LoD gets:
//!
//! - a `lodNSolid` is that LoD's geometry, its faces tagged with the boundary
//!   surface that holds (or xlinks) the same polygon;
//! - a LoD with boundary-surface geometry and no solid gets one MultiSurface
//!   of those faces, plus any standalone face at that LoD none of them holds;
//! - a standalone `lodNMultiSurface` at a LoD with no boundary geometry is a
//!   MultiSurface of its own, its faces untagged; at a LoD with a solid it is
//!   only a set of xlink targets, since the solid is that LoD's geometry.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::read::{AggregateMember, Collected, RawSolid, RefTarget, SolidGeom, SurfaceRef};
use crate::model::{
    Address, Appearance, Attribute, BoundarySurface, Face, Geometry, GmlObject, Line, Polygon,
    QName, Shape,
};
use crate::{Error, Result};

/// How deep a chain of surface aggregates xlinking (or nesting) into one
/// another may run before expansion refuses it. GML has no reason to nest
/// aggregates more than a few levels.
const MAX_XLINK_DEPTH: usize = 64;

/// One document's allowance for expanding xlinks to surface aggregates: a
/// fixed number of surface references per polygon the document has defined
/// so far, granted as each object is finished and spent by every reference
/// any expansion produces, whichever object it is in.
///
/// A polygon legitimately appears a few times over (shared by two solids of a
/// CompositeSolid, or as a boundary surface and a solid face), so the
/// allowance only bites on aggregates that xlink to each other many times
/// over, which would otherwise expand exponentially in the document's size.
#[derive(Debug, Default)]
pub struct XlinkBudget {
    allowed: AtomicUsize,
    spent: AtomicUsize,
}

impl XlinkBudget {
    const REFERENCES_PER_POLYGON: usize = 4;

    fn grant(&self, polygons: usize) {
        self.allowed
            .fetch_add(Self::REFERENCES_PER_POLYGON * polygons, Ordering::Relaxed);
    }

    /// Spend one reference; `false` once the allowance is exhausted.
    fn charge(&self) -> bool {
        self.spent.fetch_add(1, Ordering::Relaxed) < self.allowed.load(Ordering::Relaxed)
    }
}

/// Everything a reader collects for one object before resolution.
#[derive(Debug)]
pub struct ObjectBuilder {
    id: Option<String>,
    class: QName,
    /// Standalone `lodN{MultiSurface,Geometry}` surfaces, one list per LoD.
    plain_surfaces: Vec<(String, Vec<Polygon>)>,
    /// Geometry that references nothing, so needs no resolution: points,
    /// lines and TINs.
    direct: Vec<Geometry>,
    /// One solid per distinct LoD.
    solids: Vec<(String, SolidGeom)>,
    /// Every identified polygon, by `gml:id`: the xlink targets.
    polygons: HashMap<String, Polygon>,
    /// Every identified surface aggregate (`gml:CompositeSurface`,
    /// `gml:MultiSurface`) by `gml:id`: an xlink to it stands for all of its
    /// polygons.
    aggregates: HashMap<String, Vec<AggregateMember>>,
    /// The polygons [`AggregateMember::Polygon`] members index.
    aggregate_polygons: Vec<Polygon>,
    /// Polygons harvested from this object's own subtree, each counted once:
    /// what [`XlinkBudget::grant`] is sized by.
    parsed_polygons: usize,
    surfaces: Vec<BoundarySurface>,
    /// `gml:id` of a boundary polygon -> its index into `surfaces`.
    semantic_of_polygon: HashMap<String, usize>,
    /// Every boundary-surface polygon in document order, with its surface
    /// index and LoD.
    boundary_polys: Vec<(usize, String, Polygon)>,
    /// Boundary-surface members that are `xlink:href`s rather than inline
    /// polygons: `(surface index, LoD, target id)`.
    boundary_refs: Vec<(usize, String, String)>,
    attributes: Vec<Attribute>,
    addresses: Vec<Address>,
    children: Vec<ObjectBuilder>,
    appearance: Appearance,
}

/// An enclosing object's xlink targets, lent to the objects it holds.
#[derive(Clone, Copy)]
struct Targets<'a> {
    polygons: &'a HashMap<String, Polygon>,
    aggregates: &'a HashMap<String, Vec<AggregateMember>>,
    aggregate_polygons: &'a [Polygon],
}

impl ObjectBuilder {
    pub fn new(id: Option<String>, class: QName) -> Self {
        Self {
            id,
            class,
            plain_surfaces: Vec::new(),
            direct: Vec::new(),
            solids: Vec::new(),
            polygons: HashMap::new(),
            aggregates: HashMap::new(),
            aggregate_polygons: Vec::new(),
            parsed_polygons: 0,
            surfaces: Vec::new(),
            semantic_of_polygon: HashMap::new(),
            boundary_polys: Vec::new(),
            boundary_refs: Vec::new(),
            attributes: Vec::new(),
            addresses: Vec::new(),
            children: Vec::new(),
            appearance: Appearance::default(),
        }
    }

    /// Keep `geom` as the solid at `lod`, unless that LoD already has one.
    pub fn add_solid(&mut self, lod: String, geom: SolidGeom) {
        if !self.solids.iter().any(|(l, _)| *l == lod) {
            self.solids.push((lod, geom));
        }
    }

    /// Whether a standalone surface list has been recorded at `lod`.
    pub fn has_plain_surfaces(&self, lod: &str) -> bool {
        self.plain_surfaces.iter().any(|(l, _)| l == lod)
    }

    /// Record standalone surface geometry at `lod`: its identified polygons
    /// become xlink targets, and a non-empty list a MultiSurface.
    pub fn add_plain_surfaces(&mut self, lod: String, polys: Vec<Polygon>) {
        if polys.is_empty() {
            return;
        }
        self.parsed_polygons += polys.len();
        for poly in &polys {
            if let Some(id) = &poly.id {
                self.polygons.insert(id.clone(), poly.clone());
            }
        }
        self.plain_surfaces.push((lod, polys));
    }

    /// Record a standalone surface collection at `lod` (`lodNMultiSurface`,
    /// `lod0FootPrint`): its identified polygons and aggregates become xlink
    /// targets, and its polygons a MultiSurface unless the LoD has a solid
    /// or boundary-surface geometry (see [`Self::finish`]).
    pub fn add_surfaces(&mut self, lod: String, collected: Collected) {
        let polys: Vec<Polygon> = self
            .absorb(collected)
            .into_iter()
            .map(|(id, poly)| {
                if let Some(id) = id {
                    self.polygons.insert(id, poly.clone());
                }
                poly
            })
            .collect();
        if !polys.is_empty() {
            self.plain_surfaces.push((lod, polys));
        }
    }

    /// Record the points of a `MultiPoint` at `lod`.
    pub fn add_points(&mut self, lod: String, points: Vec<[f64; 3]>) {
        if !points.is_empty() {
            self.add_direct(lod, Shape::MultiPoint(points));
        }
    }

    /// Record the lines of a curve geometry at `lod`.
    pub fn add_lines(&mut self, lod: String, lines: Vec<Line>) {
        if !lines.is_empty() {
            self.add_direct(lod, Shape::MultiLineString(lines));
        }
    }

    /// Record a TIN at `lod`: its triangles as one composite surface.
    pub fn add_tin(&mut self, lod: String, collected: Collected) {
        let faces: Vec<Face> = self
            .absorb(collected)
            .iter()
            .map(|(_, poly)| plain_face(poly, None))
            .collect();
        if !faces.is_empty() {
            self.add_direct(lod, Shape::CompositeSurface(faces));
        }
    }

    /// Keep `shape` as a point, curve or TIN geometry at `lod`, unless that
    /// LoD already has one. [`Self::finish`] keeps it only at a LoD no
    /// surface or solid holds: a CityParquet object has one geometry per LoD.
    fn add_direct(&mut self, lod: String, shape: Shape) {
        if !self.direct.iter().any(|g| g.lod == lod) {
            self.direct.push(Geometry { lod, shape });
        }
    }

    /// Register harvested geometry as xlink targets only: every identified
    /// polygon and every identified surface aggregate.
    pub fn register(&mut self, collected: Collected) {
        for (id, poly) in self.absorb(collected) {
            if let Some(id) = id {
                self.polygons.insert(id, poly);
            }
        }
    }

    /// Start a boundary surface of class `class`, nested in surface `parent`
    /// (a `Door` in a wall's opening) if any; returns its index, for
    /// [`Self::add_boundary_geometry`].
    pub fn push_boundary_surface(&mut self, class: QName, parent: Option<usize>) -> usize {
        self.surfaces.push(BoundarySurface { class, parent });
        self.surfaces.len() - 1
    }

    /// The geometry of boundary surface `surface` at `lod`. Its identified
    /// polygons become xlink targets, and each face of the object carrying
    /// one of its polygon ids (inline or xlinked) is tagged with the surface.
    pub fn add_boundary_geometry(&mut self, surface: usize, lod: &str, collected: Collected) {
        let xlinks = collected.xlinks.clone();
        for (id, poly) in self.absorb(collected) {
            if let Some(id) = &id {
                self.semantic_of_polygon.insert(id.clone(), surface);
                self.polygons.insert(id.clone(), poly.clone());
            }
            self.boundary_polys.push((surface, lod.to_string(), poly));
        }
        for href in xlinks {
            self.semantic_of_polygon.insert(href.clone(), surface);
            self.boundary_refs.push((surface, lod.to_string(), href));
        }
    }

    pub fn push_attribute(&mut self, attribute: Attribute) {
        self.attributes.push(attribute);
    }

    pub fn push_address(&mut self, address: Address) {
        self.addresses.push(address);
    }

    /// Add an appearance property's materials and textures, in order.
    pub fn add_appearance(&mut self, appearance: Appearance) {
        self.appearance.materials.extend(appearance.materials);
        self.appearance.textures.extend(appearance.textures);
    }

    /// Add a part (`consistsOfBuildingPart`): an object of its own, whose
    /// polygons stay its own.
    pub fn add_part(&mut self, part: ObjectBuilder) {
        self.children.push(part);
    }

    /// Add a child feature found in a nesting property (an installation, a
    /// construction element). Its polygons are also this object's xlink
    /// targets: a parent's `lodNSolid` routinely composes faces the schema
    /// defines inside such a child. Registering them emits nothing, so a face
    /// is not duplicated; the child still owns its own geometry.
    pub fn add_child(&mut self, child: ObjectBuilder) {
        for (id, poly) in &child.polygons {
            self.polygons
                .entry(id.clone())
                .or_insert_with(|| poly.clone());
        }
        let offset = self.aggregate_polygons.len();
        self.aggregate_polygons
            .extend(child.aggregate_polygons.iter().cloned());
        for (id, members) in &child.aggregates {
            self.aggregates.entry(id.clone()).or_insert_with(|| {
                members
                    .iter()
                    .map(|m| match m {
                        AggregateMember::Polygon(i) => AggregateMember::Polygon(i + offset),
                        other => other.clone(),
                    })
                    .collect()
            });
        }
        self.children.push(child);
    }

    /// Resolve every reference and build the object, its children after it,
    /// charging aggregate expansion to the document's `budget`.
    pub fn finish(self, budget: &XlinkBudget) -> Result<GmlObject> {
        budget.grant(self.source_polygons());
        self.build(budget, &[])
    }

    /// Polygons harvested from this object's subtree, children included.
    fn source_polygons(&self) -> usize {
        self.parsed_polygons
            + self
                .children
                .iter()
                .map(Self::source_polygons)
                .sum::<usize>()
    }

    /// Count harvested geometry, keep its identified aggregates (their polygon
    /// members moved into the aggregate arena), and hand the polygons back for
    /// the caller to place.
    fn absorb(&mut self, collected: Collected) -> Vec<(Option<String>, Polygon)> {
        self.parsed_polygons += collected.polygons.len();
        let mut arena_index: HashMap<usize, usize> = HashMap::new();
        for (id, members) in collected.aggregates {
            let members = members
                .into_iter()
                .map(|m| match m {
                    AggregateMember::Polygon(i) => {
                        let at = *arena_index.entry(i).or_insert_with(|| {
                            self.aggregate_polygons
                                .push(collected.polygons[i].1.clone());
                            self.aggregate_polygons.len() - 1
                        });
                        AggregateMember::Polygon(at)
                    }
                    other => other,
                })
                .collect();
            self.aggregates.insert(id, members);
        }
        collected.polygons
    }

    /// Build the object; `outer` holds the xlink targets of its enclosing
    /// objects, nearest first.
    fn build(self, budget: &XlinkBudget, outer: &[Targets<'_>]) -> Result<GmlObject> {
        let mut geometries = Vec::new();
        for (lod, geom) in &self.solids {
            geometries.push(self.solid_geometry(geom, lod, budget)?);
        }
        // A LoD whose boundary surfaces carry geometry gets that geometry
        // below, which takes in any standalone face of the same LoD; two
        // geometries at one LoD would let a consumer keep the first and
        // silently discard the other.
        let semantic_lods: HashSet<&String> = self
            .boundary_polys
            .iter()
            .map(|(_, lod, _)| lod)
            .chain(self.boundary_refs.iter().map(|(_, lod, _)| lod))
            .collect();
        for (lod, polys) in &self.plain_surfaces {
            if semantic_lods.contains(lod) || self.solids.iter().any(|(l, _)| l == lod) {
                continue;
            }
            let faces = polys.iter().map(|p| plain_face(p, None)).collect();
            geometries.push(Geometry {
                lod: lod.clone(),
                shape: Shape::MultiSurface(faces),
            });
        }
        // Each LoD whose geometry lives only in boundary surfaces (no solid
        // at that LoD) gets one MultiSurface. Where a solid exists, the
        // boundary polygons are already its faces.
        let mut ms_lods: Vec<&String> = Vec::new();
        let boundary_lods = self
            .boundary_polys
            .iter()
            .map(|(_, lod, _)| lod)
            .chain(self.boundary_refs.iter().map(|(_, lod, _)| lod));
        for lod in boundary_lods {
            let has_solid = self.solids.iter().any(|(l, _)| l == lod);
            if !has_solid && !ms_lods.contains(&lod) {
                ms_lods.push(lod);
            }
        }
        for lod in ms_lods {
            let faces = self.boundary_faces(lod, budget)?;
            // A LoD whose members were all unresolved xlinks has no face.
            if !faces.is_empty() {
                geometries.push(Geometry {
                    lod: lod.clone(),
                    shape: Shape::MultiSurface(faces),
                });
            }
        }
        // Points, lines and TINs, at a LoD no surface or solid holds.
        for g in self.direct {
            if !geometries.iter().any(|have: &Geometry| have.lod == g.lod) {
                geometries.push(g);
            }
        }
        let mut enclosing = Vec::with_capacity(outer.len() + 1);
        enclosing.push(Targets {
            polygons: &self.polygons,
            aggregates: &self.aggregates,
            aggregate_polygons: &self.aggregate_polygons,
        });
        enclosing.extend_from_slice(outer);
        let children = self
            .children
            .into_iter()
            .map(|mut c| {
                c.inherit(&enclosing);
                c.build(budget, &enclosing)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(GmlObject {
            id: self.id,
            class: self.class,
            attributes: self.attributes,
            addresses: self.addresses,
            geometries,
            boundary_surfaces: self.surfaces,
            appearance: self.appearance,
            children,
        })
    }

    /// Take from the enclosing objects, nearest first, the xlink targets
    /// this object references but does not define: a room's solid reuses its
    /// building's wall polygons, reversed, or the building's identified
    /// `gml:CompositeSurface`. Only what is referenced is copied, an
    /// aggregate with the polygons and aggregates it holds.
    fn inherit(&mut self, outer: &[Targets<'_>]) {
        let solid_refs = self
            .solids
            .iter()
            .flat_map(|(_, geom)| match geom {
                SolidGeom::Solid(solid) => std::slice::from_ref(solid),
                SolidGeom::Composite(solids) => solids.as_slice(),
            })
            .flat_map(|solid| solid.shells.iter().flatten())
            .filter_map(|sref| match &sref.target {
                RefTarget::Xlink(id) => Some(id),
                RefTarget::Inline(_) => None,
            });
        let member_refs = self.aggregates.values().flatten().filter_map(|m| match m {
            AggregateMember::Ref(id) => Some(id),
            AggregateMember::Polygon(_) => None,
        });
        let boundary_refs = self.boundary_refs.iter().map(|(_, _, href)| href);
        let mut pending: Vec<String> = solid_refs
            .chain(member_refs)
            .chain(boundary_refs)
            .cloned()
            .collect();
        while let Some(id) = pending.pop() {
            if self.polygons.contains_key(&id) || self.aggregates.contains_key(&id) {
                continue;
            }
            for targets in outer {
                if let Some(poly) = targets.polygons.get(&id) {
                    self.polygons.insert(id, poly.clone());
                    break;
                }
                if let Some(members) = targets.aggregates.get(&id) {
                    let members = members
                        .iter()
                        .map(|m| match m {
                            AggregateMember::Polygon(i) => {
                                self.aggregate_polygons
                                    .push(targets.aggregate_polygons[*i].clone());
                                AggregateMember::Polygon(self.aggregate_polygons.len() - 1)
                            }
                            AggregateMember::Ref(r) => {
                                pending.push(r.clone());
                                m.clone()
                            }
                        })
                        .collect();
                    self.aggregates.insert(id, members);
                    break;
                }
            }
        }
    }

    fn solid_geometry(
        &self,
        geom: &SolidGeom,
        lod: &str,
        budget: &XlinkBudget,
    ) -> Result<Geometry> {
        let shape = match geom {
            SolidGeom::Solid(solid) => Shape::Solid(self.solid_shells(solid, budget)?),
            SolidGeom::Composite(solids) => Shape::CompositeSolid(
                solids
                    .iter()
                    .map(|s| self.solid_shells(s, budget))
                    .collect::<Result<_>>()?,
            ),
        };
        Ok(Geometry {
            lod: lod.to_string(),
            shape,
        })
    }

    fn solid_shells(&self, solid: &RawSolid, budget: &XlinkBudget) -> Result<Vec<Vec<Face>>> {
        let mut shells = Vec::with_capacity(solid.shells.len());
        for shell in &solid.shells {
            let mut expanded = Vec::with_capacity(shell.len());
            for sref in shell {
                expanded.extend(self.expand(sref, budget)?);
            }
            let mut faces = Vec::with_capacity(expanded.len());
            for sref in &expanded {
                let polygon = self.resolve(sref)?;
                // The face's own id: the xlink target, or an inline polygon's
                // own id. Either may be tagged with a boundary surface, an
                // inline one when a boundary surface xlinks to it.
                let id = match &sref.target {
                    RefTarget::Xlink(id) => Some(id.clone()),
                    RefTarget::Inline(poly) => poly.id.clone(),
                };
                let surface = id
                    .as_deref()
                    .and_then(|id| self.semantic_of_polygon.get(id))
                    .copied();
                faces.push(Face {
                    polygon: polygon.clone(),
                    id,
                    reversed: sref.reverse,
                    surface,
                });
            }
            shells.push(faces);
        }
        Ok(shells)
    }

    /// The faces of the MultiSurface at `lod`: the boundary polygons, the
    /// boundary xlinks resolved, then any standalone face at that LoD none of
    /// them already holds (untyped).
    fn boundary_faces(&self, lod: &str, budget: &XlinkBudget) -> Result<Vec<Face>> {
        let mut faces = Vec::new();
        // A polygon referenced more than once (two boundary surfaces xlinking
        // one id) is one face.
        let mut emitted: HashSet<&str> = HashSet::new();
        for (surface, poly_lod, poly) in &self.boundary_polys {
            if poly_lod != lod {
                continue;
            }
            if let Some(id) = poly.id.as_deref()
                && !emitted.insert(id)
            {
                continue;
            }
            faces.push(plain_face(poly, Some(*surface)));
        }
        for (surface, ref_lod, href) in &self.boundary_refs {
            if ref_lod != lod {
                continue;
            }
            if !emitted.insert(href) {
                continue;
            }
            let polys = self.xlink_polygons(href, budget)?;
            if polys.is_empty() {
                eprintln!(
                    "warning: a boundary surface of {} references #{href}, which resolves to no polygon in this object; skipping",
                    self.describe()
                );
                continue;
            }
            for poly in &polys {
                faces.push(plain_face(poly, Some(*surface)));
            }
        }
        // A standalone face no boundary surface holds would otherwise vanish:
        // one LoD yields one geometry.
        for (plain_lod, polys) in &self.plain_surfaces {
            if plain_lod != lod {
                continue;
            }
            for poly in polys {
                if let Some(id) = poly.id.as_deref()
                    && !emitted.insert(id)
                {
                    continue;
                }
                faces.push(plain_face(poly, None));
            }
        }
        Ok(faces)
    }

    /// The polygons an xlink target names: one identified polygon, or every
    /// polygon an identified surface aggregate stands for. Empty when the id
    /// resolves to no polygon.
    fn xlink_polygons(&self, id: &str, budget: &XlinkBudget) -> Result<Vec<Polygon>> {
        let sref = SurfaceRef {
            reverse: false,
            target: RefTarget::Xlink(id.to_string()),
        };
        Ok(self
            .expand(&sref, budget)?
            .iter()
            .filter_map(|r| self.resolve(r).ok().cloned())
            .collect())
    }

    /// `sref`, with an xlink to a surface aggregate expanded into one
    /// reference per polygon the aggregate stands for, following xlinked and
    /// nested members into further aggregates. Each keeps `sref`'s
    /// orientation, and a member polygon is referenced by its own id when
    /// that is registered, so its semantics resolve, else inline.
    ///
    /// The walk is iterative and refuses, naming the aggregate xlinked to: an
    /// aggregate reached again through its own members (a cycle); a chain of
    /// aggregates deeper than [`MAX_XLINK_DEPTH`]; and any reference past the
    /// document's [`XlinkBudget`], which every reference this expansion
    /// produces is charged to.
    fn expand(&self, sref: &SurfaceRef, budget: &XlinkBudget) -> Result<Vec<SurfaceRef>> {
        let RefTarget::Xlink(root) = &sref.target else {
            return Ok(vec![sref.clone()]);
        };
        let Some((root_key, root_members)) = self
            .aggregates
            .get_key_value(root)
            .filter(|_| !self.polygons.contains_key(root))
        else {
            return Ok(vec![sref.clone()]);
        };
        let mut out = Vec::new();
        let mut stack: Vec<(&str, &[AggregateMember], usize)> =
            vec![(root_key.as_str(), root_members.as_slice(), 0)];
        let mut on_path: HashSet<&str> = HashSet::from([root_key.as_str()]);
        while let Some(&(key, members, next)) = stack.last() {
            if next == members.len() {
                on_path.remove(key);
                stack.pop();
                continue;
            }
            stack.last_mut().expect("non-empty").2 += 1;
            let target = match &members[next] {
                AggregateMember::Polygon(i) => {
                    let poly = &self.aggregate_polygons[*i];
                    match &poly.id {
                        Some(pid) if self.polygons.contains_key(pid) => {
                            RefTarget::Xlink(pid.clone())
                        }
                        _ => RefTarget::Inline(poly.clone()),
                    }
                }
                AggregateMember::Ref(id) => {
                    if !self.polygons.contains_key(id)
                        && let Some((key, members)) = self.aggregates.get_key_value(id)
                    {
                        if on_path.contains(key.as_str()) {
                            let path: Vec<&str> = stack.iter().map(|f| f.0).collect();
                            return Err(self.expansion_error(
                                root,
                                &format!(
                                    "reaches an aggregate again through its own members (an \
                                     xlink cycle: {} -> {key})",
                                    path.join(" -> ")
                                ),
                            ));
                        }
                        if stack.len() >= MAX_XLINK_DEPTH {
                            return Err(self.expansion_error(
                                root,
                                &format!("nests aggregates more than {MAX_XLINK_DEPTH} deep"),
                            ));
                        }
                        on_path.insert(key.as_str());
                        stack.push((key.as_str(), members.as_slice(), 0));
                        continue;
                    }
                    RefTarget::Xlink(id.clone())
                }
            };
            if !budget.charge() {
                return Err(self.expansion_error(
                    root,
                    "expands to more surfaces than the document defines polygons for (an \
                     xlink fan-out)",
                ));
            }
            out.push(SurfaceRef {
                reverse: sref.reverse,
                target,
            });
        }
        Ok(out)
    }

    /// How an error names this object: its class's local name and its id.
    fn describe(&self) -> String {
        let which = self.id.as_deref().unwrap_or("<no gml:id>");
        format!("{} {which}", self.class.local)
    }

    fn expansion_error(&self, root: &str, what: &str) -> Error {
        Error::invalid(format!("{}: xlink #{root} {what}", self.describe()))
    }

    /// The polygon a surface reference points at: the xlink resolved against
    /// the registry, or the inline polygon.
    fn resolve<'a>(&'a self, sref: &'a SurfaceRef) -> Result<&'a Polygon> {
        match &sref.target {
            RefTarget::Inline(poly) => Ok(poly),
            RefTarget::Xlink(id) => self.polygons.get(id).ok_or_else(|| {
                // Name the object, not just the polygon: on a national corpus
                // of thousands of files the id alone gives an operator nothing
                // to search for.
                Error::invalid(format!(
                    "{}: CityGML solid references #{id}, which is not defined anywhere in \
                     this object's subtree",
                    self.describe()
                ))
            }),
        }
    }
}

/// A face that is a polygon as it is, by its own id.
fn plain_face(poly: &Polygon, surface: Option<usize>) -> Face {
    Face {
        polygon: poly.clone(),
        id: poly.id.clone(),
        reversed: false,
        surface,
    }
}
