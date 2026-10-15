//! The model as a CityGML 2.0 document: one `core:cityObjectMember` per
//! top-level object, each property where its class's content model puts it
//! ([`super::schema`]), so the document reads back as the model it was
//! written from.
//!
//! A boundary surface holds its own polygons. A solid refers by
//! `xlink:href` to those of its faces a boundary surface holds, and holds
//! the rest; a surface geometry holds only the faces no boundary surface
//! holds, and is left out when every face is in one. A relative geometry is
//! written inside the first `ImplicitGeometry` that places it and referred
//! to by the later ones, and the appearance of the relative geometries
//! written follows the objects, as `app:appearanceMember`s. Every `gml:id`
//! is written once: a repeated one is left out where it is repeated.
//!
//! What the model holds that CityGML 2.0 has no place for is left out and
//! counted in [`Losses`]: a geometry at a LoD its object already has one
//! at, or of a kind its class has no property for; the faces of a boundary
//! surface whose class has no geometry at their LoD (kept in the object's
//! geometry, untagged); an attribute naming no property of its class; and a
//! child its parent's class cannot hold, written as a top-level object of
//! its own instead.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write;

use citygml_core::convert::{Losses, PropertyKind};
use citygml_core::model::{
    Address, AdeLayout, Appearance, Attribute, AttributeName, BoundarySurface, Face, Geometry,
    GmlObject, GroupMember, Instance, Material, MemberTarget, Module, Namespace, QName, RingUvs,
    Shape, Template, Texture, Value, XmlContent, XmlNode,
};
use citygml_core::vocabulary::module_prefix;
use citygml_core::write::gml::{self, Member};
use citygml_core::write::{XmlWriter, number, numbers, xal};
use citygml_core::{Error, Result};

use super::schema::{COMMON, Hook, Slot, class_def, surface_def};
use crate::ns;

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

/// A CityGML 2.0 document being written.
pub struct DocumentWriter<W: Write> {
    x: XmlWriter<W>,
    /// The CRS, until an element has stated it.
    srs_name: Option<String>,
    templates: Vec<Template>,
    /// Whether each template has been written.
    written: Vec<bool>,
    /// Namespace URI -> its prefix, for the namespaces outside CityGML and
    /// GML the root declares.
    prefixes: HashMap<String, String>,
    content_of: ContentOf,
    layout: AdeLayout,
    /// Every `gml:id` written.
    ids: HashSet<String>,
    /// For the `gml:id`s this writer makes up.
    next_id: usize,
    /// Top-level objects written.
    objects: usize,
    losses: Losses,
}

/// The CityGML 2.0 modules, as the root declares them, with their schemas.
const SCHEMAS: &[(Module, &str)] = &[
    (
        Module::Core,
        "http://schemas.opengis.net/citygml/2.0/cityGMLBase.xsd",
    ),
    (
        Module::Appearance,
        "http://schemas.opengis.net/citygml/appearance/2.0/appearance.xsd",
    ),
    (
        Module::Bridge,
        "http://schemas.opengis.net/citygml/bridge/2.0/bridge.xsd",
    ),
    (
        Module::Building,
        "http://schemas.opengis.net/citygml/building/2.0/building.xsd",
    ),
    (
        Module::CityFurniture,
        "http://schemas.opengis.net/citygml/cityfurniture/2.0/cityFurniture.xsd",
    ),
    (
        Module::CityObjectGroup,
        "http://schemas.opengis.net/citygml/cityobjectgroup/2.0/cityObjectGroup.xsd",
    ),
    (
        Module::Generics,
        "http://schemas.opengis.net/citygml/generics/2.0/generics.xsd",
    ),
    (
        Module::LandUse,
        "http://schemas.opengis.net/citygml/landuse/2.0/landUse.xsd",
    ),
    (
        Module::Relief,
        "http://schemas.opengis.net/citygml/relief/2.0/relief.xsd",
    ),
    (
        Module::Transportation,
        "http://schemas.opengis.net/citygml/transportation/2.0/transportation.xsd",
    ),
    (
        Module::Tunnel,
        "http://schemas.opengis.net/citygml/tunnel/2.0/tunnel.xsd",
    ),
    (
        Module::Vegetation,
        "http://schemas.opengis.net/citygml/vegetation/2.0/vegetation.xsd",
    ),
    (
        Module::WaterBody,
        "http://schemas.opengis.net/citygml/waterbody/2.0/waterBody.xsd",
    ),
];

const NS_GML: &str = "http://www.opengis.net/gml";
const NS_XLINK: &str = "http://www.w3.org/1999/xlink";
const NS_XAL: &str = "urn:oasis:names:tc:ciq:xsdschema:xAL:2.0";
const NS_XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// The prefixes the root binds besides the modules' and the ADEs': what an
/// ADE's prefix must not be.
pub const RESERVED_PREFIXES: &[&str] = &["gml", "xlink", "xAL", "xsi", "xml", "ns"];

/// The prefix an element of a namespace the root does not declare binds on
/// itself.
const LOCAL_PREFIX: &str = "ns";

