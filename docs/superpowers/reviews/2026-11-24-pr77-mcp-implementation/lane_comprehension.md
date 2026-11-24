Report finalized below from already-verified reads (no new files opened). Ordered to lead with prose-teaches-what-code-doesn't-do findings.

# Comprehension audit — PR #77 (portfolio-prism), `/home/coder/wt-pp-staging` @ `df258e4`

**Read:** `pr_body.md`, `brief.md`, `commits.txt`; module docs + code of `agent/{mod,mount,protocol,staging,tools}.rs`, `auth/{pat,fairing,guard}.rs`, `access.rs`, `lib.rs`, `db_types.rs`, `backup/detail.rs`, `services/currency_service.rs`; migrations `0002`–`0006`; `scripts/audit_asset_symbols.sh`, `check_access_gate.sh`; tests `mcp_mount_test.rs`, `staging_test.rs`, `apply_test.rs`, `route_coverage_test.rs`; `openapi.json`; frontend panel + test.

## Prose-vs-code headline (the axis answer)

The PR body and three code comments teach four things the code does not do: (1) that `AuthFairing` accepts tokens — it never reads `Authorization`; (2) that every tool shares the `/api` route's function — three don't, and `tools.rs` itself says so; (3) that `pat::lookup` checks expiry against the database clock — it uses the app clock; (4) that moving `/mcp` under `/api` would silently grant session holders every tool — the route's unconditional bearer check forecloses that. Plus the body describes a stale tree (21 commits vs 23; two shipped commits undocumented).

## Paraphrase

**What this proposes.** PATs an agent can hold (`read`/`import` scopes, SHA-256-hashed, mint/list/revoke under `/api/tokens`); a staging area where a token writes a *proposal* (`agent_staged_batch` with stored payload + preview under a random handle) that only a session can approve; an MCP server at `/mcp` inside Rocket with eleven tools that self-authenticate by bearer. Four prerequisites ride along (one insert path, frontend decimal strings, closed transaction types, strict money JSON); migrations `0002`–`0006` close type/identity gaps, each auditing live data before raising.

**How the existing system works, per this document.** Rocket serves `/api/*` behind `AuthFairing` (prefix rule, session cookie only); `access.rs` is the per-id chokepoint enforced by a mutating gate script. The core control is the write/decision split: agent stages, human approves via session-only routes, and `apply` — the only writer — flips `approved → applied` exactly once in one transaction. Splits dedupe only via a unique partial index; asset identity was software-only until `0006`.

## Blockers

**C-1 · Blocker · pr_body §WS1** — "`AuthFairing` accepts either [token or session]" is false. `resolve_caller` reads only `session::read(req.cookies())` (`auth/guard.rs:128-163`); `guard.rs:232-234`: "An `Authorization` header is never consulted, only ignored"; `tools.rs:29-33`: "a token cannot reach `/api` at all." Reader concludes a scoped token can call `/api` — the opposite of the design that makes the tokens routes safe.

**C-2 · Blocker · pr_body §"The eleven tools"** — "Every tool calls the same function the corresponding `/api` route calls" is false as stated. `tools.rs:8-16` (same PR) names the gap: `list_portfolios`, `get_portfolio`, `list_transactions` "inline their own `SELECT`". The body sells as fact what the module doc calls "the gap this module has not closed."

**C-3 · Blocker · pr_body header + §Review round** — body describes a different tree: "21 commits, 79 files, +10718/−312" (tree: 23/81/+11225/−318 per brief); "The last three commits are a review round" lists `250ccdf·e4bde8c·2692a69`, but head is `df258e4` and `37bcdd7` ("a body cap that was never wired, and an approval that could never apply") and `df258e4` appear nowhere in the body. Two shipped commits are undocumented in the artifact being approved.

## Majors

**C-4 · Major · `auth/pat.rs:115-121` vs `:314`** — doc: "the thing that refuses an expired token is `lookup`, which compares against `now()` in the database … One clock, one answer." Code: `exp <= Utc::now()` — the app clock (`pat.rs:314`); only the list query uses DB `now()`. `staging.rs:1022-1024` explicitly acknowledges the two-clock situation exists. The comment asserts an invariant the code contradicts, in the module whose doc is about clock honesty.

**C-5 · Major · pr_body §Access control** — "`check_access_gate.sh` fails the build if an `/api` data route … calls no `require*`" overstates. The script's own header (`check_access_gate.sh:20-44`): "heuristics and stay heuristics"; Rule A is file-level; live counterexample in-tree — `preview` and `restore` "call no `require`, and clear Rule A through `export`'s call." Body converts a self-declared heuristic into a per-route guarantee.

