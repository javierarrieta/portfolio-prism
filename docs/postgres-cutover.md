# Postgres cutover runbook (application side)

The procedure for the day. It is spec §11 of
`docs/superpowers/specs/2026-11-14-postgres-only-design.md` written out with the commands
runnable, and it covers **the data**. The infrastructure half — the CNPG `Database`, the
`DatabaseRole`, the Secret, and the order the manifests go in — is
`docs/superpowers/specs/2026-11-14-postgres-gitops-followup.md`, and step 4 below cannot
start until that document's ordering has been followed.

Read the whole thing before you start. Step 1 is a stop condition, step 5 is the point of no
easy return, and step 2 contains the one thing this procedure knowingly loses.

## Known limitations — read these before you trust anything below

**The orphan audit has been run against the real production file, and it exited 2.** The earlier
limitation — that no run had ever touched a real `portfolio.db` — was retired on 2026-11-15, and
what the real run produced was not a clean `0`. Against a `kubectl cp` copy of the live file
(45,056 bytes) it printed:

```
orphan transactions         : 0
orphan assets               : 0
portfolios, owner missing   : 0
AUDIT ERROR: query failed against /tmp/pp.db: no such column: s.asset_id
```

Exit **2**. The three checks that ran found nothing; the fourth — `split_sync` rows whose asset is
gone — never executed, because the deployed schema has no column by that name. Production was
running `v0.0.6-M5`, whose `split_sync` is `symbol TEXT PRIMARY KEY, last_synced_at DATE`. The
per-asset rekey that added `split_sync.asset_id` arrived in #57, which is **not** an ancestor of
`v0.0.6-M5`. The script assumes a schema newer than the one a live M5 deployment writes.

*What that exit code means.* Exit 2 is “the audit could not run”, not “the data is bad”. Do not read
it as a clean result and do not repair it by editing data. If your source file is M5-era, look at the
shape and run the missing check by hand:

```
sqlite3 /path/to/dst.db '.schema split_sync'
# symbol-keyed, i.e. no asset_id — the equivalent orphan check for that shape:
sqlite3 /path/to/dst.db "select s.symbol from split_sync s
  left join assets a on a.symbol = s.symbol where a.symbol is null;"
```

*What retires this:* a script that branches on `PRAGMA table_info(split_sync)` and checks the shape
it actually finds, or a source file already on the post-#57 schema. The script has **not** been
changed, so this limitation stands for any future cutover from an M5-era file.

*The earlier synthetic rehearsal, for the record.* One synthetic file (`clean.db`, 28,672 bytes:
one user, one
portfolio, one asset, two transactions, no `historical_prices`, no `split_sync`) and three
mutations of it, each run for real. The clean file printed all four orphan counts at `0`,
`portfolios, no owner : 0`, the six-row row-count block, then `No orphans. Safe to export.`
and exited **0**. The mutated file (one transaction pointing at asset 99, one asset named
`GONE` pointing at portfolio 99) printed `orphan transactions : 1` with the offending row
(`transaction 3 points at asset 99`) and `orphan assets : 1` with `asset 9 (GONE) points at
portfolio 99`, then the stop text, and exited **1**. A text file that is not a database, and
a path that does not exist, each exited **2** naming what was missing. Those are the three exit
codes — the contract, proven on a file the contract was invented for.

**`docker-compose.auth-test.yml` has never been run by anyone.** The workspace it was written
in has no container engine, so the file is unexecuted configuration, and the Postgres cutover
edited it anyway (its `DATABASE_URL` is now a required variable). Nothing about it is
verified, including whether it starts at all.

*What retires this:* one successful `docker compose -f docker-compose.auth-test.yml --env-file
.auth-test.env up --build` that reaches `http://localhost:8080/auth/login`, anywhere, by
anyone. Until then the OIDC harness that *is* exercised is `backend_rust/scripts/auth_smoke.sh`
against a local backend, and the browser half of `docs/walkthrough.md` §1–§2 has to be done
against a real deployment.

## Execution record — 2026-11-15, `portfolio.titan.arrieta.eu`

This runbook has now been executed for real. What it found was that there was nothing to migrate.

