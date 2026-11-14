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
            println!("Auth: kiarni auth rotate|revoke-all [--data-dir CAMINHO]");
            ExitCode::SUCCESS
        }
        #[cfg(unix)]
        (Some(arg), next) if arg == "auth" => match cli::auth(next.into_iter().chain(args)) {
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
