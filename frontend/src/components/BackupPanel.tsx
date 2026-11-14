import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from 'react';
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
 * hold a plan. So it sits beside this one, in this panel, reading the same
 * capabilities — two halves of one file, in one place.
 */

interface PortfolioOption {
  id: number;
  name: string;
}

/** Rows counted by the server, in the three tables a bundle touches. One type for
 * both what a plan counts and what a restore reports having written, because the
 * server uses one shape for both. */
interface Counts {
  portfolios: number;
  assets: number;
  transactions: number;
}

/** What `/backup/preview?mode=` answers: what the bundle would write and — for
 * `replace` — what it would delete. Read from the server and never recomputed here:
 * counts a UI works out itself are counts of what the UI guessed the bundle holds,
 * and the guess is what the user reads as a promise. */
interface Plan {
  schema_version: number;
  mode: string;
  delete: Counts;
  insert: Counts;
  prices: number;
  warnings: string[];
}

type RestoreMode = 'copy' | 'replace';

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

/** The capabilities body, or `null` when it is not a capabilities body.
 *
 * A bare `as Capabilities` on whatever a 200 happened to carry is not a check: a
 * proxy's envelope, a truncated body, or a renamed field reaches
 * `caps.max_rows.toLocaleString()` and throws during render, and there is no error
 * boundary anywhere in this app, so the whole page would go blank instead of showing
 * a panel that simply cannot state the limits. Refusing the shape keeps the panel
 * usable — the export needs no capability — which is the same rule the capabilities
 * failure path already follows. */
function asCapabilities(body: unknown): Capabilities | null {
  const c = body as Partial<Capabilities> | null;
  if (!c || typeof c !== 'object') return null;
  if (!Array.isArray(c.schema_versions)) return null;
  if (typeof c.allow_price_restore !== 'boolean') return null;
  if (!Number.isFinite(c.max_bytes) || !Number.isFinite(c.max_rows)) return null;
  return c as Capabilities;
}

/** One set of the three counts a plan carries, read off an untrusted body. */
function isCounts(value: unknown): value is Counts {
  const c = value as Partial<Counts> | null;
  return !!c && typeof c === 'object'
    && Number.isFinite(c.portfolios) && Number.isFinite(c.assets) && Number.isFinite(c.transactions);
}

/** The preview body, or `null` when it is not a plan for the mode that was asked for.
 *
 * The reason `asCapabilities` gives for existing applies harder here: a `200` that is
 * valid JSON but not a plan — a proxy envelope, a renamed field, a truncated body —
 * reaches `plan.delete.portfolios` and throws during render, and with no error
 * boundary anywhere in this app a thrown render is a blank page at the exact moment
 * the user is deciding whether to destroy their data. A body that is not a plan is
 * refused, which leaves the card standing with nothing to restore against.
 *
 * The echoed `mode` has to be the mode that was asked for, because the two halves of
 * one operation are read from two places: the delete sentence is rendered from the
 * plan and the write is sent from the component's own mode. If they ever disagree the
 * card would be describing one operation while performing the other, and that is the
 * one disagreement a user cannot see. */
function asPlan(body: unknown, askedFor: RestoreMode): Plan | null {
  const p = body as Partial<Plan> | null;
  if (!p || typeof p !== 'object') return null;
  if (p.mode !== askedFor) return null;
  if (!isCounts(p.delete) || !isCounts(p.insert)) return null;
  if (!Number.isFinite(p.prices)) return null;
  if (!Array.isArray(p.warnings) || !p.warnings.every((w) => typeof w === 'string')) return null;
  return p as Plan;
}

/** What the card says when a restore went out and its outcome cannot be read — no
 * answer, or an answer that is not the outcome. It cannot promise nothing was written,
 * so it says what is true and where to look instead. */
const UNCONFIRMED_RESTORE = 'The restore could not be confirmed, so it cannot be tried again from this page. Reload the page to see what it wrote.';

/** The bundle's text.
 *
 * `FileReader` rather than `file.text()`: the latter is missing from jsdom's `File`,
 * so a component that called it could not be tested at all — the export download
 * test reads a `Blob` the same way for the same reason. It is also a read that can
 * fail, so it rejects rather than handing back an empty string and letting an empty
 * upload go to the server to be refused on someone else's behalf. */
