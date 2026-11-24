## Review — PR #77 (portfolio-prism), axis: code-level correctness

Target: `/home/coder/wt-pp-staging` @ `df258e4` (tree authoritative). Read in full: `agent/staging.rs`, `agent/tools.rs`, `agent/mount.rs`, `agent/protocol.rs`, `db_types.rs`, `access.rs`, `auth/pat.rs`, `auth/guard.rs`, `auth/fairing.rs` (grep), `services/{assets,transactions}.rs`, `backup/detail.rs`, `backup/mod.rs` (validate/parse_and_validate), `backup/restore.rs` (plan_for), `api_routes/{staged_batches,tokens,backup,transactions,analytics}.rs`, `migrations/0005`, `migrations/0006`, `scripts/audit_asset_symbols.sh`, `frontend/src/utils/csvValidator.ts`, `frontend/src/components/PortfolioDetail.tsx`, `frontend/src/components/CsvImportModal.tsx` (grep), tests: `apply_test.rs`, `staging_test.rs`, `mcp_mount_test.rs`, `strict_decimal_test.rs`, `backup_detail_test.rs`, `csvValidator.test.ts`, `route_coverage_test.rs`, `tests/common/mod.rs`. Packet: `pr_body.md`, `diffstat.txt`, `cargo_test.log` (all suites `ok`, 0 failed).

---

## Blockers

**B1 — `duplicate_symbol` is unreachable through every route; the PR body's stated reason for keeping it is false.**
`api_routes/backup.rs:412` runs `parse_and_validate` → `backup/mod.rs:323` → `collect` → the duplicate-symbol refusal at `backup/mod.rs:433`, and returns 422 before `plan_for` is ever called. `plan_for` (`backup/restore.rs:82`) is the *only* caller of `detail_from_bundle` (`backup/detail.rs:244`), so the flag set at `detail.rs:221` can never reach a client. PR body: *"the flag stays live, on the bundle side… the flag is what tells a human, in the preview, why their bundle is about to be refused by `validate`"* — the preview never renders such a bundle. `detail.rs:207-213` and `backup_detail_test.rs:588-590` repeat the claim. This is exactly the "detector that cannot fire is a reader's false premise" the same commit deleted `owned_detail`'s half for. Fix: delete the bundle-side detector too (or run `detail_from_bundle` before validation and render both).

**B2 — PR body claim about `0005` is false: the DROP-then-CREATE window does not exist.**
`migrations/0005_split_index_case_insensitive.sql:96-105` and the PR body both say the DROP/CREATE "is not atomic", is safe only because `replicas: 1` + `Recreate`, and that "at two replicas that window is a real race". sqlx 0.8 (`Cargo.toml:9`) wraps every migration in a transaction unless the filename contains `.no-transaction`; no file in `migrations/` does (listed). `DROP INDEX` takes ACCESS EXCLUSIVE on `transactions`, which blocks every concurrent INSERT until the single commit that also creates the new index. There is no observable window at any replica count. Behaviour is safe; the stated safety argument — and the warning it attaches to — is wrong. Same correction applies to `0006:104-110`'s framing of the ACCESS EXCLUSIVE lock.

---

## Majors

**M1 — `MAX_PORTFOLIOS_PER_USER` inside the transaction does not serialise concurrent applies; the comment claims it does.**
`staging.rs:1154-1163`: *"It runs on the open transaction, after the batch has been claimed by the flip, so two concurrent applies cannot both count nine and both create a tenth."* The flip locks the **batch** row (`staging.rs:975-981` `FOR UPDATE`); two different batches are two different rows. `SELECT count(*) FROM portfolios WHERE owner_id = $1` (`staging.rs:1160-1165`) takes no lock, and `pg_advisory_xact_lock(user_id)` is taken only in `stage` (`staging.rs:597`) — never in `apply`. Two approved `Target::New` batches applied concurrently both count 9 and both insert → 11. Same shape at `staging.rs:565-575`, where the early count runs *before* `pool.begin()` at `:583`, i.e. outside the advisory lock. Fix: take `pg_advisory_xact_lock(user_id)` in `apply`'s transaction too, or `SELECT … FOR UPDATE` the owner's `users` row; and correct the comment.

