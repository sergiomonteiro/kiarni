//! Local credentials and authentication authority.
#[cfg(unix)]
pub mod activation;
#[cfg(unix)]
pub mod vault;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt, sync::Mutex};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub const MAX_LIFETIME: u64 = 30 * 24 * 60 * 60;
pub const MAX_OVERLAP: u64 = 300;
pub const MAX_CONFIG_BYTES: usize = 4096;
const PREFIX: &[u8] = b"kiarni_v1_";
const TOKEN_BYTES: usize = 91;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Unauthorized,
    Unavailable,
    ClockRollback,
    InvalidConfig,
    EpochRollback,
    InvalidLifetime,
    InvalidOverlap,
    RotationInProgress,
    EntropyUnavailable,
    GenerationExhausted,
    EpochExhausted,
    FileAccess,
    InsecurePath,
    AlreadyExists,
    Busy,
    BootstrapIncomplete,
    CommitUncertain,
}

impl Error {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Unavailable => "auth_unavailable",
            Self::ClockRollback => "clock_rollback",
            Self::InvalidConfig => "invalid_auth_config",
            Self::EpochRollback => "auth_epoch_rollback",
            Self::InvalidLifetime => "invalid_token_lifetime",
            Self::InvalidOverlap => "invalid_token_overlap",
            Self::RotationInProgress => "rotation_in_progress",
            Self::EntropyUnavailable => "entropy_unavailable",
            Self::GenerationExhausted => "token_generation_exhausted",
            Self::EpochExhausted => "auth_epoch_exhausted",
            Self::FileAccess => "auth_file_error",
            Self::InsecurePath => "insecure_auth_path",
            Self::AlreadyExists => "auth_file_exists",
            Self::Busy => "auth_writer_busy",
            Self::BootstrapIncomplete => "auth_bootstrap_incomplete",
            Self::CommitUncertain => "auth_commit_uncertain",
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for Error {}

pub struct SecretToken(Zeroizing<String>);
impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretToken([REDACTED])")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    id: String,
    sha256: String,
    not_before: u64,
    expires_at: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Schema {
    version: u8,
    epoch: u64,
    keys: Vec<Record>,
}

#[derive(Clone)]
struct Key {
    id: [u8; 8],
    digest: [u8; 32],
    not_before: u64,
    expires_at: u64,
}

#[derive(Clone)]
pub struct Keyset {
    schema: Schema,
    keys: Vec<Key>,
}

impl fmt::Debug for Keyset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keyset")
            .field("epoch", &self.schema.epoch)
            .field("keys", &self.keys.len())
            .finish()
    }
}

fn hex(bytes: &[u8], output: &mut String) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
}
fn hex_string(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    hex(bytes, &mut text);
    text
}
fn unhex<const N: usize>(text: &str) -> Result<[u8; N], Error> {
    if text.len() != N * 2 {
        return Err(Error::InvalidConfig);
    }
    let mut output = [0; N];
    let digit = |b| match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        _ => Err(Error::InvalidConfig),
    };
    for (i, pair) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        output[i] = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Ok(output)
}

