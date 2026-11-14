use kiarni_http::Code;
use kiarni_storage::Error;

pub fn pagination(query: Option<&str>) -> Result<(usize, Option<&str>), Code> {
    let mut limit = None;
    let mut after = None;
    for pair in query.ok_or(Code::InvalidLimit)?.split('&') {
        let (key, value) = pair.split_once('=').ok_or(Code::InvalidLimit)?;
        match key {
            "limit" if limit.is_none() => {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(Code::InvalidLimit);
                }
                let value = value.parse::<usize>().map_err(|_| Code::InvalidLimit)?;
                if !(1..=1000).contains(&value) {
                    return Err(Code::InvalidLimit);
                }
                limit = Some(value);
            }
            "after" if after.is_none() => {
                if value.is_empty()
                    || value.len() > 128
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                {
                    return Err(Code::InvalidCollection);
                }
                after = Some(value);
            }
            _ => return Err(Code::InvalidLimit),
        }
    }
    Ok((limit.ok_or(Code::InvalidLimit)?, after))
}

pub fn storage_error(error: Error) -> Code {
    match error {
        Error::CollectionExists => Code::CollectionExists,
        Error::CollectionMissing => Code::CollectionNotFound,
        Error::CollectionNotEmpty => Code::CollectionNotEmpty,
        Error::InvalidName => Code::InvalidCollection,
        Error::InvalidLimit => Code::InvalidLimit,
        Error::QueryLimit => Code::QueryTooComplex,
        Error::InvalidDocument => Code::InvalidDocument,
        Error::MissingDocument => Code::DocumentNotFound,
        Error::Write(error) => match error {
            kiarni_engine::writes::Error::Document(error) => Code::from(error),
            kiarni_engine::writes::Error::InvalidRouteId => Code::InvalidId,
            kiarni_engine::writes::Error::IdMismatch => Code::IdMismatch,
            kiarni_engine::writes::Error::ImmutableId => Code::ImmutableId,
            kiarni_engine::writes::Error::InvalidCurrentDocument => Code::InternalError,
        },
        Error::DuplicateId => Code::DuplicateId,
        Error::CollectionIdsExhausted => Code::ServiceUnavailable,
        Error::Io
        | Error::InvalidManifest
        | Error::UnsupportedFormat
        | Error::InvalidLayout
        | Error::Storage => Code::StorageUnavailable,
    }
}

pub fn json_media_type(values: &[&[u8]], merge_patch: bool) -> Result<Option<&'static str>, Code> {
    let expected = if merge_patch {
        "application/merge-patch+json"
    } else {
        "application/json"
    };
    let [value] = values else {
        return if values.is_empty() {
            Ok(None)
        } else {
            Err(Code::UnsupportedMediaType)
        };
    };
    let value = std::str::from_utf8(value).map_err(|_| Code::UnsupportedMediaType)?;
    let mut parts = value.split(';');
    if !parts
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(expected))
    {
        return Err(Code::UnsupportedMediaType);
    }
    let mut charset = false;
    for parameter in parts {
        let (key, value) = parameter
            .trim()
            .split_once('=')
            .ok_or(Code::UnsupportedMediaType)?;
        if charset
            || !key.trim().eq_ignore_ascii_case("charset")
            || !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "utf-8" | "\"utf-8\""
            )
        {
            return Err(Code::UnsupportedMediaType);
        }
        charset = true;
    }
    Ok(Some(expected))
}
