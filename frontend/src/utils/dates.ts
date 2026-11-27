/**
 * Every date the app displays, as ISO 8601 (`YYYY-MM-DD`), built from UTC parts.
 *
 * Two reasons, and the second is the one that actually bites.
 *
 * **Locale-dependent output is not a display preference.** `toLocaleDateString()` with no
 * locale follows the browser, so the same printed statement read `11/09/2026` in one country
 * and `09/11/2026` in another — and a tax statement is a document someone files and someone
 * else audits. Passing `'en-US'` "fixes" that only by imposing one reader's convention on
 * everyone else, which is what the statement did until this module existed.
 *
 * **A date-only value round-tripped through the local zone shifts by a day.** The backend
 * sends transaction dates as `2026-10-01T00:00:00Z` — a `DATE` column serialised by chrono —
 * and `new Date(...).toLocaleDateString()` renders that in the *browser's* zone. Every user
 * west of UTC saw their own transaction on the day before it happened. The suite has pinned
 * `TZ=UTC` (`src/test/setup.ts`) since before this module existed, which is precisely why no
 * test caught it: the harness runs in the single zone where the bug is invisible.
 *
 * UTC is also the boundary the server owns — the report window's `to` is
 * `Utc::now().date_naive()` server-side — so displaying local dates beside a UTC-bounded query
 * would let the screen show a day the server refuses to accept.
 */

/**
 * The date the SERVER will call today, as `YYYY-MM-DD`.
 *
 * *** UTC parts, not local. *** The route's future-date check is `Utc::now().date_naive()`
 * (`backend_rust/src/api_routes/analytics.rs:71`), so a `to` after the UTC date is a hard 422
 * (`:669`). Built from local parts this made the tab fail on open for every user east of UTC:
 * at 01:30 in UTC+2 the local date is already tomorrow while the server still says today, and
 * the default window is built from this same value, so no preset could rescue it. UTC parts are
 * deliberate even though that can read as "yesterday" to a user after their local midnight —
 * the server owns the boundary, and the alternative is a request that is refused.
 */
export const isoDate = (d: Date): string =>
  `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, '0')}-${String(d.getUTCDate()).padStart(2, '0')}`;

/** What an unparseable or absent value prints as. A blank cell reads as missing data. */
const EMPTY = '—';

function asDate(value: string | Date | null | undefined): Date | null {
  if (value === null || value === undefined || value === '') return null;
  const d = value instanceof Date ? value : new Date(value);
  return Number.isNaN(d.getTime()) ? null : d;
}

/**
 * `YYYY-MM-DD`. Accepts what the API actually sends — a bare `2026-10-01`, a
 * `2026-10-01T00:00:00Z`, or an already-parsed `Date` — and returns the same three answers for
 * all three. That is the point: the caller should not have to know which shape a given route
 * uses, and `new Date(x).toLocaleDateString()` returns a different day depending on where the
 * browser is.
 */
export function formatDate(value: string | Date | null | undefined): string {
  const d = asDate(value);
  return d ? isoDate(d) : EMPTY;
}

/**
 * `YYYY-MM-DD HH:MM`, UTC, no zone suffix.
 *
 * No `T` and no `Z` because this is for a table cell, not a wire format, and the suffix is
 * what made the cell unreadable at a glance. The zone is stated once, in the header of the
 * panels that print it, rather than repeated on every row.
 */
export function formatDateTime(value: string | Date | null | undefined): string {
  const d = asDate(value);
  if (!d) return EMPTY;
  const hh = String(d.getUTCHours()).padStart(2, '0');
  const mm = String(d.getUTCMinutes()).padStart(2, '0');
  return `${isoDate(d)} ${hh}:${mm}`;
}
