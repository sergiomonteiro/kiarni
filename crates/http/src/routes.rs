use crate::Code;
use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    CreateCollection,
    ListCollections,
    DeleteCollection,
    InsertDocument,
    ReadDocument,
    ReplaceDocument,
    PatchDocument,
    DeleteDocument,
    Query,
    Changes,
    Liveness,
    Readiness,
    Metrics,
    Status,
    RecentErrors,
    Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowedMethods {
    Collections,
    Collection,
    Documents,
    Document,
    Query,
    Changes,
    Health,
    Metrics,
    Status,
    RecentErrors,
    Count,
}

impl AllowedMethods {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Collections => "GET, HEAD, POST",
            Self::Collection => "DELETE",
            Self::Documents => "POST",
            Self::Document => "GET, HEAD, PUT, PATCH, DELETE",
            Self::Query => "POST",
            Self::Changes => "GET",
            Self::Health | Self::Metrics | Self::Status | Self::RecentErrors | Self::Count => {
                "GET, HEAD"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejection {
    pub code: Code,
    pub allow: Option<AllowedMethods>,
}

impl From<Code> for Rejection {
    fn from(code: Code) -> Self {
        Self { code, allow: None }
    }
}

pub struct Target<'a> {
    operation: Operation,
    collection: Option<&'a str>,
    id: Option<&'a str>,
    head: bool,
}

impl fmt::Debug for Target<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target")
            .field("operation", &self.operation)
            .finish_non_exhaustive()
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// Input is the raw URI path, before percent decoding, without query string.
/// This is not an HTTP parser or authentication guard.
pub fn resolve<'a>(method: &str, path: &'a str) -> Result<Target<'a>, Rejection> {
    if path.len() > 2048
        || !path.starts_with('/')
        || path
            .bytes()
            .any(|b| b.is_ascii_control() || matches!(b, b' ' | b'%' | b'?' | b'#' | b'\\'))
    {
        return Err(Code::InvalidPath.into());
    }
    if path == "/" {
        return Err(Code::RouteNotFound.into());
    }
    let parts: Vec<_> = path.split('/').skip(1).collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(Code::InvalidPath.into());
    }
    let (allowed, collection, id) = match parts.as_slice() {
        ["v1", "errors", "recent"] => (AllowedMethods::RecentErrors, None, None),
        ["v1", "status"] => (AllowedMethods::Status, None, None),
        ["v1", "metrics"] => (AllowedMethods::Metrics, None, None),
        ["v1", "collections"] => (AllowedMethods::Collections, None, None),
        ["v1", "collections", collection] => (AllowedMethods::Collection, Some(*collection), None),
        ["v1", "collections", collection, "documents"] => {
            (AllowedMethods::Documents, Some(*collection), None)
        }
        ["v1", "collections", collection, "documents", id] => {
            (AllowedMethods::Document, Some(*collection), Some(*id))
        }
        ["v1", "collections", collection, "count"] => {
            (AllowedMethods::Count, Some(*collection), None)
        }
        ["v1", "collections", collection, "query"] => {
            (AllowedMethods::Query, Some(*collection), None)
        }
        ["v1", "collections", collection, "changes"] => {
            (AllowedMethods::Changes, Some(*collection), None)
        }
        ["v1", "health", "live" | "ready"] => (AllowedMethods::Health, None, None),
        _ => return Err(Code::RouteNotFound.into()),
    };
    if collection.is_some_and(|value| !identifier(value))
        || id.is_some_and(|value| !identifier(value))
    {
        return Err(Code::InvalidPath.into());
    }
    let operation = match (allowed, method) {
        (AllowedMethods::RecentErrors, "GET" | "HEAD") => Operation::RecentErrors,
        (AllowedMethods::Count, "GET" | "HEAD") => Operation::Count,
        (AllowedMethods::Status, "GET" | "HEAD") => Operation::Status,
        (AllowedMethods::Metrics, "GET" | "HEAD") => Operation::Metrics,
        (AllowedMethods::Collections, "GET" | "HEAD") => Operation::ListCollections,
        (AllowedMethods::Collections, "POST") => Operation::CreateCollection,
        (AllowedMethods::Collection, "DELETE") => Operation::DeleteCollection,
        (AllowedMethods::Documents, "POST") => Operation::InsertDocument,
        (AllowedMethods::Document, "GET" | "HEAD") => Operation::ReadDocument,
        (AllowedMethods::Document, "PUT") => Operation::ReplaceDocument,
        (AllowedMethods::Document, "PATCH") => Operation::PatchDocument,
        (AllowedMethods::Document, "DELETE") => Operation::DeleteDocument,
        (AllowedMethods::Query, "POST") => Operation::Query,
        (AllowedMethods::Changes, "GET") => Operation::Changes,
        (AllowedMethods::Health, "GET" | "HEAD") => {
            if path.ends_with("/live") {
                Operation::Liveness
            } else {
                Operation::Readiness
            }
        }
        _ => {
            return Err(Rejection {
                code: Code::MethodNotAllowed,
                allow: Some(allowed),
            });
        }
    };
    Ok(Target {
        operation,
        collection,
        id,
        head: method == "HEAD",
    })
}

