/// A postal address of an object, as the document's address schema (xAL)
/// spells it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Address {
    /// Each leaf of the address subtree that holds text: its local name and
    /// its text, in document order (`CountryName` -> `日本`).
    pub fields: Vec<(String, String)>,
    /// Where the address is (a front door, say): the points of the
    /// address's `MultiPoint`, empty when it gives none.
    pub location: Vec<[f64; 3]>,
}
