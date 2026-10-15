//! The model as a CityGML 3.0 document: one `core:cityObjectMember` per
//! top-level object, each property where its class's content model puts it
//! ([`super::schema`]), so the document reads back as the model it was
//! written from.
//!
//! A boundary surface holds its own polygons, in a `core:boundary` of its
//! object, a `con:fillingSurface` of the surface it fills, or a
//! transportation object's `tran:marking`; the traffic areas and auxiliary
//! traffic areas of a transportation object are those of one
//! `tran:TrafficSpace` and one `tran:AuxiliaryTrafficSpace` (of
//! granularity `way`), its holes those of one `tran:Hole`. A solid refers
//! by `xlink:href` to those of its faces a boundary surface holds, and
//! holds the rest; a surface geometry holds only the faces no boundary
//! surface holds, and is left out when every face is in one. A relative
//! geometry is written inside the first `ImplicitGeometry` that places it
//! and referred to by the later ones, and the appearance of the relative
//! geometries written follows the objects, as `core:appearanceMember`s.
//! A measured height is a construction's `con:height` whose status is
//! `measured`, from its top to its lowest ground point. An ADE's
//! properties of an object go in the `adeOf…` hook its schema names,
//! inside the ADE element the document they were read from held them in.
//! Every `gml:id` is written once: a repeated one is left out where it is
//! repeated. A group refers to each of its members: a member the group
//! holds is written after it, as an object of its own. An object's
//! children and boundary surfaces follow the content model's order, so
//! they read back in that order rather than the model's.
//!
//! What the model holds that CityGML 3.0 has no place for is left out and
//! counted in [`Losses`]: a LoD 4 geometry of an object that has one at
//! LoD 3 (any other is written at LoD 3); a geometry at a LoD its object
//! already has one of that kind at, or of a kind its class has no property
//! for; the faces of a boundary surface whose class has no geometry at
//! their LoD (kept in the object's geometry, untagged); an attribute naming
//! no property of its class; an ADE property whose hook is not known, or is
//! one of another CityGML version; and a child its parent's class cannot
//! hold, written as a top-level object of its own instead. A group member
//! the group holds, written as an object of its own, is counted too.

use std::collections::HashMap;
use std::io::Write;

use citygml_core::convert::{Losses, PropertyKind};
use citygml_core::model::{
    Address, Attribute, AttributeName, BoundarySurface, Face, Geometry, GmlObject, GroupMember,
    Instance, MemberTarget, Module, Namespace, QName, RingUvs, Shape, Template, Texture, Value,
    XmlNode,
};
use citygml_core::write::document::{
    BLANK, Base, ContentOf, Head, Plan, PlanRules, VersionWriter, blank, content_class, envelope,
    lod_number, pairs, rank, text, tin, writes_generic,
};
use citygml_core::write::gml::{self, Member, NS_GML32};
use citygml_core::write::{is_ncname, numbers, xal3};
use citygml_core::{Error, Result};

use super::schema::{
    ADE_CLASS, ADE_SURFACE, COMMON, COMMON_LEVELS, Level, MEASURED_HEIGHT, Slot, class_def,
    surface_def,
};
use crate::ns;

/// A CityGML 3.0 document being written.
pub struct DocumentWriter<W: Write> {
    b: Base<W>,
    content_of: ContentOf,
}

/// The CityGML 3.0 modules, as the root declares them, with their schemas.
const SCHEMAS: &[(Module, &str)] = &[
    (
        Module::Core,
        "http://schemas.opengis.net/citygml/3.0/core.xsd",
    ),
    (
        Module::Appearance,
        "http://schemas.opengis.net/citygml/appearance/3.0/appearance.xsd",
    ),
    (
        Module::Bridge,
        "http://schemas.opengis.net/citygml/bridge/3.0/bridge.xsd",
    ),
    (
        Module::Building,
        "http://schemas.opengis.net/citygml/building/3.0/building.xsd",
    ),
    (
        Module::CityFurniture,
        "http://schemas.opengis.net/citygml/cityfurniture/3.0/cityFurniture.xsd",
    ),
    (
        Module::CityObjectGroup,
        "http://schemas.opengis.net/citygml/cityobjectgroup/3.0/cityObjectGroup.xsd",
    ),
    (
        Module::Construction,
        "http://schemas.opengis.net/citygml/construction/3.0/construction.xsd",
    ),
    (
        Module::Generics,
        "http://schemas.opengis.net/citygml/generics/3.0/generics.xsd",
    ),
    (
        Module::LandUse,
        "http://schemas.opengis.net/citygml/landuse/3.0/landUse.xsd",
    ),
    (
        Module::Relief,
        "http://schemas.opengis.net/citygml/relief/3.0/relief.xsd",
    ),
    (
        Module::Transportation,
        "http://schemas.opengis.net/citygml/transportation/3.0/transportation.xsd",
    ),
    (
        Module::Tunnel,
        "http://schemas.opengis.net/citygml/tunnel/3.0/tunnel.xsd",
    ),
    (
        Module::Vegetation,
        "http://schemas.opengis.net/citygml/vegetation/3.0/vegetation.xsd",
    ),
    (
        Module::WaterBody,
        "http://schemas.opengis.net/citygml/waterbody/3.0/waterBody.xsd",
    ),
];

