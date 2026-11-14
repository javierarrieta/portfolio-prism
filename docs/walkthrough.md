# Walkthrough: Portfolio Prism

We have successfully built and verified the full-stack **Portfolio Prism**. The application is structured with a separate Rust (Rocket) backend and a React (Vite + Vanilla CSS) frontend.

---

## 🛠️ What Was Built

### 1. Backend Service (Rust + Rocket)
- **Database Schema**: Utilizes SQLite via SQLx. The database caches daily price bars (`HistoricalPrice` table) for offline capabilities and rapid computations.
- **Tax Lot Accounting Engine (`tax_engine.rs`)**:
  - Implements **FIFO** (First-In, First-Out) matching.
  - Implements **LIFO** (Last-In, First-Out) matching.
  - Implements a **Hybrid** method (applies LIFO matching on lots bought within a configurable window, e.g., 30 days, to shield short-term lots, falling back to FIFO for older lots).
  - Returns realized gains/losses, remaining open tax lots, and cost basis.
- **Advanced Metrics Engine (`stats_engine.rs`)**:
  - Computes historical daily portfolio values.
  - Calculates daily returns and cumulative **Time-Weighted Return (TWR)** to accurately track performance independently of capital additions/withdrawals.
  - Computes **Annualized Volatility** and **Sharpe Ratio** (using a 2% risk-free rate).
  - Computes **Portfolio Beta** relative to the **SPY** S&P 500 index.
  - Computes **Beta-Adjusted Net Exposure** (total exposure scaled by portfolio beta).
  - Generates the asset **Correlation Matrix** based on daily returns log covariance.

> **Read this bullet list with §6.** It overstates the current build: annualized
> volatility, Sharpe ratio and portfolio beta are returned as constants
> (`stats_engine.rs:323-325`) and the correlation matrix is returned empty. What is
> actually computed is the daily value series, cumulative **TWR**, portfolio value and
> beta-adjusted exposure.

### 2. Frontend Web App (React + Vite)
- **Design System (`index.css`)**: Premium slate glassmorphic dark theme styled with custom CSS variables, responsive layouts, Google Font 'Outfit', custom badges, and interactive animations.
- **Dashboard View**: Displays Net Worth, Realized Tax P&L, Latent P&L, and Total Cumulative Return. Displays a historical value timeline (Recharts Area Chart) and asset type allocation (Recharts Pie Chart).
- **Holdings & Tax Lots View**: Detail view listing current holdings, average cost, market value, realized and unrealized gains. Allows users to expand any asset to inspect individual open tax lots (dates, unit costs, and latent ROI). Supports registering new asset tickers and logging buy/sell orders.
- **Risk & Analytics View**: Displays advanced risk metrics (Volatility, Sharpe, Beta, Beta-Adjusted Net Exposure) and a color-scaled **Correlation Heatmap** grid.
- **Transaction Ledger View**: Chronological record of all transaction activities with details on quantity, execution price, broker fees, and net trade values.

---

## 🧪 Verification & Validation Results

### 1. Automated Unit Tests
We verified the tax lot engine algorithms using cargo test:
`cargo test` (run from `backend_rust/`)

- **FIFO Matching**: Validated correct lot division, fractional share parsing, unit fee inclusions, and realized gains.
- **LIFO Matching**: Verified newest shares match first.
- **Hybrid Matching**: Verified LIFO matches on assets with age $\le 30$ days and FIFO applies to older assets.

**Test Run Outcome**:
```bash
backend/tests/test_tax_engine.py ...                                     [100%]
============================== 3 passed in 0.05s ===============================
```

> **Historical transcript.** That is `pytest` output from the Python backend this
> repository replaced. `cargo test` from `backend_rust/` — the command above — prints
> Rust test names and a `test result:` line, and nothing in this tree runs `pytest`.

