//! Current on-disk manifest. Incompatible development layouts are refused.
use super::{Error, io};
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path};

const MAX_MANIFEST: u64 = 1024;
pub(crate) const CURRENT_VERSION: u64 = 2;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    magic: String,
    version: u64,
    engine: String,
}

pub(super) fn encode() -> Result<Vec<u8>, Error> {
    serde_json::to_vec(&Manifest {
        magic: "kiarni".into(),
        version: CURRENT_VERSION,
        engine: "redb-4.5.0".into(),
    })
    .map_err(|_| Error::InvalidManifest)
}

fn decode(bytes: &[u8]) -> Result<(), Error> {
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err(Error::InvalidManifest);
    }
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| Error::InvalidManifest)?;
    if manifest.magic != "kiarni" || manifest.engine != "redb-4.5.0" {
        return Err(Error::UnsupportedFormat);
    }
    if manifest.version != CURRENT_VERSION {
        return Err(Error::UnsupportedFormat);
    }
    Ok(())
}

/// Check expected format before inspecting data.redb or opening/recovering redb.
/// Only the current version is supported; there is no migration or fallback.
pub(super) fn preflight(root: &Path) -> Result<(), Error> {
    if !io(fs::symlink_metadata(root))?.file_type().is_dir() {
        return Err(Error::InvalidLayout);
    }
    let path = root.join("format.json");
    if !io(fs::symlink_metadata(&path))?.file_type().is_file() {
        return Err(Error::InvalidManifest);
    }
    let mut bytes = Vec::new();
    io(io(fs::File::open(path))?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes))?;
    decode(&bytes)?;
    if !io(fs::symlink_metadata(root.join("data.redb")))?
        .file_type()
        .is_file()
    {
        return Err(Error::InvalidLayout);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
