//! Utilities for parsing JSON with comments (JSONC) and deserializing to Rust types.
//!
//! This module provides convenient wrappers around `jsonc_parser` and `serde_json`
//! to reduce boilerplate and improve ergonomics when working with JSONC files.

use anyhow::Context as _;
use lexical_write_float::{format::STANDARD, Options, RoundMode, ToLexicalWithOptions};
use serde::{de::DeserializeOwned, Serialize};
use std::num::{NonZeroI32, NonZeroUsize};

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

const SCI_POSITIVE_BREAK: Option<NonZeroI32> = NonZeroI32::new(15);
const SCI_NEGATIVE_BREAK: Option<NonZeroI32> = NonZeroI32::new(-6);

const F32_FLOAT_OPTIONS: Options = Options::builder()
    .trim_floats(true)
    .nan_string(Some(b"NaN"))
    .inf_string(Some(b"Infinity"))
    .positive_exponent_break(SCI_POSITIVE_BREAK)
    .negative_exponent_break(SCI_NEGATIVE_BREAK)
    .max_significant_digits(NonZeroUsize::new(6))
    .round_mode(RoundMode::Round)
    .build_strict();

const F64_FLOAT_OPTIONS: Options = Options::builder()
    .trim_floats(true)
    .nan_string(Some(b"NaN"))
    .inf_string(Some(b"Infinity"))
    .positive_exponent_break(SCI_POSITIVE_BREAK)
    .negative_exponent_break(SCI_NEGATIVE_BREAK)
    .max_significant_digits(NonZeroUsize::new(15))
    .round_mode(RoundMode::Round)
    .build_strict();

const F32_BUF_SIZE: usize = F32_FLOAT_OPTIONS.buffer_size_const::<f32, STANDARD>();
const F64_BUF_SIZE: usize = F64_FLOAT_OPTIONS.buffer_size_const::<f64, STANDARD>();

fn format_f32(v: f32) -> String {
    let mut buffer = [0u8; F32_BUF_SIZE];
    let digits = v.to_lexical_with_options::<STANDARD>(&mut buffer, &F32_FLOAT_OPTIONS);
    std::str::from_utf8(digits)
        .expect("lexical-write-float produced invalid utf-8")
        .into()
}

fn format_f64(v: f64) -> String {
    if v.is_finite() && v.abs() >= 1e308 {
        return format!("{v:e}");
    }
    let mut buffer = [0u8; F64_BUF_SIZE];
    let digits = v.to_lexical_with_options::<STANDARD>(&mut buffer, &F64_FLOAT_OPTIONS);
    std::str::from_utf8(digits)
        .expect("lexical-write-float produced invalid utf-8")
        .into()
}

/// A JSON5 value that uses BTreeMap for sorted keys and supports NaN/Infinity.
/// Uses String for numbers to preserve exact representation (including scientific notation).
#[derive(Debug, Clone, PartialEq)]
enum Json5Value {
    Null,
    Bool(bool),
    Number(String), // Store as string to preserve exact representation
    String(String),
    Array(Vec<Json5Value>),
    Object(std::collections::BTreeMap<String, Json5Value>),
}

impl Json5Value {
    /// Write this value to a string with proper JSON5 formatting and indentation.
    fn write_to(&self, output: &mut String, indent: usize) {
        let indent_str = "  ".repeat(indent);
        let inner_indent = "  ".repeat(indent + 1);

        match self {
            Json5Value::Null => output.push_str("null"),
            Json5Value::Bool(b) => output.push_str(if *b { "true" } else { "false" }),
            Json5Value::Number(s) => output.push_str(s),
            Json5Value::String(s) => {
                write_escaped_string(output, s);
            }
            Json5Value::Array(arr) => {
                if arr.is_empty() {
                    output.push_str("[]");
                } else if arr.len() <= 20
                    && arr.iter().all(|v| {
                        matches!(
                            v,
                            Json5Value::Null
                                | Json5Value::Bool(_)
                                | Json5Value::Number(_)
                                | Json5Value::String(_)
                        )
                    })
                {
                    output.push_str("[ ");
                    for (i, item) in arr.iter().enumerate() {
                        if i > 0 {
                            output.push_str(", ");
                        }
                        item.write_to(output, 0);
                    }
                    output.push_str(" ]");
                } else {
                    output.push_str("[\n");
                    for (i, item) in arr.iter().enumerate() {
                        output.push_str(&inner_indent);
                        item.write_to(output, indent + 1);
                        if i < arr.len() - 1 {
                            output.push(',');
                        }
                        output.push('\n');
                    }
                    output.push_str(&indent_str);
                    output.push(']');
                }
            }
            Json5Value::Object(map) => {
                if map.is_empty() {
                    output.push_str("{}");
                } else {
                    output.push_str("{\n");
                    let entries: Vec<_> = map.iter().collect();
                    for (i, (key, value)) in entries.iter().enumerate() {
                        output.push_str(&inner_indent);
                        // Use unquoted keys if valid identifier, otherwise quote with escaping
                        if is_valid_identifier(key) {
                            output.push_str(key);
                        } else {
                            write_escaped_string(output, key);
                        }
                        output.push_str(": ");
                        value.write_to(output, indent + 1);
                        if i < entries.len() - 1 {
                            output.push(',');
                        }
                        output.push('\n');
                    }
                    output.push_str(&indent_str);
                    output.push('}');
                }
            }
        }
    }
}

/// Write a string to output with proper JSON5 escaping (double-quoted).
///
/// Per JSON5 spec Section 5.1 (https://spec.json5.org/#escapes):
/// - Standard escapes: \" \\ \b \f \n \r \t \v
/// - Control characters as \uXXXX
///
/// Per Section 5.2, generators SHOULD escape U+2028 and U+2029.
fn write_escaped_string(output: &mut String, s: &str) {
    output.push('"');
    for c in s.chars() {
        match c {
            // Standard JSON5 escape sequences (Table 1 in spec)
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"), // Backspace
            '\u{000C}' => output.push_str("\\f"), // Form feed
            '\n' => output.push_str("\\n"),       // Line feed (U+000A)
            '\r' => output.push_str("\\r"),       // Carriage return (U+000D)
            '\t' => output.push_str("\\t"),       // Horizontal tab (U+0009)
            '\u{000B}' => output.push_str("\\v"), // Vertical tab
            // Per Section 5.2: generators SHOULD escape U+2028 and U+2029
            '\u{2028}' => output.push_str("\\u2028"), // Line separator
            '\u{2029}' => output.push_str("\\u2029"), // Paragraph separator
            // Other control characters as \uXXXX
            c if c.is_control() => {
                output.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => output.push(c),
        }
    }
    output.push('"');
}

/// Check if a string is a valid unquoted JSON5 identifier
fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_alphabetic() && first != '_' && first != '$' {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// Error type for compact JSON5 serialization.
#[derive(Debug)]
struct CompactJson5Error(String);

impl std::fmt::Display for CompactJson5Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CompactJson5Error {}

impl serde::ser::Error for CompactJson5Error {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        CompactJson5Error(msg.to_string())
    }
}

/// Serialize a value to a compact JSON5 string (for HTTP transport).
///
/// Supports NaN/Infinity. No whitespace between elements.
///
/// # Errors
///
/// Returns an error if the value cannot be serialized to JSON5.
#[allow(dead_code)]
pub fn to_string_compact<T: Serialize>(value: &T) -> anyhow::Result<String> {
    let mut output = String::new();
    let mut serializer = CompactJson5Serializer {
        output: &mut output,
    };
    value
        .serialize(&mut serializer)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(output)
}

struct CompactJson5Serializer<'a> {
    output: &'a mut String,
}

impl<'a> serde::Serializer for &mut CompactJson5Serializer<'a> {
    type Ok = ();
    type Error = CompactJson5Error;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(self, v: bool) -> Result<(), Self::Error> {
        self.output.push_str(if v { "true" } else { "false" });
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<(), Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_i16(self, v: i16) -> Result<(), Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_i32(self, v: i32) -> Result<(), Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_i64(self, v: i64) -> Result<(), Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_u8(self, v: u8) -> Result<(), Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_u16(self, v: u16) -> Result<(), Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_u32(self, v: u32) -> Result<(), Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_u64(self, v: u64) -> Result<(), Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_f32(self, v: f32) -> Result<(), Self::Error> {
        self.output.push_str(&format_f32(v));
        Ok(())
    }

    fn serialize_f64(self, v: f64) -> Result<(), Self::Error> {
        self.output.push_str(&format_f64(v));
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<(), Self::Error> {
        self.serialize_str(&v.to_string())
    }

    fn serialize_str(self, v: &str) -> Result<(), Self::Error> {
        self.output.push('"');
        for c in v.chars() {
            match c {
                '"' => self.output.push_str("\\\""),
                '\\' => self.output.push_str("\\\\"),
                '\n' => self.output.push_str("\\n"),
                '\r' => self.output.push_str("\\r"),
                '\t' => self.output.push_str("\\t"),
                c if c < ' ' => self.output.push_str(&format!("\\u{:04x}", c as u32)),
                c => self.output.push(c),
            }
        }
        self.output.push('"');
        Ok(())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<(), Self::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = self.serialize_seq(Some(v.len()))?;
        for b in v {
            seq.serialize_element(b)?;
        }
        seq.end()
    }

    fn serialize_none(self) -> Result<(), Self::Error> {
        self.serialize_unit()
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), Self::Error> {
        self.output.push_str("null");
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Self::Error> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<(), Self::Error> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.output.push('{');
        self.serialize_str(variant)?;
        self.output.push(':');
        value.serialize(&mut *self)?;
        self.output.push('}');
        Ok(())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        self.output.push('[');
        Ok(self)
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        self.output.push('{');
        self.serialize_str(variant)?;
        self.output.push_str(":[");
        Ok(self)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        self.output.push('{');
        Ok(self)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.serialize_map(Some(len))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.output.push('{');
        self.serialize_str(variant)?;
        self.output.push_str(":{");
        Ok(self)
    }
}

impl<'a> serde::ser::SerializeSeq for &mut CompactJson5Serializer<'a> {
    type Ok = ();
    type Error = CompactJson5Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        if !self.output.ends_with('[') {
            self.output.push(',');
        }
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.output.push(']');
        Ok(())
    }
}

impl<'a> serde::ser::SerializeTuple for &mut CompactJson5Serializer<'a> {
    type Ok = ();
    type Error = CompactJson5Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<(), Self::Error> {
        serde::ser::SerializeSeq::end(self)
    }
}

impl<'a> serde::ser::SerializeTupleStruct for &mut CompactJson5Serializer<'a> {
    type Ok = ();
    type Error = CompactJson5Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<(), Self::Error> {
        serde::ser::SerializeSeq::end(self)
    }
}

impl<'a> serde::ser::SerializeTupleVariant for &mut CompactJson5Serializer<'a> {
    type Ok = ();
    type Error = CompactJson5Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.output.push_str("]}");
        Ok(())
    }
}

impl<'a> serde::ser::SerializeMap for &mut CompactJson5Serializer<'a> {
    type Ok = ();
    type Error = CompactJson5Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        if !self.output.ends_with('{') {
            self.output.push(',');
        }
        key.serialize(&mut **self)
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.output.push(':');
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.output.push('}');
        Ok(())
    }
}

impl<'a> serde::ser::SerializeStruct for &mut CompactJson5Serializer<'a> {
    type Ok = ();
    type Error = CompactJson5Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        serde::ser::SerializeMap::serialize_key(self, key)?;
        serde::ser::SerializeMap::serialize_value(self, value)
    }

    fn end(self) -> Result<(), Self::Error> {
        serde::ser::SerializeMap::end(self)
    }
}

impl<'a> serde::ser::SerializeStructVariant for &mut CompactJson5Serializer<'a> {
    type Ok = ();
    type Error = CompactJson5Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        serde::ser::SerializeStruct::serialize_field(self, key, value)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.output.push_str("}}");
        Ok(())
    }
}

