//! CityJSON <-> CityParquet address mapping (spec "Addresses"; M5 gap 10).
//!
//! CityJSON deliberately does not prescribe address member names ("the
//! members of an address JSON object are not prescribed, to accommodate the
//! different ways addresses are structured in different countries"). The
//! reserved `address` column, though, keeps a lean, NAMED subset of
//! 3DCityDB v5's `ADDRESS` table (spec "Addresses" / design decision
//! "Addresses stored as a struct attribute"). This module is the single
//! source of truth for which source member name maps to which reserved
//! struct field, so the encoder (source -> struct), the decoder/exporter
//! (struct -> CityJSON, using these SAME canonical names) and the comparator
//! (re-deriving the "expected" struct from a source address entry) can never
//! diverge on what counts as a recognised, "postal" member.
//!
//! `country`, `locality`, `thoroughfareName`, `thoroughfareNumber`,
//! `postcode`, and `location` follow CityJSON's own documented worked
//! example verbatim (cityjson.org/specs/2.0.1/#address, xAL-derived):
//!
//! ```json
//! "address": [{
//!   "country": "Canada",
//!   "locality": "Chibougamau",
//!   "thoroughfareNumber": "1",
//!   "thoroughfareName": "rue de la Patate",
//!   "postcode": "H0H 0H0",
//!   "location": {"type": "MultiPoint", "lod": "1", "boundaries": [231]}
//! }]
//! ```
//!
//! `administrativeArea` (-> `state`), `postBox` (-> `po_box`), and
//! `freeText` (-> `free_text`) have no CityJSON-documented precedent — they
//! are this implementation's own reasonable extension of the reserved
//! struct, not names CityJSON's spec text prescribes. A source spelling any
//! recognised field differently (there is no prescribed schema, and
//! real-world data frequently does — e.g. the common but non-standard
//! `countryName`/`postalCode`) simply does not map — a documented, accepted
//! loss (spec "Addresses" -> "Scope"): only the members outside this lean
//! set are dropped, never the struct itself.

use serde_json::Value;

pub(crate) const STREET_KEY: &str = "thoroughfareName";
pub(crate) const HOUSE_NUMBER_KEY: &str = "thoroughfareNumber";
pub(crate) const PO_BOX_KEY: &str = "postBox";
pub(crate) const ZIP_CODE_KEY: &str = "postcode";
pub(crate) const CITY_KEY: &str = "locality";
pub(crate) const STATE_KEY: &str = "administrativeArea";
pub(crate) const COUNTRY_KEY: &str = "country";
pub(crate) const FREE_TEXT_KEY: &str = "freeText";
pub(crate) const LOCATION_KEY: &str = "location";

/// One address entry's postal fields, mapped from a source's raw JSON member
/// names to the reserved struct's fields (spec "Addresses"). A field is
/// `None` when the source object carries no member under the matching key,
/// or the member's value is not a string (treated as absent, not an error —
/// an address is best-effort metadata, not load-bearing geometry).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AddressPostal {
    pub street: Option<String>,
    pub house_number: Option<String>,
    pub po_box: Option<String>,
    pub zip_code: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub free_text: Option<String>,
}

