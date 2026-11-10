import React, { useState, useEffect, ChangeEvent, FormEvent, useCallback } from 'react';
import { 
  LayoutDashboard, 
  Briefcase, 
  BarChart3, 
  History, 
  Plus, 
  Loader2, 
  AlertTriangle,
  FolderOpen,
  Sparkles,
  LogOut,
  Printer,
} from 'lucide-react';
import Dashboard from './components/Dashboard';
import PortfolioDetail, { CreateTxPayload } from './components/PortfolioDetail';
import AnalyticsView from './components/AnalyticsView';
import TransactionLedger from './components/TransactionLedger';
import PortfolioStatement from './components/PortfolioStatement';
import SellPnlModal from './components/SellPnlModal';
import { normalizePerformance, normalizeTaxSummary, normalizeTransactionList, normalizeAssetList } from './utils/decimal';
import { apiFetch, API_BASE } from './utils/apiFetch';
import { 
  Portfolio, 
  PortfolioPerformance, 
  TaxSummary, 
  Transaction, 
  Asset,
  TransactionType
} from './types';

// dashboard, portfolios, analytics, ledger, statement
const DEFAULT_TAB = 'dashboard';

export default function App() {
  const [portfolios, setPortfolios] = useState<Portfolio[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [activeTab, setActiveTab] = useState(DEFAULT_TAB);
  
  // Tax state
  const [strategy, setStrategy] = useState('FIFO');
  const [thresholdDays, setThresholdDays] = useState(30);

  // Loaded analytics
  const [performance, setPerformance] = useState<PortfolioPerformance | null>(null);
  const [taxSummary, setTaxSummary] = useState<TaxSummary | null>(null);
  // setTaxSummary runs once and is never cleared, so a portfolio switch leaves the
  // previous portfolio's summary in state until the new fetch resolves. A page with B's
  // numbers under A's heading is the one failure this artifact cannot absorb quietly --
  // unlike the zeros case, it looks right. The payload carries no portfolio id, so
  // record the id beside it and guard the statement branch on the match. Same shape as
  // sellPnl below, and for the reason that comment gives: five places change
  // selectedId, and a reset that has to be remembered in each one is a reset that gets
  // forgotten. Derived, not cleared from an effect.
  const [taxMeta, setTaxMeta] = useState<{ portfolioId: number; fetchedAt: Date } | null>(null);
  const [transactions, setTransactions] = useState<Transaction[]>([]);

  // UI state
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Set when the backend bounced us back with `?auth_error=auth_failed` after a
  // failed sign-in. Deliberately a fixed string rather than anything from the URL:
  // the backend emits exactly this one value and reflects no IdP detail, and
  // echoing a query parameter into the page would be a reflection point for free.
  // Read once during initialisation rather than set from an effect: the URL is
  // already the whole input, and a state update in an effect would repaint the page
  // for a fact that was true before the first render.
  const [authFailed] = useState(
    () => new URLSearchParams(window.location.search).get('auth_error') === 'auth_failed',
  );

  const [showNewPortfolioModal, setShowNewPortfolioModal] = useState(false);
  // Which sell's breakdown is open, and which portfolio it belonged to. The modal
  // renders only while that still matches the selected portfolio, so switching
  // portfolio cannot leave a breakdown on screen for a holding you no longer
  // look at -- with that portfolio's base currency resolved server-side.
  // Derived rather than cleared from an effect: five places change selectedId, and
  // a reset that has to be remembered in each one is a reset that gets forgotten.
  const [sellPnl, setSellPnl] = useState<{ txId: number; portfolioId: number } | null>(null);
  // The breakdown is opened from the ledger and is about a ledger row, so leaving
  // the ledger closes it. Left rendered, it would float over Dashboard or Analytics
  // -- a modal about a transaction on a screen that shows no transactions.
  const selectTab = (tab: string) => {
    setActiveTab(tab);
    setSellPnl(null);
  };
  const [newPortfolioName, setNewPortfolioName] = useState('');
  const [newPortfolioDesc, setNewPortfolioDesc] = useState('');
  const [newPortfolioCurrency, setNewPortfolioCurrency] = useState('USD');

  // Fetch initial portfolios list
  const fetchPortfolios = useCallback(async () => {
    try {
      setLoading(true);
      const res = await apiFetch(`${API_BASE}/portfolios/`);
      if (!res.ok) throw new Error('Failed to fetch portfolios');
      const data = await res.json();
      setPortfolios(data);
      if (data.length > 0 && !selectedId) {
        setSelectedId(data[0].id);
      }
      setLoading(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setLoading(false);
    }
  }, [selectedId]);

  const fetchPortfolioData = useCallback(async (id: number) => {
    try {
      setLoading(true);
      setError(null);
      
      // 1. Fetch performance (which also syncs/caches historical prices in backend)
      const perfRes = await apiFetch(`${API_BASE}/portfolios/${id}/performance`);
      if (!perfRes.ok) throw new Error('Failed to load portfolio performance');
      const perfData = await perfRes.json();
      const normalizedPerf = normalizePerformance(perfData);
      setPerformance(normalizedPerf);

      // 2. Fetch tax lot details
      const taxRes = await apiFetch(`${API_BASE}/portfolios/${id}/tax-summary?strategy=${strategy}&threshold_days=${thresholdDays}`);
      if (!taxRes.ok) throw new Error('Failed to load tax lot summary');
      const taxData = await taxRes.json();
      const normalizedTax = normalizeTaxSummary(taxData);
      setTaxSummary(normalizedTax);
      // The data-at line needs a source, and the statement renders cached state, so the
      // fetch time is captured here rather than at render time.
      setTaxMeta({ portfolioId: id, fetchedAt: new Date() });

      // 3. Fetch transactions list
      const txRes = await apiFetch(`${API_BASE}/portfolios/${id}/transactions/`);
      if (!txRes.ok) throw new Error('Failed to load transactions');
      const txData = await txRes.json();
      setTransactions(normalizeTransactionList(txData));

      // 4. Refresh portfolio object to keep assets list in sync
      const portfolioRes = await apiFetch(`${API_BASE}/portfolios/${id}`);
      if (portfolioRes.ok) {
        const portfolioData = await portfolioRes.json();
        setPortfolios(prev => prev.map(p => p.id === id ? { ...p, assets: normalizeAssetList(portfolioData.assets) } : p));
      }

      setLoading(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setLoading(false);
    }
  }, [strategy, thresholdDays]);

  useEffect(() => {
    const timer = setTimeout(() => {
      fetchPortfolios();
    }, 0);
    return () => clearTimeout(timer);
  }, [fetchPortfolios]);

  // Fetch details whenever selected portfolio, strategy, or threshold changes
  useEffect(() => {
    if (selectedId) {
      const timer = setTimeout(() => {
        fetchPortfolioData(selectedId);
      }, 0);
      return () => clearTimeout(timer);
    }
  }, [selectedId, fetchPortfolioData]);

  const handleCreatePortfolio = async (e: FormEvent) => {
    e.preventDefault();
    if (!newPortfolioName) return;
    try {
      setLoading(true);
      const res = await apiFetch(`${API_BASE}/portfolios/`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: newPortfolioName, description: newPortfolioDesc, currency: newPortfolioCurrency })
      });
      if (!res.ok) throw new Error('Failed to create portfolio');
      const newP = await res.json();
      setPortfolios([...portfolios, newP]);
      setSelectedId(newP.id);
      setNewPortfolioName('');
      setNewPortfolioDesc('');
      setNewPortfolioCurrency('USD');
      setShowNewPortfolioModal(false);
      setLoading(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setLoading(false);
    }
  };

  const handleAddAsset = async (assetData: Partial<Asset>) => {
    try {
      setLoading(true);
      const res = await apiFetch(`${API_BASE}/portfolios/${selectedId}/assets/`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(assetData)
      });
      if (!res.ok) {
        const errorData = await res.json();
        throw new Error(errorData.detail || 'Failed to add asset');
      }
      if (selectedId) await fetchPortfolioData(selectedId);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setLoading(false);
    }
  };

  const handleDeleteAsset = async (assetId: number) => {
    if (!confirm('Are you sure you want to delete this asset? All associated transactions will be removed.')) return;
    try {
      setLoading(true);
      const res = await apiFetch(`${API_BASE}/assets/${assetId}`, { method: 'DELETE' });
      if (!res.ok) throw new Error('Failed to delete asset');
      if (selectedId) await fetchPortfolioData(selectedId);
    } catch (err) {
      alert(`Error deleting asset: ${err instanceof Error ? err.message : String(err)}`);
      setLoading(false);
    }
  };

  const handleAddTransaction = async (assetId: number, txData: CreateTxPayload) => {
    try {
      setLoading(true);
      const res = await apiFetch(`${API_BASE}/portfolios/${selectedId}/assets/${assetId}/transactions/`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(txData)
      });
      if (!res.ok) throw new Error('Failed to log transaction');
      if (selectedId) await fetchPortfolioData(selectedId);
    } catch (err) {
      alert(`Error logging transaction: ${err instanceof Error ? err.message : String(err)}`);
      setLoading(false);
    }
  };

  const handleDeleteTransaction = async (txId: number) => {
    if (!confirm('Delete this transaction?')) return;
    try {
      setLoading(true);
      const res = await apiFetch(`${API_BASE}/transactions/${txId}`, { method: 'DELETE' });
      if (!res.ok) throw new Error('Failed to delete transaction');
      // Only close the breakdown if this is the sell it is showing; deleting an
      // unrelated row should not dismiss a figure someone is reading.
      setSellPnl((current) => (current?.txId === txId ? null : current));
      if (selectedId) await fetchPortfolioData(selectedId);
    } catch (err) {
      alert(`Error deleting transaction: ${err instanceof Error ? err.message : String(err)}`);
      setLoading(false);
    }
  };

  const handleDeletePortfolio = async () => {
    try {
      setLoading(true);
      const res = await apiFetch(`${API_BASE}/portfolios/${selectedId}`, { method: 'DELETE' });
      if (res.status === 409) {
        alert('Cannot delete portfolio: it still contains assets. Remove all assets first.')
        setLoading(false);
        return;
      }
      if (!res.ok) throw new Error('Failed to delete portfolio');
      setSelectedId(null);
      await fetchPortfolios();
      setLoading(false);
    } catch (err) {
      alert(`Error deleting portfolio: ${err instanceof Error ? err.message : String(err)}`);
      setLoading(false);
    }
  };

  // Create mock demo portfolio to wow the user
  const handleLoadDemo = async () => {
    try {
      setLoading(true);
      setError(null);

      // 1. Create portfolio
      const pRes = await apiFetch(`${API_BASE}/portfolios/`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ 
          name: 'Global Retirement Fund', 
          description: 'Diverse core tactical allocation comprising stocks, cryptos, and ETFs.' 
        })
      });
       const p: Portfolio = await pRes.json();

      // 2. Add symbols: AAPL (Stock), BTC-USD (Crypto), SPY (ETF)
       const symbols = [
         { symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', sector: 'Technology', currency: 'USD' },
         { symbol: 'BTC-USD', name: 'Bitcoin', asset_type: 'CRYPTO', sector: 'Financial', currency: 'USD' },
         { symbol: 'SPY', name: 'SPDR S&P 500 ETF', asset_type: 'ETF', sector: 'Index', currency: 'USD' },
         { symbol: 'SAP.DE', name: 'SAP SE', asset_type: 'STOCK', sector: 'Technology', currency: 'EUR' }
       ];

       const createdAssets: Record<string, number> = {};
      for (const s of symbols) {
        const aRes = await apiFetch(`${API_BASE}/portfolios/${p.id}/assets/`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(s)
        });
         const a: Asset = await aRes.json();
        createdAssets[s.symbol] = a.id;
      }

      // 3. Log transactions over the past months to build history
      const now = new Date();
       const txs: Array<{ symbol: string; type: TransactionType; qty: number; price: number; fee: number; daysAgo: number }> = [
        // AAPL Buys
        { symbol: 'AAPL', type: 'BUY', qty: 20, price: 175.0, fee: 5.0, daysAgo: 60 },
        { symbol: 'AAPL', type: 'BUY', qty: 15, price: 185.0, fee: 5.0, daysAgo: 30 },
        { symbol: 'AAPL', type: 'BUY', qty: 10, price: 190.0, fee: 5.0, daysAgo: 10 },
        { symbol: 'AAPL', type: 'SELL', qty: 8, price: 210.0, fee: 8.0, daysAgo: 2 }, // triggers tax matching
        
        // BTC Buys
        { symbol: 'BTC-USD', type: 'BUY', qty: 0.25, price: 60000.0, fee: 15.0, daysAgo: 45 },
        { symbol: 'BTC-USD', type: 'BUY', qty: 0.15, price: 65000.0, fee: 10.0, daysAgo: 15 },
        { symbol: 'BTC-USD', type: 'SELL', qty: 0.10, price: 69000.0, fee: 12.0, daysAgo: 1 },

         // SPY Buys
         { symbol: 'SPY', type: 'BUY', qty: 30, price: 500.0, fee: 0.0, daysAgo: 90 },
         { symbol: 'SPY', type: 'BUY', qty: 10, price: 520.0, fee: 0.0, daysAgo: 25 },

         // SAP.DE Buys
         { symbol: 'SAP.DE', type: 'BUY', qty: 5, price: 170.0, fee: 2.0, daysAgo: 40 },
         { symbol: 'SAP.DE', type: 'BUY', qty: 5, price: 175.0, fee: 2.0, daysAgo: 10 }
       ];

      for (const tx of txs) {
        const txDate = new Date(now.getTime() - tx.daysAgo * 24 * 60 * 60 * 1000);
        await apiFetch(`${API_BASE}/portfolios/${p.id}/assets/${createdAssets[tx.symbol]}/transactions/`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            type: tx.type,
            quantity: tx.qty,
            price: tx.price,
            fee: tx.fee,
            date: txDate.toISOString()
          })
        });
      }

      // Reload portfolios list and set active
      await fetchPortfolios();
      setSelectedId(p.id);
    } catch (err) {
      setError(`Failed to create demo portfolio: ${err instanceof Error ? err.message : String(err)}`);
      setLoading(false);
    }
  };

  const currentPortfolio = portfolios.find(p => p.id === selectedId);

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    if (params.get('auth_error') === 'auth_failed') {
      params.delete('auth_error');
      // Strip it so a refresh, a bookmark, or a shared URL does not resurrect a
      // sign-in failure that has already been resolved.
      const query = params.toString();
      window.history.replaceState({}, '', `${window.location.pathname}${query ? `?${query}` : ''}`);
    }
  }, []);

  return (
    <div className="app-container">
      {authFailed && (
        <div
          role="alert"
          style={{
            position: 'fixed',
            top: 0,
            left: 0,
            right: 0,
            zIndex: 1000,
            padding: '12px 16px',
            background: 'var(--danger-color, #b3261e)',
            color: '#fff',
            textAlign: 'center',
            fontWeight: 600,
          }}
        >
          Sign-in failed. Please try again.
        </div>
      )}
      {/* Sidebar Navigation */}
      <aside className="sidebar">
        <div className="brand">
          <img src="/logo.svg" alt="Logo" className="brand-logo" />
          <span>Portfolio Prism</span>
        </div>

        <ul className="nav-menu">
          <li className={`nav-item ${activeTab === 'dashboard' ? 'active' : ''}`} onClick={() => selectTab('dashboard')}>
            <LayoutDashboard size={18} /> Dashboard
          </li>
          <li className={`nav-item ${activeTab === 'portfolios' ? 'active' : ''}`} onClick={() => selectTab('portfolios')}>
            <Briefcase size={18} /> Holdings & Tax lots
          </li>
          <li className={`nav-item ${activeTab === 'analytics' ? 'active' : ''}`} onClick={() => selectTab('analytics')}>
            <BarChart3 size={18} /> Risk & Correlation
          </li>
          <li className={`nav-item ${activeTab === 'ledger' ? 'active' : ''}`} onClick={() => selectTab('ledger')}>
            <History size={18} /> Transaction Ledger
          </li>
          <li className={`nav-item ${activeTab === 'statement' ? 'active' : ''}`} onClick={() => selectTab('statement')}>
            <Printer size={18} /> Printable Statement
          </li>
        </ul>

        {/* Portfolio Switcher */}
        <div style={{ marginTop: 'auto', paddingTop: '24px', borderTop: '1px solid var(--border-color)' }}>
          <label style={{ fontSize: '0.75rem', color: 'var(--text-muted)', fontWeight: 600, display: 'block', marginBottom: '8px' }}>ACTIVE PORTFOLIO</label>
          {portfolios.length > 0 ? (
            <select 
              value={selectedId || ''} 
               onChange={(e: ChangeEvent<HTMLSelectElement>) => setSelectedId(parseInt(e.target.value))}
              className="form-control"
              style={{ marginBottom: '12px' }}
            >
              {portfolios.map(p => (
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
          ) : (
            <p style={{ fontSize: '0.875rem', color: 'var(--text-muted)', marginBottom: '12px' }}>No portfolios found</p>
          )}

          <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
            <button onClick={() => setShowNewPortfolioModal(true)} className="btn btn-secondary" style={{ width: '100%', justifyContent: 'center', fontSize: '0.875rem' }}>
              <Plus size={14} /> New Portfolio
            </button>
            {portfolios.length === 0 && (
              <button onClick={handleLoadDemo} className="btn btn-primary" style={{ width: '100%', justifyContent: 'center', fontSize: '0.875rem' }}>
                <Sparkles size={14} /> Load Demo Fund
              </button>
            )}
            {/*
              A plain anchor, not `apiFetch`. Logout is a redirect chain that leaves
              the origin for the identity provider and comes back; a `fetch` would
              follow those hops invisibly and the browser would never end its session
              there. The backend clears the cookie and forwards to the provider's
              end-session endpoint.
            */}
            <a href="/auth/logout" className="btn btn-secondary" style={{ width: '100%', justifyContent: 'center', fontSize: '0.875rem' }}>
              <LogOut size={14} /> Sign out
            </a>
            {/*
              Measured against the live provider, not guessed: this clears the app
              session and the provider then shows a page that offers to log you back
              in without asking for credentials, because the logout carries no
              `id_token_hint` — the ID token is deliberately not retained. The full
              sign-out is one more click on that page. Saying so is cheaper than
              leaving the user to discover that "signed out" still leaves the browser
              able to re-enter. Kept outside the anchor so the link's accessible name
              stays "Sign out".
            */}
            <p style={{ fontSize: '0.7rem', color: 'var(--text-muted)', marginTop: '6px', lineHeight: 1.4 }}>
              Ends this app&apos;s session. Your sign-in provider stays signed in —
              choose its log-out option on the page that follows to end that too.
            </p>
          </div>
        </div>
      </aside>

      {/* Main Content Pane */}
      <main className="main-content">
        {/* Header Row */}
        <div className="header-row">
          <div className="header-title">
            <h1>{currentPortfolio ? currentPortfolio.name : 'Portfolio Prism'}</h1>
            <p>{currentPortfolio ? `Holdings analytics in base currency: ${currentPortfolio.currency}` : 'Select or create a portfolio to begin.'}</p>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: '16px' }}>
            {loading && <Loader2 size={20} className="animate-spin" style={{ color: 'var(--color-primary)' }} />}
            {error && (
              <div style={{ display: 'flex', alignItems: 'center', gap: '8px', background: 'var(--color-danger-glow)', border: '1px solid rgba(239, 68, 68, 0.2)', padding: '8px 16px', borderRadius: '8px', color: 'var(--color-danger)', fontSize: '0.875rem' }}>
                <AlertTriangle size={16} /> Error: {error}
              </div>
            )}
          </div>
        </div>

        {/* Tab View Selection */}
        {currentPortfolio ? (
          <>
            {activeTab === 'dashboard' && (
              <Dashboard 
                performance={performance} 
                taxSummary={taxSummary} 
              />
            )}

{activeTab === 'portfolios' && taxSummary && (
                <PortfolioDetail 
                  portfolio={currentPortfolio}
                  taxSummary={taxSummary}
                  onAddAsset={handleAddAsset}
                  onDeleteAsset={handleDeleteAsset}
                  onDeletePortfolio={handleDeletePortfolio}
                  onAddTransaction={handleAddTransaction}
                  onFetchPortfolioData={() => {
                    // currentPortfolio is derived from selectedId, so this branch cannot
                    // render with selectedId null -- but the state type says otherwise and
                    // fetchPortfolioData takes a number. Guard instead of asserting.
                    if (selectedId === null) return Promise.resolve();
                    return fetchPortfolioData(selectedId);
                  }}
                  strategy={strategy}
                  setStrategy={setStrategy}
                  thresholdDays={thresholdDays}
                  setThresholdDays={setThresholdDays}
                />
              )}

            {activeTab === 'analytics' && (
              <AnalyticsView performance={performance} currency={currentPortfolio?.currency || 'USD'} />
            )}

            {activeTab === 'ledger' && (
              <TransactionLedger
                transactions={transactions}
                assets={currentPortfolio.assets || []}
                onDeleteTransaction={handleDeleteTransaction}
                onSelectSell={(txId) => setSellPnl({ txId, portfolioId: currentPortfolio.id })}
              />
            )}

            {/* The fragment creates no DOM node, which is what keeps both children direct
                children of .main-content -- the position print.css needs to hide the Print
                control and leave only the statement. A layout <div> here would be hidden
                with the control, and the tab would print a blank page while looking correct
                on screen. */}
            {activeTab === 'statement' && taxSummary && taxMeta?.portfolioId === selectedId && (
              <>
                <button
                  className="btn btn-primary"
                  data-testid="print-button"
                  onClick={() => window.print()}
                >
                  Print
                </button>
                <PortfolioStatement
                  taxSummary={taxSummary}
                  portfolio={currentPortfolio}
                  fetchedAt={taxMeta.fetchedAt}
                />
              </>
            )}

            {sellPnl !== null && sellPnl.portfolioId === selectedId && (
              <SellPnlModal
                transactionId={sellPnl.txId}
                strategy={strategy}
                thresholdDays={thresholdDays}
                onClose={() => setSellPnl(null)}
              />
            )}
          </>
        ) : (
          <div className="glass-card" style={{ padding: '48px', textAlign: 'center' }}>
            <FolderOpen size={48} style={{ color: 'var(--text-muted)', marginBottom: '16px' }} />
            <h2>No Portfolio Active</h2>
            <p style={{ color: 'var(--text-secondary)', marginTop: '8px', marginBottom: '24px' }}>
              Create a new custom portfolio or load our pre-populated demonstration fund to explore the tax engines.
            </p>
            <button onClick={handleLoadDemo} className="btn btn-primary">
              <Sparkles size={16} /> Load Demonstration Fund
            </button>
          </div>
        )}
      </main>

      {/* NEW PORTFOLIO MODAL */}
      {showNewPortfolioModal && (
        <div style={{ position: 'fixed', top: 0, left: 0, right: 0, bottom: 0, background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(4px)', display: 'flex', justifyContent: 'center', alignItems: 'center', zIndex: 100 }}>
          <div className="glass-card" style={{ padding: '32px', width: '450px', background: '#121929' }}>
            <h3 style={{ marginBottom: '20px' }}>Create New Portfolio</h3>
            <form onSubmit={handleCreatePortfolio}>
              <div className="form-group">
                <label>Portfolio Name</label>
                <input 
                  type="text" 
                  value={newPortfolioName} 
                  onChange={(e) => setNewPortfolioName(e.target.value)} 
                  className="form-control"
                  placeholder="e.g. Long-term Growth, Crypto Play, Tax Harvesting"
                  required 
                />
              </div>
              <div className="form-group">
                <label>Description</label>
                <textarea 
                  value={newPortfolioDesc} 
                  onChange={(e) => setNewPortfolioDesc(e.target.value)} 
                  className="form-control"
                  rows={3}
                  placeholder="Describe the objective of this portfolio..."
                />
              </div>
              <div className="form-group">
                <label>Currency</label>
                <select
                  value={newPortfolioCurrency}
                  onChange={(e) => setNewPortfolioCurrency(e.target.value)}
                  className="form-control"
                >
                  <option value="USD">USD - US Dollar</option>
                  <option value="EUR">EUR - Euro</option>
                  <option value="GBP">GBP - British Pound</option>
                  <option value="JPY">JPY - Japanese Yen</option>
                  <option value="CHF">CHF - Swiss Franc</option>
                  <option value="CAD">CAD - Canadian Dollar</option>
                  <option value="AUD">AUD - Australian Dollar</option>
                  <option value="NZD">NZD - New Zealand Dollar</option>
                  <option value="CNY">CNY - Chinese Yuan</option>
                  <option value="INR">INR - Indian Rupee</option>
                </select>
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '12px', marginTop: '24px' }}>
                <button type="button" onClick={() => setShowNewPortfolioModal(false)} className="btn btn-secondary">Cancel</button>
                <button type="submit" className="btn btn-primary">Create Portfolio</button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
}
