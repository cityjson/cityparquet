//! Conversions out of the model, and into it.
//!
//! [`FromGml`] is the one contract a target model implements, so a reader
//! never learns what its objects become; [`ToGml`] is the way back, so a
//! writer never learns where its objects came from. The `cjseq` feature
//! implements both for [`cjseq::CityJSONFeature`].
//!
//! Going back needs what a CityGML version names: which class a CityJSON
//! type is, which boundary surface class a semantic surface is, and which
//! of a class's properties an attribute is. A version crate answers through
//! [`Vocabulary`].

#[cfg(feature = "cjseq")]
pub mod cjseq;

use std::collections::BTreeMap;

use crate::model::QName;
use crate::{GmlObject, Result};

/// A model that can be built from one top-level [`GmlObject`].
pub trait FromGml: Sized {
    /// What the conversion needs beyond the object itself (for CityJSON, the
    /// transform its vertices are quantised against).
    type Context;

    fn from_gml(object: &GmlObject, context: &Self::Context) -> Result<Self>;
}

/// A model that can be turned into top-level [`GmlObject`]s.
pub trait ToGml {
    /// What the conversion needs beyond the value itself.
    type Context;

    fn to_gml(&self, context: &Self::Context) -> Result<GmlFeature>;
}

/// What [`ToGml`] makes of one value: the top-level objects, in order, and
/// what CityGML could not hold.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct GmlFeature {
    pub objects: Vec<GmlObject>,
    pub losses: Losses,
}

/// What a conversion left out because the target cannot hold it, counted
/// by a description of what was left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Losses(BTreeMap<String, usize>);

impl Losses {
    /// Count one more of `what`.
    pub fn note(&mut self, what: impl Into<String>) {
        *self.0.entry(what.into()).or_default() += 1;
    }

    /// Add every count of `other`.
    pub fn merge(&mut self, other: Losses) {
        for (what, n) in other.0 {
            *self.0.entry(what).or_default() += n;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Each description with its count, in alphabetical order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, usize)> {
        self.0.iter().map(|(what, n)| (what.as_str(), *n))
    }
}

/// What one CityGML version names, looked up from the CityJSON side.
pub trait Vocabulary: Send + Sync {
    /// The class an object of CityJSON type `cityjson_type` is written as,
    /// or `None` when the version has none. `attribute` gives the object's
    /// text attributes by name, for a type that maps to one of several
    /// classes (an installation inside or outside its building).
    fn class(
        &self,
        cityjson_type: &str,
        attribute: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Class>;

    /// The boundary surface class a CityJSON semantic surface of type
    /// `surface` is on an object of class `owner`, or `None` when the
    /// version has none.
    fn surface(&self, surface: &str, owner: &QName) -> Option<QName>;

    /// The property of `class` (a city object's class or a boundary
    /// surface's) an attribute named `name` is, or `None` when it is a
    /// generic attribute. Every class, an ADE's included, has the
    /// properties every city object has (`gml:name`, `core:creationDate`).
    fn property(&self, class: &QName, name: &str) -> Option<Property>;
}

/// A class a CityJSON type is written as.
#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: QName,
    /// The attributes the class itself says, which are therefore not
    /// written (`relationToConstruction` for a CityGML 2.0
    /// `IntBuildingInstallation`).
    pub implied: &'static [&'static str],
}

/// A property of a class that holds a value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Property {
    pub name: &'static str,
    pub ns: PropertyNs,
    pub kind: PropertyKind,
    /// Whether the class allows the property more than once.
    pub repeated: bool,
}

/// Where a [`Property`]'s name comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyNs {
    /// GML itself (`gml:name`).
    Gml,
    /// The CityGML module of the given name.
    Module(crate::model::Module),
}

impl Property {
    pub fn qname(&self) -> QName {
        match self.ns {
            PropertyNs::Gml => QName::new(crate::model::Namespace::Gml, self.name),
            PropertyNs::Module(m) => QName::module(m, self.name),
        }
    }
}

/// What a [`Property`] holds, as far as deciding whether a value fits it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropertyKind {
    /// Any text.
    Text,
    /// A date (`2020-01-31`).
    Date,
    /// A date and time (`2020-01-31T12:00:00Z`).
    DateTime,
    /// A year (`1998`).
    Year,
    /// A whole number, at least 0 when `non_negative`.
    Integer {
        non_negative: bool,
    },
    /// A number; `measure` when the element carries a unit (`uom`).
    Number {
        measure: bool,
    },
    Boolean,
    /// A list of numbers with a unit.
    MeasureList,
    /// One of the given words.
    Enumeration(&'static [&'static str]),
}
