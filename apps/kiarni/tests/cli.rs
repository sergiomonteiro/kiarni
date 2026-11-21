use std::process::Command;

#[test]
fn version_identifies_the_binary() {
    let output = Command::new(env!("CARGO_BIN_EXE_kiarni"))
        .arg("--version")
        .output()
        .expect("execute kiarni");

    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        format!("kiarni {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn unsupported_arguments_fail_without_echoing_input() {
    for args in [vec!["unexpected-command"], vec!["--version", "unexpected"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_kiarni"))
            .args(args)
            .output()
            .expect("execute kiarni");

        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            "Argumentos não suportados. Uso: kiarni --version | kiarni serve\n".as_bytes()
        );
    }
}

#[cfg(unix)]
#[test]
fn backup_arguments_are_bounded_required_and_do_not_echo_private_input() {
    for (args, code) in [
        (vec!["backup"], "missing_backup_output"),
        (
            vec!["backup", "--data-dir", "private_path_marker"],
            "missing_backup_output",
        ),
        (
            vec![
                "backup",
                "--output",
                "private_path_marker",
                "--output",
                "another_marker",
            ],
            "duplicate_backup_output",
        ),
        (vec!["backup", "--output"], "missing_config_value"),
        (
            vec![
                "backup",
                "--output",
                "private_path_marker",
                "--unknown",
                "another_marker",
            ],
            "invalid_configuration",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_kiarni"))
            .env_clear()
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("kiarni: {code}\n")
        );
    }
}

#[cfg(unix)]
#[test]
fn restore_requires_unique_source_and_does_not_echo_private_arguments() {
    for (args, code) in [
        (vec!["restore"], "missing_restore_source"),
        (
            vec!["restore", "--data-dir", "private_marker"],
            "missing_restore_source",
        ),
        (vec!["restore", "--from"], "missing_config_value"),
        (
            vec![
                "restore",
                "--from",
                "private_marker",
                "--from",
                "another_marker",
            ],
            "duplicate_restore_source",
        ),
        (
            vec![
                "restore",
                "--from",
                "private_marker",
                "--unknown",
                "another_marker",
            ],
            "invalid_configuration",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_kiarni"))
            .env_clear()
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("kiarni: {code}\n")
        );
    }
}

#[cfg(unix)]
#[test]
fn public_origin_never_allows_public_bind_or_echoes_invalid_origin() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("unused");
    for extra in [
        vec![
            "--bind",
            "0.0.0.0:8711",
            "--public-origin",
            "https://good.example",
        ],
        vec!["--public-origin", "http://private_origin_marker"],
        vec![
            "--public-origin",
            "https://user:private_origin_marker@good.example",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_kiarni"))
            .env_clear()
            .arg("serve")
            .arg("--data-dir")
            .arg(&root)
            .args(extra)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"kiarni: invalid_configuration\n");
        assert!(!root.exists());
    }
}

#[cfg(unix)]
#[test]
fn index_arguments_are_required_strict_bounded_and_never_initialize_a_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("unused");
    for (args, code) in [
        (vec!["index"], "unsupported_index_command"),
        (
            vec!["index", "private_command"],
            "unsupported_index_command",
        ),
        (vec!["index", "create"], "missing_index_collection"),
        (
            vec!["index", "create", "--collection", "private_collection"],
            "missing_index_field",
        ),
        (vec!["index", "rebuild"], "missing_index_collection"),
        (vec!["index", "drop"], "missing_index_collection"),
        (
            vec!["index", "audit", "--collection", "private_collection"],
            "unexpected_index_argument",
        ),
        (
            vec![
                "index",
                "drop",
                "--collection",
                "items",
                "--field",
                "private_field",
            ],
            "unexpected_index_argument",
        ),
        (
            vec![
                "index",
                "create",
                "--collection",
                "items",
                "--collection",
                "private_collection",
                "--field",
                "status",
            ],
            "duplicate_index_argument",
        ),
        (
            vec![
                "index",
                "create",
                "--collection",
                "items",
                "--field",
                "status",
                "--field",
                "private_field",
            ],
            "duplicate_index_argument",
        ),
        (
            vec!["index", "create", "--collection", "items", "--field", "_id"],
            "invalid_index_field",
        ),
        (
            vec![
                "index",
                "create",
                "--collection",
                "items",
                "--field",
                "private.field",
            ],
            "invalid_index_field",
        ),
        (
            vec!["index", "rebuild", "--collection", "../private_collection"],
            "invalid_index_collection",
        ),
        (
            vec!["index", "audit", "--unknown", "private_value"],
            "invalid_configuration",
        ),
        (vec!["index", "audit"], "data_directory_unavailable"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_kiarni"))
            .env_clear()
            .args(args)
            .arg("--data-dir")
            .arg(&root)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("kiarni: {code}\n")
        );
        assert!(!root.exists());
        assert!(!temp.path().join("unused.credentials").exists());
    }
    let mut args = vec!["index", "audit"];
    for _ in 0..33 {
        args.extend(["--unknown", "private_value"]);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_kiarni"))
        .env_clear()
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.stderr, b"kiarni: config_source_too_large\n");
    assert!(output.stdout.is_empty());
}
