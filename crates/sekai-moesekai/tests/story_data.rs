//! The `action-set` and `scenario-background` forms on objects built in memory.

#![cfg(feature = "episode")]

use sekai_convert::{
    Asset, Chunk, Content, Context, ConvertError, Converter, Object, ObjectId, ObjectIndex, PPtr,
    Value,
};
use sekai_format::story::{ActionSet, ActionSetDetail, ScenarioBackground};
use sekai_moesekai::story_data::{ActionSetForm, ScenarioBackgroundForm};

const ACTION_SET_SCRIPT: &str = "Assembly-CSharp/Sekai.ActionSetData";
const BACKGROUND_SCRIPT: &str = "Assembly-CSharp/Sekai.ScenarioBackgroundBuildData";
const MOVIE_SCRIPT: &str = "Assembly-CSharp/Sekai.ScenarioBackgroundMovie";
const OTHER_SCRIPT: &str = "Assembly-CSharp/Sekai.OtherBackgroundComponent";

const ROOT: ObjectId = ObjectId {
    bundle: 0,
    file: 0,
    path_id: 100,
};

/// One bundle with one file, holding the given objects.
struct Bundle {
    objects: Vec<Object>,
}

impl Context for Bundle {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        self.objects
            .iter()
            .find(|o| o.id == id)
            .cloned()
            .ok_or_else(|| ConvertError::new("missing_object", "no such object"))
    }

    fn resolve(&self, _: ObjectId, reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        if reference.path_id == 0 {
            return Ok(None);
        }
        let target = id(reference.path_id);
        if self.objects.iter().any(|o| o.id == target) {
            Ok(Some(target))
        } else {
            Err(ConvertError::new("missing_object", "no such object"))
        }
    }

    fn resource(&self, _: ObjectId, path: &str, _: u64, _: u64) -> Result<Chunk<'_>, ConvertError> {
        Err(ConvertError::new("missing_resource", path))
    }

    fn container(&self, _: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        Ok(Vec::new())
    }

    fn file_name(&self, _: ObjectId) -> Result<&str, ConvertError> {
        Ok("CAB-test")
    }
}

impl Bundle {
    fn new(objects: Vec<Object>) -> Self {
        Self { objects }
    }

    fn replace(&mut self, path_id: i64, edit: impl FnOnce(&mut Object)) {
        let object = self
            .objects
            .iter_mut()
            .find(|o| o.id.path_id == path_id)
            .expect("the object exists");
        edit(object);
    }
}

fn id(path_id: i64) -> ObjectId {
    ObjectId {
        bundle: 0,
        file: 0,
        path_id,
    }
}

fn s(fields: Vec<(&str, Value)>) -> Value {
    Value::Struct(fields.into_iter().map(|(n, v)| (n.into(), v)).collect())
}

fn text(v: &str) -> Value {
    Value::String(v.into())
}

fn pptr(path_id: i64) -> Value {
    Value::PPtr(PPtr {
        file_id: 0,
        path_id,
    })
}

fn null() -> Value {
    pptr(0)
}

fn references(ids: &[i64]) -> Value {
    Value::Array(ids.iter().map(|&c| pptr(c)).collect())
}

fn object(path_id: i64, class_id: i32, fields: Value) -> Object {
    Object {
        id: id(path_id),
        class_id,
        class: String::new(),
        script: None,
        fields,
    }
}

fn mono(path_id: i64, script: &str, fields: Value) -> Object {
    let mut object = object(path_id, 114, fields);
    object.script = Some(script.into());
    object
}

/// A game object (class 1) whose components are `components`.
fn game_object(path_id: i64, components: &[i64]) -> Object {
    object(path_id, 1, s(vec![("m_Component", references(components))]))
}

/// A transform of `game_object` whose children are `children`.
fn transform(path_id: i64, game_object: i64, children: &[i64]) -> Object {
    object(
        path_id,
        4,
        s(vec![
            ("m_GameObject", pptr(game_object)),
            ("m_Children", references(children)),
        ]),
    )
}

fn movie(path_id: i64, name: &str) -> Object {
    mono(
        path_id,
        MOVIE_SCRIPT,
        s(vec![("movieAssetBundleName", text(name))]),
    )
}