| step | what actually happened |
| --- | --- |
| 1 audit | Run against a `kubectl cp` copy of the live `portfolio.db`. **Exited 2** on a schema assumption — see the limitation above. The three checks that ran were clean. |
| 2, 2b export, preview | **Not exercised.** The source was empty: `portfolios 0, assets 0, transactions 0, historical_prices 0, split_sync 0, users 2`. |
| 3 stop writes | **Not exercised.** With nothing to export there was no window to freeze. |
| 4 deploy | **Executed, by GitOps rather than `kubectl`.** CNPG `Database` + `DatabaseRole` in k8s-titan #296; image roll and inline `DATABASE_URL` removal in #297. The deployment is `Recreate` with one replica, so the SQLite pod went away before the Postgres pod started; unavailability was the pod swap, about three minutes including the Flux sync. |
| 5 restore | **Not exercised** — no bundles existed. |
| 6 verify counts | **Not exercised as a comparison.** What was checked instead: the backend reached `Rocket has launched`, which `main.rs:65,74` permits only after a successful `db::connect` **and** a successful `sqlx::migrate!`, and `/api/portfolios/` answers 401 rather than 500. |
| 7 keep the PVC | **Done.** The PVC, its mount and its restic schedule are untouched; reverting the k8s-titan flip restores M5 with the file it never lost. |

The two users in the source were the `local`/`bootstrap` placeholder and one Authentik identity.
Neither was carried over: `ensure_bootstrap_owner` inserts the placeholder at startup and the
identity reappears on first login. That is the entire data path this cutover needed.

**The generalization to avoid:** an empty source made steps 2, 2b, 3, 5 and 6 unnecessary here. It
does not make them unnecessary for a deployment that has data. Those steps are still unrehearsed
against a populated file.

## What you need

- **The SQLite file the running app uses.** In production that is the file mounted at
  `/app/data/portfolio.db` in the backend container; in development it is `portfolio.db`
  in the repository root. Step 1 audits **a consistent copy** of it, made with
  `sqlite3 … .backup` and not with `cp` — see step 1, which is the procedure. Keep the
  original. Everything in step 1 runs against that copy, never against Postgres.
- **`sqlite3` on the machine doing the audit.** Not on the Postgres host — on the machine
  holding the file. If `sqlite3` is missing the audit exits **2** and says so; it refuses
  to be skipped.
- **A Postgres instance** (18.x — `backend_rust/migrations/` is written against 18) with a
  role that can `create` in its own database, and a `DATABASE_URL` reachable from the app.
- **The Postgres build of the image.** A pre-cutover image still has
  `ENV DATABASE_URL=sqlite:///app/data/portfolio.db` baked in; the current build has no
  default at all and refuses to start without `DATABASE_URL`. That refusal is the safety
  property step 4 depends on: a misconfigured cutover produces a pod that exits on its
  configuration check rather than one that silently opens an empty file database.
- **A logged-in identity for every portfolio you are exporting.** The export route is
  authenticated and scoped like every other `/api` route (`src/access.rs`); there is no
  operator-wide export. If a portfolio has no owner, no user can export it — the audit
  prints those rows for exactly that reason.

## The steps

The order is **audit → export → preview → stop writes → deploy → restore → verify → keep the
rollback path**. It is not the order you might guess, and the reason is mechanical: the export
and the preview are both HTTP routes, so an application that has already been scaled to zero
cannot serve either. Freeze before the restore, not before the export.

### 1. Audit for orphans — a stop condition

**The input is a consistent copy of the SQLite file, and nothing else.** Not the live file,
not a `cp` of the live file. A hot `cp` of a rollback-journal database can catch the writer
mid-transaction and produce a file that is quietly torn, and an audit of a torn file reports
whatever it happens to read. Make the copy the way SQLite says to make one:

```bash
sqlite3 /path/to/src.db ".backup /path/to/dst.db"
sqlite3 /path/to/dst.db 'PRAGMA integrity_check;'   # must print `ok`
```

Getting the file out of the container is the awkward half, because **the Postgres image no
longer ships `sqlite3`** — there is nothing to link it against any more, so you cannot
`kubectl exec` into the pod and run `.backup` there. Copy the raw bytes out first, then make
the consistent copy on the machine that has `sqlite3`:

