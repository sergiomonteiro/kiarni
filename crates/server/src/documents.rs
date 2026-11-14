use kiarni_core::{DocumentError, DocumentInput, PreparedInsert};
use kiarni_http::Code;
use kiarni_storage::{Collection, Error, Store};

pub fn insert(
    store: &Store,
    collection: &Collection,
    body: &[u8],
    limit: usize,
) -> Result<PreparedInsert, Code> {
    let generated = DocumentInput::parse(body, limit)
        .map_err(Code::from)?
        .id()
        .is_none();
    insert_with(store, collection, generated, || {
        DocumentInput::parse(body, limit)?.prepare_insert()
    })
}

fn insert_with(
    store: &Store,
    collection: &Collection,
    generated: bool,
    mut prepare: impl FnMut() -> Result<PreparedInsert, DocumentError>,
) -> Result<PreparedInsert, Code> {
    for _ in 0..8 {
        let candidate = prepare().map_err(Code::from)?;
        match store.insert_document(collection, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(Error::DuplicateId) if generated => {}
            Err(error) => return Err(super::collections::storage_error(error)),
        }
    }
    Err(Code::IdGenerationExhausted)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_generated_duplicates_retry_and_attempts_are_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::create(&temp.path().join("store")).unwrap();
        let collection = store.create_collection("orders").unwrap();
        let collision = || DocumentInput::parse(br#"{"_id":"taken"}"#, 1024)?.prepare_insert();
        store
            .insert_document(&collection, &collision().unwrap())
            .unwrap();
        for generated in [true, false] {
            let mut attempts = 0;
            let result = insert_with(&store, &collection, generated, || {
                attempts += 1;
                collision()
            });
            assert_eq!(
                result.unwrap_err(),
                if generated {
                    Code::IdGenerationExhausted
                } else {
                    Code::DuplicateId
                }
            );
            assert_eq!(attempts, if generated { 8 } else { 1 });
        }
        let mut attempts = 0;
        let candidate = insert_with(&store, &collection, true, || {
            attempts += 1;
            let bytes = if attempts == 1 {
                br#"{"_id":"taken"}"#.as_slice()
            } else {
                br#"{"_id":"free"}"#
            };
            DocumentInput::parse(bytes, 1024)?.prepare_insert()
        })
        .unwrap();
        assert_eq!(candidate.id(), "free");
        assert_eq!(attempts, 2);
        assert_eq!(
            store.get_document(&collection, "taken").unwrap().unwrap(),
            br#"{"_id":"taken"}"#
        );
        assert_eq!(
            insert_with(&store, &collection, true, || Err(
                DocumentError::IdGenerationFailed
            ))
            .unwrap_err(),
            Code::IdGenerationFailed
        );
    }
}
