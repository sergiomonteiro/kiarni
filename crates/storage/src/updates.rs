//! Internal grouped PUT/PATCH primitive. This is not a public HTTP transaction API.
#[cfg(test)]
mod tests;

use crate::{COLLECTIONS, DOCUMENTS, Error, METADATA, Store, storage, valid_name};
use kiarni_core::{DEFAULT_MAX_DOCUMENT_BYTES, DocumentError};
use kiarni_engine::writes::{self, Operation, PreparedWrite};
use redb::{Durability, ReadableTable, ReadableTableMetadata, TableHandle};
use std::sync::Arc;

pub const MAX_OPERATIONS: usize = 32;
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Names are resolved inside the write transaction, rather than using stale handles.
pub struct Update<'a> {
    pub collection: &'a str,
    pub id: &'a str,
    pub operation: Operation,
    pub body: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupError {
    InvalidGroup,
    /// No commit was attempted; all staged changes have been abandoned.
    Aborted(Error),
    /// Commit failed or confirmation was lost. Never interpret as a known rollback.
    CommitUncertain,
}
impl std::fmt::Display for GroupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGroup => f.write_str("invalid_update_group"),
            Self::Aborted(error) => error.fmt(f),
            Self::CommitUncertain => f.write_str("commit_uncertain"),
        }
    }
}
impl std::error::Error for GroupError {}
impl From<Error> for GroupError {
    fn from(error: Error) -> Self {
        Self::Aborted(error)
    }
}

/// Returned only after a confirmed commit or a completed no-change abort.
/// Each successful item retains its own result, including intermediate same-ID states.
#[derive(Debug)]
pub struct Receipt {
    pub outcomes: Vec<Result<PreparedWrite, Error>>,
    pub committed: bool,
}

#[derive(Debug)]
pub struct CapturedReceipt {
    pub receipt: Receipt,
    pub transitions: Vec<Option<Transition>>,
}
pub struct Transition {
    pub before: Arc<[u8]>,
    pub after: Arc<[u8]>,
}
impl std::fmt::Debug for Transition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transition").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Checkpoint {
    Staged,
    Committed,
}

