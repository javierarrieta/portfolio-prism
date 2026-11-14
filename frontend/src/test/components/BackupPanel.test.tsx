import { render, screen, waitFor, within, fireEvent, act } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import BackupPanel from '../../components/BackupPanel'

const portfolios = [
  { id: 1, name: 'Main' },
  { id: 2, name: 'IRA' },
]

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

const CAPS = { schema_versions: [1], allow_price_restore: false, max_bytes: 8388608, max_rows: 50000 }

/** A plan body. The echoed `mode` is the mode that was asked for, because the card now
 * refuses a plan whose echo disagrees with the request — and a mock that always said
 * `copy` would hide that check, and would have the summary reading "Will delete
 * nothing." at the moment the write sends `mode=replace`. */
function planBody(over: {
  mode?: string
  insert?: { portfolios: number; assets: number; transactions: number }
  delete?: { portfolios: number; assets: number; transactions: number }
  prices?: number
  warnings?: string[]
} = {}) {
  const mode = over.mode ?? 'copy'
  return {
    schema_version: 1,
    mode,
    delete: over.delete ?? (mode === 'replace'
      ? { portfolios: 2, assets: 4, transactions: 6 }
      : { portfolios: 0, assets: 0, transactions: 0 }),
    insert: over.insert ?? { portfolios: 1, assets: 2, transactions: 3 },
    prices: over.prices ?? 0,
    warnings: over.warnings ?? [],
  }
}

let fetchMock: ReturnType<typeof vi.fn>
beforeEach(() => {
  fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
    if (url.includes('/backup/preview')) {
      return jsonResponse(planBody({ mode: url.includes('mode=replace') ? 'replace' : 'copy' }))
    }
    return jsonResponse({}, 404)
  })
  vi.stubGlobal('fetch', fetchMock)
})
afterEach(() => vi.unstubAllGlobals())

describe('export card', () => {
  it('offers one checkbox per portfolio and none selected by default', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await waitFor(() => expect(screen.getByTestId('export-button')).toBeDisabled())
    expect(screen.getByRole('checkbox', { name: /Main/i })).not.toBeChecked()
    expect(screen.getByRole('checkbox', { name: /IRA/i })).not.toBeChecked()
  })

  it('says why the download is disabled', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    // A disabled control that gives no reason reads as a broken one, and the fix is
    // one tick away.
    expect(screen.getByTestId('export-button'))
      .toHaveAttribute('title', 'Select at least one portfolio')
  })

  it('builds the download href from exactly the selected ids', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.click(screen.getByRole('checkbox', { name: /IRA/i }))
    const link = await screen.findByTestId('download-bundle')
    expect(link).toHaveAttribute('href', '/api/backup/export?portfolio_ids=2')
  })

  it('names every selected id, in selection order', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.click(screen.getByRole('checkbox', { name: /IRA/i }))
    await userEvent.click(screen.getByRole('checkbox', { name: /Main/i }))
    expect(await screen.findByTestId('download-bundle'))
      .toHaveAttribute('href', '/api/backup/export?portfolio_ids=2,1')
  })

  it('hides the download again when the last portfolio is unticked', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    const box = screen.getByRole('checkbox', { name: /Main/i })
    await userEvent.click(box)
    await userEvent.click(box)
    expect(screen.queryByTestId('download-bundle')).toBeNull()
    expect(screen.getByTestId('export-button')).toBeDisabled()
  })

  it('states the limits the server reported rather than ones it invented', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    const note = await screen.findByTestId('caps-note')
    expect(note).toHaveTextContent('50,000 rows')
    expect(note).toHaveTextContent('8,388,608 bytes')
  })

  it('does not offer price history when the server has it off', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    // The note is the proof the capabilities response landed. Asserting the toggle is
    // missing before it arrives would pass on a component that never read the flag.
    await screen.findByTestId('caps-note')
    expect(screen.queryByTestId('include-prices')).toBeNull()
  })

  it('offers price history when the server allows it, and adds it to the href', async () => {
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      if (String(input).includes('/backup/capabilities')) {
        return jsonResponse({ ...CAPS, allow_price_restore: true })
      }
      return jsonResponse({}, 404)
    })
    render(<BackupPanel portfolios={portfolios} />)
    const toggle = await screen.findByTestId('include-prices')
    await userEvent.click(screen.getByRole('checkbox', { name: /Main/i }))
    await userEvent.click(toggle)
    expect(await screen.findByTestId('download-bundle'))
      .toHaveAttribute('href', '/api/backup/export?portfolio_ids=1&include_prices=1')
  })

  it('says there is nothing to export when the user owns no portfolios', () => {
    render(<BackupPanel portfolios={[]} />)
    expect(screen.getByTestId('no-portfolios')).toBeInTheDocument()
    expect(screen.getByTestId('export-button')).toBeDisabled()
  })

  it('keeps the panel usable when the capabilities read fails', async () => {
    let answered = false
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/capabilities')) {
        answered = true
        return new Response('bad gateway', { status: 500 })
      }
      return jsonResponse({}, 404)
    })
    render(<BackupPanel portfolios={portfolios} />)
    // One macrotask after the answer is in, every hop of the component's promise chain
    // has run, so the missing note below is a decision and not a response that had not
    // landed. There is no caps-note to wait for here -- that is the whole point.
    await waitFor(() => expect(answered).toBe(true))
    await new Promise((resolve) => setTimeout(resolve, 0))

    // The limits line is the only thing the read was for. Losing it must not cost the
    // export, which needs no capability to work.
    expect(screen.queryByTestId('caps-note')).toBeNull()
    await userEvent.click(screen.getByRole('checkbox', { name: /IRA/i }))
    expect(await screen.findByTestId('download-bundle'))
      .toHaveAttribute('href', '/api/backup/export?portfolio_ids=2')
  })

  it('keeps the panel usable when a 200 carries a body that is not capabilities', async () => {
    // A proxy envelope, or a renamed field: `max_rows` here is a string, and
    // `max_bytes` is missing altogether. Cast onto `Capabilities` and the limits line
    // throws during render -- and with no error boundary in this app, a thrown render
    // is a blank page, not a panel with one line missing.
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/capabilities')) return jsonResponse({ ok: true, max_rows: '50000' })
      return jsonResponse({}, 404)
    })
    render(<BackupPanel portfolios={portfolios} />)

    expect(await screen.findByTestId('export-card')).toBeInTheDocument()
    expect(screen.queryByTestId('caps-note')).toBeNull()
    await userEvent.click(screen.getByRole('checkbox', { name: /IRA/i }))
    expect(await screen.findByTestId('download-bundle'))
      .toHaveAttribute('href', '/api/backup/export?portfolio_ids=2')
  })
})

