use kiarni_config::{Config, resolve};
use std::{env, ffi::OsString, fs::File, io::Read};

fn configuration(mut args: impl Iterator<Item = OsString>) -> Result<Config, &'static str> {
    let mut file = None;
    let mut cli = Vec::new();
    while let Some(key) = args.next() {
        if cli.len() >= 32 {
            return Err("config_source_too_large");
        }
        let key = key.into_string().map_err(|_| "invalid_config_value")?;
        let value = args.next().ok_or("missing_config_value")?;
        if key == "--config" {
            if file.replace(value).is_some() {
                return Err("duplicate_config_setting");
            }
        } else {
            cli.push((
                key,
                value.into_string().map_err(|_| "invalid_config_value")?,
            ));
        }
    }
    let mut environment = Vec::new();
    for (key, value) in env::vars_os() {
        if key.as_encoded_bytes().starts_with(b"KIARNI_") {
            if environment.len() >= 32 {
                return Err("config_source_too_large");
            }
            environment.push((
                key.into_string().map_err(|_| "invalid_config_value")?,
                value.into_string().map_err(|_| "invalid_config_value")?,
            ));
        }
    }
    let bytes = file
        .map(|path| {
            use rustix::fs::{Mode, OFlags, open};
            let mut bytes = Vec::new();
            let file = File::from(
                open(
                    std::path::Path::new(&path),
                    OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|_| "config_file_unavailable")?,
            );
            if !file
                .metadata()
                .map_err(|_| "config_file_unavailable")?
                .is_file()
            {
                return Err("config_file_unavailable");
            }
            file.take(16385)
                .read_to_end(&mut bytes)
                .map_err(|_| "config_file_unavailable")?;
            Ok::<_, &str>(bytes)
        })
        .transpose()?;
    let config = resolve(
        &cli.iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        &environment
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        bytes.as_deref(),
    )
    .map_err(|_| "invalid_configuration")?;
    const MIB: u64 = 1024 * 1024;
    if config.max_result_bytes() < MIB + 1024 {
        return Err("config_result_below_format_limit");
    }
    let plan = config.max_requests()
        * (4 * MIB + config.max_result_bytes() + 2 * config.max_query_bytes())
        + config.max_streams() * (2 * config.feed_queue() * MIB + 3 * MIB)
        + 8 * MIB
        + 32 * MIB;
    if plan > kiarni_config::BUFFER_PLAN_CAP {
        return Err("config_buffer_plan_exceeded");
    }
    kiarni_server::buffer_plan_bytes(&config)?;
    Ok(config)
}

pub fn serve(args: impl Iterator<Item = OsString>) -> Result<(), Box<dyn std::error::Error>> {
    kiarni_server::serve(configuration(args)?)
}

pub fn auth(mut args: impl Iterator<Item = OsString>) -> Result<(), Box<dyn std::error::Error>> {
    let command = match args.next().as_deref().and_then(|value| value.to_str()) {
        Some("rotate") => kiarni_server::AuthCommand::Rotate,
        Some("revoke-all") => kiarni_server::AuthCommand::RevokeAll,
        _ => return Err("unsupported_auth_command".into()),
    };
    let result = kiarni_server::manage_auth(configuration(args)?, command)?;
    if let Some(path) = result.token_file {
        println!("Credencial em {path:?} (epoch {}).", result.epoch);
    } else {
        println!("Credenciais revogadas (epoch {}).", result.epoch);
    }
    Ok(())
}

