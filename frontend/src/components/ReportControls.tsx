import { formatCurrency } from '../utils/formatters';
import { quarterPresets, isoDate } from '../utils/reportWindow';
import type { ReportWindow } from '../utils/reportWindow';
import type { ReportErrorBody, ReportRow, TaxReport } from '../types';

interface ReportControlsProps {
  window: ReportWindow;
  onWindowChange: (window: ReportWindow) => void;
  /** The `format=csv` URL for the current selection; empty while there is no window to export. */
  csvHref: string;
  onRetry: () => void;
  /**
   * The loaded payload. Accepted because the tab owns one report and hands the same payload
   * to both exports of this module, but nothing here reads a figure from it: every figure it
   * carries is rendered by `TaxReportTable`, and the window these controls show is the
   * selection, not the payload's echo of it.
   */
  report: TaxReport | null;
  error: ReportErrorBody | null;
  loading: boolean;
  /** Injected so the preset rule is testable against a named date, not the suite's clock. */
  today?: Date;
}

/**
 * The report tab's window controls, the download link and the error surface.
 *
 * The lists in a 422 are capped server-side (20 symbols, 10 rate pairs) with no `truncated`
 * flag; truncation is `total > listed`, and the count of what was omitted is printed on the
 * same line as the names. A capped list shown without its count reads as the whole list,
 * which is a lie to the person reconciling the window.
 */
export default function ReportControls(props: ReportControlsProps) {
  const { window, onWindowChange, csvHref, onRetry, error, loading, today = new Date() } = props;
  const omittedPrices = error ? error.missing_prices_total - error.missing_prices.length : 0;
  const omittedRates = error ? error.missing_rates_total - error.missing_rates.length : 0;
  const nothingMissing = error !== null
    && error.missing_prices.length === 0 && error.missing_rates.length === 0;

  return (
    <section className="glass-card" style={{ padding: '20px', marginBottom: '20px' }}>
      <div style={{ display: 'flex', gap: '16px', flexWrap: 'wrap', alignItems: 'flex-end' }}>
        <div className="form-group" style={{ marginBottom: 0 }}>
          <label htmlFor="report-from">From</label>
          <input
            id="report-from"
            type="date"
            className="form-control"
            value={window.from}
            max={isoDate(today)}
            onChange={(e) => onWindowChange({ ...window, from: e.target.value })}
          />
        </div>
        <div className="form-group" style={{ marginBottom: 0 }}>
          <label htmlFor="report-to">To</label>
          <input
            id="report-to"
            type="date"
            className="form-control"
            value={window.to}
            max={isoDate(today)}
            onChange={(e) => onWindowChange({ ...window, to: e.target.value })}
          />
        </div>
        {quarterPresets(today).map(({ label, window: presetWindow, available, clamped }) => (
          <button
            key={label}
            type="button"
            className="btn btn-secondary"
            disabled={!available}
            title={available ? (clamped ? `${label} runs to ${presetWindow.to}, today: the rest of the quarter has not happened yet` : undefined) : `${label} has not started yet`}
            onClick={() => onWindowChange(presetWindow)}
          >
            {label}
          </button>
        ))}
        {/* An anchor, not a fetch: the bytes the route produced reach the disk unchanged, and
            the session cookie rides along because `/api` is same-origin. Rendered only with a
            window, because `?format=csv` without one is a 422 naming `format`. */}
        {csvHref !== '' && (
          <a data-testid="download-csv" className="btn btn-primary" href={csvHref}>
            Download CSV
          </a>
        )}
      </div>

      {error && (
        <div data-testid="report-error" role="alert" className="glass-card" style={{ marginTop: '16px' }}>
          {error.missing_prices.length > 0 && (
            <p>
              {`No price for ${error.missing_prices.join(', ')}`}
              {omittedPrices > 0 ? ` and ${omittedPrices} more` : ''}
            </p>
          )}
          {error.missing_rates.length > 0 && (
            <p>
              {`No rate for ${error.missing_rates.map((r) => `${r.from}→${r.to}`).join(', ')}`}
              {omittedRates > 0 ? ` and ${omittedRates} more` : ''}
            </p>
          )}
          {nothingMissing && <p>{error.error}</p>}
          {/* Two different 422s land here and only one of them is the window's fault.
              When the server named missing symbols or pairs, changing the dates or the
              strategy will not help -- the price or rate simply is not there. */}
          {error.retryable ? (
            <button className="btn btn-secondary" onClick={onRetry}>Try again</button>
          ) : nothingMissing ? (
              <p>
                The dates or the strategy have to change before this can be answered. The strategy
                and its threshold are set on the Holdings &amp; Tax lots tab, not here.
              </p>
            ) : (
              <p>
                There is no price or rate for what is listed above on or before the window&apos;s
                end. Changing the dates or the strategy will not produce it.
              </p>
            )}
        </div>
      )}

      {loading && <p style={{ color: 'var(--text-secondary)', marginTop: '16px' }}>Loading the report…</p>}
    </section>
  );
}

