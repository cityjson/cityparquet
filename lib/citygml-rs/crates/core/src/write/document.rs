//! What the writers of both versions share: a document's state (its XML,
//! the `gml:id`s it has, the CRS until stated, what it left out), where an
//! object's geometries, faces and boundary surfaces go ([`Plan`]), GML
//! geometry as a property, and ADE elements as the document wrote them.
//!
//! What a class holds and how its properties are named is the version's:
//! [`PlanRules`] asks. [`VersionWriter`] writes, around a version's own
//! objects and textures, the top-level members, the appearances and the
//! document's end.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write;

use super::gml::{self, Member};
use super::{XmlWriter, is_ncname, number, numbers};
use crate::convert::Losses;
use crate::model::{
    AdeLayout, Appearance, Attribute, AttributeName, BoundarySurface, Face, Geometry, GmlObject,
    Material, Module, Namespace, QName, Shape, Template, Texture, Value, XmlContent, XmlNode,
    order_key,
};
use crate::vocabulary::{module_of_prefix, module_prefix};
use crate::{Error, Result};

/// The prefixes a root binds besides the modules' and the ADEs': what an
/// ADE's prefix must not be.
pub const RESERVED_PREFIXES: &[&str] = &["gml", "xlink", "xAL", "xsi", "xml", "ns"];

/// The prefix an element of a namespace the root does not declare binds on
/// itself.
const LOCAL_PREFIX: &str = "ns";

/// The CityGML class whose content an object of an ADE class has: the
/// CityGML class the ADE class specialises, if any.
pub type ContentOf = Box<dyn Fn(&QName) -> Option<QName> + Send + Sync>;

/// An ADE a document declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ade {
    /// The prefix its namespace is bound to.
    pub prefix: String,
    /// Its namespace URI.
    pub uri: String,
    /// Where its XML Schema is, for `xsi:schemaLocation`.
    pub schema: Option<String>,
}

/// What a document's root says before its first object.
#[derive(Debug, Clone, Default)]
pub struct Head {
    /// The CRS every coordinate is in, as `srsName`.
    pub srs_name: Option<String>,
    /// The extent of the whole model, `[minx, miny, minz, maxx, maxy,
    /// maxz]`.
    pub envelope: Option<[f64; 6]>,
    /// The ADEs the objects may use.
    pub ades: Vec<Ade>,
    /// Where their elements go in their objects' content.
    pub layout: AdeLayout,
}

/// What a class holds, as a version's content models say: what [`Base::plan`]
/// asks.
pub trait PlanRules {
    /// The properties of the class a geometry at LoD `n` may be written as,
    /// best first; none when the class has no property for it.
    fn candidates(&self, geometry: &Geometry, n: u8) -> Vec<String>;
    /// The property boundary surface `s` of `surfaces` is written in, or
    /// `None` when the class cannot hold it.
    fn surface_holder(&self, surfaces: &[BoundarySurface], s: usize) -> Option<&'static str>;
    /// Whether a boundary surface of `class` has geometry at LoD `n`.
    fn surface_holds_lod(&self, class: &QName, n: u8) -> bool;
}

/// A version's document writer, for what both versions write alike around
/// what each writes its own way: the top-level objects, the appearances
/// and the end of the document.
/// The integer LoD a relief component's `dem:lod` states: its first
/// geometry's, without the CityJSON refinement (`"2.1"` is 2).
pub fn relief_lod(o: &GmlObject) -> String {
    o.geometries
        .first()
        .and_then(|g| g.lod.split('.').next())
        .filter(|l| !l.is_empty())
        .unwrap_or("1")
        .to_string()
}

pub trait VersionWriter<W: Write>: Sized {
    /// The element of an `app:Appearance` holding one material or texture.
    const SURFACE_DATA: &'static str;
    /// The element of the `app:Appearance` of a relative geometry, after
    /// the objects.
    const APPEARANCE_MEMBER: &'static str;

    fn base(&mut self) -> &mut Base<W>;
    fn into_base(self) -> Base<W>;
    /// Whether `o` can be written at all; counts it when not.
    fn writable(&mut self, o: &GmlObject) -> bool;
    /// One object, its element named by its class; adds to `hoisted` what
    /// is to be written after it as an object of its own.
    fn object(&mut self, o: GmlObject, hoisted: &mut Vec<GmlObject>) -> Result<()>;
    /// A texture, on the rings of `rings` (ring -> its polygon) it names.
    fn texture(&mut self, t: &Texture, rings: &HashMap<String, String>) -> Result<()>;
    /// The LoD of the `dem:ReliefFeature` `o` is written in, when `o` is a
    /// relief component: CityJSON has no ReliefFeature, so the reader passes
    /// through it, and the writer puts it back, since common readers
    /// (citygml4j) read a relief component only inside one.
    fn relief_feature_lod(&self, _o: &GmlObject) -> Option<String> {
        None
    }