impl Keyset {
    fn validated(schema: Schema) -> Result<Self, Error> {
        if schema.version != 1 || schema.epoch == 0 || schema.keys.len() > 2 {
            return Err(Error::InvalidConfig);
        }
        let mut ids = BTreeSet::new();
        let mut keys = Vec::new();
        for record in &schema.keys {
            if !ids.insert(&record.id)
                || record.expires_at <= record.not_before
                || record.expires_at - record.not_before > MAX_LIFETIME
            {
                return Err(Error::InvalidConfig);
            }
            keys.push(Key {
                id: unhex(&record.id)?,
                digest: unhex(&record.sha256)?,
                not_before: record.not_before,
                expires_at: record.expires_at,
            });
        }
        Ok(Self { schema, keys })
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(Error::InvalidConfig);
        }
        kiarni_core::DocumentInput::parse(bytes, MAX_CONFIG_BYTES)
            .map_err(|_| Error::InvalidConfig)?;
        Self::validated(serde_json::from_slice(bytes).map_err(|_| Error::InvalidConfig)?)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, Error> {
        let bytes = serde_json::to_vec(&self.schema).map_err(|_| Error::InvalidConfig)?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(Error::InvalidConfig);
        }
        Ok(bytes)
    }
    pub fn epoch(&self) -> u64 {
        self.schema.epoch
    }

    pub fn bootstrap(now: u64, lifetime: u64) -> Result<(Self, SecretToken), Error> {
        let (record, token) = issue(now, lifetime, &[], |bytes| {
            getrandom::fill(bytes).map_err(|_| ())
        })?;
        Ok((
            Self::validated(Schema {
                version: 1,
                epoch: 1,
                keys: vec![record],
            })?,
            token,
        ))
    }

    pub fn rotate(
        &self,
        now: u64,
        lifetime: u64,
        overlap: u64,
    ) -> Result<(Self, SecretToken), Error> {
        if overlap > MAX_OVERLAP {
            return Err(Error::InvalidOverlap);
        }
        let epoch = self
            .schema
            .epoch
            .checked_add(1)
            .ok_or(Error::EpochExhausted)?;
        let active: Vec<_> = self
            .schema
            .keys
            .iter()
            .filter(|key| key.not_before <= now && now < key.expires_at)
            .cloned()
            .collect();
        if overlap > 0 && active.len() > 1 {
            return Err(Error::RotationInProgress);
        }
        let mut keys = if overlap == 0 { Vec::new() } else { active };
        for key in &mut keys {
            key.expires_at = key
                .expires_at
                .min(now.checked_add(overlap).ok_or(Error::InvalidOverlap)?);
        }
        // Avoid even ID collisions with keys in the previous snapshot.
        let previous_ids: Vec<_> = self.schema.keys.iter().map(|key| key.id.as_str()).collect();
        let (record, token) = issue(now, lifetime, &previous_ids, |bytes| {
            getrandom::fill(bytes).map_err(|_| ())
        })?;
        keys.push(record);
        Ok((
            Self::validated(Schema {
                version: 1,
                epoch,
                keys,
            })?,
            token,
        ))
    }

    pub fn revoke_all(&self) -> Result<Self, Error> {
        Self::validated(Schema {
            version: 1,
            epoch: self
                .schema
                .epoch
                .checked_add(1)
                .ok_or(Error::EpochExhausted)?,
            keys: Vec::new(),
        })
    }
}

fn issue(
    now: u64,
    lifetime: u64,
    existing_ids: &[&str],
    mut fill: impl FnMut(&mut [u8; 40]) -> Result<(), ()>,
) -> Result<(Record, SecretToken), Error> {
    if !(1..=MAX_LIFETIME).contains(&lifetime) {
        return Err(Error::InvalidLifetime);
    }
    let expires_at = now.checked_add(lifetime).ok_or(Error::InvalidLifetime)?;
    for _ in 0..8 {
        let mut bytes = Zeroizing::new([0u8; 40]);
        fill(&mut bytes).map_err(|_| Error::EntropyUnavailable)?;
        let id = hex_string(&bytes[..8]);
        if existing_ids.contains(&id.as_str()) {
            continue;
        }
        let mut token = Zeroizing::new(String::with_capacity(TOKEN_BYTES));
        token.push_str("kiarni_v1_");
        token.push_str(&id);
        token.push('_');
        hex(&bytes[8..], &mut token);
        let hash = Sha256::digest(token.as_bytes());
        let record = Record {
            id,
            sha256: hex_string(&hash),
            not_before: now,
            expires_at,
        };
        return Ok((record, SecretToken(token)));
    }
    Err(Error::GenerationExhausted)
}

pub struct Session {
    id: [u8; 8],
    digest: [u8; 32],
}
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Session([REDACTED])")
    }
}

struct State {
    keyset: Keyset,
    available: bool,
    last_time: u64,
}
pub struct Authority(Mutex<State>);

impl Authority {
    pub fn new(keyset: Keyset, now: u64) -> Self {
        Self(Mutex::new(State {
            keyset,
            available: true,
            last_time: now,
        }))
    }

    pub fn reload(&self, candidate: Result<Keyset, Error>, now: u64) -> Result<(), Error> {
        let mut state = self.0.lock().map_err(|_| Error::Unavailable)?;
        check_time(&mut state, now)?;
        // Invalid reloads fail closed; no silent fallback to an older credential.
        state.available = false;
        let candidate = candidate?;
        if candidate.epoch() < state.keyset.epoch() {
            return Err(Error::EpochRollback);
        }
        if candidate.epoch() == state.keyset.epoch() && candidate.schema != state.keyset.schema {
            return Err(Error::InvalidConfig);
        }
        state.keyset = candidate;
        state.available = true;
        Ok(())
    }

