# Testing the authentication and isolation work

Who this is for: whoever has to convince themselves this is safe to deploy. The
implementation record says what was built; this says how to check it, in increasing
order of effort, and is explicit about which checks are automated and which are not.

Two things about the authentication work in here are **not verified by anything in CI**, and
they are the reason this document exists:

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
# Backend: 313 tests
cd backend_rust; and cargo test

# Static access gate: prints "handlers inspected: 20, cleared: 20"
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
# Frontend: 203 tests, types, lint
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

Everything in this section is about authentication except the last subsection, *Printing the
statement*, which is the report-export feature's manual check and is listed here because this
is where the document keeps the checks CI cannot run.

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

### Printing the statement

`npm run dev`, open a portfolio, click **Printable Statement**, then Ctrl-P. Check:

- no sidebar, no nav, no Print button
- dark text on white, and the table's body rules visible (a half-inverted table means
  the `.statement-table td` override in `print.css` is missing)
- one page for a small portfolio
- the three headline figures equal the payload shown in devtools -- **not** "match the
  dashboard". `Dashboard.tsx:39-41` falls back to `metrics.*` when a tax-summary figure
  is falsy, so a coincidental fallback can satisfy an eyeball comparison.
- an empty portfolio prints a short note and no figures
- all nine columns are on the page — **and run this check on a wide portfolio, not the smallest one
  you have**: something carrying a large realized or market value, e.g. `-$123,456.00` or bigger.
  Shape, not digit count, decides the width. The recorded series measured a seven-digit-realized
  shape at 635px and an eight-digit-realized one at 608px — the bigger number came out the narrower
  table, because its other columns were smaller — and the smoke's own fixture carries an eight-figure
  realized value and fits 608px as it ships. So there is no digit count you can test against; it is
  the columns moving together that fills the page. On a small stock position the table fits with
  room to spare and the check proves nothing; that is
  exactly the blind check the smoke used to perform, and why its fixture was widened. The rightmost
  three — Realized, Unrealized, Unrealized ROI — are what a too-wide table loses first, and the
  clip is silent: `.main-content` computes `overflow-x: auto` (because it declares
  `overflow-y: auto`, `index.css:163`), so an over-wide table is cut at that container and at the
  page box with no scrollbar and no warning. Three things buy the width, all in `print.css`: the
  statement table's cells shrink to `4px 6px` at `0.75rem`, the Type column prints a short display
  label (`Fund`, not the stored `MUTUAL_FUND`), and `.main-content`'s 40px side padding is
  collapsed **for the statement tab only**. Measured in a browser at the printed page box, that
  collapse takes the box from 608px to 688px — and it is worth saying plainly that the collapse is
  **margin, not necessity**: with it deleted the smoke still passes all nine assertions on the
  shipped fixture, because the shortened Type label already bought the room. Leave the rule alone;
  just do not be afraid of it, and do not expect a failing check if something else takes that slack
  away. So what to look for: all nine headers with their figures under them, the rightmost figure
  sitting inside the paper with about the same gap as the
  leftmost one (the table now runs the full width of the text above it, because the side padding
  is gone), and nothing half-cut at either edge. A missing or clipped rightmost column means the
  table is over the page box — padding alone cannot fix that, so say so before you hand the page
  to an adviser.
- **nothing is broken across two lines — not a figure, not an asset-type label.** This is the
  failure the bullet above cannot see, so check it separately. The Type column prints a
  human-readable label — `Stock`, `Crypto`, `ETF`, `Fund` — never the database enum, so no cell
  should read as half a word. Any value split over two lines, a label or a number, means the table
  has been squeezed below what it needs — with one exception to rule out before you chase it: a value
  that merely contains a break opportunity, a space or a hyphen, splits there with no squeeze at all.
  An asset type the label map has never seen falls through to the raw string, and a hyphenated symbol
  like `BRK-B` breaks on the hyphen; both are content, not layout. Absent that, the columns are
  interdependent, so whichever column the browser can shrink gives way first. Look at the money
  columns, then Type. The smoke asserts this
  directly, by counting the line boxes of every table **body** cell; a header that wraps onto two
  lines is not itself a defect, because the header row is where that excess width is meant to land.
  Nothing in `print.css` may break a token mid-word to make a squeezed table fit — that rule used
  to exist on the Type column and is what turned an over-wide table into quietly unreadable type
  instead of a failing check.