    /// `object` as a top-level object, and what it cannot hold, each in a
    /// `core:cityObjectMember`.
    fn write_top_level(&mut self, object: GmlObject) -> Result<()> {
        let mut pending = vec![object];
        while let Some(o) = pending.pop() {
            if !self.writable(&o) {
                let mut orphans = Vec::new();
                self.base().orphan_children(o, &mut orphans);
                pending.extend(orphans.into_iter().rev());
                continue;
            }
            self.base().x.open("core:cityObjectMember", &[])?;
            let relief = self.relief_feature_lod(&o);
            if let Some(lod) = &relief {
                self.base().x.open("dem:ReliefFeature", &[])?;
                self.base().x.leaf("dem:lod", &[], lod)?;
                self.base().x.open("dem:reliefComponent", &[])?;
            }
            let mut hoisted = Vec::new();
            self.object(o, &mut hoisted)?;
            if relief.is_some() {
                self.base().x.close()?;
                self.base().x.close()?;
            }
            self.base().x.close()?;
            self.base().objects += 1;
            pending.extend(hoisted.into_iter().rev());
        }
        Ok(())
    }

    /// `appearance`, one `app:Appearance` per run of a theme ([`themes`]) in
    /// property `element`: each material targeting the polygons of `targets`
    /// it names, each texture the rings `rings` gives the polygon of.
    fn appearance(
        &mut self,
        element: &str,
        appearance: &Appearance,
        targets: &HashSet<String>,
        rings: &HashMap<String, String>,
    ) -> Result<()> {
        for (theme, data) in themes(appearance) {
            self.base().x.open(element, &[])?;
            self.base().x.open("app:Appearance", &[])?;
            if !theme.is_empty() {
                self.base().x.leaf("app:theme", &[], theme)?;
            }
            for d in data {
                match d {
                    SurfaceData::Material(m) => {
                        self.base().material(Self::SURFACE_DATA, m, targets)?
                    }
                    SurfaceData::Texture(t) => self.texture(t, rings)?,
                }
            }
            self.base().x.close()?;
            self.base().x.close()?;
        }
        Ok(())
    }

    /// End the document: the appearance of the relative geometries written,
    /// then the root. Gives back what it was written to, and what was left
    /// out.
    fn finish_document(mut self) -> Result<(W, Losses)> {
        let templates = std::mem::take(&mut self.base().templates);
        let written = std::mem::take(&mut self.base().written);
        for (template, _) in templates.iter().zip(written).filter(|(_, w)| w.is_some()) {
            let (targets, rings) = targets(std::slice::from_ref(&template.geometry));
            self.appearance(
                Self::APPEARANCE_MEMBER,
                &template.appearance,
                &targets,
                &rings,
            )?;
        }
        let mut b = self.into_base();
        b.x.close()?;
        Ok((b.x.finish()?, b.losses))
    }
}

/// A document being written, as far as both versions write it alike.
pub struct Base<W: Write> {
    pub x: XmlWriter<W>,
    /// The version, as messages name it (`CityGML 2.0`).
    pub version: &'static str,
    /// The namespace URI of each module the version has.
    module_uri: fn(Module) -> Option<&'static str>,
    /// The CRS, until an element has stated it.
    srs_name: Option<String>,
    /// The relative geometries the objects' implicit geometries place.
    pub templates: Vec<Template>,
    /// The `gml:id` of each template's relative geometry, once written.
    pub written: Vec<Option<String>>,
    /// Namespace URI -> its prefix, for the namespaces outside CityGML and
    /// GML the root declares.
    pub prefixes: HashMap<String, String>,
    /// Where ADE elements go in their objects' content.
    pub layout: AdeLayout,
    /// Every `gml:id` written.
    ids: HashSet<String>,
    /// For the `gml:id`s this writer makes up.
    next_id: usize,
    /// Top-level objects written.
    pub objects: usize,
    pub losses: Losses,
}

