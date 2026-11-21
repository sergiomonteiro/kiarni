use super::super::tests::fixture;
use super::*;
use crate::format_v2::backup::tests::equal;
use std::os::unix::fs::symlink;

fn delivery(root: &Path) -> PathBuf {
    let mut name = root.file_name().unwrap().to_os_string();
    name.push(".credentials");
    root.with_file_name(name)
}
fn check(root: &Path) -> Result<()> {
    let dir = directory(root)?;
    entries(&dir, &["store", "serve.lock", "bootstrap.json"])?;
    let lock = child(&dir, "serve.lock", false)?;
    private(&lock, false)?;
    let mut bytes = Vec::new();
    child(&dir, "bootstrap.json", false)?
        .take(1025)
        .read_to_end(&mut bytes)
        .map_err(|_| Fault::Io)?;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Record {
        version: u8,
        phase: String,
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(|_| Fault::Invalid)?;
    if record.version != 1 || record.phase != "store_ready" {
        return Err(Fault::Invalid);
    }
    let credentials = directory(&delivery(root))?;
    entries(&credentials, &[])?;
    let store = open(&root.join("store")).map_err(|_| Fault::Storage)?;
    validate(&store).map_err(|_| Fault::Storage)?;
    lifecycle::audit(
        &store,
        &Collection {
            name: "indexed".into(),
            id: 1,
        },
    )
    .map_err(|_| Fault::Storage)
}

#[test]
fn restored_root_is_private_locked_ready_and_contains_no_auth() {
    let (temp, db, _) = fixture(false);
    let read = db.begin_read().unwrap();
    let artifact = temp.path().join("artifact");
    let expected = backup(&read, &artifact, |_| Ok(())).unwrap();
    let before = fs::read(artifact.join("store/data.redb")).unwrap();
    let output = temp.path().join("installed");
    let actual = restore_installation(&artifact, &output, |point| {
        if point == Point::RootPublished {
            let lock = child(&directory(&output).unwrap(), "serve.lock", false).unwrap();
            assert_eq!(
                unix::flock(&lock, unix::FlockOperation::NonBlockingLockExclusive),
                Err(rustix::io::Errno::WOULDBLOCK)
            );
            // Confirm lock exclusion with an independent process as well.
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "format_v2::artifact::installation::tests::worker",
                    "--nocapture",
                ])
                .env("KIARNI_V2_INSTALL_MODE", "probe")
                .env("KIARNI_V2_INSTALL_OUTPUT", &output)
                .output()
                .unwrap()
                .status;
            assert!(status.success());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(actual, expected);
    check(&output).unwrap();
    let lock = child(&directory(&output).unwrap(), "serve.lock", false).unwrap();
    unix::flock(&lock, unix::FlockOperation::NonBlockingLockExclusive).unwrap();
    drop(lock);
    let restored = open(&output.join("store")).unwrap();
    equal(&read, &restored.begin_read().unwrap());
    assert_eq!(fs::read(artifact.join("store/data.redb")).unwrap(), before);
    assert!(!output.join("auth").exists());
    assert!(!output.join(".bootstrap.tmp").exists());
    assert!(!fs::read_dir(temp.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".candidate-install-")
    }));
}

fn points() -> [Point; 4] {
    [
        Point::Prepared,
        Point::DeliveryPublished,
        Point::BeforeRoot,
        Point::RootPublished,
    ]
}

#[test]
fn returned_failures_keep_publication_state_explicit_and_refuse_reuse() {
    for failed in points() {
        let (temp, db, _) = fixture(false);
        let artifact = temp.path().join("artifact");
        backup(&db.begin_read().unwrap(), &artifact, |_| Ok(())).unwrap();
        let output = temp.path().join("installed");
        assert_eq!(
            restore_installation(&artifact, &output, |p| if p == failed {
                Err(Fault::Io)
            } else {
                Ok(())
            }),
            Err(if failed == Point::Prepared {
                Fault::Io
            } else {
                Fault::PublicationUncertain
            })
        );
        assert_eq!(output.exists(), failed == Point::RootPublished);
        assert_eq!(delivery(&output).exists(), failed != Point::Prepared);
        if failed == Point::RootPublished {
            check(&output).unwrap();
        }
        if failed != Point::Prepared {
            assert_eq!(fs::read_dir(delivery(&output)).unwrap().count(), 0);
            assert!(restore_installation(&artifact, &output, |_| Ok(())).is_err());
        } else {
            restore_installation(&artifact, &output, |_| Ok(())).unwrap();
            check(&output).unwrap();
        }
    }
}