```bash
# `backend` is the container that holds the file. The pod has two containers and `frontend`
# is listed first, so `-c backend` is not optional: without it `kubectl cp` runs its `tar` in
# the frontend container, which has no `/app/data`.
POD=$(kubectl -n investment-portfolio-manager get pod \
  -l app=investment-portfolio-manager -o jsonpath='{.items[0].metadata.name}')
kubectl -n investment-portfolio-manager cp -c backend "$POD":/app/data/portfolio.db ./raw-portfolio.db
sqlite3 ./raw-portfolio.db 'PRAGMA integrity_check;'   # `ok`, or the copy was torn: re-copy
sqlite3 ./raw-portfolio.db ".backup ./dst.db"
bash backend_rust/scripts/pre_cutover_audit.sh ./dst.db
```

`kubectl cp` is a plain file copy — it shells out to `tar` inside the container, which the
Debian-slim runtime image has — so the `integrity_check` between the two steps is not
ceremony: it is the only thing that tells you the raw copy survived the trip intact. If it
prints anything but `ok`, copy again, at a quieter moment. Check the deployment's
`volumeMounts` for where the file actually sits: `k8s/app-deployment.yaml` mounts the PVC at
`/app/data` in the `backend` container, and the PVC is named `postgresql-db-pvc` despite
holding the file database, which is a name that will mislead you if you go looking for the
volume by claim name instead of by mount path. In development the file is already on a machine
with `sqlite3`, so the first block is the whole procedure.

The script is read-only and its queries carry a 5 s busy timeout, so it is safe to run while
something is still writing — but by now you are auditing a static copy, and that timeout is
insurance rather than the plan.

Every count in the orphan section must be 0. A non-zero count is a stop condition: the
bundle cannot represent a row whose parent is gone, so it will be dropped without the
export reporting anything. Repair or accept the loss in writing before continuing.

The exit code is the contract, and the runbook branches on it:

| Exit | Meaning | What you do |
| --- | --- | --- |
| **0** | No orphans **in the file you were pointed at**. A `0` only means something if the input was the consistent copy from step 1: a hot `cp`, the live file, the wrong file, or a stale copy from before someone repaired something all produce a `0` that proves nothing (see *Known limitations* above). | Export, and continue. |
| **1** | Orphans found. **Stop.** The export will not carry these rows and the restore will not reject them, so nobody downstream ever learns they were missed. Repair them in SQLite — reparent each row, or confirm it is meant to be gone and delete it — then re-run and proceed only on 0. |
| **2** | The audit could not run: no argument, no `sqlite3`, an unreadable file, a file that is not a database, or a database that is not this app's schema. **Also a stop, but a different one.** You pointed the script at the wrong thing. Do not "fix" this by cleaning data; fix the command. |

One exit-2 branch needs its own sentence, because the script's own advice for it is out of
date. If it reports that the `portfolios` table has no `owner_id` column, the file predates
the ownership work and has never been opened by a backend that added it. **The current
backend cannot add it: it is Postgres-only and will not open a SQLite file at all.** Run
*the pre-cutover image* — the last build that still speaks SQLite — against a copy of the
file once, let it add the column and backfill owners, and then audit that copy. Never
against the original: the backfill writes.

**Record the numbers.** The script prints the four orphan counts, then a row-count block
(`users`, `portfolios`, `assets`, `transactions`, `historical_prices`, `split_sync`).
Save the whole output to a file — step 6 compares against it, and once the SQLite PVC is
retired in step 7 there is nothing left to re-run it against. Because the audit now runs
*before* the export, those counts are a lower bound rather than the target; step 6 says what
to compare them against.

Two things the script tells you and the runbook has to act on:

- **`portfolios, no owner` is printed as informational and is not part of the exit code** —
  but an unowned portfolio is exactly as unexportable as an orphan, because the export
  carries only portfolios the caller is authorised for and an unowned row is nobody's.
  Assign them an owner in SQLite *before* the export. Step 4 explains why nothing on the
  Postgres side can fix this after the fact.
- Expect findings on `portfolios.owner_id` and `split_sync.asset_id`. Those are the two
  relationships SQLite never enforced (one added by a bare `ALTER TABLE`, one declared with
  no foreign key at all); `assets.portfolio_id` and `transactions.asset_id` were already
  enforced.

