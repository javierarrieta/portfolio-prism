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

**The orphan audit has never been run against a production SQLite file.** Every run of
`backend_rust/scripts/pre_cutover_audit.sh` that anyone has ever done was against a synthetic
file built in a development workspace — a file with the right tables and columns, populated
by hand to prove the exit codes. What that proves is the script's contract; what it does not
prove is that the script meets the real `portfolio.db`, with its actual history of partial
writes, its Python-era rows, and whatever the deployed schema drifted into.

*What retires this:* running `bash backend_rust/scripts/pre_cutover_audit.sh /path/to/copy-of-portfolio.db`
against **a copy of the real file**, before the cutover window opens, and reading every line
of its output rather than only its exit code. Do that once and this limitation goes away.
Until it is done, treat a clean `0` from the audit as weaker evidence than it looks.

**`docker-compose.auth-test.yml` has never been run by anyone.** The workspace it was written
in has no container engine, so the file is unexecuted configuration, and the Postgres cutover
edited it anyway (its `DATABASE_URL` is now a required variable). Nothing about it is
verified, including whether it starts at all.

*What retires this:* one successful `docker compose -f docker-compose.auth-test.yml --env-file
.auth-test.env up --build` that reaches `http://localhost:8080/auth/login`, anywhere, by
anyone. Until then the OIDC harness that *is* exercised is `backend_rust/scripts/auth_smoke.sh`
against a local backend, and the browser half of `docs/walkthrough.md` §1–§2 has to be done
against a real deployment.

## What you need

- **The SQLite file the running app uses.** In production that is the file mounted at
  `/app/data/portfolio.db` in the backend container; in development it is `portfolio.db`
  in the repository root. Copy it somewhere you can read it, and keep the original.
  Everything in step 1 runs against that copy, never against Postgres.
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

## The seven steps

The order is **audit → export → stop writes → deploy → restore → verify → keep the rollback
path**. It is not the order you might guess, and the reason is mechanical: the export is an
HTTP route, so an application that has already been scaled to zero cannot serve it. Freeze
before the restore, not before the export.

### 1. Audit for orphans — a stop condition

`bash backend_rust/scripts/pre_cutover_audit.sh /path/to/portfolio.db`

Run it against the live source, with the app still serving. The script is built for that: its
queries carry a 5 s busy timeout so a read that lands inside a writer's window waits instead
of reporting "could not run".

Every count in the orphan section must be 0. A non-zero count is a stop condition: the
bundle cannot represent a row whose parent is gone, so it will be dropped without the
export reporting anything. Repair or accept the loss in writing before continuing.

The exit code is the contract, and the runbook branches on it:

| Exit | Meaning | What you do |
| --- | --- | --- |
| **0** | No orphans. | Export, and continue. |
| **1** | Orphans found. **Stop.** The export will not carry these rows and the restore will not reject them, so nobody downstream ever learns they were missed. Repair them in SQLite — reparent each row, or confirm it is meant to be gone and delete it — then re-run and proceed only on 0. |
| **2** | The audit could not run: no argument, no `sqlite3`, an unreadable file, a file that is not a database, or a database that is not this app's schema. **Also a stop, but a different one.** You pointed the script at the wrong thing. Do not "fix" this by cleaning data; fix the command. |

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
  Assign them an owner in SQLite *before* the export. The startup ownership backfill runs
  on the Postgres target, after the export that never carried the row.
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
that writes nothing** — `BEGIN ISOLATION LEVEL REPEATABLE READ` on this build
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
`replicas: 1` and `Recreate`, an image that rolls out before its `DATABASE_URL` exists is
a crash-looping app with no serving replica.

`sqlx migrate run` creates the schema; the app also runs the embedded migrations at
startup (`sqlx::migrate!`), so a correctly configured pod migrates itself before it serves
anything. Verify with:

```bash
sqlx migrate run --database-url "$DATABASE_URL"   # idempotent
psql "$DATABASE_URL" -c 'select version, description from _sqlx_migrations order by version'
```

