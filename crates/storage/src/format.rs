//! Shared manifest codec. Recognition of a version does not authorize its use.
use super::{Error, io};
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path};

const MAX_MANIFEST: u64 = 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Version {
    Legacy,
    Indexed,
}
impl Version {
    fn number(self) -> u64 {
        match self {
            Self::Legacy => 1,
            Self::Indexed => 2,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    magic: String,
    version: u64,
    engine: String,
}

pub(super) fn encode(version: Version) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(&Manifest {
        magic: "kiarni".into(),
        version: version.number(),
        engine: "redb-4.5.0".into(),
    })
    .map_err(|_| Error::InvalidManifest)
}

fn decode(bytes: &[u8]) -> Result<Version, Error> {
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err(Error::InvalidManifest);
    }
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| Error::InvalidManifest)?;
    if manifest.magic != "kiarni" || manifest.engine != "redb-4.5.0" {
        return Err(Error::UnsupportedFormat);
    }
    match manifest.version {
        1 => Ok(Version::Legacy),
        2 => Ok(Version::Indexed),
        _ => Err(Error::UnsupportedFormat),
    }
}

/// Check expected format before inspecting data.redb or opening/recovering redb.
/// Callers supply their supported version; there is no migration or default fallback.
pub(super) fn preflight(root: &Path, expected: Version) -> Result<(), Error> {
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
    if decode(&bytes)? != expected {
        return Err(Error::UnsupportedFormat);
    }
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