pub fn index(mut args: impl Iterator<Item = OsString>) -> Result<(), Box<dyn std::error::Error>> {
    let operation = args.next().ok_or("unsupported_index_command")?;
    if !["create", "rebuild", "drop", "audit"]
        .iter()
        .any(|value| operation == *value)
    {
        return Err("unsupported_index_command".into());
    }
    let (mut collection, mut field) = (None, None);
    let mut config = Vec::new();
    let mut pairs = 0;
    while let Some(key) = args.next() {
        pairs += 1;
        if pairs > 32 {
            return Err("config_source_too_large".into());
        }
        let value = args.next().ok_or("missing_config_value")?;
        if key == "--collection" {
            if collection
                .replace(
                    value
                        .into_string()
                        .map_err(|_| "invalid_index_collection")?,
                )
                .is_some()
            {
                return Err("duplicate_index_argument".into());
            }
        } else if key == "--field" {
            if field
                .replace(value.into_string().map_err(|_| "invalid_index_field")?)
                .is_some()
            {
                return Err("duplicate_index_argument".into());
            }
        } else {
            config.push(key);
            config.push(value);
        }
    }
    use kiarni_server::IndexCommand;
    let command = match operation.to_str() {
        Some("audit") => {
            if collection.is_some() || field.is_some() {
                return Err("unexpected_index_argument".into());
            }
            IndexCommand::Audit
        }
        Some("create") => IndexCommand::Create {
            collection: collection.as_deref().ok_or("missing_index_collection")?,
            field: field.as_deref().ok_or("missing_index_field")?,
        },
        _ => {
            if field.is_some() {
                return Err("unexpected_index_argument".into());
            }
            let collection = collection.as_deref().ok_or("missing_index_collection")?;
            if operation == "rebuild" {
                IndexCommand::Rebuild { collection }
            } else {
                IndexCommand::Drop { collection }
            }
        }
    };
    match kiarni_server::manage_index(configuration(config.into_iter())?, command)? {
        Some(summary) => println!(
            "Auditoria concluída ({} coleções, {} documentos).",
            summary.collections, summary.documents
        ),
        None => println!(
            "Manutenção de índice concluída. Execute kiarni index audit antes de iniciar serve."
        ),
    }
    Ok(())
}

pub fn backup(args: impl Iterator<Item = OsString>) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = args;
    let mut output = None;
    let mut config = Vec::new();
    while let Some(key) = args.next() {
        if config.len() >= 64 {
            return Err("config_source_too_large".into());
        }
        let value = args.next().ok_or("missing_config_value")?;
        if key == "--output" {
            if output.replace(value).is_some() {
                return Err("duplicate_backup_output".into());
            }
        } else {
            config.push(key);
            config.push(value);
        }
    }
    let output = output.ok_or("missing_backup_output")?;
    let summary = kiarni_server::create_backup(
        configuration(config.into_iter())?,
        std::path::Path::new(&output),
    )?;
    println!(
        "Backup concluído ({} coleções, {} documentos).",
        summary.collections, summary.documents
    );
    Ok(())
}

pub fn restore(args: impl Iterator<Item = OsString>) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = args;
    let mut source = None;
    let mut config = Vec::new();
    while let Some(key) = args.next() {
        if config.len() >= 64 {
            return Err("config_source_too_large".into());
        }
        let value = args.next().ok_or("missing_config_value")?;
        if key == "--from" {
            if source.replace(value).is_some() {
                return Err("duplicate_restore_source".into());
            }
        } else {
            config.push(key);
            config.push(value);
        }
    }
    let source = source.ok_or("missing_restore_source")?;
    let summary = kiarni_server::restore_backup(
        configuration(config.into_iter())?,
        std::path::Path::new(&source),
    )?;
    println!(
        "Restauração concluída (coleções: {}; documentos: {}). Inicie serve na nova raiz para obter a nova credencial.",
        summary.collections, summary.documents
    );
    Ok(())
}

/// Resolve ordinary configuration while keeping credential values out of arguments.
pub fn healthcheck(
    mut args: impl Iterator<Item = OsString>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut token_file = None;
    let mut config = Vec::new();
    while let Some(key) = args.next() {
        if config.len() >= 64 {
            return Err("healthcheck_failed".into());
        }
        let value = args.next().ok_or("healthcheck_failed")?;
        if key == "--token-file" {
            if token_file.replace(value).is_some() {
                return Err("healthcheck_failed".into());
            }
        } else {
            config.push(key);
            config.push(value);
        }
    }
    kiarni_server::healthcheck(
        configuration(config.into_iter())?,
        token_file.as_deref().map(std::path::Path::new),
    )?;
    Ok(())
}