If any portfolio has no owner, this is also the start that needs `BOOTSTRAP_OWNER_SUBJ`
set to the `sub` that inherits them — see `docs/auth-testing.md` §6 before setting it, the
transfer is one-way.

### 5. Restore

Preview first, as the feature requires, then restore with `apply_prices=true` and
`BACKUP_ALLOW_PRICE_RESTORE=1` — then **unset the flag again**. It is off by default
because `historical_prices` has no `owner_id`: a restore that writes it is one tenant
writing market data every other tenant's dated reports read. The backend prints a `WARN`
on every start with it on; that warning is the flag working.

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

Check the response's counts against the preview's counts. They must agree; if they do not,
that is a defect in `plan_for` versus `apply`, not a rounding error.

### 6. Verify counts

Two baselines, and they are not the same thing:

- **The bundle is the exact target.** Postgres was written from it, so Postgres's counts
  should match the bundle's, row for row, apart from the `users` row noted below.
- **The audit's row-count block from step 1 is a lower bound.** It was taken while the app was
  still serving, so any table that grew between step 1 and step 3 grew into the lost window.
  Postgres being *above* the step 1 numbers is expected; Postgres being *below* the bundle is
  a defect.

```bash
# what landed in Postgres
psql "$DATABASE_URL" -c "
select 'portfolios'   t, count(*) from portfolios
union all select 'assets',       count(*) from assets
union all select 'transactions', count(*) from transactions
union all select 'users',        count(*) from users
union all select 'historical_prices', count(*) from historical_prices
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

Expectations that are not "identical numbers":

- `users` will be **larger** on Postgres: the placeholder `local`/`bootstrap` row is
  created at startup when there is no owner to hand orphans to, and every restored owner
  is a row the SQLite file may not have had.
- `historical_prices` is shared and has no owner, so it is only comparable if you exported
  and restored prices, and it is only *equal* if nothing wrote a new bar in between.
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
   Postgres image. With `replicas: 1` and `Recreate` the old pod is torn down before the new
   one is created, so every other order — key removed first, Secret later, image already
   rolled — leaves no serving replica at all. Three facts true before the apply starts, not
   three applies made true in turn. The follow-up spec carries the ordering as its own
   numbered section.
3. **`docker-compose.auth-test.yml` — changed here, because nothing runs it in anger.** It
   is a local harness, so its `DATABASE_URL` now comes from `.auth-test.env` and it fails
   with a named error rather than starting a backend that dies on its own configuration
   check. It still has no Postgres service of its own, and it is still the unvalidated file
   described in *Known limitations* above.
4. **`k8s/pv.yaml` / `k8s/pvc.yaml` (the file-backed volume) — leave them until step 7.**
   They are the rollback path. Retiring them is a decision with a date on it, not a cleanup.

## What this runbook has and has not been shown to do

- The audit script was run for real, with all three exit codes exercised, and its output is
  in `task-10-report.md` alongside the rest of the executed-walkthrough transcript. The
  file it was run against was synthetic, which is the first limitation in *Known limitations*
  above: **no `portfolio.db` exists in this workspace**, so the audit has never seen a real
  production file. Running it against a copy of the real one is step 1's first job on the
  day, and it is not something this document can pre-answer.
- Steps 2, 5 and 6 were executed end to end against a local Postgres cluster (export →
  preview → restore → price restore → metrics comparison), with `AUTH_MODE=disabled` in
  place of a real provider login. The provider half of `docs/walkthrough.md` §1–§2 is not
  runnable here and is unchanged.
- Steps 1, 3 and 4 have never been executed at all. Step 1 for the reason above; step 3 is a
  `kubectl scale` against a deployment that does not exist in this workspace; step 4 is the
  deferred half — there is no cluster here and the CNPG `Database` and `DatabaseRole` do not
  exist yet. That is why this runbook ends with a pointer instead of a `kubectl` command.
