use super::*;
use crate::Store;

const V1: &[u8] = include_bytes!("../../tests/fixtures/format-v1.json");
const V2: &[u8] = include_bytes!("../../tests/fixtures/format-v2.json");

#[test]
fn golden_codecs_keep_legacy_bytes_and_explicit_version_identity() {
    for (version, fixture) in [(Version::Legacy, V1), (Version::Indexed, V2)] {
        assert_eq!(encode(version).unwrap(), fixture.trim_ascii_end());
        assert_eq!(decode(fixture).unwrap(), version);
        let mut padded = fixture.trim_ascii_end().to_vec();
        padded.resize(1024, b' ');
        assert_eq!(decode(&padded).unwrap(), version);
        padded.push(b' ');
        assert_eq!(decode(&padded), Err(Error::InvalidManifest));
    }
}

#[test]
fn malformed_unknown_and_incompatible_manifests_keep_distinct_errors() {
    for value in [
        b"".as_slice(),
        b"{}",
        b"{",
        br#"{"magic":"kiarni","version":-1,"engine":"redb-4.5.0"}"#,
        br#"{"magic":"kiarni","version":1.5,"engine":"redb-4.5.0"}"#,
    ] {
        assert_eq!(decode(value), Err(Error::InvalidManifest));
    }
    let text = std::str::from_utf8(V1).unwrap();
    for altered in [
        text.replace("\"version\":1", "\"version\":1,\"version\":1"),
        text.replace(
            "\"magic\":\"kiarni\"",
            "\"magic\":\"kiarni\",\"magic\":\"kiarni\"",
        ),
        text.replace(
            "\"engine\":\"redb-4.5.0\"",
            "\"engine\":\"redb-4.5.0\",\"extra\":true",
        ),
    ] {
        assert_eq!(decode(altered.as_bytes()), Err(Error::InvalidManifest));
    }
    for altered in [
        text.replace("\"version\":1", "\"version\":0"),
        text.replace("\"version\":1", "\"version\":3"),
        text.replace("\"version\":1", "\"version\":18446744073709551615"),
        text.replace("kiarni", "other"),
        text.replace("redb-4.5.0", "redb-other"),
    ] {
        assert_eq!(decode(altered.as_bytes()), Err(Error::UnsupportedFormat));
    }
}

#[test]
fn caller_policy_rejects_other_versions_before_touching_the_engine() {
    let temp = tempfile::tempdir().unwrap();
    for (version, fixture) in [(Version::Legacy, V1), (Version::Indexed, V2)] {
        let root = temp.path().join(format!("v{}", version.number()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("format.json"), fixture).unwrap();
        let other = if version == Version::Legacy {
            Version::Indexed
        } else {
            Version::Legacy
        };
        // Rejection precedes even checking for a data file.
        assert_eq!(preflight(&root, other), Err(Error::UnsupportedFormat));
        assert_eq!(preflight(&root, version), Err(Error::Io));
        fs::write(root.join("data.redb"), b"must not be opened").unwrap();
        assert_eq!(preflight(&root, version), Ok(()));
        assert_eq!(preflight(&root, other), Err(Error::UnsupportedFormat));
        if version == Version::Indexed {
            assert!(matches!(Store::open(&root), Err(Error::UnsupportedFormat)));
        }
        assert_eq!(
            fs::read(root.join("data.redb")).unwrap(),
            b"must not be opened"
        );
    }
}

#[test]
fn filesystem_shapes_and_manifest_limits_are_checked_without_repair() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("data.redb"), b"unchanged").unwrap();
    fs::create_dir(root.join("format.json")).unwrap();
    assert_eq!(
        preflight(&root, Version::Legacy),
        Err(Error::InvalidManifest)
    );
    fs::remove_dir(root.join("format.json")).unwrap();
    fs::write(root.join("format.json"), [b' '; 1025]).unwrap();
    assert_eq!(
        preflight(&root, Version::Legacy),
        Err(Error::InvalidManifest)
    );
    fs::write(root.join("format.json"), V1).unwrap();
    fs::remove_file(root.join("data.redb")).unwrap();
    fs::create_dir(root.join("data.redb")).unwrap();
    assert_eq!(preflight(&root, Version::Legacy), Err(Error::InvalidLayout));
    let file = temp.path().join("file");
    fs::write(&file, b"x").unwrap();
    assert_eq!(preflight(&file, Version::Legacy), Err(Error::InvalidLayout));
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        fs::remove_file(root.join("format.json")).unwrap();
        let external = temp.path().join("external.json");
        fs::write(&external, V1).unwrap();
        symlink(&external, root.join("format.json")).unwrap();
        assert_eq!(
            preflight(&root, Version::Legacy),
            Err(Error::InvalidManifest)
        );
        assert_eq!(fs::read(&external).unwrap(), V1);
    }
}

#[test]
fn actual_legacy_creation_and_reopen_use_the_shared_codec() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let store = Store::create(&root).unwrap();
    store.create_collection("items").unwrap();
    drop(store);
    assert_eq!(
        fs::read(root.join("format.json")).unwrap(),
        V1.trim_ascii_end()
    );
    let store = Store::open(&root).unwrap();
    assert_eq!(
        store.list_collections(10, None).unwrap().collections,
        vec!["items"]
    );
    assert_eq!(
        preflight(&root, Version::Indexed),
        Err(Error::UnsupportedFormat)
    );
}
