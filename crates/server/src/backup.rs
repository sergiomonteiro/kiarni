//! Offline artifact publication. No HTTP, authentication activation or replay.
use crate::bootstrap::Root;
use kiarni_config::Config;
use kiarni_storage::ExportSummary;
use rustix::fs::{self as unix, Mode, OFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::Path,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const READ: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
#[derive(Clone, Copy, PartialEq, Eq)]
enum Checkpoint {
    BeforePublish,
    AfterPublish,
}
#[derive(Serialize)]
struct FileRecord {
    path: &'static str,
    bytes: String,
    sha256: String,
}
#[derive(Serialize)]
struct Manifest {
    magic: &'static str,
    version: u8,
    format_version: u8,
    engine: &'static str,
    collections: String,
    documents: String,
    document_bytes: String,
    files: [FileRecord; 2],
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[(byte >> 4) as usize]));
        text.push(char::from(DIGITS[(byte & 15) as usize]));
    }
    text
}
fn digest(root: &Path, name: &'static str) -> Result<FileRecord> {
    let mut file = File::from(
        unix::open(root.join(name), READ, Mode::empty()).map_err(|_| "backup_file_unavailable")?,
    );
    let metadata = file.metadata().map_err(|_| "backup_file_unavailable")?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o600
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err("insecure_backup_file".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|_| "backup_hash_failed")?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        bytes = bytes
            .checked_add(count as u64)
            .ok_or("backup_size_overflow")?;
    }
    Ok(FileRecord {
        path: name,
        bytes: bytes.to_string(),
        sha256: hex(&hash.finalize()),
    })
}
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
fn publish(parent: &File, staging: &str, name: &std::ffi::OsStr) -> Result<()> {
    unix::renameat_with(parent, staging, parent, name, unix::RenameFlags::NOREPLACE).map_err(
        |error| {
            let code = if matches!(
                error,
                rustix::io::Errno::EXIST | rustix::io::Errno::NOTEMPTY
            ) {
                "backup_destination_exists"
            } else if matches!(
                error,
                rustix::io::Errno::NOSYS | rustix::io::Errno::NOTSUP | rustix::io::Errno::INVAL
            ) {
                "backup_publication_unsupported"
            } else {
                "backup_publish_failed"
            };
            code.into()
        },
    )
}
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
)))]
fn publish(_: &File, _: &str, _: &std::ffi::OsStr) -> Result<()> {
    Err("backup_publication_unsupported".into())
}

