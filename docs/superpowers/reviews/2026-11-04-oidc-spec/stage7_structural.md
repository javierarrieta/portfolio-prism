## Findings

**1. Severity: Major**
**Affected Scope:** Whole FeatureRange (lines 1–137) vs. standard's mandatory *Alternatives Considered*
**Rationale:** The standard's Alternatives Rule is called mandatory, and a fresh reader navigating by headings finds no `## Alternatives Considered` anywhere in the document — the heading list goes Overview → Requirements → Non-Goals → Current State → Decisions Summary → Architecture. The *substance* is satisfied (the Decisions Summary table at lines 89–104 carries a per-decision "Rejected alternative" column with specific reasons), so this is a discoverability/naming failure, not a content gap. Fix: add `## Alternatives Considered` (or rename the section `## Decisions Summary / Alternatives Considered`) so the mandated section is greppable by title, and state in one line that it is the complete alternatives register for this spec.

**2. Severity: Major**
**Affected Scope:** Whole FeatureRange; standard's *Risks & Mitigations*
**Rationale:** No section in lines 1–137 is named for risks, and none signposts where risks live. The document discharges the obligation under two non-standard headings far outside the range — `## Security Considerations` (line 550) and `## Known Limitations` (line 588). A reviewer using the FeatureRange as a briefing never learns the risk register exists. Fix: add `## Risks & Mitigations` inside the FeatureRange (3–6 bullets: enumeration, chokepoint discipline, `Disabled` fail-open, bootstrap handover, shared third-party quota), each linking to the corresponding Security Considerations bullet.

**3. Severity: Major**
**Affected Scope:** `## Decisions Summary` (89–104) precedes `## Architecture` (106–136)
**Rationale:** The decisions table forward-references `users.token_version`, `portfolios.owner_id`, `portfolio_members`, the `access` module, and `BOOTSTRAP_OWNER_SUBJ` — none of which are defined until Architecture (line 132) or Backend Changes §4/§5. A first-time reader meets the verdicts before the vocabulary. Fix: move Architecture above Decisions Summary, or move the data-model row of the "Three units" table into a short preamble to Decisions Summary.

**4. Severity: Major**
**Affected Scope:** `## Architecture` diagram, line 118
**Rationale:** Verified width defect in the one figure the section rests on. The box spans 57 columns (top border line 115 = `┌` + 40 `─` + `┴` + 14 `─` + `┐`; bottom line 125 = 55 `─` between corners). Lines 116, 117, 119–124 each carry exactly 55 characters between the `│` borders; line 118 (`│   └─ FromRequest guard → AuthedUser …`) carries 56, so its right border is one column right of every other row — the equivalent of a broken table border. Fix: delete one space before the closing `│` on line 118.

**5. Severity: Major**
**Affected Scope:** `## Architecture` diagram, lines 120–123
**Rationale:** The annotation "the only path to a portfolio by id" is split across three lines (120, 121, 123) that are interleaved with two `▼` flow arrows belonging to the main vertical path (121, 123). Read top-to-bottom the sentence is broken by flow glyphs, and the `▼` column and annotation column visually collide. Fix: move the annotation outside the box as a single right-hand callout, or as a caption line under the figure.

**6. Severity: Major**
**Affected Scope:** `## Overview` line 16 and `## Decisions Summary` line 96 ("ten hand-written SQL statements" / "ten places to forget")
**Rationale:** "Ten" is load-bearing for both the Overview's threat argument and the rejection rationale for per-handler checks, yet the section explicitly titled *Current State (evidence)* (66–87) contains no line establishing it. The only counts the document evidences are 10 paths / 13 operations / 14 mounted API operations — so "ten" reads as a stale number contradicting the surface count the document later defends (§6, lines 375–383). Fix: cite `file:line` for each statement in the evidence table, or restate as "every handler".

**7. Severity: Major**
**Affected Scope:** `## Non-Goals` (52–64)
**Rationale:** Asymmetric entries. Three bullets carry a destination pointer ("— Spec 2", "— Spec 3", "Spec 4"); three do not (roles/admin panels/quotas, `is_active` sync, replacing SQLite / `replicas: 1`). The reader cannot tell "assigned to a later spec" from "permanently unowned", and the Deferred Work table (576–586) lists only Specs 2–4, leaving the latter three with no home. Fix: give every Non-Goal either a Spec pointer or an explicit "no planned spec" marker; cross-link the SQLite item to Known Limitations, where it is duplicated verbatim.

**8. Severity: Nit** — Front matter (3–5) uses loose `**Label**: value` lines while every other metadata structure in the document is a table; no table of contents despite 20 H2 sections and four intra-document anchors. Fix: three-column metadata table plus a short TOC.
**9. Severity: Nit** — `## Current State (evidence)` (66) embeds a parenthetical qualifier, unlike every other heading; heading naming is otherwise inconsistent about ampersands (`Configuration & Deployment`, `OpenAPI, CI & Frontend Impact` vs `Backend Changes`). Fix: `## Evidence: Current State`, and pick one conjunction style.
**10. Severity: Nit** — Requirement 7 (line 44) states the `/api-docs` *prefix* requires auth; §6 (line 373) delivers two enumerated routes (`/api-docs/openapi.json`, `/openapi.json`). Prefix-vs-enum is the same drift class the document elsewhere builds a test against, and `/openapi.json` is not evidenced in the Current State table. Fix: align wording and extend the "every mounted route is in the table" test to `/api-docs`.
**11. Severity: Nit** — Requirement 6 (line 41) says "the members table"; the table is named `portfolio_members` only at line 55. Use the real name on first use.
**12. Severity: Nit** — Requirements (28–50) are a flat 1–12 list mixing functional (1–7), operational (8–10) and forward-compat (11–12) concerns, with no stable IDs despite later references to "requirement 4" and "requirement 10", and no requirement→test traceability table. Fix: group under subheads, use `R1…R12`, add a traceability table.
**13. Severity: Nit** — Fence info strings are inconsistent: the Rust and SQL blocks are tagged, the Architecture fence (108–128) is bare. Also the bottom box border (125) has no `┴` where the Authentik connector (line 126) leaves the box, and line 116 uses an ASCII `|` for OR inside a box drawn with `│`.
**14. Severity: Nit** — Overview (10–12) asserts `utoipa_swagger_ui` is an unused dependency; this is the document's only evidence-free factual claim inside a section that otherwise cites `file:line`. Add it to the evidence table.

**No Blockers found.** Verified clean: heading hierarchy is strictly H1 → H2 with no skipped or out-of-order levels (1, 7, 28, 52, 66, 89, 106); all three FeatureRange tables are well-formed with header/separator/cell counts matching (2, 3, 3 columns); all four FeatureRange anchors resolve (`#deferred-work`, `#security-considerations`, plus `#configuration--deployment` and `#rollout--rollback` used later); all code fences are closed; and all twelve Requirements are genuinely delivered in the DetailedRange (§1–§8, Configuration, Testing) — no orphaned promises.

## Not assessed
- Line-level correctness of the DetailedRange (138–614) beyond the promise-vs-delivery check above.
- Factual accuracy of cited `file:line` references (read-only structural pass; no source files opened).
- Semantic security review of the design (out of Structural role).