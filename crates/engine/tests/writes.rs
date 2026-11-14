use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentError, DocumentInput};
use kiarni_engine::writes::{Error, Operation, prepare};
use serde::Deserialize;
use serde_json::value::RawValue;

const CURRENT: &[u8] = br#"{ "_id": "doc", "a": 1 }"#;
const LIMIT: usize = DEFAULT_MAX_DOCUMENT_BYTES;

// Independent assertion: decode containers/strings, compare numeric tokens
// literally. Do not use prepare itself as the transformation oracle.
fn assert_tree(actual: &str, expected: &str) {
    use std::collections::BTreeMap;
    let actual = actual.trim();
    let expected = expected.trim();
    match expected.as_bytes()[0] {
        b'{' => {
            let actual: BTreeMap<String, Box<RawValue>> = serde_json::from_str(actual).unwrap();
            let expected: BTreeMap<String, Box<RawValue>> = serde_json::from_str(expected).unwrap();
            assert_eq!(
                actual.keys().collect::<Vec<_>>(),
                expected.keys().collect::<Vec<_>>()
            );
            for (key, value) in expected {
                assert_tree(actual[&key].get(), value.get());
            }
        }
        b'[' => {
            let actual: Vec<Box<RawValue>> = serde_json::from_str(actual).unwrap();
            let expected: Vec<Box<RawValue>> = serde_json::from_str(expected).unwrap();
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(&expected) {
                assert_tree(actual.get(), expected.get());
            }
        }
        b'"' => assert_eq!(
            serde_json::from_str::<String>(actual).unwrap(),
            serde_json::from_str::<String>(expected).unwrap()
        ),
        _ => assert_eq!(actual, expected),
    }
}

#[derive(Deserialize)]
struct Fixture {
    name: String,
    op: String,
    current: Box<RawValue>,
    body: Box<RawValue>,
    expected: Box<RawValue>,
    changed: bool,
}

#[test]
fn deterministic_transformations_and_no_ops() {
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!("fixtures.json")).unwrap();
    assert_eq!(fixtures.len(), 17);
    for fixture in fixtures {
        let operation = match fixture.op.as_str() {
            "put" => Operation::Replace,
            "patch" => Operation::MergePatch,
            _ => panic!("unknown fixture operation"),
        };
        let result = prepare(
            operation,
            "doc",
            fixture.current.get().as_bytes(),
            fixture.body.get().as_bytes(),
            LIMIT,
        )
        .unwrap_or_else(|error| panic!("{}: {error}", fixture.name));
        assert_eq!(result.changed(), fixture.changed, "{}", fixture.name);
        assert_tree(
            std::str::from_utf8(result.json()).unwrap(),
            fixture.expected.get(),
        );
        if !fixture.changed {
            assert_eq!(result.json(), fixture.current.get().as_bytes());
        }
        // Every successful patch is idempotent when applied again.
        let again = prepare(
            operation,
            "doc",
            result.json(),
            fixture.body.get().as_bytes(),
            LIMIT,
        )
        .unwrap();
        assert!(!again.changed(), "{}", fixture.name);
    }
}