fn set_field(object: &mut Object, name: &str, value: Value) {
    let Value::Struct(fields) = &mut object.fields else {
        panic!("not a structure");
    };
    fields.retain(|(n, _)| n != name);
    fields.push((name.into(), value));
}

fn bytes(asset: &Asset) -> &[u8] {
    assert_eq!(asset.members.len(), 1);
    let Content::File { extension, bytes } = &asset.members[0].content else {
        panic!("not a file");
    };
    assert_eq!(extension, "json");
    bytes
}

fn index(class_id: i32) -> ObjectIndex {
    ObjectIndex {
        path_id: 1,
        class_id,
        class: String::new(),
        name: None,
        script: None,
        script_name: None,
        references: Vec::new(),
        size: 0,
        sha256: [0; 32],
        error: None,
    }
}

// `action-set`

fn detail(character: i64, costume: i64) -> Value {
    s(vec![
        ("AreaCharacterId", Value::Int(character)),
        ("Costume2dGroupId", Value::Int(costume)),
    ])
}

fn action_set(fields: Vec<(&str, Value)>) -> Bundle {
    Bundle::new(vec![mono(ROOT.path_id, ACTION_SET_SCRIPT, s(fields))])
}

fn action_set_bundle() -> Bundle {
    action_set(vec![
        ("m_GameObject", null()),
        ("ActionSetId", Value::Int(7001)),
        ("AreaId", Value::UInt(12)),
        (
            "Details",
            Value::Array(vec![detail(3, 31), detail(1, -2), detail(4, 40)]),
        ),
        ("Unread", Value::Array(vec![Value::Int(1)])),
    ])
}

/// The action set bundle with the top-level field `name` set to `value`, or removed when
/// `value` is `None`.
fn action_set_with(name: &str, value: Option<Value>) -> Bundle {
    let mut bundle = action_set_bundle();
    bundle.replace(ROOT.path_id, |o| {
        let Value::Struct(fields) = &mut o.fields else {
            panic!("not a structure");
        };
        fields.retain(|(n, _)| n != name);
        if let Some(value) = value {
            fields.push((name.into(), value));
        }
    });
    bundle
}

fn action_set_error(name: &str, value: Option<Value>) -> ConvertError {
    ActionSetForm
        .convert(ROOT, &action_set_with(name, value))
        .expect_err("a conversion error")
}

#[test]
fn the_data_of_the_action_set_script_are_the_roots() {
    assert!(ActionSetForm.is_root(&index(114), Some(ACTION_SET_SCRIPT), &[]));
    assert!(!ActionSetForm.is_root(&index(114), Some(BACKGROUND_SCRIPT), &[]));
    assert!(!ActionSetForm.is_root(&index(114), None, &[]));
    assert!(!ActionSetForm.is_root(&index(1), Some(ACTION_SET_SCRIPT), &[]));
}

#[test]
fn an_action_set_keeps_the_stored_order_of_its_details() {
    assert_eq!(ActionSetForm.name(), "action-set");
    assert_eq!(ActionSetForm.version(), 1);
    let asset = ActionSetForm
        .convert(ROOT, &action_set_bundle())
        .unwrap()
        .unwrap();
    assert_eq!(asset.members[0].role, "action-set");
    assert_eq!(
        ActionSet::from_bytes(bytes(&asset)).unwrap(),
        ActionSet {
            format: "action-set".into(),
            version: 1,
            action_set_id: 7001,
            area_id: 12,
            details: vec![
                ActionSetDetail {
                    area_character_id: 3,
                    costume2d_group_id: 31,
                },
                ActionSetDetail {
                    area_character_id: 1,
                    costume2d_group_id: -2,
                },
                ActionSetDetail {
                    area_character_id: 4,
                    costume2d_group_id: 40,
                },
            ],
        }
    );
}

#[test]
fn an_action_set_without_details_is_a_document_without_them() {
    let bundle = action_set(vec![
        ("ActionSetId", Value::Int(1)),
        ("AreaId", Value::Int(2)),
        ("Details", Value::Array(Vec::new())),
    ]);
    let asset = ActionSetForm.convert(ROOT, &bundle).unwrap().unwrap();
    let document = ActionSet::from_bytes(bytes(&asset)).unwrap();
    assert!(document.details.is_empty());
    assert_eq!(document.action_set_id, 1);
    assert_eq!(document.area_id, 2);
}

