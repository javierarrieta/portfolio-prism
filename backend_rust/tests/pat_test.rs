//! Personal access tokens: mint, list, revoke, and — the part that matters — what
//! makes a token stop being a credential.
//!
//! The shape of these tests follows the claim being made. A token is a credential a
//! program holds, so the interesting properties are all about its death: revocation
//! taking effect on the next call, expiry being enforced rather than advisory, and a
//! deactivated user losing tokens without this module having to remember that tokens
//! exist. A mint that works is the least interesting thing here.
//!
//! The route half is here too, because "a token cannot mint a token" is a property of
//! which routes are session-only, and that is only visible through the HTTP surface.

mod common;

use chrono::{Duration, Utc};
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client;
use rocket::serde::json::Json;

use backend_rust::access::{require_own_token, require_scope, Access};
use backend_rust::auth::guard::{AuthedUser, Credential};
use backend_rust::auth::pat::{self, PatError, Scope};
use backend_rust::db::DbPool;

use common::{build_rocket, db_with_user, seed_user};

async fn mint_read(pool: &DbPool, user: i64, label: &str) -> String {
    pat::mint(pool, user, label, &["read".to_string()], None)
        .await
        .expect("mint")
        .secret
}

fn scopes_of(user: &AuthedUser) -> Vec<String> {
    match &user.credential {
        Credential::Token { scopes, .. } => scopes.iter().map(|s| s.as_str().to_string()).collect(),
        Credential::Session => vec!["session".to_string()],
    }
}

#[tokio::test]
async fn a_minted_token_authenticates_and_reports_its_scopes() {
    let (pool, user) = db_with_user("mint_and_lookup").await;
    let minted = pat::mint(&pool, user, "laptop", &["read".into(), "import".into()], None)
        .await
        .unwrap();

    assert!(minted.secret.starts_with("pp_"), "secret: {}", minted.secret);
    // 43 base62 characters after the prefix.
    assert_eq!(minted.secret.len(), 46, "secret: {}", minted.secret);
    assert!(
        minted
            .secret
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "secret must be transportable in a header verbatim: {}",
        minted.secret
    );

    let found = pat::lookup(&pool, &minted.secret).await.unwrap().unwrap();
    assert_eq!(found.id, user);
    assert_eq!(scopes_of(&found), vec!["read", "import"]);
}

