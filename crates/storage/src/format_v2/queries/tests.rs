use super::*;
use lifecycle::{Action, maintain};
use mutations::{Mutation, apply};

fn collection() -> Collection {
    Collection {
        name: "tasks".into(),
        id: 1,
    }
}
fn fixture(n: usize, wanted: usize, wide: bool) -> (tempfile::TempDir, Database) {
    let temp = tempfile::tempdir().unwrap();
    let db = create(&temp.path().join("store")).unwrap();
    collections::create_collection(&db, "tasks", |_| Ok(())).unwrap();
    let write = db.begin_write().unwrap();
    {
        let mut docs = write.open_table(DOCUMENTS).unwrap();
        for i in 0..n {
            let id = format!("d{i:05}");
            let kind = if i < wanted { "wanted" } else { "bulk" };
            let padding = if wide && i < wanted {
                "x".repeat(850000)
            } else {
                String::new()
            };
            let json = format!(
                "{{\"_id\":\"{id}\",\"kind\":\"{kind}\",\"rank\":{},\"n\":9007199254740993,\"padding\":\"{padding}\"}}",
                n - i
            );
            docs.insert((1, id.as_str()), json.as_bytes()).unwrap();
        }
    }
    write.commit().unwrap();
    maintain(&db, &collection(), Action::Build("kind"), |_| Ok(())).unwrap();
    (temp, db)
}
fn query(value: serde_json::Value) -> Query {
    Query::parse(&serde_json::to_vec(&value).unwrap()).unwrap()
}
fn equality(value: serde_json::Value) -> Query {
    query(
        serde_json::json!({"from":"tasks","limit":1000,"filter":{"op":"eq","field":"kind","value":value},"sort":[{"field":"rank","direction":"desc"}],"project":["kind","rank","n"]}),
    )
}
fn compare(read: &ReadTransaction, query: &Query) -> Result<Output, Failure> {
    let actual = execute(read, &collection(), query, Mode::Automatic);
    let reference = execute(read, &collection(), query, Mode::Scan);
    match (&actual, &reference) {
        (Ok(a), Ok(b)) => assert_eq!(a.documents, b.documents),
        (Err(a), Err(b)) => assert_eq!(a, b),
        _ => panic!("access path changed query success/error"),
    };
    actual
}
fn patch(db: &Database, id: &str, body: &[u8]) {
    apply(
        db,
        &collection(),
        id,
        Mutation::Update(kiarni_engine::writes::Operation::MergePatch, body),
        |_| Ok(()),
    )
    .unwrap();
}

