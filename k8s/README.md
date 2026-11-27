# `k8s/` — reference only. Nothing here is applied by anything running.

Written 2026-11-27, after the `v0.1.0-M4` release notes sent someone here to bump an image and
the file did not match anything. Every claim below was measured on both clusters that day, not
inferred from this directory.

## What actually runs

| | what it runs | where its manifests live |
| --- | --- | --- |
| **titan** — the live app | `apps/portfolio-prism`; `frontend:v0.1.0-M4` and `backend:v0.1.0-M4`; `spec.strategy.type: Recreate`; `replicas: 1`; backend config from the `portfolio-prism-secrets` Secret **only** | `k8s-titan` repo: `apply/50-apps/portfolio-prism/portfolio-prism.yaml` (PVC, Deployment, Service, Ingress) and `portfolio-prism-db.yaml` (CNPG `Database` + `DatabaseRole`) |
| **casa** — slated for retirement | `casa/investment-portfolio-manager`; both images `v0.0.6-M3`; config from `investment-portfolio-manager-config` + `investment-portfolio-secrets` | not this directory either — those two objects are not defined in this repo |

## Why this directory cannot be applied

`app-deployment.yaml` deploys `app` in namespace `investment-portfolio-manager`, pulling
`ghcr.io/<org>/investment-portfolio-manager/*`, with `envFrom` on `backend-config` and
`backend-secrets`. **None of those names exist on either cluster.** Measured 2026-11-27:

```bash
kubectl --context titan get ns investment-portfolio-manager   # NotFound
kubectl --context casa  get ns investment-portfolio-manager   # NotFound
kubectl --context titan get cm -A | grep backend-config       # nothing
kubectl --context casa  get cm -A | grep backend-config       # nothing
kubectl --context titan get secret -A | grep backend-secrets  # nothing
kubectl --context casa  get secret -A | grep backend-secrets  # nothing
```

So applying this directory would not update a running thing. It would create a *second*
deployment whose pods cannot start — a missing `envFrom` reference is a
`CreateContainerConfigError`, not a warning — in a namespace nothing else uses, next to the
deployment that is actually serving traffic.

## Why it is still here

The comments in these files are the valuable part, and they are not about image tags. Why there
is no `wait-for-backend` initContainer (it curled `127.0.0.1:8000` in an init phase, which runs
before the backend container exists, so it could only ever exhaust retries and delay every pod
start by 60 s). Why the backend needs a `startupProbe` (migrations run before Rocket binds the
port, and the liveness `initialDelaySeconds` would kill it mid-migration). Why the backend image
is pinned by digest rather than `:latest` (`kubectl rollout undo` on a `:latest` tag is a no-op
— the pod template is byte-identical, so the rollback you think you have does not exist).

Those reasons are true of any deployment of this app, including the live one. Keep them. What
must not happen again is reading the `metadata`, `image` and `envFrom` lines as the state of the
world, which is what the release notes did.

## If you came here to deploy

Do not edit these files to roll a release. Edit
`k8s-titan/apply/50-apps/portfolio-prism/portfolio-prism.yaml`, open a PR there, and let Flux
apply it. The ordering hazard for a database-touching release is real and is described in
`docs/postgres-cutover.md` — but it is a hazard on the live manifest, not on this one.