impl<W: Write> Base<W> {
    /// Start a document in `version` on `out`: its `core:CityModel` root,
    /// declaring each module of `modules` (with the schema
    /// `xsi:schemaLocation` names for it), each of `namespaces` by its
    /// prefix, and each ADE of `head`; then the model's envelope. Geometry
    /// is GML 3.2 when `namespaces` binds `gml` to its namespace, else GML
    /// 3.1.1.
    /// `module_uri` gives a module's namespace URI; `templates` are the
    /// relative geometries the objects' implicit geometries place, by
    /// position.
    pub fn start(
        out: W,
        version: &'static str,
        module_uri: fn(Module) -> Option<&'static str>,
        modules: &[(Module, &str)],
        namespaces: &[(&str, &str)],
        head: Head,
        templates: Vec<Template>,
    ) -> Result<Self> {
        let mut x = XmlWriter::new(out)?;
        if namespaces.contains(&("gml", gml::NS_GML32)) {
            x.gml = gml::Version::V3_2;
        }
        let mut declarations: Vec<(String, String)> = Vec::new();
        let mut locations = Vec::new();
        for (module, schema) in modules {
            let uri = module_uri(*module).ok_or_else(|| {
                Error::invalid(format!(
                    "{version} has no module {}",
                    module_prefix(*module)
                ))
            })?;
            declarations.push((format!("xmlns:{}", module_prefix(*module)), uri.into()));
            locations.push(format!("{uri} {schema}"));
        }
        let mut prefixes = HashMap::new();
        for (prefix, uri) in namespaces {
            declarations.push((format!("xmlns:{prefix}"), uri.to_string()));
            if *prefix != "gml" {
                prefixes.insert(uri.to_string(), prefix.to_string());
            }
        }
        for ade in &head.ades {
            if !is_ncname(&ade.prefix)
                || RESERVED_PREFIXES.contains(&ade.prefix.as_str())
                || module_of_prefix(&ade.prefix).is_some()
            {
                return Err(Error::invalid(format!(
                    "{} cannot be the prefix of the ADE {}",
                    ade.prefix, ade.uri
                )));
            }
            declarations.push((format!("xmlns:{}", ade.prefix), ade.uri.clone()));
            prefixes.insert(ade.uri.clone(), ade.prefix.clone());
            if let Some(schema) = &ade.schema {
                locations.push(format!("{} {schema}", ade.uri));
            }
        }
        declarations.push(("xsi:schemaLocation".into(), locations.join(" ")));
        x.open("core:CityModel", &pairs(&declarations))?;
        let mut srs_name = head.srs_name;
        if let Some(e) = head.envelope {
            x.open("gml:boundedBy", &[])?;
            envelope(&mut x, &e, srs_name.take().as_deref())?;
            x.close()?;
        }
        Ok(Self {
            x,
            version,
            module_uri,
            srs_name,
            written: vec![None; templates.len()],
            templates,
            prefixes,
            layout: head.layout,
            ids: HashSet::new(),
            next_id: 0,
            objects: 0,
            losses: Losses::default(),
        })
    }

    /// The children of `o`, an object not written, as top-level objects of
    /// their own.
    pub fn orphan_children(&mut self, o: GmlObject, out: &mut Vec<GmlObject>) {
        for child in o.children {
            self.losses
                .note("objects their parent's class cannot hold (written on their own)");
            out.push(child);
        }
    }

    /// Whether an object `o` of a class whose content is `known` (and is a
    /// TIN relief's when `tin`) can be written at all; counts it when not.
    /// An ADE class needs no known content.
    pub fn writable(&mut self, o: &GmlObject, known: bool, tin_relief: bool) -> bool {
        let ade = matches!(o.class.ns, Namespace::Other(_));
        if ade && !is_ncname(&o.class.local) {
            self.losses
                .note("objects of an ADE class whose name is not an XML name");
            return false;
        }
        if !known && !ade {
            self.losses.note(format!(
                "objects of a class {} does not have ({})",
                self.version, o.class.local
            ));
            return false;
        }
        if tin_relief && !o.geometries.iter().any(|g| tin(g).is_some()) {
            self.losses
                .note("TIN reliefs without a surface of triangles");
            return false;
        }
        true
    }

    /// A material, in a `surface_data` element, targeting the polygons of
    /// `targets` it names; left out when it targets none.
    pub fn material(
        &mut self,
        surface_data: &str,
        m: &Material,
        targets: &HashSet<String>,
    ) -> Result<()> {
        let written: Vec<&String> = m.targets.iter().filter(|t| targets.contains(*t)).collect();
        if written.len() < m.targets.len() {
            self.losses
                .note("material targets the document does not write");
        }
        if written.is_empty() {
            return Ok(());
        }
        let id = m.id.clone().filter(|id| self.claim(id));
        let attrs: Vec<(&str, &str)> = id.iter().map(|id| ("gml:id", id.as_str())).collect();
        self.x.open(surface_data, &[])?;
        self.x.open("app:X3DMaterial", &attrs)?;
        if let Some(name) = &m.name {
            self.x.leaf("gml:name", &[], name)?;
        }
        let colour = |c: &Option<Vec<f64>>| c.as_ref().map(|c| numbers(c.iter().copied()));
        for (element, value) in [
            ("app:ambientIntensity", m.ambient_intensity.map(number)),
            ("app:diffuseColor", colour(&m.diffuse_color)),
            ("app:emissiveColor", colour(&m.emissive_color)),
            ("app:specularColor", colour(&m.specular_color)),
            ("app:shininess", m.shininess.map(number)),
            ("app:transparency", m.transparency.map(number)),
            ("app:isSmooth", m.is_smooth.map(|b| b.to_string())),
        ] {
            if let Some(value) = value {
                self.x.leaf(element, &[], &value)?;
            }
        }
        for t in written {
            self.x.leaf("app:target", &[], &format!("#{t}"))?;
        }
        self.x.close()?;
        self.x.close()
    }

