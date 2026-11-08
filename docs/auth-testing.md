# Testing the authentication and isolation work

Who this is for: whoever has to convince themselves this is safe to deploy. The
implementation record says what was built; this says how to check it, in increasing
order of effort, and is explicit about which checks are automated and which are not.

Two things in here are **not verified by anything in CI**, and they are the reason this
document exists:

1. Nothing in this repository has ever completed a login against a real identity
   provider. Every OIDC test exercises the failure path.
2. Whether the provider ends its own session when this app logs out is unknown, and
   it is provider-behaviour, not our code.

> **Shell.** These examples are for **fish**, the operator's interactive shell. The
> `set -x VAR value` form is required — `VAR=value command` does not assign in fish and
> fails in a way that looks like the command failing. A bash variant is given where the
> difference matters.

---

## 0. The whole gate set, in one place

Run from the repository root. All of these are expected to pass on
`feat/oidc-identity-isolation`.

```fish
# Backend: 244 tests
cd backend_rust; and cargo test

# Static access gate: prints "handlers inspected: 15, cleared: 15"
bash scripts/check_access_gate.sh

# The gate's own self-test (it is part of `cargo test`, listed here for clarity)
cargo test --test access_gate_test

# Lints and the release build
cargo clippy --all-targets
cargo build --release

# The release binary must refuse the development mode. Run it; CI does.
set -x AUTH_MODE disabled
./target/release/backend_rust; echo "exit=$status"   # expect exit 1, names AUTH_MODE
set -e AUTH_MODE

# OpenAPI document must match the code
cargo run --quiet -- --print-openapi > /tmp/oa.json
diff <(python3 -m json.tool /tmp/oa.json) <(python3 -m json.tool ../docs/openapi/openapi.json)
```

```fish
# Frontend: 152 tests, types, lint
cd ../frontend
npm test
npx tsc --noEmit
npm run lint        # 0 errors; one pre-existing react-hooks warning is expected
```

`cargo test` covers the isolation matrix, the auth cookie paths, the access chokepoint,
startup refusals, route coverage, and the gate script itself. None of it needs a
network or a provider.

---

## 1. What is missing, and why a provider is the only way to close it

| Question | Automated? | Where it is answered |
|---|---|---|
| Does discovery resolve against a real provider? | yes | §4 |
| Is the authorization request well-formed (PKCE, state, nonce)? | yes | §4 |
| Do the failure paths answer the way the frontend expects? | yes | §4 |
| Does a real login provision a user and produce a working session? | answered: **yes** | §5 |
| Does `logout` end the provider's session? | **no** | §5, browser |
| Does per-user isolation hold against real data? | partially | §6 |

`backend_rust/scripts/auth_smoke.sh` is the automated half. It was run against a live
authentik instance and passes 20/20; its output is in §4.

---

## 2. Create the OIDC client in authentik

**No `portfolio-prism` provider exists yet.** Measured from this workspace:

```
404  https://auth.titan.arrieta.eu/application/o/portfolio-prism/.well-known/openid-configuration
200  https://auth.titan.arrieta.eu/application/o/mealie/.well-known/openid-configuration
404  https://auth.titan.arrieta.eu/.well-known/openid-configuration
```

The last line matters: authentik has **no global** discovery document. The issuer is
per-provider, `https://auth.titan.arrieta.eu/application/o/<slug>/`. Composing
`.../o/<slug>/authorize/` yourself is wrong — the discovery document's
`authorization_endpoint` is the global `/application/o/authorize/` while
`issuer` and `end_session_endpoint` are per-provider. The backend reads the document
rather than composing endpoints, so this is only a hazard if you hand-write a URL.

### The declarative way (matches how every other app in titan does it)

`k8s-titan/authentik/` holds one blueprint per app — `blueprints-mealie.yaml`,
`blueprints-coder.yaml`, `blueprints-grafana.yaml` — and there is a draft for this app
at `docs/authentik-blueprint-portfolio-prism.yaml`. Following the existing pattern
means:

