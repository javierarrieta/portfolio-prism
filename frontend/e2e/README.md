# Headless browser smoke

`e2e/run.sh` starts the real stack and drives a real Chromium through a smoke
target: `sell-pnl.smoke.mjs` by default, or the printable statement with
`statement.smoke.mjs`. It is the only test here that exercises the seam between the
two halves of the app: the vitest suite mocks `/api/transactions/<id>/pnl` with MSW,
and the cargo tests never render anything. Neither can see a wrong fetch URL, a
missing Vite proxy rule, or a table column too narrow to hold a date.

## Running it

```bash
cd frontend
npm install
npm run e2e:sell-pnl     # the sell P&L breakdown
npm run e2e:statement    # the printable statement, under emulated print media
```

That starts `cargo run` and `vite`, waits for both, seeds a fixture through the
REST API, drives the browser, and tears everything down. Exit code 0 means the
breakdown rendered and reconciled (sell-pnl) or the statement rendered under print
media and produced a PDF (statement).

`e2e:statement` is the only test in the repo that sees real print CSS: jsdom does not
apply `@media print`, so the vitest suite can only check `print.css` as text. It
asserts structure, visibility and colour and **no figure** — figures come from a live
price provider, and a smoke that asserted them would print zeros and still pass.

### Prerequisites

- **A chromium-compatible browser.** Resolved from `$CHROMIUM`, then
  `chromium`, `chromium-browser`, `google-chrome` on `PATH`.
- **Fonts.** A browser with no fonts renders zero-width text: the layout looks
  correct and every glyph is missing. `run.sh` synthesises a `fontconfig` file
  when the host has none, and the smoke measures glyph width up front so the
  failure names the cause instead of looking like a broken component.
- `playwright-core` (a devDependency; it downloads no browser of its own).

On **NixOS** neither chromium nor fonts are on `PATH` by default:

```bash
nix-shell -p chromium -p liberation_ttf --run 'cd frontend && ./e2e/run.sh'
```

### Useful env

| Variable | Effect |
| --- | --- |
| `CHROMIUM` | browser binary to launch |
| `FONT_DIR` | extra font directory for the generated `fontconfig` file |
| `SMOKE_SHOT=/tmp/x.png` | write a full-page screenshot to this path. `e2e:statement`: on any failure, whether it threw or was only asserted. `e2e:sell-pnl`: once the modal has been read (before its assertions run) and again if the run throws — an assertion failure that does not throw writes nothing, even though `run.sh` still names the path |
| `BASE_URL` / `API_URL` | override where the app and API live |

## What it asserts

Seeded through the API: buy 60 @ 100, buy 40 @ 120, sell 80 @ 130. FIFO must
split that sell across both lots.

1. The realized figure in the modal equals what `/portfolios/<id>/tax-summary`
   reports for the same asset — read over the API, not from the DOM.
2. The per-lot P&L column **sums** to that figure. This is the invariant the
   whole feature exists to provide.
3. The strategy note names the method that actually ran.
4. Buy dates render on one line box, measured with `Range.getClientRects()`.
   `textContent` cannot see wrapping; only a layout engine can.
5. No page errors reached the console.

## What it does not prove

It adds nothing about the arithmetic. FIFO/LIFO/HYBRID attribution and the
reconciliation invariant are pinned by the backend test suite; a browser cannot
improve on that. The FX conversion path is still untested end to end — every
fixture in the repo is USD-into-USD, so `get_rate` is never reached.

## Why it is not in CI

CI would need a browser plus a booted backend on every push, for a check whose
value is catching wiring mistakes rather than logic errors. Run it before
releasing a change that touches the ledger, the modal, or the endpoint.

This is a choice about CI cost, not an impossibility, and the details are worth
recording before anyone tries to add the job:

*   **A CI browser job is cheap to start.** CI runs `ubuntu-latest`, where
    `npx playwright install chromium` is one line, and `emulateMedia` and
    `page.pdf()` need no display server.
*   **The job is not browser-only.** `run.sh` compiles and starts the Rust API
    against a throwaway SQLite file and starts Vite, so the job costs a full
    cargo build and needs seeded data. A `continue-on-error` job nobody reads
    would read as a pass; the useful form is a blocking job.
*   **The harness needs `AUTH_MODE=disabled` or the backend refuses to launch.**
    With no `AUTH_MODE` and no `backend_rust/.env` the server asks for OIDC, bails
    naming four missing variables (`backend_rust/src/auth/mod.rs:136-138`), and
    `wait_for backend` burns its whole 420-second budget. `run.sh` now exports it
    beside `DATABASE_URL`, following the precedent at `start.sh:21`. That is safe
    only because `run.sh` builds debug; `disabled` is compiled out of release builds
    (`auth/mod.rs:96-101`).
*   **The target defaults through `${1:-sell-pnl.smoke.mjs}`, not `$1`.** `run.sh:16`
    is `set -uo pipefail`, so a bare `$1` aborts the script when called with no
    argument and breaks `npm run e2e:sell-pnl`, which calls it with none.
*   **Do not pin prices by writing to `historical_prices`.** No API route writes
    that table — the only writers are internal cache writes after a successful
    provider fetch (`currency_service.rs:397`, `:490`) — so pinning would mean
    writing SQLite out-of-band, against this harness's own REST-seeding rule.
    It also couples the smoke to a schema the pricing-layer work is actively
    revising. A smoke that asserts no figure cannot fail on a zero price, so
    there is nothing for it to buy.