### 2. Export every portfolio — while the app is still serving

For each owner, with `include_prices=1`, and store the file **outside the cluster**. It is
the only copy that matters.

**Say this out loud before you do it: a write that commits after the export's snapshot is
taken and before step 3 stops writes is in no bundle, and nothing will ever report it.** A
trade saved in that window is gone from the migrated data, the user will notice it gone, and
the person who was told the export was safe will then believe nothing else in this document.
It is accepted, not overlooked — the alternative is freezing writes before the export, and
the export cannot run against a frozen deployment. So: do it at low traffic, keep the window
short, tell the users it exists, and if the window is genuinely not acceptable, export again
after step 3. A frozen source exports a bundle that is exactly current, and that second
bundle is the one to restore.

What the snapshot actually is: every read the export issues runs inside **one transaction
that writes nothing** — `BEGIN READ ONLY ISOLATION LEVEL REPEATABLE READ` on this build
(`backup::export::EXPORT_ISOLATION`), and one deferred read transaction on the pre-cutover
SQLite build that is still serving on the day. Either way the bundle is one snapshot, not a
sequence of them, and a commit that lands while it is being read is simply not in it. On the
SQLite source there is a second effect worth knowing: a read transaction holds a `SHARED`
lock, so a writer trying to commit mid-export waits behind it and either lands after the
export finishes — putting itself in the lost window above — or times out and is refused. It
is not silently lost; the user is told.

```bash
curl -sS --cookie "pp_session=$PP" \
     -o "bundle-$(date +%F).json" \
     'http://localhost:8000/api/backup/export?portfolio_ids=1,2,3&include_prices=1'
```

- `portfolio_ids` is required and may appear only once (a comma-separated list). Asking
  for a portfolio you are not authorised for is a `404`, not a partial bundle.
- `include_prices=1` needs nothing special on the **export** side; the
  `BACKUP_ALLOW_PRICE_RESTORE` flag gates the *restore*. The web UI only offers the
  *Include price history* tick when `/api/backup/capabilities` reports
  `allow_price_restore`, which is why the flag is often described as an export concern.
- A bundle over `MAX_BUNDLE_ROWS` (50,000) or `MAX_BUNDLE_BYTES` (8,388,608) is refused
  with a `422` **after** every read has happened — the caps are checked on the built
  bundle. A tenant that big exports portfolio by portfolio.
- Check the file before you trust it: `format` is `portfolio-prism-backup`,
  `schema_version` is `1`, `quantity`/`price`/`fee` are quoted strings, and every
  portfolio you ticked is present. `docs/walkthrough.md` §3 has the one-command version.

### 2b. Preview every bundle now — before you stop writes

This step exists because step 5 previews the bundles *after* the freeze, and by then a
refusal costs the whole window.

Run it against the still-serving deployment, with the same `$PP` step 2 used — that cookie is
still valid here, and step 5 is where it stops being one (see step 5 for why a stale cookie on
the new instance is worse than a 401). If you would rather not put a large upload through the serving instance, run
it against a scratch backend pointed at a throwaway Postgres database instead; in `mode=copy`
a preview writes nothing either way.

```bash
for f in bundle-*.json; do
  curl -sS -X POST --cookie "pp_session=$PP" -H 'Content-Type: application/json' \
       --data-binary "@$f" \
       'http://localhost:8000/api/backup/preview?mode=copy' \
       -w "  <- $f\n"
done
```

**Why this cannot wait for step 5.** `preview` is the only validator a bundle gets. In
`mode=copy` `plan_for` issues **zero queries** — there is nothing to count — and validation
happens before the first read, so a preview costs nothing and touches nothing. It is the only
place these surface:

- a numeric a Python-era writer stored in scientific notation. `1e-05` is a perfectly good
  SQLite `REAL` and `Decimal::from_str_exact` refuses it, so the row is only rejected when
  something tries to read it back as a decimal;
- an `asset_type` outside `KNOWN_ASSET_TYPES`, or a transaction `type` outside
  `KNOWN_TRANSACTION_TYPES`;
- a `date` that is not RFC 3339, or a price `date` that is not `YYYY-MM-DD`;
- a currency that is not three characters, a blank name or symbol, an invalid ISIN, a
  duplicate `ref`, a SPLIT with a non-positive quantity or price, or two SPLITs for one
  asset on one UTC day.

