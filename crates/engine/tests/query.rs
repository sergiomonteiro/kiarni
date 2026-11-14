use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentInput};
use kiarni_engine::query::{Error, Query};
use serde_json::{Value, value::RawValue};

fn documents() -> Vec<DocumentInput> {
    let documents: Vec<Box<RawValue>> =
        serde_json::from_str(include_str!("fixtures/documents.json")).unwrap();
    documents
        .into_iter()
        .map(|raw| DocumentInput::parse(raw.get().as_bytes(), DEFAULT_MAX_DOCUMENT_BYTES).unwrap())
        .collect()
}

fn ids(output: &[Vec<u8>]) -> Vec<String> {
    output
        .iter()
        .map(|json| {
            DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
                .unwrap()
                .id()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn deterministic_contract_corpus_and_serialization_roundtrip() {
    // Keep query literals raw too, avoiding any precision/representation changes.
    let cases: Vec<Box<RawValue>> =
        serde_json::from_str(include_str!("fixtures/queries.json")).unwrap();
    for case in cases {
        let fields: std::collections::BTreeMap<String, Box<RawValue>> =
            serde_json::from_str(case.get()).unwrap();
        let metadata: Value = serde_json::from_str(case.get()).unwrap();
        let name = metadata["name"].as_str().unwrap();
        let query = Query::parse(fields["query"].get().as_bytes()).unwrap();
        assert_eq!(query.collection(), "orders");
        let roundtrip = Query::parse(&query.to_json().unwrap()).unwrap();
        let mut documents = documents();
        documents.reverse();
        match query.evaluate_fixture(&documents) {
            Ok(output) => {
                let expected: Vec<_> = metadata["ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(ids(&output), expected, "{name}");
                assert_eq!(
                    roundtrip.evaluate_fixture(&documents).unwrap(),
                    output,
                    "{name}"
                );
                if let Some(fields) = metadata["fields"].as_array() {
                    let output: Value = serde_json::from_slice(&output[0]).unwrap();
                    assert_eq!(
                        output
                            .as_object()
                            .unwrap()
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>(),
                        fields
                            .iter()
                            .map(|v| v.as_str().unwrap().to_owned())
                            .collect::<Vec<_>>(),
                        "{name}"
                    );
                }
            }
            Err(error) => assert_eq!(error.code(), metadata["error"].as_str().unwrap(), "{name}"),
        }
    }
}

#[test]
fn strict_schema_and_duplicate_keys() {
    for input in [
        r#"{"from":"orders"}"#,
        r#"{"from":"orders","limit":1,"unknown":true}"#,
        r#"{"from":"orders","limit":1,"filter":null}"#,
        r#"{"from":"orders","limit":1,"project":null}"#,
        r#"{"from":"orders","limit":1,"filter":{"op":"eq","field":"x"}}"#,
        r#"{"from":"orders","limit":1,"filter":{"op":"eq","field":"x","value":{}}}"#,
        r#"{"from":"orders","limit":1,"filter":{"op":"eq","field":"x","value":[]}}"#,
        r#"{"from":"orders","limit":1,"filter":{"op":"exists","field":"x","value":null}}"#,
        r#"{"from":"orders","limit":1,"filter":{"op":"all","args":[]}}"#,
        r#"{"from":"orders","limit":1,"filter":{"op":"not"}}"#,
        r#"{"from":"orders","limit":1,"filter":{"op":"eval","code":"anything"}}"#,
        r#"{"from":"orders","limit":1,"sort":[{"field":"x"}]}"#,
        r#"{"from":"orders","limit":1,"sort":[{"field":"x","direction":"desc","extra":1}]}"#,
        r#"{"from":"orders","limit":1,"project":["x","x"]}"#,
        r#"{"from":"orders","limit":1,"sort":[{"field":"x","direction":"asc"},{"field":"x","direction":"desc"}]}"#,
    ] {
        assert!(Query::parse(input.as_bytes()).is_err(), "{input}");
    }
    for input in [
        r#"{"from":"orders","limit":1,"limit":2}"#,
        r#"{"from":"orders","limit":1,"filter":{"op":"eq","field":"x","value":1,"value":2}}"#,
    ] {
        assert_eq!(
            Query::parse(input.as_bytes()).unwrap_err(),
            Error::InvalidJson
        );
    }
}

#[test]
fn mixed_scalar_sort_and_descending_tiebreak_are_deterministic() {
    let filter = r#""filter":{"op":"all","args":[{"op":"not","arg":{"op":"eq","field":"_id","value":"g"}},{"op":"not","arg":{"op":"eq","field":"_id","value":"h"}}]}"#;
    for (direction, expected) in [
        (
            "asc",
            vec!["b", "a", "c", "d", "e", "i", "l", "f", "k", "j"],
        ),
        (
            "desc",
            vec!["j", "k", "f", "l", "i", "e", "d", "c", "a", "b"],
        ),
    ] {
        let json = format!(
            r#"{{"from":"orders","limit":20,{filter},"sort":[{{"field":"x","direction":"{direction}"}}]}}"#
        );
        let query = Query::parse(json.as_bytes()).unwrap();
        let mut dataset = documents();
        let first = query.evaluate_fixture(&dataset).unwrap();
        dataset.reverse();
        assert_eq!(query.evaluate_fixture(&dataset).unwrap(), first);
        assert_eq!(ids(&first), expected);
    }
}

#[test]
fn numeric_limits_do_not_prevent_lossless_projection() {
    let document =
        DocumentInput::parse(br#"{"_id":"a","n":1e10001}"#, DEFAULT_MAX_DOCUMENT_BYTES).unwrap();
    let query = Query::parse(br#"{"from":"orders","limit":1,"project":["n"]}"#).unwrap();
    let output = query.evaluate_fixture(&[document]).unwrap();
    assert_eq!(
        std::str::from_utf8(&output[0]).unwrap(),
        r#"{"_id":"a","n":1e10001}"#
    );
    assert_eq!(
        Query::parse(
            br#"{"from":"orders","limit":1,"filter":{"op":"eq","field":"n","value":1e10001}}"#
        )
        .unwrap_err(),
        Error::UnsupportedNumber
    );
    let document =
        DocumentInput::parse(br#"{"_id":"a","n":1e10001}"#, DEFAULT_MAX_DOCUMENT_BYTES).unwrap();
    let query =
        Query::parse(br#"{"from":"orders","limit":1,"sort":[{"field":"n","direction":"asc"}]}"#)
            .unwrap();
    assert_eq!(
        query.evaluate_fixture(&[document]),
        Err(Error::UnsupportedNumber)
    );
}

#[test]
fn dataset_identity_and_count_are_validated_before_filtering() {
    let query = Query::parse(br#"{"from":"orders","limit":1,"id":"absent"}"#).unwrap();
    assert_eq!(
        query.evaluate_fixture(&[DocumentInput::parse(b"{}", 2).unwrap()]),
        Err(Error::InvalidDataset)
    );
    let document = || DocumentInput::parse(br#"{"_id":"a"}"#, 128).unwrap();
    assert_eq!(
        query.evaluate_fixture(&[document(), document()]),
        Err(Error::InvalidDataset)
    );
    let many: Vec<_> = (0..257)
        .map(|i| DocumentInput::parse(format!(r#"{{"_id":"id{i}"}}"#).as_bytes(), 128).unwrap())
        .collect();
    assert_eq!(query.evaluate_fixture(&many), Err(Error::DatasetLimit));
}

#[test]
fn limits_are_explicit_and_debug_is_redacted() {
    for limit in [0, 1001] {
        assert_eq!(
            Query::parse(format!(r#"{{"from":"orders","limit":{limit}}}"#).as_bytes()).unwrap_err(),
            Error::InvalidLimit
        );
    }
    for limit in [1, 1000] {
        assert!(Query::parse(format!(r#"{{"from":"orders","limit":{limit}}}"#).as_bytes()).is_ok());
    }
    assert_eq!(
        Query::parse(&vec![b' '; 16385]).unwrap_err(),
        Error::QueryTooLarge
    );
    let query = Query::parse(
        br#"{"from":"secret","limit":1,"filter":{"op":"eq","field":"secret","value":"secret"}}"#,
    )
    .unwrap();
    assert_eq!(format!("{query:?}"), "Query { .. }");
    for error in [Error::InvalidQuery, Error::UnsupportedNumber] {
        assert_eq!(error.to_string(), error.code());
    }
}

#[test]
fn predicate_depth_and_node_count_have_inclusive_boundaries() {
    for depth in [8, 9] {
        let mut predicate = r#"{"op":"exists","field":"x"}"#.to_owned();
        for _ in 1..depth {
            predicate = format!(r#"{{"op":"not","arg":{predicate}}}"#);
        }
        let query = format!(r#"{{"from":"orders","limit":1,"filter":{predicate}}}"#);
        assert_eq!(Query::parse(query.as_bytes()).is_ok(), depth == 8);
    }
    for children in [63, 64] {
        let args = vec![r#"{"op":"exists","field":"x"}"#; children].join(",");
        let query =
            format!(r#"{{"from":"orders","limit":1,"filter":{{"op":"all","args":[{args}]}}}}"#);
        assert_eq!(Query::parse(query.as_bytes()).is_ok(), children == 63);
    }
}

#[test]
fn byte_budgets_abort_without_partial_results_and_projection_can_reduce_output() {
    let query = Query::parse(br#"{"from":"orders","limit":20}"#).unwrap();
    let large: Vec<_> = (0..10)
        .map(|i| {
            let json = format!(r#"{{"_id":"id{i}","payload":"{}"}}"#, "x".repeat(900_000));
            DocumentInput::parse(json.as_bytes(), DEFAULT_MAX_DOCUMENT_BYTES).unwrap()
        })
        .collect();
    assert_eq!(
        query.evaluate_fixture(&large[..5]),
        Err(Error::ResultTooLarge)
    );
    assert_eq!(query.evaluate_fixture(&large), Err(Error::DatasetLimit));
    let projected = Query::parse(br#"{"from":"orders","limit":20,"project":[]}"#).unwrap();
    assert_eq!(projected.evaluate_fixture(&large[..5]).unwrap().len(), 5);
}

#[test]
fn boolean_short_circuit_and_type_mismatch_do_not_examine_unsupported_numbers() {
    let dataset = [DocumentInput::parse(br#"{"_id":"a","n":1e10001}"#, 128).unwrap()];
    let predicates = [
        (
            r#"{"op":"all","args":[{"op":"exists","field":"missing"},{"op":"eq","field":"n","value":0}]}"#,
            false,
        ),
        (
            r#"{"op":"any","args":[{"op":"exists","field":"n"},{"op":"eq","field":"n","value":0}]}"#,
            true,
        ),
        (r#"{"op":"eq","field":"n","value":"1e10001"}"#, false),
    ];
    for (predicate, expected) in predicates {
        let query = Query::parse(
            format!(r#"{{"from":"orders","limit":1,"filter":{predicate}}}"#).as_bytes(),
        )
        .unwrap();
        assert_eq!(
            !query.evaluate_fixture(&dataset).unwrap().is_empty(),
            expected
        );
    }
}

#[test]
fn scalar_strings_decode_escapes_without_case_or_unicode_normalization() {
    let dataset = [DocumentInput::parse(br#"{"_id":"a","s":"\u00e9"}"#, 128).unwrap()];
    for (text, expected) in [("é", true), ("e\u{301}", false), ("É", false)] {
        let query = Query::parse(format!(r#"{{"from":"orders","limit":1,"filter":{{"op":"eq","field":"s","value":"{text}"}}}}"#).as_bytes()).unwrap();
        assert_eq!(
            !query.evaluate_fixture(&dataset).unwrap().is_empty(),
            expected
        );
    }
}

#[test]
fn sort_projection_and_name_limits_have_boundaries() {
    for count in [4, 5] {
        let sort = (0..count)
            .map(|i| format!(r#"{{"field":"f{i}","direction":"asc"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let query = format!(r#"{{"from":"orders","limit":1,"sort":[{sort}]}}"#);
        assert_eq!(Query::parse(query.as_bytes()).is_ok(), count == 4);
    }
    for count in [32, 33] {
        let project = (0..count)
            .map(|i| format!("\"f{i}\""))
            .collect::<Vec<_>>()
            .join(",");
        let query = format!(r#"{{"from":"orders","limit":1,"project":[{project}]}}"#);
        assert_eq!(Query::parse(query.as_bytes()).is_ok(), count == 32);
    }
    for length in [128, 129] {
        let name = "a".repeat(length);
        let query = format!(r#"{{"from":"{name}","id":"{name}","limit":1,"project":["{name}"]}}"#);
        assert_eq!(Query::parse(query.as_bytes()).is_ok(), length == 128);
    }
    let mut exact = br#"{"from":"orders","limit":1}"#.to_vec();
    exact.resize(16 * 1024, b' ');
    assert!(Query::parse(&exact).is_ok());
}

#[test]
fn storage_executor_accepts_more_documents_than_the_fixture_harness() {
    let documents: Vec<_> = (0..300)
        .map(|index| {
            kiarni_core::DocumentInput::parse(
                format!(r#"{{"_id":"d{index:03}","n":{index}}}"#).as_bytes(),
                1024,
            )
            .unwrap()
        })
        .collect();
    let query = kiarni_engine::query::Query::parse(
        br#"{"from":"orders","limit":2,"sort":[{"field":"n","direction":"desc"}],"project":[]}"#,
    )
    .unwrap();
    assert!(matches!(
        query.evaluate_fixture(&documents),
        Err(kiarni_engine::query::Error::DatasetLimit)
    ));
    assert_eq!(
        query.evaluate_documents(&documents).unwrap(),
        [br#"{"_id":"d299"}"#.to_vec(), br#"{"_id":"d298"}"#.to_vec()]
    );
}
