//! Candidate collection generations. Not exposed through Store or HTTP.
use super::*;
use mutations::{Failure, begin_schema, begin_write};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Point {
    Catalog,
    Statistics,
    Secondary,
    BeforeCommit,
    Committed,
}

pub(super) fn create_collection(
    db: &Database,
    name: &str,
    mut point: impl FnMut(Point) -> Result<(), Error>,
) -> Result<Collection, Failure> {
    let mut stage = || -> Result<_, Error> {
        if !valid_name(name) {
            return Err(Error::InvalidName);
        }
        let write = begin_schema(db)?;
        let id;
        {
            let mut names = storage(write.open_table(COLLECTIONS))?;
            if storage(names.get(name))?.is_some() {
                return Err(Error::CollectionExists);
            }
            let mut metadata = storage(write.open_table(METADATA))?;
            id = storage(metadata.get("next_collection_id"))?
                .ok_or(Error::InvalidLayout)?
                .value();
            let next = id.checked_add(1).ok_or(Error::CollectionIdsExhausted)?;
            // An inconsistent counter must not adopt orphaned data or metadata.
            if storage(storage(write.open_table(STATS))?.get(id))?.is_some()
                || storage(storage(write.open_table(CATALOG))?.get(id))?.is_some()
                || storage(storage(write.open_table(DOCUMENTS))?.range((id, "")..(next, "")))?
                    .next()
                    .is_some()
                || storage(storage(write.open_table(ENTRIES))?.range((id, "", "")..(next, "", "")))?
                    .next()
                    .is_some()
                || storage(storage(write.open_table(COUNTS))?.range((id, "")..(next, "")))?
                    .next()
                    .is_some()
            {
                return Err(Error::InvalidLayout);
            }
            storage(metadata.insert("next_collection_id", next))?;
            storage(names.insert(name, id))?;
            point(Point::Catalog)?;
            storage(storage(write.open_table(STATS))?.insert(id, (name, 0, 0)))?;
            point(Point::Statistics)?;
            point(Point::Secondary)?;
        }
        point(Point::BeforeCommit)?;
        Ok((
            write,
            Collection {
                name: name.into(),
                id,
            },
        ))
    };
    let (write, collection) = stage().map_err(Failure::Aborted)?;
    write.commit().map_err(|_| Failure::CommitUncertain)?;
    point(Point::Committed).map_err(|_| Failure::CommitUncertain)?;
    Ok(collection)
}

pub(super) fn drop_collection(
    db: &Database,
    collection: &Collection,
    mut point: impl FnMut(Point) -> Result<(), Error>,
) -> Result<(), Failure> {
    let mut stage = || -> Result<_, Error> {
        let write = begin_write(db, collection)?;
        let cid = collection.id;
        let end = cid.checked_add(1).ok_or(Error::InvalidLayout)?;
        {
            let mut names = storage(write.open_table(COLLECTIONS))?;
            let documents = storage(write.open_table(DOCUMENTS))?;
            if storage(documents.range((cid, "")..(end, "")))?
                .next()
                .is_some()
            {
                return Err(Error::CollectionNotEmpty);
            }
            let mut stats = storage(write.open_table(STATS))?;
            if storage(stats.get(cid))?
                .is_none_or(|v| v.value() != (collection.name.as_str(), 0, 0))
            {
                return Err(Error::InvalidLayout);
            }
            let entries = storage(write.open_table(ENTRIES))?;
            let counts = storage(write.open_table(COUNTS))?;
            if storage(entries.range((cid, "", "")..(end, "", "")))?
                .next()
                .is_some()
                || storage(counts.range((cid, "")..(end, "")))?
                    .next()
                    .is_some()
            {
                return Err(Error::InvalidLayout);
            }
            let mut catalog = storage(write.open_table(CATALOG))?;
            if let Some(definition) = storage(catalog.get(cid))? {
                decode_definition(definition.value())?;
            }
            storage(catalog.remove(cid))?;
            point(Point::Secondary)?;
            storage(stats.remove(cid))?;
            point(Point::Statistics)?;
            storage(names.remove(collection.name.as_str()))?;
            point(Point::Catalog)?;
        }
        point(Point::BeforeCommit)?;
        Ok(write)
    };
    let write = stage().map_err(Failure::Aborted)?;
    write.commit().map_err(|_| Failure::CommitUncertain)?;
    point(Point::Committed).map_err(|_| Failure::CommitUncertain)
}

mod tests;