Every one of those is a `422` naming the path, and every one of them is a **`422` you would
rather have now**. As written without this step, one legacy row anywhere in one bundle turns
the cutover into a rollback after the site is already down — step 5's restore is one
transaction, so nothing is half-written, but the downtime is real and the repair has to
happen in SQLite and then be re-exported.

A `422` here is repaired in the source file and re-exported, which is cheap while the app is
serving and expensive after it is not. Do not accept "we will find out on the day".

### 3. Stop writes

Scale the SQLite deployment to zero, or put it in read-only mode. From here the source cannot
move, which is what makes step 6's comparison mean anything: two snapshots taken at
different moments are not one consistent backup, and this is the moment that makes the
bundle's moment the last one.

Note what you have now accepted, in one line, in the record: the writes between step 2's
snapshot and this step. If there are none — low traffic, no users online — say so; if there
are some, they are listed by comparing the bundle against the source copy you took in *What
you need*, and each one is either re-entered by hand after the cutover or accepted in writing.

This is also the start of the app's downtime. Everything between here and step 5 is time the
site is not serving, so rehearse it.

### 4. Deploy Postgres and start the Postgres build against an empty database

This is where the other spec takes over:
`docs/superpowers/specs/2026-11-14-postgres-gitops-followup.md` has the `Database`,
`DatabaseRole`, Secret and the ordering, and the ordering is not optional — with
`replicas: 1` and a `Recreate` rollout, an image that rolls out before its `DATABASE_URL`
exists is a crash-looping app with no serving replica. (Confirm the strategy on the live
deployment before the apply; see *What changed in the manifests* for why that is a check and
not a given.)

`sqlx migrate run` creates the schema; the app also runs the embedded migrations at
startup (`sqlx::migrate!`), so a correctly configured pod migrates itself before it serves
anything. The migrations live in `backend_rust/migrations/`, so `sqlx migrate run` has to be
told where to look — run it from `backend_rust/`, or pass `-m`:

```bash
cd backend_rust
sqlx migrate run --database-url "$DATABASE_URL"   # idempotent
psql "$DATABASE_URL" -c 'select version, description from _sqlx_migrations order by version'
```

**Nothing else goes here, and `BOOTSTRAP_OWNER_SUBJ` in particular is inert at this step.** It
transfers portfolios that are *already* in the database the backend is opening and have no
owner, and this database is empty — it has no rows to transfer. The unowned portfolios are the
SQLite ones, and they never arrive: the export cannot carry a portfolio nobody owns, which is
what step 1's `portfolios, no owner` line is for. Assign those owners **in SQLite, before the
export**, and leave this variable unset on the cutover. If some other deployment's Postgres
database genuinely holds orphaned portfolios, `docs/auth-testing.md` §6 explains the
one-way transfer; that is a different procedure from this one.

### 5. Restore

**Sign in to the new deployment first, once per owner, and mint a fresh `$PP` for each.** The
cookie you used in steps 2 and 2b is not a row in any table — it is a stateless signed cookie
holding `{uid, tv, exp}`, keyed by `ROCKET_SECRET_KEY`, and the cutover apply changes
`DATABASE_URL`, not that secret. So the old cookie still *decrypts* on the new instance, and
what happens next depends on whether the new `users` table happens to carry a row with that
`uid` at a matching `token_version`: it either 401s, or it **authenticates you as whoever that
row is**. The second outcome is the reason this is an instruction and not a nicety — a stale
cookie on a fresh database is not reliably refused, and a restore is scoped to the caller. Do
not reason about it as "the new instance has never seen my cookie, so it will refuse"; rotate
the secret with the cutover if you want refusal rather than surprise. There is no
operator-wide restore either — `restore` is scoped to the caller like every other
`/api` route, so the identity that restores a bundle is the identity that will own every row
in it. Per owner, therefore: sign in, mint `$PP`, restore that owner's bundle, repeat. Do not
restore several owners' bundles under one session; that hands every portfolio to the first
caller.

Preview again here if anything changed since step 2b — the preview is free, and it is the
same call. Then restore with `apply_prices=true` and `BACKUP_ALLOW_PRICE_RESTORE=1`.

