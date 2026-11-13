# Backup bundle schema versions

`GET /api/backup/export` produces a JSON bundle; `POST /api/backup/restore`
consumes one. This directory is the normative record of what each version of that
format means. The Rust structs in `backend_rust/src/backup/mod.rs` are the source
of truth; the `vN.json` files here are generated from them by
`cargo run --bin backend_rust -- --print-backup-schema` and CI diffs the result,
so a committed file that disagrees with the code fails the build.

## Policy

- **A released `vN.json` is never edited in a way that changes what validates.** The
  rule protects validation semantics and the meaning of `schema_version`, not the
  bytes of the file: a bundle that validated against `vN` the day it shipped must
  validate against `vN` for as long as `ACCEPTED_VERSIONS` lists it, because
  `schema_version` is the only claim a backup that is already written makes about its
  own shape.
- **Anything that changes what validates needs `vN+1.json` and a `vN -> vN+1`
  upgrader** in `backend_rust/src/backup/restore.rs`. The test is whether a bundle
  that was valid under `vN` would validate differently under the edited artifact: a
  field added to or removed from the format, a field renamed, a changed type or
  `required` set, a `pattern`, `format` or `enum` tightened or loosened, or a field
  whose meaning changes while its name does not.
- Two things sit outside that rule, and neither loosens it. A JSON Schema
  `description` is an annotation, so correcting prose in a committed artifact changes
  no validation outcome — which is why the drift guard in
  `backend_rust/src/backup/schema.rs` compares the file rather than freezing it. And
  while a version is unreleased, living only on the branch that introduces it, its
  artifact is still a working document and editing it is ordinary development. In
  both cases the invariant that matters is the drift guard's: the committed file is
  byte-identical to what this build prints.
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
- `ref` labels (`p1`, `a1`) are bundle-local, share one namespace across portfolios
  and assets, and are never database ids.
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
