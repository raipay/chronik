use axum::{http::StatusCode, response::IntoResponse};
use bitcoinsuite_error::{report_to_details, ErrorMeta, ErrorSeverity, Report};

use crate::{
    convert::ChronikConvertError,
    proto,
    protobuf::{ChronikProtobufError, Protobuf},
    server::ChronikServerError,
    validation::ChronikValidationError,
};

pub struct ReportError(pub Report);

impl From<Report> for ReportError {
    fn from(err: Report) -> Self {
        ReportError(err)
    }
}

impl From<ChronikConvertError> for ReportError {
    fn from(err: ChronikConvertError) -> Self {
        ReportError(err.into())
    }
}

impl From<ChronikProtobufError> for ReportError {
    fn from(err: ChronikProtobufError) -> Self {
        ReportError(err.into())
    }
}

impl From<ChronikValidationError> for ReportError {
    fn from(err: ChronikValidationError) -> Self {
        ReportError(err.into())
    }
}

impl From<ChronikServerError> for ReportError {
    fn from(err: ChronikServerError) -> Self {
        ReportError(err.into())
    }
}

impl IntoResponse for ReportError {
    type Body = hyper::Body;
    type BodyError = <Self::Body as axum::body::HttpBody>::Error;

    fn into_response(self) -> hyper::Response<Self::Body> {
        let details = report_to_details(&self.0, self::report_to_error_meta);
        let response = match details.severity {
            ErrorSeverity::NotFound => (
                StatusCode::NOT_FOUND,
                Protobuf(proto::Error {
                    error_code: details.error_code.to_string(),
                    msg: details.msg,
                    is_user_error: true,
                }),
            ),
            ErrorSeverity::InvalidUserInput => (
                StatusCode::BAD_REQUEST,
                Protobuf(proto::Error {
                    error_code: details.error_code.to_string(),
                    msg: details.msg,
                    is_user_error: true,
                }),
            ),
            ErrorSeverity::InvalidClientInput => {
                println!("Invalid client input: {}", details.msg);
                (
                    StatusCode::BAD_REQUEST,
                    Protobuf(proto::Error {
                        error_code: details.error_code.to_string(),
                        msg: details.msg,
                        is_user_error: false,
                    }),
                )
            }
            ErrorSeverity::Critical
            | ErrorSeverity::Unknown
            | ErrorSeverity::Bug
            | ErrorSeverity::Warning => {
                println!("Unhandled error ({:?}):", details.severity);
                println!("{}", details.full_debug_report);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Protobuf(proto::Error {
                        error_code: "internal-server-error".into(),
                        msg: "Internal server error".to_string(),
                        is_user_error: false,
                    }),
                )
            }
        };
        response.into_response()
    }
}

pub fn report_to_error_meta(report: &Report) -> Option<&dyn ErrorMeta> {
    if let Some(err) = report.downcast_ref::<ChronikConvertError>() {
        Some(err)
    } else if let Some(err) = report.downcast_ref::<ChronikProtobufError>() {
        Some(err)
    } else if let Some(err) = report.downcast_ref::<ChronikServerError>() {
        Some(err)
    } else if let Some(err) = report.downcast_ref::<ChronikValidationError>() {
        Some(err)
    } else {
        None
    }
}
