//! Generation of the committed JSON Schema artifact.
//!
//! `docs/backup-schema/v1.json` is produced by this code, not written by hand, so
//! the artifact and the structs cannot drift. CI regenerates it and diffs it, the
//! same way it does `docs/openapi/openapi.json`.

use crate::backup::Bundle;
use schemars::generate::SchemaSettings;

/// The draft-2020-12 schema for the current `Bundle`.
pub fn document() -> schemars::Schema {
    SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<Bundle>()
}

/// Pretty-printed JSON for `--print-backup-schema`.
pub fn document_json() -> String {
    // Key order comes from `serde_json`'s sorted map, so the committed file is
    // byte-stable and the CI diff is meaningful without any global feature flag.
    let value = serde_json::to_value(document()).expect("a schemars document is JSON");
    serde_json::to_string_pretty(&value).expect("a serde_json value serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_schema_is_byte_identical_to_this_builds_output() {
        // `--print-backup-schema` uses `println!`, so the committed file carries a
        // trailing newline the serialiser does not produce. This is the drift
        // guard the CI diff in Task 10 will depend on.
        assert_eq!(
            include_str!("../../../docs/backup-schema/v1.json"),
            format!("{}\n", document_json()),
            "docs/backup-schema/v1.json has drifted from the `Bundle` structs; regenerate it \
             with `cargo run --bin backend_rust -- --print-backup-schema > \
             ../docs/backup-schema/v1.json`"
        );
    }

    /// The two fields the spec says the schema enumerates must carry an `enum`, and
    /// carry **the same set `collect()` validates against**. That shared source is the
    /// only automated link between the artifact and the rules: field names and
    /// nullability come from the derive, these two come from the constants, and
    /// everything else `collect()` enforces is prose in `docs/backup-schema/README.md`.
    #[test]
    fn the_artifact_enumerates_the_two_type_fields_from_the_validating_constants() {
        let doc: serde_json::Value =
            serde_json::from_str(&document_json()).expect("document_json is JSON");
        for (def, field, known) in [
            ("BundleAsset", "asset_type", crate::backup::KNOWN_ASSET_TYPES),
            ("BundleTransaction", "type", crate::backup::KNOWN_TRANSACTION_TYPES),
        ] {
            let prop = &doc["$defs"][def]["properties"][field];
            assert_eq!(
                prop["type"],
                serde_json::json!("string"),
                "{def}.{field} must stay a string in the artifact"
            );
            assert_eq!(
                prop["enum"],
                serde_json::json!(known),
                "{def}.{field} must enumerate {known:?}, the set `collect()` enforces"
            );
        }
    }

    #[test]
    fn the_schema_pins_the_root_required_set_and_every_nested_definition() {
        let doc: serde_json::Value =
            serde_json::from_str(&document_json()).expect("document_json is JSON");
        assert_eq!(
            doc["required"],
            serde_json::json!(["format", "schema_version", "created_at", "portfolios"]),
            "the root `required` set is part of v1's contract"
        );
        for def in ["BundlePortfolio", "BundleAsset", "BundleTransaction", "BundlePrice"] {
            assert!(doc["$defs"].get(def).is_some(), "v1.json must define {def}");
        }
    }
}