/// The claim is not "we are careful with the secret" but "there is nothing to leak".
/// The stored value is a hash, and the list query does not select it at all.
#[tokio::test]
async fn the_stored_value_is_a_hash_and_the_list_never_sees_it() {
    let (pool, user) = db_with_user("hash_not_secret").await;
    let minted = pat::mint(&pool, user, "laptop", &["read".into()], None)
        .await
        .unwrap();

    let (stored,): (String,) = sqlx::query_as("SELECT secret_hash FROM api_token WHERE id = $1")
        .bind(minted.id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_ne!(stored, minted.secret);
    assert_eq!(stored.len(), 64, "SHA-256 as hex is 64 chars");
    assert!(
        stored.chars().all(|c| c.is_ascii_hexdigit()),
        "stored hash should be lowercase hex: {stored}"
    );

    let listed = pat::list(&pool, user).await.unwrap();
    assert_eq!(listed.len(), 1);
    // `TokenInfo` has no field that could hold a secret; this asserts the list is
    // populated from metadata, and the type is what makes the rest unconstructible.
    assert_eq!(listed[0].label, "laptop");
    assert_eq!(listed[0].id, minted.id);
}

/// Revocation is live, not cached: every request reads the row, so the next call after
/// a revoke is already refused. This is the answer to a leaked token, so it must not
/// be eventual.
#[tokio::test]
async fn a_revoked_token_is_refused_on_the_next_call() {
    let (pool, user) = db_with_user("revoke").await;
    let secret = mint_read(&pool, user, "leaked").await;

    assert!(pat::lookup(&pool, &secret).await.unwrap().is_some());
    let id = pat::list(&pool, user).await.unwrap()[0].id;
    assert!(pat::revoke(&pool, user, id).await.unwrap());
    assert!(
        pat::lookup(&pool, &secret).await.unwrap().is_none(),
        "a revoked token must stop working immediately, not on a cache expiry"
    );

    // Revoking again is a no-op, not an error and not a second write.
    let id = pat::list(&pool, user).await.unwrap()[0].id;
    assert!(!pat::revoke(&pool, user, id).await.unwrap());
}

#[tokio::test]
async fn an_expired_token_is_refused_and_an_unexpired_one_is_not() {
    let (pool, user) = db_with_user("expiry").await;

    let dead = pat::mint(
        &pool,
        user,
        "yesterday",
        &["read".into()],
        Some(Utc::now() - Duration::days(1)),
    )
    .await
    .unwrap();
    assert!(
        pat::lookup(&pool, &dead.secret).await.unwrap().is_none(),
        "expiry has to be enforced at lookup, or it is decoration"
    );

    let alive = pat::mint(
        &pool,
        user,
        "tomorrow",
        &["read".into()],
        Some(Utc::now() + Duration::days(1)),
    )
    .await
    .unwrap();
    assert!(pat::lookup(&pool, &alive.secret).await.unwrap().is_some());
}

/// The spec's "for free" claim, made testable. A token resolves to a `user_id` and
/// then through the same `user_by_id` the session guard uses, so deactivating an
/// account kills its tokens without `pat` knowing anything about deactivation. If
/// someone later adds a second user lookup on the token path, this is what catches
/// the divergence.
#[tokio::test]
async fn a_deactivated_user_loses_its_tokens_without_anything_in_pat_knowing() {
    let (pool, user) = db_with_user("deactivated").await;
    let secret = mint_read(&pool, user, "laptop").await;
    assert!(pat::lookup(&pool, &secret).await.unwrap().is_some());

    sqlx::query!("UPDATE users SET is_active = false WHERE id = $1", user)
        .execute(&pool)
        .await
        .unwrap();

    assert!(
        pat::lookup(&pool, &secret).await.unwrap().is_none(),
        "a deactivated account must not authenticate through a token"
    );
}

#[tokio::test]
async fn a_token_is_not_a_credential_for_someone_else() {
    let (pool, alice) = db_with_user("cross_user_token").await;
    let bob = seed_user(&pool, "bob").await;

    let alice_secret = mint_read(&pool, alice, "alice").await;
    let alice_id = pat::list(&pool, alice).await.unwrap()[0].id;

    // Bob can present Alice's secret and become Alice — that is what a bearer token
    // is, and revocation is the only answer. What he cannot do is *manage* it.
    assert!(pat::lookup(&pool, &alice_secret).await.unwrap().is_some());
    assert_eq!(
        require_own_token(&pool, bob, alice_id, Access::Owner)
            .await
            .unwrap_err(),
        Status::NotFound,
        "someone else's token id must not be distinguishable from a missing one"
    );
    assert!(!pat::revoke(&pool, bob, alice_id).await.unwrap());

    // Alice can.
    assert!(require_own_token(&pool, alice, alice_id, Access::Owner)
        .await
        .is_ok());
}

/// The scope rule in one place: a session is never scope-limited, a token is exactly
/// as wide as its scopes, and a missing scope is a refusal rather than a silent no-op.
#[test]
fn a_scope_is_a_ceiling_on_a_token_and_nothing_at_all_for_a_session() {
    let session = AuthedUser {
        id: 1,
        email: None,
        name: None,
        sub: "s".into(),
        token_version: 0,
        credential: Credential::Session,
    };
    assert!(require_scope(&session, Scope::Import).is_ok());
    assert!(require_scope(&session, Scope::Read).is_ok());

    let read_only = AuthedUser {
        credential: Credential::Token {
            id: 7,
            scopes: vec![Scope::Read],
        },
        ..session.clone()
    };
    assert!(require_scope(&read_only, Scope::Read).is_ok());
    assert_eq!(
        require_scope(&read_only, Scope::Import).unwrap_err(),
        Status::Forbidden,
        "a read-only token must not be able to stage an import"
    );
}

#[tokio::test]
async fn mint_refuses_input_it_cannot_honour() {
    let (pool, user) = db_with_user("mint_validation").await;

    assert!(matches!(
        pat::mint(&pool, user, "   ", &["read".into()], None).await,
        Err(PatError::InvalidInput)
    ));
    assert!(matches!(
        pat::mint(&pool, user, "ok", &[], None).await,
        Err(PatError::InvalidInput)
    ));
    // An unknown scope is refused, not quietly dropped: a token that turns out to do
    // nothing is a worse outcome than a request that says what the names are.
    assert!(matches!(
        pat::mint(&pool, user, "ok", &["admin".into()], None).await,
        Err(PatError::InvalidInput)
    ));
    // Spelling is not the contract; the vocabulary is.
    assert!(pat::mint(&pool, user, "ok", &[" Read ".into()], None)
        .await
        .is_ok());
    // A label is a table cell, not a document. `mint` is reachable with nothing but a
    // session and a body, and Rocket's default `json` limit is 1 MiB, so an unbounded label
    // is a 1 MiB row rendered in a list.
    assert!(matches!(
        pat::mint(&pool, user, &"x".repeat(121), &["read".into()], None).await,
        Err(PatError::InvalidInput)
    ));
    assert!(pat::mint(&pool, user, &"x".repeat(120), &["read".into()], None)
        .await
        .is_ok());
}

#[tokio::test]
async fn a_scope_listed_twice_is_one_grant_not_two() {
    // `["read","read"]` used to store two entries, which made the length of
    // `TokenInfo::scopes` mean nothing and rendered the same chip twice in the Access
    // panel. Deduplicating keeps the stored array a set, which is what a scope is.
    let (pool, user) = db_with_user("mint_dedupe").await;
    let minted = pat::mint(&pool, user, "dup", &["read".into(), "read".into()], None)
        .await
        .unwrap();
    assert_eq!(minted.scopes, vec![Scope::Read]);

    let listed = pat::list(&pool, user).await.unwrap();
    let row = listed.iter().find(|t| t.id == minted.id).unwrap();
    assert_eq!(row.scopes, vec![Scope::Read], "the list shows one grant");

    // Order survives, so `["import","read"]` still reads as it was written.
    let both = pat::mint(&pool, user, "both", &["import".into(), "read".into(), "import".into()], None)
        .await
        .unwrap();
    assert_eq!(both.scopes, vec![Scope::Import, Scope::Read]);
}

/// `last_used_at` is what the 90-day nudge reads, so it has to move — but not on
/// every call, or the credential that exists to be cheap becomes a write per request.
#[tokio::test]
async fn last_used_moves_once_and_then_holds_still_for_a_minute() {
    let (pool, user) = db_with_user("last_used").await;
    let secret = mint_read(&pool, user, "laptop").await;

    assert!(pat::lookup(&pool, &secret).await.unwrap().is_some());
    let first: (Option<chrono::DateTime<Utc>>,) =
        sqlx::query_as("SELECT last_used_at FROM api_token")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(first.0.is_some(), "the first use must be recorded");

    pat::lookup(&pool, &secret).await.unwrap();
    let second: (Option<chrono::DateTime<Utc>>,) =
        sqlx::query_as("SELECT last_used_at FROM api_token")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        first.0, second.0,
        "a second use inside the minute must not pay for another write"
    );
}

