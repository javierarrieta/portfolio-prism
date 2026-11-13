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
    let value = serde_json::to_value(document()).expect("a schemars document is JSON");
    // Key order is fixed by `preserve_order`, so the committed file is stable and
    // the CI diff is meaningful.
    serde_json::to_string_pretty(&value).expect("a serde_json value serialises")
}
