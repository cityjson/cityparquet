//! What both writers share: an indenting XML writer, GML geometry
//! ([`gml`]) and xAL addresses ([`xal`]).
//!
//! The writers name every element by a prefix the document root declares
//! (`gml`, `xlink`, `xAL`, each module's), so an element is written as the
//! `prefix:local` string it is.

pub mod document;
pub mod gml;
pub mod xal;

use std::io::Write;

use quick_xml::escape::escape;

use crate::{Error, Result};

/// An XML document being written, one element per line, indented by depth.
pub struct XmlWriter<W: Write> {
    out: W,
    /// The elements open, innermost last.
    open: Vec<String>,
}

impl<W: Write> XmlWriter<W> {
    /// Start a document on `out` with its XML declaration.
    pub fn new(mut out: W) -> Result<Self> {
        out.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>"#)
            .map_err(io)?;
        Ok(Self {
            out,
            open: Vec::new(),
        })
    }

    fn indent(&mut self) -> Result<()> {
        self.out.write_all(b"\n").map_err(io)?;
        for _ in 0..self.open.len() {
            self.out.write_all(b" ").map_err(io)?;
        }
        Ok(())
    }

    fn tag(&mut self, name: &str, attributes: &[(&str, &str)]) -> Result<()> {
        self.indent()?;
        write!(self.out, "<{name}").map_err(io)?;
        for (k, v) in attributes {
            write!(self.out, " {k}=\"{}\"", escape(xml_chars(v))).map_err(io)?;
        }
        Ok(())
    }

    /// Open element `name`; [`Self::close`] ends it.
    pub fn open(&mut self, name: &str, attributes: &[(&str, &str)]) -> Result<()> {
        self.tag(name, attributes)?;
        self.out.write_all(b">").map_err(io)?;
        self.open.push(name.to_string());
        Ok(())
    }

    /// End the innermost open element.
    pub fn close(&mut self) -> Result<()> {
        let name = self
            .open
            .pop()
            .ok_or_else(|| Error::invalid("closing an element that is not open"))?;
        self.indent()?;
        write!(self.out, "</{name}>").map_err(io)
    }

    /// An element holding `text`.
    pub fn leaf(&mut self, name: &str, attributes: &[(&str, &str)], text: &str) -> Result<()> {
        self.tag(name, attributes)?;
        write!(self.out, ">{}</{name}>", escape(xml_chars(text))).map_err(io)
    }

    /// An element with no content.
    pub fn empty(&mut self, name: &str, attributes: &[(&str, &str)]) -> Result<()> {
        self.tag(name, attributes)?;
        self.out.write_all(b"/>").map_err(io)
    }

    /// How many elements are open.
    pub fn depth(&self) -> usize {
        self.open.len()
    }

    /// End the document and give back what it was written to.
    pub fn finish(mut self) -> Result<W> {
        if !self.open.is_empty() {
            return Err(Error::invalid(format!(
                "document ended with <{}> open",
                self.open.join("> <")
            )));
        }
        self.out.write_all(b"\n").map_err(io)?;
        self.out.flush().map_err(io)?;
        Ok(self.out)
    }
}

/// `text` without the characters XML 1.0 cannot hold: the control
/// characters other than tab, line feed and carriage return, and U+FFFE and
/// U+FFFF.
fn xml_chars(text: &str) -> std::borrow::Cow<'_, str> {
    let allowed = |c: char| !matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}');
    if text.chars().all(allowed) {
        std::borrow::Cow::Borrowed(text)
    } else {
        std::borrow::Cow::Owned(text.chars().filter(|&c| allowed(c)).collect())
    }
}

/// Whether `name` can be the local part of an XML element or attribute
/// name: an NCName, which begins with a letter or `_` and holds letters,
/// digits, `-`, `_`, `.` and `·`.
pub fn is_ncname(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '·'))
}

fn io(e: std::io::Error) -> Error {
    Error::io("cannot write the CityGML document", e)
}

/// A number as XML Schema's `double` spells it: the shortest text that
/// reads back as the same value.
pub fn number(v: f64) -> String {
    if v.is_finite() {
        format!("{v}")
    } else if v.is_nan() {
        "NaN".to_string()
    } else if v > 0.0 {
        "INF".to_string()
    } else {
        "-INF".to_string()
    }
}

/// Numbers separated by spaces, as a GML coordinate or measure list.
pub fn numbers(values: impl IntoIterator<Item = f64>) -> String {
    let mut out = String::new();
    for v in values {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&number(v));
    }
    out
}
