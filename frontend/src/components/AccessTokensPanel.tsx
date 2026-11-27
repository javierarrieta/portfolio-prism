import { useCallback, useEffect, useState } from 'react';
import { KeyRound, Copy, Check, Trash2 } from 'lucide-react';
import { apiFetch, API_BASE } from '../utils/apiFetch';

/**
 * Personal access tokens: what an agent holds instead of a session.
 *
 * The panel has one job that no other panel here has, and it shapes everything
 * below: the secret exists for exactly one response. There is no endpoint that can
 * show it again — the server stores only a hash — so the mint flow has to make that
 * impossible to miss rather than assume the user read the docs. Hence the modal that
 * appears only after a successful mint, the copy button, and the sentence saying
 * plainly that this is the only showing.
 *
 * Revoke is a plain confirm and not a typed word. `BackupPanel` asks you to type
 * because `mode=replace` deletes rows; a token is revocable and re-mintable, and
 * friction spent on a reversible action teaches people to stop reading the dialogs.
 *
 * The list never receives a secret, because `GET /api/tokens` does not select one.
 * Nothing here can reveal a token; it can only mint a new one or destroy an old one.
 */

interface TokenRow {
  id: number;
  label: string;
  scopes: string[];
  created_at: string;
  expires_at: string | null;
  last_used_at: string | null;
  revoked_at: string | null;
  stale: boolean;
  expired: boolean;
}

interface MintedToken {
  id: number;
  secret: string;
  label: string;
  scopes: string[];
  expires_at: string | null;
  created_at: string;
}

/** The scope vocabulary, mirrored from the server's `Scope` enum.
 * Kept as data rather than inlined in the form so the labels and the wire values
 * cannot drift from each other; the server remains the authority and refuses any
 * name it does not know. */
const SCOPES: { value: string; label: string; blurb: string }[] = [
  { value: 'read', label: 'Read', blurb: 'Look at portfolios, transactions and tax summaries.' },
  { value: 'import', label: 'Import', blurb: 'Stage transactions for import. Still needs you to approve them.' },
];

const EXPIRY_CHOICES = [
  { days: 0, label: 'Never' },
  { days: 30, label: '30 days' },
  { days: 90, label: '90 days' },
  { days: 365, label: '1 year' },
];

const fmt = (iso: string | null): string =>
  iso ? new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' }) : '—';