- the blueprint is a **template**; a `scripts/setup-<app>-secrets.sh` substitutes the
  client id and secret and writes the rendered file, sops-encrypted, to
  `apply/10-secrets/`;
- Flux applies it, and **authentik decides when it lands** — `k8s-titan/docs/authentik-runbook.md`
  §5 records that a blueprint can be missed by the pass that should have caught it, and
  that the beat recovers on its own. Do not conclude it failed because the first
  discovery probe 404s;
- provider settings that this app cares about:
  - `issuer_mode: per_provider` — that is what makes the issuer carry the slug,
  - `sub_mode: hashed_user_id` — stable across renames, and it is the value
    `BOOTSTRAP_OWNER_SUBJ` must be set to (§6),
  - `redirect_uris` with `matching_mode: strict` — the port is part of the value,
  - `signing_key` and an `authorization_flow` with implicit consent, same as the others,
  - an owning `authentik_core.application` — a provider with no application is invisible
    to consumers, which is a recorded lesson in that repo.

### The interactive way (fine for a first test, not for keeping)

authentik admin → Applications → create provider (type *OAuth2 Provider*,
`confidential`, `Authorization Code + PKCE`), then create the application and bind the
provider. Register the redirect URI exactly as you will use it, including the port.

**One authentik quirk to know about, even though it does not bite this app:** the managed
email property mapping emits `"email_verified": false`, which is why coder and Mealie
both carry an explicit opt-out. This backend reads `email` but never checks
`email_verified`, so there is nothing to opt out of — but if a future change starts
trusting that claim, that is the trap.

---

## 3. The environment the app needs

Six variables, and the backend refuses to start if any of the first four is missing.

| Variable | Notes |
|---|---|
| `AUTH_MODE` | `oidc`. `disabled` is compiled out of the release binary on purpose. |
| `OIDC_ISSUER` | per-provider URL, **with the trailing slash**. |
| `OIDC_CLIENT_ID` | from the provider. |
| `OIDC_CLIENT_SECRET` | from the provider. |
| `OIDC_REDIRECT_URI` | registered with the provider, character for character. `http` is accepted only for `localhost`/`127.0.0.1`; anything else must be `https`, because the session cookie is `Secure` unconditionally. |
| `ROCKET_SECRET_KEY` | `openssl rand -base64 32`. Must be stable across restarts or every session dies. |
| `ALLOWED_ORIGINS` | the origin the browser uses. Wrong value = silent `403` on writes, because CORS answers before authentication. |
| `BOOTSTRAP_OWNER_SUBJ` | leave empty unless §6 applies. |

---

## 4. The automated provider check — no browser

From `backend_rust/`:

```fish
set -x OIDC_ISSUER "https://auth.titan.arrieta.eu/application/o/portfolio-prism/"
set -x OIDC_CLIENT_ID portfolio-prism
set -x OIDC_CLIENT_SECRET whatever-not-used
set -x OIDC_REDIRECT_URI http://localhost:8000/auth/callback
bash scripts/auth_smoke.sh
```

It starts its own backend on a throwaway SQLite file, so it does not touch your data.
What it checks, and what it looked like when run against a live authentik (using
another app's issuer as a stand-in, since this app's provider does not exist yet):

```
== 1. provider discovery ==
        issuer   : https://auth.titan.arrieta.eu/application/o/mealie/
        authorize: https://auth.titan.arrieta.eu/application/o/authorize/
        token    : https://auth.titan.arrieta.eu/application/o/token/
        end_ses. : https://auth.titan.arrieta.eu/application/o/mealie/end-session/
  PASS  discovery reachable and its issuer matches OIDC_ISSUER
  PASS  provider advertises an end_session_endpoint (logout can reach it)
== 2. start the backend in oidc mode ==
  PASS  backend started in oidc mode
== 3. unauthenticated data access ==
  PASS  GET /api/portfolios/ -> 401 unauthenticated
  PASS  POST /api/portfolios/ -> 401 unauthenticated
  PASS  DELETE /api/assets/1 -> 401 unauthenticated
  PASS  PATCH /api/portfolios/1 -> 401 unauthenticated
== 4. public surface ==
  PASS  GET / -> 200 (probes and the SPA entry stay reachable)
  PASS  GET /api-docs/openapi.json -> 200
== 5. the authorization request ==
  PASS  /auth/login redirects
  PASS  redirect target is the provider's authorization endpoint
  PASS  authorize URL carries client_id / code_challenge / code_challenge_method
        / state / nonce / redirect_uri
  PASS  state cookie set on the way out
== 6. the failure path ==
  PASS  browser callback failure -> redirect to /?auth_error=auth_failed
  PASS  JSON callback failure -> 401 authentication_failed

automated: 20 passed, 0 failed
```

