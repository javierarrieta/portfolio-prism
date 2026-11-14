import { useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from 'react';
import { apiFetch, API_BASE } from '../utils/apiFetch';

/**
 * Export and restore for the signed-in user's own portfolios: one card per half, in
 * one panel, so neither is looked for in a different place from the other.
 *
 * Export is fetched rather than linked even though it ends as a file, because the
 * same route that answers with the bundle answers `422` with the reason the
 * selection was refused — and a refusal carries no `Content-Disposition`, so a link
 * the browser navigates leaves the user standing on a JSON error body instead of
 * leaving the reason in the panel. The href stays the real route, so ctrl+click and
 * "save link as" still work the browser's way.
 *
 * The half that writes cannot be a link for the same reason in reverse: a destructive
 * write has to say what it will do before it offers to do it, and an href cannot
 * hold a plan. That card belongs here beside this one, reading the same capabilities.
 */

interface PortfolioOption {
  id: number;
  name: string;
}

/** What `/backup/capabilities` says this server will accept. Read, never mirrored
 * into a constant here: a limit the UI hard-codes is a limit the server can change
 * without telling it, and then the panel is quietly lying about what it will take. */
interface Capabilities {
  schema_versions: number[];
  allow_price_restore: boolean;
  max_bytes: number;
  max_rows: number;
}

/** The two shapes a backup refusal arrives in: one `error` sentence, and — for a
 * bundle that failed validation — the list of what is wrong with it. */
interface FailureBody {
  error?: string;
  errors?: { path: string; reason: string }[];
}

/** One sentence out of a refusal. Naming the first few validation failures is what
 * makes a bad bundle fixable rather than mysterious. The byte and row caps arrive as
 * a whole-bundle `error` with no `errors` at all, and render as that sentence alone. */
function describeFailure(body: FailureBody): string {
  const head = body.error ?? 'The server refused the request.';
  if (!body.errors?.length) return head;
  const named = body.errors.slice(0, 5).map((e) => (e.path ? `${e.path}: ${e.reason}` : e.reason));
  const more = body.errors.length > named.length ? ` (and ${body.errors.length - named.length} more)` : '';
  return `${head} ${named.join('; ')}${more}`;
}

/** The refusal body, or a sentence saying there was no readable one. A gateway's
 * HTML page is not the backend talking, and rendering it is worse than admitting it. */
async function readFailure(res: Response): Promise<FailureBody> {
  try {
    return ((await res.json()) ?? {}) as FailureBody;
  } catch {
    return { error: `The server answered ${res.status} without a reason this app could read.` };
  }
}

/** The name the server chose, off its `Content-Disposition`. The route names the
 * file after the day it was built, which is the name that survives a folder full of
 * them; the fallback is for an answer that named nothing, and is not a guess anyone
 * should rely on. */
function filenameFrom(disposition: string | null): string {
  return disposition?.match(/filename="([^"]+)"/)?.[1]
    ?? disposition?.match(/filename=([^";]+)/)?.[1]
    ?? 'portfolio-prism-backup.json';
}

/** Only a plain left click goes through `fetch`. A modified click is the browser's
 * own save-as or new tab, and the href exists precisely so that path still works. */
function isPlainLeftClick(event: ReactMouseEvent<HTMLAnchorElement>): boolean {
  return event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey;
}

interface Props {
  portfolios: PortfolioOption[];
}

export default function BackupPanel({ portfolios }: Props) {
  const [caps, setCaps] = useState<Capabilities | null>(null);

  // One read for the whole panel, handed down rather than fetched per card: the cards
  // describe the same server, and a second request would be a second answer to
  // disagree with. A failure is not fatal — the export needs no capability to work,
  // so the panel stays usable and the note about limits simply does not appear.
  useEffect(() => {
    let cancelled = false;
    apiFetch(`${API_BASE}/backup/capabilities`)
      .then((res) => (res.ok ? res.json() : null))
      .then((body) => { if (!cancelled && body) setCaps(body as Capabilities); })
      .catch(() => { /* nothing to switch off: see the comment above */ });
    return () => { cancelled = true; };
  }, []);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '24px' }}>
      <ExportCard portfolios={portfolios} caps={caps} />
    </div>
  );
}