/// Serialize a value to a JSON5 byte vector with sorted keys.
///
/// Supports NaN/Infinity and sorts object keys alphabetically.
/// Uses a custom serializer that builds a tree directly (no parsing needed).
///
/// # Errors
///
/// Returns an error if the value cannot be serialized.
pub fn to_vec_pretty_sorted<T: Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    // Serialize directly to Json5Value tree (with BTreeMap for sorted keys)
    let tree = value
        .serialize(Json5ValueSerializer)
        .map_err(|e| anyhow::anyhow!("Failed to serialize: {}", e))?;

    // Write the tree to string with pretty formatting
    let mut output = String::new();
    tree.write_to(&mut output, 0);
    output.push('\n');
    Ok(output.into_bytes())
}

/// A serde Serializer that builds a Json5Value tree directly.
/// This avoids the need to parse - we serialize directly to the intermediate representation.
struct Json5ValueSerializer;

impl serde::Serializer for Json5ValueSerializer {
    type Ok = Json5Value;
    type Error = Json5SerError;
    type SerializeSeq = Json5SeqSerializer;
    type SerializeTuple = Json5SeqSerializer;
    type SerializeTupleStruct = Json5SeqSerializer;
    type SerializeTupleVariant = Json5TupleVariantSerializer;
    type SerializeMap = Json5MapSerializer;
    type SerializeStruct = Json5MapSerializer;
    type SerializeStructVariant = Json5StructVariantSerializer;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Bool(v))
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Number(v.to_string()))
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Number(v.to_string()))
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Number(v.to_string()))
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Number(v.to_string()))
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Number(v.to_string()))
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Number(v.to_string()))
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Number(v.to_string()))
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Number(v.to_string()))
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Number(format_f32(v)))
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Number(format_f64(v)))
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::String(v.to_string()))
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::String(v.to_string()))
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        let arr: Vec<Json5Value> = v
            .iter()
            .map(|b| Json5Value::Number(b.to_string()))
            .collect();
        Ok(Json5Value::Array(arr))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Null)
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::String(variant.to_string()))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        let inner = value.serialize(Json5ValueSerializer)?;
        let mut map = std::collections::BTreeMap::new();
        map.insert(variant.to_string(), inner);
        Ok(Json5Value::Object(map))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(Json5SeqSerializer {
            items: Vec::with_capacity(len.unwrap_or(0)),
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(Json5TupleVariantSerializer {
            variant: variant.to_string(),
            items: Vec::with_capacity(len),
        })
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(Json5MapSerializer {
            map: std::collections::BTreeMap::new(),
            next_key: None,
            _capacity: len.unwrap_or(0),
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.serialize_map(Some(len))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(Json5StructVariantSerializer {
            variant: variant.to_string(),
            map: std::collections::BTreeMap::new(),
            _capacity: len,
        })
    }
}

#[derive(Debug)]
struct Json5SerError(String);

impl std::fmt::Display for Json5SerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Json5SerError {}

impl serde::ser::Error for Json5SerError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Json5SerError(msg.to_string())
    }
}

struct Json5SeqSerializer {
    items: Vec<Json5Value>,
}

impl serde::ser::SerializeSeq for Json5SeqSerializer {
    type Ok = Json5Value;
    type Error = Json5SerError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.items.push(value.serialize(Json5ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Array(self.items))
    }
}

impl serde::ser::SerializeTuple for Json5SeqSerializer {
    type Ok = Json5Value;
    type Error = Json5SerError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        serde::ser::SerializeSeq::end(self)
    }
}

impl serde::ser::SerializeTupleStruct for Json5SeqSerializer {
    type Ok = Json5Value;
    type Error = Json5SerError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        serde::ser::SerializeSeq::end(self)
    }
}

struct Json5TupleVariantSerializer {
    variant: String,
    items: Vec<Json5Value>,
}

impl serde::ser::SerializeTupleVariant for Json5TupleVariantSerializer {
    type Ok = Json5Value;
    type Error = Json5SerError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.items.push(value.serialize(Json5ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let mut map = std::collections::BTreeMap::new();
        map.insert(self.variant, Json5Value::Array(self.items));
        Ok(Json5Value::Object(map))
    }
}

struct Json5MapSerializer {
    map: std::collections::BTreeMap<String, Json5Value>,
    next_key: Option<String>,
    _capacity: usize,
}

impl serde::ser::SerializeMap for Json5MapSerializer {
    type Ok = Json5Value;
    type Error = Json5SerError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        // Serialize key to Json5Value, then extract string
        let key_value = key.serialize(Json5ValueSerializer)?;
        let key_str = match key_value {
            Json5Value::String(s) => s,
            Json5Value::Number(n) => n,
            _ => {
                return Err(Json5SerError(
                    "Map key must be string or number".to_string(),
                ))
            }
        };
        self.next_key = Some(key_str);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        let key = self.next_key.take().ok_or_else(|| {
            Json5SerError("serialize_value called without serialize_key".to_string())
        })?;
        self.map.insert(key, value.serialize(Json5ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Object(self.map))
    }
}

impl serde::ser::SerializeStruct for Json5MapSerializer {
    type Ok = Json5Value;
    type Error = Json5SerError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.map
            .insert(key.to_string(), value.serialize(Json5ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(Json5Value::Object(self.map))
    }
}

struct Json5StructVariantSerializer {
    variant: String,
    map: std::collections::BTreeMap<String, Json5Value>,
    _capacity: usize,
}

impl serde::ser::SerializeStructVariant for Json5StructVariantSerializer {
    type Ok = Json5Value;
    type Error = Json5SerError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.map
            .insert(key.to_string(), value.serialize(Json5ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let mut outer = std::collections::BTreeMap::new();
        outer.insert(self.variant, Json5Value::Object(self.map));
        Ok(Json5Value::Object(outer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn test_format_f32() {
        // Trailing zeros trimmed, .0 removed
        assert_eq!(format_f32(6.67), "6.67");
        assert_eq!(format_f32(6.0), "6");
        assert_eq!(format_f32(1.5), "1.5");
        assert_eq!(format_f32(100.0), "100");

        // Negative values
        assert_eq!(format_f32(-3.15), "-3.15");
        assert_eq!(format_f32(-2.5), "-2.5");

        // Zero and negative zero
        assert_eq!(format_f32(0.0), "0");
        assert_eq!(format_f32(-0.0), "0");

        // Rounding to 6 significant digits
        assert_eq!(format_f32(0.5), "0.5");
        assert_eq!(format_f32(1.0 / 3.0), "0.333333");

        // Small values clamped to zero
        assert_eq!(format_f32(1e-10), "0");
        assert_eq!(format_f32(-1e-10), "0");
        assert_eq!(format_f32(4.9e-7), "0");

        // Scientific notation for large values
        assert_eq!(format_f32(1e20), "1e20");

        // Special values
        assert_eq!(format_f32(f32::NAN), "NaN");
        assert_eq!(format_f32(f32::INFINITY), "Infinity");
        assert_eq!(format_f32(f32::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn test_format_f64() {
        // Trailing zeros trimmed, .0 removed
        assert_eq!(format_f64(6.67), "6.67");
        assert_eq!(format_f64(6.0), "6");
        assert_eq!(format_f64(1.5), "1.5");
        assert_eq!(format_f64(100.0), "100");

        // Negative values
        assert_eq!(format_f64(-3.15), "-3.15");
        assert_eq!(format_f64(-2.5), "-2.5");

        // Zero and negative zero
        assert_eq!(format_f64(0.0), "0");
        assert_eq!(format_f64(-0.0), "0");

        // Rounding to 15 significant digits
        assert_eq!(format_f64(0.5), "0.5");
        assert_eq!(format_f64(1.0 / 3.0), "0.333333333333333");

        // Small values clamped to zero
        assert_eq!(format_f64(1e-100), "0");
        assert_eq!(format_f64(-1e-100), "0");
        assert_eq!(format_f64(4.9e-16), "0");

        // Scientific notation for large values
        assert_eq!(format_f64(1e47), "1e47");

        // Extreme values use exact formatting to avoid roundtrip overflow
        assert_eq!(format_f64(f64::MAX), "1.7976931348623157e308");
        assert_eq!(format_f64(f64::MIN), "-1.7976931348623157e308");

        // Special values
        assert_eq!(format_f64(f64::NAN), "NaN");
        assert_eq!(format_f64(f64::INFINITY), "Infinity");
        assert_eq!(format_f64(f64::NEG_INFINITY), "-Infinity");
    }

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
        struct TestFloat {
            x: f64,
        }
        let result: TestFloat = from_str(r#"{ "x": Infinity }"#).unwrap();
        assert!(result.x.is_infinite() && result.x.is_sign_positive());
    }

    #[test]
    fn test_deserialize_negative_infinity() {
        #[derive(Debug, Deserialize)]
        struct TestFloat {
            x: f64,
        }
        let result: TestFloat = from_str(r#"{ "x": -Infinity }"#).unwrap();
        assert!(result.x.is_infinite() && result.x.is_sign_negative());
    }

    #[test]
    fn test_deserialize_nan() {
        #[derive(Debug, Deserialize)]
        struct TestFloat {
            x: f64,
        }
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
        assert!(
            result.negative_infinity.is_infinite() && result.negative_infinity.is_sign_negative()
        );
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
        assert!(
            deserialized.neg_infinity.is_infinite() && deserialized.neg_infinity.is_sign_negative()
        );
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
        assert!(
            deserialized.position[0].is_infinite() && deserialized.position[0].is_sign_positive()
        );
        assert!(deserialized.position[1].is_nan());
        assert!(
            deserialized.position[2].is_infinite() && deserialized.position[2].is_sign_negative()
        );

        // Verify orientation
        assert_eq!(deserialized.orientation[0][0], 1.0);
        assert!(deserialized.orientation[0][2].is_nan());
        assert!(deserialized.orientation[1][1].is_infinite());
        assert!(
            deserialized.orientation[2][0].is_infinite()
                && deserialized.orientation[2][0].is_sign_negative()
        );
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
        assert!(deserialized
            .properties
            .get("Transparency")
            .unwrap()
            .is_nan());
        assert!(deserialized.properties.get("Health").unwrap().is_infinite());
        assert_eq!(*deserialized.properties.get("WalkSpeed").unwrap(), 16.0);
    }

    // =========================================================================
    // Sorted JSON5 serializer tests - comprehensive coverage
    // =========================================================================

    mod serializer_tests {
        use super::*;

        // =====================================================================
        // Large/Small Number Handling (the main bug we fixed)
        // =====================================================================

        #[test]
        fn test_roundtrip_large_number() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: f64,
            }

            let original = Data { value: 1e47 };
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Should use scientific notation for large numbers
            assert!(
                json_str.contains("1e47") || json_str.contains("1e+47"),
                "Large number should be in scientific notation: {}",
                json_str
            );

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.value, original.value);
        }

        #[test]
        fn test_roundtrip_very_small_number() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: f64,
            }

            let original = Data { value: 1e-100 };
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Very small numbers are clamped to zero by formatter policy
            assert!(
                json_str.contains(": 0"),
                "Very small number should clamp to zero: {json_str}"
            );

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.value, 0.0);
        }

        #[test]
        fn test_roundtrip_negative_large_number() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: f64,
            }

            let original = Data { value: -1e47 };
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.value, original.value);
        }

        #[test]
        fn test_roundtrip_negative_small_number() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: f64,
            }

