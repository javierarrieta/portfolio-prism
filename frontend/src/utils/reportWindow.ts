/**
 * The window a dated tax report is asked about, and the query string that describes it.
 *
 * A module of its own rather than part of `components/ReportControls.tsx`, which exports
 * components: `react-refresh/only-export-components` is an error in this repo's lint config
 * and names this remedy itself. `src/utils/` is where this app already keeps shared pure
 * helpers (`formatters.ts`, `decimal.ts`, `csvParser.ts`).
 */

export interface ReportWindow { from: string; to: string }

/** Local-date ISO, not `toISOString()`: the latter shifts the date across midnight UTC. */
const isoDate = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;

/**
 * The default window: the current year to date. Not January 1 to December 31 -- a December
 * 31 `to` is a future date for most of the year and the route answers 422, so the default
 * would be an error rather than a default.
 */
export function currentYearToDate(today: Date = new Date()): ReportWindow {
  return { from: `${today.getFullYear()}-01-01`, to: isoDate(today) };
}

export interface ReportPreset {
  label: string;
  window: ReportWindow;
  /** False for a quarter that has not begun: there is nothing in it to report. */
  available: boolean;
}

/**
 * Four quarters plus the current year to date.
 *
 * Every window ends on or before `today`. A `to` in the future is a hard 422
 * (`backend_rust/src/api_routes/analytics.rs:669`), so a quarter still in progress is
 * clamped to today rather than filled through its own end date -- a button whose only
 * possible outcome is an error is worse than a button that answers the question the user is
 * actually asking. The clamp is not silent: the date inputs render the same state the click
 * sets, so `To` beside the button shows the clamped date. A quarter that has not begun is
 * `available: false`; it keeps its nominal range and is disabled, with a `title` saying so.
 */
export function quarterPresets(today: Date): ReportPreset[] {
  const year = today.getFullYear();
  const todayIso = isoDate(today);
  const quarters: ReportWindow[] = [
    { from: `${year}-01-01`, to: `${year}-03-31` },
    { from: `${year}-04-01`, to: `${year}-06-30` },
    { from: `${year}-07-01`, to: `${year}-09-30` },
    { from: `${year}-10-01`, to: `${year}-12-31` },
  ];
  return [
    ...quarters.map((window, i) => {
      const available = window.from <= todayIso;
      return {
        label: `Q${i + 1}`,
        window: available && window.to > todayIso ? { from: window.from, to: todayIso } : window,
        available,
      };
    }),
    { label: 'This year to date', window: currentYearToDate(today), available: true },
  ];
}

/**
 * The query the download link and the probe share, so the file cannot describe a different
 * window from the one on screen. An empty input omits its parameter rather than inventing a
 * date: `from` absent means inception and `to` absent means today, and inventing either
 * silently changes the number.
 *
 * The path is the server's, without the `/api` prefix: the caller adds `API_BASE`, which is
 * the one place the prefix is written down (`utils/api.ts`).
 */
export function buildReportQuery(
  portfolioId: number, window: ReportWindow, strategy: string, thresholdDays: number,
): string {
  const qs = new URLSearchParams();
  if (window.from) qs.set('from', window.from);
  if (window.to) qs.set('to', window.to);
  qs.set('strategy', strategy);
  qs.set('threshold_days', String(thresholdDays));
  return `/portfolios/${portfolioId}/tax-summary?${qs.toString()}`;
}
