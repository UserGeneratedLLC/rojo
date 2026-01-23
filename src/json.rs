//! Utilities for parsing JSON with comments (JSONC) and deserializing to Rust types.
//!
//! This module provides convenient wrappers around `jsonc_parser` and `serde_json`
//! to reduce boilerplate and improve ergonomics when working with JSONC files.

use anyhow::Context as _;
use serde::{de::DeserializeOwned, Serialize};

/// Parse JSONC text into a `serde_json::Value`.
///
/// This handles the common pattern of calling `jsonc_parser::parse_to_serde_value`
/// and unwrapping the `Option` with a clear error message.
///
/// # Errors
///
/// Returns an error if:
/// - The text is not valid JSONC
/// - The text contains no JSON value
#[allow(dead_code)]
pub fn parse_value(text: &str) -> anyhow::Result<serde_json::Value> {
    json5::from_str(text).context("Failed to parse JSON5")
}

/// Parse JSONC text into a `serde_json::Value` with a custom context message.
///
/// This is useful when you want to provide a specific error message that includes
/// additional information like the file path.
///
/// # Errors
///
/// Returns an error if:
/// - The text is not valid JSONC
/// - The text contains no JSON value
pub fn parse_value_with_context(
    text: &str,
    context: impl Fn() -> String,
) -> anyhow::Result<serde_json::Value> {
    json5::from_str(text).with_context(|| format!("{}: JSON5 parse error", context()))
}

/// Parse JSONC text and deserialize it into a specific type.
///
/// This combines parsing JSONC and deserializing into a single operation,
/// eliminating the need to manually chain `parse_to_serde_value` and `from_value`.
///
/// # Errors
///
/// Returns an error if:
/// - The text is not valid JSONC
/// - The text contains no JSON value
/// - The value cannot be deserialized into type `T`
pub fn from_str<T: DeserializeOwned>(text: &str) -> anyhow::Result<T> {
    json5::from_str(text).context("Failed to deserialize JSON5")
}

/// Parse JSONC text and deserialize it into a specific type with a custom context message.
///
/// This is useful when you want to provide a specific error message that includes
/// additional information like the file path.
///
/// # Errors
///
/// Returns an error if:
/// - The text is not valid JSONC
/// - The text contains no JSON value
/// - The value cannot be deserialized into type `T`
pub fn from_str_with_context<T: DeserializeOwned>(
    text: &str,
    context: impl Fn() -> String,
) -> anyhow::Result<T> {
    json5::from_str(text).with_context(|| format!("{}: JSON5 parse error", context()))
}

/// Parse JSONC bytes into a `serde_json::Value` with a custom context message.
///
/// This handles UTF-8 conversion and JSONC parsing in one step.
///
/// # Errors
///
/// Returns an error if:
/// - The bytes are not valid UTF-8
/// - The text is not valid JSONC
/// - The text contains no JSON value
pub fn parse_value_from_slice_with_context(
    slice: &[u8],
    context: impl Fn() -> String,
) -> anyhow::Result<serde_json::Value> {
    let text = std::str::from_utf8(slice)
        .with_context(|| format!("{}: File is not valid UTF-8", context()))?;
    parse_value_with_context(text, context)
}

/// Parse JSONC bytes and deserialize it into a specific type.
///
/// This handles UTF-8 conversion, JSONC parsing, and deserialization in one step.
///
/// # Errors
///
/// Returns an error if:
/// - The bytes are not valid UTF-8
/// - The text is not valid JSONC
/// - The text contains no JSON value
/// - The value cannot be deserialized into type `T`
pub fn from_slice<T: DeserializeOwned>(slice: &[u8]) -> anyhow::Result<T> {
    let text = std::str::from_utf8(slice).context("File is not valid UTF-8")?;
    from_str(text)
}