/** The per-row cells each `row_type` owns. Never a sum: every figure comes from the payload. */
function cells(row: ReportRow) {
  switch (row.row_type) {
    case 'SOLD_LOT':
      // The per-lot figures. `realized_pnl` is that sell's whole P&L and repeats on every lot
      // row of the sell, so it is deliberately not the number in this cell.
      return { date: row.sell_date, qty: row.lot_matched_qty, value: row.lot_proceeds, gain: row.lot_pnl };
    case 'OPEN_LOT':
      return { date: row.buy_date, qty: row.remaining_qty, value: row.market_value, gain: row.unrealized_gain_loss };
    case 'UNMATCHED':
      // No money exists for this row, and an em dash is what the null means.
      return { date: row.sell_date, qty: row.unmatched_qty, value: null, gain: null };
  }
}

function TotalFigure(
  { testId, label, value, currency }: { testId: string; label: string; value: string; currency: string },
) {
  return (
    <div>
      <div style={{ fontSize: '0.75rem', color: 'var(--text-muted)', fontWeight: 600 }}>{label}</div>
      <strong data-testid={testId}>{formatCurrency(value, currency)}</strong>
    </div>
  );
}

/**
 * The report itself: the run context, the payload's five totals, and one row per entry in
 * `report.rows`.
 *
 * Nothing here adds, averages or subtracts. The payload is the only source of every figure
 * on the screen, including the ones a careless renderer would recompute from the rows --
 * `row.lot_pnl` over the SOLD_LOT rows of one asset sums to that asset's
 * `realized_in_window`, but the server has already done that addition and printed it in
 * `report.totals`, so summing here would produce a second, subtly different number.
 */
export function TaxReportTable({ report }: { report: TaxReport }) {
  const { context, totals, rows } = report;
  const currency = context.base_currency;
  // `requested_from` is null when the portfolio holds no transaction on or before `to`
  // (`api_routes/analytics.rs:839`, `schemas.rs:318-323`); the route's own CSV prints
  // "inception" for that case (`analytics.rs:102`), so the screen says the same word.
  const appliedFrom = context.requested_from ?? 'inception';

  let runContext = `${context.portfolio_name} · ${currency} · ${context.strategy}`;
  if (context.strategy === 'HYBRID') runContext += ` (threshold ${context.threshold_days}d)`;
  runContext += ` · ${appliedFrom} → ${context.requested_to}`;
  runContext += ` · ${context.splits_note}`;

  return (
    <>
      <p
        data-testid="report-context"
        style={{ color: 'var(--text-secondary)', fontSize: '0.875rem', marginBottom: '16px' }}
      >
        {runContext}
      </p>

      <div
        className="glass-card"
        style={{ padding: '20px', marginBottom: '20px', display: 'flex', gap: '32px', flexWrap: 'wrap' }}
      >
        <TotalFigure testId="total-realized-in-window" label="Realized in window" value={totals.realized_in_window} currency={currency} />
        <TotalFigure testId="total-realized-cumulative" label="Realized since inception" value={totals.realized_cumulative} currency={currency} />
        <TotalFigure testId="total-unrealized-at-end" label="Unrealized at end" value={totals.unrealized_at_end} currency={currency} />
        <TotalFigure testId="total-market-value" label="Market value at end" value={totals.market_value} currency={currency} />
        <TotalFigure testId="total-cost-basis" label="Cost basis at end" value={totals.cost_basis} currency={currency} />
      </div>

      <div className="data-table-container">
        <table className="data-table">
          <thead>
            <tr>
              <th>Symbol</th><th>Kind</th><th>Date</th><th>Quantity</th>
              <th>Value</th><th>Gain/Loss</th><th>Price as of</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row, i) => {
              const c = cells(row);
              return (
                <tr key={`${row.row_type}-${row.symbol}-${i}`}>
                  <td>{row.symbol}</td>
                  <td>{row.row_type}</td>
                  {/* The payload's own token, not a re-derived local date: the CSV writes the
                      same string, so the screen and the file agree on the day. */}
                  <td>{c.date ?? '—'}</td>
                  <td>{c.qty ?? '—'}</td>
                  {/* A null money cell is an em dash, never a formatted zero. */}
                  {/* `== null`, not `=== null`: a key that is absent entirely must read as unknown, not as a formatted zero. */}
                  <td>{c.value == null ? '—' : formatCurrency(c.value, currency)}</td>
                  <td>{c.gain == null ? '—' : formatCurrency(c.gain, currency)}</td>
                  <td data-testid={`row-${i}-price-date`}>{row.price_date_used ?? '—'}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}