#[test]
fn no_op_preserves_original_bytes() {
    let result = prepare(Operation::Replace, "doc", CURRENT, br#"{"a":1}"#, LIMIT).unwrap();
    assert!(!result.changed());
    assert_eq!(result.json(), CURRENT);
}

#[test]
fn exact_output_keeps_numeric_tokens_and_ordinary_reserved_objects() {
    let result = prepare(
        Operation::MergePatch,
        "doc",
        br#"{"_id":"doc"}"#,
        br#"{"n":1.00e+99999,"z":-0,"o":{"$serde_json::private::Number":"secret"}}"#,
        LIMIT,
    )
    .unwrap();
    assert_eq!(
        result.json(),
        br#"{"_id":"doc","n":1.00e+99999,"o":{"$serde_json::private::Number":"secret"},"z":-0}"#
    );
    assert!(!format!("{result:?}").contains("secret"));
}

#[test]
fn root_identity_is_immutable() {
    for body in [
        br#"{"_id":"doc"}"#.as_slice(),
        br#"{"_id":"other"}"#,
        br#"{"_id":null}"#,
        br#"{"_id":123}"#,
        br#"{"\u005fid":"doc"}"#,
    ] {
        assert_eq!(
            prepare(Operation::MergePatch, "doc", CURRENT, body, LIMIT).unwrap_err(),
            Error::ImmutableId
        );
    }
    assert_eq!(
        prepare(
            Operation::Replace,
            "doc",
            CURRENT,
            br#"{"_id":"other"}"#,
            LIMIT
        )
        .unwrap_err(),
        Error::IdMismatch
    );
}

#[test]
fn rejects_invalid_input_without_changing_current() {
    let original = CURRENT.to_vec();
    for body in [
        b"null".as_slice(),
        b"[]",
        b"{",
        br#"{"a":1,"\u0061":2}"#,
        br#"{"obj":{"a":1,"a":2}}"#,
        br#"{"a":NaN}"#,
        &[0xff],
    ] {
        for operation in [Operation::Replace, Operation::MergePatch] {
            assert!(prepare(operation, "doc", &original, body, LIMIT).is_err());
            assert_eq!(original, CURRENT);
        }
    }
    for id in ["", "../doc", "á", &"a".repeat(129)] {
        assert_eq!(
            prepare(Operation::Replace, id, CURRENT, b"{}", LIMIT).unwrap_err(),
            Error::InvalidRouteId
        );
    }
    for current in [b"{}".as_slice(), br#"{"_id":"other"}"#, b"invalid"] {
        assert_eq!(
            prepare(Operation::Replace, "doc", current, b"{}", LIMIT).unwrap_err(),
            Error::InvalidCurrentDocument
        );
    }
}

#[test]
fn final_size_checked_after_merge_and_id_injection() {
    let current = br#"{"_id":"doc","a":"1234567890"}"#;
    let body = br#"{"b":"1234567890"}"#;
    assert_eq!(
        prepare(Operation::MergePatch, "doc", current, body, current.len()).unwrap_err(),
        Error::Document(DocumentError::DocumentTooLarge)
    );
    let result = prepare(Operation::MergePatch, "doc", current, body, LIMIT).unwrap();
    assert!(
        prepare(
            Operation::MergePatch,
            "doc",
            current,
            body,
            result.json().len()
        )
        .is_ok()
    );
    let empty = br#"{"_id":"doc"}"#;
    let body = br#"{"a":1}"#;
    assert_eq!(
        prepare(Operation::Replace, "doc", empty, body, empty.len()).unwrap_err(),
        Error::Document(DocumentError::DocumentTooLarge)
    );
    assert_eq!(
        prepare(
            Operation::Replace,
            "doc",
            empty,
            &vec![b' '; LIMIT + 1],
            LIMIT
        )
        .unwrap_err(),
        Error::Document(DocumentError::DocumentTooLarge)
    );
}

#[test]
fn depth_boundary_and_nested_nulls() {
    let nested = format!("{{\"a\":{}0{}}}", "[".repeat(63), "]".repeat(63));
    let result = prepare(
        Operation::MergePatch,
        "doc",
        br#"{"_id":"doc"}"#,
        nested.as_bytes(),
        LIMIT,
    )
    .unwrap();
    DocumentInput::parse(result.json(), LIMIT).unwrap();
    let too_deep = format!("{{\"a\":{}0{}}}", "[".repeat(64), "]".repeat(64));
    assert_eq!(
        prepare(
            Operation::MergePatch,
            "doc",
            CURRENT,
            too_deep.as_bytes(),
            LIMIT
        )
        .unwrap_err(),
        Error::Document(DocumentError::DocumentTooDeep)
    );
}
