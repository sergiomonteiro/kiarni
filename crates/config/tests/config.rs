use kiarni_config::{Admission, AdmissionError, Config, Error, resolve};
use serde_json::Value;

fn error(cli: &[(&str, &str)], env: &[(&str, &str)], file: Option<&[u8]>) -> Error {
    resolve(cli, env, file).unwrap_err()
}

#[test]
fn defaults_are_explicit_and_loopback_only() {
    let config = resolve(&[], &[], None).unwrap();
    assert_eq!(config.bind().to_string(), "127.0.0.1:8711");
    assert_eq!(config.data_dir(), "./kiarni-data");
    assert_eq!(config.max_document_bytes(), 1048576);
    assert_eq!(config.max_query_bytes(), 16384);
    assert_eq!(config.max_result_bytes(), 4194304);
    assert_eq!(config.feed_queue(), 4);
    assert_eq!(config.read_timeout_ms(), 5000);
    assert_eq!(config.write_timeout_ms(), 5000);
    assert_eq!(config.heartbeat_ms(), 15000);
    assert_eq!(config.frame_bytes(), 3 * 1048576);
    assert_eq!(config.buffer_plan_bytes().unwrap(), 353 * 1048576);
}

#[test]
fn precedence_fixtures_merge_per_field() {
    let fixtures: Vec<Value> = serde_json::from_str(include_str!("precedence.json")).unwrap();
    assert_eq!(fixtures.len(), 6);
    for fixture in fixtures {
        let pairs = |key: &str| {
            fixture[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|pair| (pair[0].as_str().unwrap(), pair[1].as_str().unwrap()))
                .collect::<Vec<_>>()
        };
        let file =
            (!fixture["file"].is_null()).then(|| serde_json::to_vec(&fixture["file"]).unwrap());
        let config = resolve(&pairs("cli"), &pairs("env"), file.as_deref()).unwrap();
        assert_eq!(config.max_requests(), fixture["requests"].as_u64().unwrap());
        assert_eq!(config.max_streams(), fixture["streams"].as_u64().unwrap());
        assert_eq!(config.data_dir(), fixture["dir"].as_str().unwrap());
    }
}

