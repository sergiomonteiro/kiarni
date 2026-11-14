use kiarni_http::{AllowedMethods, Code, RequestId, ResponseError, error_response, resolve};
use serde_json::Value;

#[test]
fn published_route_fixtures_distinguish_404_405_and_invalid_paths() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/routes.json")).unwrap();
    for case in cases {
        let method = case["method"].as_str().unwrap();
        let path = case["path"].as_str().unwrap();
        match resolve(method, path) {
            Ok(target) => {
                assert_eq!(
                    format!("{:?}", target.operation()),
                    case["operation"].as_str().unwrap(),
                    "{method} {path}"
                );
                assert_eq!(target.collection(), case["collection"].as_str());
                assert_eq!(target.id(), case["id"].as_str());
                assert_eq!(target.is_head(), case["head"].as_bool().unwrap_or(false));
            }
            Err(rejection) => {
                assert_eq!(
                    rejection.code.spec().0,
                    case["error"].as_str().unwrap(),
                    "{method} {path}"
                );
                assert_eq!(
                    u64::from(rejection.code.spec().1),
                    case["status"].as_u64().unwrap()
                );
                assert_eq!(
                    rejection.allow.map(AllowedMethods::as_str),
                    case["allow"].as_str()
                );
                let response = error_response(
                    rejection.code,
                    &RequestId::generate().unwrap(),
                    rejection.allow,
                    method == "HEAD",
                )
                .unwrap();
                if method == "HEAD" {
                    assert!(response.body.is_empty());
                }
            }
        }
    }
}

#[test]
fn body_contract_distinguishes_replace_patch_query_and_media_types() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/bodies.json")).unwrap();
    for case in cases {
        let target = resolve(
            case["method"].as_str().unwrap(),
            case["path"].as_str().unwrap(),
        )
        .unwrap();
        let result = target.validate_body(
            case["type"].as_str(),
            case["body"].as_str().unwrap().as_bytes(),
        );
        assert_eq!(
            result.err().map(|code| code.spec().0),
            case["error"].as_str(),
            "{case}"
        );
    }
}

#[test]
fn every_catalog_code_has_a_fixed_status_safe_envelope_and_required_headers() {
    let published: Value = serde_json::from_str(include_str!("fixtures/errors.json")).unwrap();
    assert_eq!(published.as_object().unwrap().len(), Code::ALL.len());
    let id = RequestId::generate().unwrap();
    for &code in Code::ALL {
        let (name, status, message) = code.spec();
        assert_eq!(published[name], status, "{name}");
        let allow = (code == Code::MethodNotAllowed).then_some(AllowedMethods::Document);
        let response = error_response(code, &id, allow, false).unwrap();
        assert_eq!(response.status, status);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"error":{"code":name,"message":message},"request_id":id.as_str()})
        );
        assert_eq!(response.headers["X-Request-ID"], id.as_str());
        assert_eq!(
            response.headers["Content-Length"],
            response.body.len().to_string()
        );
        assert_eq!(response.headers["Content-Type"], "application/json");
        assert_eq!(response.headers["Cache-Control"], "no-store");
        assert_eq!(
            response.headers.contains_key("WWW-Authenticate"),
            status == 401
        );
        assert_eq!(response.headers.contains_key("Allow"), status == 405);
        assert_eq!(
            response.headers.contains_key("Retry-After"),
            status == 429 || status == 503
        );
        assert!(message.is_ascii());
        assert!(!message.contains('\n'));
    }
}

#[test]
fn duplicate_id_envelope_is_pinned_independently_of_the_catalog() {
    let id = RequestId::generate().unwrap();
    let response = error_response(Code::DuplicateId, &id, None, false).unwrap();
    let mut actual: Value = serde_json::from_slice(&response.body).unwrap();
    actual["request_id"] = "SERVER_GENERATED".into();
    let expected: Value = serde_json::from_str(include_str!("fixtures/duplicate-id.json")).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn head_error_retains_representation_headers_without_sending_a_body() {
    let id = RequestId::generate().unwrap();
    let get = error_response(Code::DocumentNotFound, &id, None, false).unwrap();
    let head = error_response(Code::DocumentNotFound, &id, None, true).unwrap();
    assert_eq!(head.headers, get.headers);
    assert_eq!(head.status, get.status);
    assert!(head.body.is_empty());
    assert!(!get.body.is_empty());
    assert!(matches!(
        error_response(Code::MethodNotAllowed, &id, None, false),
        Err(ResponseError::InvalidAllowContext)
    ));
    assert!(matches!(
        error_response(
            Code::InternalError,
            &id,
            Some(AllowedMethods::Document),
            false
        ),
        Err(ResponseError::InvalidAllowContext)
    ));
}

#[test]
fn body_limits_are_inclusive_and_checked_before_media_type() {
    for (method, path, body, media, expected) in [
        (
            "POST",
            "/v1/collections",
            r#"{"name":"orders"}"#,
            "application/json",
            Code::RequestTooLarge,
        ),
        (
            "POST",
            "/v1/collections/orders/query",
            r#"{"from":"orders","limit":1}"#,
            "application/json",
            Code::QueryTooLarge,
        ),
        (
            "POST",
            "/v1/collections/orders/documents",
            "{}",
            "application/json",
            Code::DocumentTooLarge,
        ),
        (
            "PUT",
            "/v1/collections/orders/documents/a",
            "{}",
            "application/json",
            Code::DocumentTooLarge,
        ),
        (
            "PATCH",
            "/v1/collections/orders/documents/a",
            "{}",
            "application/merge-patch+json",
            Code::DocumentTooLarge,
        ),
    ] {
        let target = resolve(method, path).unwrap();
        let mut input = body.as_bytes().to_vec();
        input.resize(target.body_limit(), b' ');
        assert_eq!(target.validate_body(Some(media), &input), Ok(()));
        input.push(b' ');
        assert_eq!(
            target.validate_body(Some("text/plain"), &input),
            Err(expected)
        );
    }
}

#[test]
fn untrusted_metadata_is_not_echoed_in_errors_or_debug_output() {
    let path = "/v1/collections/secret-collection/documents/secret-id";
    let target = resolve("PUT", path).unwrap();
    assert_eq!(
        format!("{target:?}"),
        "Target { operation: ReplaceDocument, .. }"
    );
    let code = target
        .validate_body(
            Some("application/json"),
            br#"{"_id":"secret-other-id","secret-value":"secret-password"}"#,
        )
        .unwrap_err();
    let response = error_response(code, &RequestId::generate().unwrap(), None, false).unwrap();
    assert!(
        !std::str::from_utf8(&response.body)
            .unwrap()
            .contains("secret")
    );
    assert_eq!(format!("{response:?}"), "Response { status: 422, .. }");
}

#[test]
fn canonical_paths_and_identifier_boundaries_are_enforced() {
    assert_eq!(
        resolve("GET", "/unknown path").unwrap_err().code,
        Code::InvalidPath
    );
    for path in [
        "v1/collections",
        "/v1/collections?token=secret",
        "/v1/collections#fragment",
        "/v1/collections\\orders",
        "/v1/collections\r\nheader:secret",
    ] {
        assert_eq!(resolve("GET", path).unwrap_err().code, Code::InvalidPath);
    }
    for length in [128, 129] {
        let name = "a".repeat(length);
        let path = format!("/v1/collections/{name}/documents/{name}");
        assert_eq!(resolve("GET", &path).is_ok(), length == 128);
    }
    assert_eq!(
        resolve("GET", &format!("/{}", "a".repeat(2048)))
            .unwrap_err()
            .code,
        Code::InvalidPath
    );
}
