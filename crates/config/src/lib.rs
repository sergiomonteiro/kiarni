//! Configuration resolution and bounded admission.

mod admission;
pub use admission::{Admission, AdmissionError, AdmissionSnapshot, Permit};

use std::{collections::BTreeMap, fmt, net::SocketAddr};

use kiarni_core::DocumentInput;

const MIB: u64 = 1024 * 1024;
pub const BUFFER_PLAN_CAP: u64 = 512 * MIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    SourceTooLarge,
    UnknownSetting,
    DuplicateSetting,
    InvalidValue,
    InvalidFile,
    InvalidCombination,
    BufferPlanExceeded,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SourceTooLarge => "config_source_too_large",
            Self::UnknownSetting => "unknown_config_setting",
            Self::DuplicateSetting => "duplicate_config_setting",
            Self::InvalidValue => "invalid_config_value",
            Self::InvalidFile => "invalid_config_file",
            Self::InvalidCombination => "invalid_config_combination",
            Self::BufferPlanExceeded => "config_buffer_plan_exceeded",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
}
impl LogLevel {
    fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "off" => Ok(Self::Off),
            "error" => Ok(Self::Error),
            "warn" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            _ => Err(Error::InvalidValue),
        }
    }
}

/// Fields are private: callers cannot bypass source and cross-field validation.
pub struct Config {
    bind: SocketAddr,
    data_dir: String,
    document: u64,
    query: u64,
    result: u64,
    requests: u64,
    streams: u64,
    queue: u64,
    read_ms: u64,
    write_ms: u64,
    heartbeat_ms: u64,
    log_level: LogLevel,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config").finish_non_exhaustive()
    }
}

impl Config {
    pub fn log_level(&self) -> LogLevel {
        self.log_level
    }
    pub fn bind(&self) -> SocketAddr {
        self.bind
    }
    pub fn data_dir(&self) -> &str {
        &self.data_dir
    }
    pub fn max_document_bytes(&self) -> u64 {
        self.document
    }
    pub fn max_query_bytes(&self) -> u64 {
        self.query
    }
    pub fn max_result_bytes(&self) -> u64 {
        self.result
    }
    pub fn max_requests(&self) -> u64 {
        self.requests
    }
    pub fn max_streams(&self) -> u64 {
        self.streams
    }
    pub fn feed_queue(&self) -> u64 {
        self.queue
    }
    pub fn read_timeout_ms(&self) -> u64 {
        self.read_ms
    }
    pub fn write_timeout_ms(&self) -> u64 {
        self.write_ms
    }
    pub fn heartbeat_ms(&self) -> u64 {
        self.heartbeat_ms
    }

    pub fn frame_bytes(&self) -> u64 {
        3 * MIB
    }

    /// Planning reservation, not a measured or enforced RSS ceiling.
    pub fn buffer_plan_bytes(&self) -> Result<u64, Error> {
        buffer_plan(
            self.requests,
            self.streams,
            self.queue,
            self.document,
            self.query,
            self.result,
        )
    }

    fn apply(&mut self, key: &str, value: &str) -> Result<(), Error> {
        match key {
            "log_level" => self.log_level = LogLevel::parse(value)?,
            "bind" => {
                let bind: SocketAddr = value.parse().map_err(|_| Error::InvalidValue)?;
                if !bind.ip().is_loopback() || bind.port() == 0 {
                    return Err(Error::InvalidValue);
                }
                self.bind = bind;
            }
            "data_dir" => {
                if value.is_empty() || value.chars().any(char::is_control) {
                    return Err(Error::InvalidValue);
                }
                self.data_dir = value.into();
            }
            "max_document_bytes" => self.document = number(value, 1, MIB)?,
            "max_query_bytes" => self.query = number(value, 1, 16384)?,
            "max_result_bytes" => self.result = number(value, 1, 4 * MIB)?,
            "max_requests" => self.requests = number(value, 1, 128)?,
            "max_streams" => self.streams = number(value, 1, 32)?,
            "feed_queue" => self.queue = number(value, 1, 16)?,
            "read_timeout_ms" => self.read_ms = number(value, 100, 30000)?,
            "write_timeout_ms" => self.write_ms = number(value, 100, 30000)?,
            "heartbeat_ms" => self.heartbeat_ms = number(value, 1000, 60000)?,
            _ => return Err(Error::UnknownSetting),
        }
        Ok(())
    }
}

fn known(key: &str) -> bool {
    matches!(
        key,
        "bind"
            | "data_dir"
            | "max_document_bytes"
            | "max_query_bytes"
            | "max_result_bytes"
            | "max_requests"
            | "max_streams"
            | "feed_queue"
            | "read_timeout_ms"
            | "write_timeout_ms"
            | "heartbeat_ms"
            | "log_level"
    )
}

