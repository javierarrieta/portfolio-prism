# Headless browser smoke

`e2e/run.sh` starts the real stack and drives a real Chromium through a smoke
target: `sell-pnl.smoke.mjs` by default, or the printable statement with
`statement.smoke.mjs`. It is the only test here that exercises the seam between the
two halves of the app: the vitest suite mocks `/api/transactions/<id>/pnl` with MSW,
and the cargo tests never render anything. Neither can see a wrong fetch URL, a
missing Vite proxy rule, or a table column too narrow to hold a date.

## Running it

CI runs both of these on every push to `main` and every pull request, as the
`e2e-smoke` job. The steps below are for running them by hand.

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

## Running it in CI

The `e2e-smoke` job in `.github/workflows/ci.yml` runs both smokes on every push to
`main` and every pull request, and it is **blocking**. A browser job with
`continue-on-error: true` reads as coverage while checking nothing, which is worse
than no job at all.

Four things the job has to do that a local run does not:

*   **Install a browser.** `playwright-core` ships the downloader but, unlike
    `playwright`, has no postinstall that runs it, so the job calls
    `npx playwright-core install --with-deps chromium` itself. `--with-deps` also
    pulls the distro font packages, which is what stops currency symbols rendering
    as tofu in the statement PDF; it needs sudo, which GitHub-hosted runners have
    passwordless. Not `--only-shell` — both smokes launch a full browser with
    `--no-sandbox --disable-dev-shm-usage`, and the headless shell is not
    interchangeable for that.
*   **Hand `run.sh` the binary.** The install and the smoke are separate steps, so
    the resolved `chromium.executablePath()` is written to `$GITHUB_ENV` as
    `CHROMIUM`.
*   **Pay for a debug build.** `run.sh` starts the API with `cargo run`, so the job
    compiles the crate from scratch. That is ~86 s on a runner and the job finishes in
    ~3m15s, so there is deliberately **no cargo cache**: it would save about a minute
    while storing half a gigabyte against the shared 10 GB per-repo budget. The step
    comment on `e2e-smoke` records the numbers it was judged on.
*   **Keep the logs.** `run.sh` deletes its temp dir on exit, so the job sets
    `SMOKE_LOG_DIR` per smoke and uploads both server logs and both screenshots with
    `if: always()`.

The stack boots twice, once per smoke, because each `run.sh` invocation owns its own
throwaway database, backend and vite. The second boot is cheap — the binary is
already compiled.

### Traps that are already handled — do not re-break them

*   **`AUTH_MODE=disabled`.** With no `AUTH_MODE` and no `backend_rust/.env` the
    server asks for OIDC, bails naming four missing variables
    (`backend_rust/src/auth/mod.rs:136-138`), and `wait_for backend` burns its whole
    420-second budget. `run.sh` exports it
    beside `DATABASE_URL`, following the precedent at `start.sh:21`. That is safe
    only because `run.sh` builds debug; `disabled` is compiled out of release builds
    (`auth/mod.rs:96-101`).
*   **The target defaults through `${1:-sell-pnl.smoke.mjs}`, not `$1`.** `run.sh`
    runs under `set -uo pipefail`, so a bare `$1` aborts the script when called with no
    argument and breaks `npm run e2e:sell-pnl`, which calls it with none.
*   **Do not pin prices by writing to `historical_prices`.** No API route writes
    that table — the only writers are internal cache writes after a successful
    provider fetch (`currency_service.rs:397`, `:490`) — so pinning would mean
    writing SQLite out-of-band, against this harness's own REST-seeding rule.
    It also couples the smoke to a schema the pricing-layer work is actively
    revising. A smoke that asserts no figure cannot fail on a zero price, so
    there is nothing for it to buy.
