use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};

use kiarni_auth::activation::{Startup, Store as Auth};
use kiarni_storage::Store;
use rustix::fs::{self as unix, FlockOperation, Mode, OFlags};
use serde::{Deserialize, Serialize};

const RECORD: &str = "bootstrap.json";
const STAGING: &str = ".bootstrap.tmp";
const LOCK: &str = "serve.lock";
const PRIVATE: Mode = Mode::RUSR.union(Mode::WUSR);
const SAFE_OPEN: OFlags = OFlags::NOFOLLOW
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    InitializingStore,
    StoreReady,
    InitializingAuth,
    Initialized,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    phase: Phase,
}

pub struct Root {
    path: PathBuf,
    directory: File,
    lock: File,
}

fn private_file(file: &File) -> Result<()> {
    let metadata = file.metadata().map_err(|_| "data_file_unavailable")?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o600
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err("insecure_data_file".into());
    }
    Ok(())
}

fn private_directory(path: &Path) -> Result<File> {
    let file = File::from(
        unix::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | SAFE_OPEN,
            Mode::empty(),
        )
        .map_err(|_| "data_directory_unavailable")?,
    );
    let metadata = file.metadata().map_err(|_| "data_directory_unavailable")?;
    if metadata.mode() & 0o7777 != 0o700 || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err("insecure_data_directory".into());
    }
    Ok(file)
}

impl Root {
    pub fn lock(path: &Path) -> Result<Self> {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {
                let parent = path
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                File::open(parent)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| "data_directory_sync_failed")?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("data_directory_unavailable".into()),
        }
        Self::lock_existing(path)
    }

    pub fn lock_existing(path: &Path) -> Result<Self> {
        let directory = private_directory(path)?;
        let lock = File::from(
            unix::openat(
                &directory,
                LOCK,
                OFlags::CREATE | OFlags::RDWR | SAFE_OPEN,
                PRIVATE,
            )
            .map_err(|_| "data_lock_unavailable")?,
        );
        private_file(&lock)?;
        unix::flock(&lock, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
            if error == rustix::io::Errno::WOULDBLOCK {
                "data_directory_locked"
            } else {
                "data_lock_unavailable"
            }
        })?;
        Ok(Self {
            path: path.into(),
            directory,
            lock,
        })
    }

    fn phase(&self) -> Result<Option<Phase>> {
        let file = match unix::openat(
            &self.directory,
            RECORD,
            OFlags::RDONLY | SAFE_OPEN,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(_) => return Err("bootstrap_record_unavailable".into()),
        };
        private_file(&file)?;
        let mut bytes = Vec::new();
        file.take(1025)
            .read_to_end(&mut bytes)
            .map_err(|_| "bootstrap_record_unavailable")?;
        if bytes.len() > 1024 || bytes.first() != Some(&b'{') {
            return Err("invalid_bootstrap_record".into());
        }
        let record: Record =
            serde_json::from_slice(&bytes).map_err(|_| "invalid_bootstrap_record")?;
        if record.version != 1 {
            return Err("unsupported_bootstrap_record".into());
        }
        Ok(Some(record.phase))
    }

    fn publish(&self, phase: Phase) -> Result<()> {
        let mut file = File::from(
            unix::openat(
                &self.directory,
                STAGING,
                OFlags::CREATE | OFlags::WRONLY | SAFE_OPEN,
                PRIVATE,
            )
            .map_err(|_| "bootstrap_stage_unavailable")?,
        );
        private_file(&file)?;
        let bytes = serde_json::to_vec(&Record { version: 1, phase })?;
        file.set_len(0)
            .and_then(|()| file.write_all(&bytes))
            .and_then(|()| file.sync_all())
            .map_err(|_| "bootstrap_stage_write_failed")?;
        unix::renameat(&self.directory, STAGING, &self.directory, RECORD)
            .map_err(|_| "bootstrap_publish_failed")?;
        self.directory
            .sync_all()
            .map_err(|_| "bootstrap_sync_failed")?;
        Ok(())
    }

    pub fn start(&self, now: u64) -> Result<(Store, Startup)> {
        let mut phase = match self.phase()? {
            Some(phase) => phase,
            None => {
                for entry in fs::read_dir(&self.path).map_err(|_| "data_directory_unavailable")? {
                    let name = entry.map_err(|_| "data_directory_unavailable")?.file_name();
                    if name != LOCK && name != STAGING {
                        return Err("unrecognized_data_directory".into());
                    }
                }
                self.publish(Phase::InitializingStore)?;
                Phase::InitializingStore
            }
        };
        let path = self.path.join("store");
        let store = match fs::symlink_metadata(&path) {
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && phase == Phase::InitializingStore =>
            {
                Store::create(&path)?
            }
            Ok(_) => {
                let directory = private_directory(&path)?;
                for name in ["format.json", "data.redb"] {
                    let file = File::from(
                        unix::openat(&directory, name, OFlags::RDONLY | SAFE_OPEN, Mode::empty())
                            .map_err(|_| "incomplete_store")?,
                    );
                    private_file(&file)?;
                }
                Store::open(&path)?
            }
            Err(_) => return Err("missing_store".into()),
        };
        if phase == Phase::InitializingStore {
            self.publish(Phase::StoreReady)?;
            phase = Phase::StoreReady;
        }
        if phase != Phase::StoreReady {
            private_directory(&self.path.join("auth"))?;
        }
        let auth = Auth::open(&self.path, None)?;
        if phase == Phase::StoreReady {
            auth.prepare_initial_marker()?;
            self.publish(Phase::InitializingAuth)?;
            phase = Phase::InitializingAuth;
        }
        let startup = auth.resume_prepared(now, phase == Phase::Initialized)?;
        if phase != Phase::Initialized {
            self.publish(Phase::Initialized)?;
        }
        Ok((store, startup))
    }

    pub fn update_auth(&self, now: u64, rotate: bool) -> Result<Startup> {
        if self.phase()? != Some(Phase::Initialized) {
            return Err("installation_not_initialized".into());
        }
        // Validate the existing store and auth before issuing or revoking.
        let (_store, _) = self.start(now)?;
        let auth = Auth::open(&self.path, None)?;
        Ok(if rotate {
            auth.rotate(now, kiarni_auth::activation::DEFAULT_LIFETIME, 0)?
        } else {
            auth.revoke_all()?
        })
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = unix::flock(&self.lock, FlockOperation::Unlock);
    }
}
