//! Encoding and decoding utilities for special filename characters.
//!
//! This module encodes characters that are problematic in file names:
//! - Windows-invalid characters: < > : " / \ | ? *
//! - Periods (.) to prevent conflicts with file extension parsing
//! - Leading/trailing spaces
//! - Literal percent signs (%) to allow round-tripping
//!
//! Characters are encoded using a %NAME% format (e.g., `<` becomes `%LT%`,
//! `.` becomes `%DOT%`). Literal `%` is escaped as `%%` (like printf).

use std::collections::HashMap;
use std::sync::OnceLock;

/// Mapping of special characters to their encoded representations.
const CHAR_ENCODINGS: &[(&str, &str)] = &[
    (".", "%DOT%"),
    ("<", "%LT%"),
    (">", "%GT%"),
    (":", "%COLON%"),
    ("\"", "%QUOTE%"),
    ("/", "%SLASH%"),
    ("\\", "%BACKSLASH%"),
    ("|", "%PIPE%"),
    ("?", "%QUESTION%"),
    ("*", "%STAR%"),
];

/// Returns a map from encoded pattern (e.g., "DOT") to decoded char (e.g., ".").
fn get_decode_map() -> &'static HashMap<&'static str, &'static str> {
    static DECODE_MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    DECODE_MAP.get_or_init(|| {
        let mut map = HashMap::new();
        for (char, encoded) in CHAR_ENCODINGS {
            // Strip the leading and trailing % from the pattern
            let pattern = &encoded[1..encoded.len() - 1];
            map.insert(pattern, *char);
        }
        map
    })
}

const SPACE_ENCODING: &str = "%SPACE%";

/// Encodes special characters in a file name.
///
/// This replaces characters like `.`, `<`, `>`, `:`, `"`, `/`, `\`, `|`, `?`, `*`
/// with their encoded representations like `%DOT%`, `%LT%`, `%GT%`, etc.
/// Literal `%` is escaped as `%%`. Leading and trailing spaces are encoded as `%SPACE%`.
pub fn encode_path_name(name: &str) -> String {
    // Count leading and trailing spaces first (before any modifications)
    let leading_spaces = name.len() - name.trim_start_matches(' ').len();
    let trailing_spaces = name.len() - name.trim_end_matches(' ').len();

    // Get the middle part (without leading/trailing spaces)
    let middle = &name[leading_spaces..name.len() - trailing_spaces];

    // First: escape % as %% (before adding new % signs from encodings)
    let mut encoded_middle = middle.replace('%', "%%");

    // Then: encode special characters
    for (char, encoded) in CHAR_ENCODINGS {
        encoded_middle = encoded_middle.replace(char, encoded);
    }

    // Build final result with encoded spaces
    let encoded_prefix = SPACE_ENCODING.repeat(leading_spaces);
    let encoded_suffix = SPACE_ENCODING.repeat(trailing_spaces);

    format!("{}{}{}", encoded_prefix, encoded_middle, encoded_suffix)
}

/// Decodes encoded special characters back to their original form.
///
/// This replaces encoded representations like `%DOT%`, `%LT%`, `%GT%`, etc.
/// back to their original characters `.`, `<`, `>`, etc.
/// `%%` is decoded to literal `%`. `%SPACE%` at the start/end is decoded back to spaces.
///
/// Uses a proper left-to-right parser to correctly handle `%%` escaping.
pub fn decode_path_name(name: &str) -> String {
    let mut result = name.to_string();

    // Count and strip leading %SPACE%
    let mut leading_spaces = 0;
    while result.starts_with(SPACE_ENCODING) {
        leading_spaces += 1;
        result = result[SPACE_ENCODING.len()..].to_string();
    }

    // Count and strip trailing %SPACE%
    let mut trailing_spaces = 0;
    while result.ends_with(SPACE_ENCODING) {
        trailing_spaces += 1;
        result = result[..result.len() - SPACE_ENCODING.len()].to_string();
    }

    // Parse and decode the middle part
    let decoded_middle = decode_patterns(&result);

    // Rebuild with actual spaces
    let prefix = " ".repeat(leading_spaces);
    let suffix = " ".repeat(trailing_spaces);
    format!("{}{}{}", prefix, decoded_middle, suffix)
}

/// Parses an encoded string left-to-right, properly handling `%%` escapes and `%NAME%` patterns.
fn decode_patterns(input: &str) -> String {
    let decode_map = get_decode_map();
    let chars: Vec<char> = input.chars().collect();
    let mut output = String::with_capacity(input.len());
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '%' {
            // Check for %% (escaped percent)
            if i + 1 < chars.len() && chars[i + 1] == '%' {
                output.push('%');
                i += 2;
                continue;
            }

            // Try to match a %NAME% pattern
            if let Some((decoded, consumed)) = try_decode_pattern(&chars, i, decode_map) {
                output.push_str(&decoded);
                i += consumed;
                continue;
            }
        }

        // Regular character, just copy it
        output.push(chars[i]);
        i += 1;
    }

    output
}