impl Target<'_> {
    pub fn operation(&self) -> Operation {
        self.operation
    }
    pub fn collection(&self) -> Option<&str> {
        self.collection
    }
    pub fn id(&self) -> Option<&str> {
        self.id
    }
    pub fn is_head(&self) -> bool {
        self.head
    }

    pub const fn body_limit(&self) -> usize {
        match self.operation {
            Operation::CreateCollection => 1024,
            Operation::Query => 16 * 1024,
            Operation::InsertDocument | Operation::ReplaceDocument | Operation::PatchDocument => {
                DEFAULT_MAX_DOCUMENT_BYTES
            }
            _ => 0,
        }
    }

    /// media_type is the normalized essence supplied by the future HTTP adapter.
    /// Does not decode transfer framing, content encodings or media parameters.
    pub fn validate_body(&self, media_type: Option<&str>, bytes: &[u8]) -> Result<(), Code> {
        let limit = self.body_limit();
        if limit == 0 {
            return if bytes.is_empty() {
                Ok(())
            } else {
                Err(Code::UnexpectedBody)
            };
        }
        if bytes.len() > limit {
            return Err(match self.operation {
                Operation::Query => Code::QueryTooLarge,
                Operation::CreateCollection => Code::RequestTooLarge,
                _ => Code::DocumentTooLarge,
            });
        }
        let expected = if self.operation == Operation::PatchDocument {
            "application/merge-patch+json"
        } else {
            "application/json"
        };
        if media_type != Some(expected) {
            return Err(Code::UnsupportedMediaType);
        }
        if self.operation == Operation::Query {
            let query = kiarni_engine::query::Query::parse(bytes).map_err(Code::from)?;
            if Some(query.collection()) != self.collection {
                return Err(Code::CollectionMismatch);
            }
            return Ok(());
        }
        let input = DocumentInput::parse(bytes, limit).map_err(|error| {
            if self.operation == Operation::PatchDocument
                && error == kiarni_core::DocumentError::InvalidId
            {
                Code::ImmutableId
            } else {
                Code::from(error)
            }
        })?;
        match self.operation {
            Operation::CreateCollection => {
                if input.fields().len() != 1 {
                    return Err(Code::InvalidCollection);
                }
                let raw = input.fields().get("name").ok_or(Code::InvalidCollection)?;
                let name: String =
                    serde_json::from_str(raw.get()).map_err(|_| Code::InvalidCollection)?;
                if !identifier(&name) {
                    return Err(Code::InvalidCollection);
                }
            }
            Operation::ReplaceDocument if input.id().is_some() && input.id() != self.id => {
                return Err(Code::IdMismatch);
            }
            Operation::PatchDocument if input.fields().contains_key("_id") => {
                return Err(Code::ImmutableId);
            }
            _ => {}
        }
        Ok(())
    }
}
