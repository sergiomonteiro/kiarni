//! Transactional candidate only; no server, group writer or feed integration.
use super::*;
use kiarni_engine::writes::{self, Operation};

pub(super) enum Mutation<'a> {
    Insert(&'a PreparedInsert),
    Update(Operation, &'a [u8]),
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Checkpoint {
    Document,
    OldEntry,
    NewEntry,
    Statistics,
    BeforeCommit,
    Committed,
}

pub(super) fn index_value(json: &[u8], field: &str) -> Result<Option<String>, Error> {
    let document =
        DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES).map_err(|_| Error::InvalidLayout)?;
    let Some(raw) = document.fields().get(field) else {
        return Ok(None);
    };
    if !raw.get().starts_with('"') {
        return Ok(None);
    }
    let value: String = serde_json::from_str(raw.get()).map_err(|_| Error::InvalidLayout)?;
    Ok((value.len() <= 128).then_some(value))
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Aborted(Error),
    CommitUncertain,
}

pub(super) fn apply(
    db: &Database,
    collection: &Collection,
    id: &str,
    mutation: Mutation<'_>,
    mut checkpoint: impl FnMut(Checkpoint) -> Result<(), Error>,
) -> Result<bool, Failure> {
    let staged = stage(db, collection, id, mutation, &mut checkpoint).map_err(Failure::Aborted)?;
    let Some(write) = staged else {
        return Ok(false);
    };
    write.commit().map_err(|_| Failure::CommitUncertain)?;
    checkpoint(Checkpoint::Committed).map_err(|_| Failure::CommitUncertain)?;
    Ok(true)
}

pub(super) fn begin_write(
    db: &Database,
    collection: &Collection,
) -> Result<redb::WriteTransaction, Error> {
    if collection.id == 0 || !valid_name(&collection.name) {
        return Err(Error::InvalidLayout);
    }
    let mut write = storage(db.begin_write())?;
    storage(write.set_durability(Durability::Immediate))?;
    // Write-side open_table can create tables: check all names before any open.
    let mut names = storage(write.list_tables())?
        .take(8)
        .map(|t| t.name().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    if names
        != [
            "collection_stats",
            "collections",
            "documents",
            "index_catalog",
            "metadata",
            "secondary_counts",
            "secondary_entries",
        ]
        || storage(write.list_multimap_tables())?.next().is_some()
    {
        return Err(Error::InvalidLayout);
    }
    {
        let metadata = storage(write.open_table(METADATA))?;
        if storage(metadata.len())? != 2
            || storage(metadata.get("format_version"))?.map(|v| v.value()) != Some(2)
            || storage(metadata.get("next_collection_id"))?
                .is_none_or(|v| v.value() <= collection.id)
        {
            return Err(Error::InvalidLayout);
        }
        let collections = storage(write.open_table(COLLECTIONS))?;
        check_collection(&collections, collection)?;
    }
    Ok(write)
}

fn stage(
    db: &Database,
    collection: &Collection,
    id: &str,
    mutation: Mutation<'_>,
    mut checkpoint: impl FnMut(Checkpoint) -> Result<(), Error>,
) -> Result<Option<redb::WriteTransaction>, Error> {
    if !valid_name(id) {
        return Err(Error::InvalidDocument);
    }
    let write = begin_write(db, collection)?;
    let changed;
    {
        let catalog = storage(write.open_table(CATALOG))?;
        let field = storage(catalog.get(collection.id))?
            .map(|v| decode_definition(v.value()).map(str::to_owned))
            .transpose()?;
        let mut documents = storage(write.open_table(DOCUMENTS))?;
        let old = storage(documents.get((collection.id, id)))?.map(|v| v.value().to_vec());
        if let Some(json) = &old {
            let parsed = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
                .map_err(|_| Error::InvalidLayout)?;
            if parsed.id() != Some(id) {
                return Err(Error::InvalidLayout);
            }
        }
        let new = match mutation {
            Mutation::Insert(input) => {
                if input.id() != id {
                    return Err(Error::InvalidDocument);
                }
                if old.is_some() {
                    return Err(Error::DuplicateId);
                }
                Some(input.json().to_vec())
            }
            Mutation::Update(operation, body) => {
                let current = old.as_deref().ok_or(Error::MissingDocument)?;
                let prepared =
                    writes::prepare(operation, id, current, body, DEFAULT_MAX_DOCUMENT_BYTES)
                        .map_err(Error::Write)?;
                // Preserve the exact previous bytes on semantic no-op.
                Some(if prepared.changed() {
                    prepared.json().to_vec()
                } else {
                    current.to_vec()
                })
            }
            Mutation::Delete => {
                if old.is_none() {
                    return Err(Error::MissingDocument);
                }
                None
            }
        };
        let key = |json: Option<&[u8]>| -> Result<Option<String>, Error> {
            match (json, &field) {
                (Some(json), Some(field)) => index_value(json, field),
                _ => Ok(None),
            }
        };
        let old_key = key(old.as_deref())?;
        let new_key = key(new.as_deref())?;
        let mut entries = storage(write.open_table(ENTRIES))?;
        let mut counts = storage(write.open_table(COUNTS))?;
        let mut stats = storage(write.open_table(STATS))?;
        let (owner, count, bytes) = {
            let stored = storage(stats.get(collection.id))?.ok_or(Error::InvalidLayout)?;
            let (owner, count, bytes) = stored.value();
            (owner.to_owned(), count, bytes)
        };
        if owner != collection.name {
            return Err(Error::InvalidLayout);
        }
        let old_bytes = old.as_ref().map_or(0, |v| v.len() as u64);
        let new_bytes = new.as_ref().map_or(0, |v| v.len() as u64);
        let updated_count = count
            .checked_sub(u64::from(old.is_some()))
            .and_then(|v| v.checked_add(u64::from(new.is_some())))
            .ok_or(Error::InvalidLayout)?;
        let updated_bytes = bytes
            .checked_sub(old_bytes)
            .and_then(|v| v.checked_add(new_bytes))
            .ok_or(Error::InvalidLayout)?;
        if let Some(value) = &old_key
            && (storage(entries.get((collection.id, value.as_str(), id)))?.is_none()
                || storage(counts.get((collection.id, value.as_str())))?
                    .is_none_or(|v| v.value() == 0))
        {
            return Err(Error::InvalidLayout);
        }
        changed = old != new;
        if !changed {
            return Ok(None);
        }
        match &new {
            Some(json) => {
                storage(documents.insert((collection.id, id), json.as_slice()))?;
            }
            None => {
                storage(documents.remove((collection.id, id)))?;
            }
        }
        checkpoint(Checkpoint::Document)?;
        if old_key != new_key {
            if let Some(value) = &old_key {
                storage(entries.remove((collection.id, value.as_str(), id)))?;
                let remaining = storage(counts.get((collection.id, value.as_str())))?
                    .ok_or(Error::InvalidLayout)?
                    .value()
                    .checked_sub(1)
                    .ok_or(Error::InvalidLayout)?;
                if remaining == 0 {
                    storage(counts.remove((collection.id, value.as_str())))?;
                } else {
                    storage(counts.insert((collection.id, value.as_str()), remaining))?;
                }
            }
            checkpoint(Checkpoint::OldEntry)?;
            if let Some(value) = &new_key {
                if storage(entries.get((collection.id, value.as_str(), id)))?.is_some() {
                    return Err(Error::InvalidLayout);
                }
                let previous =
                    storage(counts.get((collection.id, value.as_str())))?.map(|v| v.value());
                if previous == Some(0) {
                    return Err(Error::InvalidLayout);
                }
                let updated = previous
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or(Error::InvalidLayout)?;
                storage(entries.insert((collection.id, value.as_str(), id), ()))?;
                storage(counts.insert((collection.id, value.as_str()), updated))?;
            }
            checkpoint(Checkpoint::NewEntry)?;
        }
        storage(stats.insert(
            collection.id,
            (owner.as_str(), updated_count, updated_bytes),
        ))?;
        checkpoint(Checkpoint::Statistics)?;
    }
    checkpoint(Checkpoint::BeforeCommit)?;
    Ok(Some(write))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(indexed: bool) -> (tempfile::TempDir, Database, Collection) {
        let temp = tempfile::tempdir().unwrap();
        let db = create(&temp.path().join("store")).unwrap();
        let write = db.begin_write().unwrap();
        write
            .open_table(COLLECTIONS)
            .unwrap()
            .insert("tasks", 1)
            .unwrap();
        write
            .open_table(METADATA)
            .unwrap()
            .insert("next_collection_id", 2)
            .unwrap();
        write
            .open_table(STATS)
            .unwrap()
            .insert(1, ("tasks", 0, 0))
            .unwrap();
        if indexed {
            write
                .open_table(CATALOG)
                .unwrap()
                .insert(1, encode_definition("status").unwrap().as_slice())
                .unwrap();
        }
        write.commit().unwrap();
        (
            temp,
            db,
            Collection {
                name: "tasks".into(),
                id: 1,
            },
        )
    }
    fn insert(db: &Database, collection: &Collection, json: &[u8]) -> Result<bool, Failure> {
        let prepared = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
            .unwrap()
            .prepare_insert()
            .unwrap();
        apply(
            db,
            collection,
            prepared.id(),
            Mutation::Insert(&prepared),
            |_| Ok(()),
        )
    }

    // Full reference comparison is test-only and tiny-fixture bounded; lifecycle
    // auditing of arbitrarily large stores remains a separate implementation.
    fn assert_consistent(db: &Database, collection: &Collection) {
        use std::collections::BTreeMap;
        let read = db.begin_read().unwrap();
        let documents = read.open_table(DOCUMENTS).unwrap();
        let catalog = read.open_table(CATALOG).unwrap();
        let field = catalog
            .get(collection.id)
            .unwrap()
            .map(|v| decode_definition(v.value()).unwrap().to_owned());
        let mut expected = BTreeMap::new();
        let mut expected_counts = BTreeMap::new();
        let (mut count, mut bytes) = (0, 0);
        for row in documents.iter().unwrap() {
            let (key, value) = row.unwrap();
            assert_eq!(key.value().0, collection.id);
            count += 1;
            bytes += value.value().len() as u64;
            if let Some(field) = &field {
                let reference: serde_json::Value = serde_json::from_slice(value.value()).unwrap();
                if let Some(value) = reference
                    .get(field)
                    .and_then(serde_json::Value::as_str)
                    .filter(|v| v.len() <= 128)
                {
                    *expected_counts
                        .entry((collection.id, value.to_owned()))
                        .or_insert(0u64) += 1;
                    expected.insert(
                        (collection.id, value.to_owned(), key.value().1.to_owned()),
                        (),
                    );
                }
            }
        }
        let actual: BTreeMap<_, _> = read
            .open_table(ENTRIES)
            .unwrap()
            .iter()
            .unwrap()
            .map(|v| {
                let (key, _) = v.unwrap();
                let (c, value, id) = key.value();
                ((c, value.to_owned(), id.to_owned()), ())
            })
            .collect();
        let counts: BTreeMap<_, _> = read
            .open_table(COUNTS)
            .unwrap()
            .iter()
            .unwrap()
            .map(|v| {
                let (key, value) = v.unwrap();
                ((key.value().0, key.value().1.to_owned()), value.value())
            })
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(counts, expected_counts);
        assert_eq!(
            read.open_table(STATS)
                .unwrap()
                .get(collection.id)
                .unwrap()
                .unwrap()
                .value(),
            (collection.name.as_str(), count, bytes)
        );
    }

    #[test]
    fn lifecycle_preserves_exact_json_and_partial_index() {
        for indexed in [false, true] {
            let (temp, db, collection) = setup(indexed);
            insert(
                &db,
                &collection,
                br#"{"_id":"a","status":"old","n":9007199254740993}"#,
            )
            .unwrap();
            insert(&db, &collection, br#"{"_id":"b","status":"old"}"#).unwrap();
            assert_consistent(&db, &collection);
            assert_eq!(
                insert(&db, &collection, br#"{"_id":"a"}"#),
                Err(Failure::Aborted(Error::DuplicateId))
            );
            let old = db.begin_read().unwrap();
            for body in [
                br#"{"status":"new"}"#.as_slice(),
                br#"{"status":null}"#,
                br#"{"status":42}"#,
                br#"{"status":"\u0000"}"#,
                br#"{"status":""}"#,
            ] {
                assert!(
                    apply(
                        &db,
                        &collection,
                        "a",
                        Mutation::Update(Operation::MergePatch, body),
                        |_| Ok(())
                    )
                    .unwrap()
                );
                assert_consistent(&db, &collection);
            }
            assert_eq!(
                old.open_table(DOCUMENTS)
                    .unwrap()
                    .get((1, "a"))
                    .unwrap()
                    .unwrap()
                    .value(),
                br#"{"_id":"a","n":9007199254740993,"status":"old"}"#
            );
            assert!(
                !apply(
                    &db,
                    &collection,
                    "a",
                    Mutation::Update(Operation::MergePatch, b"{}"),
                    |_| panic!("no-op must not commit")
                )
                .unwrap()
            );
            assert_consistent(&db, &collection);
            apply(
                &db,
                &collection,
                "a",
                Mutation::Update(Operation::MergePatch, br#"{"note":true}"#),
                |point| {
                    assert!(
                        !matches!(point, Checkpoint::OldEntry | Checkpoint::NewEntry),
                        "unchanged indexed field must not rewrite entries"
                    );
                    Ok(())
                },
            )
            .unwrap();
            assert_consistent(&db, &collection);
            for value in [
                "é".repeat(64),
                "é".repeat(65),
                "é".into(),
                "e\u{301}".into(),
            ] {
                let body = serde_json::to_vec(&serde_json::json!({"status":value})).unwrap();
                apply(
                    &db,
                    &collection,
                    "a",
                    Mutation::Update(Operation::MergePatch, &body),
                    |_| Ok(()),
                )
                .unwrap();
                assert_consistent(&db, &collection);
            }
            apply(
                &db,
                &collection,
                "a",
                Mutation::Update(Operation::Replace, br#"{"status":false}"#),
                |_| Ok(()),
            )
            .unwrap();
            assert_consistent(&db, &collection);
            apply(&db, &collection, "a", Mutation::Delete, |_| Ok(())).unwrap();
            assert_eq!(
                apply(&db, &collection, "a", Mutation::Delete, |_| Ok(())),
                Err(Failure::Aborted(Error::MissingDocument))
            );
            assert_consistent(&db, &collection);
            drop(old);
            drop(db);
            let db = open(&temp.path().join("store")).unwrap();
            assert_consistent(&db, &collection);
        }
    }

    #[test]
    fn every_precommit_failure_rolls_back_and_postcommit_failure_persists() {
        for action in 0..3 {
            for failed in [
                Checkpoint::Document,
                Checkpoint::OldEntry,
                Checkpoint::NewEntry,
                Checkpoint::Statistics,
                Checkpoint::BeforeCommit,
                Checkpoint::Committed,
            ] {
                let (temp, db, collection) = setup(true);
                if action != 0 {
                    insert(&db, &collection, br#"{"_id":"a","status":"old"}"#).unwrap();
                }
                let before = db.begin_read().unwrap();
                let prepared = DocumentInput::parse(
                    br#"{"_id":"a","status":"new"}"#,
                    DEFAULT_MAX_DOCUMENT_BYTES,
                )
                .unwrap()
                .prepare_insert()
                .unwrap();
                let mutation = match action {
                    0 => Mutation::Insert(&prepared),
                    1 => Mutation::Update(Operation::MergePatch, br#"{"status":"new"}"#),
                    _ => Mutation::Delete,
                };
                assert_eq!(
                    apply(
                        &db,
                        &collection,
                        "a",
                        mutation,
                        |point| if point == failed {
                            Err(Error::Storage)
                        } else {
                            Ok(())
                        }
                    ),
                    Err(if failed == Checkpoint::Committed {
                        Failure::CommitUncertain
                    } else {
                        Failure::Aborted(Error::Storage)
                    })
                );
                let read = db.begin_read().unwrap();
                let old = before
                    .open_table(DOCUMENTS)
                    .unwrap()
                    .get((1, "a"))
                    .unwrap()
                    .map(|v| v.value().to_vec());
                let new = read
                    .open_table(DOCUMENTS)
                    .unwrap()
                    .get((1, "a"))
                    .unwrap()
                    .map(|v| v.value().to_vec());
                if failed == Checkpoint::Committed {
                    assert_ne!(new, old);
                } else {
                    assert_eq!(new, old);
                }
                assert_consistent(&db, &collection);
                drop(read);
                drop(before);
                drop(db);
                assert_consistent(&open(&temp.path().join("store")).unwrap(), &collection);
            }
        }
    }

    #[test]
    fn corruption_stale_generation_and_overflow_abort_writes() {
        for case in 0..6 {
            let (_temp, db, collection) = setup(true);
            insert(&db, &collection, br#"{"_id":"a","status":"old"}"#).unwrap();
            let write = db.begin_write().unwrap();
            match case {
                0 => {
                    write
                        .open_table(ENTRIES)
                        .unwrap()
                        .remove((1, "old", "a"))
                        .unwrap();
                }
                1 => {
                    write
                        .open_table(COUNTS)
                        .unwrap()
                        .insert((1, "old"), 0)
                        .unwrap();
                }
                2 => {
                    write
                        .open_table(STATS)
                        .unwrap()
                        .insert(1, ("tasks", 0, 0))
                        .unwrap();
                }
                3 => {
                    write
                        .open_table(COLLECTIONS)
                        .unwrap()
                        .insert("tasks", 2)
                        .unwrap();
                }
                4 => {
                    write
                        .open_table(COUNTS)
                        .unwrap()
                        .insert((1, "new"), u64::MAX)
                        .unwrap();
                }
                5 => {
                    write.delete_table(ENTRIES).unwrap();
                }
                _ => unreachable!(),
            }
            write.commit().unwrap();
            let before = db
                .begin_read()
                .unwrap()
                .open_table(DOCUMENTS)
                .unwrap()
                .get((1, "a"))
                .unwrap()
                .unwrap()
                .value()
                .to_vec();
            assert!(
                apply(
                    &db,
                    &collection,
                    "a",
                    Mutation::Update(Operation::MergePatch, br#"{"status":"new"}"#),
                    |_| Ok(())
                )
                .is_err()
            );
            assert_eq!(
                db.begin_read()
                    .unwrap()
                    .open_table(DOCUMENTS)
                    .unwrap()
                    .get((1, "a"))
                    .unwrap()
                    .unwrap()
                    .value(),
                before
            );
            if case == 5 {
                assert!(
                    !db.begin_read()
                        .unwrap()
                        .list_tables()
                        .unwrap()
                        .any(|t| t.name() == "secondary_entries")
                );
            }
        }
    }

    #[test]
    fn invalid_updates_and_noops_preserve_document_and_index() {
        let (_temp, db, collection) = setup(true);
        insert(
            &db,
            &collection,
            br#"{"_id":"a","status":"old","n":9007199254740993}"#,
        )
        .unwrap();
        let before = db
            .begin_read()
            .unwrap()
            .open_table(DOCUMENTS)
            .unwrap()
            .get((1, "a"))
            .unwrap()
            .unwrap()
            .value()
            .to_vec();
        for (operation, body) in [
            (
                Operation::Replace,
                br#"{"_id":"other","status":"new"}"#.as_slice(),
            ),
            (Operation::MergePatch, br#"{"_id":"a"}"#),
            (
                Operation::MergePatch,
                br#"{"status":"new","status":"other"}"#,
            ),
            (Operation::Replace, b"[]"),
            (Operation::MergePatch, b"{"),
        ] {
            assert!(
                apply(
                    &db,
                    &collection,
                    "a",
                    Mutation::Update(operation, body),
                    |_| Ok(())
                )
                .is_err()
            );
            assert_consistent(&db, &collection);
            assert_eq!(
                db.begin_read()
                    .unwrap()
                    .open_table(DOCUMENTS)
                    .unwrap()
                    .get((1, "a"))
                    .unwrap()
                    .unwrap()
                    .value(),
                before
            );
        }
        assert!(
            !apply(
                &db,
                &collection,
                "a",
                Mutation::Update(
                    Operation::Replace,
                    br#"{"n":9007199254740993,"status":"old"}"#
                ),
                |_| panic!("no-op must not commit")
            )
            .unwrap()
        );
        assert_consistent(&db, &collection);
        // An unchanged key still needs its existing back-reference, even on no-op.
        let write = db.begin_write().unwrap();
        write
            .open_table(ENTRIES)
            .unwrap()
            .remove((1, "old", "a"))
            .unwrap();
        write.commit().unwrap();
        assert_eq!(
            apply(
                &db,
                &collection,
                "a",
                Mutation::Update(Operation::MergePatch, b"{}"),
                |_| Ok(())
            ),
            Err(Failure::Aborted(Error::InvalidLayout))
        );
        assert_eq!(
            db.begin_read()
                .unwrap()
                .open_table(DOCUMENTS)
                .unwrap()
                .get((1, "a"))
                .unwrap()
                .unwrap()
                .value(),
            before
        );
    }

    #[test]
    fn concurrent_writers_and_readers_keep_snapshot_equivalence() {
        let (_temp, db, collection) = setup(true);
        for n in 0..4 {
            insert(
                &db,
                &collection,
                serde_json::to_string(&serde_json::json!({"_id":format!("d{n}"),"status":"old"}))
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap();
        }
        std::thread::scope(|scope| {
            for n in 0..4 {
                let (db, collection) = (&db, &collection);
                scope.spawn(move || {
                    for round in 0..20 {
                        let body = serde_json::to_vec(
                            &serde_json::json!({"status":format!("s{}",round%3),"n":round}),
                        )
                        .unwrap();
                        apply(
                            db,
                            collection,
                            &format!("d{n}"),
                            Mutation::Update(Operation::MergePatch, &body),
                            |_| {
                                assert_consistent(db, collection);
                                Ok(())
                            },
                        )
                        .unwrap();
                    }
                });
            }
            for _ in 0..80 {
                assert_consistent(&db, &collection);
            }
        });
        assert_consistent(&db, &collection);
    }
}
