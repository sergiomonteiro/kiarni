//! Query AST validation and bounded evaluation.
mod number;

use kiarni_core::DocumentInput;
use number::Decimal;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fmt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    QueryTooLarge,
    InvalidJson,
    InvalidQuery,
    InvalidLimit,
    QueryTooComplex,
    UnsupportedNumber,
    UnsupportedSortValue,
    InvalidDataset,
    DatasetLimit,
    ResultTooLarge,
}

impl Error {
    pub const fn code(self) -> &'static str {
        match self {
            Self::QueryTooLarge => "query_too_large",
            Self::InvalidJson => "invalid_json",
            Self::InvalidQuery => "invalid_query",
            Self::InvalidLimit => "invalid_limit",
            Self::QueryTooComplex => "query_too_complex",
            Self::UnsupportedNumber => "unsupported_number",
            Self::UnsupportedSortValue => "unsupported_sort_value",
            Self::InvalidDataset => "invalid_dataset",
            Self::DatasetLimit => "dataset_limit",
            Self::ResultTooLarge => "result_too_large",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for Error {}

// Missing is None; explicit null is deserialized as T and cannot silently omit
// a field. RawValue itself accepts null as a comparison literal.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ast {
    from: String,
    limit: usize,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    id: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    filter: Option<Predicate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sort: Vec<Sort>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    project: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Op {
    Eq,
    Lt,
    Lte,
    Gt,
    Gte,
    Exists,
    All,
    Any,
    Not,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Predicate {
    op: Op,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    field: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    value: Option<Box<RawValue>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    args: Option<Vec<Predicate>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    arg: Option<Box<Predicate>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sort {
    field: String,
    direction: Direction,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Asc,
    Desc,
}

pub struct Query(Ast);

impl fmt::Debug for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Query { .. }")
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn field(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.bytes().any(|b| b.is_ascii_control())
}

impl Query {
    pub fn parse(input: &[u8]) -> Result<Self, Error> {
        if input.len() > 16 * 1024 {
            return Err(Error::QueryTooLarge);
        }
        // Reuse recursive syntax/duplicate-key validation without a Value tree.
        DocumentInput::parse(input, 16 * 1024).map_err(|error| match error {
            kiarni_core::DocumentError::InvalidJson => Error::InvalidJson,
            _ => Error::InvalidQuery,
        })?;
        let ast: Ast = serde_json::from_slice(input).map_err(|_| Error::InvalidQuery)?;
        if !identifier(&ast.from) || ast.id.as_deref().is_some_and(|id| !identifier(id)) {
            return Err(Error::InvalidQuery);
        }
        if !(1..=1000).contains(&ast.limit) {
            return Err(Error::InvalidLimit);
        }
        if ast.sort.len() > 4 {
            return Err(Error::QueryTooComplex);
        }
        let mut seen = BTreeSet::new();
        for sort in &ast.sort {
            if !field(&sort.field) || !seen.insert(&sort.field) {
                return Err(Error::InvalidQuery);
            }
        }
        if let Some(project) = &ast.project {
            if project.len() > 32 {
                return Err(Error::QueryTooComplex);
            }
            let mut seen = BTreeSet::new();
            if project
                .iter()
                .any(|name| !field(name) || !seen.insert(name))
            {
                return Err(Error::InvalidQuery);
            }
        }
        if let Some(filter) = &ast.filter {
            filter.validate(1, &mut 0)?;
        }
        Ok(Self(ast))
    }

    pub fn collection(&self) -> &str {
        &self.0.from
    }

    pub fn id(&self) -> Option<&str> {
        self.0.id.as_deref()
    }

    /// A direct string equality on a literal root field, for access selection.
    /// This does not choose a plan or impose storage-specific index limits.
    pub fn direct_string_equality(&self) -> Result<Option<(&str, String)>, Error> {
        let Some(filter) = &self.0.filter else {
            return Ok(None);
        };
        if !matches!(filter.op, Op::Eq) {
            return Ok(None);
        }
        let field = filter.field.as_deref().ok_or(Error::InvalidQuery)?;
        let value = filter.value.as_ref().ok_or(Error::InvalidQuery)?;
        if kind(value.get()) != 4 {
            return Ok(None);
        }
        let value = serde_json::from_str(value.get()).map_err(|_| Error::InvalidQuery)?;
        Ok(Some((field, value)))
    }

    pub fn to_json(&self) -> Result<Vec<u8>, Error> {
        serde_json::to_vec(&self.0).map_err(|_| Error::InvalidQuery)
    }

    /// Fixture evaluator: caller provides one collection, never a storage scan.
    /// All errors abort without returning partial output.
    pub fn evaluate_fixture(&self, documents: &[DocumentInput]) -> Result<Vec<Vec<u8>>, Error> {
        if documents.len() > 256 {
            return Err(Error::DatasetLimit);
        }
        self.evaluate_documents(documents)
    }

    /// Bounded execution over an owned snapshot supplied by storage.
    /// The caller limits raw input bytes before parsing each document.
    pub fn evaluate_documents(&self, documents: &[DocumentInput]) -> Result<Vec<Vec<u8>>, Error> {
        if documents.len() > 10000 {
            return Err(Error::DatasetLimit);
        }
        let mut bytes = 0usize;
        let mut ids = BTreeSet::new();
        for document in documents {
            let id = document.id().ok_or(Error::InvalidDataset)?;
            if !ids.insert(id) {
                return Err(Error::InvalidDataset);
            }
            for (key, value) in document.fields() {
                bytes = bytes
                    .saturating_add(key.len())
                    .saturating_add(value.get().len());
                if bytes > 8 * 1024 * 1024 {
                    return Err(Error::DatasetLimit);
                }
            }
        }
        let mut selected = Vec::new();
        for document in documents {
            let id = document.id().ok_or(Error::InvalidDataset)?;
            if self.0.id.as_deref().is_some_and(|wanted| wanted != id) {
                continue;
            }
            if let Some(filter) = &self.0.filter
                && !filter.matches(document)?
            {
                continue;
            }
            let keys = self
                .0
                .sort
                .iter()
                .map(|sort| Scalar::parse(document.fields().get(&sort.field).map(|raw| raw.get())))
                .collect::<Result<Vec<_>, _>>()?;
            selected.push((document, id, keys));
        }
        selected.sort_by(|a, b| {
            for ((left, right), sort) in a.2.iter().zip(&b.2).zip(&self.0.sort) {
                let order = left.compare(right);
                let order = if matches!(sort.direction, Direction::Desc) {
                    order.reverse()
                } else {
                    order
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
            a.1.cmp(b.1)
        });
        let mut output = Vec::new();
        let mut bytes = 0usize;
        for (document, _, _) in selected.into_iter().take(self.0.limit) {
            let fields: BTreeMap<_, _> = document
                .fields()
                .iter()
                .filter(|(name, _)| {
                    self.0
                        .project
                        .as_ref()
                        .is_none_or(|project| name.as_str() == "_id" || project.contains(name))
                })
                .collect();
            let json = serde_json::to_vec(&fields).map_err(|_| Error::InvalidDataset)?;
            bytes += json.len();
            if bytes > 4 * 1024 * 1024 {
                return Err(Error::ResultTooLarge);
            }
            output.push(json);
        }
        Ok(output)
    }
}

impl Predicate {
    fn validate(&self, depth: usize, count: &mut usize) -> Result<(), Error> {
        *count += 1;
        if depth > 8 || *count > 64 {
            return Err(Error::QueryTooComplex);
        }
        match self.op {
            Op::All | Op::Any => {
                if self.field.is_some() || self.value.is_some() || self.arg.is_some() {
                    return Err(Error::InvalidQuery);
                }
                let args = self
                    .args
                    .as_ref()
                    .filter(|args| !args.is_empty())
                    .ok_or(Error::InvalidQuery)?;
                for arg in args {
                    arg.validate(depth + 1, count)?;
                }
            }
            Op::Not => {
                if self.field.is_some() || self.value.is_some() || self.args.is_some() {
                    return Err(Error::InvalidQuery);
                }
                self.arg
                    .as_ref()
                    .ok_or(Error::InvalidQuery)?
                    .validate(depth + 1, count)?;
            }
            _ => {
                if self.args.is_some()
                    || self.arg.is_some()
                    || !self.field.as_deref().is_some_and(field)
                {
                    return Err(Error::InvalidQuery);
                }
                if matches!(self.op, Op::Exists) {
                    if self.value.is_some() {
                        return Err(Error::InvalidQuery);
                    }
                } else {
                    let value = self.value.as_ref().ok_or(Error::InvalidQuery)?;
                    if kind(value.get()) == 5 {
                        return Err(Error::InvalidQuery);
                    }
                    Scalar::parse(Some(value.get()))?;
                }
            }
        }
        Ok(())
    }

    fn matches(&self, document: &DocumentInput) -> Result<bool, Error> {
        match self.op {
            Op::All | Op::Any => {
                let args = self.args.as_ref().ok_or(Error::InvalidQuery)?;
                let all = matches!(self.op, Op::All);
                for arg in args {
                    if arg.matches(document)? != all {
                        return Ok(!all);
                    }
                }
                Ok(all)
            }
            Op::Not => Ok(!self
                .arg
                .as_ref()
                .ok_or(Error::InvalidQuery)?
                .matches(document)?),
            _ => {
                let name = self.field.as_ref().ok_or(Error::InvalidQuery)?;
                let value = document.fields().get(name);
                if matches!(self.op, Op::Exists) {
                    return Ok(value.is_some());
                }
                let Some(value) = value else {
                    return Ok(false);
                };
                let literal = self.value.as_ref().ok_or(Error::InvalidQuery)?;
                let left_kind = kind(value.get());
                if left_kind != kind(literal.get()) || left_kind == 5 {
                    return Ok(false);
                }
                if !matches!(self.op, Op::Eq) && !matches!(left_kind, 3 | 4) {
                    return Ok(false);
                }
                let order =
                    Scalar::parse(Some(value.get()))?.compare(&Scalar::parse(Some(literal.get()))?);
                Ok(match self.op {
                    Op::Eq => order == Ordering::Equal,
                    Op::Lt => order == Ordering::Less,
                    Op::Lte => order != Ordering::Greater,
                    Op::Gt => order == Ordering::Greater,
                    Op::Gte => order != Ordering::Less,
                    _ => false,
                })
            }
        }
    }
}

fn kind(raw: &str) -> u8 {
    match raw.trim().as_bytes().first() {
        Some(b'n') => 1,
        Some(b't' | b'f') => 2,
        Some(b'"') => 4,
        Some(b'{' | b'[') => 5,
        _ => 3,
    }
}

enum Scalar {
    Missing,
    Null,
    Bool(bool),
    Number(Decimal),
    String(String),
}

impl Scalar {
    fn parse(raw: Option<&str>) -> Result<Self, Error> {
        let Some(raw) = raw else {
            return Ok(Self::Missing);
        };
        Ok(match kind(raw) {
            1 => Self::Null,
            2 => Self::Bool(raw.trim() == "true"),
            3 => Self::Number(Decimal::parse(raw.trim())?),
            4 => Self::String(serde_json::from_str(raw).map_err(|_| Error::InvalidQuery)?),
            _ => return Err(Error::UnsupportedSortValue),
        })
    }

    fn rank(&self) -> u8 {
        match self {
            Self::Missing => 0,
            Self::Null => 1,
            Self::Bool(_) => 2,
            Self::Number(_) => 3,
            Self::String(_) => 4,
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        self.rank()
            .cmp(&other.rank())
            .then_with(|| match (self, other) {
                (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
                (Self::Number(a), Self::Number(b)) => a.compare(b),
                (Self::String(a), Self::String(b)) => a.cmp(b),
                _ => Ordering::Equal,
            })
    }
}
