#![cfg(unix)]
use kiarni_auth::{Authority, Keyset, vault::Vault};
use kiarni_http::{Code, authenticate_and_resolve};
use std::{fs, os::unix::fs::PermissionsExt};
use zeroize::Zeroizing;

#[test]
fn authentication_precedes_route_exposure_and_revocation_uses_the_same_401() {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let client = tempfile::tempdir().unwrap();
    fs::set_permissions(client.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let delivery = Vault::open(client.path()).unwrap();
    let (keyset, token) = Keyset::bootstrap(100, 1000).unwrap();
    let snapshot = Vault::open(dir.path())
        .unwrap()
        .provision(&keyset, &token, &delivery)
        .unwrap();
    let bytes = Zeroizing::new(fs::read(client.path().join(snapshot.token_file.unwrap())).unwrap());
    let mut header = Zeroizing::new(b"Bearer ".to_vec());
    header.extend_from_slice(&bytes);
    let authority = Authority::new(keyset.clone(), 100);
    for path in [
        "/v1/collections",
        "/v1/health/live",
        "/v1/unknown",
        "/invalid//path",
    ] {
        let result = authenticate_and_resolve(&authority, &[], 100, "GET", path);
        assert_eq!(result.err().unwrap().code, Code::Unauthorized);
    }
    assert!(
        authenticate_and_resolve(&authority, &[&header], 100, "GET", "/v1/collections").is_ok()
    );
    assert_eq!(
        authenticate_and_resolve(&authority, &[&header], 100, "GET", "/v1/unknown")
            .err()
            .unwrap()
            .code,
        Code::RouteNotFound
    );
    authority
        .reload(Ok(keyset.revoke_all().unwrap()), 101)
        .unwrap();
    assert_eq!(
        authenticate_and_resolve(&authority, &[&header], 101, "GET", "/v1/collections")
            .err()
            .unwrap()
            .code,
        Code::Unauthorized
    );
    let _ = authority.reload(Err(kiarni_auth::Error::InvalidConfig), 102);
    assert_eq!(
        authenticate_and_resolve(&authority, &[], 102, "GET", "/v1/unknown")
            .err()
            .unwrap()
            .code,
        Code::ServiceUnavailable
    );
}