#[test]
fn an_action_set_field_that_is_absent_is_malformed() {
    for name in ["ActionSetId", "AreaId", "Details"] {
        let e = action_set_error(name, None);
        assert_eq!(e.code, "malformed", "{name}");
        assert!(e.message.contains(name), "{name}: {}", e.message);
    }
}

#[test]
fn an_action_set_field_of_another_type_is_malformed() {
    let cases = [
        ("ActionSetId", text("7001"), "ActionSetId"),
        ("ActionSetId", Value::Bool(true), "ActionSetId"),
        ("AreaId", s(Vec::new()), "AreaId"),
        ("AreaId", Value::Float(12.0), "AreaId"),
        ("Details", Value::Int(0), "Details"),
        ("Details", Value::Array(vec![Value::Int(3)]), "Details[0]"),
        (
            "Details",
            Value::Array(vec![s(vec![
                ("AreaCharacterId", Value::Int(3)),
                ("Costume2dGroupId", text("31")),
            ])]),
            "Details[0].Costume2dGroupId",
        ),
        (
            "Details",
            Value::Array(vec![s(vec![("Costume2dGroupId", Value::Int(31))])]),
            "Details[0].AreaCharacterId",
        ),
        (
            "Details",
            Value::Array(vec![
                detail(3, 31),
                s(vec![("AreaCharacterId", Value::Int(1))]),
            ]),
            "Details[1].Costume2dGroupId",
        ),
    ];
    for (name, value, field) in cases {
        let e = action_set_error(name, Some(value));
        assert_eq!(e.code, "malformed", "{field}");
        assert!(e.message.contains(field), "{field}: {}", e.message);
    }
}

// `scenario-background`

/// The build data of a background over `prefab`.
fn background(prefab: Value) -> Object {
    mono(
        ROOT.path_id,
        BACKGROUND_SCRIPT,
        s(vec![("BackgroundPrefab", prefab)]),
    )
}

fn background_result(bundle: &Bundle) -> Result<Option<Asset>, ConvertError> {
    ScenarioBackgroundForm.convert(ROOT, bundle)
}

fn stored(bundle: Bundle) -> Vec<String> {
    let asset = background_result(&bundle).unwrap().expect("an asset");
    assert_eq!(asset.members[0].role, "scenario-background");
    ScenarioBackground::from_bytes(bytes(&asset))
        .unwrap()
        .movies
}

/// A prefab whose root (`200`) has three children. The first child (`201`) holds `names[0]`;
/// the second (`202`) holds `names[1]` and has a child (`203`) that holds `names[2]`; the third
/// (`205`) holds `names[3]`. Every name may be empty or repeated. The root also holds a
/// component of the movie script in a class other than `MonoBehaviour`.
fn tree(names: [&str; 4]) -> Bundle {
    Bundle::new(vec![
        background(pptr(200)),
        game_object(200, &[300, 500]),
        transform(300, 200, &[402, 403, 404]),
        game_object(201, &[402, 400]),
        transform(402, 201, &[]),
        movie(400, names[0]),
        game_object(202, &[403, 401]),
        transform(403, 202, &[405]),
        movie(401, names[1]),
        game_object(203, &[405, 406]),
        transform(405, 203, &[]),
        movie(406, names[2]),
        game_object(205, &[404, 408]),
        transform(404, 205, &[]),
        movie(408, names[3]),
        // A component of the movie script in a class other than `MonoBehaviour`.
        object(500, 115, s(vec![("movieAssetBundleName", text("x_loop"))])),
    ])
}

fn names() -> [&'static str; 4] {
    ["a_loop", "b_loop", "c_loop", "d_loop"]
}

fn deactivate(bundle: &mut Bundle, path_id: i64, value: Value) {
    bundle.replace(path_id, |o| set_field(o, "m_IsActive", value));
}

#[test]
fn the_build_data_of_the_scenario_background_script_are_the_roots() {
    assert!(ScenarioBackgroundForm.is_root(&index(114), Some(BACKGROUND_SCRIPT), &[]));
    assert!(!ScenarioBackgroundForm.is_root(&index(114), Some(ACTION_SET_SCRIPT), &[]));
    assert!(!ScenarioBackgroundForm.is_root(&index(114), None, &[]));
    assert!(!ScenarioBackgroundForm.is_root(&index(1), Some(BACKGROUND_SCRIPT), &[]));
}

