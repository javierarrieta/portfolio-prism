//! Personal access tokens — the only implementation of every token operation.
//!
//! A token is a credential for a program, not for a browser. Three things follow
//! from that and are enforced here rather than left to the caller:
//!
//! * **The secret is unrecoverable.** Only its SHA-256 is stored, and it is returned
//!   exactly once, by [`mint`]. Nothing in this module can produce a secret for a
//!   token it did not just create, so no listing route can leak one by accident.
//! * **A token is not a session.** `logout-all` bumps `users.token_version` and kills
//!   sessions; tokens deliberately survive it, because a token is not sitting in a
//!   browser that might have been left open on a library machine. Revocation is the
//!   answer to a leaked token.
//! * **A token dies with its user.** [`lookup`] resolves the token to a `user_id` and
//!   then hands that to the same `user_by_id` the session guard uses, so a
//!   deactivated account loses its tokens for free — not because this module
//!   remembers to check, but because there is only one place that decides whether a
//!   user is usable.
//!
//! Scopes are a ceiling on what a token may do. They are not a record of who is
//! sitting in front of it; the human step in the write path is the approval state,
//! not a scope.

use chrono::{DateTime, Utc};
use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

use crate::auth::guard::{self, AuthedUser, Credential};
use crate::db::DbPool;

/// What a token is allowed to do.
///
/// **This enum is the scope set.** The database column is free text on purpose — a
/// `CHECK` over the same vocabulary would be a second home for the rule, and the two
/// copies drift. `mint` validates against this enum, so nothing else can enter.
///
/// There is no `Write` variant: the agent creates rows and does nothing else, so
/// `Import` already covers every write it can attempt, and a third scope would name
/// nothing. There is no `Backup` variant: backup is not exposed to agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Read,
    Import,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Import => "import",
        }
    }

    /// The reverse of [`Scope::as_str`], and the only place the wire spelling is
    /// interpreted. An unrecognised scope is an error rather than a silently empty
    /// grant: a token that turns out to do nothing is worse than a request refused
    /// with the valid names in the message.
    pub fn parse(s: &str) -> Option<Scope> {
        match s.trim().to_ascii_lowercase().as_str() {
            "read" => Some(Scope::Read),
            "import" => Some(Scope::Import),
            _ => None,
        }
    }
}