The flag is off by default because `historical_prices` has no `owner_id`: a restore that
writes it is one tenant writing market data every other tenant's dated reports read. The
backend prints a `WARN` on every start with it on; that warning is the flag working.

**Where the flag goes, and what it costs.** It is deliberately absent from
`k8s/configmap.yaml`, so the deployed app has it off, and it is not a Secret value either —
the Secret carries `DATABASE_URL`. It is a plain container environment variable, so it lives
in `k8s/app-deployment.yaml` (or in the ConfigMap the deployment consumes, if you would
rather the value sit with the other configuration). Setting it and unsetting it are **two
rollouts**, and with the rollout strategy of step 4 each one is an interval with no serving
replica. Two rollouts inside the downtime window is the most expensive place in this
document to spend them.

The cheap ordering is to fold the flag into the rollouts you are already doing: put
`BACKUP_ALLOW_PRICE_RESTORE=1` in the Postgres rollout at step 4 — the app is not serving
anyone yet, so the flag costs nothing while the site is down — do every restore in step 5,
and remove the variable in the first rollout *after* the site is serving again. That is one
rollout, outside the window, instead of two inside it. Whichever order you pick, say which
in the record, because a deployment left with the flag on is a standing warning in the logs
that nobody remembers why they set.

```bash
# preview: the promise the restore has to keep
curl -sS -X POST --cookie "pp_session=$PP" -H 'Content-Type: application/json' \
     --data-binary @bundle-YYYY-MM-DD.json \
     'http://localhost:8000/api/backup/preview?mode=copy'
# restore
curl -sS -X POST --cookie "pp_session=$PP" -H 'Content-Type: application/json' \
     --data-binary @bundle-YYYY-MM-DD.json \
     'http://localhost:8000/api/backup/restore?mode=copy&apply_prices=1'
```

`mode=copy` is the cutover mode: it allocates fresh ids and overwrites nothing. `mode=replace`
deletes everything the caller owns first — on the day, with a bundle that is the whole
record, that is what you want **only** if the target already holds rows you mean to throw
away. The restore is one `SERIALIZABLE` transaction: a bundle that fails on row 201 leaves
the database exactly as it was.

**Compare the counts, but compare the right pair.** The preview reports `insert` and
`prices`; the restore reports `inserted` and `prices_inserted`. They are not four numbers
measuring one thing, and two of them are expected to disagree:

- **`insert` vs `inserted` must agree.** Both count the rows the bundle owns — portfolios,
  assets, transactions. `plan_for` counts them off the bundle and `apply` counts them off
  the inserts it made, and there is no `ON CONFLICT` on any of them. A difference here is a
  defect in `plan_for` versus `apply`, not a rounding error.
- **`prices` vs `prices_inserted` legitimately differ, and usually do.** `plan_for` reports
  `bundle.prices.len()` — how many bars the file carries. `apply` reports `rows_affected`
  from inserts written `ON CONFLICT (symbol, date) DO NOTHING` — how many bars the database
  did not already have. A second owner's restore conflicts with the bars the first owner
  just wrote and reports fewer; re-posting the same bundle reports `0`, which is the correct
  answer and not a failure. `docs/walkthrough.md` §4 step 6 says the same thing from the
  other direction: on the instance the bars were read *from*, a price restore inserts zero,
  by construction, and a count above zero there is the thing to investigate.

So: `insert` vs `inserted` is a check. `prices` vs `prices_inserted` is information. Do not
spend downtime-window minutes reconciling the second pair.

### 6. Verify counts

Two baselines, and they are not the same thing:

- **The bundle is the exact target** — for the tables a bundle carries. Portfolios, assets,
  transactions, and `historical_prices` if you restored prices. Postgres was written from
  it, so those counts should match the bundle's, row for row.
- **The audit's row-count block from step 1 is a lower bound.** It was taken while the app was
  still serving, so any table that grew between step 1 and step 3 grew into the lost window.
  Postgres being *above* the step 1 numbers is expected; Postgres being *below* the bundle is
  a defect.

