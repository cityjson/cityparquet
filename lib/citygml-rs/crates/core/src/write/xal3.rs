//! An [`Address`] as an xAL 3 `xAL:Address`, CityGML 3.0's.
//!
//! The model names an address's leaves as xAL 2.0 does
//! (`ThoroughfareName`), so each is written where xAL 3 keeps the same
//! part: a `ThoroughfareName` as a `Thoroughfare`'s `NameElement`. The
//! parts follow xAL 3's sequence, and a part's leaves keep their order.
//! A leaf xAL 3 has no place for, or whose part needs a name the address
//! does not give (a subarea without its area), is left out, and reported.

use std::io::Write;

use super::XmlWriter;
use crate::Result;
use crate::model::Address;

/// Where each xAL 2.0 leaf goes: the part holding it, the part inside
/// that if any, and the element.
const PLACES: &[(&str, &str, Option<&str>, &str)] = &[
    ("AddressLine", "FreeTextAddress", None, "AddressLine"),
    ("CountryName", "Country", None, "NameElement"),
    (
        "AdministrativeAreaName",
        "AdministrativeArea",
        None,
        "NameElement",
    ),
    (
        "SubAdministrativeAreaName",
        "AdministrativeArea",
        Some("SubAdministrativeArea"),
        "NameElement",
    ),
    ("LocalityName", "Locality", None, "NameElement"),
    (
        "DependentLocalityName",
        "Locality",
        Some("SubLocality"),
        "NameElement",
    ),
    ("ThoroughfareName", "Thoroughfare", None, "NameElement"),
    ("ThoroughfareNumber", "Thoroughfare", None, "Number"),
    ("PremiseName", "Premises", None, "NameElement"),
    ("PremiseNumber", "Premises", None, "Number"),
    ("PostalCodeNumber", "PostCode", None, "Identifier"),
];

/// xAL 3's parts, in the order its `Address` holds them.
const PARTS: &[&str] = &[
    "FreeTextAddress",
    "Country",
    "AdministrativeArea",
    "Locality",
    "Thoroughfare",
    "Premises",
    "PostCode",
];

/// Write `address`'s `xAL:Address`; returns the names of the leaves it
/// left out.
pub fn address<W: Write>(x: &mut XmlWriter<W>, address: &Address) -> Result<Vec<String>> {
    let mut left_out = Vec::new();
    // Each leaf written, with where it goes.
    let mut placed: Vec<(&str, Option<&str>, &str, &str)> = Vec::new();
    for (name, value) in &address.fields {
        match PLACES.iter().find(|(leaf, ..)| leaf == name) {
            Some((_, part, inner, element)) => placed.push((part, *inner, element, value)),
            None => left_out.push(name.clone()),
        }
    }
    // A part inside another needs the outer part's own name, which xAL 3
    // requires.
    let named = named_parts(address);
    placed.retain(|(part, inner, ..)| {
        let written = inner.is_none() || named.contains(part);
        if !written {
            left_out.push(format!("{} without its {part}", inner.unwrap_or_default()));
        }
        written
    });
    x.open("xAL:Address", &[])?;
    for part in PARTS {
        if !placed.iter().any(|(p, ..)| p == part) {
            continue;
        }
        x.open(&format!("xAL:{part}"), &[])?;
        for (_, _, element, value) in placed.iter().filter(|(p, i, ..)| p == part && i.is_none()) {
            x.leaf(&format!("xAL:{element}"), &[], value)?;
        }
        let mut inners: Vec<&str> = Vec::new();
        for (_, inner, ..) in placed.iter().filter(|(p, ..)| p == part) {
            if let Some(i) = inner
                && !inners.contains(i)
            {
                inners.push(i);
            }
        }
        for inner in inners {
            x.open(&format!("xAL:{inner}"), &[])?;
            for (_, _, element, value) in placed
                .iter()
                .filter(|(p, i, ..)| p == part && *i == Some(inner))
            {
                x.leaf(&format!("xAL:{element}"), &[], value)?;
            }
            x.close()?;
        }
        x.close()?;
    }
    x.close()?;
    Ok(left_out)
}

/// The parts whose own name the address gives.
fn named_parts(address: &Address) -> Vec<&'static str> {
    address
        .fields
        .iter()
        .filter_map(|(name, _)| {
            PLACES
                .iter()
                .find(|(leaf, _, inner, element)| {
                    leaf == name && inner.is_none() && *element == "NameElement"
                })
                .map(|(_, part, ..)| *part)
        })
        .collect()
}
