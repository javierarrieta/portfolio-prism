# Follow-up: the `k8s-titan` half of the Postgres move

Status: **done — 2026-11-15.** The `k8s-titan` half landed as two PRs, split on purpose: **#296**
created the CNPG `Database`, the `DatabaseRole` and the credential Secret, and **#297** rolled the
image and deleted the inline SQLite `DATABASE_URL`. Merging #296 started no clock; merging #297 was
the cutover. The live app is Postgres-backed as of `6c0f7de` in that repo, and the record of what
actually happened — including the audit exiting 2 on a schema assumption — is in
`docs/postgres-cutover.md`, *Execution record*. What follows is kept as the reasoning, not as a
task list; where it says “not started” or “has no cluster to point at”, that was true when it was
written.

Source: §14 of `docs/superpowers/specs/2026-11-14-postgres-only-design.md`. That spec
deliberately stopped at the application: the portfolio-prism Postgres-only build is merged
and has no cluster to point at yet. The work below is **its own change in the `k8s-titan`
repo**, with that repo's own gates (`make portfolio-prism-check`, `make check`) and its own
PR. Nothing in this repository can validate it — there is no cluster here, and the
application-side runbook (`docs/postgres-cutover.md`) ends with a pointer to this file for
exactly that reason.

## 1. The scope, quoted from spec §14

> - A CNPG `Database` and `DatabaseRole` for portfolio-prism on the existing
>   `postgres` cluster in `databases` — the cluster already hosts coder and
>   authentik, so the pattern exists to copy.
> - The credential Secret, and how `DATABASE_URL` reaches the deployment.
> - **Deploy ordering is the hard part and it belongs in that spec:** the Postgres
>   build refuses to start without `DATABASE_URL` (§10), so the database and its
>   Secret must exist before the image rolls out, and the cutover (§11) must happen
>   between the two. With `replicas: 1` and `Recreate`, a badly ordered apply is a
>   crash-looping app with no serving replica, not a graceful degradation. (Confirm
>   `.spec.strategy.type` on the live deployment before relying on that — see §2.)
> - Whether the SQLite PVC and its restic backup are retired, and when.

Each of those is a section below, except the third, which is §2 because it is the one that
causes an outage.

## 2. Deploy ordering — read this before you apply anything

**The Postgres build refuses to start without `DATABASE_URL`. That refusal is the safety
property, and it is also the failure mode: with `replicas: 1` and a `Recreate` strategy, an
image that rolls out before its database and Secret exist is a crash-looping app with no
serving replica — not a graceful degradation, and not something a rollback of the image
fixes without also rolling the data story back.**

`Recreate` means the old pod is torn down *before* the new one is created. There is no
overlap in which both versions serve, so any reason the new pod has to exit is downtime for
the whole app.

