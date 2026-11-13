# Portfolio Prism

Full-stack portfolio management application with tax lot tracking (FIFO/LIFO/Hybrid) and analytics.

- **Backend**: Rust (Rocket) + SQLite (SQLx)
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

Apply all resources in one step:

```bash
kubectl apply -f k8s/
```

### Resource Overview

```
k8s/
  namespace.yaml          # Namespace
  storageclass.yaml       # StorageClass
  configmap.yaml          # Backend configuration
  pv.yaml                 # Persistent volume for SQLite database
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
      └── SQLite via PVC → /app/data/portfolio.db
```

Both containers run in the same pod and communicate over `127.0.0.1`. The frontend nginx proxies all API requests (`/api/*`, `/api-docs/*`) to the backend. The browser only talks to the frontend — no cross-service calls are needed.

There is **no init container**. An earlier one curled `http://127.0.0.1:8000/` from an
init phase to wait for the backend, but init runs *before* the backend container exists,
so it could only ever exhaust its retries and exit 1 — delaying every pod start by 60
seconds and never succeeding. The frontend's nginx tolerates the backend not being up
yet; a container proxying to a not-yet-listening upstream returns `502` and the browser
retries, which is cheaper than a probe that cannot succeed by construction.

### Storage

The backend uses a SQLite database (`portfolio.db`). A persistent volume and claim are included to retain data across pod restarts. The volume is mounted at `/app/data/`.

For production, use block-backed storage (e.g. `gp2`, `ssd`, `azure-disk`) or a managed database.

> **The manifests in `k8s/` are stale and do not describe the running deployment.** The
> StorageClass is *named* `nfs-client`, but its provisioner is
> `k8s.io/minikube-hostpath` — the name is a copied convention, not a description, and
> nothing here has ever been NFS. The live app runs in the `apps` namespace against a
> `local-path-retain` PVC on the node filesystem, with `replicas: 1` and `Recreate`.
> Reconciling or deleting these manifests is tracked in #61. Do not infer the storage
> backend from a StorageClass name; check `spec.provisioner` and the PV's volume source.

### Environment Variables

Backend environment variables are configured in the ConfigMap:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `sqlite:///app/data/portfolio.db?mode=rwc` | SQLite connection string |

### Scaling

The backend is a single replica against one SQLite file, and SQLite is not a
client/server database — horizontal scaling means changing the storage engine, not
adding replicas.

> **Do not follow the previous advice in this section**, which suggested "SQLite with WAL
> mode and NFS". SQLite's own WAL documentation rules that out: *"All processes using a
> database must be the same host computer; WAL does not work over a network filesystem."*
> WAL coordinates through a shared-memory `-shm` file, and on NFS that produces
> `SQLITE_CORRUPT` rather than contention. Scaling SQLite across hosts is not a
> supported configuration by any route.

The supported path is migration to PostgreSQL (or another managed RDBMS). The manifests
are written to be adaptable for a stateless backend against a managed database.

```bash
# Not a supported configuration — see above. Kept here only to record that it was tried
# and why it is wrong.
# kubectl scale deployment backend -n investment-portfolio-manager --replicas=3
```

### Scaling

The default deployment runs a single pod. For higher availability, increase replicas:

```bash
kubectl scale deployment app -n investment-portfolio-manager --replicas=3
```

Note: scaling beyond 1 replica with SQLite requires migration to a shared database (e.g., PostgreSQL). The nginx proxy pass in the frontend container makes horizontal scaling straightforward once the storage layer is externalized.

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
