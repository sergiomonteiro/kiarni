//! Deliberately narrow HTTPS-origin grammar; no URI paths, credentials or IDNA.
use crate::Error;
use std::net::{Ipv4Addr, Ipv6Addr};
pub(crate) const MAX_BYTES: usize = 272;
pub(crate) fn parse(value: &str) -> Result<String, Error> {
    if value.len() > MAX_BYTES {
        return Err(Error::InvalidValue);
    }
    let authority = value.strip_prefix("https://").ok_or(Error::InvalidValue)?;
    let (host, suffix) = if let Some(rest) = authority.strip_prefix('[') {
        let end = rest.find(']').ok_or(Error::InvalidValue)?;
        let ip: Ipv6Addr = rest[..end].parse().map_err(|_| Error::InvalidValue)?;
        if ip.is_unspecified() || ip.is_multicast() {
            return Err(Error::InvalidValue);
        }
        let canonical = ip.to_string();
        // IPv4-mapped display syntax differs from browser IPv6 serialization.
        if canonical.contains('.') {
            return Err(Error::InvalidValue);
        }
        (format!("[{canonical}]"), &rest[end + 1..])
    } else {
        let end = authority.find(':').unwrap_or(authority.len());
        let host = &authority[..end];
        if host.is_empty() || host.len() > 253 || host.ends_with('.') || !host.is_ascii() {
            return Err(Error::InvalidValue);
        }
        let host = host.to_ascii_lowercase();
        for label in host.split('.') {
            if label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return Err(Error::InvalidValue);
            }
        }
        let host = if let Ok(ip) = host.parse::<Ipv4Addr>() {
            if ip.is_unspecified() || ip.is_multicast() {
                return Err(Error::InvalidValue);
            }
            ip.to_string()
        } else {
            let last = host.rsplit('.').next().ok_or(Error::InvalidValue)?;
            if last.bytes().all(|b| b.is_ascii_digit())
                || last
                    .strip_prefix("0x")
                    .is_some_and(|s| s.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(Error::InvalidValue);
            }
            host
        };
        (host, &authority[end..])
    };
    let port = if suffix.is_empty() {
        None
    } else {
        let port = suffix.strip_prefix(':').ok_or(Error::InvalidValue)?;
        if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::InvalidValue);
        }
        let port = port.parse::<u16>().map_err(|_| Error::InvalidValue)?;
        if port == 0 {
            return Err(Error::InvalidValue);
        }
        Some(port)
    };
    Ok(match port {
        None | Some(443) => format!("https://{host}"),
        Some(port) => format!("https://{host}:{port}"),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_https_origins_have_unambiguous_authorities() {
        for (input, expected) in [
            ("https://DB.Example.com:443", "https://db.example.com"),
            ("https://localhost:09443", "https://localhost:9443"),
            ("https://127.0.0.1:9443", "https://127.0.0.1:9443"),
            ("https://[0:0:0:0:0:0:0:1]:443", "https://[::1]"),
            (
                "https://xn--bcher-kva.example",
                "https://xn--bcher-kva.example",
            ),
        ] {
            assert_eq!(parse(input).unwrap(), expected);
        }
    }
    #[test]
    fn paths_credentials_ambiguous_ips_and_non_https_are_refused() {
        for input in [
            "",
            "http://db.example",
            "HTTPS://db.example",
            "https://",
            "https://db.example/",
            "https://db.example/a",
            "https://db.example?x=1",
            "https://db.example#x",
            "https://user:secret@db.example",
            "https://db.example:0",
            "https://db.example:65536",
            "https://db.example:",
            "https://db.example:1:2",
            "https://db.example.",
            "https://db..example",
            "https://-db.example",
            "https://db-.example",
            "https://db_example",
            "https://bücher.example",
            "https://db%2eexample",
            "https://db\\example",
            "https://db.example\n",
            "https://127.1",
            "https://0177.0.0.1",
            "https://2130706433",
            "https://0x7f000001",
            "https://0x",
            "https://db.0xff",
            "https://0.0.0.0",
            "https://224.0.0.1",
            "https://[::]",
            "https://[ff02::1]",
            "https://[::1%lo]",
            "https://[::ffff:127.0.0.1]",
            "https://[::ffff:7f00:1]",
            "https://::1",
            "https://[::1]other",
        ] {
            assert!(parse(input).is_err(), "accepted origin fixture {input:?}");
        }
        assert!(parse(&format!("https://{}.example", "x".repeat(64))).is_err());
        assert!(parse(&"x".repeat(MAX_BYTES + 1)).is_err());
    }
}
