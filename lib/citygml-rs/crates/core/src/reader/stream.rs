//! What a per-version reader yields, named so that a caller can hold either
//! version's reader behind one type.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::Document;
use crate::{GmlObject, Result};

/// A per-version reader: its objects in document order, with the up-front
/// pass's [`Document`] and what it has passed over so far. Each count is
/// complete only once the stream is drained.
pub trait ObjectStream: Iterator<Item = Result<GmlObject>> {
    /// What the up-front pass read.
    fn document(&self) -> &Arc<Document>;

    /// How many objects the reader has yielded so far.
    fn emitted_members(&self) -> usize;

    /// Members of a class the reader does not read, by the element name the
    /// document spells (`dem:RasterRelief` -> 3).
    fn skipped_members(&self) -> &BTreeMap<String, usize>;

    /// Members of an ADE's class, read, by the element name the document
    /// spells (`urf:UseDistrict` -> 3).
    fn ade_members(&self) -> &BTreeMap<String, usize>;

    /// ADE properties holding GML geometry, which the CityJSON conversion
    /// leaves out, by property name (`uro:lod0Point` -> 3).
    fn ade_geometries(&self) -> &BTreeMap<String, usize>;
}

impl<T: ObjectStream + ?Sized> ObjectStream for Box<T> {
    fn document(&self) -> &Arc<Document> {
        (**self).document()
    }

    fn emitted_members(&self) -> usize {
        (**self).emitted_members()
    }

    fn skipped_members(&self) -> &BTreeMap<String, usize> {
        (**self).skipped_members()
    }

    fn ade_members(&self) -> &BTreeMap<String, usize> {
        (**self).ade_members()
    }

    fn ade_geometries(&self) -> &BTreeMap<String, usize> {
        (**self).ade_geometries()
    }
}
