//! Virtual lives.
//!
//! [`VirtualLiveScript`] produces the `virtual-live-script` form, specified in
//! `docs/spec/virtual-live-script.md`, and [`VirtualLiveTimeline`] the `virtual-live-timeline`
//! form, specified in `docs/spec/virtual-live-timeline.md`.

use sekai_convert::fields::fields_document;
use sekai_convert::{Asset, Context, ConvertError, Converter, ObjectId, ObjectIndex};

use crate::graph::{GraphDocument, fields_only};
use crate::roots;

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

/// The timeline of the master of ceremonies of a virtual live with every object it refers to, as
/// a JSON document of their stored fields.
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtualLiveTimeline;

/// The document of the form: the objects the timeline reaches, with their fields alone.
const TIMELINE: GraphDocument = GraphDocument {
    format: "virtual-live-timeline",
    version: 1,
    role: "timeline",
    media: fields_only,
};

impl Converter for VirtualLiveTimeline {
    fn name(&self) -> &'static str {
        "virtual-live-timeline"
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
        roots::virtual_live_timeline(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        TIMELINE.asset(root, cx).map(Some)
    }
}
