use std::fmt;

use serde_json::value::RawValue;

use crate::{DocumentError, DocumentInput};

/// Candidate for a transactional insert. Not persisted or uniqueness checked.
pub struct PreparedInsert {
    id: String,
    generated: bool,
    json: Vec<u8>,
}

impl PreparedInsert {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn has_generated_id(&self) -> bool {
        self.generated
    }

    pub fn json(&self) -> &[u8] {
        &self.json
    }
}

impl fmt::Debug for PreparedInsert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedInsert")
            .field("generated", &self.generated)
            .field("bytes", &self.json.len())
            .finish_non_exhaustive()
    }
}

impl DocumentInput {
    /// Assign a missing ID and enforce the original byte limit after encoding.
    /// Storage must still check uniqueness atomically with insertion.
    pub fn prepare_insert(self) -> Result<PreparedInsert, DocumentError> {
        self.prepare_with(|bytes| getrandom::fill(bytes).map_err(|_| ()))
    }

    // Private injection point for deterministic entropy and failure tests.
    fn prepare_with(
        mut self,
        fill: impl FnOnce(&mut [u8; 16]) -> Result<(), ()>,
    ) -> Result<PreparedInsert, DocumentError> {
        let generated = self.id.is_none();
        let id = match self.id.take() {
            Some(id) => id,
            None => {
                let mut bytes = [0u8; 16];
                fill(&mut bytes).map_err(|_| DocumentError::IdGenerationFailed)?;
                encode_id(bytes)
            }
        };
        let encoded_id =
            serde_json::to_string(&id).map_err(|_| DocumentError::DocumentEncodingFailed)?;
        let raw_id =
            RawValue::from_string(encoded_id).map_err(|_| DocumentError::DocumentEncodingFailed)?;
        self.fields.insert("_id".into(), raw_id);
        let json =
            serde_json::to_vec(&self.fields).map_err(|_| DocumentError::DocumentEncodingFailed)?;
        if json.len() > self.max_bytes {
            return Err(DocumentError::DocumentTooLarge);
        }
        Ok(PreparedInsert {
            id,
            generated,
            json,
        })
    }
}

fn encode_id(bytes: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut id = String::with_capacity(32);
    for byte in bytes {
        id.push(char::from(HEX[usize::from(byte >> 4)]));
        id.push(char::from(HEX[usize::from(byte & 0xf)]));
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_MAX_DOCUMENT_BYTES;

    #[test]
    fn deterministic_bytes_produce_the_expected_id_and_json() {
        let input = DocumentInput::parse(b"{}", DEFAULT_MAX_DOCUMENT_BYTES).unwrap();
        let prepared = input
            .prepare_with(|bytes| {
                *bytes = [
                    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76,
                    0x54, 0x32, 0x10,
                ];
                Ok(())
            })
            .unwrap();
        assert_eq!(prepared.id(), "0123456789abcdeffedcba9876543210");
        assert_eq!(
            prepared.json(),
            br#"{"_id":"0123456789abcdeffedcba9876543210"}"#
        );
        assert!(prepared.has_generated_id());
    }

    #[test]
    fn explicit_id_is_preserved_without_entropy() {
        let input =
            DocumentInput::parse(br#"{"_id":"Order_A"}"#, DEFAULT_MAX_DOCUMENT_BYTES).unwrap();
        let prepared = input
            .prepare_with(|_| panic!("explicit ID must not use entropy"))
            .unwrap();
        assert_eq!(prepared.id(), "Order_A");
        assert!(!prepared.has_generated_id());
        assert_eq!(prepared.json(), br#"{"_id":"Order_A"}"#);
    }

    #[test]
    fn entropy_failure_is_safe_and_does_not_fall_back() {
        let input = DocumentInput::parse(b"{}", DEFAULT_MAX_DOCUMENT_BYTES).unwrap();
        let error = input
            .prepare_with(|bytes| {
                bytes[0] = 0xff;
                Err(())
            })
            .unwrap_err();
        assert_eq!(error, DocumentError::IdGenerationFailed);
        assert_eq!(error.to_string(), "id_generation_failed");
    }

    #[test]
    fn final_size_limit_is_inclusive_and_cannot_be_bypassed() {
        // {"_id":"<32 hex digits>"} is 42 bytes.
        for limit in [2, 41, 42] {
            let input = DocumentInput::parse(b"{}", limit).unwrap();
            let result = input.prepare_with(|bytes| {
                bytes.fill(0);
                Ok(())
            });
            if limit == 42 {
                assert_eq!(result.unwrap().json().len(), 42);
            } else {
                assert_eq!(result.unwrap_err(), DocumentError::DocumentTooLarge);
            }
        }
    }

    #[test]
    fn raw_payloads_are_preserved_and_debug_is_redacted() {
        let input = DocumentInput::parse(
            br#"{"_id":"secret-id","secret-key":{"n":123456789012345678901234567890,"x":null}}"#,
            DEFAULT_MAX_DOCUMENT_BYTES,
        )
        .unwrap();
        let prepared = input.prepare_insert().unwrap();
        let fields: std::collections::BTreeMap<String, Box<RawValue>> =
            serde_json::from_slice(prepared.json()).unwrap();
        assert_eq!(
            fields["secret-key"].get(),
            r#"{"n":123456789012345678901234567890,"x":null}"#
        );
        let debug = format!("{prepared:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("1234567890"));
    }

    #[test]
    fn system_entropy_produces_a_valid_generated_id() {
        let prepared = DocumentInput::parse(b"{}", DEFAULT_MAX_DOCUMENT_BYTES)
            .unwrap()
            .prepare_insert()
            .unwrap();
        assert!(prepared.has_generated_id());
        assert_eq!(prepared.id().len(), 32);
        assert!(
            prepared
                .id()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        let parsed = DocumentInput::parse(prepared.json(), DEFAULT_MAX_DOCUMENT_BYTES).unwrap();
        assert_eq!(parsed.id(), Some(prepared.id()));
    }
}
