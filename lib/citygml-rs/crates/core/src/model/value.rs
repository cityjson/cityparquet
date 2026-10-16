use super::QName;

/// One attribute of a city object.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: AttributeName,
    pub value: Value,
}

/// How an attribute is named.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeName {
    /// A property the class declares (`bldg:function`, `wtr:class`).
    Property(QName),
    /// A generic attribute (`gen:stringAttribute name="…"`), by its `name`.
    Generic(String),
}

impl AttributeName {
    /// The name without its namespace: a property's local name, or a generic
    /// attribute's `name`.
    pub fn local(&self) -> &str {
        match self {
            Self::Property(q) => &q.local,
            Self::Generic(name) => name,
        }
    }
}

/// An attribute value, typed as the reader understood it. A value that was
/// declared numeric but does not parse stays text.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    String(String),
    Integer(i64),
    Double(f64),
    Boolean(bool),
    /// A list property (`storeyHeightsAboveGround`), each item typed.
    List(Vec<Value>),
    /// A set of named values (`gen:genericAttributeSet`), in document order.
    Set(Vec<Attribute>),
}