describe('export download', () => {
  const realCreateObjectURL = URL.createObjectURL
  const realRevokeObjectURL = URL.revokeObjectURL
  /** Deliberately not a string that `JSON.stringify(JSON.parse(x))` reproduces: the
   * line breaks and the indent are dropped by a re-serialise, and `1e999` parses to
   * `Infinity` and comes back out as `null`. The read-back below compares the bytes
   * that were handed to `createObjectURL`, so a download that parsed the bundle and
   * wrote it again fails here rather than passing. */
  const BUNDLE_BODY =
    '{\n  "format": "portfolio-prism-backup",\n  "schema_version": 1,\n  "assets": [{ "symbol": "Österreichische", "weight": 1e999 }]\n}'

  let created: Blob[]
  let clicked: HTMLAnchorElement[]
  /** What happened to the object URL, in the order it happened. */
  let events: string[]
  let createObjectURL: Mock<(blob: Blob) => string>
  let revokeObjectURL: Mock<() => void>

  /** Points `/backup/export` at `response`; everything else answers as the card expects. */
  function exportRespondsWith(response: Response) {
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
      if (url.includes('/backup/export')) return response
      return jsonResponse({}, 404)
    })
  }

  beforeEach(() => {
    created = []
    clicked = []
    events = []
    createObjectURL = vi.fn((blob: Blob) => {
      created.push(blob)
      events.push('createObjectURL')
      return 'blob:downloaded'
    })
    revokeObjectURL = vi.fn(() => { events.push('revoke') })
    URL.createObjectURL = createObjectURL
    URL.revokeObjectURL = revokeObjectURL
    // jsdom has no download manager. Recording the click instead of running it is
    // what makes the generated anchor, and the name it downloads under, observable.
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(
      function capture(this: HTMLAnchorElement) {
        clicked.push(this)
        events.push('click')
        // Registered from inside the click the component makes, so it enters the
        // timer queue ahead of anything the component schedules once `click()` has
        // returned. A revoke left in the click's own task is recorded before this
        // probe fires; one deferred to the next task is recorded after it.
        setTimeout(() => { events.push('click-task-ended') }, 0)
      },
    )
  })
  afterEach(() => {
    URL.createObjectURL = realCreateObjectURL
    URL.revokeObjectURL = realRevokeObjectURL
    vi.restoreAllMocks()
  })

  /** jsdom's `Blob` has no `text()`; its `FileReader` does the same job. */
  function readBlob(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader()
      reader.onload = () => resolve(String(reader.result))
      reader.onerror = () => reject(reader.error)
      reader.readAsText(blob)
    })
  }

  /** Ticks one portfolio and clicks the download. Each test then waits for the
   * outcome it is about: a recorded click for a download, an error for a refusal. */
  async function tickAndClickDownload() {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.click(screen.getByRole('checkbox', { name: /IRA/i }))
    await userEvent.click(screen.getByTestId('download-bundle'))
  }

  it('writes the bytes the server sent into a file named the way the server named it', async () => {
    exportRespondsWith(new Response(BUNDLE_BODY, {
      status: 200,
      headers: {
        'Content-Type': 'application/json',
        'Content-Disposition': 'attachment; filename="portfolio-prism-backup-2026-11-13.json"',
      },
    }))
    await tickAndClickDownload()
    await waitFor(() => expect(clicked).toHaveLength(1))

    expect(createObjectURL).toHaveBeenCalledTimes(1)
    // The blob carries the server's bytes and nothing else. The fixture above is what
    // makes this worth asserting: a component that parsed the body and downloaded
    // `JSON.stringify(JSON.parse(body))` would produce a different string here.
    expect(await readBlob(created[0])).toBe(BUNDLE_BODY)
    expect(clicked[0].getAttribute('download')).toBe('portfolio-prism-backup-2026-11-13.json')
    expect(clicked[0].getAttribute('href')).toBe('blob:downloaded')
    // The revoke is deferred out of the click's task, so it is waited for rather than
    // asserted in this tick -- and the order is what matters, not just the call.
    await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledWith('blob:downloaded'))
    // The URL was still live when the click ran, and it went only after that click's
    // task had ended: revoking before the click, or in the same tick after it, both
    // cut the file off in a real browser.
    expect(events).toEqual(['createObjectURL', 'click', 'click-task-ended', 'revoke'])
  })

  it('falls back to its own name when the server named nothing', async () => {
    exportRespondsWith(new Response(BUNDLE_BODY, {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    }))
    await tickAndClickDownload()
    await waitFor(() => expect(clicked).toHaveLength(1))
    // The route does name its file. This is the answer to an answer that named
    // nothing, and it has to be a plausible file name rather than no name at all --
    // an unnamed download lands as `download`, which is the file nobody can identify
    // later in a folder of them.
    expect(clicked[0].getAttribute('download')).toBe('portfolio-prism-backup.json')
  })

  it('refuses a 200 that carried no bytes instead of handing over an empty backup', async () => {
    exportRespondsWith(new Response('', {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    }))
    await tickAndClickDownload()

    await waitFor(() => expect(screen.getByTestId('export-error'))
      .toHaveTextContent('empty file'))
    // The failure this guards is the quiet one: a zero-byte file downloads cleanly,
    // looks like a backup in the folder, and is the backup that is there when it is
    // needed. Nothing reaches the disk, and the selection stays ticked to retry.
    expect(createObjectURL).not.toHaveBeenCalled()
    expect(clicked).toHaveLength(0)
    expect(screen.getByTestId('download-bundle')).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: /IRA/i })).toBeChecked()
  })

  it('names the file when the server named it without quotes', async () => {
    exportRespondsWith(new Response(BUNDLE_BODY, {
      status: 200,
      headers: { 'Content-Disposition': 'attachment; filename=backup.json' },
    }))
    await tickAndClickDownload()
    await waitFor(() => expect(clicked).toHaveLength(1))
    expect(clicked[0].getAttribute('download')).toBe('backup.json')
  })

  it('renders the server sentence when the selection is over a cap', async () => {
    exportRespondsWith(jsonResponse({
      error: 'that selection is 60000 rows, over the 50000 row limit; export fewer portfolios',
    }, 422))
    await tickAndClickDownload()

    const card = screen.getByTestId('export-card')
    await waitFor(() => expect(within(card).getByTestId('export-error'))
      .toHaveTextContent('over the 50000 row limit'))
    // A refusal is not a download: nothing reaches the disk, and the selection stays
    // ticked so the user can drop a portfolio and try again.
    expect(createObjectURL).not.toHaveBeenCalled()
    expect(screen.getByTestId('download-bundle')).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: /IRA/i })).toBeChecked()
  })

  it('lists every validation error the server names', async () => {
    exportRespondsWith(jsonResponse({
      error: 'the bundle failed validation',
      errors: [
        { path: 'portfolios[0].name', reason: 'must not be empty' },
        { path: '', reason: 'a portfolio needs at least one asset' },
      ],
    }, 422))
    await tickAndClickDownload()

    await waitFor(() => expect(screen.getByTestId('export-error')).toBeInTheDocument())
    expect(screen.getByTestId('export-error')).toHaveTextContent('portfolios[0].name: must not be empty')
    expect(screen.getByTestId('export-error')).toHaveTextContent('a portfolio needs at least one asset')
  })

  it('says what failed when the answer is not the server talking', async () => {
    exportRespondsWith(new Response('<html>gateway timeout</html>', { status: 504 }))
    await tickAndClickDownload()

    await waitFor(() => expect(screen.getByTestId('export-error'))
      .toHaveTextContent('answered 504'))
    expect(createObjectURL).not.toHaveBeenCalled()
  })

  it('drops the refusal once the selection it was about has changed', async () => {
    exportRespondsWith(jsonResponse({ error: 'that selection is over the row limit' }, 422))
    await tickAndClickDownload()
    await waitFor(() => expect(screen.getByTestId('export-error')).toBeInTheDocument())

    // The refusal described a selection that no longer exists; leaving it on screen
    // would tell the user their new selection is refused too.
    await userEvent.click(screen.getByRole('checkbox', { name: /Main/i }))
    expect(screen.queryByTestId('export-error')).toBeNull()
  })

  it('leaves a modified click to the browser', async () => {
    exportRespondsWith(new Response(BUNDLE_BODY, { status: 200 }))
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.click(screen.getByRole('checkbox', { name: /IRA/i }))
    const link = await screen.findByTestId('download-bundle')
    // ctrl+click is the browser's own save-as, so the component must not intercept
    // it. The test cancels the click itself: jsdom has no save-as, and would print a
    // navigation error for the very click being checked.
    const cancel = (event: Event) => event.preventDefault()
    link.addEventListener('click', cancel)
    fireEvent.click(link, { ctrlKey: true })
    link.removeEventListener('click', cancel)
    // A macrotask boundary: an intercepted click fetches, builds a blob and clicks a
    // second anchor, and every hop of that is a microtask. Without this wait the
    // assertions below would run before an intercepted download had started.
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(link).toHaveAttribute('href', '/api/backup/export?portfolio_ids=2')
    expect(clicked).toHaveLength(0)
    expect(createObjectURL).not.toHaveBeenCalled()
  })
})