#[test]
fn invalid_shadowed_values_and_unknowns_fail_closed() {
    assert_eq!(
        error(
            &[("--max-requests", "16")],
            &[("KIARNI_MAX_REQUESTS", "bad")],
            None
        ),
        Error::InvalidValue
    );
    assert_eq!(
        error(
            &[("--max-requests", "16")],
            &[],
            Some(br#"{"max_requests":"bad"}"#)
        ),
        Error::InvalidValue
    );
    for key in ["KIARNI_TOKEN", "KIARNI_UNKNOWN", "KIARNI_max_requests"] {
        assert_eq!(error(&[], &[(key, "secret")], None), Error::UnknownSetting);
    }
    for key in [
        "--unknown",
        "--max_requests",
        "max-requests",
        "--MAX-REQUESTS",
    ] {
        assert_eq!(error(&[(key, "secret")], &[], None), Error::UnknownSetting);
    }
    assert_eq!(
        error(&[], &[], Some(br#"{"token":"secret"}"#)),
        Error::UnknownSetting
    );
}

#[test]
fn rejects_duplicate_keys_in_each_source_including_escaped_json() {
    assert_eq!(
        error(
            &[("--max-requests", "16"), ("--max-requests", "24")],
            &[],
            None
        ),
        Error::DuplicateSetting
    );
    assert_eq!(
        error(
            &[],
            &[("KIARNI_MAX_REQUESTS", "16"), ("KIARNI_MAX_REQUESTS", "24")],
            None
        ),
        Error::DuplicateSetting
    );
    assert_eq!(
        error(&[], &[], Some(br#"{"data_dir":"a","data_\u0064ir":"b"}"#)),
        Error::InvalidFile
    );
}

#[test]
fn numeric_syntax_and_bounds_are_strict() {
    for value in [
        "",
        "0",
        "-1",
        "+1",
        "1.0",
        "1e1",
        " 16",
        "16 ",
        "1MiB",
        "18446744073709551616",
    ] {
        assert_eq!(
            error(&[("--max-requests", value)], &[], None),
            Error::InvalidValue
        );
    }
    for (key, invalid) in [
        ("--max-document-bytes", "1048577"),
        ("--max-query-bytes", "16385"),
        ("--max-result-bytes", "4194305"),
        ("--max-requests", "129"),
        ("--max-streams", "33"),
        ("--feed-queue", "17"),
        ("--read-timeout-ms", "99"),
        ("--write-timeout-ms", "30001"),
        ("--heartbeat-ms", "999"),
    ] {
        assert_eq!(error(&[(key, invalid)], &[], None), Error::InvalidValue);
    }
    for json in [
        br#"{"max_requests":1.0}"#.as_slice(),
        br#"{"max_requests":1e1}"#,
        br#"{"max_requests":true}"#,
        br#"{"data_dir":null}"#,
    ] {
        assert_eq!(error(&[], &[], Some(json)), Error::InvalidValue);
    }
}

#[test]
fn final_relations_and_buffer_plan_refuse_invalid_combinations() {
    assert_eq!(
        error(&[("--max-requests", "2")], &[], None),
        Error::InvalidCombination
    );
    assert_eq!(
        error(&[("--max-result-bytes", "1048576")], &[], None),
        Error::InvalidCombination
    );
    assert_eq!(
        error(&[("--max-requests", "128")], &[], None),
        Error::BufferPlanExceeded
    );
    let config = resolve(
        &[
            ("--max-requests", "2"),
            ("--max-streams", "1"),
            ("--max-document-bytes", "1024"),
            ("--max-result-bytes", "2048"),
        ],
        &[],
        None,
    )
    .unwrap();
    assert_eq!(config.max_document_bytes(), 1024);
    assert_eq!(config.max_result_bytes(), 2048);
    // Relationships apply after merge, so related fields may span sources.
    assert!(
        resolve(
            &[("--max-streams", "1")],
            &[],
            Some(br#"{"max_requests":2}"#)
        )
        .is_ok()
    );
}

#[test]
fn remote_bind_and_bad_paths_are_rejected_without_io_or_expansion() {
    for bind in [
        "0.0.0.0:8711",
        "192.0.2.1:8711",
        "[::]:8711",
        "localhost:8711",
        "127.0.0.1:0",
    ] {
        assert_eq!(error(&[("--bind", bind)], &[], None), Error::InvalidValue);
    }
    assert!(resolve(&[("--bind", "[::1]:8711")], &[], None).is_ok());
    for path in ["", "a\nb", "a\0b"] {
        assert_eq!(
            error(&[("--data-dir", path)], &[], None),
            Error::InvalidValue
        );
    }
    let config = resolve(&[("--data-dir", "~/literal/$DATA")], &[], None).unwrap();
    assert_eq!(config.data_dir(), "~/literal/$DATA");
    assert!(!format!("{config:?}").contains("literal"));
    assert!(
        !error(&[("--unknown", "secret")], &[], None)
            .to_string()
            .contains("secret")
    );
}

#[test]
fn source_sizes_and_malformed_files_are_bounded() {
    assert_eq!(
        error(&vec![("--max-requests", "16"); 33], &[], None),
        Error::SourceTooLarge
    );
    assert_eq!(
        error(&[("--data-dir", &"x".repeat(4097))], &[], None),
        Error::SourceTooLarge
    );
    assert_eq!(
        error(&[], &[("KIARNI_DATA_DIR", &"x".repeat(4097))], None),
        Error::SourceTooLarge
    );
    assert_eq!(
        error(&[], &[], Some(&vec![b' '; 16385])),
        Error::InvalidFile
    );
    for file in [b"[]".as_slice(), b"null", b"{", b"{} trailing"] {
        assert_eq!(error(&[], &[], Some(file)), Error::InvalidFile);
    }
}

fn small() -> Config {
    resolve(
        &[("--max-requests", "2"), ("--max-streams", "1")],
        &[],
        None,
    )
    .unwrap()
}

#[test]
fn streams_consume_both_limits_and_drop_restores_capacity() {
    let admission = Admission::new(&small());
    let stream = admission.acquire(true).unwrap();
    assert!(matches!(
        admission.acquire(true),
        Err(AdmissionError::StreamLimit)
    ));
    let ordinary = admission.acquire(false).unwrap();
    assert!(matches!(
        admission.acquire(false),
        Err(AdmissionError::Busy)
    ));
    drop(stream);
    let stream = admission.acquire(true).unwrap();
    drop((stream, ordinary));
    assert!(admission.acquire(true).is_ok());
}

#[test]
fn concurrent_admission_shares_one_bound_and_releases_after_cancellation() {
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };
    let admission = Admission::new(&small());
    let entered = AtomicUsize::new(0);
    let attempted = Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let permit = admission.acquire(false);
                if permit.is_ok() {
                    entered.fetch_add(1, Ordering::Relaxed);
                }
                attempted.wait(); // Hold all successes until every contender tried.
                drop(permit);
            });
        }
    });
    assert_eq!(entered.load(Ordering::Relaxed), 2);
    let one = admission.acquire(false).unwrap();
    let two = admission.clone().acquire(false).unwrap();
    assert!(matches!(
        admission.acquire(false),
        Err(AdmissionError::Busy)
    ));
    drop((one, two));
    assert!(admission.acquire(true).is_ok());
}

