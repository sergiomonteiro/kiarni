//! Unix immutable snapshots. No active pointer or atomic multi-file transaction.
use crate::{Error, Keyset, MAX_CONFIG_BYTES, SecretToken};
use rustix::fs::{self, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Permissions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};
use subtle::ConstantTimeEq;

pub struct Vault {
    pub(super) directory: File,
}

#[derive(Debug)]
pub struct Snapshot {
    pub epoch: u64,
    pub verifier_file: String,
    pub token_file: Option<String>,
}

fn file_error(error: rustix::io::Errno) -> Error {
    if error == rustix::io::Errno::EXIST {
        Error::AlreadyExists
    } else {
        Error::FileAccess
    }
}

pub(super) fn validate_file(file: &File) -> Result<(), Error> {
    let metadata = file.metadata().map_err(|_| Error::FileAccess)?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(Error::InsecurePath);
    }
    Ok(())
}

impl Vault {
    /// Existing directory, final component must not be a symlink.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let directory = File::from(
            fs::open(
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(file_error)?,
        );
        let metadata = directory.metadata().map_err(|_| Error::FileAccess)?;
        if !metadata.is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o7777 != 0o700
        {
            return Err(Error::InsecurePath);
        }
        Ok(Self { directory })
    }

    pub(super) fn write_new(&self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        let mut file = File::from(
            fs::openat(
                &self.directory,
                name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(file_error)?,
        );
        // Restrictive umasks may remove owner access; never change global umask.
        file.set_permissions(Permissions::from_mode(0o600))
            .map_err(|_| Error::FileAccess)?;
        validate_file(&file)?;
        file.write_all(bytes).map_err(|_| Error::FileAccess)?;
        file.sync_all().map_err(|_| Error::FileAccess)
    }

    pub fn write_snapshot(&self, keyset: &Keyset) -> Result<Snapshot, Error> {
        let verifier_file = format!("auth-{}.json", keyset.epoch());
        let bytes = keyset.to_json()?;
        self.write_new(&verifier_file, &bytes)?;
        self.directory.sync_all().map_err(|_| Error::FileAccess)?;
        Ok(Snapshot {
            epoch: keyset.epoch(),
            verifier_file,
            token_file: None,
        })
    }

    /// Deliver the plaintext to a separate private client directory. The server
    /// directory receives only the verifier. No active configuration is changed.
    pub fn provision(
        &self,
        keyset: &Keyset,
        token: &SecretToken,
        client: &Vault,
    ) -> Result<Snapshot, Error> {
        let server_metadata = self.directory.metadata().map_err(|_| Error::FileAccess)?;
        let client_metadata = client.directory.metadata().map_err(|_| Error::FileAccess)?;
        if server_metadata.dev() == client_metadata.dev()
            && server_metadata.ino() == client_metadata.ino()
        {
            return Err(Error::InsecurePath);
        }
        let key = keyset.schema.keys.last().ok_or(Error::InvalidConfig)?;
        let expected = crate::unhex::<32>(&key.sha256)?;
        let actual: [u8; 32] = Sha256::digest(token.0.as_bytes()).into();
        if !bool::from(expected.ct_eq(&actual)) {
            return Err(Error::InvalidConfig);
        }
        let name = format!("token-{}.txt", key.id);
        client.write_new(&name, token.0.as_bytes())?;
        client.directory.sync_all().map_err(|_| Error::FileAccess)?;
        // Failure may leave an orphaned client file or partial snapshot. Never
        // activate by filename scan; only load an explicitly chosen valid epoch.
        let mut snapshot = self.write_snapshot(keyset)?;
        snapshot.token_file = Some(name);
        Ok(snapshot)
    }

    pub fn load(&self, epoch: u64) -> Result<Keyset, Error> {
        let name = format!("auth-{epoch}.json");
        let file = File::from(
            fs::openat(
                &self.directory,
                name.as_str(),
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(file_error)?,
        );
        validate_file(&file)?;
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::FileAccess)?;
        let keyset = Keyset::parse(&bytes)?;
        if keyset.epoch() != epoch {
            return Err(Error::InvalidConfig);
        }
        Ok(keyset)
    }
}