#[test]
fn the_movies_of_the_prefab_are_found() {
    assert_eq!(ScenarioBackgroundForm.name(), "scenario-background");
    assert_eq!(ScenarioBackgroundForm.version(), 1);
    assert_eq!(stored(tree(names())), names());
}

#[test]
fn the_movies_are_a_document() {
    let bundle = tree(names());
    let asset = background_result(&bundle).unwrap().unwrap();
    assert_eq!(
        ScenarioBackground::from_bytes(bytes(&asset)).unwrap(),
        ScenarioBackground {
            format: "scenario-background".into(),
            version: 1,
            movies: names().iter().map(|n| (*n).into()).collect(),
        }
    );
}

#[test]
fn the_children_are_walked_depth_first_in_sibling_order() {
    // A movie of the root itself comes before those of its children, and a grandchild before
    // the next sibling of its parent.
    let bundle = Bundle::new(vec![
        background(pptr(200)),
        game_object(200, &[300, 600]),
        transform(300, 200, &[402, 403, 301]),
        movie(600, "root_loop"),
        game_object(201, &[402, 400]),
        transform(402, 201, &[405]),
        movie(400, "first_loop"),
        game_object(203, &[405, 406]),
        transform(405, 203, &[]),
        movie(406, "grandchild_loop"),
        game_object(202, &[403, 407]),
        transform(403, 202, &[]),
        movie(407, "second_loop"),
        game_object(204, &[301, 408]),
        transform(301, 204, &[]),
        movie(408, "third_loop"),
    ]);
    assert_eq!(
        stored(bundle),
        [
            "root_loop",
            "first_loop",
            "grandchild_loop",
            "second_loop",
            "third_loop"
        ]
    );
    // A rect transform holds children the same way.
    let mut bundle = tree(names());
    bundle.replace(403, |o| o.class_id = 224);
    assert_eq!(stored(bundle), names());
}

#[test]
fn an_inactive_object_is_skipped_with_its_subtree() {
    let mut bundle = tree(names());
    deactivate(&mut bundle, 202, Value::Bool(false));
    assert_eq!(stored(bundle), ["a_loop", "d_loop"]);
    let mut bundle = tree(names());
    deactivate(&mut bundle, 203, Value::Bool(false));
    assert_eq!(stored(bundle), ["a_loop", "b_loop", "d_loop"]);
    let mut bundle = tree(names());
    deactivate(&mut bundle, 201, Value::Bool(false));
    deactivate(&mut bundle, 205, Value::Bool(false));
    assert_eq!(stored(bundle), ["b_loop", "c_loop"]);
    // An inactive prefab root reaches no movie.
    let mut bundle = tree(names());
    deactivate(&mut bundle, 200, Value::Bool(false));
    assert!(background_result(&bundle).unwrap().is_none());
}

#[test]
fn an_active_flag_is_a_true_boolean_or_a_nonzero_integer() {
    let mut bundle = tree(names());
    deactivate(&mut bundle, 202, Value::UInt(1));
    deactivate(&mut bundle, 205, Value::Int(-1));
    assert_eq!(stored(bundle), names());
    let mut bundle = tree(names());
    deactivate(&mut bundle, 202, Value::UInt(0));
    assert_eq!(stored(bundle), ["a_loop", "d_loop"]);
    // A flag of another type reads as inactive.
    let mut bundle = tree(names());
    deactivate(&mut bundle, 201, null());
    assert_eq!(stored(bundle), ["b_loop", "c_loop", "d_loop"]);
}

#[test]
fn a_movie_reached_twice_is_kept_once() {
    assert_eq!(
        stored(tree(["a_loop", "a_loop", "c_loop", "a_loop"])),
        ["a_loop", "c_loop"]
    );
    // A movie listed twice as a component of one object counts once.
    let mut bundle = tree(names());
    bundle.replace(201, |o| {
        *o = game_object(201, &[402, 400, 400]);
    });
    assert_eq!(stored(bundle), names());
}

#[test]
fn an_empty_movie_name_adds_no_movie() {
    let mut bundle = tree(names());
    bundle.replace(401, |o| {
        let Value::Struct(fields) = &mut o.fields else {
            panic!("not a structure");
        };
        fields[0].1 = text("");
    });
    assert_eq!(stored(bundle), ["a_loop", "c_loop", "d_loop"]);
}

