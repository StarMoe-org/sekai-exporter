//! Component documents, specified in `docs/spec/component-documents.md`.

use serde::{Serialize, Serializer};

use crate::number::serialize_stored;

/// The value of the `format` key.
pub const FORMAT: &str = "components";
/// The version of the document format.
pub const VERSION: u32 = 1;

/// Components of a prefab with their stored fields, referring to the prefab's nodes by path.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComponentDocument {
    /// Always [`FORMAT`].
    pub format: &'static str,
    /// Always [`VERSION`].
    pub version: u32,
    /// The components.
    pub components: Vec<Component>,
}

impl ComponentDocument {
    /// The document of `components`.
    pub fn new(components: Vec<Component>) -> Self {
        Self {
            format: FORMAT,
            version: VERSION,
            components,
        }
    }

    /// The bytes of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }
}

/// One component.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Component {
    /// The class name.
    pub class: String,
    /// For a script component, `"<assembly>/<namespace>.<class>"`.
    pub script: Option<String>,
    /// The path of its game object.
    pub node: String,
    /// Whether it is enabled.
    pub enabled: bool,
    /// Its stored fields.
    pub fields: Fields,
}

/// The fields of a structure, in stored order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fields(pub Vec<(String, Field)>);

impl Serialize for Fields {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(name, value)| (name, value)))
    }
}

/// A stored value.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    /// A null reference.
    Null,
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    UInt(u64),
    /// A `float` or `double`.
    Float(f64),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<Field>),
    /// A map, as its pairs in stored order.
    Map(Vec<(Field, Field)>),
    /// A structure.
    Struct(Fields),
    /// A reference to an object of the prefab.
    Ref(Reference),
}

/// What a reference names.
#[derive(Debug, Clone, PartialEq)]
pub enum Reference {
    /// A game object, or its transform, by path.
    Node(String),
    /// A component of the document, by index.
    Component(usize),
    /// A component the document does not list, by the path of its game object and its class.
    Unlisted {
        /// The path of its game object.
        node: String,
        /// Its class name.
        class: String,
    },
}

impl Serialize for Field {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(v) => serializer.serialize_bool(*v),
            Self::Int(v) => serializer.serialize_i64(*v),
            Self::UInt(v) => serializer.serialize_u64(*v),
            Self::Float(v) => serialize_stored(*v, serializer),
            Self::String(v) => serializer.serialize_str(v),
            Self::Array(items) => serializer.collect_seq(items),
            Self::Map(pairs) => serializer.collect_seq(pairs.iter().map(|(k, v)| [k, v])),
            Self::Struct(fields) => fields.serialize(serializer),
            Self::Ref(target) => RefJson {
                target: target.into(),
            }
            .serialize(serializer),
        }
    }
}

/// `{ "$ref": { … } }`.
#[derive(Serialize)]
struct RefJson<'a> {
    #[serde(rename = "$ref")]
    target: Target<'a>,
}

#[derive(Serialize)]
struct Target<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    node: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    component: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<&'a str>,
}

impl<'a> From<&'a Reference> for Target<'a> {
    fn from(reference: &'a Reference) -> Self {
        match reference {
            Reference::Node(node) => Self {
                node: Some(node),
                component: None,
                class: None,
            },
            Reference::Component(index) => Self {
                node: None,
                component: Some(*index),
                class: None,
            },
            Reference::Unlisted { node, class } => Self {
                node: Some(node),
                component: None,
                class: Some(class),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bytes_are_pinned() {
        let fields = |items: Vec<(&str, Field)>| {
            Fields(items.into_iter().map(|(n, v)| (n.to_owned(), v)).collect())
        };
        let doc = ComponentDocument::new(vec![
            Component {
                class: "MonoBehaviour".into(),
                script: Some("A/B.SpringManager".into()),
                node: String::new(),
                enabled: true,
                fields: fields(vec![
                    (
                        "springBones",
                        Field::Array(vec![Field::Ref(Reference::Component(1))]),
                    ),
                    ("dynamicRatio", Field::Float(f64::from(0.1f32))),
                    ("bad", Field::Float(f64::NAN)),
                ]),
            },
            Component {
                class: "AimConstraint".into(),
                script: None,
                node: "Hip/Skirt".into(),
                enabled: false,
                fields: fields(vec![
                    ("m_WorldUpObject", Field::Null),
                    (
                        "m_Sources",
                        Field::Array(vec![Field::Struct(fields(vec![
                            ("sourceTransform", Field::Ref(Reference::Node("Hip".into()))),
                            ("weight", Field::Float(1.0)),
                        ]))]),
                    ),
                    (
                        "renderer",
                        Field::Ref(Reference::Unlisted {
                            node: "Hip".into(),
                            class: "SkinnedMeshRenderer".into(),
                        }),
                    ),
                    (
                        "table",
                        Field::Map(vec![(Field::String("k".into()), Field::UInt(2))]),
                    ),
                ]),
            },
        ]);
        assert_eq!(
            std::str::from_utf8(&doc.to_bytes().unwrap()).unwrap(),
            r#"{"format":"components","version":1,"components":[{"class":"MonoBehaviour","script":"A/B.SpringManager","node":"","enabled":true,"fields":{"springBones":[{"$ref":{"component":1}}],"dynamicRatio":0.1,"bad":"NaN"}},{"class":"AimConstraint","script":null,"node":"Hip/Skirt","enabled":false,"fields":{"m_WorldUpObject":null,"m_Sources":[{"sourceTransform":{"$ref":{"node":"Hip"}},"weight":1.0}],"renderer":{"$ref":{"node":"Hip","class":"SkinnedMeshRenderer"}},"table":[["k",2]]}}]}"#
        );
    }
}