    pub fn authenticate(&self, authorization: &[&[u8]], now: u64) -> Result<Session, Error> {
        let mut state = self.0.lock().map_err(|_| Error::Unavailable)?;
        check_time(&mut state, now)?;
        if !state.available {
            return Err(Error::Unavailable);
        }
        let [header] = authorization else {
            return Err(Error::Unauthorized);
        };
        if header.len() != 7 + TOKEN_BYTES
            || !header[..6].eq_ignore_ascii_case(b"Bearer")
            || header[6] != b' '
        {
            return Err(Error::Unauthorized);
        }
        let token = &header[7..];
        let lower_hex = |b: &u8| b.is_ascii_digit() || (b'a'..=b'f').contains(b);
        if !token.starts_with(PREFIX)
            || token[26] != b'_'
            || !token[10..26].iter().all(lower_hex)
            || !token[27..].iter().all(lower_hex)
        {
            return Err(Error::Unauthorized);
        }
        let id_text = std::str::from_utf8(&token[10..26]).map_err(|_| Error::Unauthorized)?;
        let session = Session {
            id: unhex(id_text).map_err(|_| Error::Unauthorized)?,
            digest: Sha256::digest(token).into(),
        };
        verify(&state.keyset, &session, now)?;
        Ok(session)
    }

    pub fn revalidate(&self, session: &Session, now: u64) -> Result<(), Error> {
        let mut state = self.0.lock().map_err(|_| Error::Unavailable)?;
        check_time(&mut state, now)?;
        if !state.available {
            return Err(Error::Unavailable);
        }
        verify(&state.keyset, session, now)
    }
}

fn check_time(state: &mut State, now: u64) -> Result<(), Error> {
    if now < state.last_time {
        return Err(Error::ClockRollback);
    }
    state.last_time = now;
    Ok(())
}