    /// A face in a boundary surface's geometry: the polygon a solid refers
    /// to as it is, any other face as its geometry uses it.
    pub fn surface_member(&mut self, face: &Face, solid: bool) -> Result<()> {
        if solid {
            self.x.open("gml:surfaceMember", &[])?;
            gml::polygon(&mut self.x, &face.polygon, &[])?;
            self.x.close()
        } else {
            gml::member(&mut self.x, "gml:surfaceMember", face, Member::Inline)
        }
    }

    /// `id`, when no element of the document has it yet; counted when one
    /// has.
    pub fn claim(&mut self, id: &str) -> bool {
        if self.ids.insert(id.to_string()) {
            return true;
        }
        self.losses
            .note("gml:ids that are not unique in the document (left out where repeated)");
        false
    }

    /// A `gml:id` no element of the document has, claimed.
    pub fn fresh_id(&mut self) -> String {
        let id = self.unused_id();
        self.ids.insert(id.clone());
        id
    }

    /// A `gml:id` no element of the document has, for an object written
    /// later, which claims it then.
    pub fn unused_id(&mut self) -> String {
        loop {
            self.next_id += 1;
            let id = format!("_w{}", self.next_id);
            if !self.ids.contains(&id) {
                return id;
            }
        }
    }

    /// The CRS, for the first element that states it; `None` after.
    pub fn srs_name(&mut self) -> Option<String> {
        self.srs_name.take()
    }

    /// The `srsName` attribute, on the first element that states the CRS.
    pub fn srs(&mut self) -> Vec<(&'static str, String)> {
        self.srs_name()
            .map(|s| ("srsName", s))
            .into_iter()
            .collect()
    }

    /// The prefixed name of `q`, and the namespace declaration its element
    /// needs when the root has none for it; `None` for a name in a module
    /// the version does not have.
    pub fn name(&self, q: &QName) -> Option<(String, Option<(String, String)>)> {
        Some(match &q.ns {
            Namespace::Module(m) => {
                (self.module_uri)(*m)?;
                (format!("{}:{}", module_prefix(*m), q.local), None)
            }
            Namespace::Gml => (format!("gml:{}", q.local), None),
            Namespace::Other(uri) => match self.prefixes.get(uri) {
                Some(p) => (format!("{p}:{}", q.local), None),
                None => (
                    format!("{LOCAL_PREFIX}:{}", q.local),
                    Some((format!("xmlns:{LOCAL_PREFIX}"), uri.clone())),
                ),
            },
        })
    }

    /// The prefixed name of `q`, which must be in a namespace the document
    /// can name.
    pub fn element(&self, q: &QName) -> Result<String> {
        match self.name(q) {
            Some((name, None)) => Ok(name),
            _ => Err(Error::invalid(format!(
                "no {} element can be named {}",
                self.version, q.local
            ))),
        }
    }

    /// Open `q` with `attributes`, declaring its namespace when the root
    /// does not.
    pub fn open_named(&mut self, q: &QName, attributes: &[(&str, &str)]) -> Result<()> {
        let Some((name, declaration)) = self.name(q) else {
            return Err(Error::invalid(format!(
                "no {} element can be named {}",
                self.version, q.local
            )));
        };
        let mut attrs = attributes.to_vec();
        if let Some((k, v)) = &declaration {
            attrs.push((k, v));
        }
        self.x.open(&name, &attrs)
    }

