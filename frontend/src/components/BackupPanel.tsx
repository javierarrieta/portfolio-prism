import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from 'react';
import { apiFetch, API_BASE } from '../utils/apiFetch';
import { PlanSideView, isPlanSide, isCounts, countList, type Counts, type PlanSide } from './BackupPlanDetail';

/**
 * Export and restore for the signed-in user's own portfolios: one card per half, in
 * one panel, so neither is looked for in a different place from the other.
 *
 * Export is fetched rather than linked even though it ends as a file, because the
 * same route that answers with the bundle answers `422` with the reason the
 * selection was refused — and a refusal carries no `Content-Disposition`, so a link
 * the browser navigates leaves the user standing on a JSON error body instead of
 * leaving the reason in the panel. The href stays the real route, so ctrl+click and
 * "save link as" still work the browser's way. That is a deliberate hole rather than
 * an oversight: a modified click is the browser's own save-as, so it is left alone,
 * and a modified click on a refused selection therefore does land the user on the raw
 * JSON body this paragraph cites. Plain clicks — the ones carrying the intent to
 * download — are the ones kept inside the panel.
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

/** What `/backup/preview?mode=` answers: what the bundle would write and — for
 * `replace` — what it would delete. Each half is a `PlanSide`: the counts the server
 * made plus the bounded listing under them. `PlanSide`, and `Counts` for the restore
 * outcome below, come from `BackupPlanDetail` along with `countList` and the readers
 * below — the panel spells the plan sentence and checks an untrusted body with the
 * same functions the detail view does, and two copies of either is how they start
 * disagreeing.
 *
 * The plan is read from the server and never recomputed here: counts a UI works out
 * itself are counts of what the UI guessed the bundle holds, and the guess is what the
 * user reads as a promise. */
