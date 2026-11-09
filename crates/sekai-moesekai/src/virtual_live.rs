//! Virtual lives.
//!
//! [`VirtualLiveScript`] produces the `virtual-live-script` form, specified in
//! `docs/spec/virtual-live-script.md`.

use sekai_convert::{Asset, Context, ConvertError, Converter, ObjectId, ObjectIndex};

use crate::roots;
use sekai_convert::fields::fields_document;

/// The script of the master of ceremonies of a virtual live, as a JSON document.
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtualLiveScript;

impl Converter for VirtualLiveScript {
    fn name(&self) -> &'static str {
        "virtual-live-script"
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
        roots::virtual_live_script(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        fields_document(root, cx, "script").map(Some)
    }
}