    /// Where each geometry, face and boundary surface of `o` goes, as
    /// `rules` say what its class holds; makes `o`'s ids unique in the
    /// document on the way.
    pub fn plan(&mut self, o: &mut GmlObject, rules: &dyn PlanRules) -> Plan {
        let geometries = o.geometries.len();
        let mut plan = Plan {
            property: vec![None; geometries],
            lod: vec![0; geometries],
            solid: vec![false; geometries],
            tags: vec![Vec::new(); geometries],
            holder: vec![None; o.boundary_surfaces.len()],
            nested: vec![Vec::new(); o.boundary_surfaces.len()],
            lods: vec![BTreeSet::new(); o.boundary_surfaces.len()],
            targets: HashSet::new(),
            rings: HashMap::new(),
            tin: None,
        };
        for (g, geometry) in o.geometries.iter().enumerate() {
            let Some(n) = lod_number(&geometry.lod) else {
                self.losses.note(format!(
                    "geometries at a LoD {} does not have",
                    self.version
                ));
                continue;
            };
            if !is_whole(&geometry.lod) {
                self.losses
                    .note("geometries whose LoD is written as its whole number");
            }
            let shape = &geometry.shape;
            let fits = rules.candidates(geometry, n);
            if fits.is_empty() {
                self.losses.note(format!(
                    "{} geometries at a LoD their class has no {} property for",
                    kind(shape),
                    kind(shape)
                ));
                continue;
            }
            let free = fits
                .into_iter()
                .find(|c| !plan.property.iter().flatten().any(|p| p == c));
            let Some(chosen) = free else {
                self.losses
                    .note("geometries at a LoD their object already has one of that kind at");
                continue;
            };
            plan.lod[g] = n;
            plan.solid[g] = matches!(
                shape,
                Shape::Solid(_) | Shape::CompositeSolid(_) | Shape::MultiSolid(_)
            );
            plan.property[g] = Some(chosen);
        }

        // Which surfaces are written, and in which property.
        for s in 0..o.boundary_surfaces.len() {
            plan.holder[s] = rules.surface_holder(&o.boundary_surfaces, s);
            if let (Some(p), Some(_)) = (o.boundary_surfaces[s].parent, plan.holder[s]) {
                plan.nested[p].push(s);
            }
        }
        // Each face's surface, where that surface can hold it, and every id
        // made unique.
        let mut untagged = 0usize;
        for (g, geometry) in o.geometries.iter_mut().enumerate() {
            if plan.property[g].is_none() {
                continue;
            }
            let n = plan.lod[g];
            let solid = plan.solid[g];
            let mut tags = Vec::new();
            for face in faces_mut(&mut geometry.shape) {
                let tag = face.surface.filter(|&s| {
                    plan.holder[s].is_some()
                        && rules.surface_holds_lod(&o.boundary_surfaces[s].class, n)
                });
                if face.surface.is_some() && tag.is_none() {
                    untagged += 1;
                }
                if let Some(s) = tag {
                    plan.lods[s].insert(n);
                }
                if let Some(id) = &face.polygon.id
                    && !self.claim(id)
                {
                    face.polygon.id = None;
                }
                if tag.is_some() && solid && face.polygon.id.is_none() {
                    face.polygon.id = Some(self.fresh_id());
                }
                for ring in &mut face.polygon.ring_ids {
                    if let Some(id) = ring
                        && !self.claim(id)
                    {
                        *ring = None;
                    }
                }
                if let Some(id) = &face.polygon.id {
                    plan.targets.insert(id.clone());
                    for ring in face.polygon.ring_ids.iter().flatten() {
                        plan.rings.insert(ring.clone(), id.clone());
                    }
                }
                tags.push(tag);
            }
            plan.tags[g] = tags;
        }
        // A TIN's triangles carry no `gml:id`, so a material colours them
        // through the `gml:TriangulatedSurface`'s, which it can when it
        // colours every triangle.
        if let Some(g) = plan
            .property
            .iter()
            .position(|p| p.as_deref() == Some("tin"))
        {
            let triangles: Vec<Option<String>> = o.geometries[g]
                .shape
                .faces()
                .map(|f| f.polygon.id.clone())
                .collect();
            for id in triangles.iter().flatten() {
                plan.targets.remove(id);
            }
            plan.rings
                .retain(|_, polygon| !triangles.contains(&Some(polygon.clone())));
            let tin = self.fresh_id();
            if let Some(all) = triangles.into_iter().collect::<Option<HashSet<String>>>() {
                for m in &mut o.appearance.materials {
                    if all.iter().all(|t| m.targets.contains(t)) {
                        m.targets.retain(|t| !all.contains(t));
                        m.targets.push(tin.clone());
                    }
                }
            }
            plan.targets.insert(tin.clone());
            plan.tin = Some(tin);
        }
        if untagged > 0 {
            for _ in 0..untagged {
                self.losses.note(
                    "faces of boundary surfaces whose class has no geometry at their LoD (kept \
                     untagged)",
                );
            }
        }
        plan
    }

