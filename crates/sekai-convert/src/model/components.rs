//! Component documents: components of a prefab, with references as node paths.

use std::collections::BTreeMap;

use sekai_format::components::{Component, ComponentDocument, Field, Fields, Reference};

use super::prefab::Prefab;
use super::read::{flag, malformed, unsupported};
use crate::{Context, ConvertError, Object, ObjectId, Value};

/// Fields that the keys of a component already say, or that name its script.
const ENGINE_FIELDS: &[&str] = &["m_GameObject", "m_Enabled", "m_Script", "m_Name"];

/// The component document of the components of `prefab` that `listed` selects, specified in
/// `docs/spec/component-documents.md`, or `None` when it selects none.
pub fn component_document(
    prefab: &Prefab,
    listed: impl Fn(&Object) -> bool,
    cx: &dyn Context,
) -> Result<Option<ComponentDocument>, ConvertError> {
    let chosen: Vec<(&str, &Object)> = prefab
        .nodes
        .iter()
        .flat_map(|node| {
            node.components
                .iter()
                .filter(|c| listed(c))
                .map(|c| (node.path.as_str(), c))
        })
        .collect();
    if chosen.is_empty() {
        return Ok(None);
    }
    let positions: BTreeMap<ObjectId, usize> = chosen
        .iter()
        .enumerate()
        .map(|(position, (_, c))| (c.id, position))
        .collect();
    let references = References {
        prefab,
        positions: &positions,
        cx,
    };
    let components = chosen
        .into_iter()
        .map(|(node, object)| {
            let Value::Struct(stored) = &object.fields else {
                return Err(malformed("the fields of a component are not a structure"));
            };
            let fields = stored
                .iter()
                .filter(|(name, _)| !ENGINE_FIELDS.contains(&name.as_str()))
                .map(|(name, value)| Ok((name.clone(), references.field(object.id, value)?)))
                .collect::<Result<_, ConvertError>>()?;
            Ok(Component {
                class: object.class.clone(),
                script: object.script.clone(),
                node: node.to_owned(),
                enabled: flag(&object.fields, "m_Enabled")?,
                fields: Fields(fields),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(Some(ComponentDocument::new(components)))
}

/// How the references of the listed components are written.
struct References<'a> {
    prefab: &'a Prefab,
    /// The position of each listed component in the document.
    positions: &'a BTreeMap<ObjectId, usize>,
    cx: &'a dyn Context,
}

impl References<'_> {
    /// The field `value` of the component `from`.
    fn field(&self, from: ObjectId, value: &Value) -> Result<Field, ConvertError> {
        Ok(match value {
            Value::Bool(v) => Field::Bool(*v),
            Value::Int(v) => Field::Int(*v),
            Value::UInt(v) => Field::UInt(*v),
            Value::Float(v) => Field::Float(*v),
            Value::String(v) => Field::String(v.clone()),
            Value::Bytes(_) => return Err(unsupported("a component holds bytes")),
            Value::Array(items) => Field::Array(
                items
                    .iter()
                    .map(|item| self.field(from, item))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Map(pairs) => Field::Map(
                pairs
                    .iter()
                    .map(|(k, v)| Ok((self.field(from, k)?, self.field(from, v)?)))
                    .collect::<Result<_, ConvertError>>()?,
            ),
            Value::Struct(fields) => Field::Struct(Fields(
                fields
                    .iter()
                    .map(|(name, v)| Ok((name.clone(), self.field(from, v)?)))
                    .collect::<Result<_, ConvertError>>()?,
            )),
            Value::PPtr(pointer) => match self.cx.resolve(from, *pointer)? {
                None => Field::Null,
                Some(target) => Field::Ref(self.reference(target)?),
            },
        })
    }

    /// What a reference to `target` names.
    fn reference(&self, target: ObjectId) -> Result<Reference, ConvertError> {
        if let Some(&position) = self.positions.get(&target) {
            return Ok(Reference::Component(position));
        }
        if let Some(node) = self.prefab.node_of(target) {
            return Ok(Reference::Node(self.prefab.nodes[node].path.clone()));
        }
        let (node, component) = self
            .prefab
            .component(target)
            .ok_or_else(|| unsupported("a component refers to an object outside the prefab"))?;
        Ok(Reference::Unlisted {
            node: self.prefab.nodes[node].path.clone(),
            class: component.class.clone(),
        })
    }
}
