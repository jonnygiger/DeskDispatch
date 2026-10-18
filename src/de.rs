use serde::de::{self, Deserializer, IntoDeserializer, Visitor};
use std::fmt::{self, Display};
use std::marker::PhantomData;
use std::str::FromStr;

struct OptionNumberVisitor<T>(PhantomData<T>);

impl<'de, T> Visitor<'de> for OptionNumberVisitor<T>
where
    T: FromStr + de::Deserialize<'de>,
    T::Err: Display,
{
    type Value = Option<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a number, a numeric string, an empty string, or null")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(None)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(None)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            trimmed.parse::<T>().map(Some).map_err(de::Error::custom)
        }
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        T::deserialize(v.into_deserializer()).map(Some)
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        T::deserialize(v.into_deserializer()).map(Some)
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        T::deserialize(v.into_deserializer()).map(Some)
    }
}

/// Deserializes an empty string or whitespace-only string into `None`,
/// or parses a non-empty string / number into `Some(T)`.
pub fn deserialize_option_number<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + de::Deserialize<'de>,
    T::Err: Display,
{
    deserializer.deserialize_any(OptionNumberVisitor(PhantomData))
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
        // Test empty strings as submitted by forms
        let json_str = r#"{"val_i64": "", "val_f32": "  ", "val_i16": "42"}"#;
        let parsed: TestStruct = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            parsed,
            TestStruct {
                val_i64: None,
                val_f32: None,
                val_i16: Some(42),
            }
        );

        // Test non-empty values
        let json_str2 = r#"{"val_i64": "123", "val_f32": "0.95", "val_i16": "-10"}"#;
        let parsed2: TestStruct = serde_json::from_str(json_str2).unwrap();
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
        // Arrange & Act: Test whitespace-padded numeric strings in JSON
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
        let json_invalid_str = r#"{"val_i16": "not_a_number"}"#;
        let err_json_str = serde_json::from_str::<TestStruct>(json_invalid_str);
        assert!(
            err_json_str.is_err(),
            "Expected non-numeric string to fail deserialization"
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
