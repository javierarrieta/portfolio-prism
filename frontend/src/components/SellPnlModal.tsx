import { useEffect, useState } from 'react';
import { AlertTriangle, Info, Loader2, X } from 'lucide-react';
import { formatCurrency, formatPercent } from '../utils/formatters';
import { normalizeSellPnl } from '../utils/decimal';
import type { SellPnlBreakdown } from '../types';

interface Props {
  transactionId: number;
  strategy: string;
  thresholdDays: number;
  onClose: () => void;
}

const pnlColor = (v: number) => (v >= 0 ? 'var(--color-success)' : 'var(--color-danger)');
const qty = (v: number) => v.toLocaleString(undefined, { maximumFractionDigits: 6 });

export default function SellPnlModal({ transactionId, strategy, thresholdDays, onClose }: Props) {
  const [data, setData] = useState<SellPnlBreakdown | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const [retryKey, setRetryKey] = useState(0);

  // Keyed on strategy and threshold so the modal refetches when the Holdings tab
  // changes the matching method -- that is the point, not an accident to prune.
  // No setState in the effect body itself: react-hooks/set-state-in-effect rejects a
  // synchronous reset there, and the retry handler below does that reset instead.
  useEffect(() => {
    let cancelled = false;
    fetch(`/api/transactions/${transactionId}/pnl?strategy=${strategy}&threshold_days=${thresholdDays}`)
      .then((r) => {
        if (!r.ok) throw new Error(`sell pnl request failed: ${r.status}`);
        return r.json();
      })
      .then((json) => {
        if (cancelled) return;
        setData(normalizeSellPnl(json));
        setLoading(false);
      })
      .catch(() => {
        if (cancelled) return;
        setError(true);
        setLoading(false);
      });
    return () => { cancelled = true; };
  }, [transactionId, strategy, thresholdDays, retryKey]);

  const retry = () => {
    setError(false);
    setLoading(true);
    setRetryKey((k) => k + 1);
  };

  const isHybrid = data?.strategy === 'HYBRID';
  const muted = { color: 'var(--text-muted)' };

  return (
    <div
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
      style={{ position: 'fixed', top: 0, left: 0, right: 0, bottom: 0, background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(4px)', display: 'flex', justifyContent: 'center', alignItems: 'center', zIndex: 100 }}
    >
      <div className="glass-card" style={{ padding: '32px', width: '760px', maxWidth: '92vw', maxHeight: '86vh', overflowY: 'auto', background: '#121929' }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: '16px' }}>
          {data ? (
            <div>
              <h3 style={{ marginBottom: '4px' }}>Sell P&L — {data.symbol}</h3>
              <div style={{ ...muted, fontSize: '13px' }}>
                {new Date(data.sell_date).toLocaleString()} · {qty(data.sold_qty)} sold @ {formatCurrency(data.price, data.asset_currency)}
                {data.fee !== 0 && ` · fee ${formatCurrency(data.fee, data.asset_currency)}`}
                {` · proceeds ${formatCurrency(data.proceeds, data.currency)}`}
              </div>
              {data.asset_currency !== data.currency && (
                <div data-testid="as-entered-currency" style={{ ...muted, fontSize: '12px' }}>
                  price and fee as entered in {data.asset_currency}
                </div>
              )}
            </div>
          ) : (
            <h3 style={{ marginBottom: '4px' }}>Sell P&amp;L</h3>
          )}
          <button aria-label="Close" onClick={onClose} style={{ background: 'transparent', border: 'none', cursor: 'pointer', color: 'var(--text-muted)' }}>
            <X size={18} />
          </button>
        </div>

        {loading && (
          <div style={{ display: 'flex', alignItems: 'center', gap: '10px', color: 'var(--text-secondary)', padding: '24px 0' }}>
            <Loader2 size={18} className="animate-spin" /> Loading realized P&amp;L...
          </div>
        )}

        {error && !loading && (
          <div data-testid="modal-error" style={{ padding: '16px', border: '1px solid var(--color-danger)', borderRadius: '8px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px', color: 'var(--color-danger)', marginBottom: '12px' }}>
              <AlertTriangle size={16} /> Could not load the realized P&amp;L breakdown.
            </div>
            <button className="btn btn-secondary" onClick={retry}>Retry</button>
          </div>
        )}

        {data && !loading && !error && (
          <>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: '16px', marginBottom: '16px', flexWrap: 'wrap' }}>
              <div style={{ fontSize: '24px', fontWeight: 700, color: pnlColor(data.realized_pnl) }}>
                {formatCurrency(data.realized_pnl, data.currency)}
              </div>
              <span data-testid="roi" style={{ color: 'var(--text-secondary)' }}>
                {data.roi === null ? '—' : formatPercent(data.roi)}
              </span>
              <span data-testid="strategy-note" style={{ ...muted, fontSize: '13px' }}>
                {`realized · ${data.strategy}`}{isHybrid ? ` (threshold ${data.threshold_days}d)` : ''}
              </span>
            </div>

            {data.unmatched_qty > 0 && (
              <div data-testid="unmatched-banner" style={{ display: 'flex', gap: '8px', padding: '12px 16px', marginBottom: '16px', border: '1px solid var(--color-warning)', borderRadius: '8px', color: 'var(--color-warning)', fontSize: '13px' }}>
                <AlertTriangle size={16} style={{ flexShrink: 0 }} />
                <span>
                  {qty(data.unmatched_qty)} of {qty(data.sold_qty)} units had no buy lot on or before this date. Neither their cost basis nor their proceeds are included in this figure.
                </span>
              </div>
            )}

            {data.matches.length === 0 && (
              <div data-testid="no-lots-note" style={{ ...muted, padding: '12px 16px', marginBottom: '16px', border: '1px solid var(--border-color)', borderRadius: '8px', fontSize: '13px' }}>
                No buy lots were available to match on or before this date. Realized P&amp;L is {formatCurrency(data.realized_pnl, data.currency)} because nothing was matched.
              </div>
            )}

            {data.matches.length > 0 && (
              <table className="data-table" style={{ marginBottom: '16px' }}>
                <thead>
                  <tr>
                    <th>Buy date</th>
                    <th>Buy price</th>
                    <th>Qty from lot</th>
                    <th>Cost basis</th>
                    <th>Proceeds</th>
                    <th>P&amp;L</th>
                    <th>Held</th>
                    {isHybrid && <th>Term</th>}
                  </tr>
                </thead>
                <tbody>
                  {/* Rendered in payload order: that is the engine's consumption order. */}
                  {data.matches.map((m) => (
                    <tr key={`${m.buy_transaction_id}-${m.buy_date}`}>
                      <td>{new Date(m.buy_date).toLocaleDateString('en-CA')}</td>
                      <td>{formatCurrency(m.buy_price, data.currency)}</td>
                      <td>{qty(m.matched_qty)}<span style={muted}> of {qty(m.lot_qty_before_match)}</span></td>
                      <td>{formatCurrency(m.cost_basis, data.currency)}</td>
                      <td>{formatCurrency(m.proceeds, data.currency)}</td>
                      <td style={{ color: pnlColor(m.pnl) }}>{formatCurrency(m.pnl, data.currency)}</td>
                      <td>{m.holding_days}d</td>
                      {isHybrid && <td>{m.term ?? '—'}</td>}
                    </tr>
                  ))}
                </tbody>
              </table>
            )}

            <div style={{ ...muted, fontSize: '12px', display: 'flex', flexDirection: 'column', gap: '6px' }}>
              <span style={{ display: 'flex', gap: '6px' }}>
                <Info size={13} style={{ flexShrink: 0, marginTop: '2px' }} />
                Attribution follows the {data.strategy} method. A different method would consume different lots.
              </span>
              {data.split_applied && (
                <span data-testid="split-note" style={{ display: 'flex', gap: '6px' }}>
                  <Info size={13} style={{ flexShrink: 0, marginTop: '2px' }} />
                  Buy prices and unit costs are split-adjusted, not the prices as entered.
                </span>
              )}
              {data.fx_applied && (
                <span data-testid="fx-note" style={{ display: 'flex', gap: '6px' }}>
                  <Info size={13} style={{ flexShrink: 0, marginTop: '2px' }} />
                  Cost basis and proceeds were converted to {data.currency} at the latest available {data.asset_currency}→{data.currency} rate.
                </span>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