    /// Geometry `g`, `geometry`, as property `name` of `module`: the whole
    /// of a solid, referring to the faces a boundary surface holds, or the
    /// faces of a surface geometry no boundary surface holds. An implicit
    /// geometry is not GML geometry, and is the caller's to write.
    pub fn geometry(
        &mut self,
        module: &Namespace,
        name: &str,
        geometry: &Geometry,
        g: usize,
        plan: &Plan,
    ) -> Result<()> {
        let tags = &plan.tags[g];
        let property = QName::new(module.clone(), name);
        let shape = &geometry.shape;
        if let Shape::Instance(_) = shape {
            return Err(Error::invalid(
                "an implicit geometry is the version's to write",
            ));
        }
        if plan.solid[g] {
            let srs = self.srs();
            self.open_named(&property, &[])?;
            let mut faces = tags.iter();
            let mut how = |_: &Face| match faces.next() {
                Some(Some(_)) => Member::Reference,
                _ => Member::Inline,
            };
            gml::shape(&mut self.x, shape, &pairs(&srs), &mut how)?;
            return self.x.close();
        }
        match shape {
            Shape::MultiSurface(faces) | Shape::CompositeSurface(faces) => {
                let untagged: Vec<&Face> = faces
                    .iter()
                    .zip(tags)
                    .filter(|(_, t)| t.is_none())
                    .map(|(f, _)| f)
                    .collect();
                if untagged.is_empty() {
                    return Ok(());
                }
                let element = match shape {
                    Shape::CompositeSurface(_) if name.ends_with("Geometry") => {
                        "gml:CompositeSurface"
                    }
                    Shape::CompositeSurface(_) => {
                        self.losses
                            .note("CompositeSurfaces written as MultiSurfaces");
                        "gml:MultiSurface"
                    }
                    _ => "gml:MultiSurface",
                };
                let srs = self.srs();
                self.open_named(&property, &[])?;
                gml::surfaces(&mut self.x, element, &pairs(&srs), untagged, &mut |_| {
                    Member::Inline
                })?;
                self.x.close()
            }
            Shape::MultiLineString(lines) if name.ends_with("Network") => {
                let srs = self.srs();
                self.open_named(&property, &[])?;
                self.x.open("gml:GeometricComplex", &pairs(&srs))?;
                for l in lines {
                    self.x.open("gml:element", &[])?;
                    gml::line(&mut self.x, l)?;
                    self.x.close()?;
                }
                self.x.close()?;
                self.x.close()
            }
            shape => {
                let srs = self.srs();
                self.open_named(&property, &[])?;
                gml::shape(&mut self.x, shape, &pairs(&srs), &mut |_| Member::Inline)?;
                self.x.close()
            }
        }
    }

    /// An ADE element, as the document wrote it.
    pub fn node(&mut self, node: &XmlNode) -> Result<()> {
        if !is_ncname(&node.name.local) {
            self.losses
                .note("ADE elements whose name is not an XML name");
            return Ok(());
        }
        let Some((name, declaration)) = self.name(&node.name) else {
            self.losses.note(format!(
                "ADE elements in a module {} does not have",
                self.version
            ));
            return Ok(());
        };
        let mut attrs: Vec<(&str, &str)> = Vec::new();
        for a in &node.attributes {
            let bound = match a.name.split_once(':') {
                None => is_ncname(&a.name),
                Some((prefix, local)) => {
                    is_ncname(local)
                        && (RESERVED_PREFIXES.contains(&prefix)
                            || self.prefixes.values().any(|p| p == prefix))
                }
            };
            if !bound {
                self.losses.note(
                    "ADE XML attributes not named by an XML name of a namespace the document \
                     declares",
                );
            } else if a.name != "gml:id" || self.claim(&a.value) {
                attrs.push((a.name.as_str(), a.value.as_str()));
            }
        }
        if let Some((k, v)) = &declaration {
            attrs.push((k, v));
        }
        match &node.content {
            XmlContent::Empty => self.x.empty(&name, &attrs),
            XmlContent::Text(t) => self.x.leaf(&name, &attrs, t),
            XmlContent::Children(children) => {
                self.x.open(&name, &attrs)?;
                for c in children {
                    self.node(c)?;
                }
                self.x.close()
            }
        }
    }
}

/// Where an object's geometries, faces and boundary surfaces go.
pub struct Plan {
    /// The property each geometry is written as; `None` for one left out.
    pub property: Vec<Option<String>>,
    /// The LoD each geometry is written at.
    pub lod: Vec<u8>,
    /// Whether each geometry is a solid or solids.
    pub solid: Vec<bool>,
    /// Per geometry, each face's boundary surface, where that surface
    /// writes it.
    pub tags: Vec<Vec<Option<usize>>>,
    /// The property each boundary surface is written in, or `None`.
    pub holder: Vec<Option<&'static str>>,
    /// The surfaces in each surface's openings.
    pub nested: Vec<Vec<usize>>,
    /// The LoDs each boundary surface has faces at.
    pub lods: Vec<BTreeSet<u8>>,
    /// The `gml:id`s of the polygons written.
    pub targets: HashSet<String>,
    /// Ring `gml:id` -> the `gml:id` of its polygon, for the rings written.
    pub rings: HashMap<String, String>,
    /// The `gml:id` of the `gml:TriangulatedSurface` a relief's TIN is
    /// written as.
    pub tin: Option<String>,
}

