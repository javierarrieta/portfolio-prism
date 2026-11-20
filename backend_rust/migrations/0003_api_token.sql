-- Personal access tokens: a credential an agent holds, distinct from a browser session.
--
-- The table is deliberately narrow. A token is a name, a set of scopes, a hash, and
-- three timestamps; anything richer belongs to the human's session and does not
-- survive being held by a program.
--
-- `secret_hash` and not `secret`. The secret is shown once at mint and is never
-- recoverable, which is the same posture as every other working credential store. A
-- leak of this table hands over nothing usable on its own — the hash is SHA-256 of a
-- 256-bit random value, so there is nothing to brute-force back from.
--
-- `user_id` has no `ON DELETE CASCADE`: `users` rows are not deleted in this
-- application (deactivation is `is_active = false`), and a token that outlived its
-- user by cascade would be a silent difference from what a deactivated user's
-- sessions already do — which is to say, nothing, because `pat::lookup` resolves the
-- user through the same `is_active` check every other path uses.
CREATE TABLE api_token (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id),
    secret_hash TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    -- Free text rather than a CHECK over ('read','import'). The scope set has one
    -- authority — the `Scope` enum in `auth/pat.rs` — and a second copy of it in a
    -- constraint is a rule with two homes, which is how the two drift. `mint`
    -- validates against the enum, so an out-of-vocabulary scope cannot arrive here
    -- through the API.
    scopes TEXT[] NOT NULL,
    -- NULL means no expiry, which is the default at mint: the alternative default
    -- makes a token expire mid-run and teaches the user to mint long-lived ones
    -- anyway. Revocation is the answer to a leaked token, not a calendar.
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Every listing is "this user's tokens"; the uniqueness on `secret_hash` is the only
-- other access path and it is already an index.
CREATE INDEX api_token_user_idx ON api_token (user_id);