Check 6 is the one that earns its place: it follows the authorize URL to the provider and
reads the answer. It caught a real misconfiguration on first use — the provider's **slug**
is what appears in the issuer URL, while its **client ID** is a separate field that
authentik fills with a random string unless you overwrite it. From the app's side that
failure is invisible: `/auth/login` succeeds, the browser gets a well-formed authorize
URL, and the provider shows an error page the app never sees.

Note what check 7 proves: the two callback failure shapes are **different on purpose**
— a browser gets a redirect to a banner, a script gets a status it can test. A test
that accepted either would pass on a build that got the branch backwards.

---

## 5. The browser walkthrough — this is the part that closes the two open items

Either bring up the compose harness or run locally.

### Option A — compose (closest to production shape)

```fish
cp .auth-test.env.example .auth-test.env
# fill in OIDC_ISSUER, OIDC_CLIENT_ID, OIDC_CLIENT_SECRET, ROCKET_SECRET_KEY
docker compose -f docker-compose.auth-test.yml --env-file .auth-test.env up --build
```

Then open **http://localhost:8080/auth/login**.

The frontend shares the backend's network namespace on purpose — `nginx.conf` proxies
to `127.0.0.1:8000` because in production the two containers are sidecars in one pod.
**This compose file has never been run**: the machine it was written on has no docker.
The backend and the smoke script are tested; the orchestration is not. If a build
fails, look at the Dockerfiles first — they are unchanged by this branch and build in
CI.

### Option B — no containers

```fish
cd backend_rust
set -x AUTH_MODE oidc
set -x OIDC_ISSUER "https://auth.titan.arrieta.eu/application/o/portfolio-prism/"
set -x OIDC_CLIENT_ID portfolio-prism
set -x OIDC_CLIENT_SECRET (read -s)
set -x OIDC_REDIRECT_URI http://localhost:8000/auth/callback
set -x ALLOWED_ORIGINS http://localhost:5173
set -x ROCKET_SECRET_KEY (openssl rand -base64 32)
cargo run
# in another terminal
cd ../frontend; and npm run dev
```

Open **http://localhost:5173** — vite proxies both `/api` and `/auth`.

### The checklist

1. `/auth/login` → provider's login page. If it errors at the provider, the redirect
   URI you registered and the one you configured differ; compare them character by
   character.
2. Sign in. You land on the app with no `?auth_error=` in the URL.
3. `http://localhost:8000/auth/me` → your `sub` and email. **Record the `sub`** — you
   need it for §6.
4. `http://localhost:8000/api/portfolios/` → `[]`, not `401`.
5. Create a portfolio, add an asset. If writes fail with **403 and no redirect**, that
   is `ALLOWED_ORIGINS`, not authentication.
6. Sign out from the sidebar link.

### The logout question (the one genuinely open item)

`/auth/logout` clears the local cookie and redirects to the provider's
`end_session_endpoint` **without** an `id_token_hint`, because the ID token is not
retained. Per RP-initiated logout 1.0, without a hint the provider **MUST NOT**
perform the post-logout redirect silently — in practice that means a confirmation page,
not a failed logout. Whether authentik then ends its session is version-dependent and
has not been observed.

To settle it:

1. Sign in, then go to `/auth/logout`. Note what the provider shows and what you had to
   click.
