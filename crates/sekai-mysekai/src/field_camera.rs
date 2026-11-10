//! The camera that follows the player.
//!
//! [`FieldCamera`] produces the `field-camera` form, specified in `docs/spec/field-camera.md`.

use sekai_convert::roots::field_camera;
use sekai_convert::{
    Asset, Context, ConvertError, Converter, Object, ObjectId, ObjectIndex, Value,
};
use serde_json::json;

use crate::scripts::{document, malformed};
use crate::stored::{CURVE_KEY, Flag, Kind, Layout, List, Vector, XYZ, array, field, fields};

const SCRIPT: &str = "Assembly-CSharp/Sekai.Mysekai.FieldCamera";

const VIEW: Layout = &[
    ("nearClipPlane", "near clip plane", Kind::Number),
    ("farClipPlane", "far clip plane", Kind::Number),
    ("orthographic", "orthographic", Flag),
];

const SETTING: Layout = &[
    ("offset", "offset", Vector(XYZ)),
    ("distance", "distance", Kind::Number),
    ("minDistance", "minDistance", Kind::Number),
    ("maxDistance", "maxDistance", Kind::Number),
    ("initYaw", "initYaw", Kind::Number),
    ("initPitch", "initPitch", Kind::Number),
    ("minPitch", "minPitch", Kind::Number),
    ("maxPitch", "maxPitch", Kind::Number),
    ("rotSensitivity", "rotSensitivity", Kind::Number),
    ("fov", "fov", Kind::Number),
];

const CAMERA_PARAM: Layout = &[
    ("_distance", "_distance.m_Curve", List(CURVE_KEY)),
    ("_yaw", "_yaw.m_Curve", List(CURVE_KEY)),
    ("_pitch", "_pitch.m_Curve", List(CURVE_KEY)),
    ("_movePathSpeed", "_movePathSpeed.m_Curve", List(CURVE_KEY)),
    ("_movePathHigh", "_movePathHigh.m_Curve", List(CURVE_KEY)),
    (
        "_moveCameraXPath",
        "_moveCameraXPath.m_Curve",
        List(CURVE_KEY),
    ),
    (
        "_moveCameraZPath",
        "_moveCameraZPath.m_Curve",
        List(CURVE_KEY),
    ),
];

/// The field camera as a JSON document.
#[derive(Debug, Clone, Copy, Default)]
pub struct FieldCamera;

impl Converter for FieldCamera {
    fn name(&self) -> &'static str {
        "field-camera"
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
        field_camera(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let camera = behaviour(root, cx)?;
        let linked = |name: &str, layout: Layout| {
            let object = target(cx, camera.id, field(&camera.fields, name)?, name)?;
            fields(&object.fields, layout)
        };
        document(&json!({
            "view": linked("_view", VIEW)?,
            "setting": linked("_setting", SETTING)?,
            "cameraParam": linked("_cameraParam", CAMERA_PARAM)?,
        }))
        .map(Some)
    }
}

/// The `FieldCamera` behaviour of the game object `root`.
fn behaviour(root: ObjectId, cx: &dyn Context) -> Result<Object, ConvertError> {
    let game_object = cx.object(root)?;
    for entry in array(field(&game_object.fields, "m_Component")?, "m_Component")? {
        let component = target(cx, root, field(entry, "component")?, "component")?;
        if component.script.as_deref() == Some(SCRIPT) {
            return Ok(component);
        }
    }
    Err(malformed("the game object has no FieldCamera behaviour"))
}

