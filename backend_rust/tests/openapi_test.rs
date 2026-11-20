//! Schema integrity of the published document.
//!
//! `AssetUpdate` was referenced by `PATCH /api/portfolios/<id>/assets/<id>` and defined by
//! nothing: `utoipa` emits a `$ref` for whatever type a handler names, and registers a
//! definition only for types listed in `openapi.rs`'s `components(schemas(...))`. The two
//! lists are maintained by hand and nothing checked that they agreed, so the document
//! carried a pointer into a components object with no such key.
//!
//! A client that resolves references — a generated SDK, or `openapi-typescript` — fails on
//! that. A client that ignores them renders the request body as an empty object, which is
//! worse: it looks like the endpoint takes nothing.
//!
//! The one-off fix is one line in `openapi.rs`. This file is the part that keeps it fixed,
//! because the same omission on any future type is invisible until someone's codegen breaks.

use serde_json::Value;

fn document() -> Value {
    // Parsed from the serialized form rather than the typed `OpenApi` struct, so the test
    // inspects exactly what a client receives at `/api-docs/openapi.json` — the same bytes
    // `docs/openapi/openapi.json` is generated from.
    serde_json::from_str(&backend_rust::openapi::document_json())
        .expect("the document serialises to valid JSON")
}

/// Every `#/components/schemas/X` string anywhere in the document.
///
/// Walked as text rather than through a typed model on purpose: a `$ref` can appear in a
/// request body, a response, a query parameter's schema, or nested inside an `allOf`, and a
/// walker that only visits the paths it knows about would miss the rest.
fn referenced(document: &Value) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![document];
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    if key == "$ref" {
                        if let Some(name) = child
                            .as_str()
                            .and_then(|s| s.strip_prefix("#/components/schemas/"))
                        {
                            found.push(name.to_string());
                        }
                    } else {
                        stack.push(child);
                    }
                }
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    found.sort();
    found.dedup();
    found
}

#[test]
fn no_reference_points_at_a_schema_that_was_never_registered() {
    let document = document();
    let defined = document["components"]["schemas"]
        .as_object()
        .expect("the document has a components.schemas object");

    let references = referenced(&document);
    let dangling: Vec<&String> = references
        .iter()
        .filter(|name| !defined.contains_key(name.as_str()))
        .collect();

    assert!(
        dangling.is_empty(),
        "these schemas are referenced but never registered in openapi.rs's components list: \
         {dangling:?}. Add them to `components(schemas(...))`."
    );
}

#[test]
fn asset_update_resolves() {
    // The specific regression. Named separately from the general check so that a future
    // failure says which type broke, instead of listing everything that is dangling.
    let document = document();
    let schemas = document["components"]["schemas"]
        .as_object()
        .expect("components.schemas");
    assert!(
        schemas.contains_key("AssetUpdate"),
        "AssetUpdate must be registered: PATCH assets names it as its request body"
    );
    let asset_update = &schemas["AssetUpdate"];
    assert_eq!(
        asset_update["type"],
        serde_json::json!("object"),
        "AssetUpdate should resolve to an object schema, got {asset_update}"
    );
}