```bash
# what landed in Postgres. `users` and `split_sync` are printed for the record and are
# deliberately left out of the comparison below — see what they mean.
psql "$DATABASE_URL" -c "
select 'portfolios'   t, count(*) from portfolios
union all select 'assets',       count(*) from assets
union all select 'transactions', count(*) from transactions
union all select 'historical_prices', count(*) from historical_prices
union all select 'users',        count(*) from users
union all select 'split_sync',   count(*) from split_sync"

# what the bundle claimed, from the file itself
python3 - bundle-YYYY-MM-DD.json <<'PY'
import json, sys
b = json.load(open(sys.argv[1]))
print('portfolios  :', len(b['portfolios']))
print('assets      :', sum(len(p['assets']) for p in b['portfolios']))
print('transactions:', sum(len(a['transactions']) for p in b['portfolios'] for a in p['assets']))
print('prices      :', len(b.get('prices', [])))
PY
```

Only those four lines are comparable. The other two are not, and comparing them to a step-1
baseline is a mistake — the step-1 audit counted rows in a database the bundle never
transports:

- **`users` is not in a bundle at all.** No export carries a user row, so Postgres's `users`
  is built entirely by the new instance: the placeholder `local`/`bootstrap` row, which
  `ensure_bootstrap_owner` creates unconditionally at startup, plus one row per identity that
  has signed in to the new deployment. That is very often **fewer** than the SQLite file had,
  because the old file accumulated accounts that never logged in again. Expect it to differ
  in either direction; treat a match as coincidence.
- **`split_sync` is not in a bundle either.** It is a throttle table — one row per asset
  saying when its splits were last synced — and the bundle has no section for it. On
  Postgres it starts empty and grows as `POST /api/portfolios/<id>/sync-splits` runs, so its
  count means "how many of these assets have had a split sync since the cutover" and nothing
  more. A step-1 `split_sync` count has no relationship to it.
- `historical_prices` is shared and has no owner, so it is only comparable if you exported
  and restored prices, and it is only *equal* if nothing wrote a new bar in between — and
  under `ON CONFLICT DO NOTHING` several owners' bundles can carry the same bars, so the
  restore's own `prices_inserted` is the number that will differ (step 5).
- Then compare a report, not just a count: `GET /api/portfolios/<id>/tax-summary` for one
  portfolio must reproduce the pre-cutover realized-P&L figure. `docs/walkthrough.md` §6
  covers the performance endpoint, and says plainly which of its four figures are
  constants today and therefore prove nothing.

### 7. Keep the SQLite PVC until Postgres has survived one backup cycle of its own

Not one *restore* of the bundle — a cycle: a scheduled backup of the Postgres instance that
ran, was stored, and was verified to be restorable. Until then the SQLite file is the only
proven-good copy of the data, and step 2's bundle is a logical copy with the same
limitations as the export that made it.

## Rollback

**Rollback after step 5 is not free: anything written to Postgres after the restore is not
in any bundle. Re-export from Postgres before declaring the cutover done.** Restoring the
step-2 bundle after the cutover is live therefore rolls back every write made since the
export — it is a rewind to the export's snapshot, not an undo of the last hour. If you do
roll back, the SQLite file is still the authority, the Postgres rows are the ones being
discarded, and the flag from step 5 goes back off.

## What changed in the manifests, and in which order

The image side is done in this branch; the cluster side is not, and must not be done by
editing one file at a time.

