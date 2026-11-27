-- Give `assets (portfolio_id, symbol)` the unique index the write path has always assumed.
--
-- WHY THIS EXISTS
--
-- Asset identity inside a portfolio is enforced in software and nowhere else.
-- `services::assets::insert` selects the uppercased symbol and inserts only if nothing came
-- back; `agent::staging::resolve_asset` does the same inside its transaction. Neither is
-- backed by a constraint: `0001_init.sql` creates `assets_portfolio_idx`, a plain
-- non-unique index on `portfolio_id`, and nothing since has added one on the symbol. So the
-- rule is a `SELECT`-then-`INSERT` with a window in the middle, and two concurrent writers —
-- two staged batches applying at once, a restore racing an import, a psql session beside
-- either — can both find nothing and both write. Postgres does not object, because nothing
-- was ever declared to object.
--
-- That is not hypothetical in the way that matters. The staged-import duplicate matching in
-- `apply` seeds its seen-set from the rows already stored for one asset, so a second asset
-- row for the same symbol means a re-staged statement lands its rows in whichever asset the
-- lookup happened to pick, and the duplicate sits there holding the rows from the run that
-- raced. Cost basis is then split across two assets and every report reads one of them.
--
-- WHY IT WAS NOT ADDED EARLIER
--
-- Because a unique index cannot be added to a table that already violates it, and nobody had
-- looked. A bare `CREATE UNIQUE INDEX` here would fail on whatever duplicate a live database
-- already holds, and `sqlx::migrate!` runs at startup before Rocket serves a request
-- (`main.rs`), so the failure is a crashloop with the reason in a container log. Hence the
-- audit first, and `scripts/audit_asset_symbols.sh` to run it before the deploy rather than
-- during it.
--
-- ORDER MATTERS
--
-- Audit, then create. The audit groups on `upper(symbol)` — the key the index will use — so
-- it sees every collision the index would refuse, including the case-variant ones that the
-- software's own lookup is blind to today. Nothing rewrites the data; see step (2).

-- (1) Refuse if two rows in one portfolio are the same asset spelled differently, or the
--     same asset spelled the same way twice.
DO $$
DECLARE
    offenders text := '';
    dup record;
BEGIN
    FOR dup IN
        SELECT portfolio_id,
               upper(symbol) AS key,
               count(*) AS n,
               string_agg(DISTINCT quote_literal(symbol), ', '
                 ORDER BY quote_literal(symbol)) AS spellings,
               string_agg(DISTINCT id::text, ', ' ORDER BY id::text) AS ids
          FROM assets
         GROUP BY portfolio_id, upper(symbol)
        HAVING count(*) > 1
         ORDER BY portfolio_id, upper(symbol)
    LOOP
        offenders := offenders || format(
            '{portfolio_id=%s symbol=%s rows=%s ids=[%s] spellings=[%s]} ',
            dup.portfolio_id, dup.key, dup.n, dup.ids, dup.spellings);
    END LOOP;

    IF offenders <> '' THEN
        RAISE EXCEPTION
            'refusing to make assets (portfolio_id, symbol) unique: these rows are the same symbol within one portfolio and would collapse onto one key: %',
            offenders
            USING HINT = 'Decide which row is real and merge or delete the other by hand, including its transactions. The dedupe was blind to these because it was a SELECT-then-INSERT with no constraint underneath it, and a case-variant symbol is invisible to that lookup. Do not resolve this by disabling the index: a split asset is a split cost basis.';
    END IF;
END $$;

-- (2) NO normalization step, deliberately.
--
-- The obvious move here is `UPDATE assets SET symbol = upper(symbol)`, mirroring what `0005`
-- did to `transactions.type`. It is wrong here, and the reason is `historical_prices`.
-- Price rows are keyed by the symbol string, not by `asset_id`: `historical_prices` has
-- `PRIMARY KEY (symbol, date)` and every read path looks prices up by the asset's symbol
-- (`currency_service::get_historical_prices_from_db`, called from `stats_engine.rs:154` with
-- the symbol off the asset row). So rewriting an asset's symbol silently detaches its price
-- history — the asset keeps existing, the chart goes empty, and nothing errors. Fixing that
-- properly means upper-casing `historical_prices` too, which is the largest table in the
-- database, whose own primary key that rewrite could violate, on a path where a failed
-- migration is a crashloop.
--
-- It is also unnecessary. The audit above guarantees one row per (portfolio_id,
-- upper(symbol)) before the index is built, so the index builds cleanly with the data as it
-- is. What normalization was really for — making a legacy lowercase row visible to the
-- dedupe lookup — is done instead by changing the lookup to compare `upper(symbol)`, in
-- `services::assets::insert` and `agent::staging::resolve_asset`, which is the same move
-- `0005` made on the `type` CHECK: make the code and the constraint agree on one expression
-- rather than rewriting the data until they happen to.
--
-- The residue is stated rather than hidden: a symbol stored in a spelling the write path
-- never produces is still a row someone should clean up, and
-- `scripts/audit_asset_symbols.sh` counts them so whoever deploys knows how many.

-- (3) The constraint.
--
-- `upper` on `text` is IMMUTABLE, which is what makes it legal in an index expression at
-- all — the same reason `0001` wraps the split date rather than using a STABLE form. The
-- expression has to be `upper(symbol)` and not `symbol`, because the software compares the
-- uppercased value; an index on the bare column would enforce a different rule than the one
-- the code applies and would still admit `aapl` beside `AAPL`. The lookups were changed to
-- match this expression rather than the column, so a legacy lowercase row is found by them
-- and refused with the ordinary "asset already exists" answer instead of slipping past the
-- check and hitting the index as a raw violation.
--
-- Not CONCURRENTLY, and that is a deployment-shape decision rather than an oversight: a
-- plain CREATE UNIQUE INDEX takes an ACCESS EXCLUSIVE lock on `assets` for its duration,
-- which is fine when nothing else is writing. `sqlx::migrate!` runs before Rocket serves a
-- single request and the live deployment is `replicas: 1` with `Recreate`, so there is no
-- concurrent writer to block. CONCURRENTLY cannot run inside a transaction, and sqlx wraps
-- each migration in one, so it would need a `.no-transaction` file and a different failure
-- story — a failed CONCURRENTLY build leaves an INVALID index that must be dropped by hand.
-- Anyone who scales this out past one replica has to revisit this statement, exactly as
-- `0005` says of its own DROP-then-CREATE.
CREATE UNIQUE INDEX uq_assets_portfolio_symbol
    ON assets (portfolio_id, upper(symbol));
