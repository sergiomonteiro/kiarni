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
