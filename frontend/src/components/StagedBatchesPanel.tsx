import { useEffect, useState } from 'react';
import { apiFetch, API_BASE } from '../utils/apiFetch';
import { CheckCircle2, Inbox, ShieldCheck, XCircle } from 'lucide-react';

/**
 * What an agent has staged and a human has to decide about.
 *
 * This panel performs exactly one write: `staged -> approved`. It never creates, edits or
 * deletes a portfolio, an asset or a transaction, and that is the whole design rather than
 * a limitation of this component. The agent holds the token and the data; the human holds
 * the only credential that can turn a proposal into a row. If this panel also wrote
 * holdings, the approval would be one more form on the screen instead of the only thing on
 * it.
 *
 * Two rules the rendering follows, both because the alternative is a lie:
 *
 * **The preview is the server's JSON, rendered as stored.** The backend keeps the preview
 * beside the payload at staging time, so what the agent was shown and what the human reads
 * are the same bytes. Recomposing it here — nicer tables, reformatted numbers, a currency
 * symbol the payload did not carry — would mean the human approved a rendering rather than
 * the proposal, and the diff between the two is exactly what nobody would notice.
 *
 * **`expired` is a server verdict, not a clock reading.** `Date.now()` in a render is both
 * impure and a second authority: the backend decides liveness against its own clock, and a
 * browser whose clock is minutes off would offer to approve a batch the server will refuse.
 * So the field arrives on the row and this component only displays it.
 */

interface BatchRow {
  handle: string;
  status: string;
  created_at: string;
  expires_at: string;
  approved_at: string | null;
  applied_at: string | null;
  asset_count: number;
  transaction_count: number;
  /** Server-computed. See the panel comment: the browser's clock is not an authority here. */
  expired: boolean;
}

/** The detail response is the stored preview with three fields added by the route. Its
 * shape is the agent's choosing, so nothing here is typed beyond what the panel reads. */
type Preview = Record<string, unknown> & {
  status?: string;
  expires_at?: string;
  expired?: boolean;
};

const STATUS_LABEL: Record<string, string> = {
  staged: 'Waiting for you',
  approved: 'Approved',
  applied: 'Applied',
  discarded: 'Discarded',
};

function fmtTime(iso: string): string {
  return new Date(iso).toLocaleString();
}

