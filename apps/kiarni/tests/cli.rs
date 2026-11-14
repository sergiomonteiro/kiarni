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
