//! Unix test candidate: hashed artifact and fresh-store publication, no CLI/auth.
use super::*;
use rustix::fs::{self as unix, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::File,
    io::{Seek, SeekFrom},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
};

type Result<T> = std::result::Result<T, Fault>;
#[derive(Debug, PartialEq, Eq)]
enum Fault {
    Invalid,
    Io,
    Storage,
    PublicationUncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    Copied,
    Verified,
    BeforePublish,
    Published,
}
const READ: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK)
    .union(OFlags::CLOEXEC);
const PATHS: [&str; 2] = ["store/format.json", "store/data.redb"];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    path: String,
    bytes: String,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    magic: String,
    version: u8,
    format_version: u8,
    engine: String,
    collections: String,
    documents: String,
    document_bytes: String,
    files: [Record; 2],
}

fn private(file: &File, directory: bool) -> Result<()> {
    let m = file.metadata().map_err(|_| Fault::Io)?;
    if m.uid() != rustix::process::geteuid().as_raw()
        || if directory {
            !m.is_dir() || m.mode() & 0o7777 != 0o700
        } else {
            !m.is_file() || m.mode() & 0o7777 != 0o600 || m.nlink() != 1
        }
    {
        return Err(Fault::Invalid);
    }
    Ok(())
}
fn directory(path: &Path) -> Result<File> {
    let file = File::from(
        unix::open(path, READ | OFlags::DIRECTORY, Mode::empty()).map_err(|_| Fault::Invalid)?,
    );
    private(&file, true)?;
    Ok(file)
}
fn child(parent: &File, name: &str, directory: bool) -> Result<File> {
    let flags = if directory {
        READ | OFlags::DIRECTORY
    } else {
        READ
    };
    let file =
        File::from(unix::openat(parent, name, flags, Mode::empty()).map_err(|_| Fault::Invalid)?);
    private(&file, directory)?;
    Ok(file)
}
fn entries(dir: &File, expected: &[&str]) -> Result<()> {
    let mut count = 0;
    for entry in unix::Dir::read_from(dir).map_err(|_| Fault::Io)? {
        let entry = entry.map_err(|_| Fault::Io)?;
        let name = entry.file_name().to_bytes();
        if matches!(name, b"." | b"..") {
            continue;
        }
        if !expected.iter().any(|e| e.as_bytes() == name) {
            return Err(Fault::Invalid);
        }
        count += 1;
        if count > expected.len() {
            return Err(Fault::Invalid);
        }
    }
    if count != expected.len() {
        return Err(Fault::Invalid);
    }
    Ok(())
}
fn decimal(value: &str) -> Result<u64> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|v| v.is_ascii_digit()) {
        return Err(Fault::Invalid);
    }
    value.parse().map_err(|_| Fault::Invalid)
}
fn decode(bytes: &[u8]) -> Result<(Manifest, ExportSummary)> {
    if bytes.len() > 1024 {
        return Err(Fault::Invalid);
    }
    let m: Manifest = serde_json::from_slice(bytes).map_err(|_| Fault::Invalid)?;
    if m.magic != "kiarni-backup"
        || m.version != 1
        || m.format_version != 2
        || m.engine != "redb-4.5.0"
    {
        return Err(Fault::Invalid);
    }
    for (r, path) in m.files.iter().zip(PATHS) {
        if r.path != path
            || r.sha256.len() != 64
            || !r
                .sha256
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        {
            return Err(Fault::Invalid);
        }
        decimal(&r.bytes)?;
    }
    let summary = ExportSummary {
        collections: decimal(&m.collections)?,
        documents: decimal(&m.documents)?,
        document_bytes: decimal(&m.document_bytes)?,
    };
    Ok((m, summary))
}
fn hash(file: &mut File) -> Result<(u64, String)> {
    file.seek(SeekFrom::Start(0)).map_err(|_| Fault::Io)?;
    let mut sha = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|_| Fault::Io)?;
        if n == 0 {
            break;
        }
        bytes = bytes.checked_add(n as u64).ok_or(Fault::Invalid)?;
        sha.update(&buffer[..n]);
    }
    let mut hex = String::with_capacity(64);
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for byte in sha.finalize() {
        hex.push(DIGITS[(byte >> 4) as usize] as char);
        hex.push(DIGITS[(byte & 15) as usize] as char);
    }
    Ok((bytes, hex))
}
fn new_file(parent: &File, name: &str) -> Result<File> {
    Ok(File::from(
        unix::openat(
            parent,
            name,
            OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| Fault::Io)?,
    ))
}
fn destination(path: &Path) -> Result<(PathBuf, File, &OsStr)> {
    let name = path
        .file_name()
        .filter(|_| {
            matches!(
                path.components().next_back(),
                Some(std::path::Component::Normal(_))
            )
        })
        .ok_or(Fault::Invalid)?;
    let parent = fs::canonicalize(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .map_err(|_| Fault::Io)?;
    let file = File::from(
        unix::open(&parent, READ | OFlags::DIRECTORY, Mode::empty()).map_err(|_| Fault::Invalid)?,
    );
    let m = file.metadata().map_err(|_| Fault::Io)?;
    if m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o022 != 0 {
        return Err(Fault::Invalid);
    }
    match fs::symlink_metadata(parent.join(name)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(Fault::Invalid),
    }
    Ok((parent, file, name))
}
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    target_os = "ios"
))]
fn rename(parent: &File, source: &OsStr, target: &OsStr) -> Result<()> {
    unix::renameat_with(parent, source, parent, target, unix::RenameFlags::NOREPLACE)
        .map_err(|_| Fault::Io)
}
#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    target_os = "ios"
)))]
fn rename(_: &File, _: &OsStr, _: &OsStr) -> Result<()> {
    Err(Fault::Io)
}