function readFileText(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error('the file could not be read'));
    reader.readAsText(file);
  });
}

/** `{n} {noun}` with the plural spelled out, so the plan reads "1 portfolio, 2
 * assets, 3 transactions". The counts are the server's; only the English is here. */
function countList(counts: Counts): string {
  const named = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;
  return [
    named(counts.portfolios, 'portfolio'),
    named(counts.assets, 'asset'),
    named(counts.transactions, 'transaction'),
  ].join(', ');
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
      .then((body) => {
        const read = asCapabilities(body);
        if (!cancelled && read) setCaps(read);
      })
      .catch(() => { /* nothing to switch off: see the comment above */ });
    return () => { cancelled = true; };
  }, []);

  // The restore half. Everything below exists to hold one rule: the plan on screen is
  // the plan for the file and the mode on screen, or there is no plan at all.
  const [file, setFile] = useState<File | null>(null);
  const [mode, setMode] = useState<RestoreMode>('copy');
  const [confirm, setConfirm] = useState('');
  // Which plan the words were typed against. A plan that gets replaced — by a retry, a
  // mode switch, a new file — has to be confirmed again, because the counts on screen
  // are what the words were read against and a fresh answer can count more to delete
  // than the one that was.
  const [confirmFor, setConfirmFor] = useState<Plan | null>(null);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [restoreError, setRestoreError] = useState<string | null>(null);
  const [restoreResult, setRestoreResult] = useState<{ inserted: Counts } | null>(null);

  // Which preview is the current one. Two uploads in a row leave two responses in
  // flight and they may land in any order, so the response that arrives last is not
  // necessarily the one for the file on screen; every write below is guarded by this
  // counter, and a superseded answer is dropped rather than displayed.
  const previewSeq = useRef(0);

  // The one place that asks for a plan. Both callers are user actions — choosing a
  // file, and choosing a mode — so the request goes out when the user changes
  // something rather than from an effect chasing that change after the fact.
  async function runPreview(target: File, forMode: RestoreMode) {
    const seq = ++previewSeq.current;
    setPreviewing(true);
    setRestoreError(null);
    // The words belong to the plan on screen, so they go with the plan this request is
    // about to replace. Retrying a preview is not a no-op: the second answer can count
    // more to delete than the first, and a confirmation carried across it is one the
    // user never re-read.
    if (plan) {
      setConfirm('');
      setConfirmFor(null);
    }
    try {
      const res = await apiFetch(`${API_BASE}/backup/preview?mode=${forMode}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: await readFileText(target),
      });
      if (seq !== previewSeq.current) return;
      if (!res.ok) {
        // The refusal is the answer to *this* file, so there is no plan to offer and
        // the server's reason takes the place of one.
        setPlan(null);
        setRestoreError(describeFailure(await readFailure(res)));
        return;
      }
      const body = await res.json();
      if (seq !== previewSeq.current) return;
      const read = asPlan(body, forMode);
      if (!read) {
        setPlan(null);
        setRestoreError('The preview answer was not a plan this app could read, so there is nothing to restore against.');
        return;
      }
      setPlan(read);
    } catch {
      if (seq !== previewSeq.current) return;
      setPlan(null);
      setRestoreError('The bundle could not be read or sent. Check the file and the connection and try again.');
    } finally {
      // Only the current preview may clear the flag, or a superseded response would
      // stop the spinner while the newer request is still running.
      if (seq === previewSeq.current) setPreviewing(false);
    }
  }

  // Changing the mode changes what will be deleted, so the plan goes with it and a
  // fresh one is fetched for the mode now chosen — relabelling the old counts is the
  // misreading this costs a request to avoid. The typed confirmation goes with it
  // too: it was typed about the plan being thrown away, and one that survived the
  // round trip is one the user never re-read.
  const chooseMode = (next: RestoreMode) => {
    setMode(next);
    setPlan(null);
    setConfirm('');
    setConfirmFor(null);
    if (file) void runPreview(file, next);
  };

  // Take the card apart at the end of a write: no plan and no words, so there is
  // nothing left to press. Both callers are the end of a restore — one that wrote, and
  // one whose outcome cannot be read.
  const disarmRestore = () => {
    setPlan(null);
    setConfirm('');
    setConfirmFor(null);
  };

  async function runRestore() {
    // No plan, no restore: the gate is that the user has seen this file's counts, and
    // there is nothing to have seen until a preview has answered.
    if (!file || !plan) return;
    setRestoring(true);
    setRestoreError(null);
    setRestoreResult(null);
    try {
      const res = await apiFetch(`${API_BASE}/backup/restore?mode=${mode}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: await readFileText(file),
      });
      if (!res.ok) {
        // The server answered, and that is the difference between this branch and the
        // one below: a refusal is something the route reported about its own work, and
        // every refusal it can give happens before or rolled back the transaction, so
        // the counts on screen are still the counts and the same restore is there to
        // retry — which is exactly what a busy database asks for, in its own words.
        setRestoreError(describeFailure(await readFailure(res)));
        return;
      }
      const written = ((await res.json()) as { inserted?: unknown } | null)?.inserted;
      if (!isCounts(written)) {
        // A 200 whose body is not the outcome is an answer that cannot be read, which
        // is the same unknown as no answer at all.
        disarmRestore();
        setRestoreError(UNCONFIRMED_RESTORE);
        return;
      }
      // The write is done, and the plan and the words are over: a second restore is not
      // a retry of the first, it is a second write. In `replace` that would delete
      // everything again on the strength of a word typed a minute ago, and in `copy` it
      // would duplicate the portfolios with nothing in the way. The result line stays as
      // the record, and Preview arms the card again for anyone who means to go on.
      disarmRestore();
      setRestoreResult({ inserted: written });
    } catch {
      // No answer. The write may have committed, may have rolled back, or may never
      // have arrived, and nothing this client can observe tells it which, so no
      // sentence here is allowed to say "nothing was written" — that promise is how
      // someone restores the same bundle twice. The plan goes with the promise: with
      // no plan there is no restore to press, because a second write is not a retry of
      // a failed one, it is a second write. What is actually there is one reload away.
      disarmRestore();
      setRestoreError(UNCONFIRMED_RESTORE);
    } finally {
      setRestoring(false);
    }
  }

  // The words only count while they are bound to the plan on screen. `plan` itself is
  // required by the button too, so words typed before any plan existed bind to nothing.
  const replaceConfirmed = confirm === 'REPLACE' && confirmFor === plan;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '24px' }}>
      <ExportCard portfolios={portfolios} caps={caps} />

      <section className="glass-card" data-testid="restore-card">
        <h2>Restore a backup</h2>
        <p className="text-muted">
          Choose a bundle file, preview what it changes, then restore. Nothing is written until
          you restore, and a restore the server answers with a refusal writes nothing: it is
          applied in one transaction or not at all.
        </p>

        <label style={{ display: 'block', marginBottom: '12px' }}>
          Bundle file
          <input
            data-testid="bundle-file"
            className="form-control"
            type="file"
            accept="application/json,.json"
            disabled={restoring}
            onChange={(e) => {
              // Everything the old file and the old mode were true about goes with
              // them, including the confirmation: a plan beside a different file is
              // the one thing this card exists to make impossible. The new file is
              // previewed straight away for the same reason — the counts on screen
              // have to be the counts of what is actually chosen — and the Preview
              // button below stays as the explicit retry of the same call.
              const chosen = e.target.files?.[0] ?? null;
              setFile(chosen);
              setPlan(null);
              setRestoreError(null);
              setRestoreResult(null);
              setConfirm('');
              setConfirmFor(null);
              if (chosen) void runPreview(chosen, mode);
            }}
          />
        </label>

        <fieldset style={{ border: 0, padding: 0, marginBottom: '12px' }}>
          <legend>Mode</legend>
          <label>
            <input type="radio" name="mode" value="copy" checked={mode === 'copy'}
              disabled={restoring} onChange={() => chooseMode('copy')} />
            {' '}New copy — add these portfolios alongside what you already have
          </label>
          <br />
          <label>
            <input type="radio" name="mode" value="replace" checked={mode === 'replace'}
              disabled={restoring} onChange={() => chooseMode('replace')} />
            {' '}Replace — delete every portfolio you own, then restore this bundle
          </label>
        </fieldset>

        {mode === 'replace' && (
          <label style={{ display: 'block', marginBottom: '12px' }}>
            Type <strong>REPLACE</strong> to confirm that everything you own will be deleted
            <input
              data-testid="replace-confirm"
              className="form-control"
              value={confirm}
              onChange={(e) => {
                // Bound to the plan on screen at the moment the words were typed, not
                // to whatever plan happens to be on screen when the button is read.
                setConfirm(e.target.value);
                setConfirmFor(plan);
              }}
            />
          </label>
        )}

        {file && (
          <button className="btn btn-secondary" data-testid="preview-button"
            onClick={() => void runPreview(file, mode)} disabled={previewing}>
            {previewing ? 'Previewing…' : 'Preview'}
          </button>
        )}

        {plan && (
          <div data-testid="preview-summary" role="status" style={{ marginTop: '12px' }}>
            <p>
              {plan.mode === 'replace'
                ? `Will delete ${countList(plan.delete)}.`
                : 'Will delete nothing.'}
            </p>
            <p>{`Will insert ${countList(plan.insert)}.`}</p>
            {/* The card never sends `apply_prices`, so a bundle carrying price history
                restores without it. That is a decision the user should not have to guess
                at, and `prices` is the server's own count of what is being left out. */}
            {plan.prices > 0 && (
              <p data-testid="preview-prices" className="text-muted">
                {`${plan.prices} price row${plan.prices === 1 ? '' : 's'} in this bundle will be left alone — this panel does not restore price history.`}
              </p>
            )}
            {plan.warnings.map((w) => (
              <p key={w} data-testid="preview-warning" style={{ color: 'var(--color-warning)' }}>{w}</p>
            ))}
          </div>
        )}

        <button
          className="btn btn-primary"
          data-testid="restore-button"
          disabled={!plan || (mode === 'replace' && !replaceConfirmed) || restoring}
          onClick={() => void runRestore()}
        >
          {restoring ? 'Restoring…' : 'Restore'}
        </button>

        {restoreError && (
          <div data-testid="restore-error" role="alert" style={{ marginTop: '12px', color: 'var(--color-danger)' }}>
            {restoreError}
          </div>
        )}
        {restoreResult && (
          <div data-testid="restore-result" role="status" style={{ marginTop: '12px' }}>
            {`Restored ${countList(restoreResult.inserted)}.`}
          </div>
        )}
      </section>
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
      const blob = await res.blob();
      // A 200 is not a backup. A gateway that swallowed the upstream body, or a route
      // that answered before it had anything to say, arrives as an empty 200, and the
      // one file a user must never be handed is the empty one — it downloads cleanly,
      // sits in the folder looking like a backup, and is the backup they reach for
      // after the disk fails. Refusing it in the panel is the whole point.
      if (blob.size === 0) {
        setExportError('The server answered with an empty file, so nothing was downloaded. Try again.');
        return;
      }
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = filenameFrom(res.headers.get('Content-Disposition'));
      document.body.appendChild(link);
      link.click();
      link.remove();
      // The URL has to outlive the click: the browser opens the blob when the save
      // begins, and that is not until this task has yielded. Revoking in the same tick
      // is the race that truncates a file, so it goes on the next macrotask.
      setTimeout(() => URL.revokeObjectURL(url), 0);
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
      {/* Gated on `allow_price_restore`, which is the server's *restore* permission and
          is never consulted by the export route — so this is deliberately stricter than
          the backend, and not a bug to fix. A bundle carrying a price section is a
          bundle that cannot be applied while the flag is off: `restore::apply` refuses
          it (`PricesDisabled`, backend_rust/src/backup/restore.rs) before it opens a
          transaction. Offering the toggle would sell the user a file they can never
          restore, which is worse than not offering it. Do not "correct" this to match
          what the export route checks. */}
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
        <button
          data-testid="export-button"
          className="btn btn-primary"
          disabled
          title="Select at least one portfolio"
        >
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