export default function StagedBatchesPanel() {
  // `null` means "not loaded yet", which is a different state from "loaded and empty" and
  // has to stay distinguishable: an empty list is a fact about the user, a null is a fact
  // about the request.
  const [batches, setBatches] = useState<BatchRow[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  // Reads and renders are separated on purpose. This returns a result and touches no state,
  // so every `setState` lands inside a `.then` at a call site — which is what the purity
  // rule asks for, and the honest shape: a request in flight has not decided anything yet.
  // The previous version called a `load()` that set state itself, and the linter correctly
  // refused it.
  const fetchBatches = async (): Promise<
    { ok: true; rows: BatchRow[] } | { ok: false; why: string }
  > => {
    try {
      const res = await apiFetch(`${API_BASE}/staged-batches`);
      if (!res.ok) {
        return { ok: false, why: `Could not load staged batches (HTTP ${res.status}).` };
      }
      return { ok: true, rows: (await res.json()) as BatchRow[] };
    } catch {
      return { ok: false, why: 'Could not reach the server.' };
    }
  };

  const load = () => {
    fetchBatches().then((r) => {
      if (r.ok) {
        setBatches(r.rows);
        setLoadError(null);
      } else {
        setLoadError(r.why);
      }
    });
  };

  useEffect(() => {
    let cancelled = false;
    // The `cancelled` flag is the part the linter cannot write for us: a slow response must
    // not land on a component that has already moved on.
    fetchBatches().then((r) => {
      if (cancelled) return;
      if (r.ok) {
        setBatches(r.rows);
        setLoadError(null);
      } else {
        setLoadError(r.why);
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const open = async (handle: string) => {
    setSelected(handle);
    setPreview(null);
    setPreviewError(null);
    try {
      const res = await apiFetch(`${API_BASE}/staged-batches/${handle}`);
      if (!res.ok) {
        setPreviewError(`Could not load this batch (HTTP ${res.status}).`);
        return;
      }
      setPreview((await res.json()) as Preview);
    } catch {
      setPreviewError('Could not reach the server.');
    }
  };

  const approve = async (handle: string) => {
    if (!window.confirm('Approve this batch? An agent will be able to write these rows.')) {
      return;
    }
    setBusy(true);
    setNotice(null);
    try {
      const res = await apiFetch(`${API_BASE}/staged-batches/${handle}/approve`, {
        method: 'POST',
      });
      const body = await res.json().catch(() => ({}));
      if (res.ok) {
        setNotice('Approved. An agent can now apply this batch.');
      } else if (res.status === 409) {
        // Prefer the server's own sentence. It knows the reason and the panel does not: an
        // expired batch is still `staged`, and "someone else acted on it first" would be a
        // fabrication about a batch that nobody touched — the clock ran out. The composed
        // fallback is only for a server old enough not to send `detail`.
        const detail = typeof body.detail === 'string' ? body.detail : null;
        setNotice(
          detail
            ? `Not approved — ${detail}`
            : `Not approved — the batch is '${
                typeof body.current_status === 'string' ? body.current_status : 'unknown'
              }'. Someone else acted on it first.`,
        );
      } else {
        setNotice(`Not approved (HTTP ${res.status}).`);
      }
      await load();
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="glass-card" data-testid="staged-batches-panel" style={{ padding: '24px' }}>
      <h2>Agent staging area</h2>
      <p className="muted">
        An agent can propose rows, and proposing writes nothing. Nothing here becomes part of a
        portfolio until you approve it. This panel approves; it does not edit.
      </p>

      {loadError && <p className="error">{loadError}</p>}
      {notice && (
        <p className="muted" role="status" data-testid="staged-notice">
          {notice}
        </p>
      )}

      {batches === null ? (
        !loadError && <p className="muted">Loading…</p>
      ) : batches.length === 0 ? (
        <p className="muted" data-testid="staged-empty">
          <Inbox size={14} /> Nothing is waiting.
        </p>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>Staged</th>
              <th>Status</th>
              <th>Assets</th>
              <th>Rows</th>
              <th>Expires</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {batches.map((b) => (
              <tr key={b.handle} data-testid={`staged-row-${b.handle}`}>
                <td>{fmtTime(b.created_at)}</td>
                <td>
                  {STATUS_LABEL[b.status] ?? b.status}
                  {b.status === 'staged' && b.expired && (
                    // Expired and still unapproved: the offer has lapsed, and saying so is
                    // the difference between a queue and a mystery.
                    <span className="error" data-testid="staged-expired">
                      {' '}
                      — expired
                    </span>
                  )}
                </td>
                <td>{b.asset_count}</td>
                <td>{b.transaction_count}</td>
                <td>{fmtTime(b.expires_at)}</td>
                <td>
                  <button type="button" onClick={() => open(b.handle)}>
                    Review
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {selected && (
        <div className="glass-card" data-testid="staged-detail" style={{ padding: '24px' }}>
          <h3>Proposal</h3>
          {previewError && <p className="error">{previewError}</p>}
          {preview && (
            <>
              {/* Rendered as stored JSON. See the panel comment: a prettier rendering is a
                  different document, and the human would be approving the rendering. */}
              <pre data-testid="staged-preview">{JSON.stringify(preview, null, 2)}</pre>
              {preview.status === 'staged' && !preview.expired && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => approve(selected)}
                  data-testid="staged-approve"
                >
                  <CheckCircle2 size={14} /> Approve this batch
                </button>
              )}
              {preview.status === 'staged' && preview.expired && (
                <p className="error">
                  <XCircle size={14} /> This proposal has expired. Ask the agent to stage it
                  again.
                </p>
              )}
              {preview.status !== 'staged' && (
                <p className="muted">
                  <ShieldCheck size={14} /> This batch is already {preview.status}; nothing here
                  can change it.
                </p>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}
