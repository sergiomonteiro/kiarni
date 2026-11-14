//! Document validation and insertion preparation.

mod insertion;
mod unique_keys;

pub use insertion::PreparedInsert;

use std::{collections::BTreeMap, fmt};

use serde::Deserialize;
use serde_json::value::RawValue;

/// Proposed default; callers must pass an explicit limit to validation.
pub const DEFAULT_MAX_DOCUMENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentError {
    DocumentTooLarge,
    InvalidJson,
    DocumentMustBeObject,
    InvalidId,
    DocumentTooDeep,
    IdGenerationFailed,
    DocumentEncodingFailed,
}

impl DocumentError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::DocumentTooLarge => "document_too_large",
            Self::InvalidJson => "invalid_json",
            Self::DocumentMustBeObject => "document_must_be_object",
            Self::InvalidId => "invalid_id",
            Self::DocumentTooDeep => "document_too_deep",
            Self::IdGenerationFailed => "id_generation_failed",
            Self::DocumentEncodingFailed => "document_encoding_failed",
        }
    }
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for DocumentError {}

/// Validated insertion input. Missing ID still requires server generation
/// and uniqueness checks before persistence; this is not a stored document.
pub struct DocumentInput {
    fields: BTreeMap<String, Box<RawValue>>,
    id: Option<String>,
    max_bytes: usize,
}

impl fmt::Debug for DocumentInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentInput")
            .field("field_count", &self.fields.len())
            .field("has_id", &self.id.is_some())
            .finish_non_exhaustive()
    }
}

impl DocumentInput {
    pub fn parse(input: &[u8], max_bytes: usize) -> Result<Self, DocumentError> {
        if input.len() > max_bytes {
            return Err(DocumentError::DocumentTooLarge);
        }
        let mut deserializer = serde_json::Deserializer::from_slice(input);
        unique_keys::UniqueKeys::deserialize(&mut deserializer)
            .map_err(|_| DocumentError::InvalidJson)?;
        deserializer.end().map_err(|_| DocumentError::InvalidJson)?;
        if !within_depth(input) {
            return Err(DocumentError::DocumentTooDeep);
        }
        let fields: BTreeMap<String, Box<RawValue>> =
            serde_json::from_slice(input).map_err(|_| DocumentError::DocumentMustBeObject)?;
        let id = fields
            .get("_id")
            .map(|raw| {
                serde_json::from_str::<String>(raw.get()).map_err(|_| DocumentError::InvalidId)
            })
            .transpose()?;
        if let Some(id) = &id {
            let valid = !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
            if !valid {
                return Err(DocumentError::InvalidId);
            }
        }
        Ok(Self {
            fields,
            id,
            max_bytes,
        })
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn fields(&self) -> &BTreeMap<String, Box<RawValue>> {
        &self.fields
    }
}

// Input has already passed JSON validation. Ignore delimiters inside strings.
fn within_depth(input: &[u8]) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for &byte in input {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > 64 {
                        return false;
                    }
                }
                b'}' | b']' => depth -= 1,
                _ => {}
            }
        }
    }
    true
}
