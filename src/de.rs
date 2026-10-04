use serde::{de, Deserialize, Deserializer};
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
    enum OptionNum<T> {
        Num(T),
        Str(String),
        None,
    }

    match OptionNum::<T>::deserialize(deserializer)? {
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
}
