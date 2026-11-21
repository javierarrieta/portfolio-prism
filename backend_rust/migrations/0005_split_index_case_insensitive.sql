-- Rebuild the split dedupe index so it is case-insensitive, matching the CHECK.
--
-- WHY THIS EXISTS
--
-- `0001_init.sql` creates `uq_split_asset_date` with `WHERE type = 'SPLIT'`. `0002` then
-- added `transactions_type_check` as `upper(btrim(type)) IN ('BUY','SELL','SPLIT')` —
-- deliberately case-insensitive, so that a legacy `buy` would not need a data rewrite before
-- the constraint could be added. The two are asymmetric, and `0002` says so out loud:
--
--   It does not make the partial index case-insensitive. A row inserted as `'split'` after
--   this point satisfies the `CHECK` and still escapes the dedupe.
--
-- That is this migration. The asymmetry is not theoretical in one specific way that matters:
-- the index is the *only* dedupe. `apply_split_events` inserts with `ON CONFLICT DO NOTHING`
-- and has no read-based check of its own — the count-then-insert it replaced let two
-- concurrent callers both see zero and both write (#50). A split that escapes the index is a
-- split applied twice, and a double-applied split rescales cost basis silently.
--
-- WHO CAN ACTUALLY WRITE A VARIANT
--
-- Not the application. `services::transactions::insert` takes a `TxType` enum, which
-- serializes to uppercase; there is no `to_uppercase()` to forget because there is no string
-- to forget it on. The reachable writers are exactly the ones `0002` named when it chose a
-- case-insensitive CHECK: a psql session, a restore script, a future migration. That is also
-- why this migration is worth writing despite being unreachable through the API — the
-- constraint that guards the column is the one that permits the variant, and a guard that
-- permits what the dedupe cannot see is not a guard.
--
-- ORDER MATTERS
--
-- Audit, then normalize, then rebuild. Normalizing first would move a `'split'` row onto the
-- *old* index, which is still case-sensitive at that moment, and Postgres would answer with a
-- bare unique violation instead of the named, hinted refusal below. The audit runs first so
-- that the human reading the failure is told which asset and which UTC day, and is told to
-- choose — this migration will not pick which of two conflicting splits is real.

-- (1) Refuse if widening the predicate would collapse two distinct rows.
--
-- Any group with more than one row here is by construction a case- or whitespace-variant
-- collision: two rows both spelled exactly `SPLIT` on the same asset and UTC day cannot
-- exist, because the current index already forbids that. So `count(*) > 1` here means
-- `0002`'s escape hatch was used, and by what.
DO $$
DECLARE
    offenders text := '';
    dup record;
BEGIN
    FOR dup IN
        -- The UTC day is derived in the inner query, not in the GROUP BY list, for the same
        -- 42803 reason `0002` documents at its step (1). Same shape, same reason.
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
            'refusing to make uq_split_asset_date case-insensitive: these SPLIT rows are distinct only by case or whitespace and would collapse onto one key: %',
            offenders
            USING HINT = 'Decide which row is real and delete the other by hand. The dedupe was blind to these because its predicate was `type = ''SPLIT''`; making it case-insensitive turns them into the duplicate it was always meant to prevent.';
    END IF;
END $$;

-- (2) Normalize the variants, so the rebuilt index keys on one spelling.
--
-- Scoped to SPLIT rows only. `0002` step (2) normalized every row in the table; re-running
-- that here would be harmless but would make this migration silently own a rewrite it was not
-- written to perform. The blast radius is stated in the WHERE clause.
UPDATE transactions
   SET type = 'SPLIT'
 WHERE type IS DISTINCT FROM 'SPLIT'
   AND upper(btrim(type)) = 'SPLIT';

-- (3) Rebuild on the same expression the CHECK uses.
--
-- DROP then CREATE is not atomic, and between the two statements nothing constrains a
-- duplicate split. That is acceptable here and the reason is deployment shape, not luck:
-- `sqlx::migrate!` runs at startup before Rocket serves a single request, and the live
-- deployment is `replicas: 1` with `Recreate`, so there is no concurrent writer to lose.
-- Anyone who later scales this out has to revisit this statement, because at two replicas the
-- window is a real race.
--
-- `upper` and `btrim` on `text` are IMMUTABLE, which is what makes them legal in an index
-- predicate at all — the same reason `0001` wraps the date rather than using a STABLE form.
DROP INDEX uq_split_asset_date;

CREATE UNIQUE INDEX uq_split_asset_date
    ON transactions (asset_id, ((date AT TIME ZONE 'UTC')::date))
    WHERE upper(btrim(type)) = 'SPLIT';