impl Plan {
    /// Whether surface `s`, or one in its openings, has a face.
    pub fn has_faces(&self, s: usize) -> bool {
        !self.lods[s].is_empty() || self.nested[s].iter().any(|&c| self.has_faces(c))
    }

    /// The faces of surface `s` at LoD `n`, each with whether a solid
    /// refers to it.
    pub fn faces_of<'o>(&self, o: &'o GmlObject, s: usize, n: u8) -> Vec<(&'o Face, bool)> {
        let mut out = Vec::new();
        for (g, geometry) in o.geometries.iter().enumerate() {
            if self.property[g].is_none() || self.lod[g] != n {
                continue;
            }
            for (face, tag) in geometry.shape.faces().zip(&self.tags[g]) {
                if *tag == Some(s) {
                    out.push((face, self.solid[g]));
                }
            }
        }
        out
    }
}

/// Whether a model LoD is a whole number (`2`, `2.0`), which CityGML
/// holds as it is.
fn is_whole(lod: &str) -> bool {
    lod.split_once('.')
        .is_none_or(|(_, minor)| minor.chars().all(|c| c == '0'))
}

/// Where element `name` comes in `order`, the content order of the ADE
/// class or container `class`; after every element it names when it names
/// none.
pub fn rank(order: &[String], class: &QName, name: &QName) -> usize {
    let (Namespace::Other(own), Namespace::Other(uri)) = (&class.ns, &name.ns) else {
        return order.len();
    };
    let key = order_key(own, uri, &name.local);
    order.iter().position(|n| *n == key).unwrap_or(order.len())
}

/// The CityGML class whose content an object of `class` has: itself, or
/// for an ADE class the one `content_of` says it specialises.
pub fn content_class(content_of: &ContentOf, class: &QName) -> Option<QName> {
    match &class.ns {
        Namespace::Other(_) => content_of(class),
        _ => Some(class.clone()),
    }
}

/// The CityGML LoD of a model LoD: its whole number, `0` to `4`.
pub fn lod_number(lod: &str) -> Option<u8> {
    let whole = lod.split('.').next()?;
    whole.parse().ok().filter(|n| *n <= 4)
}

/// What a geometry of `shape` is, for a loss.
pub fn kind(shape: &Shape) -> &'static str {
    match shape {
        Shape::MultiPoint(_) => "MultiPoint",
        Shape::MultiLineString(_) => "MultiLineString",
        Shape::MultiSurface(_) => "MultiSurface",
        Shape::CompositeSurface(_) => "CompositeSurface",
        Shape::Solid(_) => "Solid",
        Shape::CompositeSolid(_) => "CompositeSolid",
        Shape::MultiSolid(_) => "MultiSolid",
        Shape::Instance(_) => "GeometryInstance",
    }
}

/// `geometry` as a relief's TIN, when every face is a triangle.
pub fn tin(geometry: &Geometry) -> Option<()> {
    let surfaces = matches!(
        geometry.shape,
        Shape::MultiSurface(_) | Shape::CompositeSurface(_)
    );
    let mut faces = geometry.shape.faces().peekable();
    (surfaces && faces.peek().is_some() && faces.all(gml::is_triangle)).then_some(())
}

/// Every face of `shape`, mutably, in [`Shape::faces`] order.
pub fn faces_mut(shape: &mut Shape) -> Vec<&mut Face> {
    match shape {
        Shape::MultiSurface(faces) | Shape::CompositeSurface(faces) => faces.iter_mut().collect(),
        Shape::Solid(shells) => shells.iter_mut().flatten().collect(),
        Shape::CompositeSolid(solids) | Shape::MultiSolid(solids) => {
            solids.iter_mut().flatten().flatten().collect()
        }
        Shape::MultiPoint(_) | Shape::MultiLineString(_) | Shape::Instance(_) => Vec::new(),
    }
}

/// The polygon `gml:id`s of `geometries`, and ring `gml:id` -> polygon
/// `gml:id`, for the polygons that have one.
pub fn targets(geometries: &[Geometry]) -> (HashSet<String>, HashMap<String, String>) {
    let mut targets = HashSet::new();
    let mut rings = HashMap::new();
    for face in geometries.iter().flat_map(|g| g.shape.faces()) {
        if let Some(id) = &face.polygon.id {
            targets.insert(id.clone());
            for ring in face.polygon.ring_ids.iter().flatten() {
                rings.insert(ring.clone(), id.clone());
            }
        }
    }
    (targets, rings)
}

