# Headless browser smoke

`e2e/run.sh` starts the real stack and drives a real Chromium through the sell P&L
flow. It is the only test here that exercises the seam between the two halves of
the app: the vitest suite mocks `/api/transactions/<id>/pnl` with MSW, and the
cargo tests never render anything. Neither can see a wrong fetch URL, a missing
Vite proxy rule, or a table column too narrow to hold a date.

## Running it

```bash
cd frontend
npm install
npm run e2e:sell-pnl
```

That starts `cargo run` and `vite`, waits for both, seeds a fixture through the
REST API, drives the browser, and tears everything down. Exit code 0 means the
breakdown rendered and reconciled.

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
| `SMOKE_SHOT=/tmp/x.png` | write a full-page screenshot of the open modal |
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