impl<W: Write> DocumentWriter<W> {
    /// Start a document on `out`: its root, declaring every CityGML 2.0
    /// module and each ADE of `head`, and the model's envelope. `templates`
    /// are the relative geometries the objects' `ImplicitGeometry`s place,
    /// by position; `content_of` gives an ADE class's CityGML class.
    pub fn new(
        out: W,
        head: Head,
        templates: Vec<Template>,
        content_of: ContentOf,
    ) -> Result<Self> {
        let mut x = XmlWriter::new(out)?;
        let mut declarations: Vec<(String, String)> = Vec::new();
        let mut locations = Vec::new();
        for (module, schema) in SCHEMAS {
            let uri = ns::uri(*module).expect("a CityGML 2.0 module");
            declarations.push((format!("xmlns:{}", module_prefix(*module)), uri.into()));
            locations.push(format!("{uri} {schema}"));
        }
        for (prefix, uri) in [
            ("gml", NS_GML),
            ("xlink", NS_XLINK),
            ("xAL", NS_XAL),
            ("xsi", NS_XSI),
        ] {
            declarations.push((format!("xmlns:{prefix}"), uri.into()));
        }
        let mut prefixes = HashMap::from([
            (NS_XLINK.to_string(), "xlink".to_string()),
            (NS_XAL.to_string(), "xAL".to_string()),
        ]);
        for ade in &head.ades {
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
            srs_name,
            written: vec![false; templates.len()],
            templates,
            prefixes,
            content_of,
            layout: head.layout,
            ids: HashSet::new(),
            next_id: 0,
            objects: 0,
            losses: Losses::default(),
        })
    }

    /// Write `object` as a `core:cityObjectMember`, followed by any of its
    /// descendants their parent's class cannot hold.
    pub fn write(&mut self, object: GmlObject) -> Result<()> {
        let mut pending = vec![object];
        while let Some(o) = pending.pop() {
            if !self.writable(&o) {
                continue;
            }
            self.x.open("core:cityObjectMember", &[])?;
            let mut hoisted = Vec::new();
            self.object(o, &mut hoisted)?;
            self.x.close()?;
            self.objects += 1;
            pending.extend(hoisted.into_iter().rev());
        }
        Ok(())
    }

    /// How many top-level objects have been written.
    pub fn objects(&self) -> usize {
        self.objects
    }

    /// End the document: the appearance of the relative geometries written,
    /// then the root. Gives back what it was written to, and what was left
    /// out.
    pub fn finish(mut self) -> Result<(W, Losses)> {
        let templates = std::mem::take(&mut self.templates);
        let written = std::mem::take(&mut self.written);
        for (template, _) in templates.iter().zip(written).filter(|(_, w)| *w) {
            let (targets, rings) = targets(std::slice::from_ref(&template.geometry));
            self.appearance(
                "app:appearanceMember",
                &template.appearance,
                &targets,
                &rings,
            )?;
        }
        self.x.close()?;
        Ok((self.x.finish()?, self.losses))
    }

    /// Whether `o` can be written at all; counts it when not.
    fn writable(&mut self, o: &GmlObject) -> bool {
        let class = self.content_class(&o.class);
        let def = class.as_ref().and_then(class_def);
        if def.is_none() && !matches!(o.class.ns, Namespace::Other(_)) {
            self.losses.note(format!(
                "objects of a class CityGML 2.0 does not have ({})",
                o.class.local
            ));
            return false;
        }
        if def.is_some_and(|d| d.content.contains(&Slot::Tin))
            && !o.geometries.iter().any(|g| tin(g).is_some())
        {
            self.losses
                .note("TIN reliefs without a surface of triangles");
            return false;
        }
        true
    }

    /// The CityGML class whose content `class` has: itself, or for an ADE
    /// class the one it specialises.
    fn content_class(&self, class: &QName) -> Option<QName> {
        match &class.ns {
            Namespace::Other(_) => (self.content_of)(class),
            _ => Some(class.clone()),
        }
    }

    /// `id`, when no element of the document has it yet; counted when one
    /// has.
    fn claim(&mut self, id: &str) -> bool {
        if self.ids.insert(id.to_string()) {
            return true;
        }
        self.losses
            .note("gml:ids that are not unique in the document (left out where repeated)");
        false
    }

    /// A `gml:id` no element of the document has.
    fn fresh_id(&mut self) -> String {
        loop {
            self.next_id += 1;
            let id = format!("_w{}", self.next_id);
            if self.ids.insert(id.clone()) {
                return id;
            }
        }
    }

    /// The `srsName` attribute, on the first element that states the CRS.
    fn srs(&mut self) -> Vec<(&'static str, String)> {
        self.srs_name
            .take()
            .map(|s| ("srsName", s))
            .into_iter()
            .collect()
    }

    /// The prefixed name of `q`, and the namespace declaration its element
    /// needs when the root has none for it; `None` for a name in a module
    /// CityGML 2.0 does not have.
    fn name(&self, q: &QName) -> Option<(String, Option<(String, String)>)> {
        Some(match &q.ns {
            Namespace::Module(m) => {
                ns::uri(*m)?;
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
    fn element(&self, q: &QName) -> Result<String> {
        match self.name(q) {
            Some((name, None)) => Ok(name),
            _ => Err(Error::invalid(format!(
                "no CityGML 2.0 element can be named {}",
                q.local
            ))),
        }
    }

    /// Open `q` with `attributes`, declaring its namespace when the root
    /// does not.
    fn open_named(&mut self, q: &QName, attributes: &[(&str, &str)]) -> Result<()> {
        let Some((name, declaration)) = self.name(q) else {
            return Err(Error::invalid(format!(
                "no CityGML 2.0 element can be named {}",
                q.local
            )));
        };
        let mut attrs = attributes.to_vec();
        if let Some((k, v)) = &declaration {
            attrs.push((k, v));
        }
        self.x.open(&name, &attrs)
    }

    /// One object, its element named by its class.
    fn object(&mut self, mut o: GmlObject, hoisted: &mut Vec<GmlObject>) -> Result<()> {
        let content = self.content_class(&o.class);
        let def = content.as_ref().and_then(class_def);
        let slots: &'static [Slot] = def.map_or(&[], |d| d.content);
        // The namespace of the class's own properties.
        let module = def.map_or_else(|| o.class.ns.clone(), |d| Namespace::Module(d.module));
        let ade_class = def.is_none();
        let plan = self.plan(&mut o, slots, ade_class);
        let mut children: Vec<Option<GmlObject>> = std::mem::take(&mut o.children)
            .into_iter()
            .map(Some)
            .collect();

        let id = o.id.clone().filter(|id| self.claim(id));
        let attrs: Vec<(&str, &str)> = id.iter().map(|id| ("gml:id", id.as_str())).collect();
        self.open_named(&o.class, &attrs)?;
        let mut used = vec![false; o.attributes.len()];
        for (a, used) in o.attributes.iter().zip(&mut used) {
            *used = implied(&o.class, a);
        }
        self.common(&o.attributes, &mut used, o.extent.as_ref())?;
        self.appearance("app:appearance", &o.appearance, &plan.targets, &plan.rings)?;
        self.generic(&o.attributes, &mut used)?;
        let hooks: &[Hook] = def.map_or(&[], |d| d.hooks);
        let places: Vec<Place> = o.ade.iter().map(|n| self.place(n, hooks)).collect();
        let ade = |place: Place| {
            o.ade
                .iter()
                .zip(&places)
                .filter(move |(_, p)| **p == place)
                .map(|(n, _)| n)
        };
        for node in ade(Place::CityObject) {
            self.node(node)?;
        }

        if ade_class {
            self.ade_content(&o, &module, &plan, &places)?;
        }
        for (at, slot) in slots.iter().enumerate() {
            for (h, _) in hooks.iter().enumerate().filter(|(_, h)| h.at == at) {
                for node in ade(Place::Hook(h)) {
                    self.node(node)?;
                }
            }
            match *slot {
                Slot::Value(name, kind, _) => {
                    let q = QName::new(module.clone(), name);
                    self.values(&o.attributes, &mut used, &q, kind)?;
                }
                Slot::Geometry(name) => {
                    if let Some(g) = plan
                        .property
                        .iter()
                        .position(|p| p.as_deref() == Some(name))
                    {
                        self.geometry(&module, name, &o.geometries[g], g, &plan)?;
                    }
                }
                Slot::Surfaces(holder) => self.boundary_surfaces(&o, &module, holder, &plan)?,
                Slot::Nest(property, classes) => {
                    for child in children.iter_mut() {
                        if !child
                            .as_ref()
                            .is_some_and(|c| self.nests(c, &module, classes))
                        {
                            continue;
                        }
                        let child = child.take().expect("checked above");
                        self.open_named(&QName::new(module.clone(), property), &[])?;
                        self.object(child, hoisted)?;
                        self.x.close()?;
                    }
                }
                Slot::Address(property) => {
                    for a in &o.addresses {
                        self.open_named(&QName::new(module.clone(), property), &[])?;
                        self.address(a)?;
                        self.x.close()?;
                    }
                }
                Slot::Member => {
                    for m in &o.members {
                        self.member(m, &mut children, hoisted)?;
                    }
                }
                Slot::ReliefLod => {
                    let lod = plan
                        .property
                        .iter()
                        .position(|p| p.as_deref() == Some("tin"))
                        .map_or(1, |g| plan.lod[g]);
                    self.x.leaf("dem:lod", &[], &lod.to_string())?;
                }
                Slot::Tin => {
                    if let Some(g) = plan
                        .property
                        .iter()
                        .position(|p| p.as_deref() == Some("tin"))
                    {
                        let faces: Vec<&Face> = o.geometries[g].shape.faces().collect();
                        let srs = self.srs();
                        self.x.open("dem:tin", &[])?;
                        gml::triangulated(&mut self.x, &faces, &pairs(&srs))?;
                        self.x.close()?;
                    }
                }
                Slot::Opening => {}
            }
        }
        for (h, _) in hooks
            .iter()
            .enumerate()
            .filter(|(_, h)| h.at == slots.len())
        {
            for node in ade(Place::Hook(h)) {
                self.node(node)?;
            }
        }
        if !ade_class {
            for node in ade(Place::End) {
                self.node(node)?;
            }
        }
        self.x.close()?;

        if !o.addresses.is_empty() && !slots.iter().any(|s| matches!(s, Slot::Address(_))) {
            self.losses
                .note("addresses of objects whose class has none");
        }
        for (a, used) in o.attributes.iter().zip(&used) {
            if !used {
                self.losses.note(format!(
                    "attributes that name no property of their class ({})",
                    a.name.local()
                ));
            }
        }
        for child in children.into_iter().flatten() {
            self.losses
                .note("objects their parent's class cannot hold (written on their own)");
            hoisted.push(child);
        }
        Ok(())
    }

    /// Where ADE element `node` goes in the content of an object whose class
    /// has `hooks`.
    fn place(&self, node: &XmlNode, hooks: &[Hook]) -> Place {
        let Some(hook) = self.layout.hook(&node.name) else {
            return Place::End;
        };
        if hook.ns == Namespace::Module(Module::Core)
            && hook.local == "_GenericApplicationPropertyOfCityObject"
        {
            return Place::CityObject;
        }
        hooks
            .iter()
            .position(|h| Namespace::Module(h.module) == hook.ns && h.element() == hook.local)
            .map_or(Place::End, Place::Hook)
    }

    /// The content of an object of an ADE class CityGML has no content
    /// for: its geometry and boundary surfaces in its own namespace, and its
    /// ADE elements, in the order its schema declares them where that is
    /// known, and else in that order.
    fn ade_content(
        &mut self,
        o: &GmlObject,
        module: &Namespace,
        plan: &Plan,
        places: &[Place],
    ) -> Result<()> {
        enum Item {
            Geometry(usize),
            Surfaces,
            Node(usize),
        }
        let order = self
            .layout
            .content(&o.class)
            .map(<[String]>::to_vec)
            .unwrap_or_default();
        let rank = |name: &str| order.iter().position(|n| n == name).unwrap_or(order.len());
        let mut items: Vec<(usize, Item)> = Vec::new();
        for (g, property) in plan.property.iter().enumerate() {
            if let Some(p) = property {
                items.push((rank(p), Item::Geometry(g)));
            }
        }
        items.push((rank("boundedBy"), Item::Surfaces));
        for (i, (node, place)) in o.ade.iter().zip(places).enumerate() {
            if *place == Place::End {
                let r = if node.name.ns == *module {
                    rank(&node.name.local)
                } else {
                    order.len()
                };
                items.push((r, Item::Node(i)));
            }
        }
        items.sort_by_key(|(r, _)| *r);
        for (_, item) in items {
            match item {
                Item::Geometry(g) => {
                    let property = plan.property[g].clone().expect("planned above");
                    self.geometry(module, &property, &o.geometries[g], g, plan)?;
                }
                Item::Surfaces => self.boundary_surfaces(o, module, "boundedBy", plan)?,
                Item::Node(i) => self.node(&o.ade[i])?,
            }
        }
        Ok(())
    }

    /// Whether `child` is of one of `classes` of `module`, or of an ADE
    /// class that specialises one.
    fn nests(&self, child: &GmlObject, module: &Namespace, classes: &[&str]) -> bool {
        self.content_class(&child.class)
            .is_some_and(|c| c.ns == *module && classes.contains(&c.local.as_str()))
    }

    /// Where each geometry, face and boundary surface of `o` goes; makes
    /// `o`'s ids unique in the document on the way.
    fn plan(&mut self, o: &mut GmlObject, slots: &[Slot], ade_class: bool) -> Plan {
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
        };
        let has = |name: &str| {
            ade_class
                || slots
                    .iter()
                    .any(|s| matches!(s, Slot::Geometry(n) if *n == name))
        };
        let tin_class = slots.contains(&Slot::Tin);
        let group = slots.contains(&Slot::Member);
        let mut lods: Vec<u8> = Vec::new();
        for (g, geometry) in o.geometries.iter().enumerate() {
            let Some(n) = lod_number(&geometry.lod) else {
                self.losses
                    .note("geometries at a LoD CityGML 2.0 does not have");
                continue;
            };
            if geometry.lod != n.to_string() {
                self.losses
                    .note("geometries whose LoD is written as its whole number");
            }
            let shape = &geometry.shape;
            let candidates = if group {
                vec!["geometry".to_string()]
            } else if tin_class {
                tin(geometry)
                    .map(|_| "tin".to_string())
                    .into_iter()
                    .collect()
            } else {
                candidates(shape, n)
            };
            let taken = plan
                .property
                .iter()
                .flatten()
                .any(|p| candidates.contains(p));
            let chosen = candidates
                .into_iter()
                .find(|c| has(c) || (group && c == "geometry") || (tin_class && c == "tin"));
            let Some(chosen) = chosen else {
                self.losses.note(format!(
                    "{} geometries at a LoD their class has no {} property for",
                    kind(shape),
                    kind(shape)
                ));
                continue;
            };
            if lods.contains(&n) || taken || (group && plan.property.iter().any(Option::is_some)) {
                self.losses
                    .note("geometries at a LoD their object already has one at");
                continue;
            }
            lods.push(n);
            plan.lod[g] = n;
            plan.solid[g] = matches!(
                shape,
                Shape::Solid(_) | Shape::CompositeSolid(_) | Shape::MultiSolid(_)
            );
            plan.property[g] = Some(chosen);
        }

        // Which surfaces are written, and in which property.
        for s in 0..o.boundary_surfaces.len() {
            plan.holder[s] = surface_holder(&o.boundary_surfaces, s, slots, ade_class);
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
                    plan.holder[s].is_some() && surface_holds_lod(&o.boundary_surfaces[s].class, n)
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

    /// `gml:description`, `gml:name`, the envelope and `core:`'s
    /// properties, in that order.
    fn common(
        &mut self,
        attributes: &[Attribute],
        used: &mut [bool],
        extent: Option<&[f64; 6]>,
    ) -> Result<()> {
        for p in COMMON {
            if p.name == "creationDate"
                && let Some(e) = extent
            {
                let srs = self.srs_name.take();
                self.x.open("gml:boundedBy", &[])?;
                envelope(&mut self.x, e, srs.as_deref())?;
                self.x.close()?;
            }
            self.values(attributes, used, &p.qname(), p.kind)?;
        }
        Ok(())
    }

    /// Every attribute named `q`, as a property holding a value of `kind`.
    fn values(
        &mut self,
        attributes: &[Attribute],
        used: &mut [bool],
        q: &QName,
        kind: PropertyKind,
    ) -> Result<()> {
        for (a, used) in attributes.iter().zip(used.iter_mut()) {
            if *used || a.name != AttributeName::Property(q.clone()) {
                continue;
            }
            *used = true;
            if blank(&a.value) {
                self.losses.note(BLANK);
                continue;
            }
            let element = self.element(q)?;
            let uom = matches!(
                kind,
                PropertyKind::Number { measure: true } | PropertyKind::MeasureList
            );
            let attrs: &[(&str, &str)] = if uom { &[("uom", "m")] } else { &[] };
            self.x.leaf(&element, attrs, &text(&a.value))?;
        }
        Ok(())
    }

    /// The generic attributes, as `gen:` elements.
    fn generic(&mut self, attributes: &[Attribute], used: &mut [bool]) -> Result<()> {
        for (a, used) in attributes.iter().zip(used.iter_mut()) {
            if let AttributeName::Generic(name) = &a.name {
                *used = true;
                self.generic_attribute(name, &a.value)?;
            }
        }
        Ok(())
    }

    fn generic_attribute(&mut self, name: &str, value: &Value) -> Result<()> {
        if blank(value) {
            self.losses.note(BLANK);
            return Ok(());
        }
        let element = match value {
            Value::Integer(_) => "gen:intAttribute",
            Value::Double(_) => "gen:doubleAttribute",
            Value::String(_) => "gen:stringAttribute",
            Value::Boolean(_) => {
                self.losses
                    .note("generic attributes whose value is a boolean (written as text)");
                "gen:stringAttribute"
            }
            Value::List(items) => {
                for item in items {
                    self.generic_attribute(name, item)?;
                }
                return Ok(());
            }
            Value::Set(members) => {
                self.x.open("gen:genericAttributeSet", &[("name", name)])?;
                for m in members {
                    match &m.name {
                        AttributeName::Generic(n) => self.generic_attribute(n, &m.value)?,
                        AttributeName::Property(_) => {
                            self.losses.note("properties inside generic attribute sets")
                        }
                    }
                }
                return self.x.close();
            }
        };
        self.x.open(element, &[("name", name)])?;
        self.x.leaf("gen:value", &[], &text(value))?;
        self.x.close()
    }

    /// Geometry `g`, `geometry`, as property `name` of `module`: the whole
    /// of a solid, referring to the faces a boundary surface holds, or the
    /// faces of a surface geometry no boundary surface holds.
    fn geometry(
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
        if let Shape::Instance(instance) = shape {
            self.open_named(&property, &[])?;
            self.implicit(instance)?;
            return self.x.close();
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

    /// A `core:ImplicitGeometry`: the relative geometry it places, inline
    /// the first time, else by reference.
    fn implicit(&mut self, instance: &Instance) -> Result<()> {
        let t = instance.template;
        let Some(template) = self.templates.get(t) else {
            return Err(Error::invalid(format!(
                "an implicit geometry places template {t}, which the document does not have"
            )));
        };
        let id = template.id.clone().unwrap_or_else(|| format!("_t{t}"));
        self.x.open("core:ImplicitGeometry", &[])?;
        self.x
            .leaf("core:transformationMatrix", &[], &numbers(instance.matrix))?;
        if self.written[t] {
            let href = format!("#{id}");
            self.x
                .empty("core:relativeGMLGeometry", &[("xlink:href", &href)])?;
        } else {
            self.written[t] = true;
            let shape = template.geometry.shape.clone();
            let id = Some(id).filter(|id| self.claim(id));
            let attrs: Vec<(&str, &str)> = id.iter().map(|id| ("gml:id", id.as_str())).collect();
            self.x.open("core:relativeGMLGeometry", &[])?;
            gml::shape(&mut self.x, &shape, &attrs, &mut |_| Member::Inline)?;
            self.x.close()?;
        }
        let srs = self.srs();
        self.x.open("core:referencePoint", &[])?;
        gml::point(&mut self.x, &instance.reference_point, &pairs(&srs))?;
        self.x.close()?;
        self.x.close()
    }

    /// The boundary surfaces of `o` that its property `holder` holds.
    fn boundary_surfaces(
        &mut self,
        o: &GmlObject,
        module: &Namespace,
        holder: &str,
        plan: &Plan,
    ) -> Result<()> {
        for s in 0..o.boundary_surfaces.len() {
            if o.boundary_surfaces[s].parent.is_none()
                && plan.holder[s] == Some(holder)
                && plan.has_faces(s)
            {
                self.boundary_surface(o, s, module, holder, plan)?;
            }
        }
        Ok(())
    }

    /// Boundary surface `s` of `o` in property `holder` of `module`, with
    /// the surfaces in its openings.
    fn boundary_surface(
        &mut self,
        o: &GmlObject,
        s: usize,
        module: &Namespace,
        holder: &str,
        plan: &Plan,
    ) -> Result<()> {
        let surface = &o.boundary_surfaces[s];
        self.open_named(&QName::new(module.clone(), holder), &[])?;
        self.open_named(&surface.class, &[])?;
        let mut used = vec![false; surface.attributes.len()];
        self.common(&surface.attributes, &mut used, None)?;
        self.generic(&surface.attributes, &mut used)?;
        // An ADE's surface takes its geometry in its owner's namespace.
        let surface_ns = match &surface.class.ns {
            Namespace::Other(_) => module.clone(),
            ns => ns.clone(),
        };
        let content: &[Slot] = surface_def(&surface.class).map_or(ADE_SURFACE, |(_, c)| c);
        for slot in content {
            match *slot {
                Slot::Value(name, kind, _) => {
                    let q = QName::new(surface_ns.clone(), name);
                    self.values(&surface.attributes, &mut used, &q, kind)?;
                }
                Slot::Geometry(name) => {
                    let Some(n) = surface_geometry_lod(name) else {
                        continue;
                    };
                    if !plan.lods[s].contains(&n) {
                        continue;
                    }
                    let faces = plan.faces_of(o, s, n);
                    let srs = self.srs();
                    self.open_named(&QName::new(surface_ns.clone(), name), &[])?;
                    if name.ends_with("MultiSurface") {
                        self.x.open("gml:MultiSurface", &pairs(&srs))?;
                        for (face, solid) in faces {
                            self.surface_member(face, solid)?;
                        }
                        self.x.close()?;
                    } else if let [(face, solid)] = faces.as_slice() {
                        if *solid {
                            gml::polygon(&mut self.x, &face.polygon, &pairs(&srs))?;
                        } else {
                            gml::surface(&mut self.x, face, &pairs(&srs))?;
                        }
                    } else {
                        self.x.open("gml:CompositeSurface", &pairs(&srs))?;
                        for (face, solid) in faces {
                            self.surface_member(face, solid)?;
                        }
                        self.x.close()?;
                    }
                    self.x.close()?;
                }
                Slot::Opening => {
                    for &c in &plan.nested[s] {
                        if plan.has_faces(c) {
                            self.boundary_surface(o, c, &surface_ns, "opening", plan)?;
                        }
                    }
                }
                _ => {}
            }
        }
        self.x.close()?;
        self.x.close()?;
        for (a, used) in surface.attributes.iter().zip(&used) {
            if !used {
                self.losses.note(format!(
                    "attributes that name no property of their class ({})",
                    a.name.local()
                ));
            }
        }
        Ok(())
    }

    /// A face in a boundary surface's geometry: the polygon a solid refers
    /// to as it is, any other face as its geometry uses it.
    fn surface_member(&mut self, face: &Face, solid: bool) -> Result<()> {
        if solid {
            self.x.open("gml:surfaceMember", &[])?;
            gml::polygon(&mut self.x, &face.polygon, &[])?;
            self.x.close()
        } else {
            gml::member(&mut self.x, "gml:surfaceMember", face, Member::Inline)
        }
    }

    /// A group member: a child the group holds, or a reference.
    fn member(
        &mut self,
        m: &GroupMember,
        children: &mut [Option<GmlObject>],
        hoisted: &mut Vec<GmlObject>,
    ) -> Result<()> {
        let mut attrs: Vec<(&str, String)> = Vec::new();
        match &m.target {
            MemberTarget::Child(k) => {
                let Some(child) = children.get_mut(*k).and_then(Option::take) else {
                    return Ok(());
                };
                if let Some(role) = &m.role {
                    attrs.push(("role", role.clone()));
                }
                self.x.open("grp:groupMember", &pairs(&attrs))?;
                self.object(child, hoisted)?;
                self.x.close()
            }
            MemberTarget::Id(id) => {
                attrs.push(("xlink:href", format!("#{id}")));
                if let Some(role) = &m.role {
                    attrs.push(("role", role.clone()));
                }
                self.x.empty("grp:groupMember", &pairs(&attrs))
            }
        }
    }

    /// A `core:Address`.
    fn address(&mut self, a: &Address) -> Result<()> {
        self.x.open("core:Address", &[])?;
        self.x.open("core:xalAddress", &[])?;
        for leaf in xal::address_details(&mut self.x, a)? {
            self.losses
                .note(format!("address members xAL has no place for ({leaf})"));
        }
        self.x.close()?;
        if !a.location.is_empty() {
            let srs = self.srs();
            self.x.open("core:multiPoint", &[])?;
            gml::shape(
                &mut self.x,
                &Shape::MultiPoint(a.location.clone()),
                &pairs(&srs),
                &mut |_| Member::Inline,
            )?;
            self.x.close()?;
        }
        self.x.close()
    }

    /// An ADE element, as the document wrote it.
    fn node(&mut self, node: &XmlNode) -> Result<()> {
        let Some((name, declaration)) = self.name(&node.name) else {
            self.losses
                .note("ADE elements in a module CityGML 2.0 does not have");
            return Ok(());
        };
        let mut attrs: Vec<(&str, &str)> = Vec::new();
        for a in &node.attributes {
            let bound = match a.name.split_once(':') {
                None => true,
                Some((prefix, _)) => {
                    RESERVED_PREFIXES.contains(&prefix)
                        || self.prefixes.values().any(|p| p == prefix)
                }
            };
            if bound {
                attrs.push((a.name.as_str(), a.value.as_str()));
            } else {
                self.losses
                    .note("ADE XML attributes of a namespace the document does not declare");
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

    /// `appearance`, one `app:Appearance` per theme in property `element`:
    /// each material targeting the polygons of `targets` it names, each
    /// texture the rings `rings` gives the polygon of.
    fn appearance(
        &mut self,
        element: &str,
        appearance: &Appearance,
        targets: &HashSet<String>,
        rings: &HashMap<String, String>,
    ) -> Result<()> {
        for (theme, data) in themes(appearance) {
            self.x.open(element, &[])?;
            self.x.open("app:Appearance", &[])?;
            if !theme.is_empty() {
                self.x.leaf("app:theme", &[], theme)?;
            }
            for d in data {
                match d {
                    SurfaceData::Material(m) => self.material(m, targets)?,
                    SurfaceData::Texture(t) => self.texture(t, rings)?,
                }
            }
            self.x.close()?;
            self.x.close()?;
        }
        Ok(())
    }

    fn material(&mut self, m: &Material, targets: &HashSet<String>) -> Result<()> {
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
        self.x.open("app:surfaceDataMember", &[])?;
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

    fn texture(&mut self, t: &Texture, rings: &HashMap<String, String>) -> Result<()> {
        // The rings of each polygon, in the order first met.
        let mut by_polygon: Vec<(&str, Vec<&RingUvs>)> = Vec::new();
        for ring in &t.rings {
            let Some(polygon) = rings.get(&ring.0) else {
                self.losses
                    .note("texture coordinates of rings the document does not write");
                continue;
            };
            match by_polygon.iter_mut().find(|(p, _)| *p == polygon.as_str()) {
                Some((_, rings)) => rings.push(ring),
                None => by_polygon.push((polygon, vec![ring])),
            }
        }
        if by_polygon.is_empty() {
            return Ok(());
        }
        self.x.open("app:surfaceDataMember", &[])?;
        self.x.open("app:ParameterizedTexture", &[])?;
        let border = t.border_color.as_ref().map(|c| numbers(c.iter().copied()));
        for (element, value) in [
            ("app:imageURI", t.image_uri.as_deref()),
            ("app:mimeType", t.mime_type.as_deref()),
            ("app:textureType", t.texture_type.as_deref()),
            ("app:wrapMode", t.wrap_mode.as_deref()),
            ("app:borderColor", border.as_deref()),
        ] {
            if let Some(value) = value {
                self.x.leaf(element, &[], value)?;
            }
        }
        for (polygon, rings) in by_polygon {
            let uri = format!("#{polygon}");
            self.x.open("app:target", &[("uri", &uri)])?;
            self.x.open("app:TexCoordList", &[])?;
            for (ring, uvs) in rings {
                let closed = uvs.iter().chain(uvs.first());
                let r = format!("#{ring}");
                self.x.leaf(
                    "app:textureCoordinates",
                    &[("ring", &r)],
                    &numbers(closed.flat_map(|uv| uv.iter().copied())),
                )?;
            }
            self.x.close()?;
            self.x.close()?;
        }
        self.x.close()?;
        self.x.close()
    }
}

/// Where an ADE element goes in its object's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// With the generic attributes, at `_CityObject`'s hook.
    CityObject,
    /// At the hook of that position in its class's hooks.
    Hook(usize),
    /// At the end, or where an ADE class's schema puts it.
    End,
}

/// Where an object's geometries, faces and boundary surfaces go.
struct Plan {
    /// The property each geometry is written as; `None` for one left out.
    property: Vec<Option<String>>,
    /// The LoD each geometry is written at.
    lod: Vec<u8>,
    /// Whether each geometry is a solid or solids.
    solid: Vec<bool>,
    /// Per geometry, each face's boundary surface, where that surface
    /// writes it.
    tags: Vec<Vec<Option<usize>>>,
    /// The property each boundary surface is written in, or `None`.
    holder: Vec<Option<&'static str>>,
    /// The surfaces in each surface's openings.
    nested: Vec<Vec<usize>>,
    /// The LoDs each boundary surface has faces at.
    lods: Vec<BTreeSet<u8>>,
    /// The `gml:id`s of the polygons written.
    targets: HashSet<String>,
    /// Ring `gml:id` -> the `gml:id` of its polygon, for the rings written.
    rings: HashMap<String, String>,
}

impl Plan {
    /// Whether surface `s`, or one in its openings, has a face.
    fn has_faces(&self, s: usize) -> bool {
        !self.lods[s].is_empty() || self.nested[s].iter().any(|&c| self.has_faces(c))
    }

    /// The faces of surface `s` at LoD `n`, each with whether a solid
    /// refers to it.
    fn faces_of<'o>(&self, o: &'o GmlObject, s: usize, n: u8) -> Vec<(&'o Face, bool)> {
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

/// The content of an ADE's boundary surface, which substitutes for a
/// module's: its geometry at LoDs 2 to 4.
const ADE_SURFACE: &[Slot] = &[
    Slot::Geometry("lod2MultiSurface"),
    Slot::Geometry("lod3MultiSurface"),
    Slot::Geometry("lod4MultiSurface"),
];

/// The property surface `s` is written in: its owner's, or `opening` for a
/// door or window in the opening of a surface written. `None` when neither
/// can hold it.
fn surface_holder(
    surfaces: &[BoundarySurface],
    s: usize,
    slots: &[Slot],
    ade_class: bool,
) -> Option<&'static str> {
    let surface = &surfaces[s];
    let holder: &'static str = match &surface.class.ns {
        Namespace::Other(_) => "boundedBy",
        _ if ade_class => return None,
        _ => surface_def(&surface.class)?.0,
    };
    match surface.parent {
        None if ade_class => Some(holder),
        None => slots.contains(&Slot::Surfaces(holder)).then_some(holder),
        Some(p) => {
            let (_, content) = surface_def(&surfaces.get(p)?.class)?;
            (holder == "opening" && content.contains(&Slot::Opening))
                .then(|| surface_holder(surfaces, p, slots, ade_class))
                .flatten()
                .map(|_| holder)
        }
    }
}

/// Whether a boundary surface of `class` has geometry at LoD `n`.
fn surface_holds_lod(class: &QName, n: u8) -> bool {
    let content = surface_def(class).map_or(ADE_SURFACE, |(_, c)| c);
    content.iter().any(|s| match s {
        Slot::Geometry(name) => surface_geometry_lod(name) == Some(n),
        _ => false,
    })
}

/// The LoD of a boundary surface's surface geometry property
/// (`lod3MultiSurface`, `lod2Surface`); `None` for another property.
fn surface_geometry_lod(name: &str) -> Option<u8> {
    let rest = name.strip_prefix("lod")?;
    let (digit, kind) = rest.split_at(1);
    matches!(kind, "MultiSurface" | "Surface")
        .then(|| digit.parse().ok())
        .flatten()
}

/// The CityGML LoD of a model LoD: its whole number, `0` to `4`.
fn lod_number(lod: &str) -> Option<u8> {
    let whole = lod.split('.').next()?;
    whole.parse().ok().filter(|n| *n <= 4)
}

/// The properties a geometry of `shape` at LoD `n` may be written as, best
/// first.
fn candidates(shape: &Shape, n: u8) -> Vec<String> {
    let lod = |kind: &str| format!("lod{n}{kind}");
    match shape {
        Shape::Instance(_) => vec![lod("ImplicitRepresentation")],
        Shape::Solid(_) | Shape::CompositeSolid(_) => vec![lod("Solid"), lod("Geometry")],
        Shape::MultiSolid(_) => vec![lod("MultiSolid"), lod("Geometry")],
        Shape::MultiSurface(_) if n == 0 => {
            vec!["lod0FootPrint".into(), lod("MultiSurface"), lod("Geometry")]
        }
        Shape::MultiSurface(_) => vec![lod("MultiSurface"), lod("Geometry")],
        Shape::CompositeSurface(_) => vec![lod("Geometry"), lod("MultiSurface")],
        Shape::MultiLineString(_) => {
            vec![lod("MultiCurve"), lod("Geometry"), lod("Network")]
        }
        Shape::MultiPoint(_) => vec![lod("Geometry")],
    }
}

/// What a geometry of `shape` is, for a loss.
fn kind(shape: &Shape) -> &'static str {
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
fn tin(geometry: &Geometry) -> Option<()> {
    let surfaces = matches!(
        geometry.shape,
        Shape::MultiSurface(_) | Shape::CompositeSurface(_)
    );
    let mut faces = geometry.shape.faces().peekable();
    (surfaces && faces.peek().is_some() && faces.all(gml::is_triangle)).then_some(())
}

/// Whether attribute `a` says what `class` already does: an installation's
/// `relationToConstruction`, which its class names.
fn implied(class: &QName, a: &Attribute) -> bool {
    matches!(
        &a.name,
        AttributeName::Property(q)
            if q.ns == Namespace::Module(Module::Construction)
                && q.local == "relationToConstruction"
    ) && class.local.ends_with("Installation")
}

/// Every face of `shape`, mutably, in [`Shape::faces`] order.
fn faces_mut(shape: &mut Shape) -> Vec<&mut Face> {
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
fn targets(geometries: &[Geometry]) -> (HashSet<String>, HashMap<String, String>) {
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
enum SurfaceData<'a> {
    Material(&'a Material),
    Texture(&'a Texture),
}

/// `appearance` by theme, in the order the themes first occur, each
/// theme's materials and then its textures.
fn themes<'a>(appearance: &'a Appearance) -> Vec<(&'a str, Vec<SurfaceData<'a>>)> {
    let mut out: Vec<(&'a str, Vec<SurfaceData<'a>>)> = Vec::new();
    let mut add = |theme: &'a str, data| match out.iter_mut().find(|(t, _)| *t == theme) {
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
const BLANK: &str = "attributes whose value is blank text";

/// Whether `value` is text that is empty or only whitespace.
fn blank(value: &Value) -> bool {
    matches!(value, Value::String(s) if s.trim().is_empty())
}

/// A value as the text of its property.
fn text(value: &Value) -> String {
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
fn envelope<W: Write>(x: &mut XmlWriter<W>, e: &[f64; 6], srs_name: Option<&str>) -> Result<()> {
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
fn pairs<K: AsRef<str>, V: AsRef<str>>(attributes: &[(K, V)]) -> Vec<(&str, &str)> {
    attributes
        .iter()
        .map(|(k, v)| (k.as_ref(), v.as_ref()))
        .collect()
}