impl Store {
    /// Apply a bounded group in input order, preserving a result for each operation.
    /// Domain rejections are individual; storage/integrity errors abort the group.
    pub fn update_group(
        &self,
        updates: &[Update<'_>],
        max_bytes: usize,
    ) -> Result<Receipt, GroupError> {
        self.update_group_with_hook(updates, max_bytes, |_| Ok(()))
    }

    pub fn update_group_captured(
        &self,
        updates: &[Update<'_>],
        max_bytes: usize,
        capture: &[bool],
    ) -> Result<CapturedReceipt, GroupError> {
        self.apply_update_group(updates, max_bytes, Some(capture), |_| Ok(()))
    }

    fn update_group_with_hook(
        &self,
        updates: &[Update<'_>],
        max_bytes: usize,
        hook: impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<Receipt, GroupError> {
        self.apply_update_group(updates, max_bytes, None, hook)
            .map(|captured| captured.receipt)
    }
    fn apply_update_group(
        &self,
        updates: &[Update<'_>],
        max_bytes: usize,
        capture: Option<&[bool]>,
        mut hook: impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<CapturedReceipt, GroupError> {
        self.telemetry
            .measure(crate::telemetry::Operation::UpdateGroup, || {
                if capture.is_some_and(|flags| flags.len() != updates.len()) {
                    return Err(GroupError::InvalidGroup);
                }
                if updates.is_empty()
                    || updates.len() > MAX_OPERATIONS
                    || updates
                        .iter()
                        .try_fold(0usize, |sum, update| sum.checked_add(update.body.len()))
                        .is_none_or(|bytes| bytes > MAX_BODY_BYTES)
                {
                    return Err(GroupError::InvalidGroup);
                }
                if !(1..=DEFAULT_MAX_DOCUMENT_BYTES).contains(&max_bytes) {
                    return Err(GroupError::Aborted(Error::InvalidLimit));
                }
                let mut write = storage(self.db.begin_write())?;
                storage(write.set_durability(Durability::Immediate))?;
                // Write-side open_table can create a missing table. Never silently repair schema.
                let mut tables = 0;
                for table in storage(write.list_tables())? {
                    tables |= match table.name() {
                        "metadata" => 1,
                        "collections" => 2,
                        "documents" => 4,
                        _ => return Err(Error::InvalidLayout.into()),
                    };
                }
                if tables != 7 || storage(write.list_multimap_tables())?.next().is_some() {
                    return Err(Error::InvalidLayout.into());
                }
                let mut outcomes = Vec::with_capacity(updates.len());
                let mut transitions: Vec<Option<Transition>> =
                    (0..updates.len()).map(|_| None).collect();
                let mut changed = false;
                {
                    let metadata = storage(write.open_table(METADATA))?;
                    let version =
                        storage(metadata.get("format_version"))?.map(|value| value.value());
                    let next_collection =
                        storage(metadata.get("next_collection_id"))?.map(|value| value.value());
                    if storage(metadata.len())? != 2
                        || version != Some(1)
                        || next_collection.is_none_or(|id| id == 0)
                    {
                        return Err(Error::InvalidLayout.into());
                    }
                    let collections = storage(write.open_table(COLLECTIONS))?;
                    let mut documents = storage(write.open_table(DOCUMENTS))?;
                    for (index, update) in updates.iter().enumerate() {
                        if !valid_name(update.collection) {
                            outcomes.push(Err(Error::InvalidName));
                            continue;
                        }
                        if !valid_name(update.id) {
                            outcomes.push(Err(Error::Write(writes::Error::InvalidRouteId)));
                            continue;
                        }
                        let Some(collection) = storage(collections.get(update.collection))? else {
                            outcomes.push(Err(Error::CollectionMissing));
                            continue;
                        };
                        let key = (collection.value(), update.id);
                        if key.0 == 0 || next_collection.is_none_or(|next| key.0 >= next) {
                            return Err(Error::InvalidLayout.into());
                        }
                        let Some(current) = storage(documents.get(key))? else {
                            outcomes.push(Err(Error::MissingDocument));
                            continue;
                        };
                        if current.value().len() > DEFAULT_MAX_DOCUMENT_BYTES {
                            return Err(Error::InvalidLayout.into());
                        }
                        let candidate = writes::prepare(
                            update.operation,
                            update.id,
                            current.value(),
                            update.body,
                            max_bytes,
                        );
                        let before = if capture.is_some_and(|flags| flags[index])
                            && candidate
                                .as_ref()
                                .is_ok_and(|candidate| candidate.changed())
                        {
                            Some(Arc::<[u8]>::from(current.value()))
                        } else {
                            None
                        };
                        drop(current);
                        match candidate {
                            Err(writes::Error::InvalidCurrentDocument) => {
                                return Err(Error::InvalidLayout.into());
                            }
                            Err(
                                error @ writes::Error::Document(
                                    DocumentError::DocumentEncodingFailed,
                                ),
                            ) => {
                                return Err(Error::Write(error).into());
                            }
                            Err(error) => outcomes.push(Err(Error::Write(error))),
                            Ok(mut candidate) => {
                                if candidate.changed() {
                                    storage(documents.insert(key, candidate.json()))?;
                                    if let Some(before) = before {
                                        transitions[index] = Some(Transition {
                                            before,
                                            after: candidate.shared_json(),
                                        });
                                    }
                                    changed = true;
                                }
                                outcomes.push(Ok(candidate));
                            }
                        }
                    }
                }
                if changed {
                    hook(Checkpoint::Staged)?;
                    self.commit_transaction(write)
                        .map_err(|_| GroupError::CommitUncertain)?;
                    hook(Checkpoint::Committed).map_err(|_| GroupError::CommitUncertain)?;
                } else {
                    storage(write.abort())?;
                }
                let captured = CapturedReceipt {
                    receipt: Receipt {
                        outcomes,
                        committed: changed,
                    },
                    transitions,
                };
                self.telemetry.group(&captured.receipt);
                Ok(captured)
            })
    }
}
