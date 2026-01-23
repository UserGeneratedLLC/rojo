use hyper::{header::CONTENT_TYPE, Body, Response, StatusCode};
use serde::Serialize;

use crate::json as json_module;

pub fn json_ok<T: Serialize>(value: T) -> Response<Body> {
    json_response(value, StatusCode::OK)
}

/// Alias for backwards compatibility
pub use json_response as json;

pub fn json_response<T: Serialize>(value: T, code: StatusCode) -> Response<Body> {
    // Use compact JSON5 for transport efficiency, preserves NaN/Infinity
    let serialized = match json_module::to_string_compact(&value) {
        Ok(v) => v,
        Err(err) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(CONTENT_TYPE, "text/plain")
                .body(Body::from(err.to_string()))
                .unwrap();
        }
    };

    Response::builder()
        .status(code)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(serialized))
        .unwrap()
}
