//! Candidate access selection; no HTTP or metrics contract is enabled.
use super::*;
use kiarni_engine::query::{self, Query};
use redb::ReadTransaction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    Primary,
    Secondary,
    Scan,
}
#[derive(Clone, Copy)]
enum Mode {
    Automatic,
    Scan,
}
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Storage(Error),
    Query(query::Error),
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Storage(error)
    }
}
impl From<query::Error> for Failure {
    fn from(error: query::Error) -> Self {
        Self::Query(error)
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Output {
    documents: Vec<Vec<u8>>,
    plan: Plan,
    examined: usize,
    document_bytes: usize,
}

fn literal(query: &Query, field: &str) -> Result<Option<String>, Failure> {
    let ast: serde_json::Value =
        serde_json::from_slice(&query.to_json()?).map_err(|_| Error::InvalidLayout)?;
    let filter = &ast["filter"];
    Ok((filter["op"] == "eq" && filter["field"] == field)
        .then(|| filter["value"].as_str())
        .flatten()
        .filter(|v| v.len() <= 128)
        .map(str::to_owned))
}

fn execute(
    read: &ReadTransaction,
    collection: &Collection,
    query: &Query,
    mode: Mode,
) -> Result<Output, Failure> {
    if collection.name != query.collection() {
        return Err(Error::InvalidName.into());
    }
    check_collection(&storage(read.open_table(COLLECTIONS))?, collection)?;
    let cid = collection.id;
    let metadata = storage(read.open_table(METADATA))?;
    if cid == 0
        || storage(metadata.get("format_version"))?.map(|v| v.value()) != Some(2)
        || storage(metadata.get("next_collection_id"))?.is_none_or(|v| v.value() <= cid)
    {
        return Err(Error::InvalidLayout.into());
    }
    let documents = storage(read.open_table(DOCUMENTS))?;
    let mut chosen = None;
    if query.id().is_none() && matches!(mode, Mode::Automatic) {
        let catalog = storage(read.open_table(CATALOG))?;
        if let Some(definition) = storage(catalog.get(cid))? {
            let field = decode_definition(definition.value())?;
            if let Some(value) = literal(query, field)? {
                let stats = storage(read.open_table(STATS))?;
                let stored = storage(stats.get(cid))?.ok_or(Error::InvalidLayout)?;
                let (owner, count, bytes) = stored.value();
                if owner != collection.name || (count == 0) != (bytes == 0) || bytes < count {
                    return Err(Error::InvalidLayout.into());
                }
                if count <= 10000 && bytes <= 8 * 1024 * 1024 {
                    let counts = storage(read.open_table(COUNTS))?;
                    let expected = storage(counts.get((cid, value.as_str())))?.map(|v| v.value());
                    if expected == Some(0) || expected.is_some_and(|v| v > count) {
                        return Err(Error::InvalidLayout.into());
                    }
                    let expected = expected.unwrap_or(0);
                    if expected <= count / 8 {
                        chosen = Some((field.to_owned(), value, expected));
                    }
                }
            }
        }
    }
    let mut inputs = Vec::new();
    let mut bytes = 0usize;
    let mut append = |id: &str, json: &[u8]| -> Result<(), Error> {
        if inputs.len() >= 10000 || bytes.saturating_add(json.len()) > 8 * 1024 * 1024 {
            return Err(Error::QueryLimit);
        }
        let input = DocumentInput::parse(json, DEFAULT_MAX_DOCUMENT_BYTES)
            .map_err(|_| Error::InvalidLayout)?;
        if input.id() != Some(id) {
            return Err(Error::InvalidLayout);
        }
        bytes += json.len();
        inputs.push(input);
        Ok(())
    };
    let plan = if let Some(id) = query.id() {
        // Scan mode disables only secondary selection: preserve the product's ID
        // lookup and its budget exception in both the reference and new paths.
        if let Some(value) = storage(documents.get((cid, id)))? {
            append(id, value.value())?;
        }
        Plan::Primary
    } else if let Some((field, value, expected)) = chosen {
        let entries = storage(read.open_table(ENTRIES))?;
        let mut seen = 0u64;
        for row in
            storage(entries.range((cid, value.as_str(), "")..=(cid, value.as_str(), "\u{7f}")))?
        {
            let (key, _) = storage(row)?;
            let id = key.value().2;
            if seen >= expected || !valid_name(id) {
                return Err(Error::InvalidLayout.into());
            }
            let document = storage(documents.get((cid, id)))?.ok_or(Error::InvalidLayout)?;
            if mutations::index_value(document.value(), &field)?.as_deref() != Some(value.as_str())
            {
                return Err(Error::InvalidLayout.into());
            }
            append(id, document.value())?;
            seen += 1;
        }
        if seen != expected {
            return Err(Error::InvalidLayout.into());
        }
        Plan::Secondary
    } else {
        let end = cid.checked_add(1).ok_or(Error::InvalidLayout)?;
        for row in storage(documents.range((cid, "")..(end, "")))? {
            let (key, value) = storage(row)?;
            append(key.value().1, value.value())?;
        }
        Plan::Scan
    };
    let examined = inputs.len();
    Ok(Output {
        documents: query.evaluate_documents(&inputs)?,
        plan,
        examined,
        document_bytes: bytes,
    })
}

mod tests;
