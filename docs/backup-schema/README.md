# Backup bundle schema versions

`GET /api/backup/export` produces a JSON bundle; `POST /api/backup/restore`
consumes one. This directory is the record of what each version of that format
means, and it holds the **structural** definition of each version: field names,
types, nullability, `required`, and the two `enum` sets named below. The `vN.json`
files are generated from the Rust structs in `backend_rust/src/backup/mod.rs` by
`cargo run --bin backend_rust -- --print-backup-schema`, and CI diffs the result,
so a committed file that disagrees with the *structs* fails the build.

## What validates, and what does not

**The normative validator is `collect()` in `backend_rust/src/backup/mod.rs`.** It
is hand-written, it runs on every `preview` and every `restore`, and it is the only
thing that decides whether a bundle is accepted. Nothing at runtime reads a `vN.json`
file: there is no JSON-Schema validator in the dependency path, and the only
`include_str!` of `v1.json` in the repository is the drift-guard test in
`backend_rust/src/backup/schema.rs` that compares the committed bytes with what this
build prints. A `vN.json` therefore cannot disagree with the *shape* of the structs
and cannot disagree with the two *enumerations* — and is otherwise free to disagree
with the rules, which is why the rules are listed here rather than only there.

The rules `collect()` enforces that the artifact **cannot express**, each with where
it lives:

- **Currency codes are three characters** — `portfolios[].currency`,
  `portfolios[].base_currency`, `assets[].currency`. No `pattern` in the artifact
  says so.
- **Transaction dates fall inside 1900-01-01 to 2100-01-01**, the window
  `create_transaction` already enforces. The artifact says `type: string` and nothing
  more about the range.
- **Names and symbols must not be blank** — `portfolios[].name`, `assets[].name`,
  `assets[].symbol`, `prices[].symbol` are rejected when empty or whitespace. The
  artifact requires the *key*, not a non-empty value.
- **`ref` labels are unique bundle-wide** — portfolio refs and asset refs share one
  namespace, so two rows may not claim one label, and an asset ref may not equal a
  portfolio ref. JSON Schema has no way to say "unique across this array and that
  one".
- **A SPLIT must have a positive quantity and a positive price**, mirroring the
  ordinary write path (`api_routes/transactions.rs`). `"0"` is a perfectly valid
  `"type": "string"`.
- **An ISIN, when present, must be a valid ISIN** — twelve characters, mod-10 check
  digit (`api_routes::lookup::is_valid_isin`).
- **Row and byte ceilings** — `MAX_BUNDLE_ROWS` (portfolios + assets + transactions
  + prices) and `MAX_BUNDLE_BYTES`, enforced on export, on preview and on restore.
  No JSON-Schema keyword describes a count of rows across nested arrays.
- **`format` must equal `portfolio-prism-backup` and `schema_version` must be in
  `ACCEPTED_VERSIONS`.** The artifact types both; the accepted set lives in the
  constant, and a bundle outside it is refused before field validation runs.

The two exceptions, and the only enumerations the artifact carries: **the asset-type
set (`KNOWN_ASSET_TYPES`) and the transaction-type set (`KNOWN_TRANSACTION_TYPES`)**
appear as `"enum"` on `assets[].asset_type` and `transactions[].type`. They are
generated from the same constants `collect()` compares against — `schema_with`, not a
second list — and
`the_artifact_enumerates_the_two_type_fields_from_the_validating_constants` fails if
the two ever part company. Everything else above is prose in this file and code in
`collect()`; a `vN.json` that omits it is not stale.

## Policy

- **What validates never changes for a released `vN`.** The rule protects the
  semantics of `collect()` and the meaning of `schema_version`, not the bytes of any
  file: a bundle that validated against `vN` the day it shipped must validate against
  `vN` for as long as `ACCEPTED_VERSIONS` lists it, because `schema_version` is the
  only claim a backup that is already written makes about its own shape.
- **Anything that changes what validates needs `vN+1.json` and a `vN -> vN+1`
  upgrader** in `backend_rust/src/backup/restore.rs`. The test is whether a bundle
  that was valid under `vN` would validate differently under the edited rule — and
  the rule may live in `collect()` rather than in the artifact: a field added to or
  removed from the format, a field renamed, a changed type or `required` set, a
  `pattern`, `format` or `enum` tightened or loosened, a rule added or dropped in
  `collect()` (a currency rule, a date window, a ceiling), or a field whose meaning
  changes while its name does not.
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
- Decimals (`quantity`, `price`, `fee`, `close_price`) are strings. They are plain
  `String` fields on the structs, which is why the artifact says `"type": "string"`
  — there is no `rust_decimal`/`NaiveDate` schema override anywhere in the build, and
  none is needed: the format never carries those Rust types, only their text.
- `transactions[].date` is an RFC 3339 UTC datetime; `prices[].date` is
  `YYYY-MM-DD`. The two differ because the columns differ, and normalising them
  would mean rewriting stored data.
- `prices[]` is optional and, when present, is global market data rather than
  authored data. Restoring it is disabled unless the server sets
  `BACKUP_ALLOW_PRICE_RESTORE=true`.
- Unknown fields are rejected, not ignored.
- `assets[].asset_type` and `transactions[].type` are enumerated, and the sets are
  the application's own vocabulary — `STOCK`, `CRYPTO`, `ETF`, `MUTUAL_FUND` and
  `BUY`, `SELL`, `SPLIT`. Every other rule is in *What validates, and what does not*
  above.

## Regenerating

```bash
cd backend_rust
cargo run --bin backend_rust -- --print-backup-schema > ../docs/backup-schema/v1.json
```