#[test]
fn stream_promotion_keeps_general_slot_and_failed_promotion_retains_permit() {
    let admission = Admission::new(&small());
    let mut one = admission.acquire(false).unwrap();
    let mut two = admission.acquire(false).unwrap();
    one.promote_to_stream().unwrap();
    one.promote_to_stream().unwrap(); // Idempotent, does not double charge.
    assert_eq!(two.promote_to_stream(), Err(AdmissionError::StreamLimit));
    assert!(matches!(
        admission.acquire(false),
        Err(AdmissionError::Busy)
    ));
    drop(one);
    two.promote_to_stream().unwrap();
    assert!(matches!(
        admission.acquire(true),
        Err(AdmissionError::StreamLimit)
    ));
    drop(two);
    assert!(admission.acquire(true).is_ok());
}

#[test]
fn admission_snapshot_tracks_promotion_and_shared_permit_retention() {
    use std::sync::Arc;
    let admission = Admission::new(&small());
    let snapshot = admission.snapshot();
    assert_eq!(
        (
            snapshot.admitted,
            snapshot.streaming,
            snapshot.limit,
            snapshot.stream_limit
        ),
        (0, 0, 2, 1)
    );
    let mut permit = admission.acquire(false).unwrap();
    permit.promote_to_stream().unwrap();
    permit.promote_to_stream().unwrap();
    let permit = Arc::new(permit);
    let retained = Arc::clone(&permit);
    let ordinary = admission.acquire(false).unwrap();
    assert_eq!(
        (
            admission.snapshot().admitted,
            admission.snapshot().streaming
        ),
        (2, 1)
    );
    drop(permit);
    assert_eq!(admission.snapshot().streaming, 1);
    drop(retained);
    assert_eq!(
        (
            admission.snapshot().admitted,
            admission.snapshot().streaming
        ),
        (1, 0)
    );
    drop(ordinary);
    assert_eq!(admission.snapshot(), snapshot);
}

#[test]
fn log_levels_are_strict_and_follow_source_precedence() {
    use kiarni_config::LogLevel;
    assert_eq!(resolve(&[], &[], None).unwrap().log_level(), LogLevel::Info);
    assert_eq!(
        resolve(&[], &[], Some(br#"{"log_level":"warn"}"#))
            .unwrap()
            .log_level(),
        LogLevel::Warn
    );
    assert_eq!(
        resolve(
            &[],
            &[("KIARNI_LOG_LEVEL", "error")],
            Some(br#"{"log_level":"warn"}"#)
        )
        .unwrap()
        .log_level(),
        LogLevel::Error
    );
    assert_eq!(
        resolve(
            &[("--log-level", "off")],
            &[("KIARNI_LOG_LEVEL", "error")],
            Some(br#"{"log_level":"warn"}"#)
        )
        .unwrap()
        .log_level(),
        LogLevel::Off
    );
    for level in ["debug", "INFO", "", "secret"] {
        assert_eq!(
            error(&[("--log-level", level)], &[], None),
            Error::InvalidValue
        );
        assert_eq!(
            error(
                &[("--log-level", "info")],
                &[("KIARNI_LOG_LEVEL", level)],
                None
            ),
            Error::InvalidValue
        );
    }
    assert_eq!(
        error(&[], &[], Some(br#"{"log_level":1}"#)),
        Error::InvalidValue
    );
}
