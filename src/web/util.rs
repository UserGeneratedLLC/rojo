use bytes::Bytes;
use http_body_util::Full;
use hyper::{header::CONTENT_TYPE, Response, StatusCode};
use serde::{Deserialize, Serialize};

pub fn msgpack_ok<T: Serialize>(value: T) -> Response<Full<Bytes>> {
    msgpack(value, StatusCode::OK)
}

pub fn msgpack<T: Serialize>(value: T, code: StatusCode) -> Response<Full<Bytes>> {
    let mut serialized = Vec::new();
    let mut serializer = rmp_serde::Serializer::new(&mut serialized)
        .with_human_readable()
        .with_struct_map();

    if let Err(err) = value.serialize(&mut serializer) {
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(CONTENT_TYPE, "text/plain")
            .body(Full::new(Bytes::from(err.to_string())))
            .unwrap();
    };

    Response::builder()
        .status(code)
        .header(CONTENT_TYPE, "application/msgpack")
        .body(Full::new(Bytes::from(serialized)))
        .unwrap()
}

pub fn serialize_msgpack<T: Serialize>(value: T) -> anyhow::Result<Vec<u8>> {
    let mut serialized = Vec::new();
    let mut serializer = rmp_serde::Serializer::new(&mut serialized)
        .with_human_readable()
        .with_struct_map();

    value.serialize(&mut serializer)?;

    Ok(serialized)
}

pub fn deserialize_msgpack<'a, T: Deserialize<'a>>(
    input: &'a [u8],
) -> Result<T, rmp_serde::decode::Error> {
    let mut deserializer = rmp_serde::Deserializer::new(input).with_human_readable();

    T::deserialize(&mut deserializer)
}

pub fn json<T: Serialize>(value: T, code: StatusCode) -> Response<Full<Bytes>> {
    let serialized = match serde_json::to_string(&value) {
        Ok(v) => v,
        Err(err) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(CONTENT_TYPE, "text/plain")
                .body(Full::new(Bytes::from(err.to_string())))
                .unwrap();
        }
    };

    Response::builder()
        .status(code)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(serialized)))
        .unwrap()
}
