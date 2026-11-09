//! Scenario documents.
//!
//! [`Scenario`] produces the `scenario` form, specified in `docs/spec/scenario.md`.

use crate::roots;
use sekai_convert::fields::fields_document;
use sekai_convert::{Asset, Context, ConvertError, Converter, ObjectId, ObjectIndex};

/// The scene data of a story as a JSON document.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scenario;

impl Converter for Scenario {
    fn name(&self) -> &'static str {
        "scenario"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        roots::scenario(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        fields_document(root, cx, "scenario").map(Some)
    }
}