/// Tries to decode a %NAME% pattern starting at position `start`.
/// Returns Some((decoded_string, chars_consumed)) if successful, None otherwise.
fn try_decode_pattern(
    chars: &[char],
    start: usize,
    decode_map: &HashMap<&str, &str>,
) -> Option<(String, usize)> {
    // Must start with %
    if chars.get(start) != Some(&'%') {
        return None;
    }

    // Find the closing %
    let mut end = start + 1;
    while end < chars.len() && chars[end] != '%' {
        // Pattern names are uppercase letters only
        if !chars[end].is_ascii_uppercase() {
            return None;
        }
        end += 1;
    }

    // Must have found a closing %
    if end >= chars.len() || chars[end] != '%' {
        return None;
    }

    // Extract the pattern name (without the % delimiters)
    let pattern_name: String = chars[start + 1..end].iter().collect();

    // Look up the pattern
    if let Some(&decoded) = decode_map.get(pattern_name.as_str()) {
        Some((decoded.to_string(), end - start + 1))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_all_chars() {
        let input = r#".<>:"/\|?*"#;
        let expected = "%DOT%%LT%%GT%%COLON%%QUOTE%%SLASH%%BACKSLASH%%PIPE%%QUESTION%%STAR%";
        assert_eq!(encode_path_name(input), expected);
    }

    #[test]
    fn test_decode_all_chars() {
        let input = "%DOT%%LT%%GT%%COLON%%QUOTE%%SLASH%%BACKSLASH%%PIPE%%QUESTION%%STAR%";
        let expected = r#".<>:"/\|?*"#;
        assert_eq!(decode_path_name(input), expected);
    }

    #[test]
    fn test_roundtrip() {
        let original = "Test<File>With:Special\"Chars/And\\More|Stuff?Here*End";
        let encoded = encode_path_name(original);
        let decoded = decode_path_name(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_no_special_chars() {
        let input = "NormalFileName";
        assert_eq!(encode_path_name(input), input);
        assert_eq!(decode_path_name(input), input);
    }

    #[test]
    fn test_leading_spaces() {
        let input = "  LeadingSpaces";
        let encoded = encode_path_name(input);
        assert_eq!(encoded, "%SPACE%%SPACE%LeadingSpaces");
        assert_eq!(decode_path_name(&encoded), input);
    }

    #[test]
    fn test_trailing_spaces() {
        let input = "TrailingSpaces  ";
        let encoded = encode_path_name(input);
        assert_eq!(encoded, "TrailingSpaces%SPACE%%SPACE%");
        assert_eq!(decode_path_name(&encoded), input);
    }

    #[test]
    fn test_both_spaces() {
        let input = " Both Spaces ";
        let encoded = encode_path_name(input);
        assert_eq!(encoded, "%SPACE%Both Spaces%SPACE%");
        assert_eq!(decode_path_name(&encoded), input);
    }

    #[test]
    fn test_middle_spaces_unchanged() {
        let input = "Middle Spaces Here";
        let encoded = encode_path_name(input);
        assert_eq!(encoded, "Middle Spaces Here");
        assert_eq!(decode_path_name(&encoded), input);
    }

    #[test]
    fn test_roundtrip_with_spaces() {
        let original = "  <Test>  ";
        let encoded = encode_path_name(original);
        let decoded = decode_path_name(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_encode_period() {
        let input = "My.Script";
        let expected = "My%DOT%Script";
        assert_eq!(encode_path_name(input), expected);
    }

    #[test]
    fn test_decode_period() {
        let input = "My%DOT%Script";
        let expected = "My.Script";
        assert_eq!(decode_path_name(input), expected);
    }

    #[test]
    fn test_roundtrip_with_period() {
        let original = "My.Module.Name";
        let encoded = encode_path_name(original);
        let decoded = decode_path_name(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_encode_percent() {
        let input = "My%Thing";
        let expected = "My%%Thing";
        assert_eq!(encode_path_name(input), expected);
    }

    #[test]
    fn test_decode_percent() {
        let input = "My%%Thing";
        let expected = "My%Thing";
        assert_eq!(decode_path_name(input), expected);
    }

    #[test]
    fn test_roundtrip_with_percent() {
        let original = "My%Thing";
        let encoded = encode_path_name(original);
        let decoded = decode_path_name(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_percent_encoding_pattern_roundtrip() {
        // This tests that a name containing what looks like an encoding pattern
        // round-trips correctly and doesn't get incorrectly decoded
        let original = "My%DOT%Thing";
        let encoded = encode_path_name(original);
        assert_eq!(encoded, "My%%DOT%%Thing");
        let decoded = decode_path_name(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_combined_period_and_percent() {
        let original = "My.%Thing.Other";
        let encoded = encode_path_name(original);
        let decoded = decode_path_name(&encoded);
        assert_eq!(decoded, original);
    }
}