fn str_member(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Map one source `address[]` entry's recognised members onto
/// [`AddressPostal`]. A non-object entry (malformed input) maps to every
/// field `None`.
pub(crate) fn map_postal_fields(entry: &Value) -> AddressPostal {
    let Some(obj) = entry.as_object() else {
        return AddressPostal::default();
    };
    AddressPostal {
        street: str_member(obj, STREET_KEY),
        house_number: str_member(obj, HOUSE_NUMBER_KEY),
        po_box: str_member(obj, PO_BOX_KEY),
        zip_code: str_member(obj, ZIP_CODE_KEY),
        city: str_member(obj, CITY_KEY),
        state: str_member(obj, STATE_KEY),
        country: str_member(obj, COUNTRY_KEY),
        free_text: str_member(obj, FREE_TEXT_KEY),
    }
}

/// Render one [`AddressPostal`] back into a CityJSON address object's
/// recognised members (export), skipping absent fields — the exact inverse
/// of [`map_postal_fields`]'s key mapping. `location` (a geometry, not a
/// plain string) is the caller's own concern; see `crate::export`.
pub(crate) fn postal_to_members(postal: &AddressPostal) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    let mut set = |key: &str, value: &Option<String>| {
        if let Some(v) = value {
            map.insert(key.to_string(), Value::String(v.clone()));
        }
    };
    set(STREET_KEY, &postal.street);
    set(HOUSE_NUMBER_KEY, &postal.house_number);
    set(PO_BOX_KEY, &postal.po_box);
    set(ZIP_CODE_KEY, &postal.zip_code);
    set(CITY_KEY, &postal.city);
    set(STATE_KEY, &postal.state);
    set(COUNTRY_KEY, &postal.country);
    set(FREE_TEXT_KEY, &postal.free_text);
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_recognised_members_only() {
        let entry = json!({
            "country": "Finland",
            "locality": "Helsinki",
            "administrativeArea": "Uusimaa",
            "thoroughfareName": "Mannerheimintie",
            "thoroughfareNumber": "1",
            "postcode": "00100",
            "postBox": "PO 1",
            "freeText": "note",
            "Country": "unrecognised-casing-must-not-map",
        });
        let postal = map_postal_fields(&entry);
        assert_eq!(postal.country.as_deref(), Some("Finland"));
        assert_eq!(postal.city.as_deref(), Some("Helsinki"));
        assert_eq!(postal.state.as_deref(), Some("Uusimaa"));
        assert_eq!(postal.street.as_deref(), Some("Mannerheimintie"));
        assert_eq!(postal.house_number.as_deref(), Some("1"));
        assert_eq!(postal.zip_code.as_deref(), Some("00100"));
        assert_eq!(postal.po_box.as_deref(), Some("PO 1"));
        assert_eq!(postal.free_text.as_deref(), Some("note"));
    }

    /// The common but non-standard `countryName`/`postalCode` spelling (seen
    /// in some real-world producers) is deliberately NOT recognised — only
    /// CityJSON's actual documented `country`/`postcode` are. This is a
    /// documented, accepted loss (spec "Addresses" -> "Scope"), not a bug.
    #[test]
    fn common_non_standard_country_name_and_postal_code_spelling_is_not_recognised() {
        let entry = json!({
            "countryName": "Finland",
            "postalCode": "00100",
        });
        let postal = map_postal_fields(&entry);
        assert_eq!(postal.country, None);
        assert_eq!(postal.zip_code, None);
    }

    #[test]
    fn non_object_entry_maps_to_all_none() {
        assert_eq!(
            map_postal_fields(&json!("just a string")),
            AddressPostal::default()
        );
        assert_eq!(map_postal_fields(&json!(null)), AddressPostal::default());
    }

    #[test]
    fn unrecognised_member_names_do_not_corrupt_recognised_ones() {
        let entry = json!({"Locality": "Espoo", "locality": "Helsinki"});
        let postal = map_postal_fields(&entry);
        assert_eq!(postal.city.as_deref(), Some("Helsinki"));
    }

    #[test]
    fn postal_to_members_round_trips_the_recognised_names() {
        let entry = json!({
            "country": "Finland",
            "locality": "Helsinki",
        });
        let postal = map_postal_fields(&entry);
        let members = postal_to_members(&postal);
        assert_eq!(map_postal_fields(&Value::Object(members)), postal);
    }

    /// CityJSON 2.0.1's own documented worked example
    /// (cityjson.org/specs/2.0.1/#address), verbatim:
    /// ```json
    /// "address": [{
    ///   "country": "Canada",
    ///   "locality": "Chibougamau",
    ///   "thoroughfareNumber": "1",
    ///   "thoroughfareName": "rue de la Patate",
    ///   "postcode": "H0H 0H0",
    ///   "location": {"type": "MultiPoint", "lod": "1", "boundaries": [231]}
    /// }]
    /// ```
    /// `country` and `postcode` are the real spec vocabulary — NOT
    /// `countryName`/`postalCode`, which this module previously (and
    /// wrongly) recognised instead.
    #[test]
    fn maps_the_real_cityjson_documented_worked_example() {
        let entry = json!({
            "country": "Canada",
            "locality": "Chibougamau",
            "thoroughfareNumber": "1",
            "thoroughfareName": "rue de la Patate",
            "postcode": "H0H 0H0",
        });
        let postal = map_postal_fields(&entry);
        assert_eq!(postal.country.as_deref(), Some("Canada"));
        assert_eq!(postal.city.as_deref(), Some("Chibougamau"));
        assert_eq!(postal.house_number.as_deref(), Some("1"));
        assert_eq!(postal.street.as_deref(), Some("rue de la Patate"));
        assert_eq!(postal.zip_code.as_deref(), Some("H0H 0H0"));
    }
}