fn number(value: &str, low: u64, high: u64) -> Result<u64, Error> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::InvalidValue);
    }
    let parsed = value.parse::<u64>().map_err(|_| Error::InvalidValue)?;
    if !(low..=high).contains(&parsed) {
        return Err(Error::InvalidValue);
    }
    Ok(parsed)
}

fn buffer_plan(
    requests: u64,
    streams: u64,
    queue: u64,
    document: u64,
    query: u64,
    result: u64,
) -> Result<u64, Error> {
    let plan = (|| {
        let per_request = document
            .checked_mul(4)?
            .checked_add(result)?
            .checked_add(query.checked_mul(2)?)?;
        let per_stream = queue
            .checked_mul(2)?
            .checked_mul(document)?
            .checked_add(3 * MIB)?;
        requests
            .checked_mul(per_request)?
            .checked_add(streams.checked_mul(per_stream)?)?
            .checked_add(8 * MIB)
    })()
    .ok_or(Error::BufferPlanExceeded)?;
    if plan > BUFFER_PLAN_CAP {
        return Err(Error::BufferPlanExceeded);
    }
    Ok(plan)
}

fn insert(source: &mut BTreeMap<String, String>, key: String, value: String) -> Result<(), Error> {
    if key.len() > 64 || value.len() > 4096 || source.len() >= 32 {
        return Err(Error::SourceTooLarge);
    }
    if !known(&key) {
        return Err(Error::UnknownSetting);
    }
    if source.insert(key, value).is_some() {
        return Err(Error::DuplicateSetting);
    }
    Ok(())
}

/// Normalized CLI pairs and UTF-8 environment pairs, not process-global reads.
/// An optional JSON file is supplied as bytes; no file discovery or disk I/O.
pub fn resolve(
    cli: &[(&str, &str)],
    env: &[(&str, &str)],
    file: Option<&[u8]>,
) -> Result<Config, Error> {
    let mut config = Config {
        bind: SocketAddr::from(([127, 0, 0, 1], 8711)),
        data_dir: "./kiarni-data".into(),
        document: MIB,
        query: 16384,
        result: 4 * MIB,
        requests: 32,
        streams: 8,
        queue: 4,
        read_ms: 5000,
        write_ms: 5000,
        heartbeat_ms: 15000,
        log_level: LogLevel::Info,
    };
    let mut file_values = BTreeMap::new();
    if let Some(bytes) = file {
        let input = DocumentInput::parse(bytes, 16384).map_err(|_| Error::InvalidFile)?;
        for (key, raw) in input.fields() {
            if !known(key) {
                return Err(Error::UnknownSetting);
            }
            let value = if matches!(key.as_str(), "bind" | "data_dir" | "log_level") {
                serde_json::from_str::<String>(raw.get()).map_err(|_| Error::InvalidValue)?
            } else {
                serde_json::from_str::<u64>(raw.get())
                    .map_err(|_| Error::InvalidValue)?
                    .to_string()
            };
            insert(&mut file_values, key.clone(), value)?;
        }
    }
    let mut env_values = BTreeMap::new();
    for &(key, value) in env {
        if let Some(key) = key.strip_prefix("KIARNI_") {
            if key.len() > 64 || value.len() > 4096 {
                return Err(Error::SourceTooLarge);
            }
            if !key
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            {
                return Err(Error::UnknownSetting);
            }
            insert(&mut env_values, key.to_ascii_lowercase(), value.into())?;
        }
    }
    if cli.len() > 32 {
        return Err(Error::SourceTooLarge);
    }
    let mut cli_values = BTreeMap::new();
    for &(key, value) in cli {
        let key = key.strip_prefix("--").ok_or(Error::UnknownSetting)?;
        if key.len() > 64 || value.len() > 4096 {
            return Err(Error::SourceTooLarge);
        }
        if !key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
        {
            return Err(Error::UnknownSetting);
        }
        insert(&mut cli_values, key.replace('-', "_"), value.into())?;
    }
    // Validate every source, even overwritten values; relationships apply to
    // the final merged configuration so a CLI override can tune related fields.
    for source in [file_values, env_values, cli_values] {
        for (key, value) in source {
            config.apply(&key, &value)?;
        }
    }
    if config.streams > config.requests || config.result < config.document + 1024 {
        return Err(Error::InvalidCombination);
    }
    config.buffer_plan_bytes()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arithmetic_overflow_is_rejected() {
        assert_eq!(
            buffer_plan(u64::MAX, 8, 4, MIB, 16384, 4 * MIB),
            Err(Error::BufferPlanExceeded)
        );
        assert_eq!(
            buffer_plan(1, 1, u64::MAX, MIB, 16384, 4 * MIB),
            Err(Error::BufferPlanExceeded)
        );
    }
}
