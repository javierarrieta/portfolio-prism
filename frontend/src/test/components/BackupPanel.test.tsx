import { render, screen, waitFor, within, fireEvent } from '@testing-library/react'
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

let fetchMock: ReturnType<typeof vi.fn>
beforeEach(() => {
  fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
    if (url.includes('/backup/preview')) return jsonResponse({
      schema_version: 1, mode: 'copy',
      delete: { portfolios: 0, assets: 0, transactions: 0 },
      insert: { portfolios: 1, assets: 2, transactions: 3 },
      prices: 0, warnings: [],
    })
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