**C-6 · Major · `mount.rs` module doc l.12-18 + `lib.rs:358-365`** — failure story "mounts `/mcp` under `/api` … every tool would run as whoever holds a session cookie" cannot happen while the handler unconditionally requires a bearer (`mount.rs:186`: `parts.bearer.ok_or(Status::Unauthorized)?`). Under `/api` the mount would break (both credentials demanded), not silently grant. The doc contradicts its own later sentence ("a bearer token is required and nothing else is examined") and teaches that the prefix, not the route's check, is the security boundary.

**C-7 · Major · `services/currency_service.rs:109-112`** — `apply_split_events` doc: dedupe "enforced by `uq_split_asset_date`, so a concurrent writer cannot slip a second row in between **a check** and this insert." Migration `0005`: "apply_split_events … has no read-based check of its own — the count-then-insert it replaced let two concurrent callers both see zero." Comment still describes the deleted shape, on the one dedupe the split path rests on.

## Nits

**C-8 · Nit · pr_body §Staging lifecycle** — "The preview is stored, not derived." It is derived at stage time (`staging.rs:577` `render_preview`); `staging.rs:117` says "the preview is derived from the document." Intent: "not re-derived at render time."

**C-9 · Nit · `staging.rs:116-118` vs `:579-583`** — the `what:"preview"` refusal is described as live ("arrives here from `stage`") and dead ("Unreachable for a payload that passed `validate`"); no sentence marks it a tripwire.

**C-10 · Nit · pr_body throughout** — "gate" carries three senses unmarked: approval gate (batch status), access gate (`check_access_gate.sh`), `gate()` scope fn (`tools.rs:145`).

**C-11 · Nit · undefined terms** — "pi" (`staging.rs:50`) defined only in spec, unlinked; "Task 6"/"Task 18" (`guard.rs:126`, `fairing.rs:37`) unlinked spec numbers; "TWR" in the agent-facing `get_performance` description (`tools.rs:559`); `pat.rs:288-290` says the MCP mount "lives in Workstream 2" in future tense though WS2 ships in this PR.

**C-12 · Nit · `mount.rs:3-5`** — cites spike branch+commit `f167dfb` a fresh clone may lack; in-tree `docs/superpowers/spikes/2026-11-19-mcp-in-rocket.md` unlinked.

**C-13 · Nit · pr_body §`/mcp` mount** — `SELF_AUTHENTICATED` / `PUBLIC_NAMESPACES` named without location (`tests/route_coverage_test.rs:207,223`).

**C-14 · Nit · pr_body §Deploy order** — audit script described as two exit codes; script has three (`audit_asset_symbols.sh:29-32`: `2` = could not run).

## Refutations and accepted risk (verified true — not findings)

`fairing.rs:40` and `db_types.rs:139-141` citations exact; eleven tools match; `gate()` returns `Option<CallToolResult>` (`tools.rs:145`); `authorization`+`cookie` stripped before rmcp (`mount.rs:222-234`); `legacy_session_mode=false` set; named tests all exist and can fail (`mcp_mount_test.rs:118,352,515-519`, `route_coverage_test.rs:199`, `apply_test.rs:471`, `staging_test.rs:700-704`); approve expiry predicate `staging.rs:683`, advisory lock `:597`, portfolio cap `:568`/`:1167`, single-use flip `:1012`; `Credential::Session` passes every scope (`access.rs:313-314`); tokens survive `logout-all`; openapi 23 paths, `/mcp` absent; `backup::detail` caps 25/50 as claimed; migrations' audit-first prose matches files; `AGENTS.md:133+` roadmap note exists.

## Unresolved / not reached

- **Verification numbers** (664/361 tests, six gates, clippy, `sqlx prepare --check`, "eight CI checks") — no shell in this role; unverified. Settle by CI on `df258e4`.
- **`replicas: 1` + `Recreate`** premise for 0005/0006 safety — lives in `k8s-titan`, out of scope; body itself says confirm on the live deployment.
- **"Sixteen findings; all addressed"** — count not checkable; section predates `37bcdd7`/`df258e4` (see C-3).
- **Not reached before cutoff:** frontend `parseDecimal`/Access-screen claims beyond the panel test; `openapi.rs` route inventory; `staging.rs` `resolve_asset`/`resolve_portfolio` internals (USD-literal fix taken on the strength of `apply_test.rs:471`); `0003` FK claims beyond the migration's own text.