**M2 — `mcp_refuses_a_session_cookie` never sends a cookie; it is byte-identical to the test above it.**
`mcp_mount_test.rs:111-128`: both tests call `rpc(&client, initialize(1), None)`, and `rpc` (`:36-47`) only ever sets `Content-Type`, `Accept`, and optionally `Authorization`. No `Cookie` header is attached anywhere in the file. So it cannot fail for any reason the no-credential test does not already cover, and the `cookie` strip at `mount.rs:236` is untested. `mount.rs:17-20` ("both are tested… a request carrying a **session cookie** and no bearer token is refused") and `mount.rs:231` ("`mcp_refuses_a_session_cookie` pins that") are false, as is `route_coverage_test.rs:218-222`, which makes `SELF_AUTHENTICATED` rest on these two tests. To make it fail: attach a real signed `pp_session` cookie (or any `Cookie:` header) and assert 401, plus assert the cookie does not appear in the forwarded request.

**M3 — `read_tools_answer_not_found_for_someone_elses_rows` asserts "some error string", not the code.**
`mcp_mount_test.rs:585`: `matches!(code, Value::String(_))`. Delete `require_transaction` from `get_transaction_pnl` (`tools.rs:640-645`) and the tool proceeds to `TaxLotEngine::sell_breakdown` against the offline 404 mock (`tests/common/mod.rs:99-105`), returns `refused("pricing", …)`, which is a `Value::String` — test still passes on a stranger's transaction id. Same exposure for `get_performance` if its engine path ever errors. Fix: assert the exact codes (`transaction_not_found`, `portfolio_not_found`), as `mcp_mount_test.rs:345-349` already does for `get_portfolio`.

**M4 — the two race claims the PR body calls load-bearing have no concurrency test, while the repo has the pattern.**
`pg_advisory_xact_lock` serialising the pending-batch cap (`staging.rs:597`, PR body: *"now serialised by `pg_advisory_xact_lock(user_id)`"*) and apply's single-use under two concurrent callers are both untested. `staging_test.rs:381-401` (`the_sixth_pending_batch…`) is strictly sequential and would pass with the advisory lock deleted; `apply_test.rs:104-121` (`a_batch_applies_exactly_once`) is sequential too and would pass with `FOR UPDATE` deleted, because the conditional flip alone catches a *sequential* second call. The repo already does this correctly elsewhere: `split_sync_endpoint_test.rs:173-195` and `restore_concurrency_test.rs:69-71,142-144` use `tokio::join!`. Two `tokio::join!` tests (stage×2 at 4 pending; apply×2) would make both claims falsifiable.

**M5 — `parseDecimal` has no scale or precision cap, so it accepts cells the backend refuses — contradicting its own stated rationale.**
`csvValidator.ts:206-216` says the rule is deliberately narrower than `parseFloat` precisely because *"a cell that parses here and is then refused there costs the user a failed import row and a round trip"*, but `DECIMAL_ONLY` (`:217`) is `^-?\d+(\.\d+)?$` with no bound. `parseDecimal("0.0000000000000001")` (16 dp) returns the string, `validatePrice` passes it (`Number()` ≥ 0, `:301`), the row is posted, and `is_canonical_decimal` refuses it at `db_types.rs:110-112` (MAX_DECIMAL_SCALE = 12). Same for any value over 28 total digits (`db_types.rs:114`). Fix: bound the fraction to 12 digits and the total to 28 in `DECIMAL_ONLY`, mirroring the two backend constants.

**M6 — the surviving `duplicate_symbol` detector is case-sensitive; the validator it exists to pre-announce is case-insensitive.**
`backup/detail.rs:217` dedupes on `a.symbol.as_str()`; `backup/mod.rs:433` dedupes on `a.symbol.to_uppercase()`, and `mod.rs:442-445` says the upper-casing is deliberate because the index is `upper(symbol)`. A bundle holding `MSFT` and `msft` in one portfolio is a refusal that the flag would not have flagged. Independent of B1: even if the preview were reachable, the flag misses the case the validator's own comment calls out as the real one. Fix: `a.symbol.to_uppercase()` in `detail.rs:214-220`.

---

## Nits

**N1 — `db_types.rs:80-82` documents a rule the code does not implement.** The doc says the constant is "the largest integer part… 16 integer digits is the point past which the two rules would disagree", but `MAX_DECIMAL_PRECISION = 28` and it is applied at `:114` as a *total* digit count (`int_digits.len() + frac.len()`), not an integer-part bound. The code is the correct rule (rust_decimal max is 7.9e28); the comment is stale.