Letter users change the paper size in the print dialog; `@page` says A4 and cannot be
scoped, so printing from any other tab now also gets A4 with 14mm margins. The print palette
cannot be scoped either: the other four tabs now print dark-on-white as well, which is an
improvement on the near-invisible white-on-white they used to print. Their layout is unchanged —
the hide rules are `:has()`-scoped, so their sidebar, nav and charts still print. One of those
backgrounds is a hardcoded dark literal the palette overrides cannot reach, so `print.css` forces
the sidebar white for print; if you enable background graphics and still get a dark-on-dark
sidebar on another tab, that rule is what went missing.

### Downloading the tax report

Still signed in through the provider. This is the step no gate can take: the download is a plain
`<a href>`, so the session cookie is what makes it work, and nothing in CI logs in.

1. Open a portfolio, click **Tax Report**. The window defaults to the current year to date, and
   the tab fetches as soon as it opens. If it opens on an error instead of a table, read the
   error panel before going further — the steps below assume a report is on screen.
2. Click **Download CSV**. The link is only there while a report is on screen: the route's error
   responses carry no `Content-Disposition`, so a link on a failing window would take the
   browser to a raw JSON body and throw away the app. If you do not see the button, the report
   did not load; that is the finding, not a missing control.
3. The file arrives as `tax-report-<id>-<from>-<to>.csv`, named by the server's
   `Content-Disposition` and not by the browser's guess at the URL. With `from` absent the name
   carries the portfolio's first transaction on or before `to`, not the literal `inception` — that
   appears only when the portfolio has no transaction at or before `to` at all.
4. Open it in a spreadsheet: row 1 is the header row, there is no `#` row above it, the file
   starts with a UTF-8 BOM (Excel needs it to pick UTF-8), and `splits_note` is a column. Check
   `lot_pnl` filtered to `SOLD_LOT` against `asset_realized_in_window` on the same rows — that is
   the reconciliation the file is for, and it is the **only** summable column. Summing
   `realized_pnl` instead counts a four-lot sell four times.
5. Get the request URL from the **link's `href`** (right-click **Download CSV** → Copy Link, or read
   it in devtools). The address bar will not have it: this app has no router and no deep link to a
   window, so the tab's URL is the app's, not the report's. Add `&format=csv` if it is not there,
   then load it in a shell that holds the session. `pp_session` (`auth/session.rs:19`) is HttpOnly,
   so it is not in `document.cookie` — read it from devtools' Application tab, or from the
   `Cookie:` header of a request the page already made:

   ```bash
   # the URL from step 5, not this example's id and dates
   curl --silent --show-error \
        --cookie "pp_session=<the value from devtools>" \
        "http://localhost:8000/api/portfolios/1/tax-summary?from=2024-01-01&to=2024-12-31&strategy=FIFO&threshold_days=30&format=csv" \
        -o /tmp/report.csv -D /tmp/report.headers
   ```

   The bytes must match the browser's exactly — compare `/tmp/report.csv` against the downloaded
   file, and confirm `/tmp/report.headers` carries `Content-Type: text/csv` and the
   `Content-Disposition` filename. Two runs over unchanged data must also be byte-identical to
   each other; there is no generated-at column, so a difference is a real difference.
6. **Now ask for a window that fails** — a `to` in the future, or a `strategy` that is not
   `FIFO`/`LIFO`/`HYBRID` — with the same `curl` minus the `-o` and `-D` flags, and look at what
   comes back. It is **not a file**: the status is 422 (or 503), the body is JSON with `error`,
   `missing_prices`, `missing_rates`, `missing_prices_total`, `missing_rates_total` and
   `retryable`, and there is **no `Content-Disposition` header at all**. That is why step 2's link
   is absent in an error state, and it is what a plain link click would have navigated the browser
   to. On screen the same failure is the error panel: the missing symbols and pairs with their
   omitted counts, and a **Try again** button only when `retryable` is true.
7. Print the report tab (Ctrl-P) and confirm it prints like the other four tabs — the whole
   shell, sidebar and nav included, no print stylesheet applied. The statement's hide rules are
   `:has()`-scoped (`.main-content:has([data-print="statement"]) > *:not([data-print="statement"])`,
   `print.css:26`) and match nothing on a page with no statement root, which is the intended
   behaviour here. If the report tab prints like the *statement* — no sidebar, no controls — a
   hide rule escaped its `:has()` scope.

Two things this walkthrough cannot check, and no automated check here does either: whether the
provider's answer for a pair that has no series is an error status or an empty chart, and
therefore whether a delisted currency pair reports 422 or 503 (see the release note — a 503 every
time is a possible and correct answer). And whether a genuinely cold multi-currency portfolio
exceeds the 20 s deadline on its first report; that path is only exercised at a zero deadline in
the tests.

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
