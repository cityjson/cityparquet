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

use std::collections::{HashMap, HashSet};
use std::io::Write;

use citygml_core::convert::{Losses, PropertyKind};
use citygml_core::model::{
    Address, Appearance, Attribute, AttributeName, BoundarySurface, Face, Geometry, GmlObject,
    GroupMember, Instance, Material, MemberTarget, Module, Namespace, QName, RingUvs, Shape,
    Template, Texture, Value, XmlNode, order_key,
};
use citygml_core::write::document::{
    BLANK, Base, ContentOf, Head, Plan, PlanRules, SurfaceData, blank, envelope, pairs, targets,
    text, themes, tin, writes_generic,
};
use citygml_core::write::gml::{self, Member};
use citygml_core::write::{is_ncname, number, numbers, xal};
use citygml_core::{Error, Result};

use super::schema::{COMMON, Hook, Slot, class_def, surface_def};
use crate::ns;

/// A CityGML 2.0 document being written.
pub struct DocumentWriter<W: Write> {
    b: Base<W>,
    content_of: ContentOf,
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
        let b = Base::start(
            out,
            "CityGML 2.0",
            ns::uri,
            SCHEMAS,
            &[
                ("gml", NS_GML),
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
    pub fn write(&mut self, object: GmlObject) -> Result<()> {
        let mut pending = vec![object];
        while let Some(o) = pending.pop() {
            if !self.writable(&o) {
                let mut orphans = Vec::new();
                self.b.orphan_children(o, &mut orphans);
                pending.extend(orphans.into_iter().rev());
                continue;
            }
            self.b.x.open("core:cityObjectMember", &[])?;
            let mut hoisted = Vec::new();
            self.object(o, &mut hoisted)?;
            self.b.x.close()?;
            self.b.objects += 1;
            pending.extend(hoisted.into_iter().rev());
        }
        Ok(())
    }

    /// How many top-level objects have been written.
    pub fn objects(&self) -> usize {
        self.b.objects
    }

    /// End the document: the appearance of the relative geometries written,
    /// then the root. Gives back what it was written to, and what was left
    /// out.
    pub fn finish(mut self) -> Result<(W, Losses)> {
        let templates = std::mem::take(&mut self.b.templates);
        let written = std::mem::take(&mut self.b.written);
        for (template, _) in templates.iter().zip(written).filter(|(_, w)| w.is_some()) {
            let (targets, rings) = targets(std::slice::from_ref(&template.geometry));
            self.appearance(
                "app:appearanceMember",
                &template.appearance,
                &targets,
                &rings,
            )?;
        }
        self.b.x.close()?;
        Ok((self.b.x.finish()?, self.b.losses))
    }

    /// Whether `o` can be written at all; counts it when not.
    fn writable(&mut self, o: &GmlObject) -> bool {
        if matches!(o.class.ns, Namespace::Other(_)) && !is_ncname(&o.class.local) {
            self.b
                .losses
                .note("objects of an ADE class whose name is not an XML name");
            return false;
        }
        let class = self.content_class(&o.class);
        let def = class.as_ref().and_then(class_def);
        if def.is_none() && !matches!(o.class.ns, Namespace::Other(_)) {
            self.b.losses.note(format!(
                "objects of a class CityGML 2.0 does not have ({})",
                o.class.local
            ));
            return false;
        }
        if def.is_some_and(|d| d.content.contains(&Slot::Tin))
            && !o.geometries.iter().any(|g| tin(g).is_some())
        {
            self.b
                .losses
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

    /// One object, its element named by its class.
    fn object(&mut self, mut o: GmlObject, hoisted: &mut Vec<GmlObject>) -> Result<()> {
        let content = self.content_class(&o.class);
        let def = content.as_ref().and_then(class_def);
        let slots: &'static [Slot] = def.map_or(&[], |d| d.content);
        // The namespace of the class's own properties.
        let module = def.map_or_else(|| o.class.ns.clone(), |d| Namespace::Module(d.module));
        let ade_class = def.is_none();
        let plan = self.b.plan(&mut o, &Rules { slots, ade_class });
        let mut children: Vec<Option<GmlObject>> = std::mem::take(&mut o.children)
            .into_iter()
            .map(Some)
            .collect();

        let id = o.id.clone().filter(|id| self.b.claim(id));
        let attrs: Vec<(&str, &str)> = id.iter().map(|id| ("gml:id", id.as_str())).collect();
        self.b.open_named(&o.class, &attrs)?;
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
            self.b.node(node)?;
        }

        if ade_class {
            self.ade_content(&o, &module, &plan, &places)?;
        }
        for (at, slot) in slots.iter().enumerate() {
            for (h, _) in hooks.iter().enumerate().filter(|(_, h)| h.at == at) {
                for node in ade(Place::Hook(h)) {
                    self.b.node(node)?;
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
                Slot::Opening => {}
            }
        }
        for (h, _) in hooks
            .iter()
            .enumerate()
            .filter(|(_, h)| h.at == slots.len())
        {
            for node in ade(Place::Hook(h)) {
                self.b.node(node)?;
            }
        }
        if !ade_class {
            // An ADE class's own properties, after its CityGML class's.
            let order = self.b.layout.content(&o.class).unwrap_or_default().to_vec();
            let mut end: Vec<&XmlNode> = ade(Place::End).collect();
            end.sort_by_key(|n| rank(&order, &o.class, &n.name));
            for node in end {
                self.b.node(node)?;
            }
        }
        self.b.x.close()?;
        for _ in ade(Place::Elsewhere) {
            self.b
                .losses
                .note("ADE properties of a CityGML class their object is not of");
        }

        if !o.addresses.is_empty() && !slots.iter().any(|s| matches!(s, Slot::Address(_))) {
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

    /// Where ADE element `node` goes in the content of an object whose class
    /// has `hooks`.
    fn place(&self, node: &XmlNode, hooks: &[Hook]) -> Place {
        let Some((uri, local)) = self.b.layout.hook(&node.name) else {
            return Place::End;
        };
        if uri == ns::CORE && local == "_GenericApplicationPropertyOfCityObject" {
            return Place::CityObject;
        }
        if let Some(h) = hooks
            .iter()
            .position(|h| ns::uri(h.module) == Some(uri.as_str()) && h.element() == *local)
        {
            return Place::Hook(h);
        }
        // A hook of CityGML 2.0 the class does not have; one of another
        // version says nothing about where in 2.0 the property goes.
        if ns::module(uri.as_bytes()).is_some() {
            Place::Elsewhere
        } else {
            Place::End
        }
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
            .b
            .layout
            .content(&o.class)
            .map(<[String]>::to_vec)
            .unwrap_or_default();
        let own = |local: &str| rank(&order, &o.class, &QName::new(module.clone(), local));
        let mut items: Vec<(usize, Item)> = Vec::new();
        for (g, property) in plan.property.iter().enumerate() {
            if let Some(p) = property {
                items.push((own(p), Item::Geometry(g)));
            }
        }
        items.push((own("boundedBy"), Item::Surfaces));
        for (i, (node, place)) in o.ade.iter().zip(places).enumerate() {
            if *place == Place::End {
                items.push((rank(&order, &o.class, &node.name), Item::Node(i)));
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
                Item::Node(i) => self.b.node(&o.ade[i])?,
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
                let srs = self.b.srs_name();
                self.b.x.open("gml:boundedBy", &[])?;
                envelope(&mut self.b.x, e, srs.as_deref())?;
                self.b.x.close()?;
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
            self.b.losses.note(BLANK);
            return Ok(());
        }
        let element = match value {
            Value::Integer(_) => "gen:intAttribute",
            Value::Double(_) => "gen:doubleAttribute",
            Value::String(_) => "gen:stringAttribute",
            Value::Boolean(_) => {
                self.b
                    .losses
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
                if !members.iter().any(writes_generic) {
                    self.b
                        .losses
                        .note("generic attribute sets with no member to write");
                    return Ok(());
                }
                self.b
                    .x
                    .open("gen:genericAttributeSet", &[("name", name)])?;
                for m in members {
                    match &m.name {
                        AttributeName::Generic(n) => self.generic_attribute(n, &m.value)?,
                        AttributeName::Property(_) => self
                            .b
                            .losses
                            .note("properties inside generic attribute sets"),
                    }
                }
                return self.b.x.close();
            }
        };
        self.b.x.open(element, &[("name", name)])?;
        self.b.x.leaf("gen:value", &[], &text(value))?;
        self.b.x.close()
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
        if let Shape::Instance(instance) = &geometry.shape {
            self.b.open_named(&QName::new(module.clone(), name), &[])?;
            self.implicit(instance)?;
            return self.b.x.close();
        }
        self.b.geometry(module, name, geometry, g, plan)
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
        self.b.x.open("core:ImplicitGeometry", &[])?;
        self.b
            .x
            .leaf("core:transformationMatrix", &[], &numbers(instance.matrix))?;
        if let Some(written) = &self.b.written[t] {
            let href = format!("#{written}");
            self.b
                .x
                .empty("core:relativeGMLGeometry", &[("xlink:href", &href)])?;
        } else {
            let shape = template.geometry.shape.clone();
            let id = match id.filter(|id| is_ncname(id) && self.b.claim(id)) {
                Some(id) => id,
                None => self.b.fresh_id(),
            };
            self.b.written[t] = Some(id.clone());
            self.b.x.open("core:relativeGMLGeometry", &[])?;
            gml::shape(&mut self.b.x, &shape, &[("gml:id", &id)], &mut |_| {
                Member::Inline
            })?;
            self.b.x.close()?;
        }
        let srs = self.b.srs();
        self.b.x.open("core:referencePoint", &[])?;
        gml::point(&mut self.b.x, &instance.reference_point, &pairs(&srs))?;
        self.b.x.close()?;
        self.b.x.close()
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
        self.b
            .open_named(&QName::new(module.clone(), holder), &[])?;
        self.b.open_named(&surface.class, &[])?;
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
                    let srs = self.b.srs();
                    self.b
                        .open_named(&QName::new(surface_ns.clone(), name), &[])?;
                    if name.ends_with("MultiSurface") {
                        self.b.x.open("gml:MultiSurface", &pairs(&srs))?;
                        for (face, solid) in faces {
                            self.surface_member(face, solid)?;
                        }
                        self.b.x.close()?;
                    } else if let [(face, solid)] = faces.as_slice() {
                        if *solid {
                            gml::polygon(&mut self.b.x, &face.polygon, &pairs(&srs))?;
                        } else {
                            gml::surface(&mut self.b.x, face, &pairs(&srs))?;
                        }
                    } else {
                        self.b.x.open("gml:CompositeSurface", &pairs(&srs))?;
                        for (face, solid) in faces {
                            self.surface_member(face, solid)?;
                        }
                        self.b.x.close()?;
                    }
                    self.b.x.close()?;
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

    /// A face in a boundary surface's geometry: the polygon a solid refers
    /// to as it is, any other face as its geometry uses it.
    fn surface_member(&mut self, face: &Face, solid: bool) -> Result<()> {
        if solid {
            self.b.x.open("gml:surfaceMember", &[])?;
            gml::polygon(&mut self.b.x, &face.polygon, &[])?;
            self.b.x.close()
        } else {
            gml::member(&mut self.b.x, "gml:surfaceMember", face, Member::Inline)
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
                if !self.writable(&child) {
                    self.b.orphan_children(child, hoisted);
                    return Ok(());
                }
                if let Some(role) = &m.role {
                    attrs.push(("role", role.clone()));
                }
                self.b.x.open("grp:groupMember", &pairs(&attrs))?;
                self.object(child, hoisted)?;
                self.b.x.close()
            }
            MemberTarget::Id(id) => {
                attrs.push(("xlink:href", format!("#{id}")));
                if let Some(role) = &m.role {
                    attrs.push(("role", role.clone()));
                }
                self.b.x.empty("grp:groupMember", &pairs(&attrs))
            }
        }
    }

    /// A `core:Address`.
    fn address(&mut self, a: &Address) -> Result<()> {
        self.b.x.open("core:Address", &[])?;
        self.b.x.open("core:xalAddress", &[])?;
        for leaf in xal::address_details(&mut self.b.x, a)? {
            self.b
                .losses
                .note(format!("address members xAL has no place for ({leaf})"));
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
            self.b.x.open(element, &[])?;
            self.b.x.open("app:Appearance", &[])?;
            if !theme.is_empty() {
                self.b.x.leaf("app:theme", &[], theme)?;
            }
            for d in data {
                match d {
                    SurfaceData::Material(m) => self.material(m, targets)?,
                    SurfaceData::Texture(t) => self.texture(t, rings)?,
                }
            }
            self.b.x.close()?;
            self.b.x.close()?;
        }
        Ok(())
    }

    fn material(&mut self, m: &Material, targets: &HashSet<String>) -> Result<()> {
        let written: Vec<&String> = m.targets.iter().filter(|t| targets.contains(*t)).collect();
        if written.len() < m.targets.len() {
            self.b
                .losses
                .note("material targets the document does not write");
        }
        if written.is_empty() {
            return Ok(());
        }
        let id = m.id.clone().filter(|id| self.b.claim(id));
        let attrs: Vec<(&str, &str)> = id.iter().map(|id| ("gml:id", id.as_str())).collect();
        self.b.x.open("app:surfaceDataMember", &[])?;
        self.b.x.open("app:X3DMaterial", &attrs)?;
        if let Some(name) = &m.name {
            self.b.x.leaf("gml:name", &[], name)?;
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
                self.b.x.leaf(element, &[], &value)?;
            }
        }
        for t in written {
            self.b.x.leaf("app:target", &[], &format!("#{t}"))?;
        }
        self.b.x.close()?;
        self.b.x.close()
    }

    fn texture(&mut self, t: &Texture, rings: &HashMap<String, String>) -> Result<()> {
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
        self.b.x.open("app:surfaceDataMember", &[])?;
        self.b.x.open("app:ParameterizedTexture", &[])?;
        let border = t.border_color.as_ref().map(|c| numbers(c.iter().copied()));
        for (element, value) in [
            ("app:imageURI", t.image_uri.as_deref()),
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
            let uri = format!("#{polygon}");
            self.b.x.open("app:target", &[("uri", &uri)])?;
            self.b.x.open("app:TexCoordList", &[])?;
            for (ring, uvs) in rings {
                let closed = uvs.iter().chain(uvs.first());
                let r = format!("#{ring}");
                self.b.x.leaf(
                    "app:textureCoordinates",
                    &[("ring", &r)],
                    &numbers(closed.flat_map(|uv| uv.iter().copied())),
                )?;
            }
            self.b.x.close()?;
            self.b.x.close()?;
        }
        self.b.x.close()?;
        self.b.x.close()
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
    /// Nowhere: at the hook of a class the object's class does not derive
    /// from.
    Elsewhere,
}

/// Where element `name` comes in `order`, the content order of the ADE
/// class `class`; after every element it names when it names none.
fn rank(order: &[String], class: &QName, name: &QName) -> usize {
    let (Namespace::Other(own), Namespace::Other(uri)) = (&class.ns, &name.ns) else {
        return order.len();
    };
    let key = order_key(own, uri, &name.local);
    order.iter().position(|n| *n == key).unwrap_or(order.len())
}

/// What a CityGML 2.0 class holds, for [`Base::plan`]: its content, or an
/// ADE class's, which CityGML has no content for.
struct Rules {
    slots: &'static [Slot],
    ade_class: bool,
}

impl PlanRules for Rules {
    fn candidates(&self, geometry: &Geometry, n: u8) -> Vec<String> {
        let tin_class = self.slots.contains(&Slot::Tin);
        let group = self.slots.contains(&Slot::Member);
        if group {
            // `grp:geometry` holds GML geometry, not an implicit one.
            return match geometry.shape {
                Shape::Instance(_) => Vec::new(),
                _ => vec!["geometry".to_string()],
            };
        }
        if tin_class {
            return tin(geometry)
                .map(|_| "tin".to_string())
                .into_iter()
                .collect();
        }
        candidates(&geometry.shape, n)
            .into_iter()
            .filter(|c| {
                self.ade_class
                    || self
                        .slots
                        .iter()
                        .any(|s| matches!(s, Slot::Geometry(name) if name == c))
            })
            .collect()
    }

    fn surface_holder(&self, surfaces: &[BoundarySurface], s: usize) -> Option<&'static str> {
        surface_holder(surfaces, s, self.slots, self.ade_class)
    }

    fn surface_holds_lod(&self, class: &QName, n: u8) -> bool {
        surface_holds_lod(class, n)
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
        Namespace::Other(_) if is_ncname(&surface.class.local) => "boundedBy",
        Namespace::Other(_) => return None,
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