interface Plan {
  schema_version: number;
  mode: string;
  delete: PlanSide;
  insert: PlanSide;
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
 * bundle that failed validation — the list of what is wrong with it, plus the server's
 * own flag saying that list arrived already truncated. */
interface FailureBody {
  error?: string;
  errors?: { path: string; reason: string }[];
  /** Set when the server stopped listing errors at its cap, so the array in hand is not
   * the whole answer. `collect()` in `backend_rust/src/backup/mod.rs` truncates and
   * sets it; nothing here can observe that truncation from the array alone. */
  more_errors?: boolean;
}

/** One sentence out of a refusal. Naming the first few validation failures is what
 * makes a bad bundle fixable rather than mysterious. The byte and row caps arrive as
 * a whole-bundle `error` with no `errors` at all, and render as that sentence alone. */
function describeFailure(body: FailureBody): string {
  const head = body.error ?? 'The server refused the request.';
  if (!body.errors?.length) return head;
  const named = body.errors.slice(0, 5).map((e) => (e.path ? `${e.path}: ${e.reason}` : e.reason));
  // What was left off is counted two different ways, and only one of them is arithmetic
  // this file may do. `errors.length - named.length` is what *this renderer* dropped;
  // `more_errors` is the server saying it dropped some before this body was ever built,
  // and how many it dropped is not knowable from here. So the flag turns the count into
  // a floor — "at least" — rather than letting the arithmetic state a total the server
  // never sent, which is how a 25-error bundle used to be reported as "(and 5 more)".
  const heldBack = body.errors.length - named.length;
  const more = body.more_errors
    ? ` (and at least ${heldBack + 1} more)`
    : heldBack > 0
      ? ` (and ${heldBack} more)`
      : '';
  return `${head} ${named.join('; ')}${more}`;
}

/** The refusal body, or a sentence saying there was no readable one. A gateway's
 * HTML page is not the backend talking, and rendering it is worse than admitting it. */
async function readFailure(res: Response): Promise<FailureBody> {
  if (res.status === 413) {
    // Nothing reached the backend, so there is no JSON reason to read and the cap the
    // panel advertises is not the limit that answered. Naming which limit spoke is the
    // difference between a retry, a smaller file, and a dead end.
    return { error: 'The upload was refused as too large (413) before the API read any of it. '
      + 'Something between this app and the API imposes a smaller limit than the cap this server advertises.' };
  }
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

/** Why this file cannot be sent, or `null` when it can.
 *
 * The number is the server's own, read from `/backup/capabilities`; refusing here is
 * not a shortcut around the server, which still refuses. It is the difference between
 * a sentence naming the file and its size and whatever a gateway in front of the API
 * answers with for a body over its limit — an HTML page carrying no reason at all.
 * `null` when capabilities never loaded: an unknown cap must not block an upload that
 * might be fine, and `readFailure` covers that case with what the server actually said. */
function oversizeReason(file: File, caps: Capabilities | null): string | null {
  if (!caps || file.size <= caps.max_bytes) return null;
  return `That file is ${file.size.toLocaleString('en-US')} bytes, above the ` +
    `${caps.max_bytes.toLocaleString('en-US')}-byte cap this server advertises, so it ` +
    'would be refused before anything is read.';
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
 * one disagreement a user cannot see.
 *
 * Each half is checked as a whole `PlanSide`, not as its counts: the listing beside
 * them is rendered too, so a body that counts correctly and carries `shown: null` is
 * as unrenderable as one that counts wrongly, and `isPlanSide` is the function that
 * knows what renderable means. A body in the old flat shape — three bare counts per
 * side, which is what this panel read before the plan grew its listings — fails here
 * on purpose. Accepting it would mean reading `side.counts` off `undefined`. */
function asPlan(body: unknown, askedFor: RestoreMode): Plan | null {
  const p = body as Partial<Plan> | null;
  if (!p || typeof p !== 'object') return null;
  if (p.mode !== askedFor) return null;
  if (!isPlanSide(p.delete) || !isPlanSide(p.insert)) return null;
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
  // What the database says it holds, read back once a write has committed, and whether
  // that read failed. Both are decoration beside `restoreResult` and nothing else: they
  // gate no button and no control, because a committed write must never be made to look
  // like a failure by a second read that did not land. Cleared with the result line they
  // belong to, and by any fresh preview: a readback describes the write it followed, never
  // the plan that is on screen afterwards — and one that lands after a newer question went
  // out never lands at all, because `runRestore` guards it the way previews are guarded.
  const [owned, setOwned] = useState<PlanSide | null>(null);
  const [ownedFailed, setOwnedFailed] = useState(false);

  // Which preview is the current one. Two uploads in a row leave two responses in
  // flight and they may land in any order, so the response that arrives last is not
  // necessarily the one for the file on screen; every write below is guarded by this
  // counter, and a superseded answer is dropped rather than displayed.
  const previewSeq = useRef(0);

  // The one place that asks for a plan. Both callers are user actions — choosing a
  // file, and choosing a mode — so the request goes out when the user changes
  // something rather than from an effect chasing that change after the fact.
  async function runPreview(target: File, forMode: RestoreMode) {
    // Checked before the sequence advances, so an oversized file never becomes the
    // current request: there is no in-flight answer to retire and no flag to clear.
    const oversize = oversizeReason(target, caps);
    if (oversize) {
      setPlan(null);
      setRestoreError(oversize);
      return;
    }
    const seq = ++previewSeq.current;
    setPreviewing(true);
    setRestoreError(null);
    // A preview is a fresh question about what the next write would do, and the readback
    // beside the last write's result line is the answer to the *previous* question. Left
    // up, the card reads "Will delete 2 portfolios…" directly above "Your portfolios now
    // 1 portfolio…" — two present-tense answers about the same account, and the stale one
    // is the one that looks like current state. It goes with the plan it was read against,
    // exactly as the confirmation goes with the plan it was typed against.
    setOwned(null);
    setOwnedFailed(false);
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

  // A write retires every preview that has not answered yet, at both ends of the write:
  // before it starts, and again when it finishes. Bumping the sequence is what makes an
  // in-flight answer fail the guard in `runPreview` instead of being displayed; the flag
  // has to come down here too, because that answer's `finally` clears it only for the
  // sequence number that is current, and this has just made it not current.
  const retireInFlightPreview = () => {
    previewSeq.current += 1;
    setPreviewing(false);
  };

  // Take the card apart at the end of a write: no plan and no words, so there is
  // nothing left to press. Both callers are the end of a restore — one that wrote, and
  // one whose outcome cannot be read.
  const disarmRestore = () => {
    // A preview still on its way when the write finished is retired with the plan it
    // was drawn against. Left alone, it lands after `disarmRestore` has cleared the
    // card, passes its own sequence guard, and puts a plan back — re-arming a second
    // write that nobody asked for. This is the half the guard at the top of `runRestore`
    // cannot reach: a preview asked for *during* the write carries a newer sequence
    // number than that one, so it is this bump that retires it.
    retireInFlightPreview();
    setPlan(null);
    setConfirm('');
    setConfirmFor(null);
  };

  async function runRestore() {
    // No plan, no restore: the gate is that the user has seen this file's counts, and
    // there is nothing to have seen until a preview has answered.
    if (!file || !plan) return;
    // A restore cannot be reached without a preview of this same file, and the preview
    // already refused an oversized one, so this is the same check standing where the
    // write happens rather than a path anyone is expected to need.
    const oversize = oversizeReason(file, caps);
    if (oversize) {
      setRestoreError(oversize);
      return;
    }
    retireInFlightPreview();
    setRestoring(true);
    setRestoreError(null);
    setRestoreResult(null);
    setOwned(null);
    setOwnedFailed(false);
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
      // The write is committed and the panel is now describing it, so the readback is
      // decoration: if it fails, the counts the restore itself returned stay on screen
      // with one honest line beside them. Nothing here may make a committed write look
      // like a failure — that is how the same bundle gets restored twice.
      //
      // What makes an out-of-order answer safe is the generation guard below, not the
      // `restoring` bracket this request sits inside. The bracket still matters — it is
      // what disables the file input and the mode radios until the read lands, so today
      // it is what keeps two readbacks from overlapping at all — but a safety property
      // that travelled with a disabled control would vanish the moment someone moved this
      // request below the `finally` to shave a frame off the busy state, and the card
      // would then state what the database held *before* the newer write as though it
      // were the answer about what it holds now. That is the one sentence here that is
      // not allowed to be wrong about the present tense, so the guard retires a stale
      // answer whatever the busy state happens to allow.
      //
      // The guard is the counter that already decides which preview may display, and for
      // the same reason: Preview is gated only on `previewing`, so it can be pressed in
      // the window between this write committing and this read landing. That preview is a
      // fresh question, and `runPreview` has just dropped the readback as the answer to
      // the old one — so an answer from a retired generation lands on top of the new
      // plan and reprints the account as it stood before the write. It is dropped whole:
      // no counts, no failure line, no change to what is on screen.
      const readback = previewSeq.current;
      try {
        const back = await apiFetch(`${API_BASE}/backup/owned`);
        const body = back.ok ? (await back.json()) as unknown : null;
        if (readback !== previewSeq.current) return;
        if (isPlanSide(body)) setOwned(body);
        else setOwnedFailed(true);
      } catch {
        if (readback !== previewSeq.current) return;
        setOwnedFailed(true);
      }
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
              setOwned(null);
              setOwnedFailed(false);
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
            {/* One `PlanSideView` per side, so the sentence the summary used to be is
                the first line of the detail rather than a second copy of it. The
                listing under it is the server's own bounded one — this renders what
                arrived in the preview response and nothing more, so a bigger plan
                costs a bigger response and never a re-read of the bundle. */}
            <div data-testid="preview-delete">
              {plan.mode === 'replace'
                ? <PlanSideView title="Will delete" side={plan.delete} />
                : <p>Will delete nothing.</p>}
            </div>
            <div data-testid="preview-insert">
              <PlanSideView title="Will insert" side={plan.insert} />
            </div>
            {/* The card never sends `apply_prices`, so a bundle carrying price history
                restores without it. That is a decision the user should not have to guess
                at, and `prices` is the server's own count of what is being left out. It
                is stated as the operator flow it is: this panel will never grow the
                control, so "will be left alone" read as a feature it was one checkbox
                short of. */}
            {plan.prices > 0 && (
              <p data-testid="preview-prices" className="text-muted">
                {`This bundle carries ${plan.prices} price row${plan.prices === 1 ? '' : 's'}. `
                  + 'Price history is shared market data and is restored by an operator, not from this panel.'}
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
          disabled={!plan || (mode === 'replace' && !replaceConfirmed) || restoring || previewing}
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
        {restoreResult && owned && (
          <div data-testid="restore-owned" style={{ marginTop: '12px' }}>
            <PlanSideView title="Your portfolios now" side={owned} />
          </div>
        )}
        {/* Mutually exclusive with the line above, and success wins: `owned` and
            `ownedFailed` are set in one `if`/`else` today, but two independent conditions
            would print "Your portfolios now …" and "could not be read back" about the
            same write the moment that ever stops holding. */}
        {restoreResult && !owned && ownedFailed && (
          <p data-testid="restore-owned-failed" className="text-muted" style={{ marginTop: '8px' }}>
            The restore succeeded, but your data could not be read back to confirm it. Reload to see what is there.
          </p>
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
          the backend, and not a bug to fix.

          The reason is this panel, not the backend. The restore half never sends
          `apply_prices`, so a price section in a bundle this panel restores is written
          by nobody: `restore::apply` writes prices only when the bundle carries them
          *and* `apply_prices` is set — the `PricesDisabled` guard in
          backend_rust/src/backup/restore.rs is reached on that same conjunction — and a
          bundle with a price section and no `apply_prices` restores with a plain `200`
          and the prices skipped, which is what the plan above says out loud. So the
          toggle would offer rows the user carries around in a file this panel can never
          apply, in the one format whose whole promise is that a bundle restores.

          Do not "correct" this to match what the export route checks: the export route
          answers `include_prices` whether or not the flag is on, and matching it would
          put a price section in a bundle this panel cannot apply. The gate opens when
          this panel grows an `apply_prices` control, not before. */}
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
