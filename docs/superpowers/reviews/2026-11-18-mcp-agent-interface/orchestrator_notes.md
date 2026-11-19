# Orchestrator verification notes

Evidence gathered by the orchestrator outside the review stages, for use by the synthesizer.
These are not findings; they are checks on claims the stages raised and could not settle.

---

## ON-1 — rmcp stateless mode: the unconsidered alternative is real and better-fitting than the spec assumes

**Raised by:** Stage 7 design axis (Alternative 1, unconsidered). Stage 8 was asked to rule on it.
**Status:** verified against primary sources on 2026-11-18.

### What the spec says

`2026-11-18-mcp-agent-interface-design.md:101-103` rejects mounting inside Rocket:

> Rejected: a Rocket↔tower request adapter (hand-rolling SSE through a Rocket `Responder`
> fights the framework) and hand-writing the streamable-HTTP transport on rmcp's `Transport`
> trait (would mean owning protocol negotiation, session ids and SSE).

`:83` adds a non-goal — "**No multi-replica session sharing.** The session manager is
in-memory" — and `:424-427` makes `replicas: 1` the condition under which in-memory sessions
are tolerable.

### What the sources say

1. **`2026-07-28` is stateless by default.** rmcp's README: "Per SEP-2567, `rmcp` serves the
   `2026-07-28` draft statelessly **automatically**: no `Mcp-Session-Id`, no standalone
   GET/DELETE stream, and no `Last-Event-ID` resumption. The `legacy_session_mode` flag below
   only controls behavior for *legacy* protocol versions (`< 2026-07-28`)."
   (`github.com/modelcontextprotocol/rust-sdk` README; same wording in
   `docs.rs/rmcp/latest/rmcp/transport/streamable_http_server/tower/struct.StreamableHttpServerConfig.html`.)

2. **`json_response` removes SSE from the response path.** `StreamableHttpServerConfig::json_response`:
   "When true and `legacy_session_mode` is false, the server prefers
   `Content-Type: application/json` for simple request-response tools. If the handler emits a
   notification or request before the final response, the server falls back to
   `text/event-stream` so no message is lost." Added in rust-sdk PR #683. Every tool in the
   spec's inventory is a plain request/response tool, so the fallback never triggers.

3. **In stateless mode the endpoint is POST-only.** `tower.rs` source:
   `let allowed_methods = match (self.config.legacy_session_mode, supports_stateless_replay) { (true, _) => "GET, POST, DELETE", (false, true) => "GET, POST", (false, false) => "POST" };`
   (`docs.rs/rmcp/latest/src/rmcp/transport/streamable_http_server/tower.rs.html`).
   There is no long-lived connection to buffer at the proxy.

4. **`NeverSessionManager` exists** — "disables sessions entirely (stateless mode)"
   (`StreamableHttpService` docs, *Session management*).

5. **Even for legacy clients, in-process sessions are not the only option.**
   `StreamableHttpServerConfig::session_store: Option<Arc<...>>` — "Optional external session
   store for cross-instance recovery … if an entry is found the session is transparently
   restored so the client does not need to re-initialize." The spec's `replicas: 1`
   precondition does not account for this field.

6. **Third-party corroboration that stateless is the practical default.** A production MCP
   server's ADR (ADR-0067, `pghale.com/decisions/stateless-streamable-http-mcp`, 2026-04-12,
   ACCEPTED) reports that stateful sessions pinned with a Traefik sticky cookie failed because
   "the real MCP clients we tested, Claude Code and Claude Desktop, don't persist cookies",
   and settled on stateless streamable HTTP with JSON responses. Portfolio Prism's cluster
   ingress is Traefik, so the same client behaviour applies.

### What this changes, and what it does not

If the agent listener runs rmcp with `legacy_session_mode(false)` + `json_response(true)`
against `2026-07-28` clients, then the transport is one POST in, one JSON body out. That
removes the premise of the rejection at `:101` (there is no SSE to hand-roll through a
`Responder`), and it removes:

- the streaming/SSE-through-proxy concern;
- the `replicas: 1` session-stickiness precondition (`:424-427`, SY-15);
- the "no multi-replica session sharing" non-goal (`:83`);
- the argument that a restart loses transport state — `:426` already argues Prism's semantic
  state is all in Postgres, which is exactly the precondition stateless mode requires. The
  spec is arguing for a design it already satisfies the requirements of.

It does **not** remove, because these follow from *in-process*, not from the listener:

- credential separation (PAT-only vs session-only still has to be enforced somewhere);
- Postgres pool contention (`db.rs:20` `max_connections(5)`);
- CPU (`500m`) and memory (`256Mi`) limits shared with the interactive app;
- panic isolation and the liveness-probe coupling;
- the probes in `k8s/app-deployment.yaml:72-77` targeting one port.

**Not yet verified:** whether `StreamableHttpService` can be driven without `hyper`. The
documented paths are `TowerToHyperService` (needs a hyper server) and `axum::nest_service`.
Calling `tower::Service::call` directly from a Rocket route is plausible for a single
POST/JSON exchange but the body-type bridge (`Rocket` body → `http_body::Body`) is unbuilt
work, and Rocket 0.5 is not tower-based. Note also that in stateless mode `service_factory`
runs per request, which is fine here for the same Postgres-state reason.

**Recommendation for the spec:** do not adopt on this evidence, and do not keep the current
rejection wording either. Add a pre-Workstream-2 spike: stand up a Rocket route that bridges
one POST to `StreamableHttpService` with `legacy_session_mode(false)` and
`json_response(true)`, and drive it with rmcp's own client. If it works, Workstream 2 loses
the second listener, the second port, the ingress question (SY-16), and the session section.
If it fails, rewrite `:101-103` to name the actual failure instead of SSE. Either outcome is
better than the current text, which rejects an option by describing a mode the SDK does not
require.

---

## ON-2 — SY-4 (`serde(rename_all = "uppercase")`) confirmed at the source

`serde_derive-1.0.229/src/internals/case.rs:33-53`: `RENAME_RULES` is
`lowercase`, `UPPERCASE`, `PascalCase`, `camelCase`, `snake_case`, `SCREAMING_SNAKE_CASE`,
`kebab-case`, `SCREAMING-KEBAB-CASE`, and `RenameRule::from_str` compares with `==`
(case-sensitive exact match) before returning `ParseError`. `rename_all = "uppercase"` is
therefore a compile error, not a silent no-op. SY-4 stands as a blocker.

---

## ON-3 — standing corrections to stage findings

Recorded during Stage 7 adjudication, repeated here so the synthesizer does not reintroduce them:

- **ST-9 (entropy) is incorrect.** 32 random bytes is 256 bits regardless of the encoding
  used to print them; base62 does not reduce entropy, it only changes the encoded length
  (~43 characters). The actionable part is SY-9's wording fix, not ST-9's claim.
- **ST-8 is accepted in part.** The `tower 0.6.11` claim is false (SY-1, ON-2 style evidence
  below), but the framing "direct contradiction" overstates it: asserting `hyper 1.10.1` is
  present and asking whether `rmcp` needs a newer `hyper` are different claims.
- **CP-12 / SY-12 are the same finding** (undefined anomaly flag "unusual price scale").
- **`docs/backup-schema/v1.json:134-142` already enumerates `BUY|SELL|SPLIT`**, so the
  Workstream 0 item that asked for it is already done; the spec's item should be reworded to
  say it is verifying an existing constraint, not adding one.
