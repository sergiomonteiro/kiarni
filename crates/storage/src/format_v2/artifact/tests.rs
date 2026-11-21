use super::super::backup::tests::{equal, fixture as base_fixture};
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

pub(super) fn fixture(large: bool) -> (tempfile::TempDir, Database, Collection) {
    let value = base_fixture(large);
    fs::set_permissions(value.0.path(), fs::Permissions::from_mode(0o700)).unwrap();
    value
}

fn manifest(path: &Path) -> Manifest {
    decode(&fs::read(path.join("backup.json")).unwrap())
        .unwrap()
        .0
}
fn save(path: &Path, value: &Manifest) {
    fs::write(path.join("backup.json"), serde_json::to_vec(value).unwrap()).unwrap();
}

#[test]
fn artifact_roundtrip_preserves_snapshot_hashes_and_private_files() {
    let (temp, db, _) = fixture(true);
    let read = db.begin_read().unwrap();
    let output = temp.path().join("artifact");
    let summary = backup(&read, &output, |_| Ok(())).unwrap();
    let root = directory(&output).unwrap();
    entries(&root, &["backup.json", "store"]).unwrap();
    let store = child(&root, "store", true).unwrap();
    let m = manifest(&output);
    for (record, name) in m.files.iter().zip(["format.json", "data.redb"]) {
        assert_eq!(
            hash(&mut child(&store, name, false).unwrap()).unwrap(),
            (decimal(&record.bytes).unwrap(), record.sha256.clone())
        );
    }
    let before = fs::read(output.join("store/data.redb")).unwrap();
    let restored = temp.path().join("restored");
    assert_eq!(
        restore_store(&output, &restored, |_| Ok(())).unwrap(),
        summary
    );
    equal(&read, &open(&restored).unwrap().begin_read().unwrap());
    assert_eq!(fs::read(output.join("store/data.redb")).unwrap(), before);
    private(&directory(&restored).unwrap(), true).unwrap();
    assert!(!restored.join("auth").exists());
    let empty = create(&temp.path().join("empty-store")).unwrap();
    let empty_backup = temp.path().join("empty-backup");
    let summary = backup(&empty.begin_read().unwrap(), &empty_backup, |_| Ok(())).unwrap();
    assert_eq!(
        summary,
        ExportSummary {
            collections: 0,
            documents: 0,
            document_bytes: 0
        }
    );
    assert_eq!(
        restore_store(
            &empty_backup,
            &temp.path().join("empty-restore"),
            |_| Ok(())
        )
        .unwrap(),
        summary
    );
}