function ExportCard({ portfolios, caps }: { portfolios: PortfolioOption[]; caps: Capabilities | null }) {
  const [selected, setSelected] = useState<number[]>([]);
  const [includePrices, setIncludePrices] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);

  // Selection order is the user's, and the href preserves it, so the bundle's
  // portfolio order matches the order they ticked.
  const toggle = (id: number) => {
    // A refusal was about the selection that is being edited, so it stops being
    // true the moment that selection changes. Leaving it up claims a refusal the
    // user has already fixed.
    setExportError(null);
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  };

  const exportHref = useMemo(() => {
    if (selected.length === 0) return '';
    // One comma-separated parameter, spelled out rather than built by
    // `URLSearchParams`, which would encode the comma: the route takes a single
    // `portfolio_ids` and refuses the parameter twice, and ids are numbers, so the
    // comma is the only punctuation this query has.
    const ids = `portfolio_ids=${selected.join(',')}`;
    const prices = includePrices && caps?.allow_price_restore ? '&include_prices=1' : '';
    return `${API_BASE}/backup/export?${ids}${prices}`;
  }, [selected, includePrices, caps]);

  async function download() {
    if (exportHref === '' || downloading) return;
    setDownloading(true);
    setExportError(null);
    try {
      const res = await apiFetch(exportHref);
      if (!res.ok) {
        setExportError(describeFailure(await readFailure(res)));
        return;
      }
      // The bytes go to the download manager as they arrived. Nothing here parses or
      // re-serialises the bundle, which is how a download ends up empty or corrupt.
      const url = URL.createObjectURL(await res.blob());
      const link = document.createElement('a');
      link.href = url;
      link.download = filenameFrom(res.headers.get('Content-Disposition'));
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
    } catch {
      setExportError('The download could not be completed. Check the connection and try again.');
    } finally {
      setDownloading(false);
    }
  }

  return (
    <section className="glass-card" data-testid="export-card">
      <h2>Export a backup</h2>
      <p className="text-muted">
        Choose the portfolios to include. The file is downloaded to your device and is never
        stored on the server.
      </p>
      {caps && (
        <p className="text-muted" data-testid="caps-note" style={{ fontSize: '0.8rem' }}>
          {`A bundle is refused above ${caps.max_rows.toLocaleString('en-US')} rows or ` +
            `${caps.max_bytes.toLocaleString('en-US')} bytes.`}
        </p>
      )}
      {portfolios.length === 0 && <p data-testid="no-portfolios">No portfolios to export.</p>}
      <ul style={{ listStyle: 'none', padding: 0, display: 'grid', gap: '8px' }}>
        {portfolios.map((p) => (
          <li key={p.id}>
            <label>
              <input
                type="checkbox"
                aria-label={p.name}
                checked={selected.includes(p.id)}
                onChange={() => toggle(p.id)}
              />{' '}
              {p.name}
            </label>
          </li>
        ))}
      </ul>
      {caps?.allow_price_restore && (
        <label style={{ display: 'block', marginTop: '12px' }}>
          <input
            data-testid="include-prices"
            type="checkbox"
            checked={includePrices}
            onChange={(e) => {
              setExportError(null);
              setIncludePrices(e.target.checked);
            }}
          />{' '}
          Include price history (shared market data, not your data)
        </label>
      )}
      {/* Rendered only with a selection, because an empty selection is a 422 and an
          empty bundle is the one file nobody should ever restore. */}
      {exportHref !== '' ? (
        <a
          data-testid="download-bundle"
          className="btn btn-primary"
          href={exportHref}
          onClick={(event) => {
            if (!isPlainLeftClick(event)) return;
            event.preventDefault();
            void download();
          }}
        >
          {downloading ? 'Downloading…' : 'Download bundle'}
        </a>
      ) : (
        <button data-testid="export-button" className="btn btn-primary" disabled>
          Download bundle
        </button>
      )}
      {exportError && (
        <div
          data-testid="export-error"
          role="alert"
          style={{ marginTop: '12px', color: 'var(--color-danger)' }}
        >
          {exportError}
        </div>
      )}
    </section>
  );
}
