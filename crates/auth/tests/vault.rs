#![cfg(unix)]
use kiarni_auth::{Authority, Error, Keyset, MAX_CONFIG_BYTES, vault::Vault};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    sync::{Arc, Barrier},
    thread,
};
use zeroize::Zeroizing;

fn directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn authorization(path: &std::path::Path) -> Zeroizing<Vec<u8>> {
    let bytes = Zeroizing::new(fs::read(path).unwrap());
    let mut header = Zeroizing::new(Vec::with_capacity(7 + bytes.len()));
    header.extend_from_slice(b"Bearer ");
    header.extend_from_slice(&bytes);
    header
}

#[test]
fn provision_reload_rotate_and_revoke_using_private_immutable_snapshots() {
    let dir = directory();
    let client = directory();
    let delivery = Vault::open(client.path()).unwrap();
    let vault = Vault::open(dir.path()).unwrap();
    let (initial, token) = Keyset::bootstrap(100, 1000).unwrap();
    let snapshot = vault.provision(&initial, &token, &delivery).unwrap();
    let token_path = client.path().join(snapshot.token_file.unwrap());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(dir.path().join("auth-1.json").is_file());
    let first = authorization(&token_path);
    let metadata = fs::metadata(&token_path).unwrap();
    assert_eq!(metadata.mode() & 0o7777, 0o600);
    assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
    assert_eq!(
        fs::metadata(dir.path().join(snapshot.verifier_file))
            .unwrap()
            .mode()
            & 0o7777,
        0o600
    );
    drop(token);
    let authority = Authority::new(vault.load(1).unwrap(), 100);
    let session = authority.authenticate(&[&first], 100).unwrap();
    let (rotated, token) = initial.rotate(110, 1000, 10).unwrap();
    let snapshot = vault.provision(&rotated, &token, &delivery).unwrap();
    let next = authorization(&client.path().join(snapshot.token_file.unwrap()));
    authority.reload(Ok(vault.load(2).unwrap()), 110).unwrap();
    assert_eq!(authority.revalidate(&session, 110), Ok(()));
    assert!(authority.authenticate(&[&next], 110).is_ok());
    assert_eq!(
        authority.revalidate(&session, 120),
        Err(Error::Unauthorized)
    );
    let revoked = rotated.revoke_all().unwrap();
    vault.write_snapshot(&revoked).unwrap();
    authority.reload(Ok(vault.load(3).unwrap()), 121).unwrap();
    assert_eq!(
        authority.authenticate(&[&next], 121).err(),
        Some(Error::Unauthorized)
    );
    // A fresh authority sees the persisted revocation when explicitly loaded.
    let reopened = Authority::new(Vault::open(dir.path()).unwrap().load(3).unwrap(), 121);
    assert_eq!(
        reopened.authenticate(&[&first], 121).err(),
        Some(Error::Unauthorized)
    );
}

#[test]
fn delivery_requires_a_separate_directory_and_matching_verifier() {
    let dir = directory();
    let client = directory();
    let vault = Vault::open(dir.path()).unwrap();
    let delivery = Vault::open(client.path()).unwrap();
    let (keyset, token) = Keyset::bootstrap(100, 100).unwrap();
    assert_eq!(
        vault.provision(&keyset, &token, &vault).err(),
        Some(Error::InsecurePath)
    );
    let (_, different) = Keyset::bootstrap(100, 100).unwrap();
    assert_eq!(
        vault.provision(&keyset, &different, &delivery).err(),
        Some(Error::InvalidConfig)
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(client.path()).unwrap().count(), 0);
}

#[test]
fn existing_files_and_symlinks_are_never_overwritten_or_followed() {
    let dir = directory();
    let client = directory();
    let delivery = Vault::open(client.path()).unwrap();
    let vault = Vault::open(dir.path()).unwrap();
    let (keyset, token) = Keyset::bootstrap(100, 100).unwrap();
    let snapshot = vault.provision(&keyset, &token, &delivery).unwrap();
    assert!(matches!(
        vault.provision(&keyset, &token, &delivery),
        Err(Error::AlreadyExists)
    ));
    assert_eq!(
        vault.write_snapshot(&keyset).err(),
        Some(Error::AlreadyExists)
    );
    let file = dir.path().join(snapshot.verifier_file);
    fs::rename(&file, dir.path().join("original.json")).unwrap();
    symlink(dir.path().join("original.json"), &file).unwrap();
    assert!(vault.load(1).is_err());
    assert_eq!(
        vault.write_snapshot(&keyset).err(),
        Some(Error::AlreadyExists)
    );
    let link = dir.path().join("directory-link");
    symlink(dir.path(), &link).unwrap();
    assert!(Vault::open(&link).is_err());
}

#[test]
fn wrong_modes_and_hardlinked_verifiers_fail_closed() {
    let dir = directory();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(Vault::open(dir.path()).err(), Some(Error::InsecurePath));
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let vault = Vault::open(dir.path()).unwrap();
    let (keyset, _) = Keyset::bootstrap(100, 100).unwrap();
    vault.write_snapshot(&keyset).unwrap();
    let path = dir.path().join("auth-1.json");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(vault.load(1).err(), Some(Error::InsecurePath));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&path, dir.path().join("copy.json")).unwrap();
    assert_eq!(vault.load(1).err(), Some(Error::InsecurePath));
}

#[test]
fn partial_oversized_and_wrong_epoch_snapshots_do_not_activate() {
    let dir = directory();
    let client = directory();
    let delivery = Vault::open(client.path()).unwrap();
    let vault = Vault::open(dir.path()).unwrap();
    let (keyset, token) = Keyset::bootstrap(100, 1000).unwrap();
    let snapshot = vault.provision(&keyset, &token, &delivery).unwrap();
    let header = authorization(&client.path().join(snapshot.token_file.unwrap()));
    let authority = Authority::new(keyset, 100);
    let path = dir.path().join("auth-2.json");
    fs::write(&path, b"{\"version\":1,").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        authority.reload(vault.load(2), 101),
        Err(Error::InvalidConfig)
    );
    assert_eq!(
        authority.authenticate(&[&header], 101).err(),
        Some(Error::Unavailable)
    );
    fs::write(&path, vec![b' '; MAX_CONFIG_BYTES + 1]).unwrap();
    assert_eq!(vault.load(2).err(), Some(Error::InvalidConfig));
    fs::write(&path, fs::read(dir.path().join("auth-1.json")).unwrap()).unwrap();
    assert_eq!(vault.load(2).err(), Some(Error::InvalidConfig));
}

#[test]
fn concurrent_provisioning_has_one_snapshot_winner_without_overwrite() {
    let dir = directory();
    let client = directory();
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let path = dir.path().to_owned();
        let client_path = client.path().to_owned();
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            let vault = Vault::open(&path).unwrap();
            let delivery = Vault::open(&client_path).unwrap();
            let (keyset, token) = Keyset::bootstrap(100, 100).unwrap();
            barrier.wait();
            vault.provision(&keyset, &token, &delivery)
        }));
    }
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(Error::AlreadyExists)))
            .count(),
        1
    );
    assert!(Vault::open(dir.path()).unwrap().load(1).is_ok());
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    let header = authorization(&client.path().join(winner.token_file.unwrap()));
    let authority = Authority::new(Vault::open(dir.path()).unwrap().load(1).unwrap(), 100);
    assert!(authority.authenticate(&[&header], 100).is_ok());
}
