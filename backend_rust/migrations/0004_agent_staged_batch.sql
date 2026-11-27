-- The staged-batch table: a proposal an agent wrote and a human has not yet approved.
--
-- This row is the approval gate made persistent. The design's control is that the step
-- turning a proposal into a portfolio write is one a token *cannot* perform, so the state
-- has to live server-side with a status column rather than in the agent's context, where it
-- would be whatever the model last said it was. `status` is the only thing `apply_staged`
-- consults, and only a session cookie can move `staged` to `approved`.
--
-- `payload` and `preview` are both stored, and both are load-bearing:
--   * `payload` is what apply writes. Storing it is what makes apply a separate act from
--     staging — the agent cannot re-send a different body and have it honoured.
--   * `preview` is what the human was shown, byte for byte. The UI renders this column and
--     does not recompute it, so "what I approved" and "what the agent described" cannot
--     drift apart. A preview recomputed at render time would silently change under a
--     different pricing state.
CREATE TABLE agent_staged_batch (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,

    -- 32 random bytes, not the row id. A handle that is a sequential integer leaks how
    -- many imports have happened and is enumerable, and this column is the only thing the
    -- agent holds that identifies a batch.
    handle      TEXT NOT NULL UNIQUE,

    user_id     BIGINT NOT NULL REFERENCES users(id),

    payload     JSONB NOT NULL,
    preview     JSONB NOT NULL,

    -- The state machine is `staged -> approved -> applied`, plus `discarded` as a terminal
    -- exit that needs no approval. `discarded` is in the CHECK rather than left to a deleted
    -- row so that "the agent threw it away" and "the row never existed" stay distinguishable
    -- in the audit trail.
    status      TEXT NOT NULL DEFAULT 'staged'
                CHECK (status IN ('staged', 'approved', 'applied', 'discarded')),

    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL,

    -- The human act, recorded. `approved_by` is deliberately NOT a foreign key: the spec
    -- names it as `users(id)` of the session that approved, and users are deactivated
    -- rather than deleted, so a dangling id cannot arise in normal operation while an FK
    -- would make a future user-delete a cascade decision nobody has made. It is an audit
    -- field, not a relationship the app queries through.
    approved_at TIMESTAMPTZ,
    approved_by BIGINT,
    applied_at  TIMESTAMPTZ
);

-- Every listing and every "clean up my expired batches" query is
-- `WHERE user_id = $1 AND status IN (...)`, and the pending-batch cap counts the same set.
CREATE INDEX agent_staged_batch_user_status_idx
    ON agent_staged_batch (user_id, status);

-- The handle is looked up alone, by `apply_staged` and by the session routes. The UNIQUE
-- constraint above already provides a btree index on `handle`, so there is no second index
-- here on purpose — a duplicate would be a second name for the same key and a second thing
-- to keep in the planner's statistics.
