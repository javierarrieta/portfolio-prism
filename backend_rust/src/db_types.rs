use rust_decimal::Decimal;
use std::str::FromStr;

pub fn parse_decimal(s: &str) -> Result<Decimal, rust_decimal::Error> {
    Decimal::from_str(s)
}

pub fn str_to_decimal(s: &str) -> Decimal {
    match parse_decimal(s) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("WARN: invalid decimal string '{s}': {e}");
            Decimal::ZERO
        }
    }
}

pub fn decimal_to_str(d: &Decimal) -> String {
    d.to_string()
}

pub mod decimal_json {
    use rust_decimal::Decimal;
    use rust_decimal::prelude::FromPrimitive;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(d: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&d.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DecimalVisitor;

        impl<'de> serde::de::Visitor<'de> for DecimalVisitor {
            type Value = Decimal;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string or number")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Decimal, E> {
                super::parse_decimal(v).map_err(|_| E::custom(format!("invalid decimal: {v}")))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Decimal, E> {
                Decimal::from_i64(v).ok_or_else(|| E::custom("i64 does not fit decimal"))
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Decimal, E> {
                Decimal::from_u64(v).ok_or_else(|| E::custom("u64 does not fit decimal"))
            }

            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Decimal, E> {
                // Lenient fallback for legacy numeric clients; rounds to 28-digit precision.
                Decimal::from_f64(v).ok_or_else(|| E::custom("f64 does not fit decimal"))
            }
        }

        deserializer.deserialize_any(DecimalVisitor)
    }
}

/// `decimal_json` lifted to `Option<Decimal>`: `None` serializes as JSON null,
/// `Some(d)` as the exact `d.to_string()` — no `normalize()`, no fixed scale.
/// The `Some` arm delegates to `decimal_json::deserialize`, so there is one
/// number/string parsing rule in this file, not two.
pub mod decimal_opt_json {
    use rust_decimal::Decimal;
    use serde::de::{Error, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;

    pub fn serialize<S>(d: &Option<Decimal>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match d {
            Some(v) => serializer.serialize_str(&v.to_string()),
            None => serializer.serialize_none(),
        }
    }

    struct OptionDecimalVisitor;

    impl<'de> Visitor<'de> for OptionDecimalVisitor {
        type Value = Option<Decimal>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("null, a string, or a number")
        }

        fn visit_unit<E: Error>(self) -> Result<Option<Decimal>, E> {
            Ok(None)
        }

        fn visit_none<E: Error>(self) -> Result<Option<Decimal>, E> {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Option<Decimal>, D::Error>
        where
            D: Deserializer<'de>,
        {
            super::decimal_json::deserialize(deserializer).map(Some)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_option(OptionDecimalVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Wrap { #[serde(with = "decimal_json")] d: Decimal }

    #[test]
    fn serialize_is_exact_string() {
        let w = Wrap { d: Decimal::from_str("9.99").unwrap() };
        assert_eq!(serde_json::to_string(&w).unwrap(), r#"{"d":"9.99"}"#);
    }

    #[test]
    fn integer_input_is_exact() {
        let w: Wrap = serde_json::from_str(r#"{"d":100}"#).unwrap();
        assert_eq!(w.d, Decimal::from_str("100").unwrap());
        let w: Wrap = serde_json::from_str(r#"{"d":-7}"#).unwrap();
        assert_eq!(w.d, Decimal::from_str("-7").unwrap());
    }

    #[test]
    fn string_input_is_exact() {
        let w: Wrap = serde_json::from_str(r#"{"d":"9.99"}"#).unwrap();
        assert_eq!(w.d, Decimal::from_str("9.99").unwrap());
    }

    #[test]
    fn numeric_f64_input_round_trips_cleanly() {
        let w: Wrap = serde_json::from_str(r#"{"d":160.32}"#).unwrap();
        assert_eq!(w.d.to_string(), "160.32");
        let w: Wrap = serde_json::from_str(r#"{"d":9.99}"#).unwrap();
        assert_eq!(w.d.to_string(), "9.99");
    }

    #[test]
    fn fractional_sum_is_exact() {
        let sum = Decimal::from_str("0.1").unwrap() + Decimal::from_str("0.2").unwrap();
        assert_eq!(sum, Decimal::from_str("0.3").unwrap());
        assert_eq!(serde_json::to_string(&Wrap { d: sum }).unwrap(), r#"{"d":"0.3"}"#);
    }

    #[test]
    fn invalid_string_input_errors() {
        let res = serde_json::from_str::<Wrap>(r#"{"d":"abc"}"#);
        assert!(res.is_err());
    }

    #[test]
    fn str_to_decimal_returns_zero_with_warning_on_bad_input() {
        assert_eq!(str_to_decimal("not-a-number"), Decimal::ZERO);
    }

    #[test]
    fn invalid_type_triggers_expecting() {
        let err = serde_json::from_str::<Wrap>(r#"{"d":true}"#).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("a string or number"), "unexpected error: {msg}");
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct WrapOpt { #[serde(with = "decimal_opt_json")] d: Option<Decimal> }

    #[test]
    fn optional_decimal_serializes_some_as_string() {
        let w = WrapOpt { d: Some(Decimal::from_str("9.99").unwrap()) };
        assert_eq!(serde_json::to_string(&w).unwrap(), r#"{"d":"9.99"}"#);
    }

    #[test]
    fn optional_decimal_serializes_none_as_null() {
        let w = WrapOpt { d: None };
        assert_eq!(serde_json::to_string(&w).unwrap(), r#"{"d":null}"#);
    }

    #[test]
    fn optional_decimal_deserializes_null_number_and_string() {
        assert_eq!(serde_json::from_str::<WrapOpt>(r#"{"d":null}"#).unwrap().d, None);
        assert_eq!(
            serde_json::from_str::<WrapOpt>(r#"{"d":"9.99"}"#).unwrap().d,
            Some(Decimal::from_str("9.99").unwrap())
        );
        assert_eq!(
            serde_json::from_str::<WrapOpt>(r#"{"d":100}"#).unwrap().d,
            Some(Decimal::from_str("100").unwrap())
        );
    }

    #[test]
    fn optional_decimal_rejects_garbage_string() {
        assert!(serde_json::from_str::<WrapOpt>(r#"{"d":"abc"}"#).is_err());
    }
}
