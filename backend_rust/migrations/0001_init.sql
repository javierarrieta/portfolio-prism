-- 0001_init.sql — the whole schema, in one file, applied to a fresh database.
--
-- There is no historical replay (spec §7): the data arrives as a bundle into an empty
-- Postgres database, so there is nothing to migrate incrementally. Every change after
-- this file is a new numbered file; a shipped migration is never edited.
-- `_sqlx_migrations` is the ledger and the migrator creates it.
--
-- Money and quantity stay TEXT — the decimal is a string end to end (spec §3), and
-- casting it in the schema would be a second, different rounding rule. Dates become
-- native so the engine orders and compares them instead of the app. Ids are identity,
-- which refuses a supplied id: the SQL form of the bundle rule that ids are never
-- trusted from the file.

CREATE TABLE users (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    issuer        TEXT NOT NULL,
    subj          TEXT NOT NULL,
    email         TEXT,
    name          TEXT,
    is_active     BOOLEAN NOT NULL DEFAULT TRUE,
    token_version INTEGER NOT NULL DEFAULT 0,
    created_at    TIMESTAMPTZ NOT NULL,
    last_login_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX uq_users_issuer_subj ON users (issuer, subj);

CREATE TABLE portfolios (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    owner_id      BIGINT REFERENCES users(id),
    name          TEXT NOT NULL,
    description   TEXT,
    currency      TEXT NOT NULL DEFAULT 'USD',
    base_currency TEXT NOT NULL DEFAULT 'USD'
);
CREATE INDEX portfolios_owner_idx ON portfolios (owner_id);

CREATE TABLE assets (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    portfolio_id BIGINT NOT NULL REFERENCES portfolios(id),
    symbol       TEXT NOT NULL,
    name         TEXT NOT NULL,
    asset_type   TEXT NOT NULL,
    sector       TEXT,
    currency     TEXT NOT NULL DEFAULT 'USD',
    isin         TEXT
);
CREATE INDEX assets_portfolio_idx ON assets (portfolio_id);

CREATE TABLE transactions (
    id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    asset_id BIGINT NOT NULL REFERENCES assets(id),
    type     TEXT NOT NULL,
    quantity TEXT NOT NULL,
    price    TEXT NOT NULL,
    fee      TEXT NOT NULL,
    date     TIMESTAMPTZ NOT NULL
);
CREATE INDEX transactions_asset_date_idx ON transactions (asset_id, date);

-- One SPLIT per asset per UTC calendar day. This is the dedupe, not an optimisation:
-- `apply_split_events` (currency_service.rs:113-152) inserts with
-- `ON CONFLICT DO NOTHING` and has no read-based check of its own. The count-then-insert
-- it replaced let two concurrent callers both see zero and both write, doubling a split
-- adjustment and silently corrupting cost basis for that asset (#50). Drop this index
-- and that bug comes back quietly, because `ON CONFLICT DO NOTHING` with nothing to
-- conflict with is just an insert.
--
-- Ported from the SQLite original, `CREATE UNIQUE INDEX uq_split_asset_date ON
-- transactions(asset_id, date(date)) WHERE type = 'SPLIT'` (`enforce_split_uniqueness`,
-- lib.rs:473-494, statement at :487-488). The day is spelled `(date AT TIME ZONE 'UTC')::date` and not
-- `date_trunc('day', date)` because `date_trunc` on `timestamptz` is STABLE and Postgres
-- refuses a STABLE expression in an index, while `AT TIME ZONE 'UTC'` is IMMUTABLE. Do
-- not "simplify" the wrapping away: the wrapped form is what makes the key the UTC
-- calendar day, which is what SQLite's `date(date)` produced on the stored value, and
-- anything that later changes how a transaction's calendar day is derived has to keep
-- agreeing with it.
CREATE UNIQUE INDEX uq_split_asset_date
    ON transactions (asset_id, ((date AT TIME ZONE 'UTC')::date))
    WHERE type = 'SPLIT';

-- No owner_id, deliberately: same as today, and the reason price restore stays
-- flag-gated. Composite PK replaces the unique index SQLite needed.
CREATE TABLE historical_prices (
    symbol      TEXT NOT NULL,
    date        DATE NOT NULL,
    close_price TEXT NOT NULL,
    PRIMARY KEY (symbol, date)
);

-- The split-sync throttle cache (#38), keyed on `asset_id` alone. That is the shape the
-- deployed SQLite database has, and it is the shape the queries are written against:
-- `migrate_split_sync_key` (lib.rs:516-539) drops any table still carrying a `symbol`
-- column and recreates it as `asset_id INTEGER PRIMARY KEY`, and no `effective_date`
-- column exists anywhere in the code. The throttle read is `fetch_optional` on
-- `WHERE asset_id = ?` (currency_service.rs:916) — a second row per asset would make it
-- `MultipleRowsFound`, not merely a wrong answer — and both upserts
-- (currency_service.rs:997, :1469) plus both deletes (api_routes/transactions.rs:175,
-- backup/restore.rs:262) name `asset_id` alone. The `(asset_id, day)` uniqueness that
-- looks like it belongs here is on `transactions`, above.
CREATE TABLE split_sync (
    asset_id       BIGINT NOT NULL REFERENCES assets(id),
    last_synced_at TIMESTAMPTZ NOT NULL,
    covered_from   DATE,
    PRIMARY KEY (asset_id)
);