function bundleFile(name = 'bundle.json', portfolioName = 'Restored') {
  return new File([JSON.stringify({
    format: 'portfolio-prism-backup', schema_version: 1,
    created_at: '2026-11-13T00:00:00Z',
    portfolios: [{ ref: 'p1', name: portfolioName, description: null, currency: 'USD',
      base_currency: 'USD', assets: [] }],
  })], name, { type: 'application/json' })
}

/** Rows as the server counts them -- the shape `planResponse` is built from. */
interface Counts {
  portfolios: number
  assets: number
  transactions: number
}

/** A preview answer whose insert counts the test names, so the plan on screen can be
 * told apart from the plan of the file that is no longer on screen. */
function planResponse(insert: Counts, mode = 'copy'): Response {
  return jsonResponse(planBody({ mode, insert }))
}

/** Refuses every `/backup/preview` from here on with the server's own validation
 * body, and answers nothing else. */
function previewRefusedWith(errors: Array<{ path: string; reason: string }>) {
  fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
    if (url.includes('/backup/preview')) {
      return jsonResponse({ error: 'the bundle failed validation', errors }, 422)
    }
    return jsonResponse({}, 404)
  })
}

const ONE_NAME_EMPTY = [{ path: 'portfolios[0].name', reason: 'must not be empty' }]