#[test]
fn strict_manifest_fixture_and_sha256_known_answer() {
    let fixture = include_bytes!("../../../tests/fixtures/backup-format2.json");
    let (m, summary) = decode(fixture).unwrap();
    assert_eq!(summary.documents, 0);
    assert_eq!(serde_json::to_vec(&m).unwrap(), fixture.trim_ascii_end());
    for case in 0..12 {
        let mut value: serde_json::Value = serde_json::from_slice(fixture).unwrap();
        match case {
            0 => value["format_version"] = 1.into(),
            1 => value["version"] = 2.into(),
            2 => value["extra"] = true.into(),
            3 => value["files"][0]["path"] = "../data.redb".into(),
            4 => value["files"][0]["sha256"] = "A".repeat(64).into(),
            5 => value["files"][0]["bytes"] = "-1".into(),
            6 => value["collections"] = "18446744073709551616".into(),
            7 => value["documents"] = 0.into(),
            8 => value["engine"] = "redb-other".into(),
            9 => value["files"][0]["extra"] = true.into(),
            10 => value["files"].as_array_mut().unwrap().reverse(),
            11 => {
                value["files"].as_array_mut().unwrap().pop();
            }
            _ => unreachable!(),
        }
        assert!(
            decode(&serde_json::to_vec(&value).unwrap()).is_err(),
            "case {case}"
        );
    }
    assert!(decode(br#"{"magic":"kiarni-backup","magic":"kiarni-backup"}"#).is_err());
    let duplicate = std::str::from_utf8(fixture)
        .unwrap()
        .replace("\"version\":1", "\"version\":1,\"version\":1");
    assert!(decode(duplicate.as_bytes()).is_err());
    let duplicate = std::str::from_utf8(fixture).unwrap().replace(
        "\"path\":\"store/data.redb\"",
        "\"path\":\"store/data.redb\",\"path\":\"store/data.redb\"",
    );
    assert!(decode(duplicate.as_bytes()).is_err());
    assert!(decode(&[b' '; 1025]).is_err());
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(b"abc").unwrap();
    assert_eq!(
        hash(&mut file).unwrap(),
        (
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()
        )
    );
}

#[test]
fn hash_and_logical_integrity_are_both_required() {
    for case in 0..4 {
        let (temp, db, _) = fixture(false);
        let source = temp.path().join("artifact");
        backup(&db.begin_read().unwrap(), &source, |_| Ok(())).unwrap();
        if case == 0 {
            let path = source.join("store/data.redb");
            let mut bytes = fs::read(&path).unwrap();
            let last = bytes.len() - 1;
            bytes[last] ^= 1;
            fs::write(path, bytes).unwrap();
        } else if case == 3 {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(source.join("store/data.redb"))
                .unwrap();
            file.write_all(b"changed").unwrap();
        } else if case == 1 {
            let store = open(&source.join("store")).unwrap();
            let write = store.begin_write().unwrap();
            write
                .open_table(ENTRIES)
                .unwrap()
                .remove((1, "old", "d000"))
                .unwrap();
            write.commit().unwrap();
            drop(store);
            let mut m = manifest(&source);
            let (bytes, sha256) =
                hash(&mut File::open(source.join("store/data.redb")).unwrap()).unwrap();
            m.files[1].bytes = bytes.to_string();
            m.files[1].sha256 = sha256;
            save(&source, &m);
        } else {
            let mut m = manifest(&source);
            m.documents = "999".into();
            save(&source, &m);
        }
        let before = fs::read(source.join("store/data.redb")).unwrap();
        let target = temp.path().join("target");
        assert_eq!(
            restore_store(&source, &target, |_| Ok(())),
            Err(if case == 1 {
                Fault::Storage
            } else {
                Fault::Invalid
            })
        );
        assert!(!target.exists());
        assert_eq!(fs::read(source.join("store/data.redb")).unwrap(), before);
    }
}

#[test]
fn unsafe_shapes_permissions_and_extra_entries_are_rejected() {
    for case in 0..10 {
        let (temp, db, _) = fixture(false);
        let source = temp.path().join("artifact");
        backup(&db.begin_read().unwrap(), &source, |_| Ok(())).unwrap();
        let mut argument = source.clone();
        match case {
            0 => {
                argument = temp.path().join("alias");
                symlink(&source, &argument).unwrap();
            }
            1 => {
                let external = temp.path().join("external");
                fs::rename(source.join("store"), &external).unwrap();
                symlink(external, source.join("store")).unwrap();
            }
            2 | 3 => {
                let external = temp.path().join("external");
                fs::rename(source.join("store/data.redb"), &external).unwrap();
                if case == 2 {
                    symlink(external, source.join("store/data.redb")).unwrap();
                } else {
                    fs::hard_link(external, source.join("store/data.redb")).unwrap();
                }
            }
            4 => fs::set_permissions(
                source.join("store/data.redb"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap(),
            5 => fs::set_permissions(source.join("store"), fs::Permissions::from_mode(0o755))
                .unwrap(),
            6 => fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap(),
            7 => fs::write(source.join("extra"), b"no").unwrap(),
            8 => fs::write(source.join("store/extra"), b"no").unwrap(),
            9 => {
                let external = temp.path().join("manifest");
                fs::rename(source.join("backup.json"), &external).unwrap();
                symlink(external, source.join("backup.json")).unwrap();
            }
            _ => unreachable!(),
        }
        let target = temp.path().join("target");
        assert!(
            restore_store(&argument, &target, |_| Ok(())).is_err(),
            "case {case}"
        );
        assert!(!target.exists());
    }
}

#[test]
fn publication_never_replaces_a_destination_and_reports_uncertainty() {
    for restoring in [false, true] {
        let (temp, db, _) = fixture(false);
        let read = db.begin_read().unwrap();
        let source = temp.path().join("artifact");
        backup(&read, &source, |_| Ok(())).unwrap();
        for failed in [
            Point::Copied,
            Point::Verified,
            Point::BeforePublish,
            Point::Published,
        ] {
            let target = temp.path().join(format!("target-{failed:?}"));
            let callback = |p| if p == failed { Err(Fault::Io) } else { Ok(()) };
            let result = if restoring {
                restore_store(&source, &target, callback)
            } else {
                backup(&read, &target, callback)
            };
            assert_eq!(
                result,
                Err(if failed == Point::Published {
                    Fault::PublicationUncertain
                } else {
                    Fault::Io
                })
            );
            assert_eq!(target.exists(), failed == Point::Published);
        }
        let target = temp.path().join("raced");
        let callback = |p| {
            if p == Point::BeforePublish {
                fs::create_dir(&target).unwrap();
                fs::write(target.join("sentinel"), b"untouched").unwrap();
            }
            Ok(())
        };
        let result = if restoring {
            restore_store(&source, &target, callback)
        } else {
            backup(&read, &target, callback)
        };
        assert!(result.is_err());
        assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"untouched");
        assert!(restore_store(&source, &source.join("nested"), |_| Ok(())).is_err());
        let unsafe_parent = temp.path().join("unsafe-parent");
        fs::create_dir(&unsafe_parent).unwrap();
        fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o775)).unwrap();
        let output = unsafe_parent.join("out");
        assert_eq!(backup(&read, &output, |_| Ok(())), Err(Fault::Invalid));
        assert_eq!(
            restore_store(&source, &output, |_| Ok(())),
            Err(Fault::Invalid)
        );
        assert!(!output.exists());
    }
}

#[test]
#[ignore = "subprocess fixture for candidate artifact publication"]
fn crash_worker() {
    use std::io::{Read, Write};
    let source = std::env::var_os("KIARNI_V2_ARTIFACT_SOURCE").unwrap();
    let target = std::env::var_os("KIARNI_V2_ARTIFACT_TARGET").unwrap();
    let mode = std::env::var("KIARNI_V2_ARTIFACT_MODE").unwrap();
    let requested = std::env::var("KIARNI_V2_ARTIFACT_POINT").unwrap();
    let callback = |point| {
        if format!("{point:?}") == requested {
            println!("ARTIFACT_CHECKPOINT");
            std::io::stdout().flush().unwrap();
            let _ = std::io::stdin().read_exact(&mut [0u8; 1]);
            std::process::exit(90);
        }
        Ok(())
    };
    if mode == "backup" {
        let db = Database::builder()
            .set_cache_size(8 * 1024 * 1024)
            .open_read_only(Path::new(&source).join("data.redb"))
            .unwrap();
        backup(&db.begin_read().unwrap(), Path::new(&target), callback).unwrap();
    } else {
        restore_store(Path::new(&source), Path::new(&target), callback).unwrap();
    }
    panic!("checkpoint not reached");
}

#[test]
fn kill_restart_exposes_only_complete_artifact_or_store() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    for mode in ["backup", "restore"] {
        for point in [
            Point::Copied,
            Point::Verified,
            Point::BeforePublish,
            Point::Published,
        ] {
            let (temp, db, _) = fixture(false);
            let artifact = temp.path().join("artifact");
            backup(&db.begin_read().unwrap(), &artifact, |_| Ok(())).unwrap();
            drop(db);
            let source = if mode == "backup" {
                temp.path().join("source")
            } else {
                artifact
            };
            let data = if mode == "backup" {
                source.join("data.redb")
            } else {
                source.join("store/data.redb")
            };
            let before = fs::read(&data).unwrap();
            let target = temp.path().join("target");
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "format_v2::artifact::tests::crash_worker",
                    "--nocapture",
                ])
                .env("KIARNI_V2_ARTIFACT_SOURCE", &source)
                .env("KIARNI_V2_ARTIFACT_TARGET", &target)
                .env("KIARNI_V2_ARTIFACT_MODE", mode)
                .env("KIARNI_V2_ARTIFACT_POINT", format!("{point:?}"))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let (tx, rx) = mpsc::sync_channel(1);
            let stdout = child.stdout.take().unwrap();
            let reader = std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    if line.is_ok_and(|v| v.contains("ARTIFACT_CHECKPOINT")) {
                        let _ = tx.send(());
                        break;
                    }
                }
            });
            let reached = rx.recv_timeout(Duration::from_secs(15));
            let killed = child.kill();
            let status = child.wait().unwrap();
            reader.join().unwrap();
            reached.unwrap();
            killed.unwrap();
            assert!(!status.success());
            assert_eq!(fs::read(&data).unwrap(), before);
            assert_eq!(target.exists(), point == Point::Published);
            if point == Point::Published {
                if mode == "backup" {
                    restore_store(&target, &temp.path().join("check"), |_| Ok(())).unwrap();
                } else {
                    let db = open(&target).unwrap();
                    validate(&db).unwrap();
                    lifecycle::audit(
                        &db,
                        &Collection {
                            name: "indexed".into(),
                            id: 1,
                        },
                    )
                    .unwrap();
                }
            }
        }
    }
}
