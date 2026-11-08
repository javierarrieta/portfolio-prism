# Spec review records — OIDC design (2026-11-04)

Adversarial review output for the **design document**, produced before a line of the
implementation existed. The spec under review is
[`docs/superpowers/specs/2026-11-04-oidc-identity-isolation-design.md`](../../specs/2026-11-04-oidc-identity-isolation-design.md).

**These review a plan, not the shipped code.** Several findings were resolved during
implementation in ways these reports could not foresee, and some were rejected outright.
For what was actually built and what was decided after the code existed, read
[`2026-11-07-oidc-implementation.md`](../2026-11-07-oidc-implementation.md) and
[`2026-11-07-oidc-implementation-review-r1.md`](../2026-11-07-oidc-implementation-review-r1.md).
Treat anything here as provenance — why the design is shaped the way it is — rather
than as a current statement of the system.

## Why they are kept

They record the reasoning behind constraints that would otherwise look arbitrary in the
code. Examples that survived into the implementation:

- the fairing cannot short-circuit a request, which is why denial is a URI rewrite and
  why a static gate is needed at all;
- the operation count the spec claimed did not match the router, which is why route
  coverage is derived from the router rather than asserted against a hand-written list;
- the rejected alternatives that shaped the session design — no local `users` table,
  `oauth2-proxy` with claim injection, bearer tokens in the browser — each rejected for
  a reason worth having in writing.

## Files

| File | What it is |
|---|---|
| `consolidated_review_report.md` | The merged finding list across all reviewers — the one to read first |
| `implementation_readiness_report.md` | Largest single report; whether the spec was buildable as written |
| `critic_report.md` | Design critic: rejected alternatives, unsupported claims, overstatement |
| `comprehension_report.md` | Whether the document could be read correctly at all |
| `zoom_out_report.md`, `intermediate_zoom_out_report.md` | Scope challenges — what should have been cut |
| `stage7_design.md`, `stage7_structural.md`, `stage7_style.md` | Per-axis review of the document |
| `stage9_codebase.md`, `stage9_readiness.md`, `stage9_style.md` | Per-axis review after the spec's own revision |

`stage9_*` supersede `stage7_*` where they overlap; both are kept because the diff
between them is the record of what the first revision changed.

## Provenance

Generated 2026-11-05 by the EGM review workflow, recovered from the working tree on
2026-11-08 after the branch that produced them was merged and its worktree removed.
They were never committed on the branch that carried the spec.