export default function AccessTokensPanel() {
  // null means "not loaded yet". A separate `loading` boolean would have to be set
  // when the effect starts, and a synchronous setState at the top of an effect is a
  // second render for no information — the null already says the same thing.
  const [tokens, setTokens] = useState<TokenRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [label, setLabel] = useState('');
  const [scopes, setScopes] = useState<string[]>(['read']);
  const [expiryDays, setExpiryDays] = useState(0);
  const [minting, setMinting] = useState(false);

  // The one-time reveal. Held in state and nowhere else: on reload it is gone, which
  // is the intended behaviour and not a bug to fix.
  const [revealed, setRevealed] = useState<MintedToken | null>(null);
  const [copied, setCopied] = useState(false);

  /** The list, as the server described it. Not a state setter: an effect may not call
   * one synchronously, so the fetch is the shared part and each caller decides what to
   * do with the rows. */
  const fetchTokens = async (): Promise<TokenRow[]> => {
    const res = await apiFetch(`${API_BASE}/tokens`);
    if (!res.ok) throw new Error(`status ${res.status}`);
    return (await res.json()) as TokenRow[];
  };

  useEffect(() => {
    // `cancelled` because a slow response arriving after unmount would set state on a
    // component that no longer exists.
    let cancelled = false;
    fetchTokens()
      .then((rows) => !cancelled && setTokens(rows))
      .catch(() => !cancelled && setError('Could not reach the server.'));
    return () => {
      cancelled = true;
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      setTokens(await fetchTokens());
      setError(null);
    } catch {
      setError('Could not load tokens.');
    }
  }, []);

  const mint = async () => {
    if (!label.trim() || scopes.length === 0 || minting) return;
    setMinting(true);
    try {
      const res = await apiFetch(`${API_BASE}/tokens`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          label: label.trim(),
          scopes,
          ...(expiryDays > 0 ? { expires_in_days: expiryDays } : {}),
        }),
      });
      const body = await res.json();
      if (!res.ok) {
        setError(body.message ?? body.error ?? 'Could not create the token.');
        return;
      }
      setError(null);
      setRevealed(body as MintedToken);
      setCopied(false);
      setLabel('');
      await refresh();
    } catch {
      setError('Could not reach the server.');
    } finally {
      setMinting(false);
    }
  };

  const revoke = async (row: TokenRow) => {
    // Deliberately not a typed confirmation: revoking is reversible in effect (mint
    // another) and costs nothing but a re-paste.
    if (!window.confirm(`Revoke "${row.label}"? Anything using it stops immediately.`)) return;
    const res = await apiFetch(`${API_BASE}/tokens/${row.id}`, { method: 'DELETE' });
    if (res.status !== 204 && res.status !== 404) {
      setError(`Could not revoke the token (${res.status}).`);
      return;
    }
    await refresh();
  };

  const copySecret = async () => {
    if (!revealed) return;
    try {
      await navigator.clipboard.writeText(revealed.secret);
      setCopied(true);
    } catch {
      // Clipboard access is refused in insecure contexts and in some browsers'
      // automation. The secret is on screen either way, so this is a convenience
      // failure and not a reason to stop the reveal.
      setCopied(false);
    }
  };

  return (
    <div style={{ display: 'grid', gap: '24px' }}>
      <div className="glass-card" style={{ padding: '24px' }}>
        <h2 style={{ marginTop: 0 }}>Access tokens</h2>
        <p style={{ color: 'var(--text-secondary)', maxWidth: '62ch' }}>
          A token is what an agent or a script presents instead of signing in. It can do
          only what its scopes allow, it stops working the moment you revoke it, and it
          cannot create another token — minting happens here, in a browser, and nowhere
          else.
        </p>

        <div style={{ display: 'flex', flexWrap: 'wrap', gap: '16px', alignItems: 'flex-end' }}>
          <label style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
            <span style={{ fontSize: '13px', color: 'var(--text-secondary)' }}>Name</span>
            <input
              className="form-control"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="e.g. laptop agent"
              style={{ minWidth: '220px' }}
            />
          </label>

          <fieldset style={{ border: '1px solid var(--border-color)', borderRadius: '8px', padding: '8px 12px' }}>
            <legend style={{ fontSize: '13px', color: 'var(--text-secondary)', padding: '0 6px' }}>Scopes</legend>
            {SCOPES.map((s) => (
              <label key={s.value} title={s.blurb} style={{ display: 'block', fontSize: '14px' }}>
                <input
                  type="checkbox"
                  checked={scopes.includes(s.value)}
                  onChange={(e) =>
                    setScopes((prev) => (e.target.checked ? [...prev, s.value] : prev.filter((v) => v !== s.value)))
                  }
                />{' '}
                {s.label}
              </label>
            ))}
          </fieldset>

          <label style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
            <span style={{ fontSize: '13px', color: 'var(--text-secondary)' }}>Expires</span>
            <select
              className="form-control"
              value={expiryDays}
              onChange={(e) => setExpiryDays(Number(e.target.value))}
            >
              {EXPIRY_CHOICES.map((c) => (
                <option key={c.days} value={c.days}>
                  {c.label}
                </option>
              ))}
            </select>
          </label>

          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void mint()}
            disabled={!label.trim() || scopes.length === 0 || minting}
          >
            <KeyRound size={16} /> Create token
          </button>
        </div>

        {scopes.length === 0 && (
          <p style={{ color: 'var(--color-warning)', fontSize: '13px' }}>Pick at least one scope.</p>
        )}
        {error && <p style={{ color: 'var(--color-danger)', fontSize: '14px' }}>{error}</p>}
      </div>

      <div className="glass-card" style={{ padding: '24px' }}>
        <h3 style={{ marginTop: 0 }}>Your tokens</h3>
        {tokens === null && <p style={{ color: 'var(--text-secondary)' }}>Loading…</p>}
        {tokens?.length === 0 && <p style={{ color: 'var(--text-secondary)' }}>No tokens yet.</p>}
        {tokens && tokens.length > 0 && (
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr style={{ textAlign: 'left', color: 'var(--text-secondary)', fontSize: '13px' }}>
                <th style={{ padding: '6px 8px' }}>Name</th>
                <th style={{ padding: '6px 8px' }}>Scopes</th>
                <th style={{ padding: '6px 8px' }}>Created</th>
                <th style={{ padding: '6px 8px' }}>Last used</th>
                <th style={{ padding: '6px 8px' }}>Expires</th>
                <th style={{ padding: '6px 8px' }} />
              </tr>
            </thead>
            <tbody>
              {tokens.map((t) => {
                // Both read from the server, not from a clock here. `lookup` decides
                // liveness against the database's `now()`; a second answer computed
                // from the browser's clock is a second authority, and the two would
                // disagree about a token being usable.
                const revoked = t.revoked_at !== null;
                const expired = t.expired;
                return (
                  <tr key={t.id} style={{ borderTop: '1px solid var(--border-color)', opacity: revoked ? 0.55 : 1 }}>
                    <td style={{ padding: '8px' }}>
                      {t.label}
                      {revoked && <em style={{ color: 'var(--text-secondary)' }}> (revoked)</em>}
                      {!revoked && expired && <em style={{ color: 'var(--text-secondary)' }}> (expired)</em>}
                      {/* The nudge. Computed server-side so the threshold is one number
                          in one place rather than a constant that also lives here. */}
                      {!revoked && !expired && t.stale && (
                        <em style={{ color: 'var(--color-warning)' }}> — unused for 90 days</em>
                      )}
                    </td>
                    <td style={{ padding: '8px' }}>{t.scopes.join(', ')}</td>
                    <td style={{ padding: '8px' }}>{fmt(t.created_at)}</td>
                    <td style={{ padding: '8px' }}>{fmt(t.last_used_at)}</td>
                    <td style={{ padding: '8px' }}>{t.expires_at ? fmt(t.expires_at) : 'never'}</td>
                    <td style={{ padding: '8px', textAlign: 'right' }}>
                      {!revoked && (
                        <button type="button" className="btn" onClick={() => void revoke(t)} title="Revoke">
                          <Trash2 size={15} />
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      {revealed && (
        <div
          role="dialog"
          aria-modal="true"
          aria-label="New token"
          style={{
            position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)', backdropFilter: 'blur(4px)',
            display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 100,
          }}
        >
          <div className="glass-card" style={{ padding: '28px', maxWidth: '560px' }}>
            <h3 style={{ marginTop: 0 }}>Copy this token now</h3>
            <p style={{ color: 'var(--color-warning)', fontWeight: 600 }}>
              This is the only time it will be shown. There is no way to retrieve it.
            </p>
            <code
              style={{
                display: 'block', padding: '12px', wordBreak: 'break-all',
                background: 'rgba(0,0,0,0.35)', borderRadius: '8px', fontSize: '13px',
              }}
            >
              {revealed.secret}
            </code>
            <p style={{ color: 'var(--text-secondary)', fontSize: '13px' }}>
              {revealed.label} · {revealed.scopes.join(', ')}
            </p>
            <div style={{ display: 'flex', gap: '10px', marginTop: '16px' }}>
              <button type="button" className="btn btn-primary" onClick={() => void copySecret()}>
                {copied ? <Check size={16} /> : <Copy size={16} />} {copied ? 'Copied' : 'Copy'}
              </button>
              <button type="button" className="btn" onClick={() => setRevealed(null)}>
                Done
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