const NS_XLINK: &str = "http://www.w3.org/1999/xlink";
const NS_XAL: &str = "urn:oasis:names:tc:ciq:xal:3";
const NS_XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// What every CityGML namespace URI, of any version, begins with.
const CITYGML: &str = "http://www.opengis.net/citygml/";

/// The references a measured height is measured between.
const HEIGHT_FROM: &str = "lowestGroundPoint";
const HEIGHT_TO: &str = "topOfConstruction";

/// What the plan's name for a geometry property begins with when the
/// property is a `gen:GenericThematicSurface`'s, among its object's
/// boundary surfaces.
const GENERIC: &str = "generic:";

/// The granularity of the traffic spaces the writer gathers a
/// transportation object's areas in: CityJSON says nothing finer.
const GRANULARITY: &str = "way";

impl<W: Write> DocumentWriter<W> {
    /// Start a document on `out`: its root, declaring every CityGML 3.0
    /// module and each ADE of `head`, and the model's envelope. `templates`
    /// are the relative geometries the objects' `ImplicitGeometry`s place,
    /// by position; `content_of` gives an ADE class's CityGML class.
    pub fn new(
        out: W,
        head: Head,
        templates: Vec<Template>,
        content_of: ContentOf,
    ) -> Result<Self> {
        let b = Base::start(
            out,
            "CityGML 3.0",
            ns::uri,
            SCHEMAS,
            &[
                ("gml", NS_GML32),
                ("xlink", NS_XLINK),
                ("xAL", NS_XAL),
                ("xsi", NS_XSI),
            ],
            head,
            templates,
        )?;
        Ok(Self { b, content_of })
    }

    /// Write `object` as a `core:cityObjectMember`, followed by any of its
    /// descendants their parent's class cannot hold.
    pub fn write(&mut self, mut object: GmlObject) -> Result<()> {
        self.lod4(&mut object);
        self.write_top_level(object)
    }

    /// How many top-level objects have been written.
    pub fn objects(&self) -> usize {
        self.b.objects
    }

    /// End the document: the appearance of the relative geometries written,
    /// then the root. Gives back what it was written to, and what was left
    /// out.
    pub fn finish(self) -> Result<(W, Losses)> {
        self.finish_document()
    }

    /// `o` and its descendants without LoD 4, which CityGML 3.0 does not
    /// have: a LoD 4 geometry is at LoD 3 when its object has none there,
    /// and is left out when it has.
    fn lod4(&mut self, o: &mut GmlObject) {
        let at = |g: &Geometry, n: u8| lod_number(&g.lod) == Some(n);
        if o.geometries.iter().any(|g| at(g, 4)) {
            let has_lod3 = o.geometries.iter().any(|g| at(g, 3));
            let before = o.geometries.len();
            if has_lod3 {
                o.geometries.retain(|g| !at(g, 4));
            }
            for _ in o.geometries.len()..before {
                self.b
                    .losses
                    .note("LoD 4 geometries of objects that have geometry at LoD 3");
            }
            for g in o.geometries.iter_mut().filter(|g| at(g, 4)) {
                g.lod = "3".into();
                self.b.losses.note("LoD 4 geometries written at LoD 3");
            }
        }
        for child in &mut o.children {
            self.lod4(child);
        }
    }

    /// Whether `o` can be written at all; counts it when not.
    fn writable(&mut self, o: &GmlObject) -> bool {
        let class = content_class(&self.content_of, &o.class);
        let def = class.as_ref().and_then(class_def);
        let tin_relief = def.is_some_and(|d| d.slots().any(|(_, s)| *s == Slot::Tin));
        self.b.writable(o, def.is_some(), tin_relief)
    }

    /// One object, its element named by its class.
    fn object(&mut self, mut o: GmlObject, hoisted: &mut Vec<GmlObject>) -> Result<()> {
        let content = content_class(&self.content_of, &o.class);
        let def = content.as_ref().and_then(class_def);
        let levels: &'static [&'static Level] = def.map_or(ADE_CLASS, |d| d.levels);
        let plan = self.b.plan(&mut o, &Rules { levels });
        let mut children: Vec<Option<GmlObject>> = std::mem::take(&mut o.children)
            .into_iter()
            .map(Some)
            .collect();