/// One `surfaceDataMember`'s content.
pub enum SurfaceData<'a> {
    Material(&'a Material),
    Texture(&'a Texture),
}

/// `appearance` as runs of one theme: its materials and then its textures,
/// each in its own order, a run ending where the theme changes. A reader
/// numbers materials and textures in document order, so a theme's surface
/// data interrupted by another's is not gathered into one run.
pub fn themes<'a>(appearance: &'a Appearance) -> Vec<(&'a str, Vec<SurfaceData<'a>>)> {
    let mut out: Vec<(&'a str, Vec<SurfaceData<'a>>)> = Vec::new();
    let mut add = |theme: &'a str, data| match out.last_mut().filter(|(t, _)| *t == theme) {
        Some((_, list)) => list.push(data),
        None => out.push((theme, vec![data])),
    };
    for m in &appearance.materials {
        add(&m.theme, SurfaceData::Material(m));
    }
    for t in &appearance.textures {
        add(&t.theme, SurfaceData::Texture(t));
    }
    out
}

/// The loss noted for an attribute whose value is blank text, which a
/// reader takes for no value.
pub const BLANK: &str = "attributes whose value is blank text";

/// Whether `member` of a generic attribute set writes an element.
pub fn writes_generic(member: &Attribute) -> bool {
    fn writes(value: &Value) -> bool {
        match value {
            Value::List(items) => items.iter().any(writes),
            Value::Set(members) => members.iter().any(writes_generic),
            v => !blank(v),
        }
    }
    matches!(member.name, AttributeName::Generic(_)) && writes(&member.value)
}

/// Whether `value` is text that is empty or only whitespace.
pub fn blank(value: &Value) -> bool {
    matches!(value, Value::String(s) if s.trim().is_empty())
}

/// A value as the text of its property.
pub fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Integer(i) => i.to_string(),
        Value::Double(d) => number(*d),
        Value::Boolean(b) => b.to_string(),
        Value::List(items) => items.iter().map(text).collect::<Vec<_>>().join(" "),
        Value::Set(_) => String::new(),
    }
}

/// A `gml:Envelope` of `e`.
pub fn envelope<W: Write>(
    x: &mut XmlWriter<W>,
    e: &[f64; 6],
    srs_name: Option<&str>,
) -> Result<()> {
    let mut attrs = Vec::new();
    if let Some(srs) = srs_name {
        attrs.push(("srsName", srs));
    }
    attrs.push(("srsDimension", "3"));
    x.open("gml:Envelope", &attrs)?;
    x.leaf("gml:lowerCorner", &[], &numbers(e[..3].iter().copied()))?;
    x.leaf("gml:upperCorner", &[], &numbers(e[3..].iter().copied()))?;
    x.close()
}

/// Owned attribute pairs, borrowed as the XML writer takes them.
pub fn pairs<K: AsRef<str>, V: AsRef<str>>(attributes: &[(K, V)]) -> Vec<(&str, &str)> {
    attributes
        .iter()
        .map(|(k, v)| (k.as_ref(), v.as_ref()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Material, Texture};

    /// The kind, theme and image or name of each surface data in document
    /// order.
    fn order(appearance: &Appearance) -> Vec<String> {
        themes(appearance)
            .into_iter()
            .flat_map(|(theme, data)| {
                data.into_iter().map(move |d| match d {
                    SurfaceData::Material(m) => format!("{theme} {:?}", m.name),
                    SurfaceData::Texture(t) => format!("{theme} {:?}", t.image_uri),
                })
            })
            .collect()
    }

    #[test]
    fn materials_and_textures_keep_their_order_across_themes() {
        // A reader numbers materials and textures in document order, so
        // grouping a theme's surface data that another theme's interrupts
        // would renumber them on a re-read.
        let material = |theme: &str, name: &str| Material {
            theme: theme.into(),
            name: Some(name.into()),
            ..Material::default()
        };
        let texture = |theme: &str, image: &str| Texture {
            theme: theme.into(),
            image_uri: Some(image.into()),
            ..Texture::default()
        };
        let appearance = Appearance {
            materials: vec![material("summer", "m0"), material("winter", "m1")],
            textures: vec![
                texture("winter", "t0.jpg"),
                texture("summer", "t1.jpg"),
                texture("winter", "t2.jpg"),
            ],
        };
        let order = order(&appearance);
        let materials: Vec<_> = order.iter().filter(|d| d.contains("\"m")).collect();
        let textures: Vec<_> = order.iter().filter(|d| d.contains(".jpg")).collect();
        assert_eq!(materials, ["summer Some(\"m0\")", "winter Some(\"m1\")"]);
        assert_eq!(
            textures,
            [
                "winter Some(\"t0.jpg\")",
                "summer Some(\"t1.jpg\")",
                "winter Some(\"t2.jpg\")"
            ]
        );
    }
}