fn publish(
    parent: &File,
    source: &OsStr,
    target: &OsStr,
    mut point: impl FnMut(Point) -> Result<()>,
) -> Result<()> {
    point(Point::BeforePublish)?;
    rename(parent, source, target)?;
    point(Point::Published).map_err(|_| Fault::PublicationUncertain)?;
    parent.sync_all().map_err(|_| Fault::PublicationUncertain)
}
fn backup(
    read: &redb::ReadTransaction,
    output: &Path,
    mut point: impl FnMut(Point) -> Result<()>,
) -> Result<ExportSummary> {
    let (parent_path, parent, name) = destination(output)?;
    let stage = tempfile::Builder::new()
        .prefix(".candidate-backup-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&parent_path)
        .map_err(|_| Fault::Io)?;
    let dir = directory(stage.path())?;
    let summary = super::backup::export(read, &stage.path().join("store"), |_| Ok(()))
        .map_err(|_| Fault::Storage)?;
    point(Point::Copied)?;
    let store = child(&dir, "store", true)?;
    let mut records = Vec::new();
    for (name, path) in ["format.json", "data.redb"].into_iter().zip(PATHS) {
        let (bytes, sha256) = hash(&mut child(&store, name, false)?)?;
        records.push(Record {
            path: path.into(),
            bytes: bytes.to_string(),
            sha256,
        });
    }
    let m = Manifest {
        magic: "kiarni-backup".into(),
        version: 1,
        format_version: 2,
        engine: "redb-4.5.0".into(),
        collections: summary.collections.to_string(),
        documents: summary.documents.to_string(),
        document_bytes: summary.document_bytes.to_string(),
        files: records.try_into().map_err(|_| Fault::Invalid)?,
    };
    let bytes = serde_json::to_vec(&m).map_err(|_| Fault::Invalid)?;
    decode(&bytes)?;
    let mut file = new_file(&dir, "backup.json")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| Fault::Io)?;
    dir.sync_all().map_err(|_| Fault::Io)?;
    point(Point::Verified)?;
    let stage = stage.keep();
    publish(
        &parent,
        stage.file_name().ok_or(Fault::Invalid)?,
        name,
        point,
    )?;
    Ok(summary)
}

fn restore_store(
    source: &Path,
    output: &Path,
    mut point: impl FnMut(Point) -> Result<()>,
) -> Result<ExportSummary> {
    let source_dir = directory(source)?;
    entries(&source_dir, &["backup.json", "store"])?;
    let store = child(&source_dir, "store", true)?;
    entries(&store, &["format.json", "data.redb"])?;
    let mut bytes = Vec::new();
    child(&source_dir, "backup.json", false)?
        .take(1025)
        .read_to_end(&mut bytes)
        .map_err(|_| Fault::Io)?;
    let (manifest, expected) = decode(&bytes)?;
    let (parent_path, parent, name) = destination(output)?;
    if parent_path.starts_with(fs::canonicalize(source).map_err(|_| Fault::Io)?) {
        return Err(Fault::Invalid);
    }
    let stage = tempfile::Builder::new()
        .prefix(".candidate-restore-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&parent_path)
        .map_err(|_| Fault::Io)?;
    let input = stage.path().join("input");
    let mut builder = fs::DirBuilder::new();
    use std::os::unix::fs::DirBuilderExt;
    builder.mode(0o700).create(&input).map_err(|_| Fault::Io)?;
    let target = directory(&input)?;
    for (record, name) in manifest.files.iter().zip(["format.json", "data.redb"]) {
        let expected_size = decimal(&record.bytes)?;
        let mut source = child(&store, name, false)?;
        if source.metadata().map_err(|_| Fault::Io)?.len() != expected_size {
            return Err(Fault::Invalid);
        }
        let mut out = new_file(&target, name)?;
        let mut remaining = expected_size;
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            let limit = remaining.min(buffer.len() as u64) as usize;
            let n = source.read(&mut buffer[..limit]).map_err(|_| Fault::Io)?;
            if n == 0 {
                return Err(Fault::Invalid);
            }
            out.write_all(&buffer[..n]).map_err(|_| Fault::Io)?;
            remaining -= n as u64;
        }
        if source.read(&mut [0u8; 1]).map_err(|_| Fault::Io)? != 0 {
            return Err(Fault::Invalid);
        }
        out.sync_all().map_err(|_| Fault::Io)?;
        if hash(&mut out)? != (expected_size, record.sha256.clone()) {
            return Err(Fault::Invalid);
        }
    }
    target.sync_all().map_err(|_| Fault::Io)?;
    point(Point::Copied)?;
    let restored = stage.path().join("restored");
    let actual =
        super::backup::restore(&input, &restored, |_| Ok(())).map_err(|_| Fault::Storage)?;
    if actual != expected {
        return Err(Fault::Invalid);
    }
    point(Point::Verified)?;
    let relative = stage
        .path()
        .file_name()
        .ok_or(Fault::Invalid)?
        .to_os_string();
    publish(
        &parent,
        Path::new(&relative).join("restored").as_os_str(),
        name,
        point,
    )?;
    Ok(actual)
}

mod tests;