        let id = o.id.clone().filter(|id| self.b.claim(id));
        let attrs: Vec<(&str, &str)> = id.iter().map(|id| ("gml:id", id.as_str())).collect();
        self.b.open_named(&o.class, &attrs)?;
        let mut used = vec![false; o.attributes.len()];
        let ade: Vec<(&XmlNode, Place)> =
            o.ade.iter().map(|n| (n, self.place(n, levels))).collect();

        // What every city object has, with the hooks among it.
        let [feature, lifespan, city_object] = COMMON_LEVELS;
        self.values_of(&o.attributes, &mut used, &COMMON[..2])?;
        if let Some(e) = &o.extent {
            let srs = self.b.srs_name();
            self.b.x.open("gml:boundedBy", &[])?;
            envelope(&mut self.b.x, e, srs.as_deref())?;
            self.b.x.close()?;
        }
        self.hook(feature, &ade)?;
        self.values_of(&o.attributes, &mut used, &COMMON[2..6])?;
        self.hook(lifespan, &ade)?;
        self.values_of(&o.attributes, &mut used, &COMMON[6..])?;
        self.appearance("core:appearance", &o.appearance, &plan.targets, &plan.rings)?;
        self.generic(&o.attributes, &mut used)?;
        self.hook(city_object, &ade)?;

