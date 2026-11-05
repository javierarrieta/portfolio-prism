import { Trash2, TrendingUp } from 'lucide-react';
import type { Asset, Transaction } from '../types';
import { formatCurrency } from '../utils/formatters';

interface Props {
  transactions: Transaction[];
  assets: Asset[];
  onDeleteTransaction: (txId: number) => Promise<void>;
  onSelectSell: (txId: number) => void;
}

export default function TransactionLedger({ transactions, assets, onDeleteTransaction, onSelectSell }: Props) {
  return (
    <div className="glass-card" style={{ padding: '24px' }}>
      <h3 style={{ marginBottom: '20px' }}>Transaction History</h3>
      {transactions.length === 0 ? (
        <p style={{ color: 'var(--text-secondary)', textAlign: 'center', padding: '24px 0' }}>No transactions logged. Go to the "Holdings & Tax lots" tab to log buy/sell transactions.</p>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>Date & Time</th>
              <th>Asset Symbol</th>
              <th>Type</th>
              <th>Quantity</th>
              <th>Price</th>
              <th>Fee</th>
              <th>Total Cost/Proceeds</th>
              <th>Action</th>
            </tr>
          </thead>
          <tbody>
            {transactions.map((tx) => {
              const asset = assets.find(a => a.id === tx.asset_id);
              const isSplit = tx.type === 'SPLIT';
              const isSell = tx.type === 'SELL';
              const cost = tx.quantity * tx.price;
              const total = isSplit ? null : (tx.type === 'BUY' ? (cost + tx.fee) : (cost - tx.fee));
              return (
                <tr
                  key={tx.id}
                  onClick={isSell ? () => onSelectSell(tx.id) : undefined}
                  style={isSell ? { cursor: 'pointer' } : undefined}
                >
                  <td>{new Date(tx.date).toLocaleString()}</td>
                  <td style={{ fontWeight: 600 }}>{asset ? asset.symbol : 'Unknown'}</td>
                  <td>
                    <span style={{ color: isSplit ? 'var(--text-secondary)' : (tx.type === 'BUY' ? 'var(--color-success)' : 'var(--color-danger)'), fontWeight: 700 }}>
                      {tx.type}
                    </span>
                  </td>
                  <td>{isSplit ? `${tx.quantity} : ${tx.price}` : tx.quantity.toLocaleString(undefined, { maximumFractionDigits: 6 })}</td>
                  <td>{isSplit ? '—' : formatCurrency(tx.price, asset?.currency || 'USD')}</td>
                  <td>{isSplit ? '—' : formatCurrency(tx.fee, asset?.currency || 'USD')}</td>
                  <td style={{ fontWeight: 600 }}>{isSplit ? '—' : formatCurrency(total as number, asset?.currency || 'USD')}</td>
                  <td>
                    {isSell && (
                      <button
                        aria-label={`View realized P&L for ${asset ? asset.symbol : 'this holding'}`}
                        title="View realized P&L"
                        onClick={(e) => { e.stopPropagation(); onSelectSell(tx.id); }}
                        style={{ background: 'transparent', border: 'none', cursor: 'pointer', color: 'var(--color-primary)', marginRight: '4px' }}
                      >
                        <TrendingUp size={16} />
                      </button>
                    )}
                    <button
                      title="Delete transaction"
                      onClick={(e) => { e.stopPropagation(); void onDeleteTransaction(tx.id); }}
                      style={{ background: 'transparent', border: 'none', cursor: 'pointer', color: 'var(--text-muted)' }}
                    >
                      <Trash2 size={16} />
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
