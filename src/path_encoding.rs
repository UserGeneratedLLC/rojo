//! Encoding and decoding utilities for Windows-invalid filename characters.
//!
//! Windows does not allow the following characters in file names:
//! < > : " / \ | ? *
//!
//! Windows also does not allow leading/trailing spaces in file names.
//!
//! This module provides functions to encode these characters using a %NAME%
//! format (e.g., `<` becomes `%LT%`) and decode them back.

/// Mapping of Windows-invalid characters to their encoded representations.
const CHAR_ENCODINGS: &[(&str, &str)] = &[
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

const SPACE_ENCODING: &str = "%SPACE%";

/// Encodes Windows-invalid characters in a file name.
///
/// This replaces characters like `<`, `>`, `:`, `"`, `/`, `\`, `|`, `?`, `*`
/// with their encoded representations like `%LT%`, `%GT%`, etc.
/// Leading and trailing spaces are encoded as `%SPACE%`.
pub fn encode_path_name(name: &str) -> String {
    // Count leading and trailing spaces first (before any modifications)
    let leading_spaces = name.len() - name.trim_start_matches(' ').len();
    let trailing_spaces = name.len() - name.trim_end_matches(' ').len();

    // Get the middle part (without leading/trailing spaces)
    let middle = &name[leading_spaces..name.len() - trailing_spaces];

    // Encode invalid characters in the middle part
    let mut encoded_middle = middle.to_string();
    for (char, encoded) in CHAR_ENCODINGS {
        encoded_middle = encoded_middle.replace(char, encoded);
    }

    // Build final result with encoded spaces
    let encoded_prefix = SPACE_ENCODING.repeat(leading_spaces);
    let encoded_suffix = SPACE_ENCODING.repeat(trailing_spaces);

    format!("{}{}{}", encoded_prefix, encoded_middle, encoded_suffix)
}

/// Decodes encoded Windows-invalid characters back to their original form.
///
/// This replaces encoded representations like `%LT%`, `%GT%`, etc. back
/// to their original characters `<`, `>`, etc.
/// `%SPACE%` at the start/end is decoded back to spaces.
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

    // Decode invalid characters in the middle
    for (char, encoded) in CHAR_ENCODINGS {
        result = result.replace(encoded, char);
    }

    // Rebuild with actual spaces
    let prefix = " ".repeat(leading_spaces);
    let suffix = " ".repeat(trailing_spaces);
    format!("{}{}{}", prefix, result, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_all_chars() {
        let input = r#"<>:"/\|?*"#;
        let expected = "%LT%%GT%%COLON%%QUOTE%%SLASH%%BACKSLASH%%PIPE%%QUESTION%%STAR%";
        assert_eq!(encode_path_name(input), expected);
    }

    #[test]
    fn test_decode_all_chars() {
        let input = "%LT%%GT%%COLON%%QUOTE%%SLASH%%BACKSLASH%%PIPE%%QUESTION%%STAR%";
        let expected = r#"<>:"/\|?*"#;
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
}
