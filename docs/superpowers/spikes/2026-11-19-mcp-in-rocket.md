# Spike A result — serving MCP from inside Rocket

**Date:** 2026-11-19
**Branch:** `spike/mcp-in-rocket`, worktree `/home/coder/wt-pp-mcp-spike`, commit `f167dfb`
**Base:** cached `origin/main` at `b321746` (no fetch was possible; this is the last-fetched state)
**Question:** can rmcp's `StreamableHttpService` be mounted as an ordinary Rocket route in
stateless mode, so the MCP design does not need a second in-process listener?

## Verdict

**Yes.** A Rocket route served the full MCP exchange — `initialize` → `tools/list` →
`tools/call` — statelessly, with a plain JSON body and no session id. The second listener is
not required by the protocol; it was a solution to a problem rmcp does not actually create.

The adapter is about 120 lines including comments, in one file: a request guard that captures
method, URI and headers; the buffered body wrapped as `http::Request<Full<Bytes>>`;
`StreamableHttpService::handle()`; and the response copied back into a `rocket::Response`.

The reason it is this small: `handle()` is generic over the request body
(`RequestBody: http_body::Body`) and returns `Response<BoxBody<Bytes, Infallible>>`. It is not
axum-specific, despite rmcp's own dev-dependencies pulling axum. **axum never enters the
build** — dev-dependencies of a dependency are not resolved downstream, verified with
`cargo tree -p rmcp -e normal | grep axum` → zero hits.

## What was measured, not assumed

| Question the spec left open | Answer |
|---|---|
| Does the mount work at all? | Yes. `tests/agent_spike_test.rs` passes: 200, `application/json`, no `Mcp-Session-Id`, tool executed and returned `pong`. |
| Is `tower ≥ 0.6` required? | **No — `tower` is not required at all.** rmcp needs `tower-service 0.3.3`, already in the lock. The spec's worry was about a crate it does not use. |
| Does `schemars` pull a third copy? | No. It resolves to the `1.2.3` already in the lock. `cargo tree -d -e normal` reports **no duplicate versions** after adding rmcp. |
| What does rmcp actually add? | **Eight new crates in `Cargo.lock`**: rmcp, rmcp-macros, chacha20, rand, pastey, sse-stream, unicode-general-category, futures-macro. `cargo tree -p rmcp -e normal --depth 1` lists 29 direct dependencies with the server features enabled (15 non-optional in its manifest); everything else was already in the lock. |
| What must the manifest gain for the bridge? | `http`, `http-body-util`, `bytes`, `tower-service` — all small, all already in the lock at compatible versions. "rmcp is the only new dependency" was false; "rmcp plus four small already-present crates" is true. |
| Is there a body cap? | Built in: `max_request_body_bytes` (default 4 MiB). The spec's 1 MiB is a config line, not code. |

## Traps found by running it, not by reading it

1. **`legacy_session_mode` defaults to `true`.** With `NeverSessionManager`, a client
   negotiating any pre-`2026-07-28` protocol version is routed to the session path and the
   request fails `500 — Session management is not supported`. Stateless mode must be opted
   into with `legacy_session_mode = false`. The spec's "stateless by default" was true only
   for the newest protocol version and wrong as a general statement.
2. **`#[tool_router]` alone yields an empty tool list with a 200.** The `ServerHandler` impl
   also needs `#[tool_handler]`. This fails silently — `tools/list` returns `{"tools":[]}` and
   every status code is fine. It needs an assertion on tool names, not on status.
3. **`StreamableHttpServerConfig` is `#[non_exhaustive]`** — it cannot be built with a struct
   literal, only by mutating `default()`.
4. **`rocket::Response::set_header` replaces rather than appends.** The naive adapter collapses
   repeated response headers. Harmless for MCP responses today; a real adapter should add.
5. **Rocket rejects a request with no `Host` before rmcp sees it**, and its local test client
   sends none. A test that forgets this reports a confusing `400 Bad Request: missing Host
   header` that looks like an rmcp rejection.
6. **Rocket 0.5 has no `local` feature** — the local client is always compiled. Attempting to
   enable it fails resolution with a list of the real features.

## What this removes from the design

If adopted: the second listener and its bind logic, the direct `hyper` / `hyper-util`
manifest entries, the k8s exposure work (new containerPort, Service port, ingress path, TLS
decision), the open question of what a bind failure should do, and the `replicas: 1` session
coupling — there is no session to keep sticky.

## What this does not prove

- **It used Rocket's local client, not a socket.** The bridge, the protocol handling and the
  stateless behaviour are proven; real-socket concerns (chunked bodies, keep-alive, read
  timeouts, proxy buffering in front of nginx) are not.
- **It did not use pi.** The client was a hand-written JSON-RPC sequence. pi's own behaviour —
  including the 20 KB truncation the spec now budgets against — is still untested against this
  server.
- **No auth.** The spike route has no PAT check. Under the mount design the route sits inside
  Rocket's fairing reach, and `AuthFairing` protects by the `/api/` prefix, so a `/mcp` path is
  **unauthenticated by default** and `tests/route_coverage_test.rs::PUBLIC_NAMESPACES` must be
  amended deliberately, with the route doing its own bearer check.
- **No database.** The spike tool touches nothing. Pool interaction, the 30-second deadline and
  `catch_unwind` are untested.
- The route-coverage gate tests could not run here: they require `DATABASE_URL`, which was not
  started. The static gates that do not need it (`access_gate_test`, 8 tests) pass with rmcp
  added.

## Decision — recorded 2026-11-19

**Adopted.** The spec now describes the mount: `agent/mount.rs` instead of a listener, exposure
as an nginx location beside `/api`, no session state, and the bearer-token check moved into the
route with a required test on both sides. The second listener is recorded in the spec's
*Alternatives considered* as proven viable and declined, so nobody rediscovers it and re-litigates
it.

The honest cost, restated: the mount moves the bearer-token check from a socket that structurally
cannot accept anything else into a route that must remember to check. The half of the separation
that mattered most — a PAT cannot touch the interactive `/api` surface, because those handlers
never read an `Authorization` header — is untouched by the transport choice.
