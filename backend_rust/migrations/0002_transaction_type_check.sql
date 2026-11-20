-- Close the transaction-type vocabulary at the database.
--
-- `transactions.type` has been free text since 0001. The write path uppercased whatever
-- it was given and stored it, so `"DIVIDEND"` was accepted and then silently ignored:
-- `tax_engine.rs` and `stats_engine.rs` are both `if BUY / else if SPLIT / else if SELL`
-- with no terminal `else`, so a junk type produces no lot and no position change and
-- nothing on screen looks wrong. The Rust side is closed now by `models::TxType`; this
-- closes the storage side, so a future writer that bypasses the enum cannot reintroduce
-- the invisible-data failure.
--
-- Three things happen here, in this order, and the order is the content.
--
-- sqlx runs each file in one transaction, so any `RAISE EXCEPTION` below rolls the whole
-- migration back — including the normalization. That is what makes it safe to normalize
-- before validating: a file that finds something it cannot honour leaves the database
-- exactly as it found it. `sqlx::migrate!` runs at startup, so a failing migration means
-- the app does not boot; `scripts/audit_transaction_types.sh` is the pre-flight that is
-- meant to make that impossible rather than surprising.

-- (1) Refuse to normalize if normalizing would collide.
--
-- `uq_split_asset_date` (0001_init.sql:89-91) is a *unique partial* index keyed on
-- `(asset_id, (date AT TIME ZONE 'UTC')::date)` with the predicate `WHERE type = 'SPLIT'`
-- — case-sensitive, and pinned that literally by `scripts/check_migrations.sh`. Two rows
-- spelled `'split'` and `'SPLIT'` on the same asset and UTC day therefore both sit outside
-- the predicate today: the dedupe that is supposed to stop a split being applied twice has
-- been quietly missing them.
--
-- Step (2) would collapse them into one key and the `UPDATE` would fail with a bare
-- `unique_violation` naming an index and no reason. This names the rows instead, because
-- the operator reading it has to decide which of the two duplicates is real — and that is
-- a judgement about their data, not something a migration may guess at. Guessing would
-- delete tax data.
DO $$
DECLARE
    dup RECORD;
    offenders TEXT := '';
BEGIN
    FOR dup IN
        -- The UTC day is derived in the inner query, not in the GROUP BY list. Written
        -- the obvious way — grouping on `(date AT TIME ZONE 'UTC')::date` and
        -- correlating a spellings subquery against `t.date` — Postgres answers 42803
        -- "subquery uses ungrouped column", because the raw `date` is not itself in the
        -- grouping key even though the expression is. This shape says the same thing and
        -- groups cleanly.
        SELECT g.asset_id,
               g.day,
               count(*) AS n,
               string_agg(DISTINCT quote_literal(g.type), ', '
                 ORDER BY quote_literal(g.type)) AS spellings
          FROM (
                SELECT asset_id,
                       type,
                       (date AT TIME ZONE 'UTC')::date AS day
                  FROM transactions
                 WHERE upper(btrim(type)) = 'SPLIT'
               ) g
         GROUP BY g.asset_id, g.day
        HAVING count(*) > 1
         ORDER BY g.asset_id, g.day
    LOOP
        offenders := offenders || format(
            '{asset_id=%s utc_day=%s rows=%s spellings=[%s]} ',
            dup.asset_id, dup.day, dup.n, dup.spellings);
    END LOOP;

    IF offenders <> '' THEN
        RAISE EXCEPTION
            'refusing to normalize transaction types: these SPLIT rows are distinct only by case or whitespace, and upper(btrim(type)) would collapse them onto the unique partial index uq_split_asset_date: %',
            offenders
            USING HINT = 'These rows escaped split dedupe because its predicate is `type = ''SPLIT''` and is case-sensitive. Decide which row is real and delete the other by hand; this migration will not choose for you.';
    END IF;
END $$;

-- (2) Normalize case and surrounding whitespace.
--
-- Not a semantic change: `buy` and `BUY` are the same transaction, and a row written by
-- `src/access.rs`'s test seed or by any pre-existing client is the same trade. Restricting
-- the `UPDATE` to rows that actually differ keeps it cheap and keeps `WHERE` honest about
-- what it is for.
UPDATE transactions
   SET type = upper(btrim(type))
 WHERE type IS DISTINCT FROM upper(btrim(type));

-- (3) Refuse anything still outside the set, and say what it was.
--
-- Rewriting `DIVIDEND` to `BUY` would be inventing tax data, which is the one failure
-- mode worse than the bug this migration fixes. So the migration stops and lists.
--
-- The list is capped because the exception text is what a crashlooping container prints
-- to a log; a database with a million junk rows should not produce a million-value
-- message. The count is always given, so the cap never hides the scale of the problem.
DO $$
DECLARE
    n BIGINT;
    offenders TEXT;
BEGIN
    SELECT count(*) INTO n
      FROM transactions
     WHERE upper(btrim(type)) NOT IN ('BUY', 'SELL', 'SPLIT');

    IF n > 0 THEN
        SELECT string_agg(v, ', ' ORDER BY v)
          INTO offenders
          FROM (
                SELECT DISTINCT quote_literal(type) AS v
                  FROM transactions
                 WHERE upper(btrim(type)) NOT IN ('BUY', 'SELL', 'SPLIT')
                 ORDER BY v
                 LIMIT 20
               ) q;

        -- Two substitutions, two arguments. `%%` here would be a literal percent sign,
        -- not a second placeholder, and PL/pgSQL answers a mismatch with "too many
        -- parameters specified for RAISE" pointing at the end of the file rather than
        -- at the sentence — which is how this line got it wrong once.
        RAISE EXCEPTION
            'refusing to add transactions_type_check: % row(s) hold a type outside BUY|SELL|SPLIT: %. The engines ignore any type outside that set rather than failing, which is the bug being closed; rewriting them would invent tax data.',
            n, offenders
            USING HINT = 'Reclassify or delete those rows first. scripts/audit_transaction_types.sh lists them.';
    END IF;
END $$;

-- (4) The constraint itself.
--
-- Case-insensitive on purpose, and this is the subtle part: step (2) normalized every row
-- that exists *now*, but the `CHECK` governs every row written *after* this migration by
-- anything that is not the Rust enum — a psql session, a restore script, a future
-- migration. `upper(btrim(type))` accepts a legacy `buy` without a data rewrite while
-- still refusing `DIVIDEND`, so the constraint can be added without the rewrite that
-- would have made it a data migration rather than a guard.
--
-- It does not make the partial index case-insensitive. A row inserted as `'split'` after
-- this point satisfies the `CHECK` and still escapes the dedupe — the asymmetry step (1)
-- exists to surface. Closing it properly means rebuilding `uq_split_asset_date` on the
-- same `upper(btrim(...))` expression, which changes what the index keys on and is a
-- separate decision with its own audit; see the roadmap note in `AGENTS.md`.
ALTER TABLE transactions
    ADD CONSTRAINT transactions_type_check
    CHECK (upper(btrim(type)) IN ('BUY', 'SELL', 'SPLIT'));