**N2 — three tools map database failures to `McpError`, which the module elsewhere says never to do.** `tools.rs:176`, `:214`, `:412`, `:441` use `McpError::internal_error(e.to_string(), None)`, while `discard_staged` (`tools.rs:305-312`) deliberately converts `StageError::Database` to a tool result for the stated reason that `Err(McpError)` "is the one shape that invites an identical retry". A DB error is not a malformed argument. Also `e.to_string()` puts raw sqlx text (constraint and column names) on an agent-facing wire.

**N3 — `resolve_asset` silently discards a staged ISIN, name and currency when the symbol already exists.** `staging.rs:1240-1248` returns the found id without comparing `pa.isin`/`pa.name`/`pa.currency`. The preview shows the ISIN (`staging.rs:451-453`) and a human approves it; nothing says it was not stored. Same class as the currency-default bug `250ccdf` fixed, one level down.

---

## Refutations and accepted risk

- **apply *is* single-use under two concurrent approvers.** `staging.rs:975-981` `SELECT … FOR UPDATE` makes the second caller block; on wake it reads `applied` at `:989` and returns `AlreadyApplied`. The conditional flip at `:1008-1016` is a second, independent guard, and the `rows_affected()==0` re-read at `:1018-1036` distinguishes expired from applied. Untested (M4) but correct.
- **Expiry predicates are consistent.** `approve` puts `expires_at > now()` in the same UPDATE (`staging.rs:683`); `apply` checks `expires_at <= Utc::now()` (`:996`) and repeats `expires_at > now()` in the flip (`:1012`); `stage` sweeps before counting (`:585-601`), so the cap measures live rows. `staging_test.rs:525-549` and `apply_test.rs:123-141` cover both directions.
- **`is_canonical_decimal` accepts nothing `Decimal::from_str_exact` rejects.** 28 total digits max ⇒ ≤ 9.99e27, inside rust_decimal's 7.92e28. The reverse (leading zeros counted against the cap, e.g. 32 zero-padded digits for `1`) is real but not producible by any client in this repo.
- **`mount.rs` header handling is correct.** `authorization`, `cookie`, `host`, `content-length`, `transfer-encoding` are all dropped before rmcp (`:235-241`); the bearer is consumed at `:186` and never forwarded; `MAX_BODY_BYTES` is enforced on `Data` before rmcp sees a byte (`:196-203`), and `mcp_mount_test.rs:600-655` proves the >8 KiB path with an asserted size.
- **`0006`'s audit predicate, index expression and lookup expression all agree on `upper(symbol)`.** Migration `0006:41-63` groups on `upper(symbol)`; index at `:113`; lookups at `services/assets.rs:57` and `staging.rs:1241` both use `upper(symbol) = $2`. `audit_asset_symbols.sh:60-73` is the same predicate. No data rewrite, and the `historical_prices` reasoning for that is sound.
- **`tools.rs::exact_date` cannot panic on short input** — `bytes.len() != 10` short-circuits the `||` before `digits` indexes (`tools.rs:104-110`).
- **Per-tool scope gates are present on all eleven tools** and every caller-supplied id goes through `access.rs` (`tools.rs:199, 434, 480, 559, 640`). `get_staged` on `Scope::Import` is defensible and pinned by `mcp_mount_test.rs:361-385`, as the PR body says.
- **Declared out-of-scope items hold.** Nothing in the tree ships backup/restore of staged batches, multi-currency, an `isin` index, or cluster config; `audit_asset_symbols.sh:112-127` explicitly reports ISIN collisions without constraining them.
- `cargo_test.log` shows every suite `test result: ok`, 0 failed — the PR body's test claim is consistent with the supplied log.

## Unresolved / needs operator decision

- **Whether `0005`'s non-atomicity warning should be deleted or re-scoped.** The claim is false under `sqlx::migrate!`, but an operator running the two statements by hand in `psql` without `BEGIN` *would* open the window. Settled by deciding whether hand-running migrations is a supported path; if not, delete the warning rather than hedge it.
- **Whether B1 should be fixed by deleting the bundle-side detector or by reordering preview.** Deleting matches the commit's own stated principle; reordering (render the plan, then validate) changes the preview route's contract (today a bad bundle is a 422 with paths, never a plan). Operator call.