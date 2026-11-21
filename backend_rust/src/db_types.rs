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

/// The largest number of decimal places a write may carry.
///
/// A validation rule, stated here, not inferred from a column: `transactions.quantity`,
/// `price` and `fee` are `TEXT` (`0001_init.sql`) and will hold anything, so the storage
/// type cannot be the thing that bounds scale. `rust_decimal` carries 28 digits of
/// precision, so 12 places leaves room for a price like `0.000000000001` while making a
/// value with 200 places — which is what a float round-trip leaves behind — a refusal
/// rather than a stored artefact.
pub const MAX_DECIMAL_SCALE: usize = 12;

/// The largest integer part that survives the round trip intact. `rust_decimal` is 28
/// significant digits; with the scale cap at 12, 16 integer digits is the point past
/// which the two rules would disagree about what fits.
pub const MAX_DECIMAL_PRECISION: usize = 28;

/// True for a plain decimal string: optional `-`, at least one integer digit, and an
/// optional fractional part of at most [`MAX_DECIMAL_SCALE`] digits.
///
/// Written by hand rather than with a regex crate, and deliberately narrower than
/// `Decimal::from_str`, which is the *storage* parser and accepts exponent notation,
/// leading `+`, and whitespace. Accepting those on the wire would mean a value can enter
/// the system in a form nothing else writes, and the dedup key built from the stored
/// string would treat `1e3` and `1000` as different transactions.
///
/// Rejects: `1e3`, `1E3`, `+10`, `" 10"`, `10.`, `.5`, `1,50`, `1.2345678901234` (13 dp),
/// `NaN`, `inf`, and the empty string.
pub fn is_canonical_decimal(s: &str) -> bool {
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    let int_digits = int_part.strip_prefix('-').unwrap_or(int_part);
    // A lone `-`, an empty integer part, and a non-digit anywhere (which is what catches
    // `1e3`, `1_000`, ` 10` and `NaN` without a separate rule each) are all refused here.
    if int_digits.is_empty() || !int_digits.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if let Some(f) = frac_part {
        if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        if f.len() > MAX_DECIMAL_SCALE {
            return false;
        }
    }
    if int_digits.len() + frac_part.map_or(0, str::len) > MAX_DECIMAL_PRECISION {
        return false;
    }
    true
}

/// The **write-side** decimal codec: a JSON *string* or nothing.
///
/// This is not `decimal_json` and `decimal_json` must not become this. That module is the
/// response serializer at roughly 55 sites in `schemas.rs`, and its `visit_f64` arm is
/// load-bearing for `TaxLot` and `AssetTaxSummary` round-trips — tightening it would turn
/// every tax figure the app has ever answered into a deserialization failure. This module
/// is the opposite direction: what a client may send.
///
/// Why it exists: `transactions.quantity`, `price` and `fee` are stored as
/// `Decimal::to_string()`, which is exact — but a value that arrived as a JSON *number*
/// has already passed through `f64` before it became a `Decimal`. `0.1 + 0.2` is not
/// `0.3`, and a cost basis that drifted by a rounding unit per row is wrong on tax
/// figures. The only way to stop that at a boundary is to refuse the boundary, so a number
/// is a 422 naming the fix rather than a silent float.
///
/// `deserialize_str` rather than `deserialize_any` is the mechanism: with it, serde answers
/// a JSON number with `invalid type: integer `10`, expected a decimal string ...` on its
/// own, so there is no `visit_f64` here to forget to remove.
///
/// **Deploy order.** The browser's CSV importer posted JSON numbers until
/// `refactor/frontend-decimal-strings`. That must ship and deploy first; this change is
/// what makes it a breaking one.
pub mod strict_decimal_json {
    use rust_decimal::Decimal;
    use serde::{Deserializer, Serializer};

    use super::{is_canonical_decimal, MAX_DECIMAL_SCALE};

    pub fn serialize<S>(d: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Same spelling as the response codec, so a client that reads what it sent sees
        // the same text. Writes are strings in and strings out; only the *accepting* side
        // is stricter.
        serializer.serialize_str(&d.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrictDecimal;

        impl<'de> serde::de::Visitor<'de> for StrictDecimal {
            type Value = Decimal;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(
                    formatter,
                    "a decimal string such as '10' or '10.50' (JSON numbers are refused because they pass through a float; no exponent; at most {MAX_DECIMAL_SCALE} decimal places)"
                )
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Decimal, E> {
                if !is_canonical_decimal(v) {
                    return Err(E::custom(format!(
                        "{v:?} is not an accepted decimal string: send a plain decimal in quotes, not a JSON number, with no exponent, no thousands separator, and at most {MAX_DECIMAL_SCALE} decimal places"
                    )));
                }
                super::parse_decimal(v)
                    .map_err(|e| E::custom(format!("{v:?} could not be parsed as a decimal: {e}")))
            }
        }

        deserializer.deserialize_str(StrictDecimal)
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
