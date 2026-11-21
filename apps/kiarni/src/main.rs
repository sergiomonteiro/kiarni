use std::{env, process::ExitCode};

#[cfg(unix)]
mod cli;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    match (args.next(), args.next()) {
        (Some(arg), None) if arg == "--version" => {
            println!("kiarni {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        (None, None) => {
            println!("Uso: kiarni --version | kiarni serve [--data-dir CAMINHO]");
            println!("Probe: kiarni healthcheck [--data-dir CAMINHO] [--token-file ARQUIVO]");
            println!("Backup: kiarni backup --data-dir ORIGEM --output DESTINO");
            println!("Restore: kiarni restore --from BACKUP --data-dir NOVA_RAIZ");
            println!("Auth: kiarni auth rotate|revoke-all [--data-dir CAMINHO]");
            println!(
                "Índices offline: kiarni index create|rebuild|drop --collection NOME [--field CAMPO] [--data-dir CAMINHO]"
            );
            println!("Auditoria offline: kiarni index audit [--data-dir CAMINHO]");
            ExitCode::SUCCESS
        }
        #[cfg(unix)]
        (Some(arg), next) if arg == "healthcheck" => {
            match cli::healthcheck(next.into_iter().chain(args)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => {
                    eprintln!("kiarni: healthcheck_failed");
                    ExitCode::FAILURE
                }
            }
        }
        #[cfg(unix)]
        (Some(arg), next) if arg == "restore" => match cli::restore(next.into_iter().chain(args)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("kiarni: {error}");
                ExitCode::FAILURE
            }
        },
        #[cfg(unix)]
        (Some(arg), next) if arg == "backup" => match cli::backup(next.into_iter().chain(args)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("kiarni: {error}");
                ExitCode::FAILURE
            }
        },
        #[cfg(unix)]
        (Some(arg), next) if arg == "auth" => match cli::auth(next.into_iter().chain(args)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("kiarni: {error}");
                ExitCode::FAILURE
            }
        },
        #[cfg(unix)]
        (Some(arg), next) if arg == "index" => match cli::index(next.into_iter().chain(args)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("kiarni: {error}");
                ExitCode::FAILURE
            }
        },
        #[cfg(unix)]
        (Some(arg), next) if arg == "serve" => match cli::serve(next.into_iter().chain(args)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("kiarni: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("Argumentos não suportados. Uso: kiarni --version | kiarni serve");
            ExitCode::FAILURE
        }
    }
}
