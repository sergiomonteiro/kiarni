//! Public, data-free login shell. All administrative reads use authenticated /v1.
use kiarni_http::{AllowedMethods, Code, Rejection, RequestId, Response};
use std::collections::BTreeMap;

pub fn asset(path: &str) -> Option<(&'static str, &'static [u8])> {
    Some(match path {
        "/" => (
            "text/html; charset=utf-8",
            include_bytes!("../dashboard/index.html"),
        ),
        "/dashboard/app.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../dashboard/app.js"),
        ),
        "/dashboard/metrics.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../dashboard/metrics.mjs"),
        ),
        "/dashboard/feed.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../dashboard/feed.mjs"),
        ),
        "/dashboard/style.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../dashboard/style.css"),
        ),
        _ => return None,
    })
}

pub fn response(method: &str, path: &str, id: &RequestId) -> Result<Response, Rejection> {
    let (content_type, bytes) = asset(path).ok_or(Code::RouteNotFound)?;
    if !matches!(method, "GET" | "HEAD") {
        return Err(Rejection {
            code: Code::MethodNotAllowed,
            allow: Some(AllowedMethods::Health),
        });
    }
    Ok(Response {
        status: 200,
        headers: BTreeMap::from([
            ("Content-Type", content_type.into()),
            ("Content-Length", bytes.len().to_string()),
            ("Cache-Control", "no-store".into()),
            ("X-Request-ID", id.as_str().into()),
            ("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'".into()),
            ("X-Content-Type-Options", "nosniff".into()),
            ("X-Frame-Options", "DENY".into()),
            ("Referrer-Policy", "no-referrer".into()),
        ]),
        body: if method == "HEAD" { Vec::new() } else { bytes.to_vec() },
    })
}
