//! Кодирование тел HTTP-запросов и ответов (postcard) и построение
//! ошибок протокола.

use axum::{
    body::Bytes,
    http::{StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use forup_core::api::auth::ApiError;
use serde::{Serialize, de::DeserializeOwned};

const BINARY: &str = "application/octet-stream";

pub(crate) fn decode<T: DeserializeOwned>(body: &Bytes) -> Result<T, Response> {
    postcard::from_bytes(body.as_ref()).map_err(|error| {
        api_error(
            StatusCode::BAD_REQUEST,
            format!("invalid request body: {error}"),
        )
    })
}

pub(crate) fn encode<T: Serialize>(status: StatusCode, value: &T) -> Response {
    match postcard::to_allocvec(value) {
        Ok(body) => (status, [(CONTENT_TYPE, BINARY)], body).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("response encoding error: {error}"),
        )
            .into_response(),
    }
}

pub(crate) fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    encode(
        status,
        &ApiError {
            error: message.into(),
        },
    )
}
