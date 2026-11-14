use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentError, DocumentInput};
use serde_json::Value;

#[test]
fn document_contract_fixtures() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/documents.json"))
        .expect("valid fixture corpus");
    for case in cases {
        let name = case["name"].as_str().expect("fixture name");
        let input = case["input"].as_str().expect("fixture input");
        let limit = case["max_bytes"]
            .as_u64()
            .map(|value| usize::try_from(value).expect("fixture limit fits usize"))
            .unwrap_or(DEFAULT_MAX_DOCUMENT_BYTES);
        match DocumentInput::parse(input.as_bytes(), limit) {
            Ok(document) => {
                assert_eq!(case["ok"], true, "{name}");
                assert_eq!(document.id(), case["id"].as_str(), "{name}");
            }
            Err(error) => {
                assert_eq!(Some(error.code()), case["error"].as_str(), "{name}");
                assert_eq!(error.to_string(), error.code(), "{name}");
            }
        }
    }
}

#[test]
fn id_length_boundaries() {
    for length in [1, 128, 129] {
        let input = format!(r#"{{"_id":"{}"}}"#, "a".repeat(length));
        let result = DocumentInput::parse(input.as_bytes(), DEFAULT_MAX_DOCUMENT_BYTES);
        assert_eq!(result.is_ok(), length <= 128);
        if length > 128 {
            assert_eq!(result.err(), Some(DocumentError::InvalidId));
        }
    }
}

#[test]
fn size_counts_utf8_bytes() {
    let input = r#"{"x":"é"}"#;
    assert!(DocumentInput::parse(input.as_bytes(), input.len()).is_ok());
    assert_eq!(
        DocumentInput::parse(input.as_bytes(), input.len() - 1).err(),
        Some(DocumentError::DocumentTooLarge)
    );
}

#[test]
fn invalid_utf8_is_rejected_without_payload_in_error() {
    let input = b"{\"secret\":\"\xff\"}";
    let error =
        DocumentInput::parse(input, DEFAULT_MAX_DOCUMENT_BYTES).expect_err("invalid UTF-8 fails");
    assert_eq!(error.to_string(), "invalid_json");
    assert_eq!(format!("{error:?}"), "InvalidJson");
}

#[test]
fn document_debug_omits_payload_and_id() {
    let document = DocumentInput::parse(
        br#"{"_id":"secret-id","sensitive-field":"secret-value"}"#,
        DEFAULT_MAX_DOCUMENT_BYTES,
    )
    .expect("valid document");
    assert_eq!(
        format!("{document:?}"),
        "DocumentInput { field_count: 2, has_id: true, .. }"
    );
}

#[test]
fn nested_container_limit_is_inclusive() {
    for depth in [64, 65] {
        let input = format!(
            "{{\"x\":{}0{}}}",
            "[".repeat(depth - 1),
            "]".repeat(depth - 1)
        );
        let result = DocumentInput::parse(input.as_bytes(), DEFAULT_MAX_DOCUMENT_BYTES);
        assert_eq!(result.is_ok(), depth == 64);
        if depth == 65 {
            assert_eq!(result.err(), Some(DocumentError::DocumentTooDeep));
        }
    }
}

#[test]
fn raw_values_preserve_numbers_null_and_arbitrary_keys() {
    let input = br#"{"large":123456789012345678901234567890,"decimal":1.234567890123456789,"exponent":1e9999,"null":null,"nested":{"$serde_json::private::Number":"ordinary data"},"string":"[\"{\\}"}"#;
    let document = DocumentInput::parse(input, DEFAULT_MAX_DOCUMENT_BYTES).expect("valid input");
    assert_eq!(
        document.fields()["large"].get(),
        "123456789012345678901234567890"
    );
    assert_eq!(document.fields()["decimal"].get(), "1.234567890123456789");
    assert_eq!(document.fields()["exponent"].get(), "1e9999");
    assert_eq!(document.fields()["null"].get(), "null");
    assert!(!document.fields().contains_key("missing"));
    assert_eq!(
        document.fields()["nested"].get(),
        r#"{"$serde_json::private::Number":"ordinary data"}"#
    );
}
