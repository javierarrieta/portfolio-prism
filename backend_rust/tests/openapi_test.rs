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

/// Every `#/components/<section>/<name>` reference anywhere in the document.
///
/// Walked as text rather than through a typed model on purpose: a `$ref` can appear in a
/// request body, a response, a query parameter's schema, or nested inside an `allOf`, and a
/// walker that only visits the paths it knows about would miss the rest.
///
/// The section is kept rather than discarded, because `#/components/` is not only schemas —
/// utoipa also emits `responses`, `parameters` and `securitySchemes`. A checker that looked
/// under `schemas` alone would call a dangling `#/components/responses/X` clean, which is
/// this file's own bug one key over. Today the document contains schema references only; the
/// walker does not depend on that staying true.
fn referenced(document: &Value) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut stack = vec![document];
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    if key == "$ref" {
                        if let Some(rest) = child
                            .as_str()
                            .and_then(|s| s.strip_prefix("#/components/"))
                        {
                            let mut parts = rest.splitn(2, '/');
                            if let (Some(section), Some(name)) = (parts.next(), parts.next()) {
                                found.push((section.to_string(), name.to_string()));
                            }
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
fn no_reference_points_at_a_component_that_was_never_registered() {
    let document = document();
    let components = document["components"]
        .as_object()
        .expect("the document has a components object");

    let references = referenced(&document);
    // A walker that matched nothing would pass the assertion below vacuously, so prove it is
    // reading the document at all before believing its silence.
    assert!(
        !references.is_empty(),
        "the walker found no `#/components/...` reference anywhere in the document, which \n\
         means the walker is broken rather than that the document is clean"
    );
    let dangling: Vec<&(String, String)> = references
        .iter()
        .filter(|(section, name)| {
            !components
                .get(section)
                .and_then(|set| set.as_object())
                .is_some_and(|set| set.contains_key(name))
        })
        .collect();

    assert!(
        dangling.is_empty(),
        "these components are referenced but never registered in openapi.rs's components \
         list: {dangling:?}. Add each to `components(schemas(...))` or to the section it \
         belongs to."
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
