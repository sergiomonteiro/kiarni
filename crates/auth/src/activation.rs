//! Durable local authentication activation journal on Unix.
use crate::{
    Authority, Error, Keyset, MAX_CONFIG_BYTES, MAX_LIFETIME, SecretToken,
    vault::{Vault, validate_file},
};
use rustix::fs::{self as unix, FlockOperation, Mode, OFlags};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, DirBuilder, File},
    io::Read,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
};

const META_LIMIT: usize = 1024;
pub const DEFAULT_LIFETIME: u64 = MAX_LIFETIME;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checkpoint {
    BootstrapMarked,
    TokenDelivered,
    SnapshotDurable,
    BeforeIntentRename,
    IntentRenamed,
    IntentDurable,
    InitializedDurable,
    BeforeActiveRename,
    ActiveRenamed,
    ActiveDurable,
    Complete,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pointer {
    version: u8,
    epoch: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    version: u8,
    previous: Option<Pointer>,
    next: Pointer,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u8,
    initialized: bool,
}

pub struct Startup {
    pub keyset: Keyset,
    pub created: bool,
    pub recovered: bool,
    pub token_file: Option<PathBuf>,
}

pub struct Store {
    server: Vault,
    client: Vault,
    auth_path: PathBuf,
    client_path: PathBuf,
}

struct WriterLock(File);
impl Drop for WriterLock {
    fn drop(&mut self) {
        // A concurrent fork may inherit the open description before exec closes
        // CLOEXEC descriptors. Explicit unlock ends the operation's ownership
        // even while such an inherited descriptor is briefly alive.
        let _ = unix::flock(&self.0, FlockOperation::Unlock);
    }
}

fn private_directory(path: &Path) -> Result<PathBuf, Error> {
    if !path.exists() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|_| Error::FileAccess)?;
    }
    let _ = Vault::open(path)?; // Existing insecure directories are never chmod'd.
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|_| Error::FileAccess)?;
    path.canonicalize().map_err(|_| Error::FileAccess)
}

impl Store {
    pub fn open(data: &Path, delivery: Option<&Path>) -> Result<Self, Error> {
        let basename = data.file_name().ok_or(Error::InvalidConfig)?;
        let mut sibling = basename.to_os_string();
        sibling.push(".credentials");
        let default_delivery = data.with_file_name(sibling);
        let data = private_directory(data)?;
        let client_path = private_directory(delivery.unwrap_or(&default_delivery))?;
        if data.starts_with(&client_path) || client_path.starts_with(&data) {
            return Err(Error::InsecurePath);
        }
        let auth_path = private_directory(&data.join("auth"))?;
        let server = Vault::open(&auth_path)?;
        let client = Vault::open(&client_path)?;
        let a = server.directory.metadata().map_err(|_| Error::FileAccess)?;
        let b = client.directory.metadata().map_err(|_| Error::FileAccess)?;
        use std::os::unix::fs::MetadataExt;
        if a.dev() == b.dev() && a.ino() == b.ino() {
            return Err(Error::InsecurePath);
        }
        Ok(Self {
            server,
            client,
            auth_path,
            client_path,
        })
    }