// --- the HTTP surface ---

async fn post_token(client: &Client, body: serde_json::Value) -> (Status, serde_json::Value) {
    let resp = client
        .post("/api/tokens")
        .header(ContentType::JSON)
        .body(Json(&body).to_string())
        .dispatch()
        .await;
    let status = resp.status();
    let text = resp.into_string().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or_default())
}

#[tokio::test]
async fn the_routes_mint_list_and_revoke() {
    let (pool, user) = db_with_user("token_routes").await;
    let client = Client::tracked(build_rocket(pool, user)).await.unwrap();

    let (status, minted) = post_token(
        &client,
        serde_json::json!({ "label": "agent", "scopes": ["read", "import"] }),
    )
    .await;
    assert_eq!(status, Status::Created);
    let secret = minted["secret"].as_str().unwrap().to_string();
    assert!(secret.starts_with("pp_"));
    assert_eq!(minted["scopes"], serde_json::json!(["read", "import"]));

    let resp = client.get("/api/tokens").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let list: serde_json::Value =
        serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    // The list must not carry the secret under any name.
    let rendered = list.to_string();
    assert!(
        !rendered.contains(&secret) && !rendered.contains("secret_hash"),
        "the list leaked a secret: {rendered}"
    );
    let id = list[0]["id"].as_i64().unwrap();

    let resp = client.delete(format!("/api/tokens/{id}")).dispatch().await;
    assert_eq!(resp.status(), Status::NoContent);
    // And the token really is dead, not merely listed as revoked.
    assert!(pat::lookup(&client.rocket().state::<DbPool>().unwrap(), &secret)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn the_mint_route_refuses_what_it_cannot_honour() {
    let (pool, user) = db_with_user("token_route_validation").await;
    let client = Client::tracked(build_rocket(pool, user)).await.unwrap();

    for body in [
        serde_json::json!({"label": "", "scopes": ["read"]}),
        serde_json::json!({"label": "x", "scopes": []}),
        serde_json::json!({"label": "x", "scopes": ["admin"]}),
        serde_json::json!({"label": "x", "scopes": ["read"], "expires_in_days": 0}),
        serde_json::json!({"label": "x", "scopes": ["read"], "expires_in_days": -3}),
        serde_json::json!({"label": "x", "scopes": ["read"], "expires_in_days": 99999}),
    ] {
        let (status, _) = post_token(&client, body.clone()).await;
        assert_eq!(status, Status::BadRequest, "rejected wrongly for {body}");
    }
}

// Why there is no test here for "a token cannot mint a token": it is structural, and a
// test would only restate the mount. The token routes live under `/api`, which only
// `AuthFairing` with a session cookie reaches; no route in this application reads an
// `Authorization` header at all. In development mode the guard injects a caller, so a
// request bearing a token would resolve to the injected user and the assertion would
// prove nothing about production. The tripwire for a bearer route appearing is
// `tests/route_coverage_test.rs`, which fails on any mounted `/api` route missing from
// its inventory — and a `/mcp` mount is deliberately outside `/api` and outside that
// inventory, which is why its own tests live with it in Workstream 2.
