// UTF-8 byte-order mark, written as an escape so the source file stays plain ASCII.
const BOM = '\uFEFF';

/**
 * Hand generated text to the browser as a file download.
 *
 * There was no export precedent anywhere in this app before this helper, so this is the one
 * small place the next export is expected to copy rather than re-derive. Kept separate from
 * the CSV builders so those stay free of DOM access and unit-testable in plain Node.
 *
 * The BOM is part of the contract, not decoration, and it lives here rather than in the
 * builders so every CSV this app exports is covered by construction. Excel decides the
 * encoding from the BOM: without one it falls back to the machine's ANSI codepage, so on a
 * Windows-1250/1252 install a non-ASCII cell arrives mojibake'd. The free-text columns a
 * user types at asset creation -- symbol, asset_type and currency -- reach a cell that way:
 * the asset endpoint uppercases them and checks no charset, so any of the three can hold a
 * non-ASCII byte. The import side strips a BOM too, through Papa.BYTE_ORDER_MARK, so both
 * edges agree (pinned in csvParser.test.ts).
 */
export const downloadCsv = (filename: string, csv: string): void => {
  // Idempotent on purpose. The builders are expected to stay BOM-free -- sellPnlCsv.test.ts
  // pins that for the one that exists -- but a second BOM is worse than none: it puts
  // `\uFEFF` in front of the header and breaks the first column for every consumer, Excel
  // included. One comparison keeps that invariant here instead of two files away.
  const body = csv.startsWith(BOM) ? csv : BOM + csv;
  const url = URL.createObjectURL(new Blob([body], { type: 'text/csv;charset=utf-8' }));
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  // An off-document anchor has not always been enough: Safari and older Firefox are on
  // record for dropping a download whose anchor was never in the document, so it goes in and
  // straight back out again.
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  // Deferred one task rather than done on the next line. Chrome has the transfer underway
  // by the time click() returns, but that is browser behaviour and not the spec, and
  // revoking in the same tick has been seen to cancel the download elsewhere. One task is
  // long enough for the handover and short enough that the Blob does not outlive the click.
  setTimeout(() => URL.revokeObjectURL(url), 0);
};