fn verify(keyset: &Keyset, session: &Session, now: u64) -> Result<(), Error> {
    let mut matched = subtle::Choice::from(0);
    for key in &keyset.keys {
        let equal = key.digest.ct_eq(&session.digest) & key.id.ct_eq(&session.id);
        matched |=
            equal & subtle::Choice::from(u8::from(key.not_before <= now && now < key.expires_at));
    }
    if bool::from(matched) {
        Ok(())
    } else {
        Err(Error::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(token: &SecretToken) -> Zeroizing<Vec<u8>> {
        let mut header = Zeroizing::new(Vec::with_capacity(7 + TOKEN_BYTES));
        header.extend_from_slice(b"Bearer ");
        header.extend_from_slice(token.0.as_bytes());
        header
    }

    #[test]
    fn token_is_randomly_issued_but_verifier_contains_no_plaintext_secret() {
        let (keyset, token) = Keyset::bootstrap(100, 1000).unwrap();
        assert_eq!(token.0.len(), TOKEN_BYTES);
        let bytes = keyset.to_json().unwrap();
        let json = std::str::from_utf8(&bytes).unwrap();
        assert!(!json.contains(token.0.as_str()));
        assert!(!json.contains(&token.0[27..]));
        assert_eq!(format!("{token:?}"), "SecretToken([REDACTED])");
        assert_eq!(format!("{keyset:?}"), "Keyset { epoch: 1, keys: 1 }");
        let authority = Authority::new(Keyset::parse(&bytes).unwrap(), 100);
        let header = header(&token);
        let session = authority.authenticate(&[&header], 100).unwrap();
        assert_eq!(format!("{session:?}"), "Session([REDACTED])");
    }

    #[test]
    fn malformed_missing_duplicate_unknown_and_expired_credentials_share_one_error() {
        let (keyset, token) = Keyset::bootstrap(100, 10).unwrap();
        let authority = Authority::new(keyset, 100);
        let good = header(&token);
        for headers in [
            vec![],
            vec![b"".as_slice()],
            vec![b"Bearer".as_slice()],
            vec![good.as_slice(), good.as_slice()],
        ] {
            assert_eq!(
                authority.authenticate(&headers, 100).err(),
                Some(Error::Unauthorized)
            );
        }
        let mut malformed = Zeroizing::new(good.to_vec());
        malformed.push(b' ');
        assert_eq!(
            authority.authenticate(&[&malformed], 100).err(),
            Some(Error::Unauthorized)
        );
        malformed.pop();
        malformed[6] = b'\t';
        assert_eq!(
            authority.authenticate(&[&malformed], 100).err(),
            Some(Error::Unauthorized)
        );
        malformed[6] = b' ';
        malformed[34] = 0xff;
        assert_eq!(
            authority.authenticate(&[&malformed], 100).err(),
            Some(Error::Unauthorized)
        );
        let (_other, unknown) = Keyset::bootstrap(100, 10).unwrap();
        assert_eq!(
            authority.authenticate(&[&header(&unknown)], 100).err(),
            Some(Error::Unauthorized)
        );
        let mut lowercase = Zeroizing::new(good.to_vec());
        lowercase[..6].copy_from_slice(b"bearer");
        assert!(authority.authenticate(&[&lowercase], 100).is_ok());
        assert_eq!(
            authority.authenticate(&[&good], 110).err(),
            Some(Error::Unauthorized)
        );
    }

    #[test]
    fn validity_is_start_inclusive_end_exclusive_and_clock_rollback_fails_closed() {
        let (keyset, token) = Keyset::bootstrap(100, 10).unwrap();
        let authority = Authority::new(keyset, 99);
        let header = header(&token);
        assert_eq!(
            authority.authenticate(&[&header], 99).err(),
            Some(Error::Unauthorized)
        );
        let session = authority.authenticate(&[&header], 100).unwrap();
        assert_eq!(authority.revalidate(&session, 109), Ok(()));
        assert_eq!(
            authority.revalidate(&session, 110),
            Err(Error::Unauthorized)
        );
        assert_eq!(
            authority.revalidate(&session, 109),
            Err(Error::ClockRollback)
        );
        assert_eq!(
            authority.authenticate(&[&header], 110).err(),
            Some(Error::Unauthorized)
        );
    }

    #[test]
    fn rotation_limits_overlap_and_invalidates_old_sessions_at_the_deadline() {
        let (keyset, old) = Keyset::bootstrap(100, 1000).unwrap();
        let authority = Authority::new(keyset.clone(), 100);
        let old_header = header(&old);
        let old_session = authority.authenticate(&[&old_header], 100).unwrap();
        let (rotated, new) = keyset.rotate(150, 500, 60).unwrap();
        assert_eq!(rotated.epoch(), 2);
        assert!(matches!(
            rotated.rotate(151, 500, 60),
            Err(Error::RotationInProgress)
        ));
        authority.reload(Ok(rotated), 150).unwrap();
        assert_eq!(authority.revalidate(&old_session, 150), Ok(()));
        assert!(authority.authenticate(&[&header(&new)], 150).is_ok());
        assert_eq!(authority.revalidate(&old_session, 209), Ok(()));
        assert_eq!(
            authority.revalidate(&old_session, 210),
            Err(Error::Unauthorized)
        );
        assert!(authority.authenticate(&[&header(&new)], 210).is_ok());
        let (short, _) = Keyset::bootstrap(100, 100).unwrap();
        let (rotated, _) = short.rotate(150, 100, 300).unwrap();
        assert_eq!(rotated.schema.keys[0].expires_at, 200);
    }

    #[test]
    fn emergency_rotation_and_revoke_all_close_old_sessions_and_allow_local_recovery() {
        let (first, old) = Keyset::bootstrap(100, 1000).unwrap();
        let (second, middle) = first.rotate(101, 1000, 30).unwrap();
        let (emergency, new) = second.rotate(102, 1000, 0).unwrap();
        let authority = Authority::new(second, 101);
        let session = authority.authenticate(&[&header(&middle)], 101).unwrap();
        authority.reload(Ok(emergency.clone()), 102).unwrap();
        assert_eq!(
            authority.authenticate(&[&header(&old)], 102).err(),
            Some(Error::Unauthorized)
        );
        assert_eq!(
            authority.revalidate(&session, 102),
            Err(Error::Unauthorized)
        );
        let fresh = authority.authenticate(&[&header(&new)], 102).unwrap();
        let revoked = emergency.revoke_all().unwrap();
        authority.reload(Ok(revoked.clone()), 103).unwrap();
        assert_eq!(authority.revalidate(&fresh, 103), Err(Error::Unauthorized));
        let (recovered, token) = revoked.rotate(104, 1000, 0).unwrap();
        authority.reload(Ok(recovered), 104).unwrap();
        assert!(authority.authenticate(&[&header(&token)], 104).is_ok());
    }

    #[test]
    fn reload_rejects_invalid_or_rollback_snapshots_without_fallback() {
        let (first, token) = Keyset::bootstrap(100, 1000).unwrap();
        let authority = Authority::new(first.clone(), 100);
        let header = header(&token);
        authority.reload(Ok(first.clone()), 100).unwrap();
        assert_eq!(
            authority.reload(Err(Error::InvalidConfig), 101),
            Err(Error::InvalidConfig)
        );
        assert_eq!(
            authority.authenticate(&[&header], 101).err(),
            Some(Error::Unavailable)
        );
        authority.reload(Ok(first.clone()), 102).unwrap();
        assert!(authority.authenticate(&[&header], 102).is_ok());
        let (same_epoch_different, _) = Keyset::bootstrap(100, 1000).unwrap();
        assert_eq!(
            authority.reload(Ok(same_epoch_different), 103),
            Err(Error::InvalidConfig)
        );
        assert_eq!(
            authority.authenticate(&[&header], 103).err(),
            Some(Error::Unavailable)
        );
        authority
            .reload(Ok(first.revoke_all().unwrap()), 104)
            .unwrap();
        assert_eq!(authority.reload(Ok(first), 105), Err(Error::EpochRollback));
        assert_eq!(
            authority.authenticate(&[&header], 105).err(),
            Some(Error::Unavailable)
        );
    }

    #[test]
    fn schema_is_bounded_strict_and_does_not_accept_duplicate_ids_or_plaintext() {
        let (valid, _) = Keyset::bootstrap(100, 1000).unwrap();
        let original: serde_json::Value =
            serde_json::from_slice(&valid.to_json().unwrap()).unwrap();
        let mut cases = Vec::new();
        let mut value = original.clone();
        value["version"] = 2.into();
        cases.push(value);
        let mut value = original.clone();
        value["epoch"] = 0.into();
        cases.push(value);
        let mut value = original.clone();
        value["secret"] = "not-accepted".into();
        cases.push(value);
        let mut value = original.clone();
        value["keys"][0]["sha256"] = "bad".into();
        cases.push(value);
        let mut value = original.clone();
        value["keys"][0]["expires_at"] = 100.into();
        cases.push(value);
        let mut value = original.clone();
        value["keys"][0]["expires_at"] = (MAX_LIFETIME + 101).into();
        cases.push(value);
        let mut value = original.clone();
        value["keys"] = serde_json::json!([value["keys"][0].clone(), value["keys"][0].clone()]);
        cases.push(value);
        for value in cases {
            assert!(matches!(
                Keyset::parse(&serde_json::to_vec(&value).unwrap()),
                Err(Error::InvalidConfig)
            ));
        }
        assert!(Keyset::parse(br#"{"version":1,"epoch":1,"epoch":2,"keys":[]}"#).is_err());
        assert!(Keyset::parse(&vec![b' '; MAX_CONFIG_BYTES + 1]).is_err());
    }

    #[test]
    fn generation_failures_collisions_and_lifetimes_are_bounded() {
        assert!(matches!(
            issue(100, 1, &[], |_| Err(())),
            Err(Error::EntropyUnavailable)
        ));
        let mut calls = 0;
        let result = issue(100, 1, &["0000000000000000"], |bytes| {
            calls += 1;
            bytes.fill(0);
            Ok(())
        });
        assert!(matches!(result, Err(Error::GenerationExhausted)));
        assert_eq!(calls, 8);
        for lifetime in [0, MAX_LIFETIME + 1] {
            assert!(matches!(
                Keyset::bootstrap(100, lifetime),
                Err(Error::InvalidLifetime)
            ));
        }
        assert!(Keyset::bootstrap(100, 1).is_ok());
        assert!(Keyset::bootstrap(100, MAX_LIFETIME).is_ok());
        assert!(matches!(
            Keyset::bootstrap(u64::MAX, 1),
            Err(Error::InvalidLifetime)
        ));
        let (mut keyset, _) = Keyset::bootstrap(100, 1000).unwrap();
        assert!(matches!(
            keyset.rotate(101, 100, MAX_OVERLAP + 1),
            Err(Error::InvalidOverlap)
        ));
        keyset.schema.epoch = u64::MAX;
        assert!(matches!(keyset.revoke_all(), Err(Error::EpochExhausted)));
        assert!(matches!(
            keyset.rotate(101, 100, 0),
            Err(Error::EpochExhausted)
        ));
    }

    #[test]
    fn concurrent_requests_and_sessions_observe_completed_revocation() {
        use std::{
            sync::{Arc, Barrier},
            thread,
        };
        let (keyset, token) = Keyset::bootstrap(100, 1000).unwrap();
        let authority = Arc::new(Authority::new(keyset.clone(), 100));
        let header = Arc::new(header(&token));
        let session = Arc::new(authority.authenticate(&[header.as_slice()], 100).unwrap());
        let barrier = Arc::new(Barrier::new(5));
        let mut workers = Vec::new();
        for _ in 0..4 {
            let authority = Arc::clone(&authority);
            let header = Arc::clone(&header);
            let session = Arc::clone(&session);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                assert_eq!(
                    authority.authenticate(&[header.as_slice()], 101).err(),
                    Some(Error::Unauthorized)
                );
                assert_eq!(
                    authority.revalidate(&session, 101),
                    Err(Error::Unauthorized)
                );
            }));
        }
        authority
            .reload(Ok(keyset.revoke_all().unwrap()), 101)
            .unwrap();
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
    }
}
