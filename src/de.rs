use serde::{Deserialize, Deserializer, de};
use std::borrow::Cow;
use std::fmt::Display;
use std::str::FromStr;

/// Deserializes an empty string or whitespace-only string into `None`,
/// or parses a non-empty string / number into `Some(T)`.
pub fn deserialize_option_number<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + Deserialize<'de>,
    T::Err: Display,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OptionNum<'a, T> {
        Num(T),
        Str(Cow<'a, str>),
        None,
    }

    match OptionNum::<'de, T>::deserialize(deserializer)? {
        OptionNum::Num(n) => Ok(Some(n)),
        OptionNum::Str(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                trimmed.parse::<T>().map(Some).map_err(de::Error::custom)
            }
        }
        OptionNum::None => Ok(None),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestStruct {
        #[serde(default, deserialize_with = "deserialize_option_number")]
        val_i64: Option<i64>,
        #[serde(default, deserialize_with = "deserialize_option_number")]
        val_f32: Option<f32>,
        #[serde(default, deserialize_with = "deserialize_option_number")]
        val_i16: Option<i16>,
    }

    #[test]
    fn test_deserialize_option_number_urlencoded() {
        // Test empty strings as submitted by urlencoded forms
        let urlencoded_str = "val_i64=&val_f32=%20%20&val_i16=42";
        let parsed: TestStruct = serde_urlencoded::from_str(urlencoded_str).unwrap();
        assert_eq!(
            parsed,
            TestStruct {
                val_i64: None,
                val_f32: None,
                val_i16: Some(42),
            }
        );

        // Test non-empty values
        let urlencoded_str2 = "val_i64=123&val_f32=0.95&val_i16=-10";
        let parsed2: TestStruct = serde_urlencoded::from_str(urlencoded_str2).unwrap();
        assert_eq!(
            parsed2,
            TestStruct {
                val_i64: Some(123),
                val_f32: Some(0.95),
                val_i16: Some(-10),
            }
        );
    }

    #[test]
    fn test_deserialize_option_number_json() {
        let json_str = r#"{"val_i64": 100, "val_f32": null, "val_i16": ""}"#;
        let parsed: TestStruct = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            parsed,
            TestStruct {
                val_i64: Some(100),
                val_f32: None,
                val_i16: None,
            }
        );
    }

    #[test]
    fn test_deserialize_option_number_invalid_and_boundary_inputs() {
        // Arrange & Act: Test whitespace-padded numeric strings in urlencoded & JSON
        let urlencoded_whitespace = "val_i64=%20123%20&val_f32=%0A3.5%0A&val_i16=%20-99%20";
        let parsed_ws: TestStruct = serde_urlencoded::from_str(urlencoded_whitespace).unwrap();

        // Assert: Trimmed whitespace parses successfully
        assert_eq!(
            parsed_ws,
            TestStruct {
                val_i64: Some(123),
                val_f32: Some(3.5),
                val_i16: Some(-99),
            }
        );

        let json_whitespace = r#"{"val_i64": "  500  ", "val_f32": " -0.5 ", "val_i16": "\t42\n"}"#;
        let parsed_json_ws: TestStruct = serde_json::from_str(json_whitespace).unwrap();
        assert_eq!(
            parsed_json_ws,
            TestStruct {
                val_i64: Some(500),
                val_f32: Some(-0.5),
                val_i16: Some(42),
            }
        );

        // Arrange & Act: Test invalid non-numeric strings produce deserialization errors
        let urlencoded_invalid = "val_i16=not_a_number";
        let err_urlencoded = serde_urlencoded::from_str::<TestStruct>(urlencoded_invalid);
        assert!(
            err_urlencoded.is_err(),
            "Expected non-numeric urlencoded string to fail deserialization"
        );

        let json_invalid = r#"{"val_i64": "12.34"}"#; // Float string into integer field
        let err_json = serde_json::from_str::<TestStruct>(json_invalid);
        assert!(
            err_json.is_err(),
            "Expected float string in i64 field to fail deserialization"
        );

        // Arrange & Act: Test integer overflow/underflow strings produce deserialization errors
        let json_overflow = r#"{"val_i16": "99999999999"}"#; // Exceeds i16::MAX (32767)
        let err_overflow = serde_json::from_str::<TestStruct>(json_overflow);
        assert!(
            err_overflow.is_err(),
            "Expected overflowing string in i16 field to fail deserialization"
        );
    }
}
