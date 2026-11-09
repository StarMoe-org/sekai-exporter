//! The converter interface, the writers and the general-purpose converters.
//!
//! A converter is a pure function from a root object, its closure and the table rows it reads to
//! an asset. It does not read files, choose output paths or write to the catalog: it reads
//! through a [`Context`] and returns an [`Asset`], and the evaluator does the rest.
//!
//! An asset is a list of members. A member is decoded content (a [`Surface`] or [`Pcm`]),
//! which the evaluator encodes in the forms the run needs, or finished bytes such as a JSON
//! document or a GLB file.

use std::fmt;

pub use sekai_codec::{DecodeError, HdrSurface, Movie, Pcm, Surface};
pub use sekai_unity::index::{ObjectId, ObjectIndex, PPtr};
pub use sekai_unity::{Chunk, Value};

#[cfg(feature = "animation")]
pub mod animation;
#[cfg(feature = "audio")]
pub mod audio;
pub mod fields;
#[cfg(feature = "image")]
pub mod image;
#[cfg(feature = "live2d")]
pub mod live2d;
#[cfg(feature = "model")]
pub mod model;
#[cfg(feature = "particles")]
pub mod particles;
pub mod roots;
pub mod tables;
#[cfg(feature = "text")]
pub mod text;
#[cfg(feature = "timeline")]
pub mod timeline;
#[cfg(feature = "ugui")]
pub mod ugui;
#[cfg(feature = "video")]
pub mod video;

/// A converter: the code that produces one asset form.
pub trait Converter: Send + Sync {
    /// The converter name. It is part of every fingerprint the converter produces.
    fn name(&self) -> &'static str;

    /// The converter version. It changes only when the converter's output changes.
    fn version(&self) -> u32;

    /// Whether `object` is the root of an asset of this form. `script` is the script the object
    /// refers to, as in [`Object::script`]: `"<assembly>/<namespace>.<class>"`, or `None` when
    /// the object refers to no script or the script does not resolve. `container_paths` are the
    /// paths under which bundles list the object.
    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool;

    /// Produces the asset whose root is `root`.
    ///
    /// [`Converter::is_root`] sees only the index of an object. `Ok(None)` means that the
    /// object's fields show it holds no asset of this form, such as a font texture that the
    /// engine fills at run time; this is not an error.
    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError>;

    /// Objects that converting `root` reads and that `root` does not reach through references,
    /// such as the later segments of a split file. They and what they refer to join the closure
    /// of the asset, so they are in its fingerprint and the conversion may read them. The default
    /// is none.
    fn inputs(&self, root: ObjectId, cx: &dyn Context) -> Vec<ObjectId> {
        let _ = (root, cx);
        Vec::new()
    }

    /// What converting `root` and encoding its members is expected to need, estimated from the
    /// headers of the objects it reads. `object` is the index of `root`.
    ///
    /// The estimate only guides scheduling: it is not part of any fingerprint and does not
    /// change any output. When the headers cannot be read, a converter returns its estimate for
    /// the stored size and lets [`Converter::convert`] report the error. The default is
    /// [`Cost::light`] for the stored size of `root`.
    fn cost(&self, root: ObjectId, object: &ObjectIndex, cx: &dyn Context) -> Cost {
        let _ = (root, cx);
        Cost::light(u64::from(object.size))
    }
}

/// How converting and encoding an asset uses the processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CostClass {
    /// Little work for its size, such as copying bytes or writing a document.
    Light,
    /// Processor-bound work on one thread, such as decoding a texture or audio.
    Heavy,
    /// Work whose encoding runs on two threads at once, such as a video.
    Wide,
}

/// What converting and encoding one asset is expected to need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cost {
    /// How the work uses the processor.
    pub class: CostClass,
    /// The estimated peak memory of the conversion and the encoding of its members, in bytes.
    pub memory: u64,
}

impl Cost {
    /// How many times its stored size an object is taken to need when nothing more is known.
    pub const PAYLOAD_FACTOR: u64 = 4;

    /// Light work on an object of `size` stored bytes: [`Cost::PAYLOAD_FACTOR`] times the size.
    pub fn light(size: u64) -> Self {
        Self {
            class: CostClass::Light,
            memory: size.saturating_mul(Self::PAYLOAD_FACTOR),
        }
    }
}

/// What a converter reads through.
pub trait Context {
    /// An object with its fields.
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError>;

    /// The object a reference stored in object `from` points to, or `None` for a null
    /// reference. A reference to an object that is not available is a `missing_object` error.
    fn resolve(&self, from: ObjectId, reference: PPtr) -> Result<Option<ObjectId>, ConvertError>;

    /// Bytes that object `from` stores outside itself, such as the image data of a texture
    /// (`m_StreamData`) or the samples of an audio clip (`m_Resource`). `path` is the stored
    /// path, which names a resource file of the object's bundle.
    fn resource(
        &self,
        from: ObjectId,
        path: &str,
        offset: u64,
        size: u64,
    ) -> Result<Chunk<'_>, ConvertError>;

    /// The container of the bundle that holds object `from`: each path and the object it names,
    /// in container order. Converters use it to find objects that belong together only by name,
    /// such as the wave bank of a cue sheet or the later segments of a video.
    fn container(&self, from: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError>;
}

/// An object as a converter sees it.
#[derive(Debug, Clone)]
pub struct Object {
    /// The object.
    pub id: ObjectId,
    /// The class ID.
    pub class_id: i32,
    /// The class name.
    pub class: String,
    /// For objects that refer to a script: `"<assembly>/<namespace>.<class>"`, or `None` when
    /// the object refers to no script or the script does not resolve.
    pub script: Option<String>,
    /// The fields. The root is a [`Value::Struct`].
    pub fields: Value,
}

/// The result of a conversion.
#[derive(Debug, Clone)]
pub struct Asset {
    /// The members, in a fixed order that the converter defines.
    pub members: Vec<Member>,
}

/// One member of an asset.
#[derive(Debug, Clone)]
pub struct Member {
    /// What the member is to the asset, such as `"image"` or `"base_color"`. Roles are defined
    /// by the specification of the asset form and are unique within an asset.
    pub role: String,
    /// The content.
    pub content: Content,
}

/// The content of a member.
#[derive(Debug, Clone)]
pub enum Content {
    /// An image, encoded by the evaluator.
    Image(Surface),
    /// An HDR image, encoded by the evaluator.
    HdrImage(HdrSurface),
    /// Audio, encoded by the evaluator.
    Audio(Pcm),
    /// A video, encoded by the evaluator.
    Video(Movie),
    /// Finished bytes, written as they are.
    File {
        /// The file extension, without the dot.
        extension: String,
        /// The bytes.
        bytes: Vec<u8>,
    },
}

/// Why an asset could not be produced: a named error, written with the asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertError {
    /// A fixed code, such as `missing_object` or `unsupported`.
    pub code: &'static str,
    /// A description for people.
    pub message: String,
}

impl ConvertError {
    /// An error with a code and a message.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<sekai_unity::Error> for ConvertError {
    fn from(e: sekai_unity::Error) -> Self {
        Self::new(e.code(), e.to_string())
    }
}

impl From<DecodeError> for ConvertError {
    fn from(e: DecodeError) -> Self {
        Self::new(e.code(), e.to_string())
    }
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ConvertError {}