#[test]
fn a_component_of_another_script_is_ignored() {
    let mut bundle = tree(names());
    bundle.replace(400, |o| o.script = Some(OTHER_SCRIPT.into()));
    bundle.replace(406, |o| o.script = None);
    assert_eq!(stored(bundle), ["b_loop", "d_loop"]);
}

#[test]
fn a_background_without_a_prefab_or_without_movies_has_no_asset() {
    let bundle = Bundle::new(vec![background(null())]);
    assert!(background_result(&bundle).unwrap().is_none());
    let bundle = Bundle::new(vec![mono(ROOT.path_id, BACKGROUND_SCRIPT, s(vec![]))]);
    assert!(background_result(&bundle).unwrap().is_none());
    let bundle = Bundle::new(vec![
        background(pptr(200)),
        game_object(200, &[300]),
        transform(300, 200, &[402]),
        game_object(201, &[]),
        transform(402, 201, &[]),
    ]);
    assert!(background_result(&bundle).unwrap().is_none());
    let bundle = Bundle::new(vec![background(pptr(200)), game_object(200, &[])]);
    assert!(background_result(&bundle).unwrap().is_none());
    let bundle = Bundle::new(vec![background(pptr(200)), movie(200, "a_loop")]);
    assert!(background_result(&bundle).unwrap().is_none());
}

#[test]
fn a_cycle_ends() {
    // The root's child is the root itself.
    let bundle = Bundle::new(vec![
        background(pptr(200)),
        game_object(200, &[300]),
        transform(300, 200, &[402]),
        game_object(201, &[402, 400, 401]),
        transform(402, 201, &[302]),
        transform(302, 201, &[402]),
        movie(400, "a_loop"),
        movie(401, "b_loop"),
    ]);
    assert_eq!(stored(bundle), ["a_loop", "b_loop"]);
    // Two game objects that are each other's children.
    let bundle = Bundle::new(vec![
        background(pptr(200)),
        game_object(200, &[300]),
        transform(300, 200, &[402]),
        game_object(201, &[402, 400]),
        transform(402, 201, &[403]),
        game_object(202, &[403, 401]),
        transform(403, 202, &[402]),
        movie(400, "a_loop"),
        movie(401, "b_loop"),
    ]);
    assert_eq!(stored(bundle), ["a_loop", "b_loop"]);
}

#[test]
fn a_reference_to_a_missing_object_is_an_error() {
    let bundle = Bundle::new(vec![background(pptr(999))]);
    let e = background_result(&bundle).unwrap_err();
    assert_eq!(e.code, "missing_object");
    let mut bundle = tree(names());
    bundle.replace(403, |o| {
        *o = transform(403, 202, &[405, 999]);
    });
    let e = background_result(&bundle).unwrap_err();
    assert_eq!(e.code, "missing_object");
    let mut bundle = tree(names());
    bundle.replace(201, |o| {
        *o = game_object(201, &[402, 400, 998]);
    });
    let e = background_result(&bundle).unwrap_err();
    assert_eq!(e.code, "missing_object");
}

#[test]
fn a_field_of_another_type_is_malformed() {
    let bundle = Bundle::new(vec![background(Value::Int(200))]);
    let e = background_result(&bundle).unwrap_err();
    assert_eq!(e.code, "malformed");
    assert!(e.message.contains("BackgroundPrefab"));

    let mut bundle = tree(names());
    bundle.replace(400, |o| set_field(o, "movieAssetBundleName", Value::Int(0)));
    let e = background_result(&bundle).unwrap_err();
    assert_eq!(e.code, "malformed");
    assert!(e.message.contains("movieAssetBundleName"));

    let mut bundle = tree(names());
    bundle.replace(300, |o| set_field(o, "m_Children", Value::Int(0)));
    let e = background_result(&bundle).unwrap_err();
    assert_eq!(e.code, "malformed");
    assert!(e.message.contains("m_Children"));

    let mut bundle = tree(names());
    bundle.replace(200, |o| set_field(o, "m_Component", Value::Int(0)));
    let e = background_result(&bundle).unwrap_err();
    assert_eq!(e.code, "malformed");
    assert!(e.message.contains("m_Component"));
}
