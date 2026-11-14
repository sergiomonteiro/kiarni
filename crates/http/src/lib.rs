//! HTTP routes, request IDs and stable error responses.
mod catalog;
mod routes;
pub use catalog::Code;
pub use routes::{AllowedMethods, Operation, Rejection, Target, resolve};

/// Model application admission only; transport/header parsing remains outside.
pub fn authenticate_and_resolve<'a>(
    authority: &kiarni_auth::Authority,
    authorization: &[&[u8]],
    now: u64,
    method: &str,
    raw_path: &'a str,
) -> Result<(kiarni_auth::Session, Target<'a>), Rejection> {
    let session = authority
        .authenticate(authorization, now)
        .map_err(|error| Rejection::from(Code::from(error)))?;
    let target = resolve(method, raw_path)?;
    Ok((session, target))
}

use std::{collections::BTreeMap, fmt};

pub struct RequestId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseError {
    EntropyUnavailable,
    EncodingFailed,
    InvalidAllowContext,
}

impl RequestId {
    // No constructor from incoming headers: IDs are always generated locally.
    pub fn generate() -> Result<Self, ResponseError> {
        Self::generate_with(|bytes| getrandom::fill(bytes).map_err(|_| ()))
    }

    fn generate_with(
        fill: impl FnOnce(&mut [u8; 16]) -> Result<(), ()>,
    ) -> Result<Self, ResponseError> {
        let mut bytes = [0; 16];
        fill(&mut bytes).map_err(|_| ResponseError::EntropyUnavailable)?;
        Ok(Self::from_bytes(bytes))
    }

    fn from_bytes(bytes: [u8; 16]) -> Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut id = String::with_capacity(36);
        id.push_str("req_");
        for byte in bytes {
            id.push(char::from(HEX[usize::from(byte >> 4)]));
            id.push(char::from(HEX[usize::from(byte & 15)]));
        }
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<&'static str, String>,
    pub body: Vec<u8>,
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

pub fn error_response(
    code: Code,
    request_id: &RequestId,
    allow: Option<AllowedMethods>,
    head: bool,
) -> Result<Response, ResponseError> {
    if (code == Code::MethodNotAllowed) != allow.is_some() {
        return Err(ResponseError::InvalidAllowContext);
    }
    let (code, status, message) = code.spec();
    let body = serde_json::to_vec(&serde_json::json!({
        "error": {"code": code, "message": message}, "request_id": request_id.as_str(),
    }))
    .map_err(|_| ResponseError::EncodingFailed)?;
    let mut headers = BTreeMap::from([
        ("Content-Type", "application/json".into()),
        ("Cache-Control", "no-store".into()),
        ("X-Request-ID", request_id.as_str().into()),
        ("Content-Length", body.len().to_string()),
    ]);
    if let Some(allow) = allow {
        headers.insert("Allow", allow.as_str().into());
    }
    if status == 401 {
        headers.insert("WWW-Authenticate", "Bearer".into());
    }
    if status == 429 || status == 503 {
        headers.insert("Retry-After", "1".into());
    }
    Ok(Response {
        status,
        headers,
        body: if head { Vec::new() } else { body },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_request_id_is_server_controlled_and_header_safe() {
        assert_eq!(
            RequestId::from_bytes([0; 16]).as_str(),
            "req_00000000000000000000000000000000"
        );
        assert_eq!(
            RequestId::from_bytes([255; 16]).as_str(),
            "req_ffffffffffffffffffffffffffffffff"
        );
        let real = RequestId::generate().unwrap();
        assert_eq!(real.as_str().len(), 36);
        assert!(
            real.as_str()[4..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }

    #[test]
    fn entropy_failure_does_not_create_a_fallback_id() {
        assert!(matches!(
            RequestId::generate_with(|_| Err(())),
            Err(ResponseError::EntropyUnavailable)
        ));
    }
}
