//! Offline candidate lifecycle; no CLI or server integration.
use super::*;
use mutations::{Failure, begin_write, index_value};
mod tests;

const MAX_DOCUMENTS: u64 = 10000;
const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Action<'a> {
    Build(&'a str),
    Rebuild,
    Drop,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    Cleared,
    Progress,
    Built,
    Ready,
    Audited,
    BeforeCommit,
    Committed,
}

fn audit(db: &Database, collection: &Collection) -> Result<(), Error> {
    validate(db)?;
    let read = storage(db.begin_read())?;
    check_collection(&storage(read.open_table(COLLECTIONS))?, collection)?;
    audit_tables(
        collection,
        &storage(read.open_table(DOCUMENTS))?,
        &storage(read.open_table(CATALOG))?,
        &storage(read.open_table(ENTRIES))?,
        &storage(read.open_table(STATS))?,
        &storage(read.open_table(COUNTS))?,
    )
}

// Stream one document or index record at a time; never collect a whole index.
fn audit_tables(
    collection: &Collection,
    documents: &impl ReadableTable<(u64, &'static str), &'static [u8]>,
    catalog: &impl ReadableTable<u64, &'static [u8]>,
    entries: &impl ReadableTable<(u64, &'static str, &'static str), ()>,
    stats: &impl ReadableTable<u64, (&'static str, u64, u64)>,
    counts: &impl ReadableTable<(u64, &'static str), u64>,
) -> Result<(), Error> {
    let cid = collection.id;
    let end = cid.checked_add(1).ok_or(Error::InvalidLayout)?;
    let field = storage(catalog.get(cid))?
        .map(|v| decode_definition(v.value()).map(str::to_owned))
        .transpose()?;
    let (mut n, mut bytes) = (0u64, 0u64);
    for row in storage(documents.range((cid, "")..(end, "")))? {
        let (key, value) = storage(row)?;
        let parsed = DocumentInput::parse(value.value(), DEFAULT_MAX_DOCUMENT_BYTES)
            .map_err(|_| Error::InvalidLayout)?;
        if parsed.id() != Some(key.value().1) {
            return Err(Error::InvalidLayout);
        }
        n = n.checked_add(1).ok_or(Error::InvalidLayout)?;
        bytes = bytes
            .checked_add(value.value().len() as u64)
            .ok_or(Error::InvalidLayout)?;
        if let Some(field) = &field
            && let Some(value) = index_value(value.value(), field)?
            && storage(entries.get((cid, value.as_str(), key.value().1)))?.is_none()
        {
            return Err(Error::InvalidLayout);
        }
    }
    let actual = storage(stats.get(cid))?.ok_or(Error::InvalidLayout)?;
    if actual.value() != (collection.name.as_str(), n, bytes) {
        return Err(Error::InvalidLayout);
    }
    let mut previous: Option<String> = None;
    let (mut run, mut groups) = (0u64, 0u64);
    let check_count = |value: &str, expected| -> Result<(), Error> {
        if storage(counts.get((cid, value)))?.map(|v| v.value()) != Some(expected) {
            return Err(Error::InvalidLayout);
        }
        Ok(())
    };
    for row in storage(entries.range((cid, "", "")..(end, "", "")))? {
        let (key, _) = storage(row)?;
        let (_, value, id) = key.value();
        if value.len() > 128 || !valid_name(id) {
            return Err(Error::InvalidLayout);
        }
        let field = field.as_ref().ok_or(Error::InvalidLayout)?;
        let document = storage(documents.get((cid, id)))?.ok_or(Error::InvalidLayout)?;
        if index_value(document.value(), field)?.as_deref() != Some(value) {
            return Err(Error::InvalidLayout);
        }
        if previous.as_deref() != Some(value) {
            if let Some(last) = &previous {
                check_count(last, run)?;
            }
            previous = Some(value.to_owned());
            run = 0;
            groups = groups.checked_add(1).ok_or(Error::InvalidLayout)?;
        }
        run = run.checked_add(1).ok_or(Error::InvalidLayout)?;
    }
    if let Some(last) = &previous {
        check_count(last, run)?;
    }
    let mut actual_groups = 0u64;
    for row in storage(counts.range((cid, "")..(end, "")))? {
        let (key, value) = storage(row)?;
        if key.value().1.len() > 128 || value.value() == 0 {
            return Err(Error::InvalidLayout);
        }
        actual_groups = actual_groups.checked_add(1).ok_or(Error::InvalidLayout)?;
    }
    if groups != actual_groups {
        return Err(Error::InvalidLayout);
    }
    Ok(())
}

fn maintain(
    db: &Database,
    collection: &Collection,
    action: Action<'_>,
    mut point: impl FnMut(Point) -> Result<(), Error>,
) -> Result<(), Failure> {
    let write = stage(db, collection, action, &mut point).map_err(Failure::Aborted)?;
    write.commit().map_err(|_| Failure::CommitUncertain)?;
    point(Point::Committed).map_err(|_| Failure::CommitUncertain)
}

fn stage(
    db: &Database,
    collection: &Collection,
    action: Action<'_>,
    mut point: impl FnMut(Point) -> Result<(), Error>,
) -> Result<redb::WriteTransaction, Error> {
    let definition = match action {
        Action::Build(field) => Some(encode_definition(field)?),
        _ => None,
    };
    let write = begin_write(db, collection)?;
    let cid = collection.id;
    let end = cid.checked_add(1).ok_or(Error::InvalidLayout)?;
    {
        let mut catalog = storage(write.open_table(CATALOG))?;
        let existing = storage(catalog.get(cid))?
            .map(|v| {
                decode_definition(v.value())?;
                Ok(v.value().to_vec())
            })
            .transpose()?;
        let definition = match action {
            Action::Build(_) => {
                if existing.is_some() {
                    return Err(Error::InvalidLayout);
                }
                definition.ok_or(Error::InvalidLayout)?
            }
            Action::Rebuild | Action::Drop => existing.ok_or(Error::InvalidLayout)?,
        };
        let field = decode_definition(&definition)?;
        let mut stats = storage(write.open_table(STATS))?;
        if storage(stats.get(cid))?.is_none_or(|v| v.value().0 != collection.name) {
            return Err(Error::InvalidLayout);
        }
        let mut entries = storage(write.open_table(ENTRIES))?;
        let mut counts = storage(write.open_table(COUNTS))?;
        // Removal is bounded too: a corrupt/oversized old index cannot cause
        // an unlimited maintenance transaction. Abort restores all removals.
        for table in 0..2 {
            let mut removed = 0u64;
            loop {
                let key = if table == 0 {
                    storage(entries.range((cid, "", "")..(end, "", "")))?
                        .next()
                        .map(|row| {
                            let (key, _) = storage(row)?;
                            let (_, value, id) = key.value();
                            if value.len() > 128 || !valid_name(id) {
                                return Err(Error::InvalidLayout);
                            }
                            Ok((value.to_owned(), id.to_owned()))
                        })
                        .transpose()?
                } else {
                    storage(counts.range((cid, "")..(end, "")))?
                        .next()
                        .map(|row| {
                            let (key, _) = storage(row)?;
                            if key.value().1.len() > 128 {
                                return Err(Error::InvalidLayout);
                            }
                            Ok((key.value().1.to_owned(), String::new()))
                        })
                        .transpose()?
                };
                let Some((value, id)) = key else {
                    break;
                };
                removed += 1;
                if removed > MAX_DOCUMENTS {
                    return Err(Error::QueryLimit);
                }
                if table == 0 {
                    storage(entries.remove((cid, value.as_str(), id.as_str())))?;
                } else {
                    storage(counts.remove((cid, value.as_str())))?;
                }
            }
        }
        point(Point::Cleared)?;
        let documents = storage(write.open_table(DOCUMENTS))?;
        let (mut n, mut bytes) = (0u64, 0u64);
        for row in storage(documents.range((cid, "")..(end, "")))? {
            let (key, document) = storage(row)?;
            n += 1;
            bytes = bytes
                .checked_add(document.value().len() as u64)
                .ok_or(Error::QueryLimit)?;
            if n > MAX_DOCUMENTS || bytes > MAX_BYTES {
                return Err(Error::QueryLimit);
            }
            let parsed = DocumentInput::parse(document.value(), DEFAULT_MAX_DOCUMENT_BYTES)
                .map_err(|_| Error::InvalidLayout)?;
            if parsed.id() != Some(key.value().1) {
                return Err(Error::InvalidLayout);
            }
            if !matches!(action, Action::Drop)
                && let Some(value) = index_value(document.value(), field)?
            {
                storage(entries.insert((cid, value.as_str(), key.value().1), ()))?;
                let total = storage(counts.get((cid, value.as_str())))?
                    .map_or(0, |v| v.value())
                    .checked_add(1)
                    .ok_or(Error::InvalidLayout)?;
                storage(counts.insert((cid, value.as_str()), total))?;
            }
            point(Point::Progress)?;
        }
        point(Point::Built)?;
        if matches!(action, Action::Drop) {
            storage(catalog.remove(cid))?;
        } else {
            storage(catalog.insert(cid, definition.as_slice()))?;
        }
        storage(stats.insert(cid, (collection.name.as_str(), n, bytes)))?;
        point(Point::Ready)?;
        audit_tables(collection, &documents, &catalog, &entries, &stats, &counts)?;
        point(Point::Audited)?;
    }
    point(Point::BeforeCommit)?;
    Ok(write)
}