/// The object that the reference `value`, the field `name` of the object `from`, points to.
fn target(
    cx: &dyn Context,
    from: ObjectId,
    value: &Value,
    name: &str,
) -> Result<Object, ConvertError> {
    let reference = value
        .as_pptr()
        .ok_or_else(|| malformed(format!("{name} is not a reference")))?;
    let id = cx
        .resolve(from, reference)?
        .ok_or_else(|| malformed(format!("{name} is null")))?;
    cx.object(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripts::testing::{json_of, structure};
    use sekai_convert::{Chunk, PPtr};
    use serde_json::json;
    use std::collections::HashMap;

    const GAME_OBJECT: i64 = 1;
    const TRANSFORM: i64 = 2;
    const BEHAVIOUR: i64 = 3;
    const VIEW_OBJECT: i64 = 4;
    const SETTING_OBJECT: i64 = 5;
    const CURVES_OBJECT: i64 = 6;

    fn id(path_id: i64) -> ObjectId {
        ObjectId {
            bundle: 0,
            file: 0,
            path_id,
        }
    }

    fn pointer(path_id: i64) -> Value {
        Value::PPtr(PPtr {
            file_id: 0,
            path_id,
        })
    }

    fn component(path_id: i64) -> Value {
        structure(&[("component", pointer(path_id))])
    }

    struct Objects(HashMap<i64, (Option<&'static str>, Value)>);

    impl Context for Objects {
        fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
            let (script, fields) = self.0.get(&id.path_id).expect("a known object");
            Ok(Object {
                id,
                class_id: 114,
                class: "MonoBehaviour".to_owned(),
                script: script.map(str::to_owned),
                fields: fields.clone(),
            })
        }

        fn resolve(
            &self,
            _from: ObjectId,
            reference: PPtr,
        ) -> Result<Option<ObjectId>, ConvertError> {
            Ok((reference.path_id != 0).then(|| id(reference.path_id)))
        }

        fn resource(
            &self,
            _from: ObjectId,
            path: &str,
            _offset: u64,
            _size: u64,
        ) -> Result<Chunk<'_>, ConvertError> {
            Err(ConvertError::new("missing_asset", path))
        }

        fn container(&self, _from: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
            Ok(Vec::new())
        }

        fn file_name(&self, _from: ObjectId) -> Result<&str, ConvertError> {
            Ok("CAB-test")
        }
    }

    fn key() -> Value {
        structure(&[
            ("time", Value::Float(1.0)),
            ("value", Value::Float(2.0)),
            ("inSlope", Value::Float(f64::INFINITY)),
            ("outSlope", Value::Float(0.0)),
            ("weightedMode", Value::Int(0)),
            ("inWeight", Value::Float(0.0)),
            ("outWeight", Value::Float(0.0)),
        ])
    }

    fn curve(keys: Vec<Value>) -> Value {
        structure(&[("m_Curve", Value::Array(keys))])
    }

    fn setting() -> Value {
        let offset = structure(&[
            ("x", Value::Float(1.0)),
            ("y", Value::Float(2.0)),
            ("z", Value::Float(3.0)),
        ]);
        let number = |name| (name, Value::Float(8.0));
        structure(&[
            ("offset", offset),
            number("distance"),
            number("minDistance"),
            number("maxDistance"),
            number("initYaw"),
            number("initPitch"),
            number("minPitch"),
            number("maxPitch"),
            number("rotSensitivity"),
            number("fov"),
        ])
    }

    fn curves() -> Value {
        let empty = |name| (name, curve(Vec::new()));
        structure(&[
            ("_distance", curve(vec![key()])),
            empty("_yaw"),
            empty("_pitch"),
            empty("_movePathSpeed"),
            empty("_movePathHigh"),
            empty("_moveCameraXPath"),
            empty("_moveCameraZPath"),
        ])
    }

    fn prefab(components: Vec<Value>) -> Objects {
        let view = structure(&[
            ("near clip plane", Value::Float(0.5)),
            ("far clip plane", Value::Float(1000.0)),
            ("orthographic", Value::Bool(false)),
        ]);
        let camera = structure(&[
            ("_view", pointer(VIEW_OBJECT)),
            ("_setting", pointer(SETTING_OBJECT)),
            ("_cameraParam", pointer(CURVES_OBJECT)),
        ]);
        let game_object = structure(&[("m_Component", Value::Array(components))]);
        Objects(HashMap::from([
            (GAME_OBJECT, (None, game_object)),
            (TRANSFORM, (None, structure(&[]))),
            (BEHAVIOUR, (Some(SCRIPT), camera)),
            (VIEW_OBJECT, (None, view)),
            (SETTING_OBJECT, (None, setting())),
            (CURVES_OBJECT, (None, curves())),
        ]))
    }

    #[test]
    fn the_document_joins_the_three_objects() {
        let objects = prefab(vec![component(TRANSFORM), component(BEHAVIOUR)]);
        let asset = FieldCamera
            .convert(id(GAME_OBJECT), &objects)
            .unwrap()
            .unwrap();
        let value = json_of(&asset);
        assert_eq!(
            value["view"],
            json!({"nearClipPlane": 0.5, "farClipPlane": 1000.0, "orthographic": false})
        );
        assert_eq!(value["setting"]["offset"], json!([1.0, 2.0, 3.0]));
        assert_eq!(value["setting"]["fov"], 8.0);
        assert_eq!(value["cameraParam"]["_yaw"], json!([]));
        let first = &value["cameraParam"]["_distance"][0];
        assert_eq!(first["value"], 2.0);
        assert_eq!(first["inSlope"], "Infinity");
    }

    #[test]
    fn a_game_object_without_the_behaviour_is_an_error() {
        let objects = prefab(vec![component(TRANSFORM)]);
        let error = FieldCamera.convert(id(GAME_OBJECT), &objects).unwrap_err();
        assert_eq!(error.code, "malformed");
    }
}