1. **`backend_rust/Dockerfile` — done here.** `libsqlite3-dev` (builder) and `libsqlite3-0`
   (runtime) are gone: nothing links against them since the engine switch, and `ldd` on the
   release binary says so. The baked-in `ENV DATABASE_URL=sqlite:///app/data/portfolio.db`
   is gone too, along with the `VOLUME /app/data` it existed to fill. A default that points
   at a file database is not a convenience in a Postgres-only build — it is a pod that
   starts, refuses, and looks like a database outage. The URL now arrives from the runtime
   (`docker run -e DATABASE_URL=…`, or the deployment's Secret) and nothing else.
2. **`k8s/configmap.yaml` — the `sqlite:` key is deleted at cutover, not in this branch.**
   It ships `DATABASE_URL: "sqlite:///app/data/portfolio.db?mode=rwc"` because a running
   cluster consumes this file. Deleting the key now breaks the live SQLite deployment on the
   next apply — no `DATABASE_URL` at all — and replacing it with a Postgres placeholder
   breaks it differently, with a DSN the app refuses on sight. Either edit is an outage caused
   by a commit rather than by the cutover, which is why the key stays where it is.

   **The atomic set.** The CNPG `Database`, the `DatabaseRole`, and the Secret carrying the
   real DSN must all exist and be Ready **before** the one apply that removes
   `DATABASE_URL` from the ConfigMap, puts the real value in the Secret, and rolls the
   Postgres image. With `replicas: 1` and a `Recreate` rollout the old pod is torn down
   before the new one is created, so every other order — key removed first, Secret later,
   image already rolled — leaves no serving replica at all. Three facts true before the
   apply starts, not three applies made true in turn. The follow-up spec carries the ordering
   as its own numbered section.

   **Confirm the strategy on the live deployment before the apply.** `Recreate` is assumed
   throughout this document and in `AGENTS.md`, the README and the follow-up spec, but
   `k8s/app-deployment.yaml` declares no `spec.strategy` at all — and the Kubernetes default
   when the field is absent is `RollingUpdate`, not `Recreate`. So the hazard is real only if
   the live object says so, and it is worth knowing which you are about to cause: under
   `Recreate` a badly ordered apply is an outage, and under `RollingUpdate` it is a
   crash-looping new pod while the old SQLite pod keeps serving an image that can no longer
   reach the file it expects. Check, do not assume — and note the live namespace and deployment
   name are `apps/portfolio-prism`, not the `investment-portfolio-manager/app` this file used to
   name from the stale manifests:

   ```bash
   kubectl -n apps get deploy portfolio-prism -o jsonpath='{.spec.strategy.type}'
   echo
   ```

   **Measured on the live cluster, 2026-11-15: it prints `Recreate`.** The hazard is therefore the
   immediate-outage kind, and the ordering above is not optional. Note that this is a fact about the
   live object, not about this repository: `k8s/app-deployment.yaml` still declares no
   `spec.strategy`, so re-check after any apply that rebuilds the deployment from a manifest rather
   than editing the live object. If the field is ever absent the answer is `RollingUpdate`, and a
   badly ordered apply becomes a crash-looping new pod while the old SQLite pod keeps serving an
   image that can no longer reach the file it expects.
3. **`docker-compose.auth-test.yml` — changed here, because nothing runs it in anger.** It
   is a local harness, so its `DATABASE_URL` now comes from `.auth-test.env` and it fails
   with a named error rather than starting a backend that dies on its own configuration
   check. It still has no Postgres service of its own, and it is still the unvalidated file
   described in *Known limitations* above.
4. **`k8s/pv.yaml` / `k8s/pvc.yaml` (the file-backed volume) — leave them until step 7.**
   They are the rollback path. Retiring them is a decision with a date on it, not a cleanup.

## What this runbook has and has not been shown to do

- The audit script was run for real, with all three exit codes exercised; the transcript is
  in *Known limitations* above, in this file, so a clone can read it. The file it was run
  against was synthetic. That gap has since closed, and the result was not what the rehearsal
  predicted: run against a copy of the live production file, the audit **exited 2** because the
  deployed schema is older than the one it queries. See *Known limitations* above.
- Steps 2, 2b, 5 and 6 were executed end to end against a local Postgres cluster (export →
  preview → restore → price restore → metrics comparison), with `AUTH_MODE=disabled` in
  place of a real provider login. The provider half of `docs/walkthrough.md` §1–§2 is not
  runnable here and is unchanged. Step 2b is the same `preview` call step 5 makes, so the
  call itself is exercised; what is **not** exercised is a preview that actually refuses,
  against a bundle carrying a Python-era value the Rust decimal parser rejects. No such
  bundle exists in this workspace, which is exactly why step 2b says to run it against the
  real files rather than to trust that they are fine.
- Step 1 has now been executed against a real file (exit 2, above) and step 4 has been executed
  against a real cluster via GitOps. **Step 3 has never been executed at all** — it is a
  `kubectl scale` against a deployment no workspace here has ever stopped.
- What remains unproven is the part with data in it: steps 2, 2b, 5 and 6 against a **populated**
  production file, and a real provider login at the end of it. The local rehearsal covered the
  mechanics of those steps against a database someone had filled in a development workspace; the
  production run skipped them because the live file held no portfolios at all.
