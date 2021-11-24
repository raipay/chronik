use hyper::{header::CONTENT_TYPE, HeaderMap};

use bitcoinsuite_error::{ErrorMeta, Report};

use thiserror::Error;

#[derive(Debug, Error, ErrorMeta)]
pub enum ChronikValidationError {
    #[invalid_client_input()]
    #[error("No Content-Type set")]
    NoContentTypeSet,

    #[invalid_client_input()]
    #[error("Content-Type bad encoding: {0}")]
    BadContentType(String),

    #[invalid_client_input()]
    #[error("Content-Type must be {expected}, got {actual}")]
    WrongContentType {
        expected: &'static str,
        actual: String,
    },
}

use self::ChronikValidationError::*;

pub fn check_content_type(headers: &HeaderMap, expected: &'static str) -> Result<(), Report> {
    let content_type = headers.get(CONTENT_TYPE).ok_or(NoContentTypeSet)?;
    let content_type = content_type
        .to_str()
        .map_err(|err| BadContentType(err.to_string()))?;
    if content_type != expected {
        return Err(WrongContentType {
            expected,
            actual: content_type.to_string(),
        }
        .into());
    }
    Ok(())
}
