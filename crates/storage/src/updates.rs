//! Internal grouped PUT/PATCH primitive. This is not a public HTTP transaction API.
#[cfg(test)]
mod tests;

use crate::{Error, Store};
use kiarni_engine::writes::{Operation, PreparedWrite};
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
            .map(|v| v.receipt)
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
                let result = crate::format_v2::groups::apply(
                    Some(&self.telemetry),
                    &self.db,
                    updates,
                    max_bytes,
                    capture,
                    |point| match point {
                        crate::format_v2::groups::Point::Staged => hook(Checkpoint::Staged),
                        crate::format_v2::groups::Point::Committed => hook(Checkpoint::Committed),
                        _ => Ok(()),
                    },
                )?;
                self.telemetry.group(&result.receipt);
                Ok(result)
            })
    }
}