#[test]
fn selective_dense_empty_id_and_threshold_choose_expected_paths() {
    for n in [0, 7, 8, 16] {
        let (_temp, db) = fixture(n, 1, false);
        let read = db.begin_read().unwrap();
        let output = compare(&read, &equality("wanted".into())).unwrap();
        assert_eq!(
            output.plan,
            if n == 7 { Plan::Scan } else { Plan::Secondary }
        );
        assert_eq!(output.examined, if n == 7 { 7 } else { usize::from(n > 0) });
        let absent = compare(&read, &equality("absent".into())).unwrap();
        assert_eq!(absent.plan, Plan::Secondary);
        assert_eq!(absent.examined, 0);
        if n > 1 {
            assert_eq!(
                compare(&read, &equality("bulk".into())).unwrap().plan,
                Plan::Scan
            );
        }
        let id = query(
            serde_json::json!({"from":"tasks","limit":10,"id":"d00000","filter":{"op":"eq","field":"kind","value":"wanted"}}),
        );
        assert_eq!(compare(&read, &id).unwrap().plan, Plan::Primary);
    }
    let (_temp, db) = fixture(16, 2, false);
    let ordered = query(
        serde_json::json!({"from":"tasks","limit":1,"filter":{"op":"eq","field":"kind","value":"wanted"},"sort":[{"field":"rank","direction":"asc"}],"project":["n"]}),
    );
    let output = compare(&db.begin_read().unwrap(), &ordered).unwrap();
    assert_eq!(output.plan, Plan::Secondary);
    assert_eq!(output.examined, 2);
    assert_eq!(
        output.documents,
        vec![br#"{"_id":"d00001","n":9007199254740993}"#.to_vec()]
    );
    let other_field = query(
        serde_json::json!({"from":"tasks","limit":1,"filter":{"op":"eq","field":"n","value":9007199254740993u64}}),
    );
    assert_eq!(
        compare(&db.begin_read().unwrap(), &other_field)
            .unwrap()
            .plan,
        Plan::Scan
    );
    let (_temp, db) = fixture(16, 1, false);
    maintain(&db, &collection(), Action::Drop, |_| Ok(())).unwrap();
    assert_eq!(
        compare(&db.begin_read().unwrap(), &equality("wanted".into()))
            .unwrap()
            .plan,
        Plan::Scan
    );
}

#[test]
fn mixed_json_unicode_and_fallback_predicates_match_scan() {
    let (_temp, db) = fixture(16, 1, false);
    for value in [
        serde_json::Value::Null,
        serde_json::json!(true),
        serde_json::json!(9007199254740993u64),
        serde_json::json!([1]),
        serde_json::json!({"x":1}),
        "".into(),
        "\0".into(),
        "é".repeat(64).into(),
        "é".repeat(65).into(),
        "é".into(),
        "e\u{301}".into(),
        "\u{10ffff}suffix".into(),
    ] {
        let body = serde_json::to_vec(&serde_json::json!({"kind":value})).unwrap();
        apply(
            &db,
            &collection(),
            "d00000",
            Mutation::Update(kiarni_engine::writes::Operation::Replace, &body),
            |_| Ok(()),
        )
        .unwrap();
        if value.is_array() || value.is_object() {
            let ast=serde_json::to_vec(&serde_json::json!({"from":"tasks","limit":10,"filter":{"op":"eq","field":"kind","value":value}})).unwrap();
            assert!(matches!(
                Query::parse(&ast),
                Err(query::Error::InvalidQuery)
            ));
            compare(&db.begin_read().unwrap(), &equality("wanted".into())).unwrap();
            continue;
        }
        let result = compare(&db.begin_read().unwrap(), &equality(value.clone())).unwrap();
        let eligible = value.as_str().is_some_and(|v| v.len() <= 128);
        assert_eq!(
            result.plan,
            if eligible {
                Plan::Secondary
            } else {
                Plan::Scan
            }
        );
    }
    patch(&db, "d00000", br#"{"kind":null}"#);
    compare(
        &db.begin_read().unwrap(),
        &equality(serde_json::Value::Null),
    )
    .unwrap();
    for filter in [
        serde_json::json!({"op":"exists","field":"kind"}),
        serde_json::json!({"op":"gt","field":"kind","value":"a"}),
        serde_json::json!({"op":"all","args":[{"op":"eq","field":"kind","value":"bulk"}]}),
        serde_json::json!({"op":"not","arg":{"op":"eq","field":"kind","value":"bulk"}}),
    ] {
        let q = query(serde_json::json!({"from":"tasks","limit":1000,"filter":filter}));
        assert_eq!(
            compare(&db.begin_read().unwrap(), &q).unwrap().plan,
            Plan::Scan
        );
    }
}

#[test]
fn selected_sort_and_result_errors_are_preserved_without_parsing_excluded_numbers() {
    let (_temp, db) = fixture(16, 1, false);
    patch(&db, "d00001", br#"{"rank":1e10001}"#);
    compare(&db.begin_read().unwrap(), &equality("wanted".into())).unwrap();
    patch(&db, "d00000", br#"{"rank":[]}"#);
    assert_eq!(
        compare(&db.begin_read().unwrap(), &equality("wanted".into())),
        Err(Failure::Query(query::Error::UnsupportedSortValue))
    );
    patch(&db, "d00000", br#"{"rank":1e10001}"#);
    assert_eq!(
        compare(&db.begin_read().unwrap(), &equality("wanted".into())),
        Err(Failure::Query(query::Error::UnsupportedNumber))
    );
    let (_temp, db) = fixture(64, 6, true);
    let q = query(
        serde_json::json!({"from":"tasks","limit":1000,"filter":{"op":"eq","field":"kind","value":"wanted"}}),
    );
    assert_eq!(
        compare(&db.begin_read().unwrap(), &q),
        Err(Failure::Query(query::Error::ResultTooLarge))
    );
}

#[test]
fn budgets_apply_to_the_collection_and_id_keeps_its_exception() {
    let (_temp, db) = fixture(10000, 1, false);
    let q = equality("wanted".into());
    assert_eq!(
        compare(&db.begin_read().unwrap(), &q).unwrap().plan,
        Plan::Secondary
    );
    let input = DocumentInput::parse(
        br#"{"_id":"extra","kind":"bulk"}"#,
        DEFAULT_MAX_DOCUMENT_BYTES,
    )
    .unwrap()
    .prepare_insert()
    .unwrap();
    apply(
        &db,
        &collection(),
        input.id(),
        Mutation::Insert(&input),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        compare(&db.begin_read().unwrap(), &q),
        Err(Failure::Storage(Error::QueryLimit))
    );
    let id = query(serde_json::json!({"from":"tasks","limit":1,"id":"d00000"}));
    assert_eq!(compare(&db.begin_read().unwrap(), &id).unwrap().examined, 1);
    let (_temp, db) = fixture(8, 1, false);
    let existing = db
        .begin_read()
        .unwrap()
        .open_table(STATS)
        .unwrap()
        .get(1)
        .unwrap()
        .unwrap()
        .value()
        .2;
    let mut remaining = 8 * 1024 * 1024 - existing;
    let mut n = 0;
    while remaining > 0 {
        let id = format!("large{n}");
        let shell = format!("{{\"_id\":\"{id}\",\"padding\":\"\"}}");
        let bytes = remaining.min(950000) as usize;
        let json = format!(
            "{{\"_id\":\"{id}\",\"padding\":\"{}\"}}",
            "x".repeat(bytes - shell.len())
        );
        let input = DocumentInput::parse(json.as_bytes(), DEFAULT_MAX_DOCUMENT_BYTES)
            .unwrap()
            .prepare_insert()
            .unwrap();
        assert_eq!(input.json().len(), bytes);
        apply(
            &db,
            &collection(),
            input.id(),
            Mutation::Insert(&input),
            |_| Ok(()),
        )
        .unwrap();
        remaining -= bytes as u64;
        n += 1;
    }
    assert_eq!(
        compare(&db.begin_read().unwrap(), &q).unwrap().plan,
        Plan::Secondary
    );
    let input = DocumentInput::parse(br#"{"_id":"beyond"}"#, DEFAULT_MAX_DOCUMENT_BYTES)
        .unwrap()
        .prepare_insert()
        .unwrap();
    apply(
        &db,
        &collection(),
        input.id(),
        Mutation::Insert(&input),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        compare(&db.begin_read().unwrap(), &q),
        Err(Failure::Storage(Error::QueryLimit))
    );
}

#[test]
fn detected_index_inconsistency_is_an_error_instead_of_fallback() {
    for case in 0..6 {
        let (_temp, db) = fixture(16, 1, false);
        let write = db.begin_write().unwrap();
        match case {
            0 => {
                write
                    .open_table(ENTRIES)
                    .unwrap()
                    .remove((1, "wanted", "d00000"))
                    .unwrap();
            }
            1 | 2 => {
                write
                    .open_table(ENTRIES)
                    .unwrap()
                    .insert(
                        (1, "wanted", if case == 1 { "ghost" } else { "d00001" }),
                        (),
                    )
                    .unwrap();
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .insert((1, "wanted"), 2)
                    .unwrap();
            }
            3 => {
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .insert((1, "wanted"), 0)
                    .unwrap();
            }
            4 => {
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .remove((1, "wanted"))
                    .unwrap();
            }
            5 => {
                write
                    .open_table(COUNTS)
                    .unwrap()
                    .insert((1, "wanted"), 17)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        write.commit().unwrap();
        assert_eq!(
            execute(
                &db.begin_read().unwrap(),
                &collection(),
                &equality("wanted".into()),
                Mode::Automatic
            ),
            Err(Failure::Storage(Error::InvalidLayout))
        );
    }
    let (_temp, db) = fixture(16, 1, false);
    let stale = Collection {
        name: "tasks".into(),
        id: 2,
    };
    assert_eq!(
        execute(
            &db.begin_read().unwrap(),
            &stale,
            &equality("wanted".into()),
            Mode::Automatic
        ),
        Err(Failure::Storage(Error::CollectionMissing))
    );
}

#[test]
fn concurrent_writes_keep_access_plans_equivalent_in_each_snapshot() {
    let (_temp, db) = fixture(32, 1, false);
    let old = db.begin_read().unwrap();
    let q = equality("wanted".into());
    std::thread::scope(|scope| {
        for thread in 0..4 {
            let db = &db;
            scope.spawn(move || {
                for n in 0..20 {
                    patch(
                        db,
                        &format!("d{thread:05}"),
                        if n % 2 == 0 {
                            br#"{"kind":"wanted"}"#
                        } else {
                            br#"{"kind":"other"}"#
                        },
                    );
                }
            });
        }
        for _ in 0..100 {
            let read = db.begin_read().unwrap();
            assert_eq!(compare(&read, &q).unwrap().plan, Plan::Secondary);
        }
    });
    assert_eq!(compare(&old, &q).unwrap().documents.len(), 1);
    lifecycle::audit(&db, &collection()).unwrap();
}
