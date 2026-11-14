//! Animation clips.
//!
//! [`clip_document`] reads an `AnimationClip` into a clip document, specified in
//! `docs/spec/clip-documents.md`. Asset forms that carry animation use it for their members.

use sekai_format::clip::{self, ClipDocument, Curve, CurveData, Event};
use sekai_unity::Value;

use crate::{Context, ConvertError, ObjectId};

/// The class ID of `AnimationClip`.
pub const ANIMATION_CLIP: i32 = 74;
/// The class ID of `Transform`, whose bindings take one curve per component of a property.
const TRANSFORM: i32 = 4;
/// The curve lists of the legacy representation, which clips built for the animator leave empty.
const LEGACY_CURVES: &[&str] = &[
    "m_RotationCurves",
    "m_CompressedRotationCurves",
    "m_EulerCurves",
    "m_PositionCurves",
    "m_ScaleCurves",
    "m_FloatCurves",
    "m_PPtrCurves",
];
/// Binding flags for curves that do not hold floating-point values.
const NON_FLOAT_BINDINGS: &[&str] = &["isPPtrCurve", "isIntCurve", "isSerializeReferenceCurve"];

/// The clip document of the `AnimationClip` `clip`.
pub fn clip_document(clip: ObjectId, cx: &dyn Context) -> Result<ClipDocument, ConvertError> {
    let object = cx.object(clip)?;
    if object.class_id != ANIMATION_CLIP {
        return Err(malformed(format!(
            "{} is not an AnimationClip",
            object.class
        )));
    }
    let fields = &object.fields;
    let name = field(fields, "m_Name")?
        .as_str()
        .ok_or_else(|| malformed("m_Name is not a string"))?
        .to_owned();
    if flag(fields, "m_Legacy")? || flag(fields, "m_Compressed")? {
        return Err(unsupported("the clip is legacy or compressed"));
    }
    for list in LEGACY_CURVES {
        if fields
            .get(list)
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
        {
            return Err(unsupported(format!("the clip has {list}")));
        }
    }
    if flag(fields, "m_HasGenericRootTransform")? || flag(fields, "m_HasMotionFloatCurves")? {
        return Err(unsupported("the clip moves a root transform"));
    }
    let muscle = field(fields, "m_MuscleClip")?;
    if muscle
        .get("m_CycleOffset")
        .map(number)
        .transpose()?
        .is_some_and(|v| v != 0.0)
    {
        return Err(unsupported("m_CycleOffset is not 0"));
    }
    let data = field(field(muscle, "m_Clip")?, "data")?;
    let binding_constant = field(fields, "m_ClipBindingConstant")?;
    if binding_constant
        .get("pptrCurveMapping")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
    {
        return Err(unsupported("the clip has reference curves"));
    }
    let bindings = array(binding_constant, "genericBindings")?
        .iter()
        .map(|b| Binding::read(clip, b, cx))
        .collect::<Result<Vec<_>, _>>()?;

    let mut values = streamed(field(data, "m_StreamedClip")?)?;
    values.extend(dense(field(data, "m_DenseClip")?)?);
    for value in array(field(data, "m_ConstantClip")?, "data")? {
        values.push(CurveData::Constant {
            value: finite(number(value)?, "a constant value")?,
        });
    }
    let taken: usize = bindings.iter().map(|b| usize::from(b.width)).sum();
    if values.len() != taken {
        return Err(malformed(format!(
            "{} curves for bindings that take {taken}",
            values.len()
        )));
    }
    let curves = bindings
        .iter()
        .flat_map(|binding| (0..binding.width).map(move |component| (binding, component)))
        .zip(values)
        .map(|((binding, component), data)| binding.curve(component, data))
        .collect();
    let events = fields
        .get("m_Events")
        .map(|events| {
            events
                .as_array()
                .ok_or_else(|| malformed("m_Events is not an array"))?
                .iter()
                .map(event)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(ClipDocument {
        format: clip::FORMAT.into(),
        version: clip::VERSION,
        name,
        sample_rate: finite(number(field(fields, "m_SampleRate")?)?, "m_SampleRate")?,
        start_time: finite(number(field(muscle, "m_StartTime")?)?, "m_StartTime")?,
        stop_time: finite(number(field(muscle, "m_StopTime")?)?, "m_StopTime")?,
        looping: flag(muscle, "m_LoopTime")?,
        curves,
        events,
    })
}

/// What a run of consecutive curves drives: one entry of `genericBindings`.
struct Binding {
    path: u32,
    attribute: u32,
    class_id: i32,
    script: Option<String>,
    custom_type: u8,
    /// The number of curves the binding takes.
    width: u8,
}

impl Binding {
    fn read(clip: ObjectId, binding: &Value, cx: &dyn Context) -> Result<Self, ConvertError> {
        for name in NON_FLOAT_BINDINGS {
            if binding
                .get(name)
                .map(|v| integer(v, name))
                .transpose()?
                .is_some_and(|v| v != 0)
            {
                return Err(unsupported(format!("a binding has {name} set")));
            }
        }
        let class_id = i32::try_from(integer(field(binding, "typeID")?, "typeID")?)
            .map_err(|_| malformed("typeID is out of range"))?;
        let attribute = unsigned(field(binding, "attribute")?, "attribute")?;
        let width = if class_id == TRANSFORM {
            transform_width(attribute)?
        } else {
            1
        };
        let script = match field(binding, "script")?.as_pptr() {
            None => return Err(malformed("a binding's script is not a reference")),
            Some(reference) => match cx.resolve(clip, reference)? {
                None => None,
                Some(id) => Some(script_name(&cx.object(id)?.fields)?),
            },
        };
        Ok(Self {
            path: unsigned(field(binding, "path")?, "path")?,
            attribute,
            class_id,
            script,
            custom_type: u8::try_from(integer(field(binding, "customType")?, "customType")?)
                .map_err(|_| malformed("customType is out of range"))?,
            width,
        })
    }

    /// The `component`th curve of the binding, with its values.
    fn curve(&self, component: u8, data: CurveData) -> Curve {
        Curve {
            path: self.path,
            attribute: self.attribute,
            class_id: self.class_id,
            script: self.script.clone(),
            custom_type: self.custom_type,
            component,
            data,
        }
    }
}

/// The number of curves of a `Transform` property: position, rotation as a quaternion, scale,
/// and rotation as Euler angles.
fn transform_width(attribute: u32) -> Result<u8, ConvertError> {
    match attribute {
        1 | 3 | 4 => Ok(3),
        2 => Ok(4),
        _ => Err(unsupported(format!(
            "a Transform binding has the attribute {attribute}"
        ))),
    }
}

/// The full name of a `MonoScript`, as [`crate::script_name`] writes it.
fn script_name(script: &Value) -> Result<String, ConvertError> {
    let text = |name: &str| {
        field(script, name)?
            .as_str()
            .ok_or_else(|| malformed(format!("{name} is not a string")))
    };
    Ok(crate::script_name(
        text("m_AssemblyName")?,
        text("m_Namespace")?,
        text("m_ClassName")?,
    ))
}

/// The streamed curves of `m_StreamedClip`, in curve order.
fn streamed(clip: &Value) -> Result<Vec<CurveData>, ConvertError> {
    let count = usize::try_from(integer(field(clip, "curveCount")?, "curveCount")?)
        .map_err(|_| malformed("curveCount is negative"))?;
    if clip
        .get("discreteCurveCount")
        .map(|v| integer(v, "discreteCurveCount"))
        .transpose()?
        .is_some_and(|v| v != 0)
    {
        return Err(unsupported("the streamed clip has discrete curves"));
    }
    let words = array(clip, "data")?
        .iter()
        .map(|w| unsigned(w, "a streamed word"))
        .collect::<Result<Vec<u32>, _>>()?;
    let mut segments = vec![Vec::new(); count];
    let mut at = 0;
    let mut previous = f32::NEG_INFINITY;
    while at < words.len() {
        let header = words
            .get(at..at + 2)
            .ok_or_else(|| malformed("a streamed frame is cut short"))?;
        let time = f32::from_bits(header[0]);
        let keys = usize::try_from(header[1] as i32)
            .map_err(|_| malformed("a streamed frame has a negative key count"))?;
        at += 2;
        let end = at
            .checked_add(
                keys.checked_mul(5)
                    .ok_or_else(|| malformed("too many keys"))?,
            )
            .filter(|&end| end <= words.len())
            .ok_or_else(|| malformed("a streamed frame is cut short"))?;
        if time == f32::INFINITY && keys == 0 && end == words.len() {
            break;
        }
        if !time.is_finite() || time <= previous {
            return Err(malformed("streamed frame times do not increase"));
        }
        previous = time;
        for &[curve, a, b, c, d] in words[at..end].as_chunks::<5>().0 {
            let index = usize::try_from(curve as i32)
                .ok()
                .filter(|&i| i < count)
                .ok_or_else(|| malformed("a streamed key names a curve that does not exist"))?;
            let [a, b, c, d] = [a, b, c, d].map(f32::from_bits);
            if ![a, b, c, d].iter().all(|v| v.is_finite()) {
                return Err(malformed("a streamed coefficient is not finite"));
            }
            segments[index].push([time, a, b, c, d]);
        }
        at = end;
    }
    Ok(segments
        .into_iter()
        .map(|segments| CurveData::Streamed { segments })
        .collect())
}

/// The dense curves of `m_DenseClip`, in curve order.
fn dense(clip: &Value) -> Result<Vec<CurveData>, ConvertError> {
    let size = |name: &str| {
        usize::try_from(integer(field(clip, name)?, name)?)
            .map_err(|_| malformed(format!("{name} is negative")))
    };
    let frames = size("m_FrameCount")?;
    let count = size("m_CurveCount")?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let samples = array(clip, "m_SampleArray")?
        .iter()
        .map(|v| finite(number(v)?, "a dense sample"))
        .collect::<Result<Vec<_>, _>>()?;
    if frames.checked_mul(count) != Some(samples.len()) {
        return Err(malformed("the dense sample array has the wrong length"));
    }
    let begin_time = finite(number(field(clip, "m_BeginTime")?)?, "m_BeginTime")?;
    let sample_rate = finite(number(field(clip, "m_SampleRate")?)?, "m_SampleRate")?;
    Ok((0..count)
        .map(|curve| CurveData::Dense {
            begin_time,
            sample_rate,
            samples: samples.iter().skip(curve).step_by(count).copied().collect(),
        })
        .collect())
}

/// One entry of `m_Events`.
fn event(event: &Value) -> Result<Event, ConvertError> {
    if field(event, "objectReferenceParameter")?
        .as_pptr()
        .is_some_and(|p| p.path_id != 0)
    {
        return Err(unsupported("an event has an object parameter"));
    }
    let text = |name: &str| {
        field(event, name)?
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| malformed(format!("{name} is not a string")))
    };
    Ok(Event {
        time: finite(number(field(event, "time")?)?, "an event time")?,
        function: text("functionName")?,
        data: text("data")?,
        float: finite(number(field(event, "floatParameter")?)?, "floatParameter")?,
        int: i32::try_from(integer(field(event, "intParameter")?, "intParameter")?)
            .map_err(|_| malformed("intParameter is out of range"))?,
    })
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ConvertError> {
    value
        .get(name)
        .ok_or_else(|| malformed(format!("the field {name} is missing")))
}

fn array<'a>(value: &'a Value, name: &str) -> Result<&'a [Value], ConvertError> {
    field(value, name)?
        .as_array()
        .ok_or_else(|| malformed(format!("{name} is not an array")))
}