/** Answers to `/backup/preview` only when the test says so, in the order the
 * requests arrived. The gate is what makes "no restore against a plan you have not
 * seen" checkable rather than racy: the moment the plan is at risk is the moment the
 * request is in flight, and a fetch that answers when it likes answers too early to
 * observe anything. */
function gatedPreviews(): Array<(res: Response) => void> {
  const pending: Array<(res: Response) => void> = []
  fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/backup/preview')) {
      return new Promise<Response>((resolve) => { pending.push(resolve) })
    }
    if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
    return jsonResponse({}, 404)
  })
  return pending
}

/** Answers a successful restore, and a preview of either mode, so a test can watch
 * what a write leaves behind. */
function restoreSucceeds(insert: Counts = { portfolios: 1, assets: 2, transactions: 3 }) {
  fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
    if (url.includes('/backup/preview')) {
      return jsonResponse(planBody({ mode: url.includes('mode=replace') ? 'replace' : 'copy' }))
    }
    if (url.includes('/backup/restore')) {
      return jsonResponse({ inserted: insert, portfolio_ids: [7], prices_inserted: 0 })
    }
    return jsonResponse({}, 404)
  })
}

/** Settles a held preview and lets every hop of the response land. */
async function answer(pending: Array<(res: Response) => void>, index: number, res: Response) {
  await act(async () => {
    pending[index](res)
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

describe('restore card', () => {
  it('keeps restore disabled until a preview has returned', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    expect(screen.getByTestId('restore-button')).toBeDisabled()
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    expect(screen.getByTestId('restore-button')).toBeEnabled()
  })

  it('renders the plan counts the server produced', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    expect(screen.getByTestId('preview-summary')).toHaveTextContent('1 portfolio')
    expect(screen.getByTestId('preview-summary')).toHaveTextContent('2 assets')
    expect(screen.getByTestId('preview-summary')).toHaveTextContent('3 transactions')
    // A bundle with no price section is not told to have one.
    expect(screen.queryByTestId('preview-prices')).toBeNull()
  })

  it('requires typing REPLACE before a replace restore is allowed', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    // Wait for the replace plan, so the disabled below is about the missing word and
    // not about a plan that had not landed yet.
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    expect(screen.getByTestId('restore-button')).toBeDisabled()
    await userEvent.type(screen.getByTestId('replace-confirm'), 'REPLAC')
    expect(screen.getByTestId('restore-button')).toBeDisabled()
    await userEvent.type(screen.getByTestId('replace-confirm'), 'E')
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
  })

  it('re-previews when the mode changes, so the counts match the mode', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    fetchMock.mockClear()
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      '/api/backup/preview?mode=replace', expect.objectContaining({ method: 'POST' })))
  })

  it('disables restore while the plan for the new file is still on its way', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())

    // The brief's version of this test uploaded the second file and asserted the
    // button was disabled a moment later. Under automatic preview that moment never
    // arrives -- the read, the upload and the answer all land before `upload()`
    // resolves -- so the request is held open instead, and the assertion is made
    // while the plan really is missing rather than after it has already been replaced.
    const pending = gatedPreviews()
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile('other.json'))
    await waitFor(() => expect(pending).toHaveLength(1))
    // The old file's plan went with the old file, and there is nothing to restore
    // against until the new file's plan is on screen.
    expect(screen.queryByTestId('preview-summary')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeDisabled()

    await answer(pending, 0, planResponse({ portfolios: 7, assets: 8, transactions: 9 }))
    expect(screen.getByTestId('restore-button')).toBeEnabled()
    // The counts on screen are the second file's. A plan that had survived the swap
    // would still be showing the first file's 1 portfolio, 2 assets.
    expect(screen.getByTestId('preview-summary')).toHaveTextContent('7 portfolios')
    expect(screen.getByTestId('preview-summary')).not.toHaveTextContent('1 portfolio, 2 assets')
  })

  it('drops the preview answer that arrives after a newer one', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    const pending = gatedPreviews()
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(pending).toHaveLength(1))
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile('other.json'))
    await waitFor(() => expect(pending).toHaveLength(2))

    // Two uploads, two answers, and the newer one gets in first. Without the sequence
    // guard the older answer then lands on top of it and the screen shows the first
    // file's plan beside the second file -- the mismatch this gate exists to make
    // impossible, and the one that cannot be provoked with a fetch that answers in
    // order.
    await answer(pending, 1, planResponse({ portfolios: 7, assets: 8, transactions: 9 }))
    expect(screen.getByTestId('preview-summary')).toHaveTextContent('7 portfolios')
    await answer(pending, 0, planResponse({ portfolios: 1, assets: 2, transactions: 3 }))

    expect(screen.getByTestId('preview-summary')).toHaveTextContent('7 portfolios')
    expect(screen.getByTestId('preview-summary')).not.toHaveTextContent('1 portfolio, 2 assets')
    expect(screen.getByTestId('restore-button')).toBeEnabled()
  })

  it('shows the server error when a restore is refused', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      if (String(input).includes('/backup/restore')) {
        return jsonResponse({ error: 'the bundle failed validation',
          errors: [{ path: 'portfolios[0].name', reason: 'must not be empty' }] }, 422)
      }
      return jsonResponse({}, 404)
    })
    await userEvent.click(screen.getByTestId('restore-button'))
    await waitFor(() => expect(screen.getByTestId('restore-error')).toHaveTextContent('must not be empty'))
  })

  // The rest of this describe is what the rulings name and the six above do not
  // cover: the server's own words on every refusal, a plan that survives neither a
  // file swap nor a refusal, and a confirmation that survives neither a mode switch
  // nor a second thought.

  it('drops the old plan when the new file is refused, so restore cannot run against it', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())

    // The first file had a plan. The second one is refused, and a plan left standing
    // beside a file that was refused is the stale plan this card is built to make
    // impossible -- it is the first file's counts, and restoring them is not what is
    // on screen.
    previewRefusedWith(ONE_NAME_EMPTY)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile('bad.json'))

    await waitFor(() => expect(screen.getByTestId('restore-error'))
      .toHaveTextContent('must not be empty'))
    expect(screen.queryByTestId('preview-summary')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })

  it('drops the plan when a retry against the same file is refused', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())

    // Same file, second press, different answer. Nothing about the file changed, so
    // this is the one refusal that cannot be cleared by the file input's own reset --
    // the plan on screen is the plan the server has just refused, and it has to go.
    previewRefusedWith(ONE_NAME_EMPTY)
    await userEvent.click(screen.getByTestId('preview-button'))

    await waitFor(() => expect(screen.getByTestId('restore-error'))
      .toHaveTextContent('must not be empty'))
    expect(screen.queryByTestId('preview-summary')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })

  it('renders the server reason when the preview refuses the bundle, and previews again when asked', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    previewRefusedWith(ONE_NAME_EMPTY)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-error'))
      .toHaveTextContent('portfolios[0].name: must not be empty'))
    // A refused bundle is not a plan: nothing is on screen to have been seen, so
    // there is nothing to restore against.
    expect(screen.queryByTestId('preview-summary')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeDisabled()

    // The refusal was about the file, not about the button. Asking again answers with
    // a plan, and the stale refusal goes with it.
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
      if (url.includes('/backup/preview')) {
        return planResponse({ portfolios: 1, assets: 2, transactions: 3 })
      }
      return jsonResponse({}, 404)
    })
    await userEvent.click(screen.getByTestId('preview-button'))
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    expect(screen.queryByTestId('restore-error')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeEnabled()
  })

  it('restores with the mode whose plan is on screen, and reports what was written', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    // The delete sentence and the write have to be about the same operation: the mock
    // echoes the mode that was asked for and counts what `replace` would really
    // delete, so the line below is the plan for the request about to be sent.
    await waitFor(() => expect(screen.getByTestId('preview-summary'))
      .toHaveTextContent('Will delete 2 portfolios, 4 assets, 6 transactions.'))
    await userEvent.type(screen.getByTestId('replace-confirm'), 'REPLACE')
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())

    // The write has to carry the mode the plan on screen was fetched for: a restore
    // that sent `copy` under a `replace` confirmation would delete nothing after
    // asking to be told it could delete everything.
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      if (String(input).includes('/backup/restore')) {
        return jsonResponse({ inserted: { portfolios: 1, assets: 2, transactions: 3 },
          portfolio_ids: [7], prices_inserted: 0 })
      }
      return jsonResponse({}, 404)
    })
    await userEvent.click(screen.getByTestId('restore-button'))
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      '/api/backup/restore?mode=replace', expect.objectContaining({ method: 'POST' })))
    // What the server says it wrote, not what the card hoped it wrote.
    await waitFor(() => expect(screen.getByTestId('restore-result'))
      .toHaveTextContent('Restored 1 portfolio, 2 assets, 3 transactions.'))
  })

  it('says the database is busy, and stays retryable, when a restore hits a lock', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      if (String(input).includes('/backup/restore')) {
        return jsonResponse({ error: 'the database is busy; try again in a moment' }, 503)
      }
      return jsonResponse({}, 404)
    })
    await userEvent.click(screen.getByTestId('restore-button'))
    // The server's own sentence, which is the one that says a retry is expected --
    // not a generic failure that reads like the bundle was thrown away.
    await waitFor(() => expect(screen.getByTestId('restore-error'))
      .toHaveTextContent('the database is busy; try again in a moment'))
    // A lock wrote nothing, so the same plan is still the plan and the same restore
    // is still available. Disabling it here would turn a busy database into a dead end.
    expect(screen.getByTestId('restore-button')).toBeEnabled()
  })

  it('will not promise anything was written when the answer never arrives', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      if (String(input).includes('/backup/restore')) return Promise.reject(new Error('connection reset'))
      return jsonResponse({}, 404)
    })
    await userEvent.click(screen.getByTestId('restore-button'))

    // A request that got no answer is not a request that did nothing: the write may
    // have landed, may have rolled back, or may never have arrived. "Something went
    // wrong, nothing was written" here is how a user restores the same bundle twice,
    // so the sentence says the outcome is unknown and sends them to reload.
    await waitFor(() => expect(screen.getByTestId('restore-error'))
      .toHaveTextContent('could not be confirmed'))
    expect(screen.getByTestId('restore-error')).not.toHaveTextContent('nothing was written')
    expect(screen.queryByTestId('restore-result')).toBeNull()
    // And the card does not offer what it cannot justify: pressing Restore again would
    // be a second write, not a retry, so the plan -- the only thing that arms the
    // button -- is gone. A preview stays on offer because a preview only ever reads.
    expect(screen.queryByTestId('preview-summary')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeDisabled()
    expect(screen.getByTestId('preview-button')).toBeEnabled()
  })

  it('treats a 200 that is not the outcome as an unknown, not a success', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/preview')) return jsonResponse(planBody())
      if (url.includes('/backup/restore')) return jsonResponse({ ok: true })
      return jsonResponse({}, 404)
    })
    await userEvent.click(screen.getByTestId('restore-button'))

    // A 200 whose body is not the outcome is an answer that cannot be read, which is
    // the same unknown as no answer: the write may have landed, so nothing may be
    // claimed about it and nothing may be offered on top of it.
    await waitFor(() => expect(screen.getByTestId('restore-error'))
      .toHaveTextContent('could not be confirmed'))
    expect(screen.queryByTestId('restore-result')).toBeNull()
    expect(screen.queryByTestId('preview-summary')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })

  it('asks for the typed confirmation again after a mode switch back to replace', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    await userEvent.type(screen.getByTestId('replace-confirm'), 'REPLACE')
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())

    // Away and back: the words were typed about a plan that no longer exists, and a
    // confirmation that survives the round trip is one the user never re-read.
    await userEvent.click(screen.getByRole('radio', { name: /copy/i }))
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    expect(screen.getByTestId('replace-confirm')).toHaveValue('')
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })
  it('discards the typed words on a mode switch even with no file chosen', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    // No preview runs when there is no file, so this is the mode switch on its own: the
    // words were typed about an operation that has just been walked away from, and a
    // box that still reads REPLACE on the way back is Ruling 3 broken in plain sight.
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    await userEvent.type(screen.getByTestId('replace-confirm'), 'REPLACE')
    await userEvent.click(screen.getByRole('radio', { name: /copy/i }))
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    expect(screen.getByTestId('replace-confirm')).toHaveValue('')
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })

  it('disarms the card after a restore, so a second write needs a fresh look', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
    restoreSucceeds()
    await userEvent.click(screen.getByTestId('restore-button'))
    await waitFor(() => expect(screen.getByTestId('restore-result')).toBeInTheDocument())

    // One restore is one write. An armed button makes the next click a second write:
    // a silent duplicate in `copy`, and in `replace` a second deletion of everything,
    // on the strength of a word typed before the first restore had landed.
    expect(screen.queryByTestId('preview-summary')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeDisabled()
    // The record of what was written stays, and the read-only way back is offered.
    expect(screen.getByTestId('restore-result')).toHaveTextContent('Restored 1 portfolio, 2 assets, 3 transactions.')
    expect(screen.getByTestId('preview-button')).toBeEnabled()

    // A fresh preview re-arms it, so this is friction and not a dead end.
    await userEvent.click(screen.getByTestId('preview-button'))
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
  })

  it('takes the typed words with a restore that succeeded', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    await userEvent.type(screen.getByTestId('replace-confirm'), 'REPLACE')
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())

    restoreSucceeds()
    await userEvent.click(screen.getByTestId('restore-button'))
    await waitFor(() => expect(screen.getByTestId('restore-result')).toBeInTheDocument())

    // The box still says what was deleted last time, which is not a promise about the
    // next time. Empty is the only honest value once the write is done.
    expect(screen.getByTestId('replace-confirm')).toHaveValue('')
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })

  it('takes the typed words with a re-preview of the same file', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    await userEvent.type(screen.getByTestId('replace-confirm'), 'REPLACE')
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())

    // The retry Ruling 1 mandates is not a no-op: the second answer is a different set
    // of counts -- it can count more to delete than the first -- and the words were
    // read against the first. They go, and the box says so by going empty.
    await userEvent.click(screen.getByTestId('preview-button'))
    await waitFor(() => expect(screen.getByTestId('preview-button')).toBeEnabled())
    expect(screen.getByTestId('replace-confirm')).toHaveValue('')
    expect(screen.getByTestId('restore-button')).toBeDisabled()

    await userEvent.type(screen.getByTestId('replace-confirm'), 'REPLACE')
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
  })

  it('refuses a preview answer whose echoed mode is not the one it asked for', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    // The delete sentence is rendered from the plan and the write is sent from the
    // component's own mode, so a plan that answers a different question than the one
    // asked would have the card describing one operation while performing the other.
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
      if (url.includes('/backup/preview')) return jsonResponse(planBody({ mode: 'copy' }))
      return jsonResponse({}, 404)
    })
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))

    await waitFor(() => expect(screen.getByTestId('restore-error'))
      .toHaveTextContent('not a plan this app could read'))
    expect(screen.queryByTestId('preview-summary')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })

  it('refuses a preview answer that is not a plan at all', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
      // The right mode, the right prices, and an `insert` that is not three counts: a
      // proxy that echoed the request and mangled the body on the way past it. The
      // failure this prevents is the quiet one: `countList(plan.insert)` reading
      // `portfolios` off the string 'two' throws during render, and with no error
      // boundary anywhere in this app the page would go blank at the moment the user is
      // deciding whether to destroy their data.
      if (url.includes('/backup/preview')) {
        return jsonResponse({
          mode: 'copy', prices: 0, warnings: [], ok: true,
          delete: { portfolios: 0, assets: 0, transactions: 0 },
          insert: 'two',
        })
      }
      return jsonResponse({}, 404)
    })
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())

    await waitFor(() => expect(screen.getByTestId('restore-error'))
      .toHaveTextContent('not a plan this app could read'))
    expect(screen.getByTestId('restore-card')).toBeInTheDocument()
    expect(screen.queryByTestId('preview-summary')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })

  it('refuses a plan with no warnings to render', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
      // Every count on it is fine; the array the summary maps over is simply not there.
      if (url.includes('/backup/preview')) {
        return jsonResponse({
          schema_version: 1, mode: 'copy', prices: 0,
          delete: { portfolios: 0, assets: 0, transactions: 0 },
          insert: { portfolios: 1, assets: 2, transactions: 3 },
        })
      }
      return jsonResponse({}, 404)
    })
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())

    await waitFor(() => expect(screen.getByTestId('restore-error'))
      .toHaveTextContent('not a plan this app could read'))
    expect(screen.queryByTestId('preview-summary')).toBeNull()
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })

  it('locks the file and the mode while a write is in flight', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())

    let release!: (res: Response) => void
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
      if (url.includes('/backup/preview')) return jsonResponse(planBody())
      if (url.includes('/backup/restore')) {
        return new Promise<Response>((resolve) => { release = resolve })
      }
      return jsonResponse({}, 404)
    })
    await userEvent.click(screen.getByTestId('restore-button'))
    await waitFor(() => expect(release).toBeDefined())

    // The summary describes the write that is in flight. Choosing a different file or a
    // different mode now would leave the card reading one operation and performing
    // another — the mismatch every other guard here exists to prevent.
    expect(screen.getByTestId('bundle-file')).toBeDisabled()
    expect(screen.getByRole('radio', { name: /copy/i })).toBeDisabled()
    expect(screen.getByRole('radio', { name: /replace/i })).toBeDisabled()

    await act(async () => {
      release(jsonResponse({ inserted: { portfolios: 1, assets: 2, transactions: 3 },
        portfolio_ids: [7], prices_inserted: 0 }))
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(screen.getByTestId('restore-result')).toBeInTheDocument()
    expect(screen.getByTestId('bundle-file')).toBeEnabled()
    expect(screen.getByRole('radio', { name: /copy/i })).toBeEnabled()
  })

  it('says what it leaves out of a bundle that carries price history', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
      if (url.includes('/backup/preview')) return jsonResponse(planBody({ prices: 4200 }))
      return jsonResponse({}, 404)
    })
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())

    // The card never sends `apply_prices`, so these rows are not coming. Saying so is
    // the difference between a bundle restored and a bundle restored except for the
    // part nobody mentioned.
    await waitFor(() => expect(screen.getByTestId('preview-prices'))
      .toHaveTextContent('4200 price rows in this bundle will be left alone'))
    // Still a plan, still restorable: naming what is left out is not a refusal.
    expect(screen.getByTestId('preview-summary')).toBeInTheDocument()
    expect(screen.getByTestId('restore-button')).toBeEnabled()
  })

  it('posts the bytes of the file that was chosen, to both routes', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile('mine.json', 'The Chosen One'))
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    restoreSucceeds()
    await userEvent.click(screen.getByTestId('restore-button'))
    await waitFor(() => expect(screen.getByTestId('restore-result')).toBeInTheDocument())

    // Both bodies are the file's own bytes, not a body the component assembled: a
    // preview of one bundle and a write of another is the mismatch every other guard
    // here is built to rule out.
    const bodies = fetchMock.mock.calls
      .filter(([url]) => String(url).includes('/backup/preview') || String(url).includes('/backup/restore'))
      .map(([, init]) => String((init as { body: string }).body))
    expect(bodies).toHaveLength(2)
    for (const body of bodies) {
      expect(JSON.parse(body).portfolios[0].name).toBe('The Chosen One')
    }
  })

})

