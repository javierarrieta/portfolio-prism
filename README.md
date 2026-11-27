# Portfolio Prism

Full-stack portfolio management application with tax lot tracking (FIFO/LIFO/Hybrid) and analytics.

- **Backend**: Rust (Rocket) + PostgreSQL (SQLx)
- **Frontend**: React 19 + Vite + TypeScript + Recharts

---

## Local Development

```bash
# Install prerequisites
# Rust toolchain, Node.js 20+

# Run everything (backend on :8000, frontend on :5173)
./start.sh
```

- Frontend: <http://localhost:5173>
- Backend API: <http://127.0.0.1:8000>
- OpenAPI docs: <http://127.0.0.1:8000/api-docs/openapi.json>

---

## API Routes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api-docs/openapi.json` | OpenAPI spec |
| `POST` | `/api/portfolios/` | Create portfolio |
| `GET` | `/api/portfolios/` | List portfolios |
| `GET` | `/api/portfolios/:id` | Get portfolio |
| `DELETE` | `/api/portfolios/:id` | Delete portfolio |
| `POST` | `/api/portfolios/:portfolio_id/assets` | Add asset |
| `DELETE` | `/api/assets/:id` | Remove asset |
| `POST` | `/api/portfolios/:portfolio_id/assets/:asset_id/transactions` | Add transaction |
| `GET` | `/api/portfolios/:portfolio_id/transactions` | List transactions |
| `DELETE` | `/api/transactions/:id` | Delete transaction |
| `GET` | `/api/portfolios/:id/tax-summary` | Tax summary |
| `GET` | `/api/portfolios/:id/performance` | Performance analytics |
| `GET` | `/api/backup/capabilities` | Bundle limits and flags (`allow_price_restore`, `max_bytes`, `max_rows`) |
| `GET` | `/api/backup/export` | Export a bundle; `?portfolio_ids=1,2` (required), `?include_prices=1` |
| `POST` | `/api/backup/preview` | Preview a restore; `?mode=copy\|replace` (required), body is the bundle |
| `POST` | `/api/backup/restore` | Restore a bundle; `?mode=copy\|replace` (required), `?apply_prices=1` (needs the flag below) |

The four backup routes are authenticated like every other `/api` route. Their bundle format is
`docs/backup-schema/v1.json` and the manual walkthrough is `docs/walkthrough.md`.

---

## CSV Encoding

Exports are UTF-8 **with a byte-order mark** (`EF BB BF`). It is added once, in
`frontend/src/utils/download.ts`, so every export this app grows is covered by construction;
the CSV builders stay BOM-free and `downloadCsv` refuses to add a second one. Imports accept
a BOM and strip it.