    fn lock(&self) -> Result<WriterLock, Error> {
        let flags = OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
        let file = match unix::openat(
            &self.server.directory,
            "writer.lock",
            flags | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => {
                let file = File::from(fd);
                file.set_permissions(fs::Permissions::from_mode(0o600))
                    .map_err(|_| Error::FileAccess)?;
                file
            }
            Err(rustix::io::Errno::EXIST) => File::from(
                unix::openat(&self.server.directory, "writer.lock", flags, Mode::empty())
                    .map_err(|_| Error::FileAccess)?,
            ),
            Err(_) => return Err(Error::FileAccess),
        };
        validate_file(&file)?;
        unix::flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
            if error == rustix::io::Errno::WOULDBLOCK {
                Error::Busy
            } else {
                Error::FileAccess
            }
        })?;
        Ok(WriterLock(file))
    }

    fn read(&self, name: &str, limit: usize) -> Result<Option<Vec<u8>>, Error> {
        let fd = match unix::openat(
            &self.server.directory,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(_) => return Err(Error::InsecurePath),
        };
        let file = File::from(fd);
        validate_file(&file)?;
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::FileAccess)?;
        if bytes.len() > limit {
            return Err(Error::InvalidConfig);
        }
        Ok(Some(bytes))
    }

    fn metadata<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, Error> {
        self.read(name, META_LIMIT)?
            .map(|bytes| {
                kiarni_core::DocumentInput::parse(&bytes, META_LIMIT)
                    .map_err(|_| Error::InvalidConfig)?;
                serde_json::from_slice(&bytes).map_err(|_| Error::InvalidConfig)
            })
            .transpose()
    }

    fn replace(
        &self,
        name: &str,
        bytes: &[u8],
        before: Option<Checkpoint>,
        after: Option<Checkpoint>,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<(), Error> {
        // Validate destination type/mode before replacement; no symlink overwrite.
        let _ = self.read(name, MAX_CONFIG_BYTES)?;
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| Error::EntropyUnavailable)?;
        let temp = format!("pending-{}.tmp", crate::hex_string(&nonce));
        self.server.write_new(&temp, bytes)?;
        if let Some(point) = before {
            hook(point)?;
        }
        unix::renameat(
            &self.server.directory,
            temp.as_str(),
            &self.server.directory,
            name,
        )
        .map_err(|_| Error::FileAccess)?;
        if let Some(point) = after {
            hook(point).map_err(|_| Error::CommitUncertain)?;
        }
        self.server
            .directory
            .sync_all()
            .map_err(|_| Error::CommitUncertain)
    }

    fn pointer(
        &self,
        keyset: &Keyset,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<Pointer, Error> {
        let bytes = keyset.to_json()?;
        let pointer = Pointer {
            version: 1,
            epoch: keyset.epoch(),
            sha256: crate::hex_string(&Sha256::digest(&bytes)),
        };
        let name = format!("state-{}.json", pointer.sha256);
        if let Some(existing) = self.read(&name, MAX_CONFIG_BYTES)? {
            if existing != bytes {
                return Err(Error::InvalidConfig);
            }
        } else {
            self.replace(&name, &bytes, None, None, hook)?;
        }
        hook(Checkpoint::SnapshotDurable)?;
        Ok(pointer)
    }

    fn load_pointer(&self, pointer: &Pointer) -> Result<Keyset, Error> {
        if pointer.version != 1 || pointer.epoch == 0 {
            return Err(Error::InvalidConfig);
        }
        crate::unhex::<32>(&pointer.sha256)?;
        let bytes = self
            .read(&format!("state-{}.json", pointer.sha256), MAX_CONFIG_BYTES)?
            .ok_or(Error::InvalidConfig)?;
        if crate::hex_string(&Sha256::digest(&bytes)) != pointer.sha256 {
            return Err(Error::InvalidConfig);
        }
        let keyset = Keyset::parse(&bytes)?;
        if keyset.epoch() != pointer.epoch {
            return Err(Error::InvalidConfig);
        }
        Ok(keyset)
    }

    fn finish(
        &self,
        pointer: &Pointer,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let marker = serde_json::to_vec(&Marker {
            version: 1,
            initialized: true,
        })
        .map_err(|_| Error::InvalidConfig)?;
        self.replace("marker.json", &marker, None, None, hook)?;
        hook(Checkpoint::InitializedDurable).map_err(|_| Error::CommitUncertain)?;
        self.replace(
            "active.json",
            &serde_json::to_vec(pointer).map_err(|_| Error::InvalidConfig)?,
            Some(Checkpoint::BeforeActiveRename),
            Some(Checkpoint::ActiveRenamed),
            hook,
        )
        .map_err(|_| Error::CommitUncertain)?;
        hook(Checkpoint::ActiveDurable).map_err(|_| Error::CommitUncertain)
    }

    fn current(
        &self,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<Option<Startup>, Error> {
        let marker: Option<Marker> = self.metadata("marker.json")?;
        let intent: Option<Intent> = self.metadata("intent.json")?;
        let active: Option<Pointer> = self.metadata("active.json")?;
        let Some(marker) = marker else {
            if intent.is_some() || active.is_some() {
                return Err(Error::BootstrapIncomplete);
            }
            // Only the lock may predate the initial marker. Never infer state
            // from snapshot names or reset an unknown directory.
            for entry in fs::read_dir(&self.auth_path).map_err(|_| Error::FileAccess)? {
                if entry.map_err(|_| Error::FileAccess)?.file_name() != "writer.lock" {
                    return Err(Error::BootstrapIncomplete);
                }
            }
            return Ok(None);
        };
        if marker.version != 1 {
            return Err(Error::InvalidConfig);
        }
        let Some(intent) = intent else {
            if marker.initialized || active.is_some() {
                return Err(Error::BootstrapIncomplete);
            }
            return Ok(None); // interrupted, never-activated initial attempt
        };
        let expected_epoch = match &intent.previous {
            Some(previous) => previous.epoch.checked_add(1).ok_or(Error::InvalidConfig)?,
            None => 1,
        };
        if intent.version != 1 || expected_epoch != intent.next.epoch {
            return Err(Error::InvalidConfig);
        }
        if let Some(previous) = &intent.previous {
            self.load_pointer(previous)?;
        }
        let keyset = self.load_pointer(&intent.next)?;
        if active.as_ref() == Some(&intent.next) {
            if !marker.initialized {
                return Err(Error::InvalidConfig);
            }
            return Ok(Some(Startup {
                keyset,
                created: false,
                recovered: false,
                token_file: None,
            }));
        }
        if active.is_some() && active != intent.previous {
            return Err(Error::EpochRollback);
        }
        self.finish(&intent.next, hook)
            .map_err(|_| Error::CommitUncertain)?;
        Ok(Some(Startup {
            keyset,
            created: false,
            recovered: true,
            token_file: None,
        }))
    }

    fn publish(
        &self,
        previous: Option<Pointer>,
        keyset: &Keyset,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let next = self.pointer(keyset, hook)?;
        let bytes = serde_json::to_vec(&Intent {
            version: 1,
            previous,
            next: next.clone(),
        })
        .map_err(|_| Error::InvalidConfig)?;
        self.replace(
            "intent.json",
            &bytes,
            Some(Checkpoint::BeforeIntentRename),
            Some(Checkpoint::IntentRenamed),
            hook,
        )?;
        hook(Checkpoint::IntentDurable).map_err(|_| Error::CommitUncertain)?;
        self.finish(&next, hook)
            .map_err(|_| Error::CommitUncertain)?;
        hook(Checkpoint::Complete).map_err(|_| Error::CommitUncertain)
    }

    fn deliver(
        &self,
        keyset: &Keyset,
        token: &SecretToken,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<PathBuf, Error> {
        let key = keyset.schema.keys.last().ok_or(Error::InvalidConfig)?;
        let name = format!("token-{}.txt", key.id);
        self.client.write_new(&name, token.0.as_bytes())?;
        self.client
            .directory
            .sync_all()
            .map_err(|_| Error::FileAccess)?;
        hook(Checkpoint::TokenDelivered)?;
        Ok(self.client_path.join(name))
    }

    pub fn bootstrap_or_load(&self, now: u64) -> Result<Startup, Error> {
        self.bootstrap_with_hook(now, &mut |_| Ok(()))
    }

    /// Persist the initial marker before the server authorizes token delivery.
    pub fn prepare_initial_marker(&self) -> Result<(), Error> {
        let _lock = self.lock()?;
        if self.metadata::<Intent>("intent.json")?.is_some()
            || self.metadata::<Pointer>("active.json")?.is_some()
            || self.current(&mut |_| Ok(()))?.is_some()
        {
            return Err(Error::InvalidConfig);
        }
        if self.metadata::<Marker>("marker.json")?.is_none() {
            self.write_initial_marker(&mut |_| Ok(()))?;
        }
        Ok(())
    }

    /// Continue the server journal. A missing marker cannot authorize issuance.
    pub fn resume_prepared(&self, now: u64, initialized: bool) -> Result<Startup, Error> {
        let _lock = self.lock()?;
        let marker: Marker = self
            .metadata("marker.json")?
            .ok_or(Error::BootstrapIncomplete)?;
        if initialized && !marker.initialized {
            return Err(Error::BootstrapIncomplete);
        }
        if let Some(startup) = self.current(&mut |_| Ok(()))? {
            return Ok(startup);
        }
        self.issue_initial(now, &mut |_| Ok(()))
    }

    /// Fault injection for the disposable spike/worker, never an HTTP callback.
    pub fn bootstrap_with_hook(
        &self,
        now: u64,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<Startup, Error> {
        let _lock = self.lock()?;
        if let Some(startup) = self.current(hook)? {
            return Ok(startup);
        }
        self.write_initial_marker(hook)?;
        self.issue_initial(now, hook)
    }

    fn write_initial_marker(
        &self,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.replace(
            "marker.json",
            &serde_json::to_vec(&Marker {
                version: 1,
                initialized: false,
            })
            .map_err(|_| Error::InvalidConfig)?,
            None,
            None,
            hook,
        )?;
        hook(Checkpoint::BootstrapMarked)
    }

    fn issue_initial(
        &self,
        now: u64,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<Startup, Error> {
        let (keyset, token) = Keyset::bootstrap(now, DEFAULT_LIFETIME)?;
        let token_file = self.deliver(&keyset, &token, hook)?;
        self.publish(None, &keyset, hook)?;
        Ok(Startup {
            keyset,
            created: true,
            recovered: false,
            token_file: Some(token_file),
        })
    }

    pub fn rotate(&self, now: u64, lifetime: u64, overlap: u64) -> Result<Startup, Error> {
        self.update_with_hook(Some((now, lifetime, overlap)), &mut |_| Ok(()))
    }
    pub fn revoke_all(&self) -> Result<Startup, Error> {
        self.update_with_hook(None, &mut |_| Ok(()))
    }

    pub fn update_with_hook(
        &self,
        rotation: Option<(u64, u64, u64)>,
        hook: &mut impl FnMut(Checkpoint) -> Result<(), Error>,
    ) -> Result<Startup, Error> {
        let _lock = self.lock()?;
        let current = self.current(hook)?.ok_or(Error::BootstrapIncomplete)?;
        let previous = self
            .metadata::<Pointer>("active.json")?
            .ok_or(Error::BootstrapIncomplete)?;
        let (keyset, token_file) = if let Some((now, lifetime, overlap)) = rotation {
            let (next, token) = current.keyset.rotate(now, lifetime, overlap)?;
            let path = self.deliver(&next, &token, hook)?;
            (next, Some(path))
        } else {
            (current.keyset.revoke_all()?, None)
        };
        self.publish(Some(previous), &keyset, hook)?;
        Ok(Startup {
            keyset,
            created: false,
            recovered: false,
            token_file,
        })
    }

    pub fn refresh(&self, authority: &Authority, now: u64) -> Result<(), Error> {
        let result = (|| {
            let _lock = self.lock()?;
            self.current(&mut |_| Ok(()))?
                .map(|s| s.keyset)
                .ok_or(Error::BootstrapIncomplete)
        })();
        authority.reload(result, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_unlock_does_not_wait_for_an_inherited_open_description_to_close() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = Store::open(&root.path().join("data"), None).unwrap();
        let owner = store.lock().unwrap();
        let inherited = owner.0.try_clone().unwrap();
        assert!(matches!(store.lock(), Err(Error::Busy)));
        drop(owner);
        let next = store.lock().unwrap();
        drop(next);
        drop(inherited);
    }
}