#[test]
fn existing_or_raced_names_never_get_replaced_or_cleaned() {
    for case in 0..6 {
        let (temp, db, _) = fixture(false);
        let artifact = temp.path().join("artifact");
        backup(&db.begin_read().unwrap(), &artifact, |_| Ok(())).unwrap();
        let output = temp.path().join("installed");
        let credentials = delivery(&output);
        match case {
            0 => {
                fs::create_dir(&output).unwrap();
                fs::write(output.join("sentinel"), b"keep").unwrap();
            }
            1 => {
                fs::create_dir(&credentials).unwrap();
                fs::write(credentials.join("sentinel"), b"keep").unwrap();
            }
            2 => {
                symlink(&artifact, &credentials).unwrap();
            }
            3 => {
                symlink(&artifact, &output).unwrap();
            }
            _ => {}
        }
        let result = restore_installation(&artifact, &output, |point| {
            if (case == 4 && point == Point::Prepared) || (case == 5 && point == Point::BeforeRoot)
            {
                let path = if case == 4 { &credentials } else { &output };
                fs::create_dir(path).unwrap();
                fs::write(path.join("sentinel"), b"keep").unwrap();
            }
            Ok(())
        });
        assert!(result.is_err());
        if case == 5 {
            assert_eq!(result, Err(Fault::PublicationUncertain));
            assert_eq!(fs::read_dir(&credentials).unwrap().count(), 0);
        }
        if matches!(case, 0 | 5) {
            assert_eq!(fs::read(output.join("sentinel")).unwrap(), b"keep");
        }
        if matches!(case, 1 | 4) {
            assert_eq!(fs::read(credentials.join("sentinel")).unwrap(), b"keep");
        }
        if case == 2 {
            assert!(
                fs::symlink_metadata(&credentials)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
        if case == 3 {
            assert!(
                fs::symlink_metadata(&output)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }
}

#[test]
fn corrupt_artifact_and_internal_destination_never_publish_delivery() {
    let (temp, db, _) = fixture(false);
    let artifact = temp.path().join("artifact");
    backup(&db.begin_read().unwrap(), &artifact, |_| Ok(())).unwrap();
    let internal = artifact.join("installed");
    assert!(restore_installation(&artifact, &internal, |_| Ok(())).is_err());
    assert!(!internal.exists());
    assert!(!delivery(&internal).exists());
    let output = temp.path().join("installed");
    fs::write(artifact.join("backup.json"), b"invalid").unwrap();
    assert!(restore_installation(&artifact, &output, |_| Ok(())).is_err());
    assert!(!output.exists());
    assert!(!delivery(&output).exists());
}

#[test]
#[ignore = "subprocess fixture for installation publication and lock probe"]
fn worker() {
    use std::io::{Read, Write};
    let output = std::env::var_os("KIARNI_V2_INSTALL_OUTPUT").unwrap();
    if std::env::var("KIARNI_V2_INSTALL_MODE").as_deref() == Ok("probe") {
        let lock = child(&directory(Path::new(&output)).unwrap(), "serve.lock", false).unwrap();
        assert_eq!(
            unix::flock(&lock, unix::FlockOperation::NonBlockingLockExclusive),
            Err(rustix::io::Errno::WOULDBLOCK)
        );
        return;
    }
    let source = std::env::var_os("KIARNI_V2_INSTALL_SOURCE").unwrap();
    let requested = std::env::var("KIARNI_V2_INSTALL_POINT").unwrap();
    restore_installation(Path::new(&source), Path::new(&output), |point| {
        if format!("{point:?}") == requested {
            println!("INSTALL_CHECKPOINT");
            std::io::stdout().flush().unwrap();
            let _ = std::io::stdin().read_exact(&mut [0u8; 1]);
            std::process::exit(90);
        }
        Ok(())
    })
    .unwrap();
    panic!("checkpoint not reached");
}

#[test]
fn killed_installation_publication_has_no_partial_ready_root_or_credentials() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    for point in points() {
        let (temp, db, _) = fixture(false);
        let artifact = temp.path().join("artifact");
        backup(&db.begin_read().unwrap(), &artifact, |_| Ok(())).unwrap();
        let before = fs::read(artifact.join("store/data.redb")).unwrap();
        let output = temp.path().join("installed");
        let mut child_process = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "format_v2::artifact::installation::tests::worker",
                "--nocapture",
            ])
            .env("KIARNI_V2_INSTALL_OUTPUT", &output)
            .env("KIARNI_V2_INSTALL_SOURCE", &artifact)
            .env("KIARNI_V2_INSTALL_POINT", format!("{point:?}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let stdout = child_process.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.is_ok_and(|v| v.contains("INSTALL_CHECKPOINT")) {
                    let _ = tx.send(());
                    break;
                }
            }
        });
        let reached = rx.recv_timeout(Duration::from_secs(15));
        let killed = child_process.kill();
        let status = child_process.wait().unwrap();
        reader.join().unwrap();
        reached.unwrap();
        killed.unwrap();
        assert!(!status.success());
        assert_eq!(fs::read(artifact.join("store/data.redb")).unwrap(), before);
        assert_eq!(output.exists(), point == Point::RootPublished);
        assert_eq!(delivery(&output).exists(), point != Point::Prepared);
        if point != Point::Prepared {
            assert_eq!(fs::read_dir(delivery(&output)).unwrap().count(), 0);
            assert!(restore_installation(&artifact, &output, |_| Ok(())).is_err());
        }
        if point == Point::RootPublished {
            check(&output).unwrap();
            let lock = child(&directory(&output).unwrap(), "serve.lock", false).unwrap();
            unix::flock(&lock, unix::FlockOperation::NonBlockingLockExclusive).unwrap();
        }
    }
}
