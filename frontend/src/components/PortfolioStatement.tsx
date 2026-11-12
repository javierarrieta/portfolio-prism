import { formatCurrency, formatPercent } from '../utils/formatters';
import type { Portfolio, TaxSummary } from '../types';

interface PortfolioStatementProps {
  taxSummary: TaxSummary;
  portfolio: Portfolio;
  fetchedAt: Date;
}

const DATE_OPTS: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric', year: 'numeric' };
const DATETIME_OPTS: Intl.DateTimeFormatOptions = { ...DATE_OPTS, hour: '2-digit', minute: '2-digit' };
const QTY_OPTS: Intl.NumberFormatOptions = { maximumFractionDigits: 6 };

/* The Type column prints a display label, not the stored enum: MUTUAL_FUND is an internal
 * identifier rather than financial data, and at 11 unbreakable characters (`_` is not a
 * line-break opportunity) it was the widest token the table printed, so it alone decided whether
 * the row fit. The `??` fallback is load-bearing: AssetTaxSummary.asset_type is a plain string
 * (types.ts:53), so an AssetType this map has never seen still prints instead of blanking the
 * cell. PortfolioDetail.tsx still prints the raw enum on screen: different artifact, different
 * width budget, deliberately left alone. */
const ASSET_TYPE_LABEL: Record<string, string> = {
  STOCK: 'Stock', CRYPTO: 'Crypto', ETF: 'ETF', MUTUAL_FUND: 'Fund',
};

export default function PortfolioStatement(
  { taxSummary, portfolio, fetchedAt }: PortfolioStatementProps,
) {
  const currency = taxSummary.currency;
  const printDate = new Date().toLocaleDateString('en-US', DATE_OPTS);
  const empty = taxSummary.assets.length === 0;

  // The empty-portfolio response carries no strategy, so run-context is absent rather
  // than rendered with a dangling separator.
  let runContext: string | null = null;
  if (!empty && taxSummary.strategy) {
    runContext = `${portfolio.name} · ${portfolio.currency} · ${taxSummary.strategy}`;
    if (taxSummary.strategy === 'HYBRID' && taxSummary.threshold_days !== undefined) {
      runContext += ` (threshold ${taxSummary.threshold_days}d)`;
    }
  }

  return (
    <section data-print="statement" className="statement">
      <header className="statement-header">
        <h1>Portfolio Statement</h1>
        {runContext && <p data-testid="run-context">{runContext}</p>}
        <p data-testid="generated-at">Printed {printDate}</p>
        {!empty && (
          <p data-testid="data-at">Data fetched {fetchedAt.toLocaleString('en-US', DATETIME_OPTS)}</p>
        )}
      </header>

      {empty ? (
        <p data-testid="empty-note">No holdings in this portfolio on {printDate}.</p>
      ) : (
        <>
          <div className="statement-headline">
            <div className="statement-figure">
              <span>Portfolio value</span>
              <strong data-testid="figure-value">
                {formatCurrency(taxSummary.total_portfolio_value, currency)}
              </strong>
            </div>
            <div className="statement-figure">
              <span data-testid="label-realized">Realized P&L — cumulative since inception</span>
              <strong data-testid="figure-realized">
                {formatCurrency(taxSummary.total_realized_pnl, currency)}
              </strong>
            </div>
            <div className="statement-figure">
              <span>Unrealized P&L — as of today</span>
              <strong data-testid="figure-unrealized">
                {formatCurrency(taxSummary.total_unrealized_pnl, currency)}
              </strong>
            </div>
          </div>

          <table className="data-table statement-table" data-testid="holdings">
            <thead>
              <tr>
                <th>Symbol</th><th>Type</th><th>Quantity</th><th>Avg cost</th><th>Price</th>
                <th>Market value</th><th>Realized</th><th>Unrealized</th><th>Unrealized ROI</th>
              </tr>
            </thead>
            <tbody>
              {taxSummary.assets.map((a) => (
                <tr key={`${a.symbol}-${a.asset_type}`}>
                  <td>{a.symbol}</td>
                  <td>{ASSET_TYPE_LABEL[a.asset_type] ?? a.asset_type}</td>
                  <td>{a.current_shares.toLocaleString(undefined, QTY_OPTS)}</td>
                  <td>{formatCurrency(a.average_cost, currency)}</td>
                  <td>{formatCurrency(a.current_price, currency)}</td>
                  <td>{formatCurrency(a.market_value, currency)}</td>
                  <td>{formatCurrency(a.realized_pnl, currency)}</td>
                  <td>{formatCurrency(a.unrealized_pnl, currency)}</td>
                  <td>{formatPercent(a.unrealized_roi)}</td>
                </tr>
              ))}
            </tbody>
          </table>

          <footer className="statement-notes">
            <p data-testid="rounding-note">
              Figures are rounded for display from full-precision calculations.
            </p>
            <p data-testid="splits-note">
              Split-adjusted history is as recorded when this page was loaded; late-recorded
              splits can change past figures.
            </p>
          </footer>
        </>
      )}
    </section>
  );
}
