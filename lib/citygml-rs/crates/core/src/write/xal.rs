//! An [`Address`] as an xAL 2.0 `xAL:AddressDetails` tree.
//!
//! The model keeps only an address's leaves, so the tree is rebuilt: one
//! `Country` holding the country's names, its administrative area and
//! subarea if any, and a `Locality` holding the rest, each leaf where xAL's
//! schema allows it. A leaf xAL has no place for here is left out, and
//! reported.

use std::io::Write;

use super::XmlWriter;
use crate::Result;
use crate::model::Address;

/// Where a leaf goes: the containers from `Country` down, and its element.
const PLACES: &[(&str, &[&str])] = &[
    ("AddressLine", &[]),
    ("CountryName", &[]),
    ("AdministrativeAreaName", &["AdministrativeArea"]),
    (
        "SubAdministrativeAreaName",
        &["AdministrativeArea", "SubAdministrativeArea"],
    ),
    ("LocalityName", &["Locality"]),
    ("PostBoxNumber", &["Locality", "PostBox"]),
    ("ThoroughfareNumber", &["Locality", "Thoroughfare"]),
    ("ThoroughfareName", &["Locality", "Thoroughfare"]),
    ("PremiseName", &["Locality", "Premise"]),
    ("PremiseNumber", &["Locality", "Premise"]),
    ("BuildingName", &["Locality", "Premise"]),
    ("DependentLocalityName", &["Locality", "DependentLocality"]),
    ("PostalCodeNumber", &["Locality", "PostalCode"]),
];

/// The order of containers inside their parent, as xAL's sequences put
/// them: a `Locality` inside the innermost administrative area there is.
const ORDER: &[&str] = &[
    "AdministrativeArea",
    "SubAdministrativeArea",
    "Locality",
    "PostBox",
    "Thoroughfare",
    "Premise",
    "DependentLocality",
    "PostalCode",
];

/// Write `address`'s `xAL:AddressDetails`; returns the names of the leaves
/// it left out.
pub fn address_details<W: Write>(x: &mut XmlWriter<W>, address: &Address) -> Result<Vec<String>> {
    let mut left_out = Vec::new();
    // Each leaf with the path of containers it goes in.
    let mut placed: Vec<(Vec<&str>, &str, &str)> = Vec::new();
    let administrative = address
        .fields
        .iter()
        .any(|(k, _)| k == "AdministrativeAreaName" || k == "SubAdministrativeAreaName");
    let sub = address
        .fields
        .iter()
        .any(|(k, _)| k == "SubAdministrativeAreaName");
    for (name, value) in &address.fields {
        let Some((_, path)) = PLACES.iter().find(|(leaf, _)| leaf == name) else {
            left_out.push(name.clone());
            continue;
        };
        let mut full: Vec<&str> = Vec::new();
        if path.first() == Some(&"Locality") {
            if administrative {
                full.push("AdministrativeArea");
            }
            if sub {
                full.push("SubAdministrativeArea");
            }
        }
        full.extend_from_slice(path);
        placed.push((full, name, value));
    }
    x.open("xAL:AddressDetails", &[])?;
    x.open("xAL:Country", &[])?;
    write_level(x, &placed, &[])?;
    x.close()?;
    x.close()?;
    Ok(left_out)
}

/// The leaves directly in the container at `path`, in document order, then
/// each container inside it in xAL's order, written recursively.
fn write_level<W: Write>(
    x: &mut XmlWriter<W>,
    placed: &[(Vec<&str>, &str, &str)],
    path: &[&str],
) -> Result<()> {
    // xAL puts a container's AddressLine first, then its names and numbers;
    // the leaves of one container keep their order otherwise.
    let here = |(p, _, _): &&(Vec<&str>, &str, &str)| p.as_slice() == path;
    for (_, name, value) in placed.iter().filter(here).filter(|l| l.1 == "AddressLine") {
        x.leaf(&format!("xAL:{name}"), &[], value)?;
    }
    for leaf in ORDER_OF_LEAVES {
        for (_, name, value) in placed.iter().filter(here).filter(|l| l.1 == *leaf) {
            x.leaf(&format!("xAL:{name}"), &[], value)?;
        }
    }
    for container in ORDER {
        let mut inner = path.to_vec();
        inner.push(container);
        if placed.iter().any(|(p, _, _)| p.starts_with(&inner)) {
            x.open(&format!("xAL:{container}"), &[])?;
            write_level(x, placed, &inner)?;
            x.close()?;
        }
    }
    Ok(())
}

/// The order of leaves inside one container, as xAL's sequences put them
/// (a `Thoroughfare`'s number before its name, a `Premise`'s name before
/// its number and its building name).
const ORDER_OF_LEAVES: &[&str] = &[
    "CountryName",
    "AdministrativeAreaName",
    "SubAdministrativeAreaName",
    "LocalityName",
    "PostBoxNumber",
    "ThoroughfareNumber",
    "ThoroughfareName",
    "PremiseName",
    "PremiseNumber",
    "BuildingName",
    "DependentLocalityName",
    "PostalCodeNumber",
];
