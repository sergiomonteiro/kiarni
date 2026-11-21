use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput};
use kiarni_engine::query::{Error, Query};

fn parse(filter: serde_json::Value) -> Query {
    Query::parse(
        &serde_json::to_vec(&serde_json::json!({"from":"items","limit":10,"filter":filter}))
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn direct_equality_decodes_strings_and_preserves_literal_field_names() {
    for (field, value) in [
        ("status", "ready"),
        ("a.b", "\0"),
        ("espaço literal", "é"),
        ("e\u{301}", "e\u{301}"),
    ] {
        let q = parse(serde_json::json!({"op":"eq","field":field,"value":value}));
        let before = q.to_json().unwrap();
        assert_eq!(
            q.direct_string_equality().unwrap(),
            Some((field, value.into()))
        );
        assert_eq!(q.to_json().unwrap(), before);
    }
    let q=Query::parse(br#"{"from":"items","limit":10,"filter":{"op":"eq","field":"status","value":"\u0072eady"}}"#).unwrap();
    assert_eq!(
        q.direct_string_equality().unwrap(),
        Some(("status", "ready".into()))
    );
    let value = "é".repeat(65);
    let q = parse(serde_json::json!({"op":"eq","field":"status","value":value}));
    assert_eq!(q.direct_string_equality().unwrap().unwrap().1.len(), 130);
}

#[test]
fn accessor_does_not_extract_compositions_ranges_or_other_types() {
    let q = Query::parse(br#"{"from":"items","limit":10}"#).unwrap();
    assert_eq!(q.direct_string_equality().unwrap(), None);
    for filter in [
        serde_json::json!({"op":"eq","field":"x","value":null}),
        serde_json::json!({"op":"eq","field":"x","value":true}),
        serde_json::json!({"op":"eq","field":"x","value":9007199254740993u64}),
        serde_json::json!({"op":"gt","field":"x","value":"ready"}),
        serde_json::json!({"op":"exists","field":"x"}),
        serde_json::json!({"op":"all","args":[{"op":"eq","field":"x","value":"ready"}]}),
        serde_json::json!({"op":"any","args":[{"op":"eq","field":"x","value":"ready"}]}),
        serde_json::json!({"op":"not","arg":{"op":"eq","field":"x","value":"ready"}}),
    ] {
        assert_eq!(parse(filter).direct_string_equality().unwrap(), None);
    }
    assert!(matches!(
        Query::parse(br#"{"from":"items","limit":10,"filter":{"op":"eq","field":"x","value":[]}}"#),
        Err(Error::InvalidQuery)
    ));
}

#[test]
fn inspection_does_not_change_evaluation_or_id_priority_information() {
    let q=Query::parse(br#"{"from":"items","id":"b","limit":1,"filter":{"op":"eq","field":"a.b","value":"ready"},"project":["n"]}"#).unwrap();
    let documents = [
        br#"{"_id":"a","a.b":"ready","n":1}"#.as_slice(),
        br#"{"_id":"b","a.b":"ready","n":9007199254740993}"#,
    ]
    .map(|raw| DocumentInput::parse(raw, DEFAULT_MAX_DOCUMENT_BYTES).unwrap());
    let before = q.evaluate_documents(&documents).unwrap();
    assert_eq!(
        q.direct_string_equality().unwrap(),
        Some(("a.b", "ready".into()))
    );
    assert_eq!(q.id(), Some("b"));
    assert_eq!(q.evaluate_documents(&documents).unwrap(), before);
    assert_eq!(
        before,
        vec![br#"{"_id":"b","n":9007199254740993}"#.to_vec()]
    );
}
