//! Grouped PUT/PATCH for explicit format 2, using the product internal request/receipt types.
use super::*;
use crate::updates::{
    CapturedReceipt, GroupError, MAX_BODY_BYTES, MAX_OPERATIONS, Receipt, Transition, Update,
};
use kiarni_core::DocumentError;
use kiarni_engine::writes;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Point {
    Operation(usize),
    Staged,
    Committed,
}

pub(super) fn apply(
    db: &Database,
    updates: &[Update<'_>],
    max_bytes: usize,
    capture: Option<&[bool]>,
    mut point: impl FnMut(Point) -> Result<(), Error>,
) -> Result<CapturedReceipt, GroupError> {
    if updates.is_empty()
        || updates.len() > MAX_OPERATIONS
        || updates
            .iter()
            .try_fold(0usize, |sum, u| sum.checked_add(u.body.len()))
            .is_none_or(|n| n > MAX_BODY_BYTES)
        || capture.is_some_and(|v| v.len() != updates.len())
    {
        return Err(GroupError::InvalidGroup);
    }
    if !(1..=DEFAULT_MAX_DOCUMENT_BYTES).contains(&max_bytes) {
        return Err(GroupError::Aborted(Error::InvalidLimit));
    }
    let write = mutations::begin_schema(db)?;
    let next = storage(storage(write.open_table(METADATA))?.get("next_collection_id"))?
        .ok_or(Error::InvalidLayout)?
        .value();
    let mut outcomes = Vec::with_capacity(updates.len());
    let mut transitions: Vec<Option<Transition>> = (0..updates.len()).map(|_| None).collect();
    let mut changed = false;
    for (index, update) in updates.iter().enumerate() {
        if !valid_name(update.collection) {
            outcomes.push(Err(Error::InvalidName));
            continue;
        }
        if !valid_name(update.id) {
            outcomes.push(Err(Error::Write(writes::Error::InvalidRouteId)));
            continue;
        }
        let cid = storage(storage(write.open_table(COLLECTIONS))?.get(update.collection))?
            .map(|v| v.value());
        let Some(cid) = cid else {
            outcomes.push(Err(Error::CollectionMissing));
            continue;
        };
        if cid == 0 || cid >= next {
            return Err(Error::InvalidLayout.into());
        }
        let current = storage(storage(write.open_table(DOCUMENTS))?.get((cid, update.id)))?
            .map(|v| v.value().to_vec());
        let Some(current) = current else {
            outcomes.push(Err(Error::MissingDocument));
            continue;
        };
        if current.len() > DEFAULT_MAX_DOCUMENT_BYTES {
            return Err(Error::InvalidLayout.into());
        }
        let mut prepared = match writes::prepare(
            update.operation,
            update.id,
            &current,
            update.body,
            max_bytes,
        ) {
            Ok(value) => value,
            Err(writes::Error::InvalidCurrentDocument) => return Err(Error::InvalidLayout.into()),
            Err(error @ writes::Error::Document(DocumentError::DocumentEncodingFailed)) => {
                return Err(Error::Write(error).into());
            }
            Err(error) => {
                outcomes.push(Err(Error::Write(error)));
                continue;
            }
        };
        let collection = Collection {
            name: update.collection.into(),
            id: cid,
        };
        let staged = mutations::stage_in(
            &write,
            &collection,
            update.id,
            mutations::Mutation::Prepared(&prepared),
            |_| Ok(()),
        )?;
        if staged != prepared.changed() {
            return Err(Error::InvalidLayout.into());
        }
        if staged {
            if capture.is_some_and(|flags| flags[index]) {
                transitions[index] = Some(Transition {
                    before: Arc::from(current),
                    after: prepared.shared_json(),
                });
            }
            changed = true;
            point(Point::Operation(index))?;
        }
        outcomes.push(Ok(prepared));
    }
    if changed {
        point(Point::Staged)?;
        write.commit().map_err(|_| GroupError::CommitUncertain)?;
        point(Point::Committed).map_err(|_| GroupError::CommitUncertain)?;
    } else {
        storage(write.abort())?;
    }
    Ok(CapturedReceipt {
        receipt: Receipt {
            outcomes,
            committed: changed,
        },
        transitions,
    })
}

#[cfg(test)]
mod tests;
