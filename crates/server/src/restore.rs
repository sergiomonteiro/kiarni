//! Restore verified bytes into a fresh installation; never activate source auth.
use crate::{
    backup::{READ, digest, hex, publish_new},
    bootstrap::Root,
};
use kiarni_config::Config;
use kiarni_storage::{ExportSummary, Store};
use rustix::fs::{self as unix, Mode, OFlags};
use serde::Deserialize;
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::Path,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const PRIVATE_DIR: Mode = Mode::RUSR.union(Mode::WUSR).union(Mode::XUSR);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRecord {
    path: String,
    bytes: String,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    magic: String,
    version: u8,
    format_version: u8,
    engine: String,
    collections: String,
    documents: String,
    document_bytes: String,
    files: [FileRecord; 2],
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Checkpoint {
    BeforePublish,
    CredentialsPublished,
    AfterPublish,
}
fn decimal(value: &str) -> Result<u64> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid_backup_manifest".into());
    }
    value.parse().map_err(|_| "invalid_backup_manifest".into())
}
fn private(file: &File, directory: bool) -> Result<()> {
    let metadata = file.metadata().map_err(|_| "restore_source_unavailable")?;
    if metadata.uid() != rustix::process::geteuid().as_raw()
        || (if directory {
            !metadata.is_dir() || metadata.mode() & 0o7777 != 0o700
        } else {
            !metadata.is_file() || metadata.mode() & 0o7777 != 0o600 || metadata.nlink() != 1
        })
    {
        return Err("insecure_restore_source".into());
    }
    Ok(())
}
fn open_dir(path: &Path) -> Result<File> {
    let file = File::from(
        unix::open(path, READ | OFlags::DIRECTORY, Mode::empty())
            .map_err(|_| "restore_source_unavailable")?,
    );
    private(&file, true)?;
    Ok(file)
}
fn child_dir(parent: &File, name: &str) -> Result<File> {
    let file = File::from(
        unix::openat(parent, name, READ | OFlags::DIRECTORY, Mode::empty())
            .map_err(|_| "restore_source_unavailable")?,
    );
    private(&file, true)?;
    Ok(file)
}
fn entries(path: &Path, names: &[&str]) -> Result<()> {
    let mut count = 0;
    for entry in fs::read_dir(path).map_err(|_| "restore_source_unavailable")? {
        let name = entry.map_err(|_| "restore_source_unavailable")?.file_name();
        if !names.iter().any(|expected| name == *expected) {
            return Err("unexpected_backup_entry".into());
        }
        count += 1;
        if count > names.len() {
            return Err("unexpected_backup_entry".into());
        }
    }
    if count != names.len() {
        return Err("unexpected_backup_entry".into());
    }
    Ok(())
}
fn manifest(directory: &File) -> Result<(Manifest, ExportSummary)> {
    let file = File::from(
        unix::openat(directory, "backup.json", READ, Mode::empty())
            .map_err(|_| "restore_source_unavailable")?,
    );
    private(&file, false)?;
    let mut bytes = Vec::new();
    file.take(1025)
        .read_to_end(&mut bytes)
        .map_err(|_| "restore_source_unavailable")?;
    if bytes.len() > 1024 {
        return Err("invalid_backup_manifest".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| "invalid_backup_manifest")?;
    if manifest.magic != "kiarni-backup"
        || manifest.version != 1
        || manifest.format_version != 2
        || manifest.engine != "redb-4.5.0"
    {
        return Err("unsupported_backup_format".into());
    }
    let summary = ExportSummary {
        collections: decimal(&manifest.collections)?,
        documents: decimal(&manifest.documents)?,
        document_bytes: decimal(&manifest.document_bytes)?,
    };
    for (record, name) in manifest
        .files
        .iter()
        .zip(["store/format.json", "store/data.redb"])
    {
        if record.path != name
            || record.sha256.len() != 64
            || !record
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid_backup_manifest".into());
        }
        decimal(&record.bytes)?;
    }
    Ok((manifest, summary))
}
fn mkdir(parent: &File, name: &str) -> Result<File> {
    unix::mkdirat(parent, name, PRIVATE_DIR).map_err(|_| "restore_stage_unavailable")?;
    child_dir(parent, name)
}
fn copy_file(source: &File, target: &File, name: &str, expected: u64) -> Result<()> {
    let mut source = File::from(
        unix::openat(source, name, READ, Mode::empty())
            .map_err(|_| "restore_source_unavailable")?,
    );
    private(&source, false)?;
    if source
        .metadata()
        .map_err(|_| "restore_source_unavailable")?
        .len()
        != expected
    {
        return Err("backup_size_mismatch".into());
    }
    let mut target = File::from(
        unix::openat(
            target,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| "restore_copy_failed")?,
    );
    let mut remaining = expected;
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        let limit = remaining.min(buffer.len() as u64) as usize;
        let count = source
            .read(&mut buffer[..limit])
            .map_err(|_| "restore_copy_failed")?;
        if count == 0 {
            return Err("backup_size_mismatch".into());
        }
        target
            .write_all(&buffer[..count])
            .map_err(|_| "restore_copy_failed")?;
        remaining -= count as u64;
    }
    if source
        .read(&mut buffer[..1])
        .map_err(|_| "restore_copy_failed")?
        != 0
    {
        return Err("backup_size_mismatch".into());
    }
    target.sync_all().map_err(|_| "restore_copy_failed")?;
    Ok(())
}
fn absent(parent: &Path, name: &OsStr, code: &'static str) -> Result<()> {
    match fs::symlink_metadata(parent.join(name)) {
        Ok(_) => Err(code.into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("restore_destination_unavailable".into()),
    }
}
fn publication_error(
    error: rustix::io::Errno,
    collision: &'static str,
) -> Box<dyn std::error::Error> {
    if matches!(
        error,
        rustix::io::Errno::EXIST | rustix::io::Errno::NOTEMPTY
    ) {
        collision.into()
    } else if matches!(
        error,
        rustix::io::Errno::NOSYS | rustix::io::Errno::NOTSUP | rustix::io::Errno::INVAL
    ) {
        "restore_publication_unsupported".into()
    } else {
        "restore_publish_failed".into()
    }
}
pub fn restore_backup(config: Config, artifact: &Path) -> Result<ExportSummary> {
    run(config, artifact, |_| Ok(()))
}
fn run(
    config: Config,
    artifact: &Path,
    mut checkpoint: impl FnMut(Checkpoint) -> Result<()>,
) -> Result<ExportSummary> {
    let source_dir = open_dir(artifact)?;
    let source_path = fs::canonicalize(artifact).map_err(|_| "restore_source_unavailable")?;
    entries(&source_path, &["backup.json", "store"])?;
    let source_store = child_dir(&source_dir, "store")?;
    entries(&source_path.join("store"), &["format.json", "data.redb"])?;
    let (manifest, expected) = manifest(&source_dir)?;
    let destination = Path::new(config.data_dir());
    let name = destination
        .file_name()
        .filter(|_| {
            matches!(
                destination.components().next_back(),
                Some(std::path::Component::Normal(_))
            )
        })
        .ok_or("invalid_restore_destination")?;
    let parent_path = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_path = fs::canonicalize(parent_path).map_err(|_| "restore_parent_unavailable")?;
    if parent_path.starts_with(&source_path) {
        return Err("restore_destination_overlaps_backup".into());
    }
    let parent = File::from(
        unix::open(&parent_path, READ | OFlags::DIRECTORY, Mode::empty())
            .map_err(|_| "restore_parent_unavailable")?,
    );
    let metadata = parent
        .metadata()
        .map_err(|_| "restore_parent_unavailable")?;
    if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o022 != 0 {
        return Err("insecure_restore_parent".into());
    }
    let mut credentials = name.to_os_string();
    credentials.push(".credentials");
    absent(&parent_path, name, "restore_destination_exists")?;
    absent(&parent_path, &credentials, "restore_credentials_exists")?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "restore_entropy_unavailable")?;
    let staging = format!(".kiarni-restore-{}", hex(&random));
    let directory = mkdir(&parent, &staging)?;
    parent.sync_all().map_err(|_| "restore_stage_sync_failed")?;
    let stage = parent_path.join(&staging);
    let input = mkdir(&directory, ".restore-input")?;
    let input_store = mkdir(&input, "store")?;
    for (record, name) in manifest.files.iter().zip(["format.json", "data.redb"]) {
        copy_file(&source_store, &input_store, name, decimal(&record.bytes)?)?;
    }
    input_store.sync_all().map_err(|_| "restore_copy_failed")?;
    for (record, name) in manifest
        .files
        .iter()
        .zip(["store/format.json", "store/data.redb"])
    {
        let actual = digest(&stage.join(".restore-input"), name)
            .map_err(|_| "restore_copy_verification_failed")?;
        if actual.bytes != decimal(&record.bytes)?.to_string() || actual.sha256 != record.sha256 {
            return Err("backup_hash_mismatch".into());
        }
    }
    let store = Store::open(&stage.join(".restore-input/store"))?;
    let summary = store.export_snapshot(&stage.join("store"))?;
    drop(store);
    if summary != expected {
        return Err("backup_counts_mismatch".into());
    }
    drop(input_store);
    drop(input);
    for name in ["data.redb", "format.json"] {
        fs::remove_file(stage.join(".restore-input/store").join(name))
            .map_err(|_| "restore_stage_cleanup_failed")?;
    }
    fs::remove_dir(stage.join(".restore-input/store"))
        .and_then(|()| fs::remove_dir(stage.join(".restore-input")))
        .map_err(|_| "restore_stage_cleanup_failed")?;
    let root = Root::lock_existing(&stage)?;
    root.prepare_restored()?;
    let delivery = mkdir(&directory, ".credentials")?;
    delivery
        .sync_all()
        .map_err(|_| "restore_stage_sync_failed")?;
    directory
        .sync_all()
        .map_err(|_| "restore_stage_sync_failed")?;
    checkpoint(Checkpoint::BeforePublish)?;
    publish_new(&parent, &format!("{staging}/.credentials"), &credentials)
        .map_err(|e| publication_error(e, "restore_credentials_exists"))?;
    // Two publication steps: any later error requires inspecting both names.
    parent
        .sync_all()
        .map_err(|_| "restore_publication_uncertain")?;
    checkpoint(Checkpoint::CredentialsPublished).map_err(|_| "restore_publication_uncertain")?;
    publish_new(&parent, &staging, name).map_err(|_| "restore_publication_uncertain")?;
    checkpoint(Checkpoint::AfterPublish).map_err(|_| "restore_publication_uncertain")?;
    parent
        .sync_all()
        .map_err(|_| "restore_publication_uncertain")?;
    drop(root);
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use redb::{Database, ReadableDatabase, TableDefinition};
    use sha2::{Digest, Sha256};
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
    fn config(path: &Path) -> Config {
        kiarni_config::resolve(&[("--data-dir", path.to_str().unwrap())], &[], None).unwrap()
    }
    fn setup(parent: &Path) -> PathBuf {
        let source = parent.join("source");
        let root = Root::lock(&source).unwrap();
        let (store, _) = root.start(crate::now().unwrap()).unwrap();
        let collection = store.create_collection("orders").unwrap();
        let removed = store.create_collection("removed").unwrap();
        store.drop_collection(&removed).unwrap();
        store.create_collection("empty").unwrap();
        store
            .insert_fixture(
                &collection,
                br#"{"_id":"doc","n":9007199254740993,"p":1.234567890123456789,"status":"ready"}"#,
            )
            .unwrap();
        store.create_index(&collection, "status").unwrap();
        drop(store);
        drop(root);
        let artifact = parent.join("backup");
        crate::create_backup(config(&source), &artifact).unwrap();
        artifact
    }
    fn delivery(path: &Path) -> PathBuf {
        let mut name = path.file_name().unwrap().to_os_string();
        name.push(".credentials");
        path.with_file_name(name)
    }
    fn json(artifact: &Path) -> serde_json::Value {
        serde_json::from_slice(&fs::read(artifact.join("backup.json")).unwrap()).unwrap()
    }
    fn write_json(artifact: &Path, value: &serde_json::Value) {
        fs::write(
            artifact.join("backup.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
    fn rehash(artifact: &Path) {
        let mut value = json(artifact);
        for entry in value["files"].as_array_mut().unwrap() {
            let bytes = fs::read(artifact.join(entry["path"].as_str().unwrap())).unwrap();
            entry["bytes"] = serde_json::json!(bytes.len().to_string());
            entry["sha256"] = serde_json::json!(hex(&Sha256::digest(bytes)));
        }
        write_json(artifact, &value);
    }
    fn verify(path: &Path) {
        let phase: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join("bootstrap.json")).unwrap()).unwrap();
        assert_eq!(
            phase,
            serde_json::json!({"version":2,"store_format":2,"phase":"store_ready"})
        );
        assert!(!path.join("auth").exists());
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o7777, 0o700);
        assert_eq!(fs::metadata(delivery(path)).unwrap().mode() & 0o7777, 0o700);
        assert_eq!(fs::read_dir(delivery(path)).unwrap().count(), 0);
        let mut names: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, ["bootstrap.json", "serve.lock", "store"]);
        for name in [
            "bootstrap.json",
            "serve.lock",
            "store/format.json",
            "store/data.redb",
        ] {
            assert_eq!(
                fs::metadata(path.join(name)).unwrap().mode() & 0o7777,
                0o600
            );
        }
    }
    #[test]
    fn restore_preserves_raw_json_counter_and_artifact_and_bootstraps_only_after_serve() {
        let temp = temporary();
        let artifact = setup(temp.path());
        let destination = temp.path().join("restored");
        let before = fs::read(artifact.join("store/data.redb")).unwrap();
        let manifest = fs::read(artifact.join("backup.json")).unwrap();
        let summary = restore_backup(config(&destination), &artifact).unwrap();
        assert_eq!(summary.collections, 2);
        assert_eq!(summary.documents, 1);
        verify(&destination);
        assert!(fs::read(artifact.join("store/data.redb")).unwrap() == before);
        assert!(fs::read(artifact.join("backup.json")).unwrap() == manifest);
        let db = Database::open(destination.join("store/data.redb")).unwrap();
        let read = db.begin_read().unwrap();
        assert_eq!(
            read.open_table(TableDefinition::<&str, u64>::new("metadata"))
                .unwrap()
                .get("next_collection_id")
                .unwrap()
                .unwrap()
                .value(),
            4
        );
        drop(read);
        db.close().unwrap();
        let root = Root::lock_existing(&destination).unwrap();
        let (store, startup) = root.start(crate::now().unwrap()).unwrap();
        assert!(startup.created);
        assert!(startup.token_file.is_some());
        assert_eq!(
            store
                .get_document(&store.collection("orders").unwrap(), "doc")
                .unwrap()
                .unwrap(),
            br#"{"_id":"doc","n":9007199254740993,"p":1.234567890123456789,"status":"ready"}"#
        );
        store.create_collection("next").unwrap();
        drop(store);
        drop(root);
        let db = Database::open(destination.join("store/data.redb")).unwrap();
        let read = db.begin_read().unwrap();
        assert_eq!(
            read.open_table(TableDefinition::<&str, u64>::new("collections"))
                .unwrap()
                .get("next")
                .unwrap()
                .unwrap()
                .value(),
            4
        );
        assert_eq!(
            read.open_table(TableDefinition::<&str, u64>::new("metadata"))
                .unwrap()
                .get("next_collection_id")
                .unwrap()
                .unwrap()
                .value(),
            5
        );
        drop(read);
        db.close().unwrap();
        let root = Root::lock_existing(&destination).unwrap();
        let (store, startup) = root.start(crate::now().unwrap()).unwrap();
        assert!(!startup.created);
        assert!(startup.token_file.is_none());
        drop(store);
    }
    #[test]
    fn rejects_bad_manifests_paths_sizes_hashes_counts_and_versions_before_publication() {
        let temp = temporary();
        let artifact = setup(temp.path());
        let original = fs::read(artifact.join("backup.json")).unwrap();
        for case in 0..12 {
            let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
            match case {
                0 => value["version"] = serde_json::json!(2),
                1 => value["format_version"] = serde_json::json!(1),
                2 => value["engine"] = serde_json::json!("other"),
                3 => value["extra"] = serde_json::json!(true),
                4 => value["files"][0]["path"] = serde_json::json!("../outside"),
                5 => value["files"][1]["bytes"] = serde_json::json!("0"),
                6 => value["files"][1]["sha256"] = serde_json::json!("a".repeat(64)),
                7 => value["documents"] = serde_json::json!("999"),
                8 => value["document_bytes"] = serde_json::json!("18446744073709551616"),
                9 => value["files"][0]["sha256"] = serde_json::json!("G".repeat(64)),
                10 => value["files"] = serde_json::json!([]),
                _ => value["collections"] = serde_json::json!("-1"),
            }
            write_json(&artifact, &value);
            let out = temp.path().join(format!("out{case}"));
            assert!(
                restore_backup(config(&out), &artifact).is_err(),
                "accepted bad manifest case {case}"
            );
            assert!(!out.exists());
            assert!(!delivery(&out).exists());
        }
        for bytes in [
            b"{".as_slice(),
            b"{\"version\":1,\"version\":1}".as_slice(),
            &vec![b' '; 1025],
        ] {
            fs::write(artifact.join("backup.json"), bytes).unwrap();
            assert!(restore_backup(config(&temp.path().join("out")), &artifact).is_err());
        }
    }
    #[test]
    fn rejects_extra_entries_links_permissions_and_truncated_or_modified_files() {
        for case in 0..7 {
            let temp = temporary();
            let artifact = setup(temp.path());
            let out = temp.path().join("out");
            match case {
                0 => fs::write(artifact.join("auth"), b"unexpected").unwrap(),
                1 => fs::write(artifact.join("store/extra"), b"unexpected").unwrap(),
                2 => {
                    let data = artifact.join("store/data.redb");
                    let saved = temp.path().join("saved");
                    fs::rename(&data, &saved).unwrap();
                    symlink(saved, data).unwrap();
                }
                3 => {
                    let manifest = artifact.join("backup.json");
                    fs::hard_link(&manifest, temp.path().join("linked")).unwrap();
                }
                4 => fs::set_permissions(
                    artifact.join("backup.json"),
                    fs::Permissions::from_mode(0o644),
                )
                .unwrap(),
                5 => {
                    let f = fs::OpenOptions::new()
                        .write(true)
                        .open(artifact.join("store/data.redb"))
                        .unwrap();
                    f.set_len(100).unwrap();
                }
                _ => {
                    let mut bytes = fs::read(artifact.join("store/data.redb")).unwrap();
                    let last = bytes.len() - 1;
                    bytes[last] ^= 1;
                    fs::write(artifact.join("store/data.redb"), bytes).unwrap();
                }
            }
            assert!(
                restore_backup(config(&out), &artifact).is_err(),
                "accepted bad artifact case {case}"
            );
            assert!(!out.exists());
            assert!(!delivery(&out).exists());
        }
        let temp = temporary();
        let artifact = setup(temp.path());
        let link = temp.path().join("link");
        symlink(&artifact, &link).unwrap();
        assert!(restore_backup(config(&temp.path().join("out")), &link).is_err());
    }
    #[test]
    fn invalid_structure_with_recomputed_hashes_still_fails_and_never_mutates_backup() {
        for case in 0..7 {
            let temp = temporary();
            let artifact = setup(temp.path());
            let db = Database::open(artifact.join("store/data.redb")).unwrap();
            let write = db.begin_write().unwrap();
            match case {
                0 => {
                    write
                        .open_table(TableDefinition::<&str, u64>::new("metadata"))
                        .unwrap()
                        .insert("next_collection_id", 1)
                        .unwrap();
                }
                1 => {
                    write
                        .open_table(TableDefinition::<&str, u64>::new("collections"))
                        .unwrap()
                        .insert("duplicate", 1)
                        .unwrap();
                }
                2 => {
                    write
                        .open_table(TableDefinition::<(u64, &str), &[u8]>::new("documents"))
                        .unwrap()
                        .insert((99, "orphan"), br#"{"_id":"orphan"}"#.as_slice())
                        .unwrap();
                }
                3 => {
                    write
                        .open_table(TableDefinition::<(u64, &str), &[u8]>::new("documents"))
                        .unwrap()
                        .insert((1, "doc"), br#"{"_id":"wrong"}"#.as_slice())
                        .unwrap();
                }
                5 | 6 => {
                    write
                        .open_table(TableDefinition::<(u64, &str, &str), ()>::new(
                            "secondary_entries",
                        ))
                        .unwrap()
                        .remove((1, "ready", "doc"))
                        .unwrap();
                    if case == 6 {
                        write
                            .open_table(TableDefinition::<(u64, &str), u64>::new(
                                "secondary_counts",
                            ))
                            .unwrap()
                            .remove((1, "ready"))
                            .unwrap();
                    }
                }
                _ => {
                    write
                        .open_table(TableDefinition::<&str, u64>::new("unexpected"))
                        .unwrap();
                }
            }
            write.commit().unwrap();
            db.close().unwrap();
            rehash(&artifact);
            let bytes = fs::read(artifact.join("store/data.redb")).unwrap();
            let out = temp.path().join("out");
            assert!(
                restore_backup(config(&out), &artifact).is_err(),
                "accepted invalid layout case {case}"
            );
            assert!(!out.exists());
            assert!(!delivery(&out).exists());
            assert!(fs::read(artifact.join("store/data.redb")).unwrap() == bytes);
        }
    }
    #[test]
    fn destination_and_credentials_collisions_overlap_and_post_publish_failure_are_safe() {
        let temp = temporary();
        let artifact = setup(temp.path());
        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("sentinel"), b"keep").unwrap();
        assert_eq!(
            restore_backup(config(&out), &artifact)
                .unwrap_err()
                .to_string(),
            "restore_destination_exists"
        );
        assert_eq!(fs::read(out.join("sentinel")).unwrap(), b"keep");
        let link = temp.path().join("destination-link");
        symlink(&out, &link).unwrap();
        assert_eq!(
            restore_backup(config(&link), &artifact)
                .unwrap_err()
                .to_string(),
            "restore_destination_exists"
        );
        assert_eq!(fs::read(out.join("sentinel")).unwrap(), b"keep");
        let unsafe_parent = temp.path().join("unsafe");
        fs::create_dir(&unsafe_parent).unwrap();
        fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            restore_backup(config(&unsafe_parent.join("out")), &artifact)
                .unwrap_err()
                .to_string(),
            "insecure_restore_parent"
        );
        let out = temp.path().join("credentials");
        fs::create_dir(delivery(&out)).unwrap();
        assert_eq!(
            restore_backup(config(&out), &artifact)
                .unwrap_err()
                .to_string(),
            "restore_credentials_exists"
        );
        assert!(!out.exists());
        assert_eq!(
            restore_backup(config(&artifact.join("nested")), &artifact)
                .unwrap_err()
                .to_string(),
            "restore_destination_overlaps_backup"
        );
        let out = temp.path().join("race");
        let error = run(config(&out), &artifact, |point| {
            if point == Checkpoint::BeforePublish {
                fs::create_dir(delivery(&out))?;
                fs::write(delivery(&out).join("sentinel"), b"keep")?;
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "restore_credentials_exists");
        assert_eq!(fs::read(delivery(&out).join("sentinel")).unwrap(), b"keep");
        assert!(!out.exists());
        let out = temp.path().join("race-root");
        let error = run(config(&out), &artifact, |point| {
            if point == Checkpoint::CredentialsPublished {
                fs::create_dir(&out)?;
                fs::write(out.join("sentinel"), b"keep")?;
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "restore_publication_uncertain");
        assert_eq!(fs::read(out.join("sentinel")).unwrap(), b"keep");
        let out = temp.path().join("uncertain");
        let error = run(config(&out), &artifact, |point| {
            if point == Checkpoint::AfterPublish {
                return Err("injected_sync_failure".into());
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "restore_publication_uncertain");
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
    #[ignore = "subprocess worker for restore interruption tests"]
    fn crash_worker() {
        let Some(artifact) = std::env::var_os("KIARNI_TEST_RESTORE_SOURCE") else {
            return;
        };
        let out = PathBuf::from(std::env::var_os("KIARNI_TEST_RESTORE_OUT").unwrap());
        let signal = PathBuf::from(std::env::var_os("KIARNI_TEST_RESTORE_SIGNAL").unwrap());
        let point = match std::env::var("KIARNI_TEST_RESTORE_POINT").unwrap().as_str() {
            "before" => Checkpoint::BeforePublish,
            "credentials" => Checkpoint::CredentialsPublished,
            _ => Checkpoint::AfterPublish,
        };
        run(config(&out), Path::new(&artifact), |phase| {
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
    fn kill_at_each_publication_boundary_preserves_complete_roots_or_explicit_empty_reservation() {
        for point in ["before", "credentials", "after"] {
            let temp = temporary();
            let artifact = setup(temp.path());
            let out = temp.path().join("out");
            let signal = temp.path().join("signal");
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "restore::tests::crash_worker",
                    "--ignored",
                    "--nocapture",
                ])
                .env("KIARNI_TEST_RESTORE_SOURCE", &artifact)
                .env("KIARNI_TEST_RESTORE_OUT", &out)
                .env("KIARNI_TEST_RESTORE_SIGNAL", &signal)
                .env("KIARNI_TEST_RESTORE_POINT", point)
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
            assert_eq!(delivery(&out).exists(), point != "before");
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            if point == "after" {
                verify(&out);
                let root = Root::lock_existing(&out).unwrap();
                let (store, startup) = root.start(crate::now().unwrap()).unwrap();
                assert!(startup.created);
                drop(store);
            } else if point == "credentials" {
                assert_eq!(fs::read_dir(delivery(&out)).unwrap().count(), 0);
                assert_eq!(
                    restore_backup(config(&out), &artifact)
                        .unwrap_err()
                        .to_string(),
                    "restore_credentials_exists"
                );
            } else {
                assert_eq!(
                    restore_backup(config(&out), &artifact).unwrap().documents,
                    1
                );
            }
        }
    }
}
