//! Offline index operations under the installation lock. Never activates auth.
use crate::bootstrap::Root;
use kiarni_config::Config;
use kiarni_storage::{Error, ExportSummary, IndexedWriteError};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy)]
pub enum IndexCommand<'a> {
    Create { collection: &'a str, field: &'a str },
    Rebuild { collection: &'a str },
    Drop { collection: &'a str },
    Audit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Point {
    Opened,
    Committed,
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn write_error(error: IndexedWriteError) -> Box<dyn std::error::Error> {
    match error {
        IndexedWriteError::Aborted(Error::QueryLimit) => "index_maintenance_limit".into(),
        other => Box::new(other),
    }
}

/// Returns a global audit summary only for Audit; mutations concern one index.
pub fn manage_index(config: Config, command: IndexCommand<'_>) -> Result<Option<ExportSummary>> {
    run(config, command, |_| Ok(()))
}

fn run(
    config: Config,
    command: IndexCommand<'_>,
    mut point: impl FnMut(Point) -> Result<()>,
) -> Result<Option<ExportSummary>> {
    match command {
        IndexCommand::Create { collection, field } => {
            if !valid_name(collection) {
                return Err("invalid_index_collection".into());
            }
            if !valid_name(field) || field == "_id" {
                return Err("invalid_index_field".into());
            }
        }
        IndexCommand::Rebuild { collection } | IndexCommand::Drop { collection } => {
            if !valid_name(collection) {
                return Err("invalid_index_collection".into());
            }
        }
        IndexCommand::Audit => {}
    }
    // Root is declared first and outlives every store/maintenance handle below.
    let root = Root::lock_existing(config.data_dir().as_ref())?;
    match command {
        IndexCommand::Create { collection, field } => {
            let store = root.open_initialized_store()?;
            let collection = store.collection(collection)?;
            if store.index_field(&collection)?.is_some() {
                return Err("index_already_exists".into());
            }
            point(Point::Opened)?;
            store
                .create_index(&collection, field)
                .map_err(write_error)?;
            point(Point::Committed).map_err(|_| "commit_uncertain")?;
            Ok(None)
        }
        IndexCommand::Rebuild { collection } | IndexCommand::Drop { collection } => {
            let maintenance = root.index_maintenance()?;
            let collection = maintenance.collection(collection)?;
            if maintenance.index_field(&collection)?.is_none() {
                return Err("index_not_found".into());
            }
            point(Point::Opened)?;
            match command {
                IndexCommand::Rebuild { .. } => maintenance.rebuild_index(&collection),
                _ => maintenance.drop_index(&collection),
            }
            .map_err(write_error)?;
            point(Point::Committed).map_err(|_| "commit_uncertain")?;
            Ok(None)
        }
        IndexCommand::Audit => {
            let maintenance = root.index_maintenance()?;
            point(Point::Opened)?;
            Ok(Some(maintenance.audit()?))
        }
    }
}

#[cfg(test)]
mod tests;