### 2. End-to-End API Integration Verification
We executed an integration verification script ([verify_app.py](file:///Users/javier/.gemini/antigravity/brain/fb89866f-f765-45d0-98a1-2b68c71256dc/scratch/verify_app.py)) that creates a portfolio, logs transactions, pulls live prices, and checks metrics.

**Math Verification Match**:
- **Transactions**:
  - BUY 10 AAPL at $170.00, fee = $5.00. (Unit Cost = $170.50).
  - SELL 3 AAPL at $190.00, fee = $2.00. (Net Proceeds = $568.00).
- **FIFO Profit Math**:
  - Matched 3 AAPL against the first lot.
  - Cost Basis = $3 \times 170.50 = 511.50$.
  - Realized Profit = $568.00 - 511.50 = 56.50$.
- **Verification Output**:
  - AAPL current price dynamically fetched from Yahoo Finance.
  - Total Portfolio Value: **$33,378.52**
  - Total Realized P&L: **$56.50** (100% match with math).
  - Portfolio Beta: **0.75** (Defensive asset posture relative to S&P 500).
  - Portfolio Volatility: **26.05%**.
  - Sharpe Ratio: **3.38**.

> **Historical transcript.** The beta, volatility and Sharpe figures above were recorded
> against a build that computed them. The current one returns constants for all three, so
> do not expect to reproduce them — see §6.

---

## 🚀 How to Run the Application

To run both backend and frontend servers concurrently, a unified startup script `start.sh` has been created in the project root folder.

### Run in Local Terminal
1. Open a new terminal on your Mac.
2. Navigate to your project folder:
   ```bash
   cd /Users/javier/code/investment-portfolios
   ```
3. Run the startup script:
   ```bash
   ./start.sh
   ```

Both servers will start up concurrently:
- **Web App**: Accessible at [http://localhost:5173](http://localhost:5173)
- **API**: Accessible at [http://127.0.0.1:8000](http://127.0.0.1:8000)

To stop both servers, simply press `CTRL+C` in your terminal.

> [!TIP]
> **macOS Localhost Fix**: We explicitly configured the React frontend to communicate with `http://127.0.0.1:8000` instead of `localhost:8000`. This prevents connection drops on macOS caused by `localhost` resolving to IPv6 `::1` while the local Rust server binds to IPv4.

---

## Backup: export and restore

`cargo test` proves the bundle format, the validation errors, the byte and row caps, the
preview plan, and a copy/replace round trip against an in-memory SQLite database. CI also
diffs the three generated artifacts (`docs/openapi/openapi.json`,
`docs/backup-schema/v1.json`, `frontend/src/types/api.d.ts`) and runs the portability and
access gates. None of that touches a browser, a real identity provider, a file on a disk,
a second instance, or a locked database.

The sections below are the manual half, in the order to run them. Each assumes the ones
before it.

> **Shell.** These are **bash**. `docs/auth-testing.md` is written for fish; where the
> same command appears there, the fish form of `export VAR=value` is `set -x VAR value`.

### 0. What you need

The backup routes are authenticated like every other `/api` route, so §1 has to happen
before anything else works. From the repository root:

```bash
cd backend_rust
export AUTH_MODE=oidc
export OIDC_ISSUER="https://auth.example/application/o/portfolio-prism/"   # trailing slash
export OIDC_CLIENT_ID=portfolio-prism
export OIDC_CLIENT_SECRET='<from the provider>'
export OIDC_REDIRECT_URI=http://localhost:5173/auth/callback
export ALLOWED_ORIGINS=http://localhost:5173
export ROCKET_SECRET_KEY="$(openssl rand -base64 32)"
# Only §4 step 6 and §6 need price restore. Leave it unset otherwise.
# export BACKUP_ALLOW_PRICE_RESTORE=1
cargo run
# second terminal
cd frontend && npm run dev
```

Then open **http://localhost:5173**. Expected: the backend prints no `startup refused:`
line and binds `:8000`; if you exported `BACKUP_ALLOW_PRICE_RESTORE=1` it prints
`WARN: BACKUP_ALLOW_PRICE_RESTORE is on; a restore may write shared historical_prices
rows`, and that warning is the flag working, not a problem.

`OIDC_REDIRECT_URI` must be the `:5173` origin you browse on, not `:8000`: logout's
post-logout redirect is derived from it, and Authentik matches redirect URIs strictly.

Later sections need a `pp_session` cookie value for `curl`. It is `HttpOnly`, so it is
**not** in `document.cookie` — read it in devtools under Application → Cookies, or copy
the `Cookie:` header off a request the page already made:

```bash
export PP='<the pp_session value>'
```

### 1. A real login through Authentik

Nothing in this repository has ever completed a login against a real provider; every OIDC
test in CI exercises the failure path. This is the first of the two things CI cannot see.

1. Before signing in, from a shell:
   ```bash
   curl -sS -o /dev/null -w '%{http_code}\n' http://localhost:8000/api/portfolios/
   ```
   Expected: `401`. The backup routes answer the same way —
   `curl -sS -o /dev/null -w '%{http_code}\n' 'http://localhost:8000/api/backup/export?portfolio_ids=1'`
   is also `401`.
2. In the browser, go to **http://localhost:5173**. The app's first API call 401s and
   `apiFetch` redirects to `/auth/login`. Expected: Authentik's own login page, not an
   error page. An `invalid_client` or a redirect-URI complaint from the provider means the
   registered URI and the configured one differ — compare them character by character.
3. Sign in with your real credentials. Expected: you land on the app with **no**
   `?auth_error=` in the URL.
4. Open **http://localhost:8000/auth/me**. Expected: your `sub` and email. Record the
   `sub` — §4 step 5 needs it, and `docs/auth-testing.md` §6 does if the database has
   unowned rows.
5. Click **Backup** in the sidebar. Expected: an *Export a backup* card listing each of
   your portfolios with a checkbox, a line reading
   `A bundle is refused above 50,000 rows or 8,388,608 bytes` (those are
   `MAX_BUNDLE_ROWS` and `MAX_BUNDLE_BYTES`), a disabled **Download bundle** button, and a
   *Restore a backup* card below it. The tab opens even with no portfolios at all — that
   is deliberate, because restore is the way back for the user who has none.

### 2. Does logout end the provider's session?

The second thing CI cannot see, and it belongs here rather than only in
`docs/auth-testing.md` because a restore is a `POST` whose only credential is the session
cookie. The typed `REPLACE` in §4 is **client-side friction with no server-side
counterpart** — the route takes no confirmation parameter — so what "sign out" actually
retires is the whole question on a shared browser.

1. Sign in, then click **Sign out** in the sidebar (it points at `/auth/logout`).
   Expected: the browser leaves the app and lands on Authentik. Write down what it shows
   and what you had to click; the answer is provider-version-dependent and has to be
   observed, not inferred.
2. Check what that logout retired on this side. In devtools, Application → Cookies shows
   **no** `pp_session`. Now try the value you saved in §0:
   ```bash
   curl -sS -o /dev/null -w '%{http_code}\n' --cookie "pp_session=$PP" \
        http://localhost:8000/api/portfolios/
   ```
   Expected: **`200`, not `401`** — and that is the design, not a regression to report.
   `/auth/logout` clears the cookie and does not bump `token_version`, so a copy of the
   cookie stays good until its `exp` (8 hours, `src/auth/session.rs:25`). The route that
   does retire it is `POST /auth/logout-all`; prove that it works:
   ```bash
   curl -sS -o /dev/null -w '%{http_code}\n' -X POST --cookie "pp_session=$PP" \
        http://localhost:8000/auth/logout-all
   curl -sS -o /dev/null -w '%{http_code}\n' --cookie "pp_session=$PP" \
        http://localhost:8000/api/portfolios/
   ```
   Expected: `302`, then `401`. Say this out loud when you report §2: on a shared
   browser, **Sign out** removes the browser's copy of the credential, it does not retire
   the credential — and a restore is reachable with a copy someone kept.
3. Open a **new tab** and go to `http://localhost:5173/auth/login`.
   - **a credential prompt** → the provider ended its own session. The current design is
     correct and this closes.
   - **straight back into the app** → the provider session survived, and "log out" does
     not mean what a user expects on a shared browser. Measured against the live
     Authentik on 2026-11-07 this is the answer (`docs/auth-testing.md` §5, *ANSWERED*):
     Authentik shows *"You've logged out of portfolio-prism"* and one silent click
     re-enters. The fix is a server-side ID token store so the logout can carry
     `id_token_hint`; nothing on this branch changes that, and re-measuring is the point
     of this step.

Sign back in before §3, and re-export `PP` — a new login mints a new cookie.

### 3. The file is really yours and really complete

1. Sign in and note your portfolios, their assets, and a transaction count you recognise:
   ```bash
   curl -sS --cookie "pp_session=$PP" http://localhost:8000/api/portfolios/ \
     | python3 -c 'import json,sys;[print(p["id"],p["name"],p["currency"]) for p in json.load(sys.stdin)]'
   curl -sS --cookie "pp_session=$PP" http://localhost:8000/api/portfolios/<id>/transactions \
     | python3 -c 'import json,sys;print(len(json.load(sys.stdin)),"transactions")'
   ```
   `GET /api/portfolios/` answers `currency`, not `base_currency` — `base_currency` is a
   *bundle* field (`PortfolioOut` does not carry it), so asking the API for it raises
   `KeyError: 'base_currency'`. Step 3 reads it out of the file instead.
2. Open **Backup**, tick one portfolio of several — **one that has transactions**, because
   step 3 reads the first transaction out of the file and a portfolio with none has no
   first transaction — and click **Download bundle**.
   Expected: the browser saves `portfolio-prism-backup-YYYY-MM-DD.json` — that name comes
   from the server's `Content-Disposition`, not from the URL, and the response also
   carries `Cache-Control: no-store`.
3. Open the file. Confirm: the unselected portfolios are absent; `quantity`, `price` and
   `fee` are quoted strings, not numbers; transaction `date` values carry a time and a UTC
   offset; there is no `prices` array unless you asked for it. In one command, with the
   downloaded file copied to `/tmp/bundle.json`:
   ```bash
   python3 - <<'PY'
   import json
   b = json.load(open('/tmp/bundle.json'))
   print('format/version:', b['format'], b['schema_version'], b['created_at'])
   print('portfolios   :', [p['name'] for p in b['portfolios']])
   print('base currency:', [p['base_currency'] for p in b['portfolios']])
   print('has prices   :', 'prices' in b)
   txs = [t for p in b['portfolios'] for a in p['assets'] for t in a['transactions']]
   if not txs:
       print('transactions : none in this bundle — tick a portfolio that has some')
   else:
       t = txs[0]
       print('tx field types:', {k: type(t[k]).__name__ for k in ('quantity','price','fee')})
       print('tx date      :', t['date'])
   PY
   ```
   Expected: `portfolio-prism-backup 1 <RFC 3339 UTC>` (that tag is `FORMAT_TAG`, and a
   file without it is refused as "not a portfolio-prism-backup bundle"); only the ticked
   portfolio names; `base_currency` present on every portfolio (it is a bundle field, and
   this is where you read it); `has prices: False` (or `True` with a flat `prices` array if
   you ticked price history); `{'quantity': 'str', 'price': 'str', 'fee': 'str'}`; and a
   date like `2024-05-02T14:30:00+00:00`.

   **`+00:00`, not `Z`.** `export` writes `DateTime<Utc>::to_rfc3339()`, which spells UTC
   with the explicit offset, and `created_at` is the same call so it also carries
   sub-second digits. A `Z` is accepted on the way in — `parse_from_rfc3339` takes either
   spelling, and the fixtures in `backend_rust/tests/backup_test.rs` are written with one —
   but nothing in this build writes one, so expecting `Z` here is expecting a row the
   exporter cannot produce. A bare `2024-05-02` is a defect: the column is a datetime and
   the time is what orders same-day rows.
4. Open the **Tax Report** tab for that portfolio and compare its realized-P&L figure
   against the numbers the bundle implies. If they disagree, stop — the format is missing
   something, and the round-trip test in `backend_rust/tests/backup_test.rs` should be
   extended rather than this walkthrough written off. The route behind that tab is
   `GET /api/portfolios/<id>/tax-summary?strategy=FIFO&threshold_days=30&from=…&to=…`,
   and `docs/auth-testing.md` §5 records the reconciliation that is worth doing by hand:
   `lot_pnl` filtered to `SOLD_LOT`, summed, against `asset_realized_in_window`. Summing
   `realized_pnl` instead counts a four-lot sell four times.

### 4. Restore into a fresh instance

This is the step that justifies the feature, and no automated test has a real second
instance to restore into.

1. Start an empty instance and sign in as yourself. The easy path is to stop the first
   instance and restart it against a new file, because the provider matches redirect URIs
   strictly and a second origin has to be registered to run both at once:
   ```bash
   export DATABASE_URL='sqlite:/tmp/second.db?mode=rwc'
   cargo run
   ```
   Expected: a fresh, empty database — `GET /api/portfolios/` answers `[]`, not `401`.
2. Restore in **New copy** mode. Choose `/tmp/bundle.json` in *Bundle file*. Expected: the
   card previews on its own and shows `Will delete nothing.` and
   `Will insert N portfolios, M assets, K transactions.` Click **Restore**. Expected:
   `Restored N portfolios, M assets, K transactions.` — the same three numbers the preview
   named. Confirm the portfolios appear with the same assets and transaction counts, and
   that the tax report matches the source.
3. Restore the same bundle again in **New copy** mode. Expected: you now have two copies
   and nothing was overwritten — the portfolio list doubles, and the first copy's ids are
   unchanged. Bundle `ref` labels (`p1`, `a1`) are **not** those ids; a restore always
   allocates fresh ids, which is why the same file can be restored twice.
4. Restore in **Replace** mode. Select the *Replace* radio. Expected, in this order:
   - a *Type **REPLACE** to confirm that everything you own will be deleted* field
     appears, and **Restore stays disabled** while it is empty;
   - the plan re-previews for the new mode and now reads
     `Will delete N portfolios, M assets, K transactions.` — those are *your* rows, the
     ones you were shown before the button enabled;
   - typing `REPLACE` enables the button. Anything else — `replace`, a trailing space —
     leaves it disabled, and changing the mode clears the field and re-previews, because
     the confirmation is bound to the plan it was typed against.

   Click **Restore**. Expected: everything you owned is gone and only the bundle's
   contents remain, and the delete counts you were shown are exactly what disappeared.
   Afterwards the card is disarmed — no plan, no typed word — so a second destructive
   write needs a fresh preview and a fresh `REPLACE`.
5. Sign in as a **different user** on the same instance and confirm the first user's
   restore changed nothing visible to them: their portfolio list is what it was, and a
   `replace` by user A leaves B's rows alone. `GET /api/portfolios/` as B is the quick
   check; the automated version of this is
   `replace_leaves_other_users_data_alone` in `backend_rust/tests/backup_test.rs`.
6. Price history is deliberately not restorable from this panel: the card never sends
   `apply_prices`, so a bundle carrying a `prices` section previews
   `N price rows in this bundle will be left alone — this panel does not restore price
   history.` and restores without them. Checking the flag itself needs a bundle that
   **has** a `prices` section, and that needs the server flag on at export time — the
   export card renders the *Include price history* tick only when
   `/api/backup/capabilities` reports `allow_price_restore` (the `caps?.allow_price_restore`
   gate in `BackupPanel.tsx`), and §0 left the flag unset.

   **Read this before starting, because it is the step most easily misread.** `prices_for`
   (`backend_rust/src/backup/export.rs`) reads the bars out of the `historical_prices`
   table of the instance you export from, and restore writes them with
   `INSERT … ON CONFLICT (symbol, date) DO NOTHING`. Export from an instance and post the
   same file back **into that same instance** and every row conflicts with the row it was
   just read from, so `prices_inserted` is necessarily `0` — a correct answer, not a failed
   step. Steps a–c are that same-instance check: they prove the **guard**. Step d is what
   proves the **write**. The other way to see a count above zero is to restore into a
   *different* `DATABASE_URL`, one with no bars for those symbols — that is the case §6
   step 5 needs to compare metrics across instances — and it means signing in again after
   the switch, because a session cookie names a user row of the database that minted it.
   Steps a–d all stay on the instance from step 1 and need three restarts.

   a. **Precondition: this instance has to have bars, and nothing earlier in this document
      gives it any.** §6 step 5 says plainly that a fresh instance has no bars until the
      provider fills them, and §4 has been running on a fresh instance since step 1. Open
      the **Dashboard** for one of the restored portfolios and wait for the value chart to
      draw — that request is what fills `historical_prices` — then check, with the same
      `$DB` derivation §5 uses:
      ```bash
      export DB="${DATABASE_URL:-sqlite:../portfolio.db?mode=rwc}"
      export DB="${DB#sqlite:}"; export DB="${DB%%\?*}"
      sqlite3 "$DB" 'select symbol, count(*) from historical_prices group by symbol'
      ```
      Expected: at least one symbol with rows. If it answers nothing, the provider did not
      answer either, and every number below depends on that being fixed first.

      Then restart the backend with `export BACKUP_ALLOW_PRICE_RESTORE=1`, tick **Include
      price history** on the export card — tick the same portfolio you opened — download,
      and save the file as `/tmp/bundle-prices.json`. Expected: it carries a `prices` array
      of `{symbol, date, close_price}` rows, and
      `python3 -c 'import json;print(len(json.load(open("/tmp/bundle-prices.json"))["prices"]))'`
      prints a number above zero, and it is the bars for the ticked portfolios' symbols and
      no others (`prices_for` is asked for exactly those). With the flag off the tick is not
      offered at all and the file has no `prices` key, which is exactly what would make the
      checks below unreachable.
   b. Restart with the flag **off** (unset the variable) and post that file:
      ```bash
      curl -sS -X POST --cookie "pp_session=$PP" -H 'Content-Type: application/json' \
           --data-binary @/tmp/bundle-prices.json -w 'status=%{http_code}\n' \
           'http://localhost:8000/api/backup/restore?mode=copy&apply_prices=1'
      ```
      Expected: `422` naming price restore as disabled, and **nothing written** — the
      refusal is the first thing `apply` does, before `pool.begin()` (the `PricesDisabled`
      guard at the top of `apply`, `backend_rust/src/backup/restore.rs`), so none of the
      bundle's portfolios appear either.
   c. Restart with the flag **on** and post the same file. Expected: `200` and
      `"prices_inserted": 0` — **zero, on this instance, by construction.** This is the
      database those rows were read from, so all of them conflict and `DO NOTHING` writes
      none. A count above zero here would be the thing to investigate, not the thing to
      celebrate. What this step does prove is that the flag is what lets a price-bearing
      bundle through at all: the identical request was a `422` one restart earlier.
   d. Now the write itself, still on this instance. Clear one symbol's bars, and post the
      same file again:
      ```bash
      SYMBOL=$(python3 -c 'import json;print(json.load(open("/tmp/bundle-prices.json"))["prices"][0]["symbol"])')
      IN_BUNDLE=$(python3 -c 'import json,sys; print(sum(1 for p in json.load(open("/tmp/bundle-prices.json"))["prices"] if p["symbol"] == sys.argv[1]))' "$SYMBOL")
      echo "$SYMBOL: $IN_BUNDLE rows in the bundle"
      sqlite3 "$DB" "delete from historical_prices where symbol = '$SYMBOL'"
      curl -sS -X POST --cookie "pp_session=$PP" -H 'Content-Type: application/json' \
           --data-binary @/tmp/bundle-prices.json -w 'status=%{http_code}\n' \
           'http://localhost:8000/api/backup/restore?mode=copy&apply_prices=1'
      sqlite3 "$DB" "select count(*) from historical_prices where symbol = '$SYMBOL'"
      ```
      Expected: `prices_inserted` equals `$IN_BUNDLE`, and the last `select` prints
      `$IN_BUNDLE` too — the restore filled the gap it found. Count the rows in the bundle
      rather than the rows in the table because the two can differ: if the provider cached
      new bars after the export, the table had more rows than the file, and only the file's
      can be inserted. Post it once more and `prices_inserted` is `0`, which is the other
      half of the rule and the reason the statement is `DO NOTHING` and not `DO UPDATE`: a
      bundle may fill gaps in the cache, it may not overwrite a bar that is already there.

      Do the delete and the post back to back. If a dashboard loads in between, the
      provider refills the gap and the count comes back lower for that reason alone.

   Each of b, c and d is a `mode=copy` restore, so the portfolio list keeps growing as you
   run them. That is expected, and it is not what these steps are checking.

   Note what these pin down between them: the guard is the **combination** of a bundle that
   carries prices, `apply_prices` set, and the flag off. A flag-off restore of a bundle
   with no price section is a plain `200` whether or not `apply_prices` is set — that is the
   guard's own design ("a bundle with no prices asks for nothing the flag could forbid") and
   is pinned by a test, so it is not a hole to report.

   And c and d are not left to this page alone: both halves are pinned in
   `backend_rust/tests/backup_test.rs` — `a_price_restore_fills_gaps_and_never_overwrites_an_existing_close`
   (a stored date is neither overwritten nor counted, a gap is, and a row repeated inside
   one bundle counts once) and `the_price_section_round_trips_byte_for_byte` (empty the
   table, restore the file the export just produced, and every row comes back with its
   decimals unrescaled).

If step 2 or 4 disagrees with the preview you were shown, that is a defect in `plan_for`
versus `apply` — the two must count the same rows by the same rule.

### 5. A locked database answers 503

Single-writer SQLite with `replicas: 1` means `SQLITE_BUSY` is reachable in production, and
the backup module turns it into a `503` with a `Retry-After` rather than a `500`. No test
can show you what that feels like from the outside.

Run this from `backend_rust/`, the directory the backend resolves its relative
`DATABASE_URL` against — with the dev default, `sqlite3 "$DB"` opens the repository-root
`portfolio.db` only from there (`main.rs:53-54`). Open the database and take the write
lock:

```bash
export DB="${DATABASE_URL:-sqlite:../portfolio.db?mode=rwc}"
export DB="${DB#sqlite:}"; export DB="${DB%%\?*}"
sqlite3 "$DB"
```
```
sqlite> .timeout 0
sqlite> BEGIN IMMEDIATE;
```

`BEGIN IMMEDIATE` is the whole trick: it takes the RESERVED write lock and holds it until
the transaction ends. Nothing needs to happen inside it, and writing something would not
change the outcome — under the rollback journal (`main.rs:56-76` deliberately does not
enable WAL) a write statement still only holds RESERVED, and EXCLUSIVE is taken at COMMIT,
which this transaction never reaches.

Then, from a second terminal, with that transaction still open, drive the **restore**
route and prove a refused restore writes nothing. Use a portfolio id you actually own on
this instance — the one you exported in §3, not a literal `1` — because a foreign id is a
`404`, and a `404` here reads as the lock test failing:

```bash
sqlite3 "$DB" 'select count(*) from portfolios'
curl -sS -X POST --cookie "pp_session=$PP" -H 'Content-Type: application/json' \
     --data-binary @/tmp/bundle.json -o /dev/null -w 'status=%{http_code}\n' \
     'http://localhost:8000/api/backup/restore?mode=copy'
sqlite3 "$DB" 'select count(*) from portfolios'
```

Expected: the request **hangs for about five seconds** — that is the `busy_timeout` in
`main.rs`, and it is the honest experience, not a bug — then
`HTTP/1.1 503 Service Unavailable`, a `Retry-After: 2` header, and
`{"error":"the database is busy; try again in a moment"}`. The two counts are identical:
the restore is one transaction, so a refused restore is a restore that never happened.
This is the manual version of `a_locked_database_is_a_503_with_retry_after_from_the_restore_route`
in `backend_rust/tests/backup_test.rs`, which holds a write transaction open the same way.

Now the export, with the lock **still held**, and expect the opposite:

```bash
curl -sS --cookie "pp_session=$PP" -o /dev/null -w 'status=%{http_code}\n' \
     "http://localhost:8000/api/backup/export?portfolio_ids=<the id you own>"
```

Expected: **`200`, immediately.** Not a `503`, and not the feature failing. A RESERVED
writer does not block readers under the rollback journal, and `backup_export::build`
issues no write, so there is nothing for a read-only route to wait on — the auth guard's
own read of `users` gets through for the same reason. The consequence is written into the
route at the `DbFailure::Lock` arm of `export`'s error mapping in
`src/api_routes/backup.rs`: **the export's `Lock` arm is unreachable from a held lock in
this journal mode.** The failure that could reach it, pool exhaustion,
is met earlier by the guard and answered a `500` before that arm. Both halves are pinned by
tests instead (`an_exhausted_pool_is_a_lock_not_an_outage`, and
`a_lock_is_a_503_that_says_when_to_retry` for the response shape). So a `200` here is the
correct answer: the read-only routes are simply not where `SQLITE_BUSY` comes from, and
this walkthrough must not pretend otherwise.

Finish with `ROLLBACK;` in the sqlite3 prompt and re-run the restore: expected `200` — the
lock was the only thing refusing it.

The one way to get a false pass: a plain `BEGIN;` will not do it, because SQLite lets a
deferred transaction stay a reader and nothing blocks. It has to be `BEGIN IMMEDIATE`.

### 6. Metrics equality before and after a restore — this one is manual

**Say this plainly when you run it: nothing automated compares the metrics engine's answer
before and after a restore.** `backend_rust/tests/backup_test.rs` round-trips a bundle and
proves the format is lossless (`a_round_trip_through_export_and_restore_is_lossless`) and
that the tax engine answers identically on the restored rows
(`a_restored_portfolio_produces_the_same_tax_result`); no test calls
`StatsEngine::calculate_portfolio_performance` on a restored database and compares it
with the source. It stays manual because the engine reads live provider prices and the
shared `historical_prices` cache, so an automated version would need a provider stub and a
frozen clock — a different piece of work, not a missing assertion.

**Read this before you compare anything.** Three of the four figures the Risk & Analytics
tab shows are constants in the code as it stands: `backend_rust/src/engines/stats_engine.rs:322-330`
returns `"volatility": "0.0"`, `"sharpe_ratio": "0.0"` and `"beta": "1.0"` on every path,
and `stats_engine.rs:383` asserts the Sharpe is `"0.0"`. Comparing those three before and
after a restore proves nothing, because they cannot change. What the engine does compute —
and therefore what this check is actually about — is the daily value series, the cumulative
**TWR** in `history[].twr`, `portfolio_value` and `beta_adjusted_exposure`. When the engine
starts computing volatility, Sharpe and beta for real, this section becomes the only thing
that checks they survive a restore; until then, do not record a passing comparison of three
constants as if it were a check.

1. On the source instance, capture the metrics for each portfolio you are about to export:
   ```bash
   curl -sS --cookie "pp_session=$PP" "http://localhost:8000/api/portfolios/<src-id>/performance" \
     | python3 -c 'import json,sys; d=json.load(sys.stdin); print(json.dumps(d["metrics"],indent=2,sort_keys=True)); print("last twr:", d["history"][-1]["twr"] if d["history"] else "n/a")' \
     > /tmp/metrics-before.txt
   ```
2. Do the restore (§4), and read the new portfolio id out of the restore response's
   `portfolio_ids` — it is never the id in the bundle.
3. Capture the same thing for the restored portfolio into `/tmp/metrics-after.txt`.
4. Compare:
   ```bash
   diff /tmp/metrics-before.txt /tmp/metrics-after.txt
   ```
   Expected: no output.
5. If it differs, rule out the two things that are not the restore's fault before you
   call it a defect:
   - **The window ends today.** `stats_engine.rs:121` sets `end_date = Utc::now()` and the
     last point of the series is a live quote, so a comparison made across a market move
     or across a midnight boundary is not like for like. Take both readings in one sitting.
   - **The price cache is shared and has no owner.** `historical_prices` carries no
     `owner_id`, so a restore into the *same* instance sees the same bars and the
     comparison is meaningful. A restore into a *fresh* instance has no bars at all until
     the provider fills them, and the numbers will differ for that reason alone. To
     compare across instances you must carry the bars: export with price history, restore
     with `apply_prices=1` and `BACKUP_ALLOW_PRICE_RESTORE=1` (§4 step 6) — and remember
     that writing those rows is one tenant writing market data every other tenant's dated
     reports read, which is why the flag ships off. (Note which count belongs where: §4
     step 6 posts back into the instance the bars were read from and so expects
     `prices_inserted: 0`; it is a restore into a file with no bars for those symbols —
     the case this bullet is about — that puts a number above zero.)
6. Record which figures you actually compared, and say that a human did it.

### 7. What this still does not prove

- **The metrics comparison is manual and will drift.** It is the only check of its kind,
  and three of its four named figures are constants today (§6).
- **The typed `REPLACE` is friction, not a control.** The restore route has no
  confirmation parameter; the gate is authentication plus the `SameSite=Lax` session
  cookie. An authenticated caller can already delete every portfolio one at a time, so
  `replace` adds speed, not capability.
- **Concurrency.** `replicas: 1`, single-writer SQLite. `mode=replace` under concurrent
  writes on Postgres probably needs a `SERIALISABLE` restore transaction; see the plan's
  follow-ups.
- **Price restore is a mitigation, not a model.** `historical_prices` has no `owner_id`;
  the flag is what keeps one tenant from silently rewriting shared market data.
