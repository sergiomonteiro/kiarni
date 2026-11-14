use kiarni_http_contract_spike::Code;

#[test]
fn domain_errors_have_explicit_public_mappings() {
    use kiarni_core::DocumentError as D;
    use kiarni_feed_spike::Error as F;
    use kiarni_query_spike::Error as Q;
    use kiarni_storage_spike::Error as S;
    let cases = [
        (Code::from(D::DocumentTooLarge), "document_too_large", 413),
        (Code::from(D::InvalidJson), "invalid_json", 400),
        (
            Code::from(D::DocumentMustBeObject),
            "document_must_be_object",
            422,
        ),
        (Code::from(D::InvalidId), "invalid_id", 422),
        (Code::from(D::DocumentTooDeep), "document_too_deep", 422),
        (
            Code::from(D::IdGenerationFailed),
            "id_generation_failed",
            503,
        ),
        (Code::from(D::DocumentEncodingFailed), "internal_error", 500),
        (Code::from(Q::QueryTooLarge), "query_too_large", 413),
        (Code::from(Q::InvalidJson), "invalid_json", 400),
        (Code::from(Q::InvalidQuery), "invalid_query", 422),
        (Code::from(Q::InvalidLimit), "invalid_limit", 422),
        (Code::from(Q::QueryTooComplex), "query_too_complex", 422),
        (Code::from(Q::UnsupportedNumber), "unsupported_number", 422),
        (
            Code::from(Q::UnsupportedSortValue),
            "unsupported_sort_value",
            422,
        ),
        (Code::from(Q::InvalidDataset), "internal_error", 500),
        (Code::from(Q::DatasetLimit), "internal_error", 500),
        (Code::from(Q::ResultTooLarge), "result_too_large", 422),
        (Code::from(S::Storage), "storage_unavailable", 503),
        (Code::from(S::InvalidDocument), "invalid_document", 422),
        (Code::from(S::DuplicateId), "duplicate_id", 409),
        (Code::from(S::MissingDocument), "document_not_found", 404),
        (
            Code::from(S::IdGenerationExhausted),
            "id_generation_exhausted",
            503,
        ),
        (Code::from(F::Busy), "write_busy", 503),
        (Code::from(F::Closed), "shutting_down", 503),
        (Code::from(F::InvalidLimits), "internal_error", 500),
        (Code::from(F::InvalidCollection), "invalid_collection", 422),
        (Code::from(F::SubscriberLimit), "subscription_limit", 429),
        (Code::from(F::SequenceExhausted), "sequence_exhausted", 503),
        (Code::from(F::Poisoned), "service_unavailable", 503),
        (Code::from(F::Aborted), "internal_error", 500),
        (Code::from(F::Storage(S::DuplicateId)), "duplicate_id", 409),
    ];
    for (actual, name, status) in cases {
        assert_eq!((actual.spec().0, actual.spec().1), (name, status));
    }
}