pub fn create_backup(config: Config, destination: &Path) -> Result<ExportSummary> {
    run(config, destination, |_| Ok(()))
}
fn run(
    config: Config,
    destination: &Path,
    mut checkpoint: impl FnMut(Checkpoint) -> Result<()>,
) -> Result<ExportSummary> {
    let root = Root::lock_existing(config.data_dir().as_ref())?;
    let source = fs::canonicalize(config.data_dir()).map_err(|_| "data_directory_unavailable")?;
    let name = destination
        .file_name()
        .filter(|_| {
            matches!(
                destination.components().next_back(),
                Some(std::path::Component::Normal(_))
            )
        })
        .ok_or("invalid_backup_destination")?;
    let parent_path = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_path = fs::canonicalize(parent_path).map_err(|_| "backup_parent_unavailable")?;
    let credentials = source.with_file_name(format!(
        "{}.credentials",
        source
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("invalid_backup_source")?
    ));
    if parent_path.starts_with(&source) || parent_path.starts_with(&credentials) {
        return Err("backup_destination_overlaps_source".into());
    }
    let parent = File::from(
        unix::open(&parent_path, READ | OFlags::DIRECTORY, Mode::empty())
            .map_err(|_| "backup_parent_unavailable")?,
    );
    let metadata = parent.metadata().map_err(|_| "backup_parent_unavailable")?;
    if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o022 != 0 {
        return Err("insecure_backup_parent".into());
    }
    match fs::symlink_metadata(parent_path.join(name)) {
        Ok(_) => return Err("backup_destination_exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("backup_destination_unavailable".into()),
    }
    let store = root.backup_store()?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "backup_entropy_unavailable")?;
    let staging = format!(".kiarni-backup-{}", hex(&random));
    unix::mkdirat(
        &parent,
        staging.as_str(),
        Mode::RUSR | Mode::WUSR | Mode::XUSR,
    )
    .map_err(|_| "backup_stage_unavailable")?;
    let stage = parent_path.join(&staging);
    let directory = File::from(
        unix::openat(
            &parent,
            staging.as_str(),
            READ | OFlags::DIRECTORY,
            Mode::empty(),
        )
        .map_err(|_| "backup_stage_unavailable")?,
    );
    parent.sync_all().map_err(|_| "backup_stage_sync_failed")?;
    let summary = store.export_snapshot(&stage.join("store"))?;
    let manifest = Manifest {
        magic: "kiarni-backup",
        version: 1,
        format_version: 1,
        engine: "redb-4.5.0",
        collections: summary.collections.to_string(),
        documents: summary.documents.to_string(),
        document_bytes: summary.document_bytes.to_string(),
        files: [
            digest(&stage, "store/format.json")?,
            digest(&stage, "store/data.redb")?,
        ],
    };
    let bytes = serde_json::to_vec(&manifest).map_err(|_| "backup_manifest_failed")?;
    let mut file = File::from(
        unix::openat(
            &directory,
            "backup.json",
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| "backup_manifest_failed")?,
    );
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "backup_manifest_failed")?;
    drop(file);
    directory
        .sync_all()
        .map_err(|_| "backup_stage_sync_failed")?;
    checkpoint(Checkpoint::BeforePublish)?;
    publish(&parent, &staging, name)?;
    // After this point failures must not imply the destination is absent.
    checkpoint(Checkpoint::AfterPublish).map_err(|_| "backup_publication_uncertain")?;
    parent
        .sync_all()
        .map_err(|_| "backup_publication_uncertain")?;
    drop(store); // Keep the installation lock until the source engine closes.
    drop(root);
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        path::PathBuf,
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    fn temporary() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        temp
    }
    fn config(source: &Path) -> Config {
        kiarni_config::resolve(&[("--data-dir", source.to_str().unwrap())], &[], None).unwrap()
    }
    fn setup(parent: &Path) -> PathBuf {
        let source = parent.join("data");
        let root = Root::lock(&source).unwrap();
        let (store, _) = root.start(crate::now().unwrap()).unwrap();
        let orders = store.create_collection("orders").unwrap();
        store
            .insert_fixture(&orders, br#"{"_id":"doc","n":9007199254740993}"#)
            .unwrap();
        drop(store);
        drop(root);
        source
    }
    fn verify(destination: &Path) -> serde_json::Value {
        assert_eq!(fs::metadata(destination).unwrap().mode() & 0o7777, 0o700);
        let bytes = fs::read(destination.join("backup.json")).unwrap();
        assert!(bytes.len() < 1024);
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(manifest["magic"], "kiarni-backup");
        assert_eq!(manifest["version"], 1);
        assert_eq!(manifest["format_version"], 1);
        assert_eq!(manifest["engine"], "redb-4.5.0");
        let files = manifest["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["path"], "store/format.json");
        assert_eq!(files[1]["path"], "store/data.redb");
        for record in files {
            let path = destination.join(record["path"].as_str().unwrap());
            let bytes = fs::read(&path).unwrap();
            assert_eq!(record["bytes"], bytes.len().to_string());
            assert_eq!(record["sha256"], hex(&Sha256::digest(&bytes)));
            assert_eq!(fs::metadata(path).unwrap().mode() & 0o7777, 0o600);
        }
        assert_eq!(
            fs::metadata(destination.join("backup.json"))
                .unwrap()
                .mode()
                & 0o7777,
            0o600
        );
        let mut names: Vec<_> = fs::read_dir(destination)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, ["backup.json", "store"]);
        assert_eq!(fs::read_dir(destination.join("store")).unwrap().count(), 2);
        manifest
    }
    #[test]
    fn artifact_has_hashes_private_files_and_no_auth_and_leaves_source_usable() {
        let temp = temporary();
        let source = setup(temp.path());
        let auth = fs::read(source.join("auth/active.json")).unwrap();
        let credentials: Vec<_> = fs::read_dir(temp.path().join("data.credentials"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let destination = temp.path().join("backup");
        let summary = create_backup(config(&source), &destination).unwrap();
        assert_eq!(summary.documents, 1);
        let manifest = verify(&destination);
        assert_eq!(manifest["collections"], "1");
        assert_eq!(manifest["documents"], "1");
        assert_eq!(
            manifest["document_bytes"],
            summary.document_bytes.to_string()
        );
        assert!(fs::read(source.join("auth/active.json")).unwrap() == auth);
        let after: Vec<_> = fs::read_dir(temp.path().join("data.credentials"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(after, credentials);
        let root = Root::lock_existing(&source).unwrap();
        root.backup_store().unwrap().check_readable().unwrap();
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    #[test]
    fn refuses_live_lock_existing_targets_overlap_and_unsafe_parent() {
        let temp = temporary();
        let source = setup(temp.path());
        let destination = temp.path().join("backup");
        let lock = Root::lock_existing(&source).unwrap();
        assert_eq!(
            create_backup(config(&source), &destination)
                .unwrap_err()
                .to_string(),
            "data_directory_locked"
        );
        drop(lock);
        for directory in [&source, &temp.path().join("data.credentials")] {
            assert_eq!(
                create_backup(config(&source), &directory.join("backup"))
                    .unwrap_err()
                    .to_string(),
                "backup_destination_overlaps_source"
            );
        }
        fs::write(&destination, b"sentinel").unwrap();
        assert_eq!(
            create_backup(config(&source), &destination)
                .unwrap_err()
                .to_string(),
            "backup_destination_exists"
        );
        assert_eq!(fs::read(&destination).unwrap(), b"sentinel");
        let link = temp.path().join("link");
        symlink(&destination, &link).unwrap();
        assert_eq!(
            create_backup(config(&source), &link)
                .unwrap_err()
                .to_string(),
            "backup_destination_exists"
        );
        let unsafe_parent = temp.path().join("unsafe");
        fs::create_dir(&unsafe_parent).unwrap();
        fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            create_backup(config(&source), &unsafe_parent.join("backup"))
                .unwrap_err()
                .to_string(),
            "insecure_backup_parent"
        );
        assert!(!unsafe_parent.join("backup").exists());
    }
    #[test]
    fn manifest_preflight_and_initialization_errors_never_open_or_create_a_store() {
        let temp = temporary();
        let source = setup(temp.path());
        let data = fs::read(source.join("store/data.redb")).unwrap();
        fs::write(source.join("store/format.json"), b"{}").unwrap();
        let out = temp.path().join("backup");
        assert!(create_backup(config(&source), &out).is_err());
        assert!(fs::read(source.join("store/data.redb")).unwrap() == data);
        assert!(!out.exists());
        let other = temp.path().join("empty");
        let root = Root::lock(&other).unwrap();
        drop(root);
        assert_eq!(
            create_backup(config(&other), &out).unwrap_err().to_string(),
            "installation_not_initialized"
        );
        assert!(!other.join("store").exists());
    }
    #[test]
    fn concurrent_destination_is_not_replaced_and_post_publish_failure_is_uncertain() {
        let temp = temporary();
        let source = setup(temp.path());
        let out = temp.path().join("backup");
        let error = run(config(&source), &out, |phase| {
            if phase == Checkpoint::BeforePublish {
                fs::create_dir(&out)?;
                fs::write(out.join("sentinel"), b"keep")?;
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "backup_destination_exists");
        assert_eq!(fs::read(out.join("sentinel")).unwrap(), b"keep");
        let out = temp.path().join("uncertain");
        let error = run(config(&source), &out, |phase| {
            if phase == Checkpoint::AfterPublish {
                return Err("injected_sync_failure".into());
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "backup_publication_uncertain");
        verify(&out);
    }
    struct KillOnDrop(Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[test]
    #[ignore = "subprocess worker for abrupt interruption test"]
    fn crash_worker() {
        let Some(source) = std::env::var_os("KIARNI_TEST_BACKUP_SOURCE") else {
            return;
        };
        let out = PathBuf::from(std::env::var_os("KIARNI_TEST_BACKUP_OUT").unwrap());
        let signal = PathBuf::from(std::env::var_os("KIARNI_TEST_BACKUP_SIGNAL").unwrap());
        let point = if std::env::var("KIARNI_TEST_BACKUP_POINT").unwrap() == "before" {
            Checkpoint::BeforePublish
        } else {
            Checkpoint::AfterPublish
        };
        run(config(Path::new(&source)), &out, |phase| {
            if phase == point {
                fs::write(&signal, b"ready")?;
                loop {
                    thread::park();
                }
            }
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn abrupt_kill_before_and_after_publication_never_leaves_partial_final_artifact() {
        for point in ["before", "after"] {
            let temp = temporary();
            let source = setup(temp.path());
            let out = temp.path().join("backup");
            let signal = temp.path().join("signal");
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "backup::tests::crash_worker",
                    "--ignored",
                    "--nocapture",
                ])
                .env("KIARNI_TEST_BACKUP_SOURCE", &source)
                .env("KIARNI_TEST_BACKUP_OUT", &out)
                .env("KIARNI_TEST_BACKUP_SIGNAL", &signal)
                .env("KIARNI_TEST_BACKUP_POINT", point)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut child = KillOnDrop(child);
            let deadline = Instant::now() + Duration::from_secs(8);
            while !signal.exists() {
                assert!(
                    child.0.try_wait().unwrap().is_none(),
                    "worker exited before checkpoint"
                );
                assert!(Instant::now() < deadline, "checkpoint deadline");
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(out.exists(), point == "after");
            assert_eq!(
                Root::lock_existing(&source).err().unwrap().to_string(),
                "data_directory_locked"
            );
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            if point == "after" {
                verify(&out);
            } else {
                let stages: Vec<_> = fs::read_dir(temp.path())
                    .unwrap()
                    .filter_map(|e| {
                        let p = e.unwrap().path();
                        p.file_name()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .starts_with(".kiarni-backup-")
                            .then_some(p)
                    })
                    .collect();
                assert_eq!(stages.len(), 1);
                verify(&stages[0]);
            }
            let root = Root::lock_existing(&source).unwrap();
            root.backup_store().unwrap().check_readable().unwrap();
        }
    }
}
