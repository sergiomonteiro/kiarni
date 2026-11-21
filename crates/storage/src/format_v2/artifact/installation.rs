//! Test-only restore of a format-2 installation skeleton. Never activates auth.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    Prepared,
    DeliveryPublished,
    BeforeRoot,
    RootPublished,
}

fn mkdir(parent: &File, name: &str) -> Result<File> {
    unix::mkdirat(parent, name, Mode::RUSR | Mode::WUSR | Mode::XUSR).map_err(|_| Fault::Io)?;
    child(parent, name, true)
}

fn restore_installation(
    source: &Path,
    output: &Path,
    mut point: impl FnMut(Point) -> Result<()>,
) -> Result<ExportSummary> {
    let (parent_path, parent, name) = destination(output)?;
    if parent_path.starts_with(fs::canonicalize(source).map_err(|_| Fault::Io)?) {
        return Err(Fault::Invalid);
    }
    let mut delivery_name = name.to_os_string();
    delivery_name.push(".credentials");
    match fs::symlink_metadata(parent_path.join(&delivery_name)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(Fault::Invalid),
    }
    let stage = tempfile::Builder::new()
        .prefix(".candidate-install-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&parent_path)
        .map_err(|_| Fault::Io)?;
    let staging = directory(stage.path())?;
    let root = mkdir(&staging, "root")?;
    let lock = new_file(&root, "serve.lock")?;
    private(&lock, false)?;
    unix::flock(&lock, unix::FlockOperation::NonBlockingLockExclusive).map_err(|_| Fault::Io)?;
    lock.sync_all().map_err(|_| Fault::Io)?;
    // Root lock precedes opening the copied engine. Hash/structural validation
    // and reexport are the same operations already tested for the store envelope.
    let summary = restore_store(source, &stage.path().join("root/store"), |_| Ok(()))?;
    entries(&root, &["serve.lock", "store"])?;
    let mut record = new_file(&root, "bootstrap.json")?;
    record
        .write_all(br#"{"version":1,"phase":"store_ready"}"#)
        .and_then(|_| record.sync_all())
        .map_err(|_| Fault::Io)?;
    root.sync_all().map_err(|_| Fault::Io)?;
    let delivery = mkdir(&staging, "delivery")?;
    delivery.sync_all().map_err(|_| Fault::Io)?;
    staging.sync_all().map_err(|_| Fault::Io)?;
    point(Point::Prepared)?;
    // From the first publication onward preserve staging on errors, and never
    // remove either final name. The two directories cannot be published at once.
    let stage = stage.keep();
    let relative = Path::new(stage.file_name().ok_or(Fault::Invalid)?);
    rename(
        &parent,
        relative.join("delivery").as_os_str(),
        &delivery_name,
    )?;
    parent.sync_all().map_err(|_| Fault::PublicationUncertain)?;
    point(Point::DeliveryPublished).map_err(|_| Fault::PublicationUncertain)?;
    point(Point::BeforeRoot).map_err(|_| Fault::PublicationUncertain)?;
    rename(&parent, relative.join("root").as_os_str(), name)
        .map_err(|_| Fault::PublicationUncertain)?;
    point(Point::RootPublished).map_err(|_| Fault::PublicationUncertain)?;
    parent.sync_all().map_err(|_| Fault::PublicationUncertain)?;
    // Success leaves no intermediate input copies or empty staging directory.
    fs::remove_dir(&stage).map_err(|_| Fault::PublicationUncertain)?;
    parent.sync_all().map_err(|_| Fault::PublicationUncertain)?;
    drop(lock);
    Ok(summary)
}

mod tests;