/// A stored boolean, or `false` when the field is absent.
fn flag(value: &Value, name: &str) -> Result<bool, ConvertError> {
    match value.get(name) {
        None => Ok(false),
        Some(v) => v
            .as_bool()
            .or_else(|| v.as_u64().map(|n| n != 0))
            .ok_or_else(|| malformed(format!("{name} is not a boolean"))),
    }
}

/// A stored 32-bit float.
fn number(value: &Value) -> Result<f32, ConvertError> {
    value
        .as_f64()
        .map(|v| v as f32)
        .ok_or_else(|| malformed("a number is not a floating-point value"))
}

fn finite(value: f32, what: &str) -> Result<f32, ConvertError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(malformed(format!("{what} is not finite")))
    }
}

fn integer(value: &Value, name: &str) -> Result<i64, ConvertError> {
    value
        .as_i64()
        .ok_or_else(|| malformed(format!("{name} is not an integer")))
}

fn unsigned(value: &Value, name: &str) -> Result<u32, ConvertError> {
    value
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| malformed(format!("{name} is not a 32-bit unsigned integer")))
}

fn malformed(message: impl Into<String>) -> ConvertError {
    ConvertError::new("malformed", message)
}

fn unsupported(message: impl Into<String>) -> ConvertError {
    ConvertError::new("unsupported", message)
}