        for level in levels {
            let module = Namespace::Module(level.module);
            for slot in level.slots {
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
                            self.geometry(name, &o.geometries[g], g, &plan)?;
                        }
                    }
                    Slot::Boundary => {
                        let property = QName::module(Module::Core, "boundary");
                        self.boundary_surfaces(&o, "boundary", &property, &plan)?;
                        self.generic_surfaces(&o, &plan)?;
                    }
                    Slot::Surfaces(name) => {
                        let property = QName::new(module.clone(), name);
                        self.boundary_surfaces(&o, name, &property, &plan)?;
                    }
                    Slot::Spaces(name, class, granularity) => {
                        self.space(&o, &plan, (level.module, name, class), granularity)?;
                    }
                    Slot::Nest(property, classes) => {
                        for child in children.iter_mut() {
                            if !child
                                .as_ref()
                                .is_some_and(|c| self.nests(c, level.module, classes))
                            {
                                continue;
                            }
                            let child = child.take().expect("checked above");
                            self.b
                                .open_named(&QName::new(module.clone(), property), &[])?;
                            self.object(child, hoisted)?;
                            self.b.x.close()?;
                        }
                    }
                    Slot::Address(property) => {
                        for a in &o.addresses {
                            self.b
                                .open_named(&QName::new(module.clone(), property), &[])?;
                            self.address(a)?;
                            self.b.x.close()?;
                        }
                    }
                    Slot::Member => {
                        for m in &o.members {
                            self.member(m, &mut children, hoisted)?;
                        }
                    }
                    Slot::Height => self.height(&o.attributes, &mut used)?,
                    Slot::ReliefLod => {
                        let lod = plan
                            .property
                            .iter()
                            .position(|p| p.as_deref() == Some("tin"))
                            .map_or(1, |g| plan.lod[g]);
                        self.b.x.leaf("dem:lod", &[], &lod.to_string())?;
                    }
                    Slot::Tin => {
                        if let Some(g) = plan
                            .property
                            .iter()
                            .position(|p| p.as_deref() == Some("tin"))
                        {
                            let faces: Vec<&Face> = o.geometries[g].shape.faces().collect();
                            let srs = self.b.srs();
                            self.b.x.open("dem:tin", &[])?;
                            gml::triangulated(&mut self.b.x, &faces, &pairs(&srs))?;
                            self.b.x.close()?;
                        }
                    }
                    Slot::Filling => {}
                }
            }
            self.hook(level, &ade)?;
        }
        // An ADE class's own properties, after its CityGML class's, in
        // the order its schema declares them.
        let order = self.b.layout.content(&o.class).unwrap_or_default().to_vec();
        let mut end: Vec<&XmlNode> = ade
            .iter()
            .filter(|(_, p)| *p == Place::End)
            .map(|(n, _)| *n)
            .collect();
        let ade_class = matches!(o.class.ns, Namespace::Other(_));
        if ade_class {
            end.sort_by_key(|n| rank(&order, &o.class, &n.name));
            for node in end {
                self.b.node(node)?;
            }
        } else {
            for _ in end {
                self.b
                    .losses
                    .note("ADE properties whose place in their object is not known");
            }
        }
        self.b.x.close()?;
        for (_, place) in &ade {
            match place {
                Place::Elsewhere => self
                    .b
                    .losses
                    .note("ADE properties of a CityGML class their object is not of"),
                Place::OtherVersion => self
                    .b
                    .losses
                    .note("ADE properties of an ADE for another CityGML version"),
                _ => {}
            }
        }

        let has_address = levels
            .iter()
            .any(|l| l.slots.iter().any(|s| matches!(s, Slot::Address(_))));
        if !o.addresses.is_empty() && !has_address {
            self.b
                .losses
                .note("addresses of objects whose class has none");
        }
        for (a, used) in o.attributes.iter().zip(&used) {
            if !used {
                self.b.losses.note(format!(
                    "attributes that name no property of their class ({})",
                    a.name.local()
                ));
            }
        }
        for child in children.into_iter().flatten() {
            self.b
                .losses
                .note("objects their parent's class cannot hold (written on their own)");
            hoisted.push(child);
        }
        Ok(())
    }

    /// Where ADE element `node` goes in the content of an object whose
    /// class has `levels`: the hook its container, or else itself,
    /// substitutes for.
    fn place(&self, node: &XmlNode, levels: &[&Level]) -> Place {
        let container = self
            .b
            .layout
            .container(&node.name)
            .map(|c| QName::new(node.name.ns.clone(), c));
        let hook = container
            .as_ref()
            .and_then(|c| self.b.layout.hook(c))
            .or_else(|| self.b.layout.hook(&node.name));
        let Some((uri, local)) = hook else {
            return Place::End;
        };
        let Some(module) = ns::module(uri.as_bytes()) else {
            // A 3.0 module this writer writes no object of (dynamizer,
            // point cloud, versioning) holds no object here.
            return if uri.starts_with(CITYGML) && !uri.ends_with("/3.0") {
                Place::OtherVersion
            } else if uri.starts_with(CITYGML) {
                Place::Elsewhere
            } else {
                Place::End
            };
        };
        let Some(class) = local.strip_prefix("ADEOf") else {
            return Place::End;
        };
        COMMON_LEVELS
            .iter()
            .chain(levels)
            .find(|l| l.module == module && l.class == class)
            .map_or(Place::Elsewhere, |l| Place::Hook(l.module, l.class))
    }

    /// The ADE elements of `ade` placed at `level`'s hook: each in an
    /// `adeOf…` element, those of one container in one, ordered as the
    /// container's schema declares them.
    fn hook(&mut self, level: &Level, ade: &[(&XmlNode, Place)]) -> Result<()> {
        let here: Vec<&XmlNode> = ade
            .iter()
            .filter(|(_, p)| *p == Place::Hook(level.module, level.class))
            .map(|(n, _)| *n)
            .collect();
        if here.is_empty() {
            return Ok(());
        }
        let property = QName::module(level.module, level.hook());
        // Each container, in the order its first element comes.
        let mut containers: Vec<Option<QName>> = Vec::new();
        for node in &here {
            let container = self
                .b
                .layout
                .container(&node.name)
                .map(|c| QName::new(node.name.ns.clone(), c));
            if !containers.contains(&container) {
                containers.push(container);
            }
        }
        for container in containers {
            let mut nodes: Vec<&XmlNode> = here
                .iter()
                .copied()
                .filter(|n| {
                    self.b
                        .layout
                        .container(&n.name)
                        .map(|c| QName::new(n.name.ns.clone(), c))
                        == container
                })
                .collect();
            match container {
                Some(container) => {
                    let order = self
                        .b
                        .layout
                        .content(&container)
                        .unwrap_or_default()
                        .to_vec();
                    nodes.sort_by_key(|n| rank(&order, &container, &n.name));
                    self.b.open_named(&property, &[])?;
                    self.b.open_named(&container, &[])?;
                    for node in nodes {
                        self.b.node(node)?;
                    }
                    self.b.x.close()?;
                    self.b.x.close()?;
                }
                // An element that substitutes for the hook's itself.
                None => {
                    for node in nodes {
                        self.b.open_named(&property, &[])?;
                        self.b.node(node)?;
                        self.b.x.close()?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether `child` is of one of `classes` of `module`, or of an ADE
    /// class that specialises one.
    fn nests(&self, child: &GmlObject, module: Module, classes: &[&str]) -> bool {
        content_class(&self.content_of, &child.class).is_some_and(|c| {
            c.ns == Namespace::Module(module) && classes.contains(&c.local.as_str())
        })
    }

    /// The attributes that are `properties`, in their order.
    fn values_of(
        &mut self,
        attributes: &[Attribute],
        used: &mut [bool],
        properties: &[citygml_core::convert::Property],
    ) -> Result<()> {
        for p in properties {
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
                self.b.losses.note(BLANK);
                continue;
            }
            let element = self.b.element(q)?;
            let uom = matches!(
                kind,
                PropertyKind::Number { measure: true } | PropertyKind::MeasureList
            );
            let attrs: &[(&str, &str)] = if uom { &[("uom", "m")] } else { &[] };
            self.b.x.leaf(&element, attrs, &text(&a.value))?;
        }
        Ok(())
    }

    /// A construction's measured height, as its `con:height`.
    fn height(&mut self, attributes: &[Attribute], used: &mut [bool]) -> Result<()> {
        let q = MEASURED_HEIGHT.qname();
        for (a, used) in attributes.iter().zip(used.iter_mut()) {
            if *used || a.name != AttributeName::Property(q.clone()) {
                continue;
            }
            *used = true;
            if blank(&a.value) {
                self.b.losses.note(BLANK);
                continue;
            }
            self.b.x.open("con:height", &[])?;
            self.b.x.open("con:Height", &[])?;
            self.b.x.leaf("con:highReference", &[], HEIGHT_TO)?;
            self.b.x.leaf("con:lowReference", &[], HEIGHT_FROM)?;
            self.b.x.leaf("con:status", &[], "measured")?;
            self.b
                .x
                .leaf("con:value", &[("uom", "m")], &text(&a.value))?;
            self.b.x.close()?;
            self.b.x.close()?;
        }
        Ok(())
    }

    /// The generic attributes, each in a `core:genericAttribute`.
    fn generic(&mut self, attributes: &[Attribute], used: &mut [bool]) -> Result<()> {
        for (a, used) in attributes.iter().zip(used.iter_mut()) {
            if let AttributeName::Generic(name) = &a.name {
                *used = true;
                self.generic_attribute("core:genericAttribute", name, &a.value)?;
            }
        }
        Ok(())
    }

    /// Generic attribute `name` of `value` in property `property`.
    fn generic_attribute(&mut self, property: &str, name: &str, value: &Value) -> Result<()> {
        if blank(value) {
            self.b.losses.note(BLANK);
            return Ok(());
        }
        let element = match value {
            Value::Integer(_) => "gen:IntAttribute",
            Value::Double(_) => "gen:DoubleAttribute",
            Value::String(_) => "gen:StringAttribute",
            Value::Boolean(_) => {
                self.b
                    .losses
                    .note("generic attributes whose value is a boolean (written as text)");
                "gen:StringAttribute"
            }
            Value::List(items) => {
                for item in items {
                    self.generic_attribute(property, name, item)?;
                }
                return Ok(());
            }
            Value::Set(members) => {
                if !members.iter().any(writes_generic) {
                    self.b
                        .losses
                        .note("generic attribute sets with no member to write");
                    return Ok(());
                }
                self.b.x.open(property, &[])?;
                self.b.x.open("gen:GenericAttributeSet", &[])?;
                self.b.x.leaf("gen:name", &[], name)?;
                for m in members {
                    match &m.name {
                        AttributeName::Generic(n) => {
                            self.generic_attribute("gen:genericAttribute", n, &m.value)?;
                        }
                        AttributeName::Property(_) => self
                            .b
                            .losses
                            .note("properties inside generic attribute sets"),
                    }
                }
                self.b.x.close()?;
                return self.b.x.close();
            }
        };
        self.b.x.open(property, &[])?;
        self.b.x.open(element, &[])?;
        self.b.x.leaf("gen:name", &[], name)?;
        self.b.x.leaf("gen:value", &[], &text(value))?;
        self.b.x.close()?;
        self.b.x.close()
    }

    /// Geometry `g`, `geometry`, as the core module's property `name`.
    fn geometry(&mut self, name: &str, geometry: &Geometry, g: usize, plan: &Plan) -> Result<()> {
        let property = QName::module(Module::Core, name);
        match &geometry.shape {
            Shape::Instance(instance) => {
                self.b.open_named(&property, &[])?;
                self.implicit(instance)?;
                self.b.x.close()
            }
            Shape::MultiPoint(points) if name.ends_with("Point") => {
                let srs = self.b.srs();
                self.b.open_named(&property, &[])?;
                gml::point(&mut self.b.x, &points[0], &pairs(&srs))?;
                self.b.x.close()
            }
            _ => self.b.geometry(&property.ns, name, geometry, g, plan),
        }
    }

    /// A `core:ImplicitGeometry`: the relative geometry it places, inline
    /// the first time, else by reference.
    fn implicit(&mut self, instance: &Instance) -> Result<()> {
        let t = instance.template;
        let Some(template) = self.b.templates.get(t) else {
            return Err(Error::invalid(format!(
                "an implicit geometry places template {t}, which the document does not have"
            )));
        };
        let id = template.id.clone();
        let shape = self.b.written[t]
            .is_none()
            .then(|| template.geometry.shape.clone());
        self.b.x.open("core:ImplicitGeometry", &[])?;
        self.b
            .x
            .leaf("core:transformationMatrix", &[], &numbers(instance.matrix))?;
        let srs = self.b.srs();
        self.b.x.open("core:referencePoint", &[])?;
        gml::point(&mut self.b.x, &instance.reference_point, &pairs(&srs))?;
        self.b.x.close()?;
        if let Some(written) = &self.b.written[t] {
            let href = format!("#{written}");
            self.b
                .x
                .empty("core:relativeGeometry", &[("xlink:href", &href)])?;
        } else if let Some(shape) = shape {
            let id = match id.filter(|id| is_ncname(id) && self.b.claim(id)) {
                Some(id) => id,
                None => self.b.fresh_id(),
            };
            self.b.written[t] = Some(id.clone());
            self.b.x.open("core:relativeGeometry", &[])?;
            gml::shape(&mut self.b.x, &shape, &[("gml:id", &id)], &mut |_| {
                Member::Inline
            })?;
            self.b.x.close()?;
        }
        self.b.x.close()
    }

    /// The boundary surfaces of `o` its property `holder` holds, each in a
    /// `property` of its own.
    fn boundary_surfaces(
        &mut self,
        o: &GmlObject,
        holder: &str,
        property: &QName,
        plan: &Plan,
    ) -> Result<()> {
        for s in 0..o.boundary_surfaces.len() {
            if o.boundary_surfaces[s].parent.is_none()
                && plan.holder[s] == Some(holder)
                && plan.has_faces(s)
            {
                self.boundary_surface(o, s, property, plan)?;
            }
        }
        Ok(())
    }

    /// The surface geometries of `o` its class has no property for, each
    /// as a `gen:GenericThematicSurface` among its boundary surfaces: the
    /// faces no boundary surface holds.
    fn generic_surfaces(&mut self, o: &GmlObject, plan: &Plan) -> Result<()> {
        for (g, property) in plan.property.iter().enumerate() {
            let Some(name) = property.as_deref().and_then(|p| p.strip_prefix(GENERIC)) else {
                continue;
            };
            if plan.tags[g].iter().all(Option::is_some) {
                continue;
            }
            self.b.x.open("core:boundary", &[])?;
            self.b.x.open("gen:GenericThematicSurface", &[])?;
            self.b.geometry(
                &Namespace::Module(Module::Core),
                name,
                &o.geometries[g],
                g,
                plan,
            )?;
            self.b.x.close()?;
            self.b.x.close()?;
        }
        Ok(())
    }

    /// The boundary surfaces of `o` a transportation space holds: one space
    /// of `class` in `o`'s property `name` of `module`, holding each in a
    /// `core:boundary`, of the granularity it must state when `granularity`.
    fn space(
        &mut self,
        o: &GmlObject,
        plan: &Plan,
        (module, name, class): (Module, &'static str, &'static str),
        granularity: bool,
    ) -> Result<()> {
        let any = (0..o.boundary_surfaces.len()).any(|s| {
            o.boundary_surfaces[s].parent.is_none()
                && plan.holder[s] == Some(name)
                && plan.has_faces(s)
        });
        if !any {
            return Ok(());
        }
        self.b.open_named(&QName::module(module, name), &[])?;
        self.b.open_named(&QName::module(module, class), &[])?;
        let boundary = QName::module(Module::Core, "boundary");
        self.boundary_surfaces(o, name, &boundary, plan)?;
        if granularity {
            let element = self.b.element(&QName::module(module, "granularity"))?;
            self.b.x.leaf(&element, &[], GRANULARITY)?;
        }
        self.b.x.close()?;
        self.b.x.close()
    }

    /// Boundary surface `s` of `o` in a `property`, with the surfaces that
    /// fill it.
    fn boundary_surface(
        &mut self,
        o: &GmlObject,
        s: usize,
        property: &QName,
        plan: &Plan,
    ) -> Result<()> {
        let surface = &o.boundary_surfaces[s];
        self.b.open_named(property, &[])?;
        self.b.open_named(&surface.class, &[])?;
        let mut used = vec![false; surface.attributes.len()];
        self.values_of(&surface.attributes, &mut used, COMMON)?;
        self.generic(&surface.attributes, &mut used)?;
        let levels = surface_def(&surface.class).map_or(ADE_SURFACE, |d| d.levels);
        for level in levels {
            for slot in level.slots {
                match *slot {
                    Slot::Value(name, kind, _) => {
                        let q = QName::module(level.module, name);
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
                        let srs = self.b.srs();
                        self.b.open_named(&QName::module(Module::Core, name), &[])?;
                        self.b.x.open("gml:MultiSurface", &pairs(&srs))?;
                        for (face, solid) in faces {
                            self.b.surface_member(face, solid)?;
                        }
                        self.b.x.close()?;
                        self.b.x.close()?;
                    }
                    Slot::Filling => {
                        let filling = QName::module(level.module, "fillingSurface");
                        for &c in &plan.nested[s] {
                            if plan.has_faces(c) {
                                self.boundary_surface(o, c, &filling, plan)?;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        self.b.x.close()?;
        self.b.x.close()?;
        for (a, used) in surface.attributes.iter().zip(&used) {
            if !used {
                self.b.losses.note(format!(
                    "attributes that name no property of their class ({})",
                    a.name.local()
                ));
            }
        }
        Ok(())
    }

    /// A group member, in its `grp:Role`. CityGML 3.0's `grp:groupMember`
    /// only refers to its member, so a child the group holds is written
    /// after it as an object of its own, and referred to.
    fn member(
        &mut self,
        m: &GroupMember,
        children: &mut [Option<GmlObject>],
        hoisted: &mut Vec<GmlObject>,
    ) -> Result<()> {
        let id = match &m.target {
            MemberTarget::Child(k) => {
                let Some(mut child) = children.get_mut(*k).and_then(Option::take) else {
                    return Ok(());
                };
                if !self.writable(&child) {
                    self.b.orphan_children(child, hoisted);
                    return Ok(());
                }
                let id = match &child.id {
                    Some(id) => id.clone(),
                    None => {
                        let id = self.b.unused_id();
                        child.id = Some(id.clone());
                        id
                    }
                };
                self.b
                    .losses
                    .note("group members written as objects of their own, referred to");
                hoisted.push(child);
                id
            }
            MemberTarget::Id(id) => id.clone(),
        };
        self.b.x.open("grp:groupMember", &[])?;
        self.b.x.open("grp:Role", &[])?;
        if let Some(role) = &m.role {
            self.b.x.leaf("grp:role", &[], role)?;
        }
        let href = format!("#{id}");
        self.b
            .x
            .empty("grp:groupMember", &[("xlink:href", &href)])?;
        self.b.x.close()?;
        self.b.x.close()
    }

    /// A `core:Address`.
    fn address(&mut self, a: &Address) -> Result<()> {
        self.b.x.open("core:Address", &[])?;
        self.b.x.open("core:xalAddress", &[])?;
        for leaf in xal3::address(&mut self.b.x, a)? {
            self.b
                .losses
                .note(format!("address members xAL 3 has no place for ({leaf})"));
        }
        self.b.x.close()?;
        if !a.location.is_empty() {
            let srs = self.b.srs();
            self.b.x.open("core:multiPoint", &[])?;
            gml::shape(
                &mut self.b.x,
                &Shape::MultiPoint(a.location.clone()),
                &pairs(&srs),
                &mut |_| Member::Inline,
            )?;
            self.b.x.close()?;
        }
        self.b.x.close()
    }

    fn texture(&mut self, t: &Texture, rings: &HashMap<String, String>) -> Result<()> {
        let Some(image) = &t.image_uri else {
            self.b.losses.note("textures without an image");
            return Ok(());
        };
        // The rings of each polygon, in the order first met.
        let mut by_polygon: Vec<(&str, Vec<&RingUvs>)> = Vec::new();
        for ring in &t.rings {
            let Some(polygon) = rings.get(&ring.0) else {
                self.b
                    .losses
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
        self.b.x.open("app:surfaceData", &[])?;
        self.b.x.open("app:ParameterizedTexture", &[])?;
        let border = t.border_color.as_ref().map(|c| numbers(c.iter().copied()));
        for (element, value) in [
            ("app:imageURI", Some(image.as_str())),
            ("app:mimeType", t.mime_type.as_deref()),
            ("app:textureType", t.texture_type.as_deref()),
            ("app:wrapMode", t.wrap_mode.as_deref()),
            ("app:borderColor", border.as_deref()),
        ] {
            if let Some(value) = value {
                self.b.x.leaf(element, &[], value)?;
            }
        }
        for (polygon, rings) in by_polygon {
            self.b.x.open("app:textureParameterization", &[])?;
            self.b.x.open("app:TextureAssociation", &[])?;
            self.b.x.leaf("app:target", &[], &format!("#{polygon}"))?;
            self.b.x.open("app:textureParameterization", &[])?;
            self.b.x.open("app:TexCoordList", &[])?;
            for (_, uvs) in &rings {
                let closed = uvs.iter().chain(uvs.first());
                self.b.x.leaf(
                    "app:textureCoordinates",
                    &[],
                    &numbers(closed.flat_map(|uv| uv.iter().copied())),
                )?;
            }
            for (ring, _) in &rings {
                self.b.x.leaf("app:ring", &[], &format!("#{ring}"))?;
            }
            self.b.x.close()?;
            self.b.x.close()?;
            self.b.x.close()?;
            self.b.x.close()?;
        }
        self.b.x.close()?;
        self.b.x.close()
    }
}

impl<W: Write> VersionWriter<W> for DocumentWriter<W> {
    const SURFACE_DATA: &'static str = "app:surfaceData";
    const APPEARANCE_MEMBER: &'static str = "core:appearanceMember";

    fn base(&mut self) -> &mut Base<W> {
        &mut self.b
    }

    fn into_base(self) -> Base<W> {
        self.b
    }

    fn writable(&mut self, o: &GmlObject) -> bool {
        DocumentWriter::writable(self, o)
    }

    fn object(&mut self, o: GmlObject, hoisted: &mut Vec<GmlObject>) -> Result<()> {
        DocumentWriter::object(self, o, hoisted)
    }

    fn texture(&mut self, t: &Texture, rings: &HashMap<String, String>) -> Result<()> {
        DocumentWriter::texture(self, t, rings)
    }
}

/// Where an ADE element goes in its object's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// In the `adeOf…` hook of the level of this module and class.
    Hook(Module, &'static str),
    /// At the end, where an ADE class's schema puts it.
    End,
    /// Nowhere: at the hook of a class the object's class does not derive
    /// from.
    Elsewhere,
    /// Nowhere: at the hook of another CityGML version.
    OtherVersion,
}

/// What a CityGML 3.0 class holds, for [`Base::plan`]: its levels.
struct Rules {
    levels: &'static [&'static Level],
}

impl Rules {
    fn slots(&self) -> impl Iterator<Item = &'static Slot> + '_ {
        self.levels.iter().flat_map(|l| l.slots.iter())
    }
}

impl PlanRules for Rules {
    fn candidates(&self, geometry: &Geometry, n: u8) -> Vec<String> {
        if self.slots().any(|s| *s == Slot::Tin) {
            return tin(geometry)
                .map(|_| "tin".to_string())
                .into_iter()
                .collect();
        }
        let mut fits: Vec<String> = candidates(&geometry.shape, n)
            .into_iter()
            .filter(|c| {
                self.slots()
                    .any(|s| matches!(s, Slot::Geometry(name) if name == c))
            })
            .collect();
        // Surfaces at a LoD a space has no property for (LoD 1) are a
        // generic thematic surface's among its boundary surfaces.
        let surfaces = matches!(
            geometry.shape,
            Shape::MultiSurface(_) | Shape::CompositeSurface(_)
        );
        if surfaces && fits.is_empty() && n <= 3 && self.slots().any(|s| *s == Slot::Boundary) {
            fits.push(format!("{GENERIC}lod{n}MultiSurface"));
        }
        fits
    }

    fn surface_holder(&self, surfaces: &[BoundarySurface], s: usize) -> Option<&'static str> {
        surface_holder(surfaces, s, self.levels)
    }

    fn surface_holds_lod(&self, class: &QName, n: u8) -> bool {
        let levels = surface_def(class).map_or(ADE_SURFACE, |d| d.levels);
        levels.iter().flat_map(|l| l.slots.iter()).any(|s| match s {
            Slot::Geometry(name) => surface_geometry_lod(name) == Some(n),
            _ => false,
        })
    }
}

/// The property surface `s` is written in: its owner's, or a filling
/// surface's for a door or window filling a surface written. `None` when
/// neither can hold it.
fn surface_holder(
    surfaces: &[BoundarySurface],
    s: usize,
    levels: &[&Level],
) -> Option<&'static str> {
    let surface = &surfaces[s];
    let holder: &'static str = match &surface.class.ns {
        Namespace::Other(_) if is_ncname(&surface.class.local) => "boundary",
        Namespace::Other(_) => return None,
        _ => surface_def(&surface.class)?.holder,
    };
    match surface.parent {
        None => {
            // A door or window of no surface is one of its object's.
            let holder = if holder == "fillingSurface" {
                "boundary"
            } else {
                holder
            };
            levels
                .iter()
                .flat_map(|l| l.slots.iter())
                .any(|slot| slot.holder() == Some(holder))
                .then_some(holder)
        }
        Some(p) => {
            let parent = surface_def(&surfaces.get(p)?.class)?;
            let fillable = parent
                .levels
                .iter()
                .any(|l| l.slots.contains(&Slot::Filling));
            (holder == "fillingSurface" && fillable)
                .then(|| surface_holder(surfaces, p, levels))
                .flatten()
                .map(|_| holder)
        }
    }
}

/// The LoD of a boundary surface's geometry property
/// (`lod3MultiSurface`); `None` for another property.
fn surface_geometry_lod(name: &str) -> Option<u8> {
    let rest = name.strip_prefix("lod")?;
    let (digit, kind) = rest.split_at(1);
    (kind == "MultiSurface")
        .then(|| digit.parse().ok())
        .flatten()
}

/// The properties a geometry of `shape` at LoD `n` may be written as.
fn candidates(shape: &Shape, n: u8) -> Vec<String> {
    let lod = |kind: &str| format!("lod{n}{kind}");
    match shape {
        Shape::Instance(_) => vec![lod("ImplicitRepresentation")],
        Shape::Solid(_) | Shape::CompositeSolid(_) => vec![lod("Solid")],
        Shape::MultiSurface(_) | Shape::CompositeSurface(_) => vec![lod("MultiSurface")],
        Shape::MultiLineString(_) => vec![lod("MultiCurve")],
        Shape::MultiPoint(points) if points.len() == 1 => vec![lod("Point")],
        Shape::MultiPoint(_) | Shape::MultiSolid(_) => Vec::new(),
    }
}