| consumer | behaviour with the BOM present |
|----------|--------------------------------|
| Excel (Windows or Mac) | needs it. Without one Excel decodes the file as the machine's ANSI codepage and any non-ASCII cell arrives mojibake'd |
| Google Sheets | detects UTF-8 and drops the BOM on import |
| this app's own CSV import | strips it, via `Papa.BYTE_ORDER_MARK` |
| Python `utf-8`, `pandas.read_csv` | keeps it: the first column comes back as `\ufeffrow_type`. Read these files with `encoding='utf-8-sig'`, which is correct whether or not a BOM is there (pandas issues [#4793](https://github.com/pandas-dev/pandas/issues/4793), [#13497](https://github.com/pandas-dev/pandas/issues/13497)) |

So: a new programmatic consumer should be pointed at `utf-8-sig`. Do not drop the BOM to
spare one reader — that silently re-breaks Excel, and nothing in the test suite will notice
except the byte assertions in `frontend/src/utils/download.test.ts` and `frontend/e2e/`.

---

## Kubernetes Deployment

The application is packaged as two Docker images (backend and frontend) and can be deployed to any Kubernetes cluster using the manifests below.

### Prerequisites

- Kubernetes cluster (v1.24+)
- `kubectl` configured to communicate with the cluster
- Container registry accessible by the cluster (images published via `ghcr.io`)

### Building and Pushing Images

```bash
# Build and push backend
docker build -t ghcr.io/<org>/investment-portfolio-manager/backend:latest -f backend_rust/Dockerfile .
docker push ghcr.io/<org>/investment-portfolio-manager/backend:latest

# Build and push frontend
docker build -t ghcr.io/<org>/investment-portfolio-manager/frontend:latest -f frontend/Dockerfile frontend/
docker push ghcr.io/<org>/investment-portfolio-manager/frontend:latest
```

Replace `<org>` with your GitHub username or organization.

### Single-File Deployment

```bash
kubectl apply -f k8s/
```

**Only while the app still speaks the storage these manifests describe.** It does not any
more: the current image is Postgres-only and `k8s/configmap.yaml` still ships a `sqlite:`
DSN, so this one command is exactly the outage `docs/postgres-cutover.md` documents — the
new pod refuses its configuration and there is no serving replica. Read that document's
*What changed in the manifests, and in which order* before applying any of these files, and
note that the atomic cutover apply is deliberately **not** `kubectl apply -f k8s/`: the CNPG
`Database`, the `DatabaseRole` and the Secret must already exist and be Ready. Also confirm
`.spec.strategy.type` on the live deployment first — `k8s/app-deployment.yaml` declares no
`spec.strategy`, so the manifest does not tell you whether you are about to cause a
`Recreate` outage or a silently crash-looping rollout.

### Resource Overview

```
k8s/
  namespace.yaml          # Namespace
  storageclass.yaml       # StorageClass
  configmap.yaml          # Backend configuration
  pv.yaml                 # Persistent volume for the (pre-cutover) file-backed database
  pvc.yaml                # Persistent volume claim
  app-deployment.yaml     # Single pod: frontend + backend containers
  app-service.yaml        # ClusterIP service (:80)
  ingress.yaml            # Ingress routing all traffic to frontend
```

### Architecture

```
Ingress (host: invest.example.com)
  └── /          → app-service:80

Single Pod
  ├── frontend (nginx, :80)
  │   ├── serves static React build at /
  │   └── proxies /api/* and /api-docs → 127.0.0.1:8000
  │
  └── backend (Rocket, :8000)
      └── DATABASE_URL → Postgres (the ConfigMap still names a SQLite file; see Storage)
```

Both containers run in the same pod and communicate over `127.0.0.1`. The frontend nginx proxies all API requests (`/api/*`, `/api-docs/*`) to the backend. The browser only talks to the frontend — no cross-service calls are needed.

There is **no init container**. An earlier one curled `http://127.0.0.1:8000/` from an
init phase to wait for the backend, but init runs *before* the backend container exists,
so it could only ever exhaust its retries and exit 1 — delaying every pod start by 60
seconds and never succeeding. The frontend's nginx tolerates the backend not being up
yet; a container proxying to a not-yet-listening upstream returns `502` and the browser
retries, which is cheaper than a probe that cannot succeed by construction.

### Storage

The backend connects to **PostgreSQL** and nothing else: `DATABASE_URL` is required, has
no default, and the process refuses to start without it. In development
`backend_rust/scripts/dev-db.sh` brings up a local cluster on `127.0.0.1:55432` and
prints the URL to export; `./start.sh` does that for you if the environment does not
already set one. Schema comes from `backend_rust/migrations/` at startup.

Only `k8s/configmap.yaml` still ships a `sqlite:` DSN, and it does so on purpose: a running
deployment consumes that manifest, so changing it is an ordered cutover step — database and
Secret first, image last — rather than a commit. See `docs/postgres-cutover.md` (*What
changed in the manifests, and in which order*) and
`docs/superpowers/specs/2026-11-14-postgres-gitops-followup.md`. `backend_rust/Dockerfile`
no longer ships one: it has no `DATABASE_URL`, no `libsqlite3`, and no `/app/data` volume,
so a container started without an injected URL exits immediately and names the variable.
Until the cutover the persistent volume below is unused by the current image.

> **The manifests in `k8s/` are stale and do not describe the running deployment.** The
> StorageClass is *named* `nfs-client`, but its provisioner is
> `k8s.io/minikube-hostpath` — the name is a copied convention, not a description, and
> nothing here has ever been NFS. The live app runs in the `apps` namespace against a
> `local-path-retain` PVC on the node filesystem, with `replicas: 1` and a `Recreate`
> strategy. Confirm `.spec.strategy.type` on the live deployment before relying on that: the
> manifests declare no `spec.strategy`, whose default is `RollingUpdate`. Reconciling or
> deleting these manifests is tracked in #61. Do not infer the storage backend from a
> StorageClass name; check `spec.provisioner` and the PV's volume source.

### Environment Variables

Backend environment variables are configured in the ConfigMap:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | *none — required* | Postgres connection string. There is no default and no SQLite support; a `sqlite:` URL is refused at startup. Development: `postgres://postgres@127.0.0.1:55432/prism`, printed by `backend_rust/scripts/dev-db.sh`. |
| `BACKUP_ALLOW_PRICE_RESTORE` | `false` | Let `POST /api/backup/restore` write the bundle's `prices` section. **Security-relevant:** `historical_prices` has no `owner_id`, so a restore that writes it is one tenant writing market data every other tenant's dated reports read. `1/true/yes` enable, `0/false/no` disable, anything else stays off and logs a warning, and the backend prints a `WARN` on every start with it on. Deliberately **absent** from `k8s/configmap.yaml`, so the deployed app has it off. See `docs/backup-schema/README.md` and `docs/walkthrough.md` §4 step 6. |
| `MCP_ALLOWED_HOSTS` | `localhost,127.0.0.1,::1` | Which `Host` values `/mcp` will answer, as that header arrives at the backend — behind the shipped nginx that is the public hostname. **Security-relevant and required for any real deployment:** the default is rmcp's loopback list, and the check runs *after* the bearer token is accepted, so an unconfigured host answers every MCP call `403 Forbidden: Host header is not allowed` with a perfectly valid token — it reads as a broken credential, not as missing configuration. Comma-separated bare authorities, **no scheme** (`portfolio.example.com`, not `https://portfolio.example.com`); a portless entry matches any port; `*` disables the check and logs a `WARN`. Set it to the host your ingress serves. |

### Scaling

The backend is stateless now that it talks to a Postgres server, so replicas are a
matter of pointing every pod at the same `DATABASE_URL`. What still holds it to one
replica is the deployment: `k8s/` ships a file-backed volume and a `sqlite:` DSN in its
ConfigMap (see *Storage* and `docs/postgres-cutover.md`), and a `Recreate` rollout strategy
— confirm `.spec.strategy.type` on the live deployment, because `k8s/app-deployment.yaml`
declares no `spec.strategy` and the default is `RollingUpdate`.

> **Do not follow the previous advice in this section**, which suggested "SQLite with WAL
> mode and NFS". SQLite's own WAL documentation rules that out: *"All processes using a
> database must be the same host computer; WAL does not work over a network filesystem."*
> WAL coordinates through a shared-memory `-shm` file, and on NFS that produces
> `SQLITE_CORRUPT` rather than contention. Scaling SQLite across hosts is not a
> supported configuration by any route.

The supported path is migration to PostgreSQL (or another managed RDBMS). The manifests
are written to be adaptable for a stateless backend against a managed database.

The default deployment runs a single pod, and `--replicas=3` is still not meaningful until
the cutover has happened: a Postgres service or managed instance in the cluster, the
`sqlite:` DSN in `k8s/configmap.yaml` replaced by the Secret's `DATABASE_URL`, and the
file-backed `pv.yaml`/`pvc.yaml` retired. That is
`docs/superpowers/specs/2026-11-14-postgres-gitops-followup.md`, and the data side is
`docs/postgres-cutover.md`. The nginx proxy pass in the frontend container already makes
horizontal scaling straightforward once those are done.

```bash
# Not a supported configuration yet — see above. Kept here only to record that it was tried
# and why it is wrong, and so that nobody re-derives it: scaling the deployment while the
# ConfigMap still names a file database gives every replica its own empty file.
# kubectl scale deployment backend -n investment-portfolio-manager --replicas=3
```

### Cleanup

```bash
kubectl delete namespace investment-portfolio-manager
```

---

## Project Structure

```
.
├── backend_rust/          # Rust backend (Rocket + SQLx)
│   ├── src/
│   │   ├── api_routes/    # HTTP route handlers
│   │   ├── engines/       # Tax engine (FIFO/LIFO/Hybrid) + analytics
│   │   └── services/      # Domain services
│   ├── Dockerfile
│   └── Cargo.toml
├── frontend/              # React + Vite frontend
│   ├── src/
│   ├── Dockerfile
│   ├── nginx.conf         # Proxies /api/* to backend sidecar
│   └── package.json
├── k8s/                   # Kubernetes manifests (single-pod deployment)
├── scripts/               # Utility scripts
└── start.sh               # Local dev launcher
```

---

## License

Apache License 2.0 — see [LICENSE](LICENSE).
