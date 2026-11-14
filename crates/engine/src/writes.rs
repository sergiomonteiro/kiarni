//! Pure document replacement and Merge Patch transformations.

use std::{collections::BTreeMap, fmt};

use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentError, DocumentInput};
use serde::Serialize;
use serde_json::value::RawValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Document(DocumentError),
    InvalidRouteId,
    InvalidCurrentDocument,
    IdMismatch,
    ImmutableId,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Document(error) => error.code(),
            Self::InvalidRouteId => "invalid_id",
            Self::InvalidCurrentDocument => "invalid_current_document",
            Self::IdMismatch => "id_mismatch",
            Self::ImmutableId => "immutable_id",
        })
    }
}

impl std::error::Error for Error {}

impl From<DocumentError> for Error {
    fn from(value: DocumentError) -> Self {
        Self::Document(value)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Operation {
    Replace,
    MergePatch,
}

/// Candidate only. `changed()` does not certify a successful commit.
pub struct PreparedWrite {
    json: PreparedJson,
    changed: bool,
}
enum PreparedJson {
    Owned(Vec<u8>),
    Shared(std::sync::Arc<[u8]>),
}

impl PreparedWrite {
    pub fn json(&self) -> &[u8] {
        match &self.json {
            PreparedJson::Owned(bytes) => bytes,
            PreparedJson::Shared(bytes) => bytes,
        }
    }
    /// Share an observed update result with changefeeds without retaining a second copy.
    /// Unobserved writes retain their original Vec representation.
    pub fn shared_json(&mut self) -> std::sync::Arc<[u8]> {
        if let PreparedJson::Shared(bytes) = &self.json {
            return std::sync::Arc::clone(bytes);
        }
        let previous = std::mem::replace(&mut self.json, PreparedJson::Owned(Vec::new()));
        let bytes: std::sync::Arc<[u8]> = match previous {
            PreparedJson::Owned(bytes) => bytes.into(),
            PreparedJson::Shared(bytes) => bytes,
        };
        self.json = PreparedJson::Shared(std::sync::Arc::clone(&bytes));
        bytes
    }

    pub fn changed(&self) -> bool {
        self.changed
    }
}

impl fmt::Debug for PreparedWrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedWrite")
            .field("changed", &self.changed)
            .finish_non_exhaustive()
    }
}

// Raw scalar numbers avoid float conversion and serde_json::Value's special
// arbitrary_precision object representation. Strings are decoded/canonicalized.
#[derive(Serialize)]
#[serde(untagged)]
enum Node {
    Object(BTreeMap<String, Node>),
    Array(Vec<Node>),
    Scalar(Box<RawValue>),
}

impl Node {
    // Called only after DocumentInput validates duplicates, size and depth.
    fn parse(raw: &str) -> Result<Self, Error> {
        let raw = raw.trim();
        match raw.as_bytes().first() {
            Some(b'{') => {
                let fields: BTreeMap<String, Box<RawValue>> = decode(raw)?;
                Ok(Self::Object(
                    fields
                        .into_iter()
                        .map(|(key, value)| Ok((key, Self::parse(value.get())?)))
                        .collect::<Result<_, Error>>()?,
                ))
            }
            Some(b'[') => {
                let values: Vec<Box<RawValue>> = decode(raw)?;
                Ok(Self::Array(
                    values
                        .into_iter()
                        .map(|value| Self::parse(value.get()))
                        .collect::<Result<_, _>>()?,
                ))
            }
            Some(b'"') => {
                let value: String = decode(raw)?;
                Ok(Self::Scalar(encode_raw(&value)?))
            }
            _ => Ok(Self::Scalar(decode(raw)?)),
        }
    }

    fn merge(&mut self, patch: Self) {
        let Self::Object(fields) = patch else {
            *self = patch;
            return;
        };
        if !matches!(self, Self::Object(_)) {
            *self = Self::Object(BTreeMap::new());
        }
        if let Self::Object(target) = self {
            for (key, value) in fields {
                if matches!(&value, Self::Scalar(raw) if raw.get() == "null") {
                    target.remove(&key);
                } else if let Some(current) = target.get_mut(&key) {
                    current.merge(value);
                } else {
                    let mut current = Self::Object(BTreeMap::new());
                    current.merge(value);
                    target.insert(key, current);
                }
            }
        }
    }
}

fn decode<'a, T: serde::Deserialize<'a>>(raw: &'a str) -> Result<T, Error> {
    serde_json::from_str(raw).map_err(|_| DocumentError::InvalidJson.into())
}

fn encode_raw(value: &impl Serialize) -> Result<Box<RawValue>, Error> {
    serde_json::value::to_raw_value(value).map_err(|_| DocumentError::DocumentEncodingFailed.into())
}

fn encode(value: &Node) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(value).map_err(|_| DocumentError::DocumentEncodingFailed.into())
}

/// Caller supplies an existing document under its mutation lock/transaction.
/// Missing documents are a lookup error, never an implicit insert.
pub fn prepare(
    operation: Operation,
    route_id: &str,
    current: &[u8],
    body: &[u8],
    max_bytes: usize,
) -> Result<PreparedWrite, Error> {
    // Validate route independently, even when both documents omit their ID.
    if route_id.is_empty()
        || route_id.len() > 128
        || !route_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(Error::InvalidRouteId);
    }
    let stored = DocumentInput::parse(current, DEFAULT_MAX_DOCUMENT_BYTES)
        .map_err(|_| Error::InvalidCurrentDocument)?;
    if stored.id() != Some(route_id) {
        return Err(Error::InvalidCurrentDocument);
    }
    let input = DocumentInput::parse(body, max_bytes).map_err(|error| {
        if matches!(operation, Operation::MergePatch) && error == DocumentError::InvalidId {
            Error::ImmutableId
        } else {
            Error::Document(error)
        }
    })?;
    let current_text = std::str::from_utf8(current).map_err(|_| Error::InvalidCurrentDocument)?;
    let body_text = std::str::from_utf8(body).map_err(|_| DocumentError::InvalidJson)?;
    let mut target = Node::parse(current_text)?;
    let original = encode(&target)?;
    let patch = Node::parse(body_text)?;
    match operation {
        Operation::Replace => {
            if input.id().is_some_and(|id| id != route_id) {
                return Err(Error::IdMismatch);
            }
            target = patch;
            if let Node::Object(fields) = &mut target {
                fields.insert("_id".into(), Node::Scalar(encode_raw(&route_id)?));
            }
        }
        Operation::MergePatch => {
            if input.fields().contains_key("_id") {
                return Err(Error::ImmutableId);
            }
            target.merge(patch);
        }
    }
    let candidate = encode(&target)?;
    let changed = candidate != original;
    if changed {
        DocumentInput::parse(&candidate, max_bytes)?;
    }
    Ok(PreparedWrite {
        json: PreparedJson::Owned(if changed { candidate } else { current.to_vec() }),
        changed,
    })
}