2. Open a **new tab** and go to `/auth/login`.
   - **a credential prompt** → the provider session ended. The current design is
     correct and this closes.
   - **straight back into the app** → the provider session survived, and "log out" does
     not mean what a user expects on a shared browser. The fix is to keep the ID token
     server-side (encrypted, session-scoped) and send it as `id_token_hint`. Browser
     storage was considered and rejected — implementation record §10.

### ANSWERED — measured against the live Authentik, 2026-11-07

Authentik shows a logout-complete page: *"You've logged out of portfolio-prism"*, with
three choices — *Go back to overview*, *Log out of authentik*, *Log back into
portfolio-prism*. **"Log back into portfolio-prism" re-enters the app with no
credential prompt.**

So the provider session survives an app logout. The app session genuinely ends (cookie
cleared, `/api` answers `401`), but re-entry is one silent click away, and the page's
wording describes the application rather than the identity provider — which is exactly
why it reads as finished when it isn't.

Whether that matters is a deployment question, not a bug: on a single-user laptop it
costs nothing; on a shared browser it is a real exposure. The fix is a server-side ID
token store so the logout can carry `id_token_hint` — see implementation record §9.14
for the trade-off and what remains unmeasured about it.

**Dev note found on the way:** `post_logout_redirect_uri` is derived from the origin of
`OIDC_REDIRECT_URI`. If the backend runs on `:8000` while you browse via vite on
`:5173`, logout bounces you to the backend instead of the app, and Authentik may refuse
the unregistered URI. Set `OIDC_REDIRECT_URI=http://localhost:5173/auth/callback` so
login, callback and logout all stay on one origin.

Either answer is useful. Neither can be found from here.

---

## 6. `BOOTSTRAP_OWNER_SUBJ` — read this before setting it

With an existing database that has portfolios and no owner, the backend **refuses to
start** until you name who inherits them. That refusal is the feature.

The value is the provider's `sub` claim — with `sub_mode: hashed_user_id` that is a
hash, not your username. Get it by logging in once and reading `/auth/me` (step 3
above), *before* setting the variable.

**The transfer is one-way.** The first login whose `sub` matches takes the placeholder
row and everything it owns, and there is no admin surface to undo it. Set it to the
wrong value, let that person log in, and the recovery is hand-editing SQLite with the
pod scaled to zero. This was raised in review and accepted by the operator on the basis
that the operator is the first identity to log in.

Leave it **empty** to keep the refusal rather than guess. A placeholder string such as
`REPLACE_ME` counts as *set* and is worse than empty.

---

## 7. When it does not work

| Symptom | Cause |
|---|---|
| `404` on the discovery URL | the provider does not exist yet, or the issuer has no trailing slash, or you used the global `/.well-known/…` — authentik has none |
| `403` on writes, no redirect to login | `ALLOWED_ORIGINS` does not match the browser's origin. CORS answers before authentication |
| Logged in, then every request is `401`, cookie present in DevTools | you are not on `localhost`. The session cookie is `Secure` unconditionally; browsers drop it over plain http to an IP address |
| Session dies on restart | `ROCKET_SECRET_KEY` regenerated. It must be stable |
| `startup refused: AUTH_MODE=disabled is not available in a release build` | working as designed. Use a debug build for local development |
| `startup refused: OIDC_ISSUER requires…` | one of the four required variables is missing |
| Login loops back to the app with no session | the provider issued a token the app rejected — check the server log, which is where provider detail is deliberately kept |
| `state mismatch` at callback | two logins in flight in the same browser, or the cookie was dropped (see the `Secure` row) |

---

## 8. What this still does not prove

- **Concurrency.** Single-writer SQLite, `replicas: 1`. The read endpoints write
  (analytics inserts split rows on a `GET`), which is how `SQLITE_BUSY` becomes
  reachable — that is why a database failure now answers `503` instead of `401`.
- **Key rotation.** The JWKS cache has a 15-minute TTL, so a provider key rotation is
  self-healing within that window rather than an outage until restart. Not tested
  against a real rotation.
- **Anything at scale.** No load testing was done, and two `users` reads happen per
  request by design.
