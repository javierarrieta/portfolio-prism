# Backup bundle schema versions

`GET /api/backup/export` produces a JSON bundle; `POST /api/backup/restore`
consumes one. This directory is the normative record of what each version of that
format means. The Rust structs in `backend_rust/src/backup/mod.rs` are the source
of truth; the `vN.json` files here are generated from them by
`cargo run --bin backend_rust -- --print-backup-schema` and CI diffs the result,
so a committed file that disagrees with the code fails the build.

## Policy

- **A committed `vN.json` is never edited.** A format change adds `vN+1.json` and a
  `vN -> vN+1` upgrader in `backend_rust/src/backup/restore.rs`.
- The build declares the versions it accepts in
  `backend_rust::backup::ACCEPTED_VERSIONS`. A bundle whose `schema_version` is
  outside that set is refused with a `422` naming the accepted versions, and
  nothing is written.
- **A released version is never dropped from `ACCEPTED_VERSIONS`.** Removing a
  version makes every backup already written in that format unrestorable, with no
  way for its owner to recover it, so the set only ever grows: a new format
  appends `vN+1` and ships the upgrader, and `vN` keeps being accepted. The
  `vN.json` file stays in this directory forever either way; it is history.
  `accepted_versions_are_exactly_v1` in `backend_rust/src/backup/mod.rs` is the
  tripwire that fails if a released version is removed.

## Versions

### v1 — `v1.json`

Introduced 2026-11-13 with the backup feature. Accepted by every build since.

- Nested structure: `portfolios[]` → `assets[]` → `transactions[]`. Only
  `portfolios` has an `owner_id` in the database, so nesting is what makes
  ownership structural.
- `ref` labels (`p1`, `a1`) are bundle-local and are never database ids.
- Decimals (`quantity`, `price`, `fee`, `close_price`) are strings.
- `transactions[].date` is an RFC 3339 UTC datetime; `prices[].date` is
  `YYYY-MM-DD`. The two differ because the columns differ, and normalising them
  would mean rewriting stored data.
- `prices[]` is optional and, when present, is global market data rather than
  authored data. Restoring it is disabled unless the server sets
  `BACKUP_ALLOW_PRICE_RESTORE=true`.
- Unknown fields are rejected, not ignored.

## Regenerating

```bash
cd backend_rust
cargo run --bin backend_rust -- --print-backup-schema > ../docs/backup-schema/v1.json
```
