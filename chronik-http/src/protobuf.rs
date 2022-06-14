use async_trait::async_trait;
use axum::{
    extract::{FromRequest, RequestParts},
    http::HeaderValue,
    response::{IntoResponse, Response},
};
use bitcoinsuite_error::ErrorMeta;
use hyper::{body::to_bytes, header::CONTENT_TYPE, Body};
use prost::Message;
use thiserror::Error;

use crate::{error::ReportError, validation::check_content_type};

pub struct Protobuf<P: Message + Default>(pub P);

pub const CONTENT_TYPE_PROTOBUF: &str = "application/x-protobuf";

#[derive(Debug, Error, ErrorMeta)]
pub enum ChronikProtobufError {
    #[invalid_client_input()]
    #[error("Invalid body: {0}")]
    InvalidBody(String),

    #[invalid_client_input()]
    #[error("Bad protobuf: {0}")]
    BadProtobuf(String),
}

use self::ChronikProtobufError::*;

#[async_trait]
impl<P: Message + Default> FromRequest<Body> for Protobuf<P> {
    type Rejection = ReportError;

    async fn from_request(req: &mut RequestParts<Body>) -> Result<Self, Self::Rejection> {
        let headers = req.headers();
        check_content_type(headers, CONTENT_TYPE_PROTOBUF)?;
        let mut body = req.take_body().expect("Body taken");
        let mut body_bytes = to_bytes(&mut body)
            .await
            .map_err(|err| InvalidBody(err.to_string()))?;
        let proto = P::decode(&mut body_bytes).map_err(|err| BadProtobuf(err.to_string()))?;
        Ok(Protobuf(proto))
    }
}

impl<P: Message + Default> IntoResponse for Protobuf<P> {
    fn into_response(self) -> Response {
        let mut response = Response::builder()
            .body(axum::body::boxed(Body::from(self.0.encode_to_vec())))
            .unwrap();
        response.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static(CONTENT_TYPE_PROTOBUF),
        );
        response
    }
}