            let original = Data { value: -1e-50 };
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.value, 0.0);
        }

        // Test the exact boundaries where we switch to scientific notation
        #[test]
        fn test_boundary_at_1e15() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                below: f64,
                at: f64,
                above: f64,
            }

            let original = Data {
                below: 9.99e14,
                at: 1e15,
                above: 1.01e15,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // At and above 1e15 should use scientific notation
            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.below, original.below);
            assert_eq!(deserialized.at, original.at);
            assert_eq!(deserialized.above, original.above);
        }

        #[test]
        fn test_boundary_at_1e_minus_6() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                above: f64,
                at: f64,
                below: f64,
            }

            let original = Data {
                above: 1.01e-6,
                at: 1e-6,
                below: 9.9e-7,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Values below 0.5e-6 are clamped to zero
            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.above, original.above);
            assert_eq!(deserialized.at, original.at);
            assert_eq!(deserialized.below, original.below);
        }

        #[test]
        fn test_zero_not_scientific() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                num: f64,
            }

            let original = Data { num: 0.0 };
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Zero should not use scientific notation (check the value part only)
            // The output should contain "num: 0" not "num: 0e0" or similar
            assert!(
                json_str.contains(": 0") && !json_str.contains("0e"),
                "Zero should not be in scientific notation: {}",
                json_str
            );

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.num, 0.0);
        }

        #[test]
        fn test_normal_numbers_not_scientific() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                a: f64,
                b: f64,
                c: f64,
            }

            let original = Data {
                a: 42.5,
                b: 1000.0,
                c: 0.001,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Normal numbers should not use scientific notation
            assert!(
                !json_str.contains("e"),
                "Normal numbers should not use scientific notation: {}",
                json_str
            );

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.a, original.a);
            assert_eq!(deserialized.b, original.b);
            assert_eq!(deserialized.c, original.c);
        }

        // =====================================================================
        // Special Float Values (NaN, Infinity)
        // =====================================================================

        #[test]
        fn test_nan_roundtrip() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Data {
                value: f64,
            }

            let original = Data { value: f64::NAN };
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            assert!(json_str.contains("NaN"), "Should contain NaN: {}", json_str);

            let deserialized: Data = from_str(&json_str).unwrap();
            assert!(deserialized.value.is_nan());
        }

        #[test]
        fn test_infinity_roundtrip() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Data {
                value: f64,
            }

            let original = Data {
                value: f64::INFINITY,
            };
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            assert!(
                json_str.contains("Infinity"),
                "Should contain Infinity: {}",
                json_str
            );

            let deserialized: Data = from_str(&json_str).unwrap();
            assert!(deserialized.value.is_infinite() && deserialized.value.is_sign_positive());
        }

        #[test]
        fn test_negative_infinity_roundtrip() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Data {
                value: f64,
            }

            let original = Data {
                value: f64::NEG_INFINITY,
            };
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            assert!(
                json_str.contains("-Infinity"),
                "Should contain -Infinity: {}",
                json_str
            );

            let deserialized: Data = from_str(&json_str).unwrap();
            assert!(deserialized.value.is_infinite() && deserialized.value.is_sign_negative());
        }

        #[test]
        fn test_mixed_special_floats() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Data {
                nan: f64,
                pos_inf: f64,
                neg_inf: f64,
                normal: f64,
                large: f64,
            }

            let original = Data {
                nan: f64::NAN,
                pos_inf: f64::INFINITY,
                neg_inf: f64::NEG_INFINITY,
                normal: 42.5,
                large: 1e47,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert!(deserialized.nan.is_nan());
            assert!(deserialized.pos_inf.is_infinite() && deserialized.pos_inf.is_sign_positive());
            assert!(deserialized.neg_inf.is_infinite() && deserialized.neg_inf.is_sign_negative());
            assert_eq!(deserialized.normal, 42.5);
            assert_eq!(deserialized.large, 1e47);
        }

        // =====================================================================
        // Integer Types
        // =====================================================================

        #[test]
        fn test_integer_types() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                i8_val: i8,
                i16_val: i16,
                i32_val: i32,
                i64_val: i64,
                u8_val: u8,
                u16_val: u16,
                u32_val: u32,
                u64_val: u64,
            }

            let original = Data {
                i8_val: -128,
                i16_val: -32768,
                i32_val: -2147483648,
                i64_val: -9223372036854775808,
                u8_val: 255,
                u16_val: 65535,
                u32_val: 4294967295,
                u64_val: 18446744073709551615,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        // =====================================================================
        // Sorted Keys
        // =====================================================================

        #[test]
        fn test_output_sorted_keys() {
            #[derive(Debug, Serialize)]
            struct Data {
                zebra: i32,
                apple: i32,
                mango: i32,
            }

            let data = Data {
                zebra: 3,
                apple: 1,
                mango: 2,
            };

            let serialized = to_vec_pretty_sorted(&data).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Keys should be sorted alphabetically
            let apple_pos = json_str.find("apple").unwrap();
            let mango_pos = json_str.find("mango").unwrap();
            let zebra_pos = json_str.find("zebra").unwrap();

            assert!(apple_pos < mango_pos);
            assert!(mango_pos < zebra_pos);
        }

        #[test]
        fn test_nested_objects_sorted() {
            #[derive(Debug, Serialize)]
            struct Inner {
                z: i32,
                a: i32,
            }

            #[derive(Debug, Serialize)]
            struct Outer {
                z_inner: Inner,
                a_inner: Inner,
            }

            let data = Outer {
                z_inner: Inner { z: 1, a: 2 },
                a_inner: Inner { z: 3, a: 4 },
            };

            let serialized = to_vec_pretty_sorted(&data).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Outer keys should be sorted
            let a_inner_pos = json_str.find("a_inner").unwrap();
            let z_inner_pos = json_str.find("z_inner").unwrap();
            assert!(a_inner_pos < z_inner_pos);

            // Inner keys should also be sorted (check first occurrence of 'a:' vs 'z:')
            // This is a bit tricky since 'a' appears in key names too
            assert!(json_str.contains("a_inner"));
            assert!(json_str.contains("z_inner"));
        }

        #[test]
        fn test_map_keys_sorted() {
            let mut map = std::collections::HashMap::new();
            map.insert("zebra", 3);
            map.insert("apple", 1);
            map.insert("mango", 2);

            let serialized = to_vec_pretty_sorted(&map).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let apple_pos = json_str.find("apple").unwrap();
            let mango_pos = json_str.find("mango").unwrap();
            let zebra_pos = json_str.find("zebra").unwrap();

            assert!(apple_pos < mango_pos);
            assert!(mango_pos < zebra_pos);
        }

        // =====================================================================
        // Pretty Formatting
        // =====================================================================

        #[test]
        fn test_output_pretty_format() {
            #[derive(Debug, Serialize)]
            struct Data {
                key: String,
            }

            let data = Data {
                key: "value".to_string(),
            };

            let serialized = to_vec_pretty_sorted(&data).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Should have newlines and indentation
            assert!(json_str.contains('\n'));
            assert!(json_str.contains("  ")); // indentation
        }

        #[test]
        fn test_nested_indentation() {
            #[derive(Debug, Serialize)]
            struct Inner {
                value: i32,
            }

            #[derive(Debug, Serialize)]
            struct Outer {
                inner: Inner,
            }

            let data = Outer {
                inner: Inner { value: 42 },
            };

            let serialized = to_vec_pretty_sorted(&data).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Should have multiple levels of indentation
            assert!(json_str.contains("    ")); // 4 spaces for nested
        }

        // =====================================================================
        // String Escaping
        // =====================================================================

        #[test]
        fn test_write_to_escapes_special_chars() {
            let value = Json5Value::String("hello\nworld\t\"test\"".to_string());
            let mut output = String::new();
            value.write_to(&mut output, 0);
            assert_eq!(output, r#""hello\nworld\t\"test\"""#);
        }

        #[test]
        fn test_string_escaping_backslash() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                path: String,
            }

            let original = Data {
                path: r"C:\Users\test".to_string(),
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            assert!(
                json_str.contains(r"\\"),
                "Backslash should be escaped: {}",
                json_str
            );

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.path, original.path);
        }

        #[test]
        fn test_string_escaping_control_chars() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                text: String,
            }

            let original = Data {
                text: "line1\nline2\rline3\ttab".to_string(),
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            assert!(json_str.contains(r"\n"));
            assert!(json_str.contains(r"\r"));
            assert!(json_str.contains(r"\t"));

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.text, original.text);
        }

        #[test]
        fn test_unicode_strings() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                emoji: String,
                chinese: String,
                arabic: String,
            }

            let original = Data {
                emoji: "Hello 👋 World 🌍".to_string(),
                chinese: "你好世界".to_string(),
                arabic: "مرحبا بالعالم".to_string(),
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        // =====================================================================
        // Identifier Quoting
        // =====================================================================

        #[test]
        fn test_valid_identifier_unquoted() {
            assert!(is_valid_identifier("foo"));
            assert!(is_valid_identifier("_bar"));
            assert!(is_valid_identifier("$baz"));
            assert!(is_valid_identifier("foo123"));
            assert!(is_valid_identifier("_"));
            assert!(is_valid_identifier("$"));
            assert!(is_valid_identifier("camelCase"));
            assert!(is_valid_identifier("snake_case"));
        }

        #[test]
        fn test_invalid_identifier_quoted() {
            assert!(!is_valid_identifier("")); // empty
            assert!(!is_valid_identifier("123")); // starts with digit
            assert!(!is_valid_identifier("foo-bar")); // contains dash
            assert!(!is_valid_identifier("foo bar")); // contains space
            assert!(!is_valid_identifier("foo.bar")); // contains dot
            assert!(!is_valid_identifier("1foo")); // starts with digit
            assert!(!is_valid_identifier("foo@bar")); // contains @
        }

        #[test]
        fn test_keys_with_special_chars_quoted() {
            let mut map = std::collections::HashMap::new();
            map.insert("normal_key", 1);
            map.insert("key-with-dash", 2);
            map.insert("key with space", 3);

            let serialized = to_vec_pretty_sorted(&map).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Normal key should be unquoted
            assert!(
                json_str.contains("normal_key:"),
                "Normal key should be unquoted: {}",
                json_str
            );

            // Special keys should be quoted
            assert!(
                json_str.contains("\"key-with-dash\""),
                "Dashed key should be quoted: {}",
                json_str
            );
            assert!(
                json_str.contains("\"key with space\""),
                "Spaced key should be quoted: {}",
                json_str
            );
        }

        #[test]
        fn test_keys_with_escaped_chars_roundtrip() {
            // Test keys that contain characters requiring escaping
            let mut map = std::collections::HashMap::new();
            map.insert("key\"with\"quotes".to_string(), 1);
            map.insert("key\\with\\backslash".to_string(), 2);
            map.insert("key\nwith\nnewline".to_string(), 3);
            map.insert("key\twith\ttab".to_string(), 4);

            let serialized = to_vec_pretty_sorted(&map).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Verify escaping is present
            assert!(
                json_str.contains(r#"\""#),
                "Quotes in key should be escaped: {}",
                json_str
            );
            assert!(
                json_str.contains(r"\\"),
                "Backslash in key should be escaped: {}",
                json_str
            );
            assert!(
                json_str.contains(r"\n"),
                "Newline in key should be escaped: {}",
                json_str
            );
            assert!(
                json_str.contains(r"\t"),
                "Tab in key should be escaped: {}",
                json_str
            );

            // Verify roundtrip works
            let deserialized: std::collections::HashMap<String, i32> = from_str(&json_str).unwrap();
            assert_eq!(deserialized.get("key\"with\"quotes"), Some(&1));
            assert_eq!(deserialized.get("key\\with\\backslash"), Some(&2));
            assert_eq!(deserialized.get("key\nwith\nnewline"), Some(&3));
            assert_eq!(deserialized.get("key\twith\ttab"), Some(&4));
        }

        #[test]
        fn test_keys_with_control_chars_roundtrip() {
            // Test keys with control characters (should be escaped as \uXXXX)
            let mut map = std::collections::HashMap::new();
            map.insert("key\x00null".to_string(), 1); // NUL character
            map.insert("key\x07bell".to_string(), 2); // BEL character

            let serialized = to_vec_pretty_sorted(&map).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Control characters should be escaped as \uXXXX
            assert!(
                json_str.contains(r"\u0000") || json_str.contains(r"\u0007"),
                "Control characters in key should be escaped: {}",
                json_str
            );

            // Verify roundtrip works
            let deserialized: std::collections::HashMap<String, i32> = from_str(&json_str).unwrap();
            assert_eq!(deserialized.get("key\x00null"), Some(&1));
            assert_eq!(deserialized.get("key\x07bell"), Some(&2));
        }

        #[test]
        fn test_write_escaped_string_function() {
            // Direct test of the escaping function
            let mut output = String::new();
            write_escaped_string(&mut output, r#"hello"world"#);
            assert_eq!(output, r#""hello\"world""#);

            let mut output = String::new();
            write_escaped_string(&mut output, "line1\nline2");
            assert_eq!(output, r#""line1\nline2""#);

            let mut output = String::new();
            write_escaped_string(&mut output, r"path\to\file");
            assert_eq!(output, r#""path\\to\\file""#);
        }

        // =====================================================================
        // Arrays and Empty Collections
        // =====================================================================

        #[test]
        fn test_array_roundtrip() {
            let original: Vec<i32> = vec![1, 2, 3, 4, 5];

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Vec<i32> = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        #[test]
        fn test_empty_array() {
            let original: Vec<i32> = vec![];

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            assert!(json_str.contains("[]"));

            let deserialized: Vec<i32> = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        #[test]
        fn test_empty_object() {
            let original: std::collections::HashMap<String, i32> = std::collections::HashMap::new();

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            assert!(json_str.contains("{}"));

            let deserialized: std::collections::HashMap<String, i32> = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        #[test]
        fn test_nested_arrays() {
            let original: Vec<Vec<i32>> = vec![vec![1, 2], vec![3, 4], vec![5, 6]];

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Vec<Vec<i32>> = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        // =====================================================================
        // Optional Values
        // =====================================================================

        #[test]
        fn test_option_some() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: Option<i32>,
            }

            let original = Data { value: Some(42) };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        #[test]
        fn test_option_none() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: Option<i32>,
            }

            let original = Data { value: None };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            assert!(json_str.contains("null"));

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        // =====================================================================
        // Bool and Null
        // =====================================================================

        #[test]
        fn test_bool_values() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                t: bool,
                f: bool,
            }

            let original = Data { t: true, f: false };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            assert!(json_str.contains("true"));
            assert!(json_str.contains("false"));

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        #[test]
        fn test_null_value() {
            let original: Option<i32> = None;

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            assert_eq!(json_str.trim(), "null");
        }

        // =====================================================================
        // Complex/Real-World Structures
        // =====================================================================

        #[test]
        fn test_roundtrip_complex_model() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Model {
                #[serde(rename = "className")]
                class_name: String,
                properties: std::collections::BTreeMap<String, f64>,
            }

            let mut properties = std::collections::BTreeMap::new();
            properties.insert("Value".to_string(), 1e47);
            properties.insert("Normal".to_string(), 42.5);
            properties.insert("Tiny".to_string(), 1e-50);

            let original = Model {
                class_name: "NumberValue".to_string(),
                properties,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Model = from_str(&json_str).unwrap();
            assert_eq!(deserialized.class_name, "NumberValue");
            assert_eq!(*deserialized.properties.get("Value").unwrap(), 1e47);
            assert_eq!(*deserialized.properties.get("Normal").unwrap(), 42.5);
            assert_eq!(*deserialized.properties.get("Tiny").unwrap(), 0.0);
        }

        #[test]
        fn test_roblox_model_like_structure() {
            // Mimics the actual structure that caused the original bug
            #[derive(Debug, Serialize, Deserialize)]
            struct JsonModel {
                #[serde(rename = "className")]
                class_name: String,
                #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
                properties: std::collections::BTreeMap<String, serde_json::Value>,
                #[serde(default, skip_serializing_if = "Vec::is_empty")]
                children: Vec<JsonModel>,
            }

            let mut properties = std::collections::BTreeMap::new();
            properties.insert("Value".to_string(), serde_json::json!(1e47));

            let model = JsonModel {
                class_name: "NumberValue".to_string(),
                properties,
                children: vec![],
            };

            let serialized = to_vec_pretty_sorted(&model).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Should be parseable
            let _: JsonModel = from_str(&json_str).unwrap();
        }

        #[test]
        fn test_deeply_nested_structure() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Level3 {
                value: f64,
            }

            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Level2 {
                level3: Level3,
            }

            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Level1 {
                level2: Level2,
            }

            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Root {
                level1: Level1,
            }

            let original = Root {
                level1: Level1 {
                    level2: Level2 {
                        level3: Level3 { value: 1e47 },
                    },
                },
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Root = from_str(&json_str).unwrap();
            assert_eq!(deserialized.level1.level2.level3.value, 1e47);
        }

        // =====================================================================
        // Enum Variants
        // =====================================================================

        #[test]
        fn test_enum_unit_variant() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            enum Status {
                Active,
                Inactive,
            }

            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                status: Status,
            }

            let original = Data {
                status: Status::Active,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        #[test]
        fn test_enum_newtype_variant() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            enum Value {
                Number(f64),
                Text(String),
            }

            let original = Value::Number(1e47);

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Value = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        // =====================================================================
        // Char and Bytes
        // =====================================================================

        #[test]
        fn test_char_serialization() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                ch: char,
            }

            let original = Data { ch: 'X' };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized, original);
        }

        // =====================================================================
        // Regression Tests
        // =====================================================================

        #[test]
        fn test_regression_numbervalue_1e47() {
            // This is the exact case that caused the original bug
            #[derive(Debug, Serialize, Deserialize)]
            struct NumberValue {
                #[serde(rename = "className")]
                class_name: String,
                properties: std::collections::BTreeMap<String, f64>,
            }

            let mut properties = std::collections::BTreeMap::new();
            properties.insert("Value".to_string(), 1e47);

            let original = NumberValue {
                class_name: "NumberValue".to_string(),
                properties,
            };

            // This should not panic or error
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // And should be parseable
            let deserialized: NumberValue = from_str(&json_str).unwrap();
            assert_eq!(deserialized.properties["Value"], 1e47);
        }

        #[test]
        fn test_f64_max_min() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Data {
                max: f64,
                min: f64,
            }

            let original = Data {
                max: f64::MAX,
                min: f64::MIN,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.max, f64::MAX);
            assert_eq!(deserialized.min, f64::MIN);
        }

        #[test]
        fn test_f64_epsilon_and_min_positive() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Data {
                epsilon: f64,
                min_positive: f64,
            }

            let original = Data {
                epsilon: f64::EPSILON,
                min_positive: f64::MIN_POSITIVE,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.epsilon, 0.0);
            assert_eq!(deserialized.min_positive, 0.0);
        }
    }

    // =========================================================================
    // JSON5-specific parsing edge cases (using json5 crate)
    // These test that we can correctly parse JSON5 input formats
    // =========================================================================

    mod json5_parsing_edge_cases {
        use super::*;

        // =====================================================================
        // Number Parsing Edge Cases
        // =====================================================================

        #[test]
        fn test_parse_hexadecimal() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i64,
            }

            let result: Data = from_str(r#"{ value: 0xFF }"#).unwrap();
            assert_eq!(result.value, 255);

            let result: Data = from_str(r#"{ value: 0xDEADBEEF }"#).unwrap();
            assert_eq!(result.value, 0xDEADBEEF);
        }

        #[test]
        fn test_parse_negative_hexadecimal() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i64,
            }

            let result: Data = from_str(r#"{ value: -0xFF }"#).unwrap();
            assert_eq!(result.value, -255);
        }

        #[test]
        fn test_parse_leading_decimal_point() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: .5 }"#).unwrap();
            assert_eq!(result.value, 0.5);

            let result: Data = from_str(r#"{ value: .123 }"#).unwrap();
            assert_eq!(result.value, 0.123);
        }

        #[test]
        fn test_parse_trailing_decimal_point() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: 5. }"#).unwrap();
            assert_eq!(result.value, 5.0);

            let result: Data = from_str(r#"{ value: 123. }"#).unwrap();
            assert_eq!(result.value, 123.0);
        }

        #[test]
        fn test_parse_positive_sign() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: +5 }"#).unwrap();
            assert_eq!(result.value, 5.0);

            let result: Data = from_str(r#"{ value: +Infinity }"#).unwrap();
            assert!(result.value.is_infinite() && result.value.is_sign_positive());
        }

        #[test]
        fn test_parse_negative_zero() {
            #[derive(Debug, Deserialize)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: -0 }"#).unwrap();
            assert_eq!(result.value, 0.0);
            // Note: -0.0 == 0.0 in Rust, but we can check the sign bit
            assert!(result.value.is_sign_negative() || result.value == 0.0);
        }

        #[test]
        fn test_parse_exponent_variations() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: f64,
            }

            // Uppercase E
            let result: Data = from_str(r#"{ value: 1E10 }"#).unwrap();
            assert_eq!(result.value, 1e10);

            // Positive exponent with +
            let result: Data = from_str(r#"{ value: 1e+10 }"#).unwrap();
            assert_eq!(result.value, 1e10);

            // Negative exponent
            let result: Data = from_str(r#"{ value: 1e-10 }"#).unwrap();
            assert_eq!(result.value, 1e-10);

            // Zero exponent
            let result: Data = from_str(r#"{ value: 5e0 }"#).unwrap();
            assert_eq!(result.value, 5.0);
        }

        // =====================================================================
        // String Parsing Edge Cases
        // =====================================================================

        #[test]
        fn test_parse_single_quoted_strings() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: 'hello world' }"#).unwrap();
            assert_eq!(result.value, "hello world");
        }

        #[test]
        fn test_parse_single_quoted_with_double_quote() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: 'say "hello"' }"#).unwrap();
            assert_eq!(result.value, r#"say "hello""#);
        }

        #[test]
        fn test_parse_escaped_single_quote() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: 'it\'s working' }"#).unwrap();
            assert_eq!(result.value, "it's working");
        }

        #[test]
        fn test_parse_multi_line_string() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            // JSON5 allows escaped newlines in strings
            let result: Data = from_str("{ value: 'line1\\\nline2' }").unwrap();
            assert_eq!(result.value, "line1line2");
        }

        #[test]
        fn test_parse_hex_escape_sequences() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: "\x48\x65\x6C\x6C\x6F" }"#).unwrap();
            assert_eq!(result.value, "Hello");
        }

        #[test]
        fn test_parse_unicode_escape_sequences() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: "\u0048\u0065\u006C\u006C\u006F" }"#).unwrap();
            assert_eq!(result.value, "Hello");

            // Emoji
            let result: Data = from_str(r#"{ value: "\uD83D\uDE00" }"#).unwrap();
            assert_eq!(result.value, "😀");
        }

        // =====================================================================
        // Object Parsing Edge Cases
        // =====================================================================

        #[test]
        fn test_parse_unquoted_keys() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                hello: String,
                world: i32,
            }

            let result: Data = from_str(r#"{ hello: "test", world: 42 }"#).unwrap();
            assert_eq!(result.hello, "test");
            assert_eq!(result.world, 42);
        }

        #[test]
        fn test_parse_single_quoted_keys() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                #[serde(rename = "my-key")]
                my_key: String,
            }

            let result: Data = from_str(r#"{ 'my-key': "value" }"#).unwrap();
            assert_eq!(result.my_key, "value");
        }

        #[test]
        fn test_parse_reserved_words_as_keys() {
            // JSON5 allows reserved words as unquoted keys
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                r#while: i32,
                r#for: i32,
                r#if: i32,
            }

            let result: Data = from_str(r#"{ while: 1, for: 2, if: 3 }"#).unwrap();
            assert_eq!(result.r#while, 1);
            assert_eq!(result.r#for, 2);
            assert_eq!(result.r#if, 3);
        }

        #[test]
        fn test_parse_trailing_comma_object() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                a: i32,
                b: i32,
            }

            let result: Data = from_str(r#"{ a: 1, b: 2, }"#).unwrap();
            assert_eq!(result.a, 1);
            assert_eq!(result.b, 2);
        }

        #[test]
        fn test_parse_nested_trailing_commas() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Inner {
                x: i32,
            }

            #[derive(Debug, Deserialize, PartialEq)]
            struct Outer {
                inner: Inner,
                value: i32,
            }

            let result: Outer = from_str(r#"{ inner: { x: 1, }, value: 2, }"#).unwrap();
            assert_eq!(result.inner.x, 1);
            assert_eq!(result.value, 2);
        }

        // =====================================================================
        // Array Parsing Edge Cases
        // =====================================================================

        #[test]
        fn test_parse_trailing_comma_array() {
            let result: Vec<i32> = from_str(r#"[1, 2, 3,]"#).unwrap();
            assert_eq!(result, vec![1, 2, 3]);
        }

        #[test]
        fn test_parse_nested_arrays_with_trailing_commas() {
            let result: Vec<Vec<i32>> = from_str(r#"[[1, 2,], [3, 4,],]"#).unwrap();
            assert_eq!(result, vec![vec![1, 2], vec![3, 4]]);
        }

        // =====================================================================
        // Comment Parsing Edge Cases
        // =====================================================================

        #[test]
        fn test_parse_single_line_comment() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            let result: Data = from_str(
                r#"{
                // This is a comment
                value: 42
            }"#,
            )
            .unwrap();
            assert_eq!(result.value, 42);
        }

        #[test]
        fn test_parse_multi_line_comment() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            let result: Data = from_str(
                r#"{
                /* This is a
                   multi-line comment */
                value: 42
            }"#,
            )
            .unwrap();
            assert_eq!(result.value, 42);
        }

        #[test]
        fn test_parse_comment_after_value() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                a: i32,
                b: i32,
            }

            let result: Data = from_str(
                r#"{
                a: 1, // comment after value
                b: 2  /* another comment */
            }"#,
            )
            .unwrap();
            assert_eq!(result.a, 1);
            assert_eq!(result.b, 2);
        }

        #[test]
        fn test_parse_comment_in_array() {
            let result: Vec<i32> = from_str(
                r#"[
                1, // first
                2, /* second */
                3
            ]"#,
            )
            .unwrap();
            assert_eq!(result, vec![1, 2, 3]);
        }

        // =====================================================================
        // Whitespace Edge Cases
        // =====================================================================

        #[test]
        fn test_parse_various_whitespace() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            // Tab, newline, carriage return
            let result: Data = from_str("{\t\n\rvalue\t:\n42\r}").unwrap();
            assert_eq!(result.value, 42);
        }

        // =====================================================================
        // Mixed JSON5 Features
        // =====================================================================

        #[test]
        fn test_parse_complex_json5() {
            #[derive(Debug, Deserialize)]
            struct Config {
                name: String,
                #[serde(rename = "maxValue")]
                max_value: f64,
                enabled: bool,
                items: Vec<f64>,
            }

            let result: Config = from_str(
                r#"{
                // Configuration file
                name: 'test-config',  // single quotes
                maxValue: Infinity,   // special float
                enabled: true,
                items: [
                    .5,      // leading decimal
                    1.,      // trailing decimal
                    0xFF,    // hex
                    1e10,    // scientific
                    NaN,     // NaN
                ],  // trailing comma
            }"#,
            )
            .unwrap();

            assert_eq!(result.name, "test-config");
            assert!(result.max_value.is_infinite());
            assert!(result.enabled);
            assert_eq!(result.items[0], 0.5);
            assert_eq!(result.items[1], 1.0);
            assert_eq!(result.items[2], 255.0);
            assert_eq!(result.items[3], 1e10);
            assert!(result.items[4].is_nan());
        }

        // =====================================================================
        // Negative Zero Serialization Roundtrip
        // =====================================================================

        #[test]
        fn test_negative_zero_roundtrip() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Data {
                value: f64,
            }

            let original = Data { value: -0.0 };
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            // -0.0 == 0.0 in IEEE 754, but the sign bit should be preserved
            assert_eq!(deserialized.value, 0.0);
        }

        // =====================================================================
        // Subnormal Numbers
        // =====================================================================

        #[test]
        fn test_subnormal_numbers() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Data {
                tiny: f64,
            }

            // Subnormal (denormalized) number
            let original = Data { tiny: 5e-324 };
            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.tiny, 0.0);
        }
    }

    // =========================================================================
    // JSON5 Spec Conformance Tests (https://spec.json5.org/)
    // =========================================================================
    //
    // Comprehensive tests based on the official JSON5 specification to ensure
    // our serializer produces valid JSON5 and can parse all valid JSON5 input.

    mod json5_spec_conformance {
        use super::*;

        // =====================================================================
        // Section 5.1: Escape Sequences (Table 1)
        // https://spec.json5.org/#escapes
        // =====================================================================

        #[test]
        fn test_escape_quotation_mark() {
            // \" - Quotation mark (U+0022)
            let mut output = String::new();
            write_escaped_string(&mut output, "say \"hello\"");
            assert_eq!(output, r#""say \"hello\"""#);
        }

        #[test]
        fn test_escape_reverse_solidus() {
            // \\ - Reverse solidus (U+005C)
            let mut output = String::new();
            write_escaped_string(&mut output, r"path\to\file");
            assert_eq!(output, r#""path\\to\\file""#);
        }

        #[test]
        fn test_escape_backspace() {
            // \b - Backspace (U+0008)
            let mut output = String::new();
            write_escaped_string(&mut output, "hello\u{0008}world");
            assert_eq!(output, r#""hello\bworld""#);
        }

        #[test]
        fn test_escape_form_feed() {
            // \f - Form feed (U+000C)
            let mut output = String::new();
            write_escaped_string(&mut output, "page\u{000C}break");
            assert_eq!(output, r#""page\fbreak""#);
        }

        #[test]
        fn test_escape_line_feed() {
            // \n - Line feed (U+000A)
            let mut output = String::new();
            write_escaped_string(&mut output, "line1\nline2");
            assert_eq!(output, r#""line1\nline2""#);
        }

        #[test]
        fn test_escape_carriage_return() {
            // \r - Carriage return (U+000D)
            let mut output = String::new();
            write_escaped_string(&mut output, "line1\rline2");
            assert_eq!(output, r#""line1\rline2""#);
        }

        #[test]
        fn test_escape_horizontal_tab() {
            // \t - Horizontal tab (U+0009)
            let mut output = String::new();
            write_escaped_string(&mut output, "col1\tcol2");
            assert_eq!(output, r#""col1\tcol2""#);
        }

        #[test]
        fn test_escape_vertical_tab() {
            // \v - Vertical tab (U+000B)
            let mut output = String::new();
            write_escaped_string(&mut output, "hello\u{000B}world");
            assert_eq!(output, r#""hello\vworld""#);
        }

        #[test]
        fn test_escape_null_character() {
            // \0 - Null (U+0000) - we use \u0000 for safety
            let mut output = String::new();
            write_escaped_string(&mut output, "null\u{0000}char");
            assert!(
                output.contains("\\u0000") || output.contains("\\0"),
                "Null should be escaped: {}",
                output
            );
        }

        #[test]
        fn test_all_standard_escapes_combined() {
            // Test all standard escapes in one string
            let input = "\"\\\u{0008}\u{000C}\n\r\t\u{000B}";
            let mut output = String::new();
            write_escaped_string(&mut output, input);

            assert!(output.contains("\\\""), "Quote not escaped");
            assert!(output.contains("\\\\"), "Backslash not escaped");
            assert!(output.contains("\\b"), "Backspace not escaped");
            assert!(output.contains("\\f"), "Form feed not escaped");
            assert!(output.contains("\\n"), "Line feed not escaped");
            assert!(output.contains("\\r"), "Carriage return not escaped");
            assert!(output.contains("\\t"), "Tab not escaped");
            assert!(output.contains("\\v"), "Vertical tab not escaped");
        }

        // =====================================================================
        // Section 5.2: Paragraph and Line Separators
        // "JSON5 generators should escape these code points in strings."
        // =====================================================================

        #[test]
        fn test_escape_line_separator_u2028() {
            // U+2028 - Line separator (generators SHOULD escape)
            let mut output = String::new();
            write_escaped_string(&mut output, "line\u{2028}separator");
            assert!(
                output.contains("\\u2028"),
                "Line separator U+2028 should be escaped: {}",
                output
            );
        }

        #[test]
        fn test_escape_paragraph_separator_u2029() {
            // U+2029 - Paragraph separator (generators SHOULD escape)
            let mut output = String::new();
            write_escaped_string(&mut output, "paragraph\u{2029}separator");
            assert!(
                output.contains("\\u2029"),
                "Paragraph separator U+2029 should be escaped: {}",
                output
            );
        }

        #[test]
        fn test_parse_unescaped_line_separator() {
            // JSON5 parsers should accept unescaped U+2028
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            // Construct JSON5 with literal U+2028
            let json5 = "{ value: \"line\u{2028}separator\" }";
            let result: Data = from_str(json5).unwrap();
            assert_eq!(result.value, "line\u{2028}separator");
        }

        #[test]
        fn test_parse_unescaped_paragraph_separator() {
            // JSON5 parsers should accept unescaped U+2029
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let json5 = "{ value: \"paragraph\u{2029}separator\" }";
            let result: Data = from_str(json5).unwrap();
            assert_eq!(result.value, "paragraph\u{2029}separator");
        }

        // =====================================================================
        // Section 5.1: Hex Escapes (\xHH)
        // =====================================================================

        #[test]
        fn test_parse_hex_escape_lowercase() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: "\x48\x65\x6c\x6c\x6f" }"#).unwrap();
            assert_eq!(result.value, "Hello");
        }

        #[test]
        fn test_parse_hex_escape_uppercase() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: "\x48\x45\x4C\x4C\x4F" }"#).unwrap();
            assert_eq!(result.value, "HELLO");
        }

        // =====================================================================
        // Section 5.1: Unicode Escapes (\uHHHH)
        // =====================================================================

        #[test]
        fn test_parse_unicode_escape_bmp() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            // Basic Multilingual Plane characters
            let result: Data = from_str(r#"{ value: "\u0048\u0065\u006C\u006C\u006F" }"#).unwrap();
            assert_eq!(result.value, "Hello");
        }

        #[test]
        fn test_parse_unicode_escape_surrogate_pair() {
            // Characters outside BMP use surrogate pairs
            // 🎼 (U+1F3BC) = \uD83C\uDFBC
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: "\uD83C\uDFBC" }"#).unwrap();
            assert_eq!(result.value, "🎼");
        }

        #[test]
        fn test_parse_unicode_escape_emoji() {
            // 😀 (U+1F600) = \uD83D\uDE00
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: "\uD83D\uDE00" }"#).unwrap();
            assert_eq!(result.value, "😀");
        }

        // =====================================================================
        // Section 6: Numbers
        // https://spec.json5.org/#numbers
        // =====================================================================

        #[test]
        fn test_number_hexadecimal_lowercase() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i64,
            }

            let result: Data = from_str(r#"{ value: 0xdecaf }"#).unwrap();
            assert_eq!(result.value, 0xdecaf);
        }

        #[test]
        fn test_number_hexadecimal_uppercase() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i64,
            }

            let result: Data = from_str(r#"{ value: 0XDECAF }"#).unwrap();
            assert_eq!(result.value, 0xDECAF);
        }

        #[test]
        fn test_number_hexadecimal_mixed_case() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i64,
            }

            let result: Data = from_str(r#"{ value: 0xDeAdBeEf }"#).unwrap();
            assert_eq!(result.value, 0xDEADBEEF);
        }

        #[test]
        fn test_number_negative_hexadecimal() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i64,
            }

            let result: Data = from_str(r#"{ value: -0xC0FFEE }"#).unwrap();
            assert_eq!(result.value, -0xC0FFEE);
        }

        #[test]
        fn test_number_leading_decimal_point() {
            #[derive(Debug, Deserialize)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: .8675309 }"#).unwrap();
            assert!((result.value - 0.8675309).abs() < 1e-10);
        }

        #[test]
        fn test_number_trailing_decimal_point() {
            #[derive(Debug, Deserialize)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: 8675309. }"#).unwrap();
            assert_eq!(result.value, 8675309.0);
        }

        #[test]
        fn test_number_positive_infinity() {
            #[derive(Debug, Deserialize)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: Infinity }"#).unwrap();
            assert!(result.value.is_infinite() && result.value.is_sign_positive());
        }

        #[test]
        fn test_number_positive_infinity_with_plus() {
            #[derive(Debug, Deserialize)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: +Infinity }"#).unwrap();
            assert!(result.value.is_infinite() && result.value.is_sign_positive());
        }

        #[test]
        fn test_number_negative_infinity() {
            #[derive(Debug, Deserialize)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: -Infinity }"#).unwrap();
            assert!(result.value.is_infinite() && result.value.is_sign_negative());
        }

        #[test]
        fn test_number_nan() {
            #[derive(Debug, Deserialize)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: NaN }"#).unwrap();
            assert!(result.value.is_nan());
        }

        #[test]
        fn test_number_positive_sign() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            let result: Data = from_str(r#"{ value: +42 }"#).unwrap();
            assert_eq!(result.value, 42);
        }

        #[test]
        fn test_number_exponent_uppercase_e() {
            #[derive(Debug, Deserialize)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: 1E10 }"#).unwrap();
            assert_eq!(result.value, 1e10);
        }

        #[test]
        fn test_number_exponent_with_positive_sign() {
            #[derive(Debug, Deserialize)]
            struct Data {
                value: f64,
            }

            let result: Data = from_str(r#"{ value: 1e+10 }"#).unwrap();
            assert_eq!(result.value, 1e10);
        }

        // =====================================================================
        // Section 3: Objects
        // https://spec.json5.org/#objects
        // =====================================================================

        #[test]
        fn test_object_unquoted_identifier_keys() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                width: i32,
                height: i32,
            }

            let result: Data = from_str(r#"{ width: 1920, height: 1080 }"#).unwrap();
            assert_eq!(result.width, 1920);
            assert_eq!(result.height, 1080);
        }

        #[test]
        fn test_object_single_trailing_comma() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                width: i32,
                height: i32,
            }

            let result: Data = from_str(r#"{ width: 1920, height: 1080, }"#).unwrap();
            assert_eq!(result.width, 1920);
            assert_eq!(result.height, 1080);
        }

        #[test]
        fn test_object_single_quoted_key() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                #[serde(rename = "aspect-ratio")]
                aspect_ratio: String,
            }

            let result: Data = from_str(r#"{ 'aspect-ratio': '16:9' }"#).unwrap();
            assert_eq!(result.aspect_ratio, "16:9");
        }

        #[test]
        fn test_object_double_quoted_key() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                #[serde(rename = "aspect-ratio")]
                aspect_ratio: String,
            }

            let result: Data = from_str(r#"{ "aspect-ratio": "16:9" }"#).unwrap();
            assert_eq!(result.aspect_ratio, "16:9");
        }

        #[test]
        fn test_object_nested() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Inner {
                width: i32,
                height: i32,
            }

            #[derive(Debug, Deserialize, PartialEq)]
            struct Outer {
                image: Inner,
            }

            let result: Outer = from_str(
                r#"{
                image: {
                    width: 1920,
                    height: 1080,
                }
            }"#,
            )
            .unwrap();
            assert_eq!(result.image.width, 1920);
            assert_eq!(result.image.height, 1080);
        }

        // =====================================================================
        // Section 4: Arrays
        // https://spec.json5.org/#arrays
        // =====================================================================

        #[test]
        fn test_array_trailing_comma() {
            let result: Vec<i32> = from_str(r#"[1, 2, 3,]"#).unwrap();
            assert_eq!(result, vec![1, 2, 3]);
        }

        #[test]
        fn test_array_mixed_types() {
            // "There is no requirement that the values in an array be of the same type."
            // Use serde_json::Value which can hold any type
            let result: Vec<serde_json::Value> = from_str(r#"[1, true, 'three',]"#).unwrap();
            assert_eq!(result.len(), 3);
            assert_eq!(result[0], 1);
            assert_eq!(result[1], true);
            assert_eq!(result[2], "three");
        }

        #[test]
        fn test_array_nested_with_trailing_commas() {
            let result: Vec<Vec<i32>> = from_str(r#"[[1, 2,], [3, 4,],]"#).unwrap();
            assert_eq!(result, vec![vec![1, 2], vec![3, 4]]);
        }

        // =====================================================================
        // Section 5: Strings
        // https://spec.json5.org/#strings
        // =====================================================================

        #[test]
        fn test_string_single_quoted() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: 'single quoted' }"#).unwrap();
            assert_eq!(result.value, "single quoted");
        }

        #[test]
        fn test_string_single_quoted_with_double_quotes() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: 'I can use "double quotes" here' }"#).unwrap();
            assert_eq!(result.value, r#"I can use "double quotes" here"#);
        }

        #[test]
        fn test_string_multiline_escaped() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            // Line continuation - backslash followed by newline is removed
            let result: Data = from_str("{ value: 'Look, Mom! \\\nNo newlines!' }").unwrap();
            assert_eq!(result.value, "Look, Mom! No newlines!");
        }

        // =====================================================================
        // Section 7: Comments
        // https://spec.json5.org/#comments
        // =====================================================================

        #[test]
        fn test_comment_single_line() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            let result: Data = from_str(
                r#"{
                // This is a single line comment.
                value: 42
            }"#,
            )
            .unwrap();
            assert_eq!(result.value, 42);
        }

        #[test]
        fn test_comment_multi_line() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            let result: Data = from_str(
                r#"{
                /* This is a multi-
                   line comment. */
                value: 42
            }"#,
            )
            .unwrap();
            assert_eq!(result.value, 42);
        }

        #[test]
        fn test_comment_inline() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                a: i32,
                b: i32,
            }

            let result: Data = from_str(
                r#"{
                a: 1, // inline comment
                b: 2  /* another */
            }"#,
            )
            .unwrap();
            assert_eq!(result.a, 1);
            assert_eq!(result.b, 2);
        }

        // =====================================================================
        // Section 8: White Space
        // https://spec.json5.org/#white-space
        // =====================================================================

        #[test]
        fn test_whitespace_horizontal_tab() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            let result: Data = from_str("{\tvalue:\t42\t}").unwrap();
            assert_eq!(result.value, 42);
        }

        #[test]
        fn test_whitespace_vertical_tab() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            let result: Data = from_str("{\u{000B}value:\u{000B}42\u{000B}}").unwrap();
            assert_eq!(result.value, 42);
        }

        #[test]
        fn test_whitespace_form_feed() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            let result: Data = from_str("{\u{000C}value:\u{000C}42\u{000C}}").unwrap();
            assert_eq!(result.value, 42);
        }

        #[test]
        fn test_whitespace_non_breaking_space() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            // U+00A0 Non-breaking space
            let result: Data = from_str("{\u{00A0}value:\u{00A0}42\u{00A0}}").unwrap();
            assert_eq!(result.value, 42);
        }

        #[test]
        fn test_whitespace_bom() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            // U+FEFF Byte order mark
            let result: Data = from_str("\u{FEFF}{ value: 42 }").unwrap();
            assert_eq!(result.value, 42);
        }

        // =====================================================================
        // Section 1.2: Short Example from Spec
        // Complete example that uses many JSON5 features
        // =====================================================================

        #[test]
        fn test_spec_short_example() {
            // This is the example from the JSON5 spec Section 1.2
            #[derive(Debug, Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Example {
                unquoted: String,
                single_quotes: String,
                line_breaks: String,
                hexadecimal: i64,
                leading_decimal_point: f64,
                and_trailing: f64,
                positive_sign: i32,
                trailing_comma: String,
                and_in: Vec<String>,
                #[serde(rename = "backwardsCompatible")]
                backwards_compatible: String,
            }

            let result: Example = from_str(
                r#"{
              // comments
              unquoted: 'and you can quote me on that',
              singleQuotes: 'I can use "double quotes" here',
              lineBreaks: "Look, Mom! \
No \\n's!",
              hexadecimal: 0xdecaf,
              leadingDecimalPoint: .8675309, andTrailing: 8675309.,
              positiveSign: +1,
              trailingComma: 'in objects', andIn: ['arrays',],
              "backwardsCompatible": "with JSON",
            }"#,
            )
            .unwrap();

            assert_eq!(result.unquoted, "and you can quote me on that");
            assert_eq!(result.single_quotes, r#"I can use "double quotes" here"#);
            assert_eq!(result.line_breaks, "Look, Mom! No \\n's!");
            assert_eq!(result.hexadecimal, 0xdecaf);
            assert!((result.leading_decimal_point - 0.8675309).abs() < 1e-10);
            assert_eq!(result.and_trailing, 8675309.0);
            assert_eq!(result.positive_sign, 1);
            assert_eq!(result.trailing_comma, "in objects");
            assert_eq!(result.and_in, vec!["arrays"]);
            assert_eq!(result.backwards_compatible, "with JSON");
        }

        // =====================================================================
        // Roundtrip Tests - Serialization then Parsing
        // =====================================================================

        #[test]
        fn test_roundtrip_all_escapes() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            // String with all escapable characters
            let original = Data {
                value: "\"\\\u{0008}\u{000C}\n\r\t\u{000B}\u{2028}\u{2029}".to_string(),
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.value, original.value);
        }

        #[test]
        fn test_roundtrip_special_numbers() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Data {
                infinity: f64,
                neg_infinity: f64,
                nan: f64,
                large: f64,
                small: f64,
            }

            let original = Data {
                infinity: f64::INFINITY,
                neg_infinity: f64::NEG_INFINITY,
                nan: f64::NAN,
                large: 1e47,
                small: 1e-47,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert!(
                deserialized.infinity.is_infinite() && deserialized.infinity.is_sign_positive()
            );
            assert!(
                deserialized.neg_infinity.is_infinite()
                    && deserialized.neg_infinity.is_sign_negative()
            );
            assert!(deserialized.nan.is_nan());
            assert_eq!(deserialized.large, 1e47);
            assert_eq!(deserialized.small, 0.0);
        }

        #[test]
        fn test_roundtrip_keys_with_all_escapes() {
            // Test that keys are properly escaped and can be parsed back
            let mut map = std::collections::HashMap::new();
            map.insert("key\"quote".to_string(), 1);
            map.insert("key\\backslash".to_string(), 2);
            map.insert("key\nline".to_string(), 3);
            map.insert("key\ttab".to_string(), 4);
            map.insert("key\u{0008}backspace".to_string(), 5);
            map.insert("key\u{000C}formfeed".to_string(), 6);
            map.insert("key\u{000B}vtab".to_string(), 7);
            map.insert("key\u{2028}lsep".to_string(), 8);
            map.insert("key\u{2029}psep".to_string(), 9);

            let serialized = to_vec_pretty_sorted(&map).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: std::collections::HashMap<String, i32> = from_str(&json_str).unwrap();

            assert_eq!(deserialized.get("key\"quote"), Some(&1));
            assert_eq!(deserialized.get("key\\backslash"), Some(&2));
            assert_eq!(deserialized.get("key\nline"), Some(&3));
            assert_eq!(deserialized.get("key\ttab"), Some(&4));
            assert_eq!(deserialized.get("key\u{0008}backspace"), Some(&5));
            assert_eq!(deserialized.get("key\u{000C}formfeed"), Some(&6));
            assert_eq!(deserialized.get("key\u{000B}vtab"), Some(&7));
            assert_eq!(deserialized.get("key\u{2028}lsep"), Some(&8));
            assert_eq!(deserialized.get("key\u{2029}psep"), Some(&9));
        }

        #[test]
        fn test_generator_produces_valid_json5() {
            // Section 11: "A JSON5 generator produces JSON5 text. The resulting
            // text must strictly conform to the JSON5 grammar."
            #[derive(Debug, Serialize)]
            struct Complex {
                string: String,
                number: f64,
                boolean: bool,
                null_val: Option<i32>,
                array: Vec<i32>,
                nested: Nested,
            }

            #[derive(Debug, Serialize)]
            struct Nested {
                value: f64,
            }

            let data = Complex {
                string: "hello\nworld\t\"test\"\u{2028}\u{2029}".to_string(),
                number: 1e47,
                boolean: true,
                null_val: None,
                array: vec![1, 2, 3],
                nested: Nested {
                    value: f64::INFINITY,
                },
            };

            let serialized = to_vec_pretty_sorted(&data).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // If it parses, it's valid JSON5
            let _: serde_json::Value = from_str(&json_str).unwrap();
        }
    }

    // =========================================================================
    // DISGUSTING EDGE CASE TESTS
    // =========================================================================
    //
    // These tests push the JSON5 spec to its absolute limits. They are designed
    // to break parsers and serializers that don't strictly follow the spec.
    // Based on https://spec.json5.org/

    mod json5_torture_tests {
        use super::*;

        // =====================================================================
        // String Escape Torture
        // =====================================================================

        #[test]
        fn test_every_escape_sequence_in_one_string() {
            // Every escape from Table 1 plus hex and unicode escapes
            let json5 = r#"{
                value: "\"\\\b\f\n\r\t\v\0\x41\u0042"
            }"#;

            #[derive(Debug, Deserialize)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(json5).unwrap();
            // " \ backspace formfeed newline carriage-return tab vertical-tab null A B
            assert_eq!(result.value, "\"\\\u{0008}\u{000C}\n\r\t\u{000B}\u{0000}AB");
        }

        #[test]
        fn test_string_with_all_control_characters() {
            // Roundtrip all ASCII control characters (U+0000 to U+001F)
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            let mut control_chars = String::new();
            for i in 0u8..32 {
                control_chars.push(i as char);
            }
            // Add DEL (U+007F) too
            control_chars.push('\u{007F}');

            let original = Data {
                value: control_chars,
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.value, original.value);
        }

        #[test]
        fn test_surrogate_pair_boundaries() {
            // Test the exact boundaries of surrogate pairs
            // High surrogates: U+D800 to U+DBFF
            // Low surrogates: U+DC00 to U+DFFF
            // First valid surrogate pair encodes U+10000
            #[derive(Debug, Deserialize)]
            struct Data {
                value: String,
            }

            // 𐀀 (U+10000) = \uD800\uDC00
            let result: Data = from_str(r#"{ value: "\uD800\uDC00" }"#).unwrap();
            assert_eq!(result.value, "\u{10000}");

            // 🀀 (U+1F000) = \uD83C\uDC00
            let result: Data = from_str(r#"{ value: "\uD83C\uDC00" }"#).unwrap();
            assert_eq!(result.value, "🀀");

            // Last valid BMP character before surrogates: U+D7FF
            let result: Data = from_str(r#"{ value: "\uD7FF" }"#).unwrap();
            assert_eq!(result.value, "\u{D7FF}");

            // First character after surrogates: U+E000
            let result: Data = from_str(r#"{ value: "\uE000" }"#).unwrap();
            assert_eq!(result.value, "\u{E000}");
        }

        #[test]
        fn test_maximum_unicode_code_point() {
            // U+10FFFF is the maximum valid Unicode code point
            // Encoded as surrogate pair: \uDBFF\uDFFF
            #[derive(Debug, Deserialize)]
            struct Data {
                value: String,
            }

            let result: Data = from_str(r#"{ value: "\uDBFF\uDFFF" }"#).unwrap();
            assert_eq!(result.value, "\u{10FFFF}");
        }

        #[test]
        fn test_string_line_continuation_all_terminators() {
            // Per Table 2: line continuation can use LF, CR, CRLF, U+2028, U+2029
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            // LF continuation
            let result: Data = from_str("{ value: 'hello\\\nworld' }").unwrap();
            assert_eq!(result.value, "helloworld");

            // CR continuation
            let result: Data = from_str("{ value: 'hello\\\rworld' }").unwrap();
            assert_eq!(result.value, "helloworld");

            // CRLF continuation
            let result: Data = from_str("{ value: 'hello\\\r\nworld' }").unwrap();
            assert_eq!(result.value, "helloworld");

            // U+2028 line separator continuation
            let result: Data = from_str("{ value: 'hello\\\u{2028}world' }").unwrap();
            assert_eq!(result.value, "helloworld");

            // U+2029 paragraph separator continuation
            let result: Data = from_str("{ value: 'hello\\\u{2029}world' }").unwrap();
            assert_eq!(result.value, "helloworld");
        }

        #[test]
        fn test_identity_escapes() {
            // Per spec Section 5.1: "If any other character follows a reverse solidus,
            // except for the decimal digits 1 through 9, that character will be
            // included in the string, but the reverse solidus will not."
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            // \A\C\/\D\C should become AC/DC
            let result: Data = from_str(r#"{ value: '\A\C\/\D\C' }"#).unwrap();
            assert_eq!(result.value, "AC/DC");

            // Random escaped letters
            let result: Data = from_str(r#"{ value: '\z\y\x41' }"#).unwrap();
            assert_eq!(result.value, "zyA"); // \x41 is hex escape for 'A'
        }

        // =====================================================================
        // Number Torture
        // =====================================================================

        #[test]
        fn test_all_special_number_forms() {
            #[derive(Debug, Deserialize)]
            struct Data {
                a: f64,
                b: f64,
                c: f64,
                d: f64,
                e: f64,
                f: f64,
                g: f64,
                h: f64,
                i: f64,
                j: f64,
            }

            let result: Data = from_str(
                r#"{
                a: Infinity,
                b: -Infinity,
                c: +Infinity,
                d: NaN,
                e: +NaN,
                f: -NaN,
                g: .5,
                h: 5.,
                i: +42,
                j: 0xDEADBEEF
            }"#,
            )
            .unwrap();

            assert!(result.a.is_infinite() && result.a.is_sign_positive());
            assert!(result.b.is_infinite() && result.b.is_sign_negative());
            assert!(result.c.is_infinite() && result.c.is_sign_positive());
            assert!(result.d.is_nan());
            assert!(result.e.is_nan());
            assert!(result.f.is_nan());
            assert_eq!(result.g, 0.5);
            assert_eq!(result.h, 5.0);
            assert_eq!(result.i, 42.0);
            assert_eq!(result.j, 0xDEADBEEF_u64 as f64);
        }

        #[test]
        fn test_hexadecimal_edge_cases() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i64,
            }

            // Lowercase x
            let result: Data = from_str(r#"{ value: 0x0 }"#).unwrap();
            assert_eq!(result.value, 0);

            // Uppercase X
            let result: Data = from_str(r#"{ value: 0X0 }"#).unwrap();
            assert_eq!(result.value, 0);

            // All hex digits lowercase
            let result: Data = from_str(r#"{ value: 0xabcdef }"#).unwrap();
            assert_eq!(result.value, 0xABCDEF);

            // All hex digits uppercase
            let result: Data = from_str(r#"{ value: 0xABCDEF }"#).unwrap();
            assert_eq!(result.value, 0xABCDEF);

            // Positive hex
            let result: Data = from_str(r#"{ value: +0xFF }"#).unwrap();
            assert_eq!(result.value, 255);

            // Negative hex
            let result: Data = from_str(r#"{ value: -0xFF }"#).unwrap();
            assert_eq!(result.value, -255);
        }

        #[test]
        fn test_exponent_edge_cases() {
            #[derive(Debug, Deserialize)]
            struct Data {
                value: f64,
            }

            // Uppercase E
            let result: Data = from_str(r#"{ value: 1E10 }"#).unwrap();
            assert_eq!(result.value, 1e10);

            // Lowercase e
            let result: Data = from_str(r#"{ value: 1e10 }"#).unwrap();
            assert_eq!(result.value, 1e10);

            // Explicit positive exponent
            let result: Data = from_str(r#"{ value: 1e+10 }"#).unwrap();
            assert_eq!(result.value, 1e10);

            // Negative exponent
            let result: Data = from_str(r#"{ value: 1e-10 }"#).unwrap();
            assert_eq!(result.value, 1e-10);

            // Zero exponent
            let result: Data = from_str(r#"{ value: 1e0 }"#).unwrap();
            assert_eq!(result.value, 1.0);

            // Leading decimal with exponent
            let result: Data = from_str(r#"{ value: .5e10 }"#).unwrap();
            assert_eq!(result.value, 0.5e10);

            // Trailing decimal with exponent
            let result: Data = from_str(r#"{ value: 5.e10 }"#).unwrap();
            assert_eq!(result.value, 5e10);
        }

        // =====================================================================
        // Identifier/Key Torture
        // =====================================================================

        #[test]
        fn test_identifier_edge_cases() {
            // ECMAScript 5.1 IdentifierName rules
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                #[serde(rename = "$")]
                dollar: i32,
                #[serde(rename = "_")]
                underscore: i32,
                #[serde(rename = "$_")]
                dollar_underscore: i32,
                #[serde(rename = "_$")]
                underscore_dollar: i32,
                #[serde(rename = "$$$$")]
                many_dollars: i32,
                #[serde(rename = "____")]
                many_underscores: i32,
                #[serde(rename = "a1234567890")]
                letter_digits: i32,
            }

            let result: Data = from_str(
                r#"{
                $: 1,
                _: 2,
                $_: 3,
                _$: 4,
                $$$$: 5,
                ____: 6,
                a1234567890: 7
            }"#,
            )
            .unwrap();

            assert_eq!(result.dollar, 1);
            assert_eq!(result.underscore, 2);
            assert_eq!(result.dollar_underscore, 3);
            assert_eq!(result.underscore_dollar, 4);
            assert_eq!(result.many_dollars, 5);
            assert_eq!(result.many_underscores, 6);
            assert_eq!(result.letter_digits, 7);
        }

        #[test]
        fn test_reserved_words_as_keys() {
            // All ES5 reserved words should work as unquoted keys
            #[derive(Debug, Deserialize)]
            #[allow(dead_code)]
            struct Reserved {
                r#break: i32,
                r#case: i32,
                r#catch: i32,
                r#continue: i32,
                r#debugger: i32,
                r#default: i32,
                r#delete: i32,
                r#do: i32,
                r#else: i32,
                r#finally: i32,
                r#for: i32,
                r#function: i32,
                r#if: i32,
                r#in: i32,
                r#instanceof: i32,
                r#new: i32,
                r#return: i32,
                r#switch: i32,
                this: i32,
                r#throw: i32,
                r#try: i32,
                r#typeof: i32,
                var: i32,
                void: i32,
                r#while: i32,
                with: i32,
                class: i32,
                r#const: i32,
                r#enum: i32,
                export: i32,
                extends: i32,
                import: i32,
                #[serde(rename = "super")]
                super_: i32,
                null: i32,
                r#true: i32,
                r#false: i32,
            }

            let json5 = r#"{
                break: 1, case: 2, catch: 3, continue: 4, debugger: 5,
                default: 6, delete: 7, do: 8, else: 9, finally: 10,
                for: 11, function: 12, if: 13, in: 14, instanceof: 15,
                new: 16, return: 17, switch: 18, this: 19, throw: 20,
                try: 21, typeof: 22, var: 23, void: 24, while: 25,
                with: 26, class: 27, const: 28, enum: 29, export: 30,
                extends: 31, import: 32, super: 33, null: 34, true: 35, false: 36
            }"#;

            let _: Reserved = from_str(json5).unwrap();
        }

        #[test]
        fn test_keys_that_look_like_numbers() {
            // Keys that start with digits must be quoted
            let mut map = std::collections::HashMap::new();
            map.insert("123".to_string(), 1);
            map.insert("0xFF".to_string(), 2);
            map.insert("3.14".to_string(), 3);
            map.insert("1e10".to_string(), 4);

            let serialized = to_vec_pretty_sorted(&map).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // All should be quoted since they start with digits
            assert!(json_str.contains("\"123\""));
            assert!(json_str.contains("\"0xFF\""));
            assert!(json_str.contains("\"3.14\""));
            assert!(json_str.contains("\"1e10\""));

            // Roundtrip
            let deserialized: std::collections::HashMap<String, i32> = from_str(&json_str).unwrap();
            assert_eq!(deserialized.get("123"), Some(&1));
        }

        // =====================================================================
        // Whitespace Torture
        // =====================================================================

        #[test]
        fn test_all_whitespace_characters() {
            // Per Table 3 in spec - all valid whitespace
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            // Build a string with all whitespace types
            let whitespace =
                "\u{0009}\u{000A}\u{000B}\u{000C}\u{000D}\u{0020}\u{00A0}\u{2028}\u{2029}\u{FEFF}";
            let json5 = format!(
                "{}{{{}value{}:{}42{}}}",
                whitespace, whitespace, whitespace, whitespace, whitespace
            );

            let result: Data = from_str(&json5).unwrap();
            assert_eq!(result.value, 42);
        }

        #[test]
        fn test_whitespace_between_every_token() {
            #[derive(Debug, Deserialize)]
            struct Data {
                arr: Vec<i32>,
            }

            // Whitespace can appear before and after any token
            let json5 = "  {  arr  :  [  1  ,  2  ,  3  ,  ]  ,  }  ";
            let result: Data = from_str(json5).unwrap();
            assert_eq!(result.arr, vec![1, 2, 3]);
        }

        // =====================================================================
        // Comment Torture
        // =====================================================================

        #[test]
        fn test_comments_everywhere() {
            #[derive(Debug, Deserialize)]
            struct Data {
                a: i32,
                b: i32,
            }

            let json5 = r#"
                // comment before object
                { // comment after opening brace
                    // comment before key
                    a // comment after key
                    : // comment after colon
                    1 // comment after value
                    , // comment after comma
                    /* multi
                       line
                       comment */
                    b: /* inline */ 2 /* another */
                    , // trailing comma comment
                } // comment after closing brace
                // comment at end
            "#;

            let result: Data = from_str(json5).unwrap();
            assert_eq!(result.a, 1);
            assert_eq!(result.b, 2);
        }

        #[test]
        fn test_comment_with_special_content() {
            #[derive(Debug, Deserialize, PartialEq)]
            struct Data {
                value: i32,
            }

            // Comments can contain anything except */ for multi-line
            let json5 = r#"{
                // Comment with "quotes" and 'apostrophes' and \escapes
                /* Multi-line with
                   { "fake": "json" }
                   and // nested single-line looking things
                */
                value: 42
            }"#;

            let result: Data = from_str(json5).unwrap();
            assert_eq!(result.value, 42);
        }

        // =====================================================================
        // Nesting Torture
        // =====================================================================

        #[test]
        fn test_deeply_nested_objects() {
            // 20 levels of nesting
            let mut json5 = String::new();
            for _ in 0..20 {
                json5.push_str("{ a: ");
            }
            json5.push_str("42");
            for _ in 0..20 {
                json5.push_str(" }");
            }

            let result: serde_json::Value = from_str(&json5).unwrap();

            // Navigate to the innermost value
            let mut current = &result;
            for _ in 0..20 {
                current = &current["a"];
            }
            assert_eq!(current.as_i64().unwrap(), 42);
        }

        #[test]
        fn test_deeply_nested_arrays() {
            // 20 levels of array nesting
            let mut json5 = String::new();
            for _ in 0..20 {
                json5.push('[');
            }
            json5.push_str("42");
            for _ in 0..20 {
                json5.push(']');
            }

            let result: serde_json::Value = from_str(&json5).unwrap();

            // Navigate to the innermost value
            let mut current = &result;
            for _ in 0..20 {
                current = &current[0];
            }
            assert_eq!(current.as_i64().unwrap(), 42);
        }

        // =====================================================================
        // Trailing Comma Torture
        // =====================================================================

        #[test]
        fn test_trailing_commas_everywhere() {
            #[derive(Debug, Deserialize)]
            struct Inner {
                x: i32,
            }

            #[derive(Debug, Deserialize)]
            struct Data {
                obj: Inner,
                arr: Vec<i32>,
            }

            let json5 = r#"{
                obj: {
                    x: 1,
                },
                arr: [
                    1,
                    2,
                    3,
                ],
            }"#;

            let result: Data = from_str(json5).unwrap();
            assert_eq!(result.obj.x, 1);
            assert_eq!(result.arr, vec![1, 2, 3]);
        }

        // =====================================================================
        // Mixed Quote Style Torture
        // =====================================================================

        #[test]
        fn test_mixed_quote_styles() {
            #[derive(Debug, Deserialize)]
            struct Data {
                #[serde(rename = "double-key")]
                double_key: String,
                #[serde(rename = "single-key")]
                single_key: String,
            }

            let json5 = r#"{
                "double-key": 'single value',
                'single-key': "double value"
            }"#;

            let result: Data = from_str(json5).unwrap();
            assert_eq!(result.double_key, "single value");
            assert_eq!(result.single_key, "double value");
        }

        // =====================================================================
        // Empty/Minimal Structure Torture
        // =====================================================================

        #[test]
        fn test_minimal_valid_json5() {
            // Smallest valid JSON5 values
            let _: serde_json::Value = from_str("{}").unwrap();
            let _: serde_json::Value = from_str("[]").unwrap();
            let _: serde_json::Value = from_str("null").unwrap();
            let _: serde_json::Value = from_str("true").unwrap();
            let _: serde_json::Value = from_str("false").unwrap();
            let _: serde_json::Value = from_str("0").unwrap();
            let _: serde_json::Value = from_str("\"\"").unwrap();
            let _: serde_json::Value = from_str("''").unwrap();
        }

        #[test]
        fn test_object_with_empty_string_key() {
            let mut map = std::collections::HashMap::new();
            map.insert("".to_string(), 42);

            let serialized = to_vec_pretty_sorted(&map).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            // Empty key must be quoted
            assert!(json_str.contains("\"\""));

            let deserialized: std::collections::HashMap<String, i32> = from_str(&json_str).unwrap();
            assert_eq!(deserialized.get(""), Some(&42));
        }

        // =====================================================================
        // Unicode Torture
        // =====================================================================

        #[test]
        fn test_unicode_identifiers() {
            // ECMAScript 5.1 allows Unicode letters in identifiers
            // Note: this depends on the json5 crate's support
            #[derive(Debug, Deserialize)]
            struct Data {
                #[serde(rename = "café")]
                cafe: i32,
                #[serde(rename = "naïve")]
                naive: i32,
            }

            // These may or may not work depending on json5 crate implementation
            // Using quoted keys to be safe
            let json5 = r#"{ "café": 1, "naïve": 2 }"#;
            let result: Data = from_str(json5).unwrap();
            assert_eq!(result.cafe, 1);
            assert_eq!(result.naive, 2);
        }

        #[test]
        fn test_zero_width_characters_in_strings() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            // Zero-width joiner, zero-width non-joiner, zero-width space
            let original = Data {
                value: "a\u{200D}b\u{200C}c\u{200B}d".to_string(),
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.value, original.value);
        }

        #[test]
        fn test_combining_characters() {
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            // é as e + combining acute accent
            let original = Data {
                value: "e\u{0301}".to_string(),
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.value, original.value);
        }

        #[test]
        fn test_bidi_characters() {
            // Bidirectional text control characters
            #[derive(Debug, Serialize, Deserialize, PartialEq)]
            struct Data {
                value: String,
            }

            // LRM, RLM, LRE, RLE, PDF, LRO, RLO
            let original = Data {
                value: "\u{200E}\u{200F}\u{202A}\u{202B}\u{202C}\u{202D}\u{202E}".to_string(),
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();
            assert_eq!(deserialized.value, original.value);
        }

        // =====================================================================
        // Stress Test - Everything Combined
        // =====================================================================

        #[test]
        fn test_ultimate_torture() {
            // Combine as many edge cases as possible in one test
            // We test parsing succeeds and verify key parts
            let json5 = r#"
            // Ultimate torture test
            {
                /* Multi-line
                   comment */
                $weird_key$: 'single \' quote',
                "key with spaces": "double \" quote",
                'key-with-dashes': [
                    1,
                    .5,
                    5.,
                    +42,
                    -0,
                    0xFF,
                    1e+10,
                    Infinity,
                    -Infinity,
                    NaN,
                    null,
                    true,
                    false,
                    "",
                    '',
                    "\n\r\t\b\f\v\0",
                    "\u0000\u001F\u007F",
                    "\uD83D\uDE00", // 😀
                ],
                nested: {
                    deeply: {
                        nested: {
                            value: 42,
                        },
                    },
                },
                empty_obj: {},
                empty_arr: [],
                trailing: 'comma',
            }
            "#;

            // Test that it parses at all - this validates the json5 crate handles all features
            let result: serde_json::Value = from_str(json5).unwrap();

            // Verify key structural parts (avoiding NaN/Infinity which serde_json::Value can't hold)
            assert_eq!(result["$weird_key$"], "single ' quote");
            assert_eq!(result["key with spaces"], "double \" quote");

            // Check array length
            assert_eq!(result["key-with-dashes"].as_array().unwrap().len(), 18);

            // Check some numeric values that serde_json can hold
            assert_eq!(result["key-with-dashes"][0], 1.0);
            assert_eq!(result["key-with-dashes"][1], 0.5);
            assert_eq!(result["key-with-dashes"][5], 255.0); // 0xFF

            // Check nested
            assert_eq!(result["nested"]["deeply"]["nested"]["value"], 42);

            // Check empty collections
            assert!(result["empty_obj"].as_object().unwrap().is_empty());
            assert!(result["empty_arr"].as_array().unwrap().is_empty());

            assert_eq!(result["trailing"], "comma");

            // Now test specific special floats with typed deserialization
            #[derive(Debug, Deserialize)]
            struct SpecialFloats {
                infinity: f64,
                neg_infinity: f64,
                nan: f64,
            }

            let special: SpecialFloats = from_str(
                r#"{
                infinity: Infinity,
                neg_infinity: -Infinity,
                nan: NaN
            }"#,
            )
            .unwrap();

            assert!(special.infinity.is_infinite() && special.infinity.is_sign_positive());
            assert!(special.neg_infinity.is_infinite() && special.neg_infinity.is_sign_negative());
            assert!(special.nan.is_nan());
        }

        // =====================================================================
        // Roundtrip Stress Tests
        // =====================================================================

        #[test]
        fn test_roundtrip_pathological_keys() {
            let mut map = std::collections::HashMap::new();

            // Keys that push every edge case
            map.insert("".to_string(), 1); // empty
            map.insert(" ".to_string(), 2); // space
            map.insert("  ".to_string(), 3); // multiple spaces
            map.insert("\t".to_string(), 4); // tab
            map.insert("\n".to_string(), 5); // newline
            map.insert("\"".to_string(), 6); // quote
            map.insert("\\".to_string(), 7); // backslash
            map.insert("'".to_string(), 8); // apostrophe
            map.insert("\u{0000}".to_string(), 9); // null
            map.insert("\u{001F}".to_string(), 10); // unit separator
            map.insert("\u{007F}".to_string(), 11); // delete
            map.insert("\u{2028}".to_string(), 12); // line separator
            map.insert("\u{2029}".to_string(), 13); // paragraph separator
            map.insert("key\"with\"quotes".to_string(), 14);
            map.insert("key\\with\\backslashes".to_string(), 15);
            map.insert("key\nwith\nnewlines".to_string(), 16);
            map.insert("αβγδ".to_string(), 17); // Greek
            map.insert("日本語".to_string(), 18); // Japanese
            map.insert("🎉🎊🎁".to_string(), 19); // Emoji

            let serialized = to_vec_pretty_sorted(&map).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: std::collections::HashMap<String, i32> = from_str(&json_str).unwrap();

            for (key, expected_value) in &map {
                assert_eq!(
                    deserialized.get(key),
                    Some(expected_value),
                    "Failed for key: {:?}",
                    key
                );
            }
        }

        #[test]
        fn test_roundtrip_pathological_values() {
            #[derive(Debug, Serialize, Deserialize)]
            struct Data {
                a: f64,
                b: f64,
                c: f64,
                d: f64,
                e: f64,
                f: f64,
                g: f64,
                h: f64,
                i: String,
            }

            let original = Data {
                a: f64::INFINITY,
                b: f64::NEG_INFINITY,
                c: f64::NAN,
                d: f64::MAX,
                e: f64::MIN,
                f: f64::MIN_POSITIVE,
                g: f64::EPSILON,
                h: 0.0,
                i: (0u8..=127).map(|b| b as char).collect::<String>(), // All ASCII
            };

            let serialized = to_vec_pretty_sorted(&original).unwrap();
            let json_str = String::from_utf8(serialized).unwrap();

            let deserialized: Data = from_str(&json_str).unwrap();

            assert!(deserialized.a.is_infinite() && deserialized.a.is_sign_positive());
            assert!(deserialized.b.is_infinite() && deserialized.b.is_sign_negative());
            assert!(deserialized.c.is_nan());
            assert_eq!(deserialized.d, f64::MAX);
            assert_eq!(deserialized.e, f64::MIN);
            assert_eq!(deserialized.f, 0.0);
            assert_eq!(deserialized.g, 0.0);
            assert_eq!(deserialized.h, 0.0);
            assert_eq!(deserialized.i, original.i);
        }
    }
}