**Confirm the strategy on the live deployment before you rely on the word `Recreate` anywhere
in these documents.** `k8s/app-deployment.yaml` in this repository declares no
`spec.strategy` at all, and the default is `RollingUpdate` — under which the old pod keeps
serving and the shape of the outage below is softer. The manifests in this repo are known to
be stale (see #61) — so stale that the command this section used to give,
`kubectl -n investment-portfolio-manager get deploy app`, names a namespace that exists on no
cluster. The live object is the only authority, and it is on titan:

```sh
kubectl --context titan -n apps get deploy portfolio-prism -o jsonpath='{.spec.strategy.type}'
```

Measured 2026-11-15 and again 2026-11-27: it prints `Recreate`. The live manifest is
`k8s-titan/apply/50-apps/portfolio-prism/portfolio-prism.yaml`, not anything in this repo; see
`k8s/README.md`.

If it answers `RollingUpdate`, the ordering rule below does **not** relax: a new pod that
cannot start never becomes ready, so you get a half-deployed rollout and a stale pod instead
of a clean crash loop, which is harder to notice and harder to undo.

The reasons this build exits are all configuration: no `DATABASE_URL`, a
`sqlite:` `DATABASE_URL`, a `DATABASE_URL` it cannot connect to, migrations that fail, or
orphaned rows with no `BOOTSTRAP_OWNER_SUBJ` to hand them to. Every one of those is an
operator mistake, and every one of them looks the same from outside: `CrashLoopBackOff`.

The order, with the reason attached to each step:

1. **CNPG `Database` and `DatabaseRole`**, and wait for both to report `Ready`. The
   `Database` is what makes the schema's home exist; the `DatabaseRole` is what makes a
   credential exist to reach it. Applying anything before this is the mistake the warning
   above is about.
2. **The Secret.** CNPG writes the role's password into its own secret
   (`<cluster>-<role>`); the deployment needs `DATABASE_URL` assembled from host, port,
   database and that password. Decide explicitly which mechanism assembles it (a sealed
   secret, SOPS, external-secrets, whatever `k8s-titan` already uses for authentik and
   coder — copy that, do not invent a third) and make sure the resulting key lands in
   `backend-secrets` as `DATABASE_URL`.
   The deployment's `envFrom` lists `backend-config` **then** `backend-secrets`, and a key
   present in a later source overrides the earlier one, so the Secret's `DATABASE_URL` wins
   over the ConfigMap's. That is the mechanism: rely on it once, on purpose, and **delete**
   the ConfigMap's `DATABASE_URL` key in the same apply so there is exactly one place the URL
   lives. A placeholder left behind in the ConfigMap is a trap for whoever edits it next, and
   deleting that key before this Secret exists is the outage — which is why 1, 2 and 3 are one
   atomic set and not three tickets.
3. **The image**, only now, in the same apply that removes the ConfigMap key. It starts
   against an empty database, applies `backend_rust/migrations/` itself
   (`sqlx::migrate!`), and — if any portfolio has no owner — needs `BOOTSTRAP_OWNER_SUBJ` set
   to the `sub` that inherits them, or it refuses to start. Read `docs/auth-testing.md` §6
   before setting that; the transfer is one-way.
4. **The cutover** (`docs/postgres-cutover.md`) straddles the three above rather than following
   them: its step 1 (audit) and step 2 (export) run against the still-serving SQLite app and
   so happen **before** 3, its step 3 (stop writes) is the moment the old deployment goes
   quiet, and its step 5 (restore) lands in the new database. The runbook's numbers and this
   list's numbers are different sequences; do not merge them in your head.
5. **Only after the runbook's step 7** (Postgres has survived one backup cycle of its own)
   does anything get retired.

Rollback if the order goes wrong: the SQLite deployment is still there until item 5 of this
list, so scale it back up. That is the whole reason §4 exists. Anything written to Postgres
after a restore is in no bundle until you export again — say it out loud before you press
restore, not after.

## 3. The `Database`, the `DatabaseRole`, and the Secret

- Cluster: the existing `postgres` cluster in the `databases` namespace. It already hosts
  coder and authentik, so the pattern is there to copy — copy it rather than composing a
  new shape.
- Version: **18**. The migrations were written and verified against PostgreSQL 18 (the
  local harness runs 18.7, production is expected on 18.6; same major, nothing in the
  schema depends on the difference).
- The role needs `CREATE` on its own database — the app runs `sqlx::migrate!` at startup and
  the migrations are the schema. A role that can only `SELECT`/`INSERT` cannot start the
  app.
- Pool sizing: the app opens **at most 5 connections** (`db::pool_options`,
  `max_connections(5)`, `acquire_timeout` 5 s, `min_connections 0`) and sets
  `statement_timeout = 30s` per connection. CNPG's default `max_connections=100` leaves
  room for two replicas with no pooler. If a pooler is added later, note that
  transaction-mode pooling and a `SERIALIZABLE` restore transaction are a subtle
  combination and deserve their own measurement, not a checkbox.
- The Secret shape is in `k8s/secret.example.yaml`: `DATABASE_URL` alongside
  `OIDC_CLIENT_SECRET` and `ROCKET_SECRET_KEY`.

## 4. What happens to the SQLite PVC and its restic backup

Unanswered on purpose, and it needs a date rather than a cleanup.

- `k8s/pv.yaml` / `k8s/pvc.yaml` in this repository are the file-backed volume the old
  image wrote to. They are the rollback path until the Postgres instance has survived one
  backup cycle of its own (`docs/postgres-cutover.md` step 7).
- The restic schedule that backs up that PVC has to be replaced by a backup of the CNPG
  cluster **before** the PVC goes away, not after. Two things named "backup" that both stop
  working is the failure this section exists to prevent.
- `k8s/` in this repository is separately known to be stale and not what the live cluster
  runs (see *Storage* in `README.md` and #61). Reconciling those manifests is that issue's
  job; this section's job is only to make sure the storage retirement is a decision with a
  date on it.

## 5. What "done" means

- `make portfolio-prism-check` and `make check` in `k8s-titan` pass.
- The app starts with no `startup refused:` line, and `psql` against the new URL shows the
  migrations applied (`select version, description from _sqlx_migrations order by version`).
- The cutover runbook has been executed for real, end to end, **including its step 1 against
  the actual production SQLite file** — that audit has never run against a real file, only
  against synthetic ones; the runbook's *Known limitations* says so in its own preamble and
  carries the transcript of the runs that were done.
- The counts in runbook step 6 were recorded, and the report comparison passed.
- A backup of the Postgres instance exists, and was restored somewhere to prove it.
- Only then: the PVC and the restic schedule are retired, in writing, with the date.
