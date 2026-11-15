//! Bounded authenticated loopback probe. Never opens or modifies the store.
use kiarni_config::Config;
use rustix::fs::{self as unix, Mode, OFlags};
use std::{
    fs::{self, File},
    io::{Read, Write},
    net::TcpStream,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK)
    .union(OFlags::CLOEXEC);
const FAILURE: &str = "healthcheck_failed";

fn private(file: &File, directory: bool) -> Result<(), &'static str> {
    let meta = file.metadata().map_err(|_| FAILURE)?;
    if meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o7777 != if directory { 0o700 } else { 0o600 }
        || if directory {
            !meta.is_dir()
        } else {
            !meta.is_file() || meta.nlink() != 1
        }
    {
        return Err(FAILURE);
    }
    Ok(())
}

fn token(file: File) -> Result<Zeroizing<String>, &'static str> {
    private(&file, false)?;
    let mut bytes = Zeroizing::new(String::new());
    file.take(92)
        .read_to_string(&mut bytes)
        .map_err(|_| FAILURE)?;
    if bytes.len() != 91
        || !bytes.starts_with("kiarni_v1_")
        || !bytes
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(FAILURE);
    }
    Ok(bytes)
}

pub fn healthcheck(config: Config, token_file: Option<&Path>) -> Result<(), &'static str> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut tokens = Vec::new();
    if let Some(path) = token_file {
        tokens.push(token(File::from(
            unix::open(path, FLAGS, Mode::empty()).map_err(|_| FAILURE)?,
        ))?);
    } else {
        let data = PathBuf::from(config.data_dir());
        let mut name = data.file_name().ok_or(FAILURE)?.to_os_string();
        name.push(".credentials");
        let path = data.with_file_name(name);
        let directory = File::from(
            unix::open(&path, FLAGS | OFlags::DIRECTORY, Mode::empty()).map_err(|_| FAILURE)?,
        );
        private(&directory, true)?;
        for (index, entry) in fs::read_dir(path).map_err(|_| FAILURE)?.enumerate() {
            if index >= 64 {
                return Err(FAILURE);
            }
            let name = entry.map_err(|_| FAILURE)?.file_name();
            let Some(text) = name.to_str() else {
                continue;
            };
            let Some(id) = text
                .strip_prefix("token-")
                .and_then(|text| text.strip_suffix(".txt"))
            else {
                continue;
            };
            if id.len() != 16 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            let file = File::from(
                unix::openat(&directory, &name, FLAGS, Mode::empty()).map_err(|_| FAILURE)?,
            );
            tokens.push(token(file)?);
        }
    }
    for token in tokens {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|v| !v.is_zero())
            .ok_or(FAILURE)?;
        let mut stream =
            TcpStream::connect_timeout(&config.bind(), remaining).map_err(|_| FAILURE)?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| FAILURE)?;
        stream
            .set_write_timeout(Some(remaining))
            .map_err(|_| FAILURE)?;
        let host = config
            .public_authority()
            .map(str::to_owned)
            .unwrap_or_else(|| config.bind().to_string());
        let request = Zeroizing::new(format!(
            "GET /v1/health/ready HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
            *token
        ));
        stream.write_all(request.as_bytes()).map_err(|_| FAILURE)?;
        let mut response = [0u8; 4096];
        let mut length = 0;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|v| !v.is_zero())
                .ok_or(FAILURE)?;
            stream
                .set_read_timeout(Some(remaining))
                .map_err(|_| FAILURE)?;
            let read = stream.read(&mut response[length..]).map_err(|_| FAILURE)?;
            if read == 0 || length + read == response.len() {
                break;
            }
            length += read;
            if response[..length].windows(2).any(|line| line == b"\r\n") {
                break;
            }
        }
        if response[..length].windows(2).any(|line| line == b"\r\n")
            && response[..length].starts_with(b"HTTP/1.1 200 ")
        {
            return Ok(());
        }
    }
    Err(FAILURE)
}