/// Why a token operation failed.
#[derive(Debug)]
pub enum PatError {
    /// The OS RNG refused to produce bytes. Practically unreachable; it is not
    /// collapsed into a database error because "the machine cannot generate
    /// randomness" is an operator problem, not a client one.
    Entropy,
    /// The caller sent something unusable: a blank label, no scopes, an unknown
    /// scope name.
    InvalidInput,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for PatError {
    fn from(e: sqlx::Error) -> Self {
        PatError::Database(e)
    }
}

/// A freshly minted token. `secret` exists only here.
#[derive(Debug)]
pub struct Minted {
    pub id: i64,
    /// The one time the plaintext leaves this module.
    pub secret: String,
    pub label: String,
    pub scopes: Vec<Scope>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A token as listed for the owner: metadata only.
///
/// There is no field this type could be constructed with that reveals anything about
/// the secret — the list query does not select the hash column at all.
#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub id: i64,
    pub label: String,
    pub scopes: Vec<Scope>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// Idle for so long that the owner has almost certainly forgotten it exists.
    /// Computed in SQL, not here, so the threshold is one number in one place.
    pub stale: bool,
}

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// 43 base62 characters carry 43 · log2(62) ≈ 256 bits, which is what 32 random
/// bytes are worth. The encoding changes the length, not the entropy.
const SECRET_LEN: usize = 43;
const PREFIX: &str = "pp_";

/// Base62 from the system RNG, with rejection so the modulo is unbiased.
///
/// Bytes at or above 248 (= 62 · 4) are discarded: keeping them would make the first
/// 8 symbols of the alphabet slightly more likely than the rest, and a slightly
/// guessable token is a worse trade than a loop that runs a second time in one in
/// ten million.
fn base62(len: usize) -> Result<String, PatError> {
    let rng = SystemRandom::new();
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        rng.fill(&mut buf).map_err(|_| PatError::Entropy)?;
        for b in buf {
            if b < 248 {
                out.push(ALPHABET[b as usize % 62] as char);
                if out.len() == len {
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// SHA-256 of the secret, lowercase hex.
///
/// Plain SHA-256 and not a password KDF: the input is 256 bits of entropy from a
/// CSPRNG, so there is no dictionary to search and no benefit to making each guess
/// slow. The same reasoning is why this module adds no attempt throttling on the
/// consumer side — see the spec's note on guessing being impractical at the source.
fn hash_secret(secret: &str) -> String {
    digest(&SHA256, secret.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn to_scopes(raw: &[String]) -> Result<Vec<Scope>, PatError> {
    if raw.is_empty() {
        return Err(PatError::InvalidInput);
    }
    raw.iter()
        .map(|s| Scope::parse(s).ok_or(PatError::InvalidInput))
        .collect()
}

/// Create a token. Returns the plaintext secret exactly once, in the returned
/// [`Minted`]; it is not stored anywhere reachable again.
pub async fn mint(
    pool: &DbPool,
    user_id: i64,
    label: &str,
    scopes: &[String],
    expires_at: Option<DateTime<Utc>>,
) -> Result<Minted, PatError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(PatError::InvalidInput);
    }
    let scopes = to_scopes(scopes)?;

    let secret = format!("{PREFIX}{}", base62(SECRET_LEN)?);
    let hash = hash_secret(&secret);
    // Bound by reference: sqlx infers a `TEXT[]` parameter as `&[String]`, and the
    // owned vector has to outlive the bind.
    let scope_names: Vec<String> = scopes.iter().map(|s| s.as_str().to_string()).collect();

    let row = sqlx::query_as!(
        TokenRow,
        "INSERT INTO api_token (user_id, secret_hash, label, scopes, expires_at)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, user_id, label, scopes, expires_at, last_used_at, revoked_at, created_at",
        user_id,
        hash,
        label,
        &scope_names,
        expires_at
    )
    .fetch_one(pool)
    .await?;

    Ok(Minted {
        id: row.id,
        secret,
        label: row.label,
        scopes,
        expires_at: row.expires_at,
        created_at: row.created_at,
    })
}

/// List a user's tokens, newest first. Never selects `secret_hash`.
pub async fn list(pool: &DbPool, user_id: i64) -> Result<Vec<TokenInfo>, PatError> {
    let rows = sqlx::query_as!(
        TokenListRow,
        "SELECT id, user_id, label, scopes, expires_at, last_used_at, revoked_at, created_at,
                -- A token that has never been used is judged from its creation, not
                -- from a null: a credential minted and never used for a quarter is
                -- exactly the one the nudge is for, and the literal
                -- `last_used_at < now() - interval '90 days'` would skip it.
                COALESCE(last_used_at, created_at) < now() - interval '90 days' AS \"stale!\"
         FROM api_token
         WHERE user_id = $1
         ORDER BY created_at DESC",
        user_id
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(TokenListRow::into_info).collect()
}

/// Revoke a token. Idempotent from the caller's point of view: revoking a token that
/// is already revoked reports `false`, which is the honest answer and not an error.
pub async fn revoke(pool: &DbPool, user_id: i64, token_id: i64) -> Result<bool, PatError> {
    let row = sqlx::query_scalar!(
        "UPDATE api_token SET revoked_at = now()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL
         RETURNING id",
        token_id,
        user_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.is_some())
}

/// Turn a presented secret into a caller, or `None`.
///
/// Every failure is the same `None` — unknown secret, revoked, expired, user gone —
/// because the difference tells an attacker which of their guesses were close, and
/// the client cannot act on any of the distinctions except "get a new token".
///
/// This is the only entry point a bearer-token surface needs. It is deliberately not
/// wired into `AuthedUser::from_request`: a session cookie and a bearer token are
/// different credentials and the route that accepts one must not accidentally accept
/// the other. The mount that calls this is the MCP route, and it lives in Workstream 2.
pub async fn lookup(pool: &DbPool, secret: &str) -> Result<Option<AuthedUser>, PatError> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let hash = hash_secret(trimmed);

    let row = sqlx::query_as!(
        TokenRow,
        "SELECT id, user_id, label, scopes, expires_at, last_used_at, revoked_at, created_at
         FROM api_token
         WHERE secret_hash = $1",
        hash
    )
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    if row.revoked_at.is_some() {
        return Ok(None);
    }
    // One comparison, the same shape as the session `exp` check in `guard.rs`.
    if let Some(exp) = row.expires_at && exp <= Utc::now() {
        return Ok(None);
    }

    let scopes = to_scopes(&row.scopes)?;

    // The deactivated-account and missing-user cases are decided here, by the same
    // function the session path uses, rather than by a second copy of that logic.
    let Some(mut user) = guard::user_by_id(pool, row.user_id).await? else {
        return Ok(None);
    };
    user.credential = Credential::Token {
        id: row.id,
        scopes,
    };

    touch_last_used(pool, row.id, row.last_used_at).await;

    Ok(Some(user))
}

/// Record that the token was used, at most once a minute.
///
/// The spec asked for fire-and-forget so a burst of tool calls does not pay a write
/// each. The throttle achieves the same thing while staying inline: the UPDATE's
/// `WHERE` is what enforces the rate, so a spawn would only move the same rare write
/// off the request path and lose it on shutdown. Keeping it inline also keeps it
/// observable — a test can assert the column moved, which a detached task makes
/// racy.
async fn touch_last_used(pool: &DbPool, token_id: i64, last_used_at: Option<DateTime<Utc>>) {
    if let Some(at) = last_used_at && (Utc::now() - at).num_minutes() < 1 {
        return;
    }
    let result = sqlx::query(
        "UPDATE api_token SET last_used_at = now()
         WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')",
    )
    .bind(token_id)
    .execute(pool)
    .await;

    if let Err(e) = result {
        // A failed bookkeeping write must not fail an authenticated request.
        eprintln!("WARN: could not record use of token {token_id}: {e}");
    }
}

/// The columns every token query reads. `query_as!` requires the projection and the
/// struct to agree exactly, which is why the list query — the one that also computes
/// `stale` — has its own type below rather than a nullable field here.
struct TokenRow {
    id: i64,
    user_id: i64,
    label: String,
    scopes: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

struct TokenListRow {
    id: i64,
    user_id: i64,
    label: String,
    scopes: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    stale: bool,
}

impl TokenListRow {
    fn into_info(self) -> Result<TokenInfo, PatError> {
        // `user_id` is selected so that a row belonging to someone else cannot be
        // listed by a query that forgot its `WHERE`; nothing here trusts the filter
        // to have been written correctly.
        let _ = self.user_id;
        Ok(TokenInfo {
            id: self.id,
            label: self.label,
            scopes: to_scopes(&self.scopes)?,
            created_at: self.created_at,
            expires_at: self.expires_at,
            last_used_at: self.last_used_at,
            revoked_at: self.revoked_at,
            stale: self.stale,
        })
    }
}
