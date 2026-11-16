# A gzipped bundle container

Status: **proposed, not started.** Recorded 2026-11-15, alongside the upload-path fix that
motivated it.

## Scope: the upload direction only

The response side is already handled where it belongs — nginx, in `frontend/nginx.conf`,
compressing on the way out. That costs one config block, changes no format, and the browser
still saves a plain `.json`.

This document is about the **upload**, where the file itself has to be compressed, because a
browser will not compress a request body. Nothing here is urgent: `MAX_BUNDLE_ROWS` bounds a
bundle to a few megabytes, and the 413 that prompted the discussion was a proxy misconfiguration
that has been fixed. Write it down so that if the cap ever has to move, the shape of the change
is already thought through.

## The measured case for it

A real bundle — three portfolios, three assets, 38 transactions and 13,668 deduplicated price
rows:

| | bytes |
| --- | --- |
| plain JSON | 1,284,105 |
| `gzip -9` | 113,036 |
| ratio | **11.4×** |

Repetitive JSON compresses extremely well. The same bundle would have cleared nginx's 1 MiB
default without the fix, which is the argument for it. The counter-argument is that the fix was
one line and the bundle is already inside every limit that matters.

## Container, not schema

A gzipped bundle is a new **container**, not a new schema. The JSON inside is still `v1`, so
`docs/backup-schema/v1.json` does not change and no upgrader is needed. What does change:

- The restore must accept **both containers, forever** — the same promise already made about
  schema versions.
- `GET /api/backup/capabilities` has to advertise which containers this server accepts, so a
  client is not guessing from a failure.
- Detection is by **magic bytes** (`1f 8b`), not by `Content-Encoding`. A proxy may add, strip,
  or fail to honour that header; the first two bytes of the body cannot lie about what arrived.

## The cap semantics — the part that has to be right

`MAX_BUNDLE_BYTES` (8 MiB) currently bounds the byte stream the route reads, via
`data.open(MAX_BUNDLE_BYTES.bytes())`. That single meaning has to split in two:

| limit | bounds |
| --- | --- |
| proxy `client_max_body_size` | **compressed** bytes on the wire |
| `MAX_BUNDLE_BYTES` | **decompressed** bytes handed to the validator |

Getting this backwards is a decompression bomb: 12 MiB of gzip can expand to gigabytes, and
JSON is exactly the content type where a hostile input can be crafted to compress a thousand-fold
or more. So:

1. Gunzip as a **stream**, never into memory first.
2. Bound the **output** with a hard `take(MAX_BUNDLE_BYTES + 1)` and refuse on overflow — the
   same refusal the plain path already gives, so a bomb and an oversized plain bundle produce the
   same typed error rather than one of them producing an outage.
3. Never trust the decompressed size a client claims.

`scripts/check_upload_path.sh` asserts `proxy ≥ MAX_BUNDLE_BYTES`. With compression that
inequality compares a compressed number against an uncompressed one and no longer means what it
currently means. It needs a second clause — the proxy limit must be at least as large as the
compressed bundle the API is willing to accept, which in practice means the gate should assert
the proxy limit is ≥ `MAX_BUNDLE_BYTES` **and** document that the two are now different
quantities. A gate that silently stops meaning anything is worse than no gate.

## What it costs

- **The artifact stops being inspectable.** `jq`, `diff`, and opening a backup in an editor are
  real properties of a format you reach for when something has gone wrong. A `.json.gz` gives
  them up. Accepting both containers keeps them available, which is the main reason not to make
  compression mandatory.
- **The panel's pre-flight compares the wrong number.** `file.size` for a gzipped file is
  compressed bytes; the advertised cap is uncompressed. Either `capabilities` carries both, or
  the local refusal has to stop pretending it can predict the outcome for compressed input.
- **`readFileText()` becomes `arrayBuffer()`** on that path, and the body is sent as bytes.
- **Two code paths through validation forever**, plus a bomb fixture proving the second one
  cannot be made to expand.

## If it is built

- Accept `1f 8b` sniffing on `POST /api/backup/preview` and `/restore`; plain JSON keeps working
  unchanged and stays the default on export.
- Streaming inflate with the output cap above, and a test that feeds a real bomb — a small
  compressed input whose decompressed form exceeds the cap — and asserts a typed refusal, not a
  memory spike.
- `capabilities` gains the accepted containers and, if the pre-flight is to stay honest, a
  compressed-cap alongside the uncompressed one.
- `check_upload_path.sh` gains the second clause described above, with a mutation case proving
  it bites.
- Export keeps emitting plain JSON. Compressing the artifact by default would make every
  existing bundle and every `curl` example a special case for no gain the download path does not
  already provide.

## Do not

- Do not make gzip **required**. Every bundle already in existence, and every scripted `curl`,
  becomes a special case.
- Do not gunzip into memory to keep the handler simple. The cap is the whole feature.
- Do not treat this as a schema version bump. It is a container; `v1` JSON inside a gzip stream
  is still `v1`.
