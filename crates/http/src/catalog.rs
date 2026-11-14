// Single static catalog: dynamic input must never become an error message.
macro_rules! catalog {
    ($( $variant:ident => ($code:literal, $status:literal, $message:literal)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Code { $( $variant ),+ }
        impl Code {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub const fn spec(self) -> (&'static str, u16, &'static str) {
                match self { $( Self::$variant => ($code, $status, $message) ),+ }
            }
        }
    }
}

catalog! {
    InvalidPath => ("invalid_path", 400, "The request path is invalid."),
    RouteNotFound => ("route_not_found", 404, "The route does not exist."),
    MethodNotAllowed => ("method_not_allowed", 405, "The method is not allowed for this route."),
    InvalidJson => ("invalid_json", 400, "The JSON input is invalid."),
    UnexpectedBody => ("unexpected_body", 400, "This operation does not accept a body."),
    InvalidDocument => ("invalid_document", 422, "The document is invalid."),
    InvalidId => ("invalid_id", 422, "The document ID is invalid."),
    DocumentMustBeObject => ("document_must_be_object", 422, "The document must be an object."),
    DocumentTooDeep => ("document_too_deep", 422, "The document exceeds the nesting limit."),
    InvalidCollection => ("invalid_collection", 422, "The collection name is invalid."),
    InvalidQuery => ("invalid_query", 422, "The query is invalid."),
    InvalidLimit => ("invalid_limit", 422, "The result limit is invalid."),
    QueryTooComplex => ("query_too_complex", 422, "The query exceeds the complexity limit."),
    UnsupportedNumber => ("unsupported_number", 422, "A compared number exceeds the supported range."),
    UnsupportedSortValue => ("unsupported_sort_value", 422, "The sort value type is unsupported."),
    IdMismatch => ("id_mismatch", 422, "The body ID differs from the route ID."),
    ImmutableId => ("immutable_id", 422, "The patch must not include the reserved ID field."),
    CollectionMismatch => ("collection_mismatch", 422, "The query collection differs from the route collection."),
    DuplicateId => ("duplicate_id", 409, "The document ID already exists in this collection."),
    CollectionExists => ("collection_exists", 409, "The collection already exists."),
    CollectionNotFound => ("collection_not_found", 404, "The collection does not exist."),
    DocumentNotFound => ("document_not_found", 404, "The document does not exist."),
    CollectionNotEmpty => ("collection_not_empty", 409, "The collection must be empty before deletion."),
    DocumentTooLarge => ("document_too_large", 413, "The document exceeds the byte limit."),
    QueryTooLarge => ("query_too_large", 413, "The query exceeds the byte limit."),
    RequestTooLarge => ("request_too_large", 413, "The request body exceeds the byte limit."),
    ResultTooLarge => ("result_too_large", 422, "The result exceeds the response byte limit."),
    UnsupportedMediaType => ("unsupported_media_type", 415, "The request media type is unsupported."),
    NotAcceptable => ("not_acceptable", 406, "The requested response media type is unsupported."),
    Unauthorized => ("unauthorized", 401, "Authentication is required or invalid."),
    Forbidden => ("forbidden", 403, "The request is not permitted."),
    ResumeNotSupported => ("resume_not_supported", 400, "Stream resumption is unsupported."),
    SubscriptionLimit => ("subscription_limit", 429, "The subscription limit has been reached."),
    WriteBusy => ("write_busy", 503, "The writer is busy."),
    StorageUnavailable => ("storage_unavailable", 503, "Storage is unavailable."),
    ServiceUnavailable => ("service_unavailable", 503, "The service is unavailable."),
    ShuttingDown => ("shutting_down", 503, "The server is shutting down."),
    IdGenerationFailed => ("id_generation_failed", 503, "Document ID generation is unavailable."),
    IdGenerationExhausted => ("id_generation_exhausted", 503, "Document ID generation attempts were exhausted."),
    SequenceExhausted => ("sequence_exhausted", 503, "The local event sequence is exhausted."),
    InternalError => ("internal_error", 500, "An internal error occurred."),
    RequestTimeout => ("request_timeout", 408, "The request was not received in time."),
    HeadersTooLarge => ("headers_too_large", 431, "The request headers exceed the byte limit."),
}

impl From<kiarni_core::DocumentError> for Code {
    fn from(error: kiarni_core::DocumentError) -> Self {
        use kiarni_core::DocumentError as E;
        match error {
            E::DocumentTooLarge => Self::DocumentTooLarge,
            E::InvalidJson => Self::InvalidJson,
            E::DocumentMustBeObject => Self::DocumentMustBeObject,
            E::InvalidId => Self::InvalidId,
            E::DocumentTooDeep => Self::DocumentTooDeep,
            E::IdGenerationFailed => Self::IdGenerationFailed,
            E::DocumentEncodingFailed => Self::InternalError,
        }
    }
}

impl From<kiarni_engine::query::Error> for Code {
    fn from(error: kiarni_engine::query::Error) -> Self {
        use kiarni_engine::query::Error as E;
        match error {
            E::QueryTooLarge => Self::QueryTooLarge,
            E::InvalidJson => Self::InvalidJson,
            E::InvalidQuery => Self::InvalidQuery,
            E::InvalidLimit => Self::InvalidLimit,
            E::QueryTooComplex => Self::QueryTooComplex,
            E::UnsupportedNumber => Self::UnsupportedNumber,
            E::UnsupportedSortValue => Self::UnsupportedSortValue,
            E::ResultTooLarge => Self::ResultTooLarge,
            // Harness-only guards must not become public query contracts.
            E::InvalidDataset | E::DatasetLimit => Self::InternalError,
        }
    }
}

impl From<kiarni_auth::Error> for Code {
    fn from(error: kiarni_auth::Error) -> Self {
        use kiarni_auth::Error as E;
        match error {
            E::Unauthorized => Self::Unauthorized,
            E::Unavailable
            | E::Busy
            | E::BootstrapIncomplete
            | E::CommitUncertain
            | E::ClockRollback
            | E::InvalidConfig
            | E::EpochRollback
            | E::EntropyUnavailable
            | E::GenerationExhausted
            | E::EpochExhausted
            | E::FileAccess
            | E::InsecurePath => Self::ServiceUnavailable,
            // Local administration errors have no remote administration API.
            E::InvalidLifetime | E::InvalidOverlap | E::RotationInProgress | E::AlreadyExists => {
                Self::InternalError
            }
        }
    }
}