/// Parse JSONC bytes and deserialize it into a specific type with a custom context message.
///
/// This handles UTF-8 conversion, JSONC parsing, and deserialization in one step.
///
/// # Errors
///
/// Returns an error if:
/// - The bytes are not valid UTF-8
/// - The text is not valid JSONC
/// - The text contains no JSON value
/// - The value cannot be deserialized into type `T`
pub fn from_slice_with_context<T: DeserializeOwned>(
    slice: &[u8],
    context: impl Fn() -> String,
) -> anyhow::Result<T> {
    let text = std::str::from_utf8(slice)
        .with_context(|| format!("{}: File is not valid UTF-8", context()))?;
    from_str_with_context(text, context)
}

/// A JSON5 value that uses BTreeMap for sorted keys and supports NaN/Infinity.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum Json5Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json5Value>),
    Object(std::collections::BTreeMap<String, Json5Value>),
}

/// Serialize a value to a JSON5 byte vector with sorted keys.
///
/// Supports NaN/Infinity and sorts object keys alphabetically.
///
/// # Errors
///
/// Returns an error if the value cannot be serialized to JSON5.
pub fn to_vec_pretty_sorted<T: Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    // Serialize to json5 string (preserves NaN/Infinity)
    let json5_str = json5::to_string(value).context("Failed to serialize to JSON5")?;
    
    // Parse into Json5Value which uses BTreeMap (sorted keys)
    let sorted: Json5Value = json5::from_str(&json5_str).context("Failed to parse JSON5 for sorting")?;
    
    // Serialize back to json5 (now sorted)
    let output = json5::to_string(&sorted).context("Failed to serialize sorted JSON5")?;
    Ok(output.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn test_parse_value() {
        let value = parse_value(r#"{"foo": "bar"}"#).unwrap();
        assert_eq!(value["foo"], "bar");
    }

    #[test]
    fn test_parse_value_with_comments() {
        let value = parse_value(
            r#"{
            // This is a comment
            "foo": "bar" // Inline comment
        }"#,
        )
        .unwrap();
        assert_eq!(value["foo"], "bar");
    }

    #[test]
    fn test_parse_value_with_trailing_comma() {
        let value = parse_value(
            r#"{
            "foo": "bar",
            "baz": 123,
        }"#,
        )
        .unwrap();
        assert_eq!(value["foo"], "bar");
        assert_eq!(value["baz"], 123);
    }

    #[test]
    fn test_parse_value_empty() {
        // Empty string should fail to parse
        assert!(parse_value("").is_err());
    }

    #[test]
    fn test_parse_value_invalid() {
        // Invalid JSON should fail to parse
        assert!(parse_value("{invalid}").is_err());
    }

    #[test]
    fn test_parse_value_with_context() {
        let err = parse_value_with_context("{invalid}", || "test.json".to_string()).unwrap_err();
        assert!(err.to_string().contains("test.json"));
        assert!(err.to_string().contains("parse"));
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestStruct {
        foo: String,
        bar: i32,
    }

    #[test]
    fn test_from_str() {
        let result: TestStruct = from_str(r#"{"foo": "hello", "bar": 42}"#).unwrap();
        assert_eq!(
            result,
            TestStruct {
                foo: "hello".to_string(),
                bar: 42
            }
        );
    }

    #[test]
    fn test_from_str_with_comments() {
        let result: TestStruct = from_str(
            r#"{
            // Comment
            "foo": "hello",
            "bar": 42, // Trailing comma is fine
        }"#,
        )
        .unwrap();
        assert_eq!(
            result,
            TestStruct {
                foo: "hello".to_string(),
                bar: 42
            }
        );
    }

    #[test]
    fn test_from_str_invalid_type() {
        let err = from_str::<TestStruct>(r#"{"foo": "hello"}"#).unwrap_err();
        assert!(err.to_string().contains("deserialize"));
    }

    #[test]
    fn test_from_str_with_context() {
        let err = from_str_with_context::<TestStruct>(r#"{"foo": "hello"}"#, || {
            "config.json".to_string()
        })
        .unwrap_err();
        assert!(err.to_string().contains("config.json"));
        assert!(err.to_string().contains("JSON5 parse error"));
    }

    #[test]
    fn test_parse_value_from_slice_with_context() {
        let err = parse_value_from_slice_with_context(b"{invalid}", || "test.json".to_string())
            .unwrap_err();
        assert!(err.to_string().contains("test.json"));
        assert!(err.to_string().contains("parse"));
    }

    #[test]
    fn test_parse_value_from_slice_with_context_invalid_utf8() {
        let err = parse_value_from_slice_with_context(&[0xFF, 0xFF], || "test.json".to_string())
            .unwrap_err();
        assert!(err.to_string().contains("test.json"));
        assert!(err.to_string().contains("UTF-8"));
    }

    #[test]
    fn test_from_slice() {
        let result: TestStruct = from_slice(br#"{"foo": "hello", "bar": 42}"#).unwrap();
        assert_eq!(
            result,
            TestStruct {
                foo: "hello".to_string(),
                bar: 42
            }
        );
    }

    #[test]
    fn test_from_slice_with_comments() {
        let result: TestStruct = from_slice(
            br#"{
            // Comment
            "foo": "hello",
            "bar": 42, // Trailing comma is fine
        }"#,
        )
        .unwrap();
        assert_eq!(
            result,
            TestStruct {
                foo: "hello".to_string(),
                bar: 42
            }
        );
    }

    #[test]
    fn test_from_slice_invalid_utf8() {
        let err = from_slice::<TestStruct>(&[0xFF, 0xFF]).unwrap_err();
        assert!(err.to_string().contains("UTF-8"));
    }

    #[test]
    fn test_from_slice_with_context() {
        let err = from_slice_with_context::<TestStruct>(br#"{"foo": "hello"}"#, || {
            "config.json".to_string()
        })
        .unwrap_err();
        assert!(err.to_string().contains("config.json"));
        assert!(err.to_string().contains("JSON5 parse error"));
    }

    #[test]
    fn test_from_slice_with_context_invalid_utf8() {
        let err =
            from_slice_with_context::<TestStruct>(&[0xFF, 0xFF], || "config.json".to_string())
                .unwrap_err();
        assert!(err.to_string().contains("config.json"));
        assert!(err.to_string().contains("UTF-8"));
    }

    #[test]
    fn test_to_vec_pretty_json5_format() {
        use indexmap::IndexMap;

        let mut map: IndexMap<String, i32> = IndexMap::new();
        map.insert("zebra".to_string(), 1);
        map.insert("apple".to_string(), 2);
        map.insert("mango".to_string(), 3);

        let result = to_vec_pretty_sorted(&map).unwrap();
        let output = String::from_utf8(result).unwrap();

        // JSON5 output should contain all keys (may be unquoted)
        assert!(output.contains("zebra"));
        assert!(output.contains("apple"));
        assert!(output.contains("mango"));
    }

    #[test]
    fn test_serialize_special_floats() {
        use serde::Serialize;

        #[derive(Serialize)]
        struct SpecialFloats {
            infinity: f64,
            neg_infinity: f64,
            nan: f64,
            normal: f64,
        }

        let data = SpecialFloats {
            infinity: f64::INFINITY,
            neg_infinity: f64::NEG_INFINITY,
            nan: f64::NAN,
            normal: 42.5,
        };

        let output = String::from_utf8(to_vec_pretty_sorted(&data).unwrap()).unwrap();

        // JSON5 should output special float values correctly
        assert!(output.contains("Infinity"));
        assert!(output.contains("-Infinity"));
        assert!(output.contains("NaN"));
        assert!(output.contains("42.5"));
    }

    // =========================================================================
    // JSON5 NaN/Infinity tests - critical for Roblox sync/syncback
    // =========================================================================
    //
    // Note: serde_json::Value cannot represent NaN/Infinity (as_f64 returns None).
    // However, direct deserialization into f64 fields works correctly.
    // This is fine for Rojo since properties are deserialized into typed structs.

    #[derive(Debug, Deserialize, PartialEq)]
    struct Vector3 {
        x: f64,
        y: f64,
        z: f64,
    }

    #[test]
    fn test_deserialize_infinity() {
        #[derive(Debug, Deserialize)]
        struct TestFloat { x: f64 }
        let result: TestFloat = from_str(r#"{ "x": Infinity }"#).unwrap();
        assert!(result.x.is_infinite() && result.x.is_sign_positive());
    }

    #[test]
    fn test_deserialize_negative_infinity() {
        #[derive(Debug, Deserialize)]
        struct TestFloat { x: f64 }
        let result: TestFloat = from_str(r#"{ "x": -Infinity }"#).unwrap();
        assert!(result.x.is_infinite() && result.x.is_sign_negative());
    }

    #[test]
    fn test_deserialize_nan() {
        #[derive(Debug, Deserialize)]
        struct TestFloat { x: f64 }
        let result: TestFloat = from_str(r#"{ "x": NaN }"#).unwrap();
        assert!(result.x.is_nan());
    }

    #[test]
    fn test_deserialize_vector_with_infinity() {
        let result: Vector3 = from_str(r#"{ "x": Infinity, "y": -Infinity, "z": 0 }"#).unwrap();
        assert!(result.x.is_infinite() && result.x.is_sign_positive());
        assert!(result.y.is_infinite() && result.y.is_sign_negative());
        assert_eq!(result.z, 0.0);
    }

    #[test]
    fn test_deserialize_vector_with_nan() {
        let result: Vector3 = from_str(r#"{ "x": NaN, "y": 1.5, "z": NaN }"#).unwrap();
        assert!(result.x.is_nan());
        assert_eq!(result.y, 1.5);
        assert!(result.z.is_nan());
    }

    #[derive(Debug, Deserialize)]
    struct CFrameLike {
        position: Vector3,
        orientation: [f64; 9],
    }

    #[test]
    fn test_deserialize_cframe_with_special_values() {
        // Simulates a CFrame with NaN/Infinity in position and rotation matrix
        let result: CFrameLike = from_str(
            r#"{
            "position": { "x": Infinity, "y": NaN, "z": -Infinity },
            "orientation": [1, 0, 0, 0, NaN, 0, 0, 0, Infinity]
        }"#,
        )
        .unwrap();

        assert!(result.position.x.is_infinite() && result.position.x.is_sign_positive());
        assert!(result.position.y.is_nan());
        assert!(result.position.z.is_infinite() && result.position.z.is_sign_negative());
        assert_eq!(result.orientation[0], 1.0);
        assert!(result.orientation[4].is_nan());
        assert!(result.orientation[8].is_infinite());
    }

    #[derive(Debug, Deserialize)]
    struct TypedProperties {
        #[serde(rename = "Size")]
        size: Vector3,
        #[serde(rename = "Transparency")]
        transparency: f64,
    }

    #[derive(Debug, Deserialize)]
    struct RojoPropertyTyped {
        #[serde(rename = "$className")]
        class_name: String,
        #[serde(rename = "$properties")]
        properties: TypedProperties,
    }

    #[test]
    fn test_deserialize_rojo_style_properties_with_special_floats() {
        // Simulates how Rojo project files might contain properties with NaN/Infinity
        let result: RojoPropertyTyped = from_str(
            r#"{
            "$className": "Part",
            "$properties": {
                "Size": { "x": Infinity, "y": NaN, "z": -Infinity },
                "Transparency": NaN
            }
        }"#,
        )
        .unwrap();

        assert_eq!(result.class_name, "Part");
        assert!(result.properties.size.x.is_infinite());
        assert!(result.properties.size.y.is_nan());
        assert!(result.properties.size.z.is_infinite());
        assert!(result.properties.transparency.is_nan());
    }

    #[test]
    fn test_json5_with_comments_and_special_floats() {
        // Combines JSON5 features: comments, trailing commas, and special float values
        let result: Vector3 = from_str(
            r#"{
            // Position with infinite X
            "x": Infinity,
            "y": -Infinity, // Negative infinity
            "z": NaN, // Not a number
        }"#,
        )
        .unwrap();

        assert!(result.x.is_infinite() && result.x.is_sign_positive());
        assert!(result.y.is_infinite() && result.y.is_sign_negative());
        assert!(result.z.is_nan());
    }

    #[test]
    fn test_from_slice_with_special_floats() {
        let result: Vector3 =
            from_slice(br#"{ "x": Infinity, "y": NaN, "z": -Infinity }"#).unwrap();
        assert!(result.x.is_infinite());
        assert!(result.y.is_nan());
        assert!(result.z.is_infinite());
    }

    #[test]
    fn test_deserialize_array_of_special_floats() {
        #[derive(Debug, Deserialize)]
        struct ArrayHolder {
            values: Vec<f64>,
        }
        let result: ArrayHolder =
            from_str(r#"{ "values": [1.0, Infinity, -Infinity, NaN, 0.0] }"#).unwrap();
        assert_eq!(result.values[0], 1.0);
        assert!(result.values[1].is_infinite() && result.values[1].is_sign_positive());
        assert!(result.values[2].is_infinite() && result.values[2].is_sign_negative());
        assert!(result.values[3].is_nan());
        assert_eq!(result.values[4], 0.0);
    }

    #[test]
    fn test_deserialize_nested_special_floats() {
        // Simulates nested structures like CFrame matrices
        #[derive(Debug, Deserialize)]
        struct Transform {
            position: [f64; 3],
            rotation: [[f64; 3]; 3],
        }
        let result: Transform = from_str(
            r#"{
            "position": [Infinity, NaN, -Infinity],
            "rotation": [
                [1, 0, NaN],
                [0, Infinity, 0],
                [-Infinity, 0, 1]
            ]
        }"#,
        )
        .unwrap();

        assert!(result.position[0].is_infinite());
        assert!(result.position[1].is_nan());
        assert!(result.position[2].is_infinite());
        assert!(result.rotation[0][2].is_nan());
        assert!(result.rotation[1][1].is_infinite());
        assert!(result.rotation[2][0].is_infinite());
    }

    #[test]
    fn test_mixed_json5_features_with_special_floats() {
        // All JSON5 features together: unquoted keys would work too but we use quoted for Rojo
        #[derive(Debug, Deserialize)]
        struct MixedTest {
            normal: f64,
            infinity: f64,
            negative_infinity: f64,
            nan_value: f64,
        }
        let result: MixedTest = from_str(
            r#"{
            // Regular number
            "normal": 42.5,
            "infinity": Infinity,     // Positive infinity
            "negative_infinity": -Infinity,
            "nan_value": NaN,         // Not a number
        }"#, // trailing comma
        )
        .unwrap();

        assert_eq!(result.normal, 42.5);
        assert!(result.infinity.is_infinite() && result.infinity.is_sign_positive());
        assert!(result.negative_infinity.is_infinite() && result.negative_infinity.is_sign_negative());
        assert!(result.nan_value.is_nan());
    }

    // =========================================================================
    // Syncback roundtrip tests - serialize then deserialize
    // =========================================================================

    #[test]
    fn test_syncback_roundtrip_special_floats() {
        use serde::Serialize;

        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct SyncbackData {
            normal: f64,
            infinity: f64,
            neg_infinity: f64,
        }

        let original = SyncbackData {
            normal: 42.5,
            infinity: f64::INFINITY,
            neg_infinity: f64::NEG_INFINITY,
        };

        // Serialize with json5
        let serialized = to_vec_pretty_sorted(&original).unwrap();
        let json_str = String::from_utf8(serialized).unwrap();

        // Verify JSON5 output contains special values
        assert!(json_str.contains("Infinity"));
        assert!(json_str.contains("-Infinity"));

        // Deserialize back
        let deserialized: SyncbackData = from_str(&json_str).unwrap();

        assert_eq!(deserialized.normal, original.normal);
        assert!(deserialized.infinity.is_infinite() && deserialized.infinity.is_sign_positive());
        assert!(deserialized.neg_infinity.is_infinite() && deserialized.neg_infinity.is_sign_negative());
    }

    #[test]
    fn test_syncback_roundtrip_nan() {
        use serde::Serialize;

        #[derive(Debug, Serialize, Deserialize)]
        struct NanData {
            value: f64,
        }

        let original = NanData { value: f64::NAN };

        // Serialize with json5
        let serialized = to_vec_pretty_sorted(&original).unwrap();
        let json_str = String::from_utf8(serialized).unwrap();

        // Verify JSON5 output contains NaN
        assert!(json_str.contains("NaN"));

        // Deserialize back
        let deserialized: NanData = from_str(&json_str).unwrap();
        assert!(deserialized.value.is_nan());
    }

    #[test]
    fn test_syncback_cframe_like_roundtrip() {
        use serde::Serialize;

        #[derive(Debug, Serialize, Deserialize)]
        struct CFrameData {
            position: [f64; 3],
            orientation: [[f64; 3]; 3],
        }

        let original = CFrameData {
            position: [f64::INFINITY, f64::NAN, f64::NEG_INFINITY],
            orientation: [
                [1.0, 0.0, f64::NAN],
                [0.0, f64::INFINITY, 0.0],
                [f64::NEG_INFINITY, 0.0, 1.0],
            ],
        };

        // Serialize
        let serialized = to_vec_pretty_sorted(&original).unwrap();
        let json_str = String::from_utf8(serialized).unwrap();

        // Deserialize
        let deserialized: CFrameData = from_str(&json_str).unwrap();

        // Verify position
        assert!(deserialized.position[0].is_infinite() && deserialized.position[0].is_sign_positive());
        assert!(deserialized.position[1].is_nan());
        assert!(deserialized.position[2].is_infinite() && deserialized.position[2].is_sign_negative());

        // Verify orientation
        assert_eq!(deserialized.orientation[0][0], 1.0);
        assert!(deserialized.orientation[0][2].is_nan());
        assert!(deserialized.orientation[1][1].is_infinite());
        assert!(deserialized.orientation[2][0].is_infinite() && deserialized.orientation[2][0].is_sign_negative());
    }

    #[test]
    fn test_syncback_rojo_meta_like_roundtrip() {
        use serde::Serialize;
        use std::collections::BTreeMap;

        #[derive(Debug, Serialize, Deserialize)]
        struct MetaFile {
            #[serde(rename = "className")]
            class_name: String,
            properties: BTreeMap<String, f64>,
        }

        let mut properties = BTreeMap::new();
        properties.insert("Transparency".to_string(), f64::NAN);
        properties.insert("Health".to_string(), f64::INFINITY);
        properties.insert("WalkSpeed".to_string(), 16.0);

        let original = MetaFile {
            class_name: "Humanoid".to_string(),
            properties,
        };

        // Serialize
        let serialized = to_vec_pretty_sorted(&original).unwrap();
        let json_str = String::from_utf8(serialized).unwrap();

        // Verify output format
        assert!(json_str.contains("Humanoid"));
        assert!(json_str.contains("NaN"));
        assert!(json_str.contains("Infinity"));

        // Deserialize
        let deserialized: MetaFile = from_str(&json_str).unwrap();

        assert_eq!(deserialized.class_name, "Humanoid");
        assert!(deserialized.properties.get("Transparency").unwrap().is_nan());
        assert!(deserialized.properties.get("Health").unwrap().is_infinite());
        assert_eq!(*deserialized.properties.get("WalkSpeed").unwrap(), 16.0);
    }
}
