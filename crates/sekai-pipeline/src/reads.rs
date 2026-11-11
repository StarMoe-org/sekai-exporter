//! What a conversion reads.

use std::cell::RefCell;
use std::collections::BTreeSet;

use sekai_convert::{Chunk, Context, ConvertError, EngineObject, Object, ObjectId, PPtr};

use crate::index_record::StreamDigest;

/// A [`Context`] that records what a conversion reads and refuses anything outside the closure,
/// so that everything an output depends on is in its fingerprint.
pub(crate) struct Reads<'a> {
    inner: &'a dyn Context,
    closure: &'a [ObjectId],
    streams: Option<&'a [Vec<Vec<StreamDigest>>]>,
    objects: RefCell<BTreeSet<ObjectId>>,
    resources: RefCell<BTreeSet<(usize, String)>>,
}

impl<'a> Reads<'a> {
    /// Reads from `inner` within `closure`, the root first and the rest in order, verifying
    /// resource slice reads against `streams` when present.
    pub(crate) fn new(
        inner: &'a dyn Context,
        closure: &'a [ObjectId],
        streams: Option<&'a [Vec<Vec<StreamDigest>>]>,
    ) -> Self {
        Self {
            inner,
            closure,
            streams,
            objects: RefCell::default(),
            resources: RefCell::default(),
        }
    }

    fn check(&self, id: ObjectId) -> Result<(), ConvertError> {
        if id == self.closure[0] || self.closure[1..].binary_search(&id).is_ok() {
            return Ok(());
        }
        Err(ConvertError::new(
            "undeclared_read",
            format!("object {} is outside the closure of the asset", id.path_id),
        ))
    }

    /// The root, then the other objects read, in order; and the resource files read, in order.
    pub(crate) fn finish(self) -> (Vec<ObjectId>, Vec<(usize, String)>) {
        let root = self.closure[0];
        let mut objects = self.objects.into_inner();
        objects.remove(&root);
        let objects = std::iter::once(root).chain(objects).collect();
        (objects, self.resources.into_inner().into_iter().collect())
    }
}

impl Context for Reads<'_> {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        let object = self.inner.object(id)?;
        self.check(id)?;
        self.objects.borrow_mut().insert(id);
        Ok(object)
    }

    fn resolve(&self, from: ObjectId, reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        self.inner.resolve(from, reference)
    }

    fn resource(
        &self,
        from: ObjectId,
        path: &str,
        offset: u64,
        size: u64,
    ) -> Result<Chunk<'_>, ConvertError> {
        let chunk = self.inner.resource(from, path, offset, size)?;
        self.check(from)?;
        let file = path.rsplit('/').next().unwrap_or(path);
        if let Some(streams) = self.streams {
            let indexed = streams
                .get(from.bundle)
                .and_then(|b| b.get(from.file))
                .and_then(|list| {
                    list.binary_search_by_key(&from.path_id, |d| d.path_id)
                        .ok()
                        .map(|pos| &list[pos])
                })
                .is_some_and(|d| d.file == file && d.offset == offset && d.size == size);
            if !indexed {
                return Err(ConvertError::new(
                    "undeclared_read",
                    format!(
                        "resource {file} ({offset}..{}) is not indexed on object {}",
                        offset.saturating_add(size),
                        from.path_id
                    ),
                ));
            }
        }
        self.resources
            .borrow_mut()
            .insert((from.bundle, file.to_owned()));
        Ok(chunk)
    }

    fn container(&self, from: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        self.inner.container(from)
    }

    fn file_name(&self, from: ObjectId) -> Result<&str, ConvertError> {
        self.inner.file_name(from)
    }

    fn engine_object(
        &self,
        from: ObjectId,
        reference: PPtr,
    ) -> Result<Option<EngineObject>, ConvertError> {
        self.inner.engine_object(from, reference)
    }
}